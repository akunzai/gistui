//! `Screen::Diff` — key handling, view-model, paint, palette items, and apply handlers
//! colocated in one file (issue #287, Phase 2; issue #383).

use crate::tui::bg::{record_pin_sync, refresh_locals, LoopFlow};
use crate::tui::gist_content::{ContentLookup, FetchPolicy};
use crate::tui::render::{diff_labels, preview_diff_text};
use crate::tui::view_model::{ChromeVm, DiffVm};
use crate::tui::{AppState, ConfigField, HelpTopic, HitTarget, KeyOutcome, PendingAction};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};
use std::path::PathBuf;

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
    let ContentLookup::Miss(file) =
        state
            .gist_content_store
            .lookup(&state.gist_catalog, file, FetchPolicy::Refresh)
    else {
        unreachable!("fresh fetch always bypasses cached content")
    };
    let (local_label, gist_label) = diff_labels(local_path, &gist);
    (file, local_label, gist_label)
}

/// Shared "would this key actually do something" predicate for `Screen::Diff`, mirrored by
/// both [`AppState::handle_key_diff`]'s match-arm guards and `diff_palette_items` so the two
/// can never silently drift (issue #288).
pub(crate) fn diff_guard(state: &AppState, code: KeyCode) -> bool {
    match code {
        KeyCode::Char('d' | 'u') => state.diff_allows_sync() && !state.diff_identical(),
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
                return KeyOutcome::DownloadRequested {
                    target: self.download_target(),
                };
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
        Some(PendingAction::Upload {
            gist_id, filename, ..
        }) => format!("Upload → gist {gist_id} / {filename}"),
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
            let local = state.preview_local();
            let target = state.download_target();
            if local.as_os_str().is_empty() || local == target {
                format!("{label} → {}", crate::config::display_path(&target))
            } else {
                format!(
                    "{label}: {} → {}",
                    crate::config::display_path(&local),
                    crate::config::display_path(&target)
                )
            }
        }
    }
}

/// The `Screen::Diff` preview: the diff pane plus a scroll/commands footer.
///
/// #72 audit: this footer intentionally does not surface `state.status`. Diff actions (`d`/`u`)
/// transition to `Screen::Confirm` or to the IO that lands back on `List`; their results surface
/// on those destination screens (which read `state.status`), so no status is set while on Diff.
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
    if !state.diff_allows_sync() {
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
    let download_target = state.download_target();
    let preview_local = state.preview_local();
    let ext = download_target
        .file_name()
        .or_else(|| preview_local.file_name())
        .and_then(|n| n.to_str())
        .and_then(crate::tui::view_model::file_ext);
    DiffVm {
        title: diff_title(state),
        body,
        footer: diff_footer(state),
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
        true,
        crate::tui::keymap::for_screen(&state.screen),
        &state.settings.theme(),
    );
    if chrome.mouse_enabled {
        let close = crate::tui::render_close_button(frame, area, &state.settings.theme());
        layout.register(HitTarget::Close, close);
    }
}

/// `PreviewDiff` outcome: build the local-vs-gist preview diff and, if the two sides are
/// byte-identical, opportunistically refresh the pin-sync cache with the content already
/// fetched (see the inline comment below for why).
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
    gist_file: Option<crate::domain::GistFileRef>,
) -> LoopFlow {
    match result {
        Ok(remote) => {
            match local_path
                .as_ref()
                .map(|p| crate::domain::read_text_file_capped(p))
                .transpose()
            {
                Ok(local) => {
                    let local_content = local.unwrap_or_default();
                    let diff = preview_diff_text(
                        upload_orientation,
                        &local_label,
                        &local_content,
                        &gist_label,
                        &remote,
                        state.settings.ignore_trailing_newline(),
                    );
                    let identical = crate::diff::content_eq(
                        &local_content,
                        &remote,
                        state.settings.ignore_trailing_newline(),
                    );
                    state.open_deferred(
                        entry,
                        crate::tui::Screen::Diff(Box::new(crate::tui::DiffState {
                            body: crate::tui::ScrollBody {
                                text: diff,
                                ..crate::tui::ScrollBody::default()
                            },
                            remote_content: remote,
                            local_path: local_path.unwrap_or_default(),
                            download_target: target,
                            identical,
                            gist_id: gist_file.as_ref().map(|file| file.gist_id.clone()),
                            gist_filename: gist_file.map(|file| file.filename),
                        })),
                    );
                    // A pin diff that turns out identical confirms the cached
                    // last_seen_hash is (still) accurate — refresh it for free
                    // using the content we already fetched, so the Pins list's
                    // content-hash check (AppState::compute_pin_sync_status) stays
                    // correct even if the gist changed elsewhere since the last
                    // real sync. Hash the LOCAL content's raw bytes (not the
                    // trailing-newline-normalized `identical` comparison), so
                    // this matches the raw-byte hashing compute_pin_sync_status does.
                    if identical {
                        let pin = state.diff().and_then(|d| {
                            Some((
                                d.gist_id.clone()?,
                                d.gist_filename.clone()?,
                                d.local_path.clone(),
                            ))
                        });
                        if let Some((gid, fname, local_abs)) = pin {
                            record_pin_sync(state, &local_abs, &gid, &fname, &local_content, None);
                        }
                    }
                }
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
    target: PathBuf,
    local_label: String,
    gist_label: String,
    file: crate::domain::GistFileRef,
) -> LoopFlow {
    match result {
        Ok(remote) => {
            if target.exists() {
                match crate::domain::read_text_file_capped(&target) {
                    Ok(local_content) => {
                        let diff = crate::diff::unified_diff(
                            &local_label,
                            &local_content,
                            &gist_label,
                            &remote,
                            state.settings.ignore_trailing_newline(),
                        );
                        let identical = crate::diff::content_eq(
                            &local_content,
                            &remote,
                            state.settings.ignore_trailing_newline(),
                        );
                        state.open_deferred(
                            entry,
                            crate::tui::Screen::Diff(Box::new(crate::tui::DiffState {
                                body: crate::tui::ScrollBody {
                                    text: diff,
                                    ..crate::tui::ScrollBody::default()
                                },
                                remote_content: remote,
                                local_path: target.clone(),
                                download_target: target,
                                identical,
                                gist_id: Some(file.gist_id),
                                gist_filename: Some(file.filename),
                            })),
                        );
                    }
                    Err(error) => state.set_status(error),
                }
            } else {
                match crate::actions::execute_download(
                    &target,
                    &remote,
                    crate::actions::DownloadMode::CreateNew,
                ) {
                    Ok(()) => {
                        state.set_status(format!(
                            "Downloaded {}",
                            target
                                .file_name()
                                .unwrap_or(target.as_os_str())
                                .to_string_lossy()
                        ));
                        record_pin_sync(
                            state,
                            &target,
                            &file.gist_id,
                            &file.filename,
                            &remote,
                            Some(crate::domain::SyncDirection::Download),
                        );
                        refresh_locals(state, Some(&target));
                    }
                    Err(error) => state.set_status(format!("download failed: {error}")),
                }
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
            let diff = crate::diff::unified_diff(
                &old_label,
                &old_content,
                &new_label,
                &new_content,
                state.settings.ignore_trailing_newline(),
            );
            let identical = old_content == new_content;
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
                    ..crate::tui::DiffState::default()
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

    #[test]
    fn u_in_diff_screen_returns_upload_intent() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/config"),
            modified: None,
        }];
        state.gist_catalog.owned = vec![GistFile {
            description: "x".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "settings.json")
        }];
        state.screen = Screen::Diff(Box::default());
        // The gist has no "config" file -> case B -> add directly.
        assert!(matches!(
            state.handle_key(KeyCode::Char('u')),
            KeyOutcome::UploadAdd { .. }
        ));
    }

    #[test]
    fn enter_diff_sets_diff_screen() {
        let mut state = initial_state();
        state.enter_diff(
            "the diff".into(),
            "remote body".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/cwd/x"),
        );
        assert!(state.screen.is_diff());
        assert!(state.diff_previewed());
        assert_eq!(state.preview_remote(), "remote body");
        assert_eq!(state.preview_local(), PathBuf::from("/tmp/x"));
        assert_eq!(state.download_target(), PathBuf::from("/tmp/cwd/x"));
        assert_eq!(state.scroll_body().expect("Diff ScrollBody").scroll, 0);
    }

    #[test]
    fn diff_scroll_respects_bounds() {
        let mut state = initial_state();
        state.enter_diff(
            "l1\nl2\nl3".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
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
        state.enter_diff(
            "abcd\nab".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
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
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/local"),
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
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/local"),
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
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/local"),
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
        state.enter_diff(
            String::new(),
            String::new(),
            PathBuf::new(),
            home.join("notes.txt"),
        );
        assert_eq!(diff_title(&state), "Diff → ~/notes.txt");
    }

    #[test]
    fn diff_view_title_shortens_both_home_paths() {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
        let mut state = initial_state();
        state.enter_diff(
            String::new(),
            String::new(),
            home.join("src").join("a.txt"),
            home.join("b.txt"),
        );
        assert_eq!(diff_title(&state), "Diff: ~/src/a.txt → ~/b.txt");
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
            None,
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
            None,
        );

        let diff = state.diff().expect("expected Screen::Diff");
        assert_eq!(diff.remote_content, "remote body");
        assert!(!diff.identical);
    }

    #[test]
    fn on_download_selected_err_sets_status() {
        let mut state = initial_state();

        on_download_selected(
            &mut state,
            initial_state().defer_entry(),
            Err("boom".into()),
            PathBuf::from("target"),
            "local".into(),
            "gist".into(),
            gist_file_ref("g1", "a.txt"),
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
            target.clone(),
            "local".into(),
            "gist".into(),
            gist_file_ref("g1", "a.txt"),
        );

        let diff = state.diff().expect("expected Screen::Diff");
        assert_eq!(diff.remote_content, "remote body");
        assert_eq!(diff.gist_id.as_deref(), Some("g1"));
        assert_eq!(diff.gist_filename.as_deref(), Some("a.txt"));
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
}
