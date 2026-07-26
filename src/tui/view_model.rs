//! Pure presentation seam: `AppState` (+ pin-sync cache) → immutable view models.
//!
//! The draw path builds a [`ViewModel`] once per frame and paints from it for the first-batch
//! screens (Pins, Confirm modal, Help). Other screens use [`ScreenVm::Legacy`] and may still
//! read `AppState` for their body. Builders never touch the filesystem or network (issue #241).

use super::render::{
    about_topic_lines_plain, confirm_modal_style, confirm_prompt, count_label, footer_with_status,
    help_topic_body, pin_row_label, CREATE_DESC_PREFIX, CREATE_DESC_SUFFIX, MINIMAL_HINT,
};
use super::{AppState, HelpTopic, PendingAction, Screen};
use crate::domain::SyncStatus;
use ratatui::style::Color;

/// Full-frame presentation contract produced by [`build_view_model`].
#[derive(Debug, Clone, PartialEq)]
pub struct ViewModel {
    pub chrome: ChromeVm,
    pub screen: ScreenVm,
}

/// Cross-screen chrome facts shared by every body (top bar / overlays).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChromeVm {
    /// Whether mouse hit targets (close button, list hits) should be recorded.
    pub mouse_enabled: bool,
    /// Background task overlay message, if any.
    pub bg_task_msg: Option<String>,
    pub spinner_frame: usize,
}

/// Per-screen body. First-batch screens carry structured data; the rest are [`Legacy`].
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenVm {
    Pins(PinsVm),
    Confirm(ConfirmVm),
    Help(HelpVm),
    /// Body still painted from `AppState` directly (transition).
    Legacy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinsVm {
    pub title: String,
    pub empty: PinsEmptyKind,
    pub rows: Vec<PinRowVm>,
    /// Selected index into [`Self::rows`] (not the raw pin index).
    pub selected: Option<usize>,
    pub filtering: bool,
    pub filter_query: String,
    pub footer_title: String,
    pub footer: String,
    pub footer_colored: bool,
    pub hscroll: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinsEmptyKind {
    /// Has rows to show.
    HasRows,
    NoMappings,
    NoFilterMatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinRowVm {
    pub pin_index: usize,
    pub status: SyncStatus,
    /// Full row label before horizontal scroll (paint and hscroll share this string).
    pub label: String,
}

/// Confirm **modal** contract only — background diff/gist still reads `AppState`.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmVm {
    pub title: &'static str,
    pub border: Color,
    pub kind: ConfirmModalKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmModalKind {
    /// Static y/n (or multi-key) prompt body.
    Prompt { text: String },
    /// Create-flow description editor.
    DescriptionInput {
        prefix: &'static str,
        value: String,
        suffix: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpVm {
    pub mode: HelpModeVm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpModeVm {
    Index {
        items: Vec<HelpIndexItemVm>,
        selected: usize,
    },
    Topic {
        title: String,
        /// Plain lines for the topic body (About is pre-formatted without ratatui spans).
        lines: Vec<String>,
        scroll: u16,
        /// Repo-link row index in About body lines, when this is the About topic.
        about_repo_line: Option<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpIndexItemVm {
    pub key: String,
    pub title: String,
}

/// Pure: map app state (+ pin sync cache) into a view model. No FS / network / mutation.
pub fn build_view_model(state: &AppState) -> ViewModel {
    let chrome = ChromeVm {
        mouse_enabled: state.mouse_enabled,
        bg_task_msg: state.bg_task_msg.clone(),
        spinner_frame: state.spinner_frame,
    };
    let screen = match state.screen {
        Screen::Pins => ScreenVm::Pins(build_pins_vm(state)),
        Screen::Confirm => ScreenVm::Confirm(build_confirm_vm(state)),
        Screen::Help => ScreenVm::Help(build_help_vm(state)),
        _ => ScreenVm::Legacy,
    };
    ViewModel { chrome, screen }
}

fn build_pins_vm(state: &AppState) -> PinsVm {
    let (footer_title, footer, footer_colored) = if state.pins.filtering {
        (
            "Filter (↑↓ move · Enter apply · Esc clear)".to_string(),
            format!("/{}_", state.pins.filter_query),
            false,
        )
    } else {
        let (footer, colored) = footer_with_status(state.status.as_deref(), MINIMAL_HINT);
        (String::new(), footer, colored)
    };

    let visible = state.visible_pin_indices();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let (empty, rows) = if state.pinned.is_empty() {
        (PinsEmptyKind::NoMappings, Vec::new())
    } else if visible.is_empty() {
        (PinsEmptyKind::NoFilterMatch, Vec::new())
    } else {
        let rows = visible
            .iter()
            .map(|&i| {
                let m = &state.pinned[i];
                let entry = state.cached_pin_sync_entry(i);
                let status = entry.status;
                let age = |ts: Option<u64>| {
                    ts.map(|t| crate::domain::humanize_age(now - t as i64))
                        .unwrap_or_else(|| "?".to_string())
                };
                let local_age = if status == SyncStatus::Missing {
                    "missing".to_string()
                } else {
                    age(entry.local_ts)
                };
                let label = pin_row_label(
                    status.icon(),
                    &m.local_path,
                    &m.gist_id,
                    &m.gist_filename,
                    &local_age,
                    &age(entry.remote_ts),
                );
                PinRowVm {
                    pin_index: i,
                    status,
                    label,
                }
            })
            .collect();
        (PinsEmptyKind::HasRows, rows)
    };

    let mut title = format!(
        "Pinned Mappings {}",
        count_label(visible.len(), state.pinned.len())
    );
    if !state.pins.filter_query.is_empty() {
        title.push_str(&format!(" · /{}", state.pins.filter_query));
    }
    if state.pins.sort != crate::tui::PinSort::Default {
        title.push_str(&format!(" · sort:{}", state.pins.sort.label()));
    }

    PinsVm {
        title,
        empty,
        rows,
        selected: (!visible.is_empty()).then_some(state.pins.index),
        filtering: state.pins.filtering,
        filter_query: state.pins.filter_query.to_string(),
        footer_title,
        footer,
        footer_colored,
        hscroll: state.pins.hscroll,
    }
}

fn build_confirm_vm(state: &AppState) -> ConfirmVm {
    let (title, border) = confirm_modal_style(state);
    let kind = if matches!(state.pending_action, Some(PendingAction::Create { .. }))
        && state.editing_description
    {
        ConfirmModalKind::DescriptionInput {
            prefix: CREATE_DESC_PREFIX,
            value: state.description_input.to_string(),
            suffix: CREATE_DESC_SUFFIX,
        }
    } else {
        ConfirmModalKind::Prompt {
            text: confirm_prompt(state),
        }
    };
    ConfirmVm {
        title,
        border,
        kind,
    }
}

fn build_help_vm(state: &AppState) -> HelpVm {
    let mode = if state.help.index_open {
        let items = HelpTopic::all()
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let key = if *t == HelpTopic::About {
                    "0".to_string()
                } else {
                    (i + 1).to_string()
                };
                HelpIndexItemVm {
                    key,
                    title: t.title().to_string(),
                }
            })
            .collect();
        HelpModeVm::Index {
            items,
            selected: state.help.index_sel,
        }
    } else {
        let title = format!(
            "Help · {} — Tab topics · ↑↓ scroll · Esc back",
            state.help.topic.title()
        );
        let (lines, about_repo_line) = if state.help.topic == HelpTopic::About {
            (
                about_topic_lines_plain(state),
                Some(super::render::ABOUT_REPO_LINE),
            )
        } else {
            (
                help_topic_body(state.help.topic)
                    .lines()
                    .map(str::to_string)
                    .collect(),
                None,
            )
        };
        HelpModeVm::Topic {
            title,
            lines,
            scroll: state.help.scroll,
            about_repo_line,
        }
    };
    HelpVm { mode }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{PinnedMapping, SyncStatus};
    use crate::tui::initial_state;
    use std::path::PathBuf;

    #[test]
    fn pins_vm_reads_cache_not_requiring_disk_for_status() {
        let mut state = initial_state();
        state.screen = Screen::Pins;
        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("notes.txt"),
            gist_id: "g1".into(),
            gist_filename: "notes.txt".into(),
            direction: None,
            last_seen_hash: None,
        }];
        // Hand-populate cache — builder must not need a real file.
        state.pin_sync_cache = vec![crate::tui::PinSyncCacheEntry {
            status: SyncStatus::InSync,
            local_ts: Some(100),
            remote_ts: Some(100),
        }];
        state.pin_sync_cache_dirty = false;

        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::Pins(pins) => {
                assert_eq!(pins.empty, PinsEmptyKind::HasRows);
                assert_eq!(pins.rows.len(), 1);
                assert_eq!(pins.rows[0].status, SyncStatus::InSync);
                assert!(pins.rows[0].label.contains('✓'));
                assert!(pins.rows[0].label.contains("notes.txt"));
            }
            other => panic!("expected Pins, got {other:?}"),
        }
    }

    #[test]
    fn pins_vm_unknown_when_cache_missing() {
        let mut state = initial_state();
        state.screen = Screen::Pins;
        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("a.txt"),
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: None,
        }];
        state.pin_sync_cache.clear();
        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::Pins(pins) => {
                assert_eq!(pins.rows[0].status, SyncStatus::Unknown);
                assert!(pins.rows[0].label.starts_with('?'));
            }
            other => panic!("expected Pins, got {other:?}"),
        }
    }

    #[test]
    fn pins_vm_empty_states() {
        let mut state = initial_state();
        state.screen = Screen::Pins;
        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::Pins(pins) => assert_eq!(pins.empty, PinsEmptyKind::NoMappings),
            other => panic!("expected Pins, got {other:?}"),
        }

        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("a.txt"),
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: None,
        }];
        state.pins.filter_query = crate::tui::TextInput::from("zzz-no-match");
        state.pin_sync_cache = vec![crate::tui::PinSyncCacheEntry::default()];
        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::Pins(pins) => assert_eq!(pins.empty, PinsEmptyKind::NoFilterMatch),
            other => panic!("expected Pins, got {other:?}"),
        }
    }

    #[test]
    fn confirm_vm_prompt_identity() {
        let mut state = initial_state();
        state.screen = Screen::Confirm;
        state.pending_action = Some(PendingAction::Upload {
            gist_id: "g1".into(),
            filename: "notes.txt".into(),
            local_path: PathBuf::from("notes.txt"),
        });
        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::Confirm(c) => {
                assert_eq!(c.title, "Upload");
                match c.kind {
                    ConfirmModalKind::Prompt { text } => {
                        assert!(text.contains("Upload notes.txt to gist g1"));
                    }
                    other => panic!("expected Prompt, got {other:?}"),
                }
            }
            other => panic!("expected Confirm, got {other:?}"),
        }
    }

    #[test]
    fn confirm_vm_overwrite_download() {
        let mut state = initial_state();
        state.screen = Screen::Confirm;
        state.pending_action = Some(PendingAction::Download);
        state.download_target = PathBuf::from("notes.txt");
        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::Confirm(c) => {
                assert_eq!(c.title, "Overwrite");
                match c.kind {
                    ConfirmModalKind::Prompt { text } => {
                        assert!(text.contains("Overwrite notes.txt"));
                    }
                    other => panic!("expected Prompt, got {other:?}"),
                }
            }
            other => panic!("expected Confirm, got {other:?}"),
        }
    }

    #[test]
    fn help_vm_index_lists_topics() {
        let mut state = initial_state();
        state.screen = Screen::Help;
        state.help.index_open = true;
        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::Help(h) => match h.mode {
                HelpModeVm::Index { items, selected } => {
                    assert!(!items.is_empty());
                    assert_eq!(selected, 0);
                    assert!(items.iter().any(|i| i.title.contains("List")));
                }
                other => panic!("expected Index, got {other:?}"),
            },
            other => panic!("expected Help, got {other:?}"),
        }
    }

    #[test]
    fn legacy_for_list_screen() {
        let state = initial_state();
        assert!(matches!(build_view_model(&state).screen, ScreenVm::Legacy));
    }

    #[test]
    fn chrome_carries_bg_task() {
        let mut state = initial_state();
        state.bg_task_msg = Some("Working…".into());
        state.spinner_frame = 3;
        let vm = build_view_model(&state);
        assert_eq!(vm.chrome.bg_task_msg.as_deref(), Some("Working…"));
        assert_eq!(vm.chrome.spinner_frame, 3);
    }
}
