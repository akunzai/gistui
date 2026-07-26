//! Pure presentation seam: `AppState` (+ pin-sync cache) → immutable view models.
//!
//! The draw path builds a [`ViewModel`] once per frame and paints from it for migrated
//! screens (Pins, Confirm modal, Help, List, …). Other screens use [`ScreenVm::Legacy`] and
//! may still read `AppState` for their body. Builders never touch the filesystem or network
//! (issues #241 / #250).

use super::render::{
    about_topic_lines_plain, confirm_modal_style, confirm_prompt, count_label, footer_with_status,
    gist_group_row_label, gist_info_line, gist_row_display, help_topic_body, marked_row_text,
    pin_row_label, row_mark, spinner_glyph, unix_now, RowMark, CREATE_DESC_PREFIX,
    CREATE_DESC_SUFFIX, MINIMAL_HINT,
};
use super::{AppState, DetailFocus, FocusPane, HelpTopic, PendingAction, Screen};
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

/// Per-screen body. Migrated screens carry structured data; the rest are [`Legacy`].
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenVm {
    List(ListVm),
    Gists(GistsVm),
    GistDetail(GistDetailVm),
    Pins(PinsVm),
    Confirm(ConfirmVm),
    Help(HelpVm),
    /// Body still painted from `AppState` directly (transition toward #250).
    Legacy,
}

/// Single-gist detail presentation (#250). Layout geometry still filled during paint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GistDetailVm {
    /// No `detail.gist_id` or group missing from state.
    pub missing: bool,
    pub block_title: String,
    pub info_line: String,
    pub focus: DetailFocus,
    pub files: Vec<String>,
    pub file_cursor: usize,
    pub comments: CommentsPaneVm,
    pub footer: String,
    pub footer_colored: bool,
    pub editing_description: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentsPaneVm {
    Loading,
    /// Not loaded yet — prompt to open the tab.
    PromptLoad,
    Error {
        message: String,
    },
    Empty,
    Thread {
        title: String,
        affordance: CommentsAffordance,
        lines: Vec<CommentLineVm>,
        scroll: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentsAffordance {
    LoadingMore,
    LoadOlder,
    StartOfThread,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentLineVm {
    Author { text: String },
    Body { text: String },
    Blank,
}

/// Gist-manager list presentation (#250).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GistsVm {
    pub title: String,
    pub empty: GistsEmptyKind,
    pub empty_message: Option<String>,
    pub rows: Vec<GistGroupRowVm>,
    pub selected: Option<usize>,
    pub filtering: bool,
    pub footer_title: String,
    pub footer: String,
    pub footer_colored: bool,
    pub hscroll: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GistsEmptyKind {
    HasRows,
    NoGists,
    NoFilterMatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GistGroupRowVm {
    pub gist_id: String,
    /// Full row label before horizontal scroll.
    pub label: String,
}

/// Main dual-pane List screen presentation (#250).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListVm {
    pub local: ListPaneVm,
    pub gist: ListPaneVm,
    pub local_hscroll: u16,
    pub gist_hscroll: u16,
    pub footer: ListFooterVm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPaneVm {
    pub title: String,
    pub focused: bool,
    pub selected: Option<usize>,
    pub empty: ListPaneEmpty,
    /// Prebuilt empty/loading/filter-miss message when [`Self::empty`] is not [`HasRows`].
    pub empty_message: Option<String>,
    pub rows: Vec<ListRowVm>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListPaneEmpty {
    HasRows,
    /// Local scan or gist fetch in progress with no rows yet.
    Loading,
    NoItems,
    NoFilterMatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRowVm {
    /// Full row text including pin mark prefix, before horizontal scroll.
    pub label: String,
    pub mark: RowMark,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListFooterVm {
    /// Idle command hints (colourised keys).
    Hints { text: String },
    /// One-shot status message (plain).
    Status { text: String },
    /// Inline filter on the focused pane; paint still uses live `TextInput` for the caret.
    Filtering { focus: FocusPane },
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

/// Pure chrome facts shared across screens (and palette backgrounds).
pub(crate) fn build_chrome(state: &AppState) -> ChromeVm {
    ChromeVm {
        mouse_enabled: state.mouse_enabled,
        bg_task_msg: state.bg_task_msg.clone(),
        spinner_frame: state.spinner_frame,
    }
}

/// Pure: map app state (+ pin sync cache) into a view model. No FS / network / mutation.
pub fn build_view_model(state: &AppState) -> ViewModel {
    let chrome = build_chrome(state);
    let screen = match state.screen {
        Screen::List => ScreenVm::List(build_list_vm(state)),
        Screen::Gists => ScreenVm::Gists(build_gists_vm(state)),
        Screen::GistDetail => ScreenVm::GistDetail(build_gist_detail_vm(state)),
        Screen::Pins => ScreenVm::Pins(build_pins_vm(state)),
        Screen::Confirm => ScreenVm::Confirm(build_confirm_vm(state)),
        Screen::Help => ScreenVm::Help(build_help_vm(state)),
        _ => ScreenVm::Legacy,
    };
    ViewModel { chrome, screen }
}

/// Gist detail body — usable under Palette-over-GistDetail as well.
pub(crate) fn build_gist_detail_vm(state: &AppState) -> GistDetailVm {
    let (footer, footer_colored) = footer_with_status(state.status.as_deref(), MINIMAL_HINT);
    let Some(gist_id) = state.detail.gist_id.as_deref() else {
        return GistDetailVm {
            missing: true,
            block_title: String::new(),
            info_line: String::new(),
            focus: state.detail.focus,
            files: Vec::new(),
            file_cursor: 0,
            comments: CommentsPaneVm::PromptLoad,
            footer,
            footer_colored,
            editing_description: state.editing_description,
        };
    };
    let Some(group) = state.group_by_id(gist_id) else {
        return GistDetailVm {
            missing: true,
            block_title: String::new(),
            info_line: String::new(),
            focus: state.detail.focus,
            files: Vec::new(),
            file_cursor: 0,
            comments: CommentsPaneVm::PromptLoad,
            footer,
            footer_colored,
            editing_description: state.editing_description,
        };
    };

    let block_title = if group.description.trim().is_empty() {
        format!("Gist {}", group.id)
    } else {
        format!("Gist: {}", group.description)
    };
    let info_line = gist_info_line(
        &group,
        unix_now(),
        state.current_user_login.as_deref(),
        state.gist_is_starred(gist_id),
        state.gist_counts(gist_id),
    );
    let files = state.gist_file_display_names(gist_id);
    let file_cursor = state.detail.file_cursor.min(files.len().saturating_sub(1));
    let comments = build_comments_pane_vm(state);

    GistDetailVm {
        missing: false,
        block_title,
        info_line,
        focus: state.detail.focus,
        files,
        file_cursor,
        comments,
        footer,
        footer_colored,
        editing_description: state.editing_description,
    }
}

fn build_comments_pane_vm(state: &AppState) -> CommentsPaneVm {
    let now = unix_now() as i64;
    match (
        &state.detail.comments,
        state.detail.comments_loading,
        &state.detail.comments_error,
    ) {
        (None, true, _) => CommentsPaneVm::Loading,
        (None, false, _) => CommentsPaneVm::PromptLoad,
        (Some(_), _, Some(err)) => CommentsPaneVm::Error {
            message: format!("comments error: {err}"),
        },
        (Some(comments), _, None) if comments.is_empty() => CommentsPaneVm::Empty,
        (Some(comments), _, None) => {
            let affordance = if state.detail.comments_loading_more {
                CommentsAffordance::LoadingMore
            } else if state.detail.comments_loaded_oldest_page > 1 {
                CommentsAffordance::LoadOlder
            } else {
                CommentsAffordance::StartOfThread
            };
            let mut lines = Vec::new();
            for c in comments {
                let age = crate::domain::parse_rfc3339_to_unix(&c.created_at)
                    .map(|t| crate::domain::humanize_age(now - t as i64))
                    .unwrap_or_else(|| "?".into());
                lines.push(CommentLineVm::Author {
                    text: format!("{} · {age}", c.author),
                });
                for raw in c.body.lines() {
                    lines.push(CommentLineVm::Body {
                        text: format!("  {raw}"),
                    });
                }
                lines.push(CommentLineVm::Blank);
            }
            CommentsPaneVm::Thread {
                title: comments_title_text(state),
                affordance,
                lines,
                scroll: state.detail.scroll,
            }
        }
    }
}

/// Mirror of render-side comments title (pure; used by the view model).
fn comments_title_text(state: &AppState) -> String {
    match (&state.detail.comments, state.detail.comments_total) {
        (Some(c), _) if state.detail.comments_error.is_some() => format!("Comments ({})", c.len()),
        (Some(c), Some(total)) if !c.is_empty() => {
            let loaded = c.len() as u32;
            let first = total.saturating_sub(loaded) + 1;
            format!("Comments ({first}–{total} / {total})")
        }
        (Some(c), None) if !c.is_empty() => format!("Comments (newest {})", c.len()),
        _ => "Comments".to_string(),
    }
}

/// Gists manager body — usable under Palette-over-Gists as well.
pub(crate) fn build_gists_vm(state: &AppState) -> GistsVm {
    let (footer_title, footer, footer_colored) = if state.gist_manager.filtering {
        (
            "Filter (↑↓ move · Enter apply · Esc clear)".to_string(),
            format!("/{}_", state.gist_manager.filter_query),
            false,
        )
    } else {
        let (footer, colored) = footer_with_status(state.status.as_deref(), MINIMAL_HINT);
        (String::new(), footer, colored)
    };

    let groups = state.visible_gist_groups();
    let total_groups = state.gist_groups().len();
    let now = unix_now();

    let (empty, empty_message, rows) = if groups.is_empty() {
        if total_groups == 0 {
            (
                GistsEmptyKind::NoGists,
                Some("  📭 No gists found".into()),
                Vec::new(),
            )
        } else {
            (
                GistsEmptyKind::NoFilterMatch,
                Some("  🔍 No gists match the filter".into()),
                Vec::new(),
            )
        }
    } else {
        let rows = groups
            .iter()
            .map(|g| GistGroupRowVm {
                gist_id: g.id.clone(),
                label: gist_group_row_label(
                    g,
                    now,
                    state.gist_manager.sort,
                    (
                        state.gist_comment_counts.get(&g.id).copied().unwrap_or(0),
                        state.gist_star_counts.get(&g.id).copied().unwrap_or(0),
                        state.gist_fork_counts.get(&g.id).copied().unwrap_or(0),
                    ),
                    state.gist_is_starred(&g.id),
                    state.current_user_login.as_deref(),
                ),
            })
            .collect();
        (GistsEmptyKind::HasRows, None, rows)
    };

    let mut title = format!(
        "Gists {}  ·  sort:{}  ·  type:{}  ·  ★ {}  ·  ⑂ {}",
        count_label(groups.len(), total_groups),
        state.gist_manager.sort.label(),
        state.gist_manager.type_filter.label(),
        state.starred_gist_count(),
        state.owned_fork_gist_count()
    );
    if !state.gist_manager.filter_query.is_empty() {
        title.push_str(&format!("  ·  /{}", state.gist_manager.filter_query));
    }

    GistsVm {
        title,
        empty,
        empty_message,
        rows,
        selected: (!groups.is_empty()).then_some(state.gist_manager.index),
        filtering: state.gist_manager.filtering,
        footer_title,
        footer,
        footer_colored,
        hscroll: state.gist_manager.hscroll,
    }
}

/// List body only — usable while `state.screen` is List **or** Palette-over-List (#250).
pub(crate) fn build_list_vm(state: &AppState) -> ListVm {
    let (visible_locals, ranked) = state.list_pane_snapshots();

    let local_empty;
    let local_empty_message;
    let local_rows;
    if state.local_scanning && state.locals.is_empty() {
        local_empty = ListPaneEmpty::Loading;
        local_empty_message = Some(format!(
            "  {} Scanning files…",
            spinner_glyph(state.spinner_frame)
        ));
        local_rows = Vec::new();
    } else if state.locals.is_empty() {
        local_empty = ListPaneEmpty::NoItems;
        local_empty_message = Some("  📭 No local files found".into());
        local_rows = Vec::new();
    } else if visible_locals.is_empty() {
        local_empty = ListPaneEmpty::NoFilterMatch;
        local_empty_message = Some("  🔍 No files match the filter".into());
        local_rows = Vec::new();
    } else {
        local_empty = ListPaneEmpty::HasRows;
        local_empty_message = None;
        local_rows = visible_locals
            .iter()
            .map(|r| {
                let mark = row_mark(&r.reasons);
                let base = super::text::local_row_label(&r.candidate.path, &state.cwd);
                ListRowVm {
                    label: marked_row_text(base, mark),
                    mark,
                }
            })
            .collect();
    }

    let recursive_marker = if state.local_recursive { " [↓]" } else { "" };
    let scanning_marker = if state.local_scanning { " …" } else { "" };
    let mut local_title = format!(
        "[1] Local {} · {}{}{} · sort:{}",
        count_label(visible_locals.len(), state.locals.len()),
        crate::config::display_path(&state.cwd),
        recursive_marker,
        scanning_marker,
        state.local_sort.label()
    );
    if !state.local_filter_query.is_empty() {
        local_title.push_str(&format!(" · /{}", state.local_filter_query));
    }
    if state.anchor == FocusPane::Local {
        local_title.push_str(" · ⚓");
    }

    let gist_empty;
    let gist_empty_message;
    let gist_rows;
    if state.loading && ranked.is_empty() {
        gist_empty = ListPaneEmpty::Loading;
        gist_empty_message = Some(format!(
            "  {} Loading gists…",
            spinner_glyph(state.spinner_frame)
        ));
        gist_rows = Vec::new();
    } else if ranked.is_empty() {
        if !state.filter_query.is_empty() {
            gist_empty = ListPaneEmpty::NoFilterMatch;
            gist_empty_message = Some("  🔍 No gists match the filter".into());
        } else {
            gist_empty = ListPaneEmpty::NoItems;
            gist_empty_message = Some("  📭 No gists found".into());
        }
        gist_rows = Vec::new();
    } else {
        gist_empty = ListPaneEmpty::HasRows;
        gist_empty_message = None;
        gist_rows = ranked
            .iter()
            .map(|g| {
                let mark = row_mark(&g.reasons);
                let base = gist_row_display(g, state.gist_view, state);
                ListRowVm {
                    label: marked_row_text(base, mark),
                    mark,
                }
            })
            .collect();
    }

    let mut gist_title = format!(
        "[2] Gists {} · {} · {}",
        count_label(ranked.len(), state.gists.len()),
        state.gist_type_filter.label(),
        state.gist_sort.label()
    );
    if !state.filter_query.is_empty() {
        gist_title.push_str(&format!(" · /{}", state.filter_query));
    }
    if state.anchor == FocusPane::Gist {
        gist_title.push_str(" · ⚓");
    }

    let footer = if state.filtering {
        ListFooterVm::Filtering { focus: state.focus }
    } else if let Some(message) = &state.status {
        ListFooterVm::Status {
            text: message.clone(),
        }
    } else {
        ListFooterVm::Hints {
            text: MINIMAL_HINT.to_string(),
        }
    };

    ListVm {
        local: ListPaneVm {
            title: local_title,
            focused: state.focus == FocusPane::Local,
            selected: (local_empty == ListPaneEmpty::HasRows).then_some(state.local_index),
            empty: local_empty,
            empty_message: local_empty_message,
            rows: local_rows,
        },
        gist: ListPaneVm {
            title: gist_title,
            focused: state.focus == FocusPane::Gist,
            selected: (gist_empty == ListPaneEmpty::HasRows).then_some(state.gist_index),
            empty: gist_empty,
            empty_message: gist_empty_message,
            rows: gist_rows,
        },
        local_hscroll: state.local_hscroll,
        gist_hscroll: state.gist_hscroll,
        footer,
    }
}

/// Pins body only — usable under Palette-over-Pins as well.
pub(crate) fn build_pins_vm(state: &AppState) -> PinsVm {
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

pub(crate) fn build_confirm_vm(state: &AppState) -> ConfirmVm {
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

/// Help body only — usable under Palette-over-Help as well.
pub(crate) fn build_help_vm(state: &AppState) -> HelpVm {
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
    fn list_vm_empty_local_and_gist_messages() {
        let state = initial_state();
        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::List(list) => {
                assert_eq!(list.local.empty, ListPaneEmpty::NoItems);
                assert!(list
                    .local
                    .empty_message
                    .as_deref()
                    .unwrap_or("")
                    .contains("No local files"));
                assert_eq!(list.gist.empty, ListPaneEmpty::NoItems);
                assert!(list
                    .gist
                    .empty_message
                    .as_deref()
                    .unwrap_or("")
                    .contains("No gists found"));
                assert!(matches!(list.footer, ListFooterVm::Hints { .. }));
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn list_vm_rows_include_pin_mark_and_star() {
        use crate::domain::{GistFile, LocalCandidate, PinnedMapping};

        let mut state = initial_state();
        state.cwd = PathBuf::from("/tmp/proj");
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("notes.txt"),
            pinned: true,
            modified: None,
        }];
        state.gists = vec![GistFile {
            description: "demo notes".into(),
            ..GistFile::for_sync("g1".into(), "notes.txt".into(), None)
        }];
        state.starred_gist_ids.insert("g1".into());
        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("notes.txt"),
            gist_id: "g1".into(),
            gist_filename: "notes.txt".into(),
            direction: None,
            last_seen_hash: None,
        }];
        state.focus = FocusPane::Local;
        state.anchor = FocusPane::Local;
        state.local_index = 0;
        state.gist_index = 0;

        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::List(list) => {
                assert_eq!(list.local.empty, ListPaneEmpty::HasRows);
                assert_eq!(list.gist.empty, ListPaneEmpty::HasRows);
                assert!(!list.local.rows.is_empty());
                assert!(!list.gist.rows.is_empty());
                assert!(
                    list.gist.rows[0].label.contains('★'),
                    "starred gist row: {}",
                    list.gist.rows[0].label
                );
                assert!(
                    list.local
                        .rows
                        .iter()
                        .any(|r| r.label.contains("notes.txt")),
                    "local rows: {:?}",
                    list.local.rows
                );
                // Pin or exact-filename mark when both sides share the pair.
                let marked = list
                    .local
                    .rows
                    .iter()
                    .chain(list.gist.rows.iter())
                    .any(|r| {
                        matches!(r.mark, RowMark::Pinned | RowMark::SameName)
                            || r.label.contains('📌')
                    });
                assert!(
                    marked,
                    "local={:?} gist={:?}",
                    list.local.rows, list.gist.rows
                );
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn list_vm_status_footer_and_filter_mode() {
        let mut state = initial_state();
        state.status = Some("Downloaded a.txt".into());
        match build_view_model(&state).screen {
            ScreenVm::List(list) => match list.footer {
                ListFooterVm::Status { text } => assert!(text.contains("Downloaded")),
                other => panic!("expected Status footer, got {other:?}"),
            },
            other => panic!("expected List, got {other:?}"),
        }

        state.status = None;
        state.filtering = true;
        state.focus = FocusPane::Gist;
        match build_view_model(&state).screen {
            ScreenVm::List(list) => match list.footer {
                ListFooterVm::Filtering { focus } => assert_eq!(focus, FocusPane::Gist),
                other => panic!("expected Filtering footer, got {other:?}"),
            },
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn gists_vm_empty_and_rows() {
        use crate::domain::GistFile;

        let mut state = initial_state();
        state.screen = Screen::Gists;
        match build_view_model(&state).screen {
            ScreenVm::Gists(g) => {
                assert_eq!(g.empty, GistsEmptyKind::NoGists);
                assert!(g
                    .empty_message
                    .as_deref()
                    .unwrap_or("")
                    .contains("No gists found"));
                assert!(g.title.contains("Gists"));
            }
            other => panic!("expected Gists, got {other:?}"),
        }

        state.gists = vec![
            GistFile {
                description: "alpha".into(),
                ..GistFile::for_sync("g1".into(), "a.txt".into(), None)
            },
            GistFile {
                description: "beta".into(),
                ..GistFile::for_sync("g2".into(), "b.txt".into(), None)
            },
        ];
        state.starred_gist_ids.insert("g1".into());
        state.gist_comment_counts.insert("g1".into(), 2);
        match build_view_model(&state).screen {
            ScreenVm::Gists(g) => {
                assert_eq!(g.empty, GistsEmptyKind::HasRows);
                assert_eq!(g.rows.len(), 2);
                assert!(g.rows.iter().any(|r| r.gist_id == "g1"));
                let starred = g.rows.iter().find(|r| r.gist_id == "g1").unwrap();
                assert!(
                    starred.label.contains('★') || starred.label.contains("g1"),
                    "row: {}",
                    starred.label
                );
                assert!(starred.label.contains("💬") || starred.label.contains("g1"));
            }
            other => panic!("expected Gists, got {other:?}"),
        }
    }

    #[test]
    fn gists_vm_filter_miss_and_status_footer() {
        use crate::domain::GistFile;

        let mut state = initial_state();
        state.screen = Screen::Gists;
        state.gists = vec![GistFile::for_sync("g1".into(), "a.txt".into(), None)];
        state.gist_manager.filter_query = crate::tui::TextInput::from("zzz-nope");
        match build_view_model(&state).screen {
            ScreenVm::Gists(g) => assert_eq!(g.empty, GistsEmptyKind::NoFilterMatch),
            other => panic!("expected Gists, got {other:?}"),
        }

        state.gist_manager.filter_query = crate::tui::TextInput::default();
        state.status = Some("Compacted g1".into());
        match build_view_model(&state).screen {
            ScreenVm::Gists(g) => {
                assert!(!g.footer_colored);
                assert!(g.footer.contains("Compacted"));
            }
            other => panic!("expected Gists, got {other:?}"),
        }
    }

    #[test]
    fn gist_detail_vm_missing_without_id() {
        let mut state = initial_state();
        state.screen = Screen::GistDetail;
        match build_view_model(&state).screen {
            ScreenVm::GistDetail(d) => {
                assert!(d.missing);
                assert!(matches!(d.comments, CommentsPaneVm::PromptLoad));
            }
            other => panic!("expected GistDetail, got {other:?}"),
        }
    }

    #[test]
    fn gist_detail_vm_files_and_comments_states() {
        use crate::domain::{GistComment, GistFile};

        let mut state = initial_state();
        state.screen = Screen::GistDetail;
        state.gists = vec![
            GistFile {
                description: "demo".into(),
                ..GistFile::for_sync("g1".into(), "a.txt".into(), None)
            },
            GistFile::for_sync("g1".into(), "b.txt".into(), None),
        ];
        state.detail.gist_id = Some("g1".into());
        state.detail.focus = DetailFocus::Files;
        state.detail.file_cursor = 1;

        match build_view_model(&state).screen {
            ScreenVm::GistDetail(d) => {
                assert!(!d.missing);
                assert!(d.block_title.contains("demo") || d.block_title.contains("g1"));
                assert!(
                    d.info_line.contains("g1")
                        || d.info_line.contains("secret")
                        || d.info_line.contains("public")
                );
                assert_eq!(d.files.len(), 2);
                assert_eq!(d.file_cursor, 1);
                assert_eq!(d.focus, DetailFocus::Files);
                assert!(matches!(d.comments, CommentsPaneVm::PromptLoad));
            }
            other => panic!("expected GistDetail, got {other:?}"),
        }

        state.detail.focus = DetailFocus::Comments;
        state.detail.comments = Some(vec![GistComment {
            author: "alice".into(),
            body: "hello\nworld".into(),
            created_at: "2020-01-01T00:00:00Z".into(),
        }]);
        state.detail.comments_total = Some(1);
        state.detail.comments_loaded_oldest_page = 1;
        match build_view_model(&state).screen {
            ScreenVm::GistDetail(d) => match d.comments {
                CommentsPaneVm::Thread {
                    title,
                    affordance,
                    lines,
                    ..
                } => {
                    assert!(title.contains("Comments"));
                    assert_eq!(affordance, CommentsAffordance::StartOfThread);
                    assert!(lines.iter().any(|l| matches!(
                        l,
                        CommentLineVm::Author { text } if text.contains("alice")
                    )));
                    assert!(lines.iter().any(|l| matches!(
                        l,
                        CommentLineVm::Body { text } if text.contains("hello")
                    )));
                }
                other => panic!("expected Thread, got {other:?}"),
            },
            other => panic!("expected GistDetail, got {other:?}"),
        }

        state.detail.comments = Some(vec![]);
        match build_view_model(&state).screen {
            ScreenVm::GistDetail(d) => assert!(matches!(d.comments, CommentsPaneVm::Empty)),
            other => panic!("expected GistDetail, got {other:?}"),
        }
    }

    #[test]
    fn legacy_for_diff_screen() {
        let mut state = initial_state();
        state.screen = Screen::Diff;
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
