//! `Screen::Diff` — key handling, view-model, paint, palette items, and apply handlers
//! colocated in one file (issue #287, Phase 2; issue #383).

use crate::tui::bg::{confirm_sync_baseline, LoopFlow};
use crate::tui::gist_content::GistContentStore;
use crate::tui::render::diff_labels;
use crate::tui::view_model::ChromeVm;
use crate::tui::{AppState, ConfigField, HelpTopic, HitTarget, KeyOutcome, PendingAction};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};
use std::path::PathBuf;

/// Diff screen / confirm background pane facts (#250). Highlighting still applied at paint time
/// with the live theme (body text + ext are pure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiffVm {
    pub title: String,
    /// Diff body after optional context collapse.
    pub body: String,
    pub footer: String,
    pub footer_colored: bool,
    pub wrap: bool,
    pub scroll: u16,
    pub hscroll: u16,
    pub syntax_highlight: bool,
    pub ext: Option<String>,
}

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::List;

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

pub(crate) fn wheel_step() -> usize {
    3
}

/// Stage a gist-vs-local diff before dispatch starts its fetch.
pub(crate) fn stage_preview_diff(
    state: &mut AppState,
    local_path: Option<PathBuf>,
    file: crate::domain::GistFileRef,
) -> (crate::domain::GistFileRef, String, String) {
    stage_gist_diff_fetch(state, local_path.as_deref(), file)
}

/// Stage labels for downloading a gist file before dispatch starts its fetch.
pub(crate) fn stage_download_gist(
    state: &mut AppState,
    target: PathBuf,
    file: crate::domain::GistFileRef,
) -> (crate::domain::GistFileRef, String, String) {
    stage_gist_diff_fetch(state, Some(&target), file)
}

fn stage_gist_diff_fetch(
    state: &mut AppState,
    local_path: Option<&std::path::Path>,
    file: crate::domain::GistFileRef,
) -> (crate::domain::GistFileRef, String, String) {
    let gist = state.gist_file_for_diff(&file);
    let file = GistContentStore::fetch_target(&state.gist_catalog, file);
    let (local_label, gist_label) = diff_labels(local_path, &gist);
    (file, local_label, gist_label)
}

/// Shared "would this key actually do something" predicate for `Screen::Diff`, mirrored by
/// both [`AppState::handle_key_diff`]'s match-arm guards and `diff_palette_items` so the two
/// can never silently drift (issue #288).
pub(crate) fn diff_guard(state: &AppState, code: KeyCode) -> bool {
    match code {
        KeyCode::Char('d' | 'u') => state.sync_pair().is_some() && !state.diff_identical(),
        _ => false,
    }
}

impl AppState {
    pub(crate) fn handle_key_diff(&mut self, code: KeyCode) -> KeyOutcome {
        match code {
            // In the diff, q and Esc return to wherever `enter()` recorded (List, Pins, …).
            KeyCode::Char('q') | KeyCode::Esc => {
                // Diff pairing identity lives on the payload; leaving drops it (not a full
                // `back_to_list()` — that would also discard the rest of `nav_stack`).
                self.leave();
            }
            // Identical files have nothing to sync, so download/upload are not offered.
            // Revision-history diffs are read-only (no local file pairing).
            KeyCode::Char('d') if diff_guard(self, code) => {
                if let Some(pair) = self.sync_pair() {
                    return KeyOutcome::DownloadRequested {
                        target: pair.local.clone(),
                    };
                }
            }
            KeyCode::Char('u') if diff_guard(self, code) => {
                return self.upload_intent();
            }
            // Toggle between the configured context radius and the full file; the line
            // count changes, so reset the vertical scroll. The choice is persisted.
            KeyCode::Char('c') => {
                let change = self
                    .settings
                    .adjust(ConfigField::DiffShowFull, true)
                    .unwrap();
                if let Some(body) = self.scroll_body_mut() {
                    body.scroll = 0;
                }
                return KeyOutcome::PersistSettings {
                    effect: change.effect,
                    success_message: if self.settings.diff_show_full() {
                        "Diff context: full file".into()
                    } else {
                        format!("Diff context: {} lines", self.settings.diff_context())
                    },
                };
            }
            // Soft-wrap long lines instead of horizontal scrolling; reset the now-meaningless
            // horizontal offset so wrapped lines start at column 0.
            KeyCode::Char('w') => {
                self.diff_wrap = !self.diff_wrap;
                if let Some(body) = self.scroll_body_mut() {
                    body.hscroll = 0;
                }
            }
            _ => {}
        }
        KeyOutcome::None
    }
}

/// The diff pane title. The gist id, filenames, and both sides' mtimes live in the diff's
/// `--- / +++` header lines (see `diff_labels`); the title stays concise and avoids
/// repeating a path.
pub(crate) fn diff_title(state: &AppState) -> String {
    match state.pending_action() {
        Some(PendingAction::Upload(draft)) => {
            format!("Upload → gist {} / {}", draft.gist_id, draft.filename)
        }
        Some(PendingAction::Create { local_path }) => {
            format!(
                "Create gist from {}",
                crate::config::display_path(local_path)
            )
        }
        Some(PendingAction::Delete { gist_id, .. }) => {
            format!("Delete gist {gist_id}")
        }
        Some(PendingAction::RemoveFile {
            gist_id, filename, ..
        }) => {
            format!("Remove {filename} from gist {gist_id}")
        }
        _ => {
            let label = if state.diff_identical() {
                "Diff (identical)"
            } else {
                "Diff"
            };
            match state.sync_pair() {
                Some(pair) => format!("{label} → {}", crate::config::display_path(&pair.local)),
                None => label.to_string(),
            }
        }
    }
}

/// The `Screen::Diff` preview: the diff pane plus a scroll/commands footer.
///
/// Footer hints for `Screen::Diff` (pure for tests).
pub(crate) fn diff_footer(state: &AppState) -> String {
    let context = if state.settings.diff_show_full() {
        "c context [full]".to_string()
    } else {
        format!("c context [{}]", state.settings.diff_context())
    };
    // When wrapping, horizontal scroll (←→) is meaningless — drop it from the hint.
    let scroll = if state.diff_wrap {
        "↑↓ PgUp/Dn scroll"
    } else {
        "↑↓←→ PgUp/Dn scroll"
    };
    let wrap = if state.diff_wrap {
        "w wrap [on]"
    } else {
        "w wrap [off]"
    };
    let back = "Esc/q back";
    if state.sync_pair().is_none() {
        if state.diff_identical() {
            format!("Files are identical  ·  {scroll}  ·  {wrap}  ·  {context}  ·  {back}")
        } else {
            format!("{scroll}  ·  {wrap}  ·  {context}  ·  {back}")
        }
    } else if state.diff_identical() {
        format!("Files are identical — nothing to sync  ·  {scroll}  ·  {wrap}  ·  {context}  ·  {back}")
    } else {
        format!("{scroll}  ·  d download  ·  u upload  ·  {wrap}  ·  {context}  ·  {back}")
    }
}

/// Diff pane facts — also used as Confirm overwrite background (non-compact).
pub(crate) fn build_diff_vm(state: &AppState) -> DiffVm {
    let (text, scroll, hscroll) = match state.scroll_body() {
        Some(b) => (b.text.as_str(), b.scroll, b.hscroll),
        None => ("", 0, 0),
    };
    let body = match state.effective_diff_context() {
        Some(radius) => crate::diff::collapse_context(text, radius),
        None => text.to_string(),
    };
    let ext = state
        .sync_pair()
        .and_then(|pair| pair.local.file_name())
        .and_then(|n| n.to_str())
        .and_then(crate::tui::render::labels::file_ext);
    let (footer, footer_colored) =
        crate::tui::footer_with_status(state.status.as_deref(), &diff_footer(state));
    DiffVm {
        title: diff_title(state),
        body,
        footer,
        footer_colored,
        wrap: state.diff_wrap,
        scroll,
        hscroll,
        syntax_highlight: state.syntax_highlight,
        ext,
    }
}

pub(crate) fn render_diff_vm(
    frame: &mut Frame,
    state: &AppState,
    diff: &DiffVm,
    chrome: &ChromeVm,
    layout: &mut crate::tui::MouseFrame,
) {
    let area = frame.area();
    let area = crate::tui::render_top_bar(
        frame,
        area,
        &state.settings.theme(),
        chrome.mouse_enabled,
        layout,
    );
    // Hint lines are trimmed to one row (#342); they never wrap.
    let footer_lines = 1;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(footer_lines)])
        .split(area);

    crate::tui::render_diff_pane_vm(frame, chunks[0], diff, &state.settings.theme());

    crate::tui::render_footer(
        frame,
        chunks[1],
        "",
        &diff.footer,
        diff.footer_colored,
        crate::tui::keymap::for_screen(&state.screen),
        &state.settings.theme(),
    );
    if chrome.mouse_enabled {
        let close = crate::tui::render_close_button(frame, area, &state.settings.theme());
        layout.register(HitTarget::Close, close);
    }
}

/// One Sync pair about to be shown as a Diff: both sides as read, and how to frame them.
struct SyncDiff {
    pair: crate::domain::SyncPair,
    local_content: String,
    remote: String,
    local_label: String,
    gist_label: String,
    /// Frame the diff as an upload (gist → local) rather than a download.
    upload_orientation: bool,
}

/// Open the Diff for a Sync pair. An identical pair is in sync: see
/// [`confirm_sync_baseline`].
fn open_sync_diff(state: &mut AppState, entry: crate::tui::DeferredEntry, sync: SyncDiff) {
    let policy = state.settings.sync_policy();
    let text = policy.preview_diff(
        sync.upload_orientation,
        &sync.local_label,
        &sync.local_content,
        &sync.gist_label,
        &sync.remote,
    );
    let identical = policy.identical(&sync.local_content, &sync.remote);
    let pair = sync.pair.clone();
    state.open_deferred(
        entry,
        crate::tui::Screen::Diff(Box::new(crate::tui::DiffState {
            body: crate::tui::ScrollBody {
                text,
                ..crate::tui::ScrollBody::default()
            },
            identical,
            kind: crate::tui::DiffKind::Sync {
                pair: sync.pair,
                remote: sync.remote.clone(),
            },
        })),
    );
    // After entering the Diff, which clears the status a failed record appends to.
    if identical {
        confirm_sync_baseline(
            state,
            &pair.local,
            &pair.gist,
            &sync.local_content,
            &sync.remote,
        );
    }
}

/// `PreviewDiff` outcome: diff the pair's local file (`local_path`, when one is selected)
/// against the fetched gist file. `d` / `u` from the Diff then write the pair — `target` and
/// `file` — and nothing else (#524).
#[allow(clippy::too_many_arguments)]
pub(crate) fn on_preview_diff(
    state: &mut AppState,
    entry: crate::tui::DeferredEntry,
    result: std::result::Result<String, String>,
    local_path: Option<PathBuf>,
    local_label: String,
    gist_label: String,
    target: PathBuf,
    upload_orientation: bool,
    file: crate::domain::GistFileRef,
) -> LoopFlow {
    match result {
        Ok(remote) => {
            match local_path
                .as_ref()
                .map(|p| crate::domain::read_text_file_capped(p))
                .transpose()
            {
                Ok(local) => open_sync_diff(
                    state,
                    entry,
                    SyncDiff {
                        pair: crate::domain::SyncPair {
                            local: target,
                            gist: file,
                        },
                        local_content: local.unwrap_or_default(),
                        remote,
                        local_label,
                        gist_label,
                        upload_orientation,
                    },
                ),
                Err(error) => state.set_status(format!("read failed: {error}")),
            }
        }
        Err(error) => state.set_status(format!("fetch failed: {error}")),
    }

    LoopFlow::Proceed
}

/// `DownloadSelected` outcome: diff against an existing local file, or write a new one.
pub(crate) fn on_download_selected(
    state: &mut AppState,
    entry: crate::tui::DeferredEntry,
    result: std::result::Result<String, String>,
    pair: crate::domain::SyncPair,
    local_label: String,
    gist_label: String,
) -> LoopFlow {
    match result {
        Ok(remote) => {
            if pair.local.exists() {
                match crate::domain::read_text_file_capped(&pair.local) {
                    Ok(local_content) => open_sync_diff(
                        state,
                        entry,
                        SyncDiff {
                            pair,
                            local_content,
                            remote,
                            local_label,
                            gist_label,
                            upload_orientation: false,
                        },
                    ),
                    Err(error) => state.set_status(error),
                }
            } else {
                let _ = crate::tui::bg::write_download(
                    state,
                    &pair.local,
                    &remote,
                    crate::actions::DownloadMode::CreateNew,
                    Some(&pair.gist),
                );
            }
        }
        Err(error) => state.set_status(format!("fetch failed: {error}")),
    }

    LoopFlow::Proceed
}

/// `RevisionDiff` outcome: diff two historical revisions of the same file.
pub(crate) fn on_revision_diff(
    state: &mut AppState,
    entry: crate::tui::DeferredEntry,
    result: std::result::Result<(String, String), String>,
    old_label: String,
    new_label: String,
) -> LoopFlow {
    match result {
        Ok((old_content, new_content)) => {
            let diff = state.settings.sync_policy().diff(
                &old_label,
                &old_content,
                &new_label,
                &new_content,
            );
            let identical = state
                .settings
                .sync_policy()
                .identical(&old_content, &new_content);
            // `enter_diff` (via `enter`) parks the live Revisions screen so Esc
            // restores list cursor/entries.
            state.open_deferred(
                entry,
                crate::tui::Screen::Diff(Box::new(crate::tui::DiffState {
                    body: crate::tui::ScrollBody {
                        text: diff,
                        ..crate::tui::ScrollBody::default()
                    },
                    identical,
                    kind: crate::tui::DiffKind::Revision,
                })),
            );
        }
        Err(error) => state.set_status(error),
    }

    LoopFlow::Proceed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::{
        gist_file_ref, set_diff_body, set_diff_scroll, set_pending, state_with_gists,
    };
    use crate::tui::*;
    use crossterm::event::KeyCode;
    use std::path::PathBuf;

    #[test]
    fn stage_preview_diff_builds_labels() {
        let mut state = state_with_gists();
        let file = gist_file_ref("g1", "a.txt");

        let (_, local_label, gist_label) =
            stage_preview_diff(&mut state, Some(PathBuf::from("/tmp/a.txt")), file);

        assert!(local_label.starts_with("local: a.txt"));
        assert!(gist_label.starts_with("gist g1 / a.txt"));
    }

    #[test]
    fn stage_download_gist_builds_labels() {
        let mut state = state_with_gists();
        let file = gist_file_ref("g1", "a.txt");

        let (_, local_label, gist_label) =
            stage_download_gist(&mut state, PathBuf::from("/tmp/a.txt"), file);

        assert!(local_label.starts_with("local: a.txt"));
        assert!(gist_label.starts_with("gist g1 / a.txt"));
    }

    #[test]
    fn diff_w_toggles_wrap_and_resets_hscroll() {
        let mut state = initial_state();
        state.screen = Screen::Diff(Box::default());
        state.scroll_body_mut().expect("Diff ScrollBody").hscroll = 5;
        assert!(!state.diff_wrap);
        state.handle_key(KeyCode::Char('w'));
        assert!(state.diff_wrap);
        // Horizontal offset is meaningless once wrapping, so it resets.
        assert_eq!(state.scroll_body().expect("Diff ScrollBody").hscroll, 0);
        state.handle_key(KeyCode::Char('w'));
        assert!(!state.diff_wrap);
    }

    #[test]
    fn diff_footer_reflects_wrap_toggle() {
        let mut state = initial_state();
        state.screen = Screen::Diff(Box::default());
        assert!(diff_footer(&state).contains("w wrap [off]"));
        state.diff_wrap = true;
        let footer = diff_footer(&state);
        assert!(footer.contains("w wrap [on]"));
        // The horizontal-scroll arrows are dropped from the hint when wrapping.
        assert!(!footer.contains("←→"));
    }

    #[test]
    fn diff_vm_surfaces_upload_error_in_footer() {
        let mut state = initial_state();
        state.screen = Screen::Diff(Box::default());
        state.set_status("upload failed: HTTP 403: Resource not accessible by token");

        let vm = build_diff_vm(&state);

        assert_eq!(
            vm.footer,
            "upload failed: HTTP 403: Resource not accessible by token"
        );
        assert!(!vm.footer_colored);
    }

    #[test]
    fn page_up_saturates_at_top_in_diff() {
        let mut state = initial_state();
        state.screen = Screen::Diff(Box::default());
        set_diff_body(&mut state, "a\nb\nc");
        set_diff_scroll(&mut state, 1);
        state.handle_key(KeyCode::PageUp);
        assert_eq!(state.scroll_body().expect("Diff ScrollBody").scroll, 0);
    }

    #[test]
    fn diff_context_toggle_flips_effective_radius() {
        let mut state = initial_state();
        assert_eq!(state.effective_diff_context(), Some(3));

        // Pressing `c` in the diff view flips to full view and resets the scroll.
        state.screen = Screen::Diff(Box::default());
        set_diff_scroll(&mut state, 12);
        let outcome = state.handle_key(KeyCode::Char('c'));
        assert_eq!(
            outcome,
            KeyOutcome::PersistSettings {
                effect: None,
                success_message: "Diff context: full file".into(),
            }
        );
        assert!(state.settings.diff_show_full());
        assert_eq!(state.scroll_body().expect("Diff ScrollBody").scroll, 0);
        assert_eq!(state.effective_diff_context(), None);

        // Pressing it again returns to the configured radius.
        state.handle_key(KeyCode::Char('c'));
        assert!(!state.settings.diff_show_full());
        assert_eq!(state.effective_diff_context(), Some(3));
    }

    /// A sync Diff whose gist file is not in the gist yet adds it (no overwrite preview);
    /// a revision Diff offers no upload at all.
    #[test]
    fn u_in_a_sync_diff_adds_a_gist_file_the_gist_lacks() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![GistFile::fixture("g1", "settings.json")];
        crate::tui::test_support::enter_sync_diff(
            &mut state,
            "d".into(),
            String::new(),
            PathBuf::from("/tmp/config"),
        );
        assert_eq!(
            state.handle_key(KeyCode::Char('u')),
            KeyOutcome::UploadAdd {
                local_path: PathBuf::from("/tmp/config"),
                gist_id: "g1".into(),
                filename: "config".into(),
            }
        );

        let mut state = initial_state();
        crate::tui::test_support::enter_revision_diff(&mut state, "d".into());
        assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::None);
    }

    /// Issues #494 / #524: a Diff may compare a local file with a gist file named
    /// differently — a pin, or a List selection. `u` and `d` write exactly the two files it
    /// shows, whichever screen opened it, never a file named after the other side.
    #[test]
    fn d_and_u_in_a_sync_diff_write_the_files_it_shows() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![GistFile::fixture("g1", "zshrc")];
        state.screen = Screen::Diff(Box::new(DiffState {
            kind: crate::tui::DiffKind::Sync {
                pair: crate::domain::SyncPair {
                    local: PathBuf::from("/home/u/.zshrc"),
                    gist: crate::domain::GistFileRef::id_name("g1", "zshrc"),
                },
                remote: String::new(),
            },
            ..DiffState::default()
        }));

        let KeyOutcome::UploadPreview {
            local_path, file, ..
        } = state.handle_key(KeyCode::Char('u'))
        else {
            panic!("expected an upload preview of the compared gist file");
        };
        assert_eq!(local_path, PathBuf::from("/home/u/.zshrc"));
        assert_eq!(
            (file.gist_id.as_str(), file.filename.as_str()),
            ("g1", "zshrc")
        );

        assert_eq!(
            state.handle_key(KeyCode::Char('d')),
            KeyOutcome::DownloadRequested {
                target: PathBuf::from("/home/u/.zshrc")
            }
        );
    }

    #[test]
    fn enter_diff_sets_diff_screen() {
        let mut state = initial_state();
        crate::tui::test_support::enter_sync_diff(
            &mut state,
            "the diff".into(),
            "remote body".into(),
            PathBuf::from("/tmp/cwd/x"),
        );
        assert!(state.screen.is_diff());
        assert!(state.diff_previewed());
        assert_eq!(
            state.sync_pair().map(|p| p.local.clone()),
            Some(PathBuf::from("/tmp/cwd/x"))
        );
        assert_eq!(state.scroll_body().expect("Diff ScrollBody").scroll, 0);
    }

    #[test]
    fn diff_scroll_respects_bounds() {
        let mut state = initial_state();
        crate::tui::test_support::enter_sync_diff(
            &mut state,
            "l1\nl2\nl3".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
        );
        assert_eq!(state.scroll_body().expect("Diff ScrollBody").scroll, 0);
        state.handle_key(KeyCode::Down);
        assert_eq!(state.scroll_body().expect("Diff ScrollBody").scroll, 1);
        state.handle_key(KeyCode::Up);
        assert_eq!(state.scroll_body().expect("Diff ScrollBody").scroll, 0);
    }

    #[test]
    fn diff_hscroll_respects_bounds() {
        let mut state = initial_state();
        crate::tui::test_support::enter_sync_diff(
            &mut state,
            "abcd\nab".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
        );
        assert_eq!(state.scroll_body().expect("Diff ScrollBody").hscroll, 0);
        state.handle_key(KeyCode::Right);
        assert_eq!(state.scroll_body().expect("Diff ScrollBody").hscroll, 1);
        state.handle_key(KeyCode::Left);
        assert_eq!(state.scroll_body().expect("Diff ScrollBody").hscroll, 0);
    }

    #[test]
    fn d_in_diff_requests_download_when_file_absent() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.json");
        let mut state = initial_state();
        crate::tui::test_support::enter_sync_diff(
            &mut state,
            "d".into(),
            "r".into(),
            missing.clone(),
        );
        assert!(matches!(
            state.handle_key(KeyCode::Char('d')),
            KeyOutcome::DownloadRequested { target } if target == missing
        ));
    }

    #[test]
    fn d_in_diff_requests_download_when_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("exists.json");
        std::fs::write(&existing, "old").unwrap();
        let mut state = initial_state();
        crate::tui::test_support::enter_sync_diff(
            &mut state,
            "d".into(),
            "r".into(),
            existing.clone(),
        );
        assert!(matches!(
            state.handle_key(KeyCode::Char('d')),
            KeyOutcome::DownloadRequested { target } if target == existing
        ));
        assert!(state.screen.is_diff());
    }

    #[test]
    fn d_in_diff_on_existing_requests_download() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("exists.json");
        std::fs::write(&existing, "old").unwrap();
        let mut state = initial_state();
        crate::tui::test_support::enter_sync_diff(
            &mut state,
            "d".into(),
            "r".into(),
            existing.clone(),
        );
        assert!(matches!(
            state.handle_key(KeyCode::Char('d')),
            KeyOutcome::DownloadRequested { target } if target == existing
        ));
        assert!(state.screen.is_diff());
    }

    #[test]
    fn create_diff_title_shortens_home_path() {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Create {
                local_path: home.join("notes.txt"),
            },
        );
        assert_eq!(diff_title(&state), "Create gist from ~/notes.txt");
    }

    #[test]
    fn diff_view_title_shortens_single_home_path() {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
        let mut state = initial_state();
        crate::tui::test_support::enter_sync_diff(
            &mut state,
            String::new(),
            String::new(),
            home.join("notes.txt"),
        );
        assert_eq!(diff_title(&state), "Diff → ~/notes.txt");
    }

    #[test]
    fn on_preview_diff_err_sets_status() {
        let mut state = initial_state();

        on_preview_diff(
            &mut state,
            initial_state().defer_entry(),
            Err("boom".into()),
            None,
            "local".into(),
            "gist".into(),
            PathBuf::from("target"),
            false,
            gist_file_ref("g1", "a.txt"),
        );

        assert_eq!(state.status.as_deref(), Some("fetch failed: boom"));
    }

    #[test]
    fn on_preview_diff_ok_without_local_enters_diff() {
        let mut state = initial_state();

        on_preview_diff(
            &mut state,
            initial_state().defer_entry(),
            Ok("remote body".into()),
            None,
            "local".into(),
            "gist".into(),
            PathBuf::from("target"),
            false,
            gist_file_ref("g1", "a.txt"),
        );

        let diff = state.diff().expect("expected Screen::Diff");
        assert!(matches!(
            &diff.kind,
            crate::tui::DiffKind::Sync { pair, remote }
                if remote == "remote body" && pair.local == std::path::Path::new("target")
        ));
        assert!(!diff.identical);
    }

    /// Issue #465: a CRLF gist against an LF local file is identical (nothing to sync) with
    /// normalization on; with it off the difference is real, so `d` / `u` stay available.
    #[test]
    fn on_preview_diff_line_endings_only_follows_the_setting() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("a.txt");
        std::fs::write(&local, "a\nb\n").unwrap();

        for normalize in [true, false] {
            let mut state = initial_state();
            if !normalize {
                state
                    .settings
                    .adjust(crate::tui::ConfigField::NormalizeLineEndings, true);
            }
            on_preview_diff(
                &mut state,
                initial_state().defer_entry(),
                Ok("a\r\nb\r\n".into()),
                Some(local.clone()),
                "local".into(),
                "gist".into(),
                local.clone(),
                false,
                gist_file_ref("g1", "a.txt"),
            );

            let diff = state.diff().expect("expected Screen::Diff");
            assert_eq!(diff.identical, normalize, "normalize={normalize}");
            assert_eq!(
                diff.body.text.contains("line endings differ"),
                !normalize,
                "normalize={normalize}"
            );
        }
    }

    #[test]
    fn on_download_selected_err_sets_status() {
        let mut state = initial_state();

        on_download_selected(
            &mut state,
            initial_state().defer_entry(),
            Err("boom".into()),
            crate::domain::SyncPair {
                local: PathBuf::from("target"),
                gist: gist_file_ref("g1", "a.txt"),
            },
            "local".into(),
            "gist".into(),
        );

        assert_eq!(state.status.as_deref(), Some("fetch failed: boom"));
    }

    #[test]
    fn on_download_selected_ok_existing_target_enters_diff() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        std::fs::write(&target, "local body").unwrap();
        let mut state = initial_state();

        on_download_selected(
            &mut state,
            initial_state().defer_entry(),
            Ok("remote body".into()),
            crate::domain::SyncPair {
                local: target.clone(),
                gist: gist_file_ref("g1", "a.txt"),
            },
            "local".into(),
            "gist".into(),
        );

        let diff = state.diff().expect("expected Screen::Diff");
        assert!(
            matches!(&diff.kind, crate::tui::DiffKind::Sync { remote, .. } if remote == "remote body")
        );
        assert_eq!(
            state.sync_pair(),
            Some(&crate::domain::SyncPair {
                local: target,
                gist: gist_file_ref("g1", "a.txt"),
            })
        );
    }

    /// Run `f` with a config dir holding one pin of `local_file` ↔ gist `g1` / `a.txt`.
    fn with_pinned_pair(local_file: &str, f: impl FnOnce(&mut AppState, PathBuf)) {
        let dir = tempfile::tempdir().unwrap();

        let local_path = dir.path().join("a.txt");
        std::fs::write(&local_path, local_file).unwrap();
        let mapping = crate::domain::PinnedMapping::fixture(local_path.clone(), "g1", "a.txt");

        let mut state = crate::tui::test_support::state_with_stored_pin(dir.path(), mapping);
        f(&mut state, local_path);
    }

    fn assert_baseline(state: &AppState, local: &str, remote: &str) {
        assert_eq!(
            state.pinned[0].baseline.local_sha256.as_deref(),
            Some(crate::domain::sha256_hex(local.as_bytes()).as_str())
        );
        assert_eq!(
            state.pinned[0].baseline.remote_blob_sha.as_deref(),
            Some(crate::domain::git_blob_sha1(remote.as_bytes()).as_str())
        );
    }

    /// Issue #492: a pin pull whose sides are identical under the Sync policy (here only a
    /// trailing newline apart) confirms the pin is in sync, so it must not stay on Pull.
    #[test]
    fn identical_pin_pull_records_the_sync_baseline() {
        with_pinned_pair("a", |state, local_path| {
            on_download_selected(
                state,
                initial_state().defer_entry(),
                Ok("a\n".into()),
                crate::domain::SyncPair {
                    local: local_path,
                    gist: gist_file_ref("g1", "a.txt"),
                },
                "local".into(),
                "gist".into(),
            );

            assert!(state.diff_identical());
            assert_baseline(state, "a", "a\n");
        });
    }

    /// Issues #466, #492, #494: an identical preview diff of a pinned pair records the
    /// baseline, wherever it was opened from.
    #[test]
    fn identical_preview_diff_records_the_sync_baseline() {
        with_pinned_pair("a\n", |state, local_path| {
            on_preview_diff(
                state,
                initial_state().defer_entry(),
                Ok("a\n".into()),
                Some(local_path.clone()),
                "local".into(),
                "gist".into(),
                local_path,
                false,
                gist_file_ref("g1", "a.txt"),
            );

            assert!(state.diff_identical());
            assert_eq!(
                state.sync_pair().map(|p| p.gist.gist_id.as_str()),
                Some("g1")
            );
            assert_baseline(state, "a\n", "a\n");
        });
    }

    #[test]
    fn on_revision_diff_ok_enters_diff() {
        let mut state = initial_state();

        on_revision_diff(
            &mut state,
            initial_state().defer_entry(),
            Ok(("old body".into(), "new body".into())),
            "old".into(),
            "new".into(),
        );

        let diff = state.diff().expect("expected Screen::Diff");
        assert!(!diff.identical);
    }

    #[test]
    fn on_revision_diff_err_sets_status() {
        let mut state = initial_state();

        on_revision_diff(
            &mut state,
            initial_state().defer_entry(),
            Err("boom".into()),
            "old".into(),
            "new".into(),
        );

        assert_eq!(state.status.as_deref(), Some("boom"));
    }

    #[test]
    fn diff_vm_title_footer_and_body() {
        let mut state = initial_state();
        crate::tui::test_support::enter_sync_diff(
            &mut state,
            "--- a\n+++ b\n-old\n+new\n".into(),
            String::new(),
            PathBuf::from("notes.txt"),
        );
        let d = build_diff_vm(&state);
        assert!(d.title.contains("Diff") || d.title.contains("notes"));
        assert!(d.body.contains("+new") || d.body.contains("old"));
        assert!(d.footer.contains("scroll") || d.footer.contains("back"));
        assert_eq!(d.ext.as_deref(), Some("txt"));
    }
}
