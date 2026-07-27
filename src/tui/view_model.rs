//! Pure presentation seam: `AppState` (+ pin-sync cache) → immutable view models.
//!
//! The draw path builds a [`ViewModel`] once per frame and paints from it for every screen.
//! Builders never touch the filesystem or network (issues #241 / #250).

use super::render::{
    about_topic_lines_plain, confirm_modal_style, confirm_prompt, count_label, diff_footer,
    diff_title, footer_with_status, gist_group_row_label, gist_info_line, gist_row_display,
    help_topic_body, marked_row_text, pin_row_label, revision_row_label, row_mark, spinner_glyph,
    unix_now, RowMark, CREATE_DESC_PREFIX, CREATE_DESC_SUFFIX, MINIMAL_HINT,
};
use super::{
    AppState, ConfigField, DetailFocus, FocusPane, HelpTopic, PaletteMode, PendingAction, Screen,
};
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

/// Per-screen body presentation contract (#250).
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenVm {
    List(ListVm),
    Gists(GistsVm),
    GistDetail(GistDetailVm),
    Revisions(RevisionsVm),
    Config(ConfigVm),
    Diff(DiffVm),
    Preview(PreviewVm),
    Pins(PinsVm),
    Confirm(ConfirmVm),
    Help(HelpVm),
    Palette(PaletteVm),
}

/// Command palette / context menu overlay (#250). `background` is the already-built ViewModel
/// for the screen underneath (issue #272) — `None` for a Confirm-origin (still unpainted, #277)
/// or Palette-origin (unreachable: the palette can't be opened while itself active).
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteVm {
    pub background: Option<Box<ScreenVm>>,
    pub title: &'static str,
    pub has_query: bool,
    pub selected: usize,
    pub items: Vec<PaletteRowVm>,
    pub key_width: usize,
    pub mode: PaletteMode,
    pub anchor: Option<(u16, u16)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteRowVm {
    pub key_hint: String,
    pub label: String,
    pub enabled: bool,
}

/// Compact-gist confirm background (info + file list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactGistBgVm {
    pub block_title: String,
    pub info_line: String,
    pub files: Vec<String>,
    pub file_cursor: usize,
}

/// Diff screen / confirm background pane facts (#250). Highlighting still applied at paint time
/// with the live theme (body text + ext are pure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffVm {
    pub title: String,
    /// Diff body after optional context collapse.
    pub body: String,
    pub footer: String,
    pub wrap: bool,
    pub scroll: u16,
    pub hscroll: u16,
    pub syntax_highlight: bool,
    pub ext: Option<String>,
}

/// Full-screen file preview (#250).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewVm {
    pub title: String,
    /// Raw file content (one logical source; paint may highlight/wrap).
    pub body: String,
    pub footer: String,
    pub footer_colored: bool,
    pub wrap: bool,
    pub scroll: u16,
    pub hscroll: u16,
    pub syntax_highlight: bool,
    pub ext: Option<String>,
}

/// Revision history list (#250).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionsVm {
    pub title: String,
    pub empty: RevisionsEmptyKind,
    pub empty_message: Option<String>,
    pub rows: Vec<String>,
    pub selected: Option<usize>,
    pub footer: String,
    pub footer_colored: bool,
    pub hscroll: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionsEmptyKind {
    HasRows,
    Loading,
    NoRevisions,
}

/// Settings screen (#250).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigVm {
    pub rows: Vec<String>,
    pub selected: usize,
    pub status: Option<String>,
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

/// Confirm modal + which background to paint under it.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmVm {
    pub title: &'static str,
    pub border: Color,
    pub kind: ConfirmModalKind,
    pub background: ConfirmBackgroundVm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmBackgroundVm {
    /// Standard overwrite/upload/create backdrop: use [`build_diff_vm`].
    Diff,
    /// Compaction confirm: gist info + file list.
    CompactGist(CompactGistBgVm),
    /// Missing group or nothing to show.
    Empty,
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
    let screen = match &state.screen {
        Screen::List => ScreenVm::List(build_list_vm(state)),
        Screen::Gists(_) => ScreenVm::Gists(build_gists_vm(state)),
        Screen::GistDetail(_) => ScreenVm::GistDetail(build_gist_detail_vm(state)),
        Screen::Revisions(_) => ScreenVm::Revisions(build_revisions_vm(state)),
        Screen::Config(_) => ScreenVm::Config(build_config_vm(state)),
        Screen::Diff(_) => ScreenVm::Diff(build_diff_vm(state)),
        Screen::Preview(_) => ScreenVm::Preview(build_preview_vm(state)),
        Screen::Pins(_) => ScreenVm::Pins(build_pins_vm(state)),
        Screen::Confirm(_) => ScreenVm::Confirm(build_confirm_vm(state)),
        Screen::Help(_) => ScreenVm::Help(build_help_vm(state)),
        Screen::Palette(_) => ScreenVm::Palette(build_palette_vm(state)),
    };
    ViewModel { chrome, screen }
}

/// Palette overlay body, plus the ViewModel for whatever screen it's covering (issue #272).
pub(crate) fn build_palette_vm(state: &AppState) -> PaletteVm {
    let p = state.palette().cloned().unwrap_or_default();
    let background = build_background_screen_vm(state, &p.origin_screen).map(Box::new);
    let has_query = p.mode == PaletteMode::Command;
    let title = match p.mode {
        PaletteMode::Menu => "Menu",
        PaletteMode::Command => "Command palette",
    };
    let items: Vec<PaletteRowVm> = state
        .palette_visible_items()
        .into_iter()
        .map(|item| PaletteRowVm {
            key_hint: item.key_hint.clone(),
            label: item.label.clone(),
            enabled: item.enabled,
        })
        .collect();
    let key_width = items
        .iter()
        .map(|item| item.key_hint.chars().count())
        .max()
        .unwrap_or(1)
        .max(1);
    PaletteVm {
        background,
        title,
        has_query,
        selected: p.selected,
        items,
        key_width,
        mode: p.mode,
        anchor: p.anchor,
    }
}

/// ViewModel for the screen a palette is covering, by its origin's tag. `state`'s accessors
/// (`config()`/`help()`/etc., #242) already resolve through a palette-parked payload, so these
/// build fns are called directly rather than on `p.origin_screen` itself.
///
/// `None` for Confirm (blank background preserved as-is, tracked separately in #277) and Palette
/// (unreachable — the palette can't be opened while itself active).
fn build_background_screen_vm(state: &AppState, origin: &Screen) -> Option<ScreenVm> {
    match origin {
        Screen::List => Some(ScreenVm::List(build_list_vm(state))),
        Screen::Gists(_) => Some(ScreenVm::Gists(build_gists_vm(state))),
        Screen::GistDetail(_) => Some(ScreenVm::GistDetail(build_gist_detail_vm(state))),
        Screen::Revisions(_) => Some(ScreenVm::Revisions(build_revisions_vm(state))),
        Screen::Config(_) => Some(ScreenVm::Config(build_config_vm(state))),
        Screen::Diff(_) => Some(ScreenVm::Diff(build_diff_vm(state))),
        Screen::Preview(_) => Some(ScreenVm::Preview(build_preview_vm(state))),
        Screen::Pins(_) => Some(ScreenVm::Pins(build_pins_vm(state))),
        Screen::Help(_) => Some(ScreenVm::Help(build_help_vm(state))),
        Screen::Confirm(_) | Screen::Palette(_) => None,
    }
}

pub(crate) fn build_compact_gist_bg_vm(state: &AppState, gist_id: &str) -> Option<CompactGistBgVm> {
    let group = state.group_by_id(gist_id)?;
    let block_title = if group.description.trim().is_empty() {
        format!("Gist {}", group.id)
    } else {
        format!("Gist: {}", group.description)
    };
    let files = state.gist_file_display_names(gist_id);
    let file_cursor = state
        .detail()
        .map(|d| d.file_cursor)
        .unwrap_or(0)
        .min(files.len().saturating_sub(1));
    Some(CompactGistBgVm {
        block_title,
        info_line: gist_info_line(
            &group,
            unix_now(),
            state.current_user_login.as_deref(),
            state.gist_is_starred(gist_id),
            state.gist_counts(gist_id),
        ),
        files,
        file_cursor,
    })
}

fn file_ext(name: &str) -> Option<String> {
    std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
}

/// Diff pane facts — also used as Confirm overwrite background (non-compact).
pub(crate) fn build_diff_vm(state: &AppState) -> DiffVm {
    let text = state.diff_body_text();
    let body = match state.effective_diff_context() {
        Some(radius) => crate::diff::collapse_context(text, radius),
        None => text.to_string(),
    };
    let download_target = state.download_target();
    let preview_local = state.preview_local();
    let ext = download_target
        .file_name()
        .or_else(|| preview_local.file_name())
        .and_then(|n| n.to_str())
        .and_then(file_ext);
    DiffVm {
        title: diff_title(state),
        body,
        footer: diff_footer(state),
        wrap: state.diff_wrap,
        scroll: state.diff_scroll(),
        hscroll: state.diff_hscroll(),
        syntax_highlight: state.syntax_highlight,
        ext,
    }
}

/// Preview body — usable under Palette-over-Preview as well.
pub(crate) fn build_preview_vm(state: &AppState) -> PreviewVm {
    let p = state.preview().cloned().unwrap_or_default();
    let hints = if state.preview_wrap {
        "↑↓ PgUp/Dn scroll  ·  w wrap [on]  ·  y/Y copy url/content  ·  R refresh  ·  Esc/q back"
    } else {
        "↑↓←→ PgUp/Dn scroll  ·  w wrap [off]  ·  y/Y copy url/content  ·  R refresh  ·  Esc/q back"
    };
    let (footer, footer_colored) = footer_with_status(state.status.as_deref(), hints);
    let ext = p
        .gist_key
        .as_ref()
        .and_then(|(_, filename)| file_ext(filename));
    PreviewVm {
        title: p.title,
        body: p.text,
        footer,
        footer_colored,
        wrap: state.preview_wrap,
        scroll: p.scroll,
        hscroll: p.hscroll,
        syntax_highlight: state.syntax_highlight,
        ext,
    }
}

/// Revisions body — usable under Palette-over-Revisions as well.
pub(crate) fn build_revisions_vm(state: &AppState) -> RevisionsVm {
    let rev = state.revision().cloned().unwrap_or_default();
    let (footer, footer_colored) = if let Some(message) = &state.status {
        (message.clone(), false)
    } else if rev.entries.is_none() {
        ("Loading revisions…".to_string(), false)
    } else if let Some(err) = &rev.fetch_error {
        (err.clone(), false)
    } else {
        let file = state.revision_target_file_label();
        (format!("file={file}"), false)
    };

    let gist_id = rev.gist_id.as_deref().unwrap_or("");
    let label = state
        .group_by_id(gist_id)
        .map(|g| {
            if g.description.trim().is_empty() {
                g.id.clone()
            } else {
                g.description.clone()
            }
        })
        .unwrap_or_else(|| gist_id.to_string());

    let now = unix_now();
    let (empty, empty_message, rows, selected) = match &rev.entries {
        None => (
            RevisionsEmptyKind::Loading,
            Some("  ⏳ Loading revisions…".into()),
            Vec::new(),
            None,
        ),
        Some(entries) if entries.is_empty() => (
            RevisionsEmptyKind::NoRevisions,
            Some("  📭 No revisions found".into()),
            Vec::new(),
            None,
        ),
        Some(entries) => {
            let rows = entries
                .iter()
                .enumerate()
                .map(|(i, r)| revision_row_label(r, i, now))
                .collect();
            (RevisionsEmptyKind::HasRows, None, rows, Some(rev.index))
        }
    };

    let count = rows.len();
    RevisionsVm {
        title: format!("Revisions: {label} {}", count_label(count, count)),
        empty,
        empty_message,
        rows,
        selected,
        footer,
        footer_colored,
        hscroll: rev.hscroll,
    }
}

/// Config/settings body — usable under Palette-over-Config as well.
pub(crate) fn build_config_vm(state: &AppState) -> ConfigVm {
    let rows = ConfigField::ALL
        .iter()
        .map(|field| {
            let label = field.label();
            let value = state.config_field_value(*field);
            let hint = if field.is_numeric() {
                "←/→"
            } else {
                "Enter"
            };
            format!("  {label:<28} {value:<8}  ({hint})")
        })
        .collect();
    ConfigVm {
        rows,
        selected: state.config().map(|c| c.index).unwrap_or(0),
        status: state.status.clone(),
    }
}

/// Gist detail body — usable under Palette-over-GistDetail as well.
pub(crate) fn build_gist_detail_vm(state: &AppState) -> GistDetailVm {
    let (footer, footer_colored) = footer_with_status(state.status.as_deref(), MINIMAL_HINT);
    let detail = state.detail().cloned().unwrap_or_default();
    let Some(gist_id) = detail.gist_id.as_deref() else {
        return GistDetailVm {
            missing: true,
            block_title: String::new(),
            info_line: String::new(),
            focus: detail.focus,
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
            focus: detail.focus,
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
    let file_cursor = detail.file_cursor.min(files.len().saturating_sub(1));
    let comments = build_comments_pane_vm(state);

    GistDetailVm {
        missing: false,
        block_title,
        info_line,
        focus: detail.focus,
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
    let detail = state.detail().cloned().unwrap_or_default();
    match (
        &detail.comments,
        detail.comments_loading,
        &detail.comments_error,
    ) {
        (None, true, _) => CommentsPaneVm::Loading,
        (None, false, _) => CommentsPaneVm::PromptLoad,
        (Some(_), _, Some(err)) => CommentsPaneVm::Error {
            message: format!("comments error: {err}"),
        },
        (Some(comments), _, None) if comments.is_empty() => CommentsPaneVm::Empty,
        (Some(comments), _, None) => {
            let affordance = if detail.comments_loading_more {
                CommentsAffordance::LoadingMore
            } else if detail.comments_loaded_oldest_page > 1 {
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
                scroll: detail.scroll,
            }
        }
    }
}

/// Mirror of render-side comments title (pure; used by the view model).
fn comments_title_text(state: &AppState) -> String {
    let detail = state.detail().cloned().unwrap_or_default();
    match (&detail.comments, detail.comments_total) {
        (Some(c), _) if detail.comments_error.is_some() => format!("Comments ({})", c.len()),
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
    let gm = state.gist_manager().cloned().unwrap_or_default();
    let (footer_title, footer, footer_colored) = if gm.filtering {
        (
            "Filter (↑↓ move · Enter apply · Esc clear)".to_string(),
            format!("/{}_", gm.filter_query),
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
                    gm.sort,
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
        gm.sort.label(),
        gm.type_filter.label(),
        state.starred_gist_count(),
        state.owned_fork_gist_count()
    );
    if !gm.filter_query.is_empty() {
        title.push_str(&format!("  ·  /{}", gm.filter_query));
    }

    GistsVm {
        title,
        empty,
        empty_message,
        rows,
        selected: (!groups.is_empty()).then_some(gm.index),
        filtering: gm.filtering,
        footer_title,
        footer,
        footer_colored,
        hscroll: gm.hscroll,
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
    let pins = state.pins().cloned().unwrap_or_default();
    let (footer_title, footer, footer_colored) = if pins.filtering {
        (
            "Filter (↑↓ move · Enter apply · Esc clear)".to_string(),
            format!("/{}_", pins.filter_query),
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
    if !pins.filter_query.is_empty() {
        title.push_str(&format!(" · /{}", pins.filter_query));
    }
    if pins.sort != crate::tui::PinSort::Default {
        title.push_str(&format!(" · sort:{}", pins.sort.label()));
    }

    PinsVm {
        title,
        empty,
        rows,
        selected: (!visible.is_empty()).then_some(pins.index),
        filtering: pins.filtering,
        filter_query: pins.filter_query.to_string(),
        footer_title,
        footer,
        footer_colored,
        hscroll: pins.hscroll,
    }
}

pub(crate) fn build_confirm_vm(state: &AppState) -> ConfirmVm {
    let (title, border) = confirm_modal_style(state);
    let kind = if matches!(state.pending_action(), Some(PendingAction::Create { .. }))
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
    let background = match state.pending_action() {
        Some(PendingAction::CompactGist { gist_id, .. }) => {
            match build_compact_gist_bg_vm(state, gist_id) {
                Some(bg) => ConfirmBackgroundVm::CompactGist(bg),
                None => ConfirmBackgroundVm::Empty,
            }
        }
        _ => ConfirmBackgroundVm::Diff,
    };
    ConfirmVm {
        title,
        border,
        kind,
        background,
    }
}

/// Help body only — usable under Palette-over-Help as well.
pub(crate) fn build_help_vm(state: &AppState) -> HelpVm {
    let help = state.help().cloned().unwrap_or_default();
    let mode = if help.index_open {
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
            selected: help.index_sel,
        }
    } else {
        let title = format!(
            "Help · {} — Tab topics · ↑↓ scroll · Esc back",
            help.topic.title()
        );
        let (lines, about_repo_line) = if help.topic == HelpTopic::About {
            (
                about_topic_lines_plain(state),
                Some(super::render::ABOUT_REPO_LINE),
            )
        } else {
            (
                help_topic_body(help.topic)
                    .lines()
                    .map(str::to_string)
                    .collect(),
                None,
            )
        };
        HelpModeVm::Topic {
            title,
            lines,
            scroll: help.scroll,
            about_repo_line,
        }
    };
    HelpVm { mode }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{PinnedMapping, SyncStatus};
    use crate::tui::{initial_state, ConfigState, HelpState};
    use std::path::PathBuf;

    #[test]
    fn pins_vm_reads_cache_not_requiring_disk_for_status() {
        let mut state = initial_state();
        state.screen = Screen::Pins(Box::default());
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
        state.screen = Screen::Pins(Box::default());
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
        state.screen = Screen::Pins(Box::default());
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
        if let Some(p) = state.pins_mut() {
            p.filter_query = crate::tui::TextInput::from("zzz-no-match");
        }
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
        state.enter_confirm(
            PendingAction::Upload {
                gist_id: "g1".into(),
                filename: "notes.txt".into(),
                local_path: PathBuf::from("notes.txt"),
            },
            String::new(),
        );
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
        state.enter_diff(
            String::new(),
            String::new(),
            PathBuf::new(),
            PathBuf::from("notes.txt"),
        );
        state.enter_confirm_from_diff(PendingAction::Download);
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
        state.screen = Screen::Help(Box::new(HelpState {
            index_open: true,
            ..HelpState::default()
        }));
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
        state.screen = Screen::Gists(Box::default());
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
        state.screen = Screen::Gists(Box::default());
        state.gists = vec![GistFile::for_sync("g1".into(), "a.txt".into(), None)];
        if let Some(gm) = state.gist_manager_mut() {
            gm.filter_query = crate::tui::TextInput::from("zzz-nope");
        }
        match build_view_model(&state).screen {
            ScreenVm::Gists(g) => assert_eq!(g.empty, GistsEmptyKind::NoFilterMatch),
            other => panic!("expected Gists, got {other:?}"),
        }

        if let Some(gm) = state.gist_manager_mut() {
            gm.filter_query = crate::tui::TextInput::default();
        }
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
        state.screen = Screen::GistDetail(Box::default());
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
        state.screen = Screen::GistDetail(Box::default());
        state.gists = vec![
            GistFile {
                description: "demo".into(),
                ..GistFile::for_sync("g1".into(), "a.txt".into(), None)
            },
            GistFile::for_sync("g1".into(), "b.txt".into(), None),
        ];
        if let Some(d) = state.detail_mut() {
            d.gist_id = Some("g1".into());
            d.focus = DetailFocus::Files;
            d.file_cursor = 1;
        }

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

        if let Some(d) = state.detail_mut() {
            d.focus = DetailFocus::Comments;
            d.comments = Some(vec![GistComment {
                author: "alice".into(),
                body: "hello\nworld".into(),
                created_at: "2020-01-01T00:00:00Z".into(),
            }]);
            d.comments_total = Some(1);
            d.comments_loaded_oldest_page = 1;
        }
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

        if let Some(d) = state.detail_mut() {
            d.comments = Some(vec![]);
        }
        match build_view_model(&state).screen {
            ScreenVm::GistDetail(d) => assert!(matches!(d.comments, CommentsPaneVm::Empty)),
            other => panic!("expected GistDetail, got {other:?}"),
        }
    }

    #[test]
    fn revisions_vm_loading_and_rows() {
        use crate::domain::{GistFile, GistRevision};

        let mut state = initial_state();
        state.screen = Screen::Revisions(Box::default());
        if let Some(r) = state.revision_mut() {
            r.gist_id = Some("g1".into());
        }
        state.gists = vec![GistFile {
            description: "hist".into(),
            ..GistFile::for_sync("g1".into(), "a.txt".into(), None)
        }];
        match build_view_model(&state).screen {
            ScreenVm::Revisions(r) => {
                assert_eq!(r.empty, RevisionsEmptyKind::Loading);
                assert!(r.footer.contains("Loading"));
                assert!(r.title.contains("hist") || r.title.contains("g1"));
            }
            other => panic!("expected Revisions, got {other:?}"),
        }

        if let Some(r) = state.revision_mut() {
            r.entries = Some(vec![GistRevision {
                version: "abc1234deadbeef".into(),
                committed_at: "2020-01-01T00:00:00Z".into(),
                user: "alice".into(),
                change_status: crate::domain::GistRevisionChangeStatus {
                    total: 1,
                    additions: 1,
                    deletions: 0,
                },
            }]);
            r.index = 0;
        }
        match build_view_model(&state).screen {
            ScreenVm::Revisions(r) => {
                assert_eq!(r.empty, RevisionsEmptyKind::HasRows);
                assert_eq!(r.rows.len(), 1);
                assert!(r.rows[0].contains("alice") || r.rows[0].contains("abc"));
                assert_eq!(r.selected, Some(0));
            }
            other => panic!("expected Revisions, got {other:?}"),
        }
    }

    #[test]
    fn config_vm_rows_and_status() {
        let mut state = initial_state();
        state.screen = Screen::Config(Box::new(ConfigState {
            index: 1,
            ..ConfigState::default()
        }));
        state.status = Some("Theme saved".into());
        match build_view_model(&state).screen {
            ScreenVm::Config(c) => {
                assert!(!c.rows.is_empty());
                assert_eq!(c.selected, 1);
                assert_eq!(c.status.as_deref(), Some("Theme saved"));
                assert!(c
                    .rows
                    .iter()
                    .any(|r| r.contains("Theme") || r.contains("Mouse")));
            }
            other => panic!("expected Config, got {other:?}"),
        }
    }

    #[test]
    fn diff_vm_title_footer_and_body() {
        let mut state = initial_state();
        state.enter_diff(
            "--- a\n+++ b\n-old\n+new\n".into(),
            String::new(),
            PathBuf::new(),
            PathBuf::from("notes.txt"),
        );
        match build_view_model(&state).screen {
            ScreenVm::Diff(d) => {
                assert!(d.title.contains("Diff") || d.title.contains("notes"));
                assert!(d.body.contains("+new") || d.body.contains("old"));
                assert!(d.footer.contains("scroll") || d.footer.contains("back"));
                assert_eq!(d.ext.as_deref(), Some("txt"));
            }
            other => panic!("expected Diff, got {other:?}"),
        }
    }

    #[test]
    fn preview_vm_title_and_status_footer() {
        let mut state = initial_state();
        state.enter_preview(
            "gist: notes.txt".into(),
            "hello preview\n".into(),
            Some(("g1".into(), "notes.rs".into())),
        );
        state.status = Some("refresh failed".into());
        match build_view_model(&state).screen {
            ScreenVm::Preview(p) => {
                assert_eq!(p.title, "gist: notes.txt");
                assert!(p.body.contains("hello preview"));
                assert!(!p.footer_colored);
                assert!(p.footer.contains("refresh failed"));
                assert_eq!(p.ext.as_deref(), Some("rs"));
            }
            other => panic!("expected Preview, got {other:?}"),
        }
    }

    #[test]
    fn palette_vm_items_and_title() {
        use crate::tui::palette::{PaletteExec, PaletteItem, PaletteMode, PaletteState};
        use crossterm::event::KeyCode;

        let mut state = initial_state();
        state.screen = Screen::Palette(Box::new(PaletteState {
            mode: PaletteMode::Menu,
            items: vec![PaletteItem {
                key_hint: "d".into(),
                label: "download".into(),
                exec: PaletteExec::Key(KeyCode::Char('d'), crossterm::event::KeyModifiers::empty()),
                enabled: true,
                search: "d download".into(),
            }],
            selected: 0,
            origin_screen: Screen::List,
            anchor: Some((10, 5)),
            ..PaletteState::default()
        }));
        match build_view_model(&state).screen {
            ScreenVm::Palette(p) => {
                assert_eq!(p.title, "Menu");
                match p.background.as_deref() {
                    Some(ScreenVm::List(_)) => {}
                    other => panic!("expected List background, got {other:?}"),
                }
                assert_eq!(p.items.len(), 1);
                assert_eq!(p.items[0].label, "download");
                assert!(p.items[0].enabled);
                assert_eq!(p.anchor, Some((10, 5)));
            }
            other => panic!("expected Palette, got {other:?}"),
        }
    }

    #[test]
    fn palette_vm_background_stays_blank_over_confirm() {
        let mut state = initial_state();
        state.enter_confirm(
            PendingAction::Upload {
                gist_id: "g1".into(),
                filename: "notes.txt".into(),
                local_path: PathBuf::from("notes.txt"),
            },
            String::new(),
        );
        // `;` (menu) has no items over Confirm and won't open (palette.rs:60-66); Ctrl+P
        // (command palette) always opens because it also carries the cross-screen items.
        state.open_palette_command();
        match build_view_model(&state).screen {
            ScreenVm::Palette(p) => assert!(p.background.is_none()),
            other => panic!("expected Palette, got {other:?}"),
        }
    }

    #[test]
    fn confirm_vm_compact_background() {
        use crate::domain::GistFile;

        let mut state = initial_state();
        state.gists = vec![GistFile {
            description: "pack".into(),
            ..GistFile::for_sync("g1".into(), "a.txt".into(), None)
        }];
        state.enter_confirm(
            PendingAction::CompactGist {
                gist_id: "g1".into(),
                label: "pack".into(),
                count: 3,
            },
            String::new(),
        );
        match build_view_model(&state).screen {
            ScreenVm::Confirm(c) => match c.background {
                ConfirmBackgroundVm::CompactGist(bg) => {
                    assert!(bg.block_title.contains("pack") || bg.block_title.contains("g1"));
                    assert!(!bg.files.is_empty());
                }
                other => panic!("expected CompactGist bg, got {other:?}"),
            },
            other => panic!("expected Confirm, got {other:?}"),
        }
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
