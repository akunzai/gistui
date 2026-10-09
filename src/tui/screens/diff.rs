//! `Screen::Diff` — key handling, view-model, paint, palette items, and apply handlers
//! colocated in one file (issue #287, Phase 2; issue #383).

use crate::tui::bg::LoopFlow;
use crate::tui::view_model::ChromeVm;
use crate::tui::{AppState, ConfigField, HelpTopic, HitTarget, KeyOutcome, PendingAction};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

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
    pub sides: Option<SideBySideVm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SideBySideVm {
    pub geometry: crate::tui::diff_geometry::DiffGeometry,
    pub selected: usize,
    pub local_title: String,
    pub gist_title: String,
}

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::Diff;

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

pub(crate) fn wheel_step() -> usize {
    3
}

/// Shared "would this key actually do something" predicate for `Screen::Diff`, mirrored by
/// both [`AppState::handle_key_diff`]'s match-arm guards and `diff_palette_items` so the two
/// can never silently drift (issue #288).
pub(crate) fn diff_guard(state: &AppState, code: KeyCode) -> bool {
    match code {
        KeyCode::Char('d' | 'u') => {
            state.sync_pair().is_some() && !state.diff_identical() && !state.hunks_dirty()
        }
        KeyCode::Char('n' | 'N' | '[' | ']') => {
            state
                .diff()
                .and_then(|d| d.merge.as_ref())
                .is_some_and(|m| m.hunk_count() > 0)
                && (code != KeyCode::Char(']')
                    || state.sync_pair().is_some_and(|p| {
                        state
                            .gist_catalog
                            .owned
                            .iter()
                            .any(|g| g.gist_id == p.gist.gist_id && g.filename == p.gist.filename)
                    }))
        }
        KeyCode::Char('z') => state
            .diff()
            .and_then(|d| d.merge.as_ref())
            .is_some_and(|m| m.can_undo()),
        KeyCode::Char('s') => state.hunks_dirty(),
        _ => false,
    }
}

impl crate::tui::DiffState {
    fn geometry(
        &self,
        radius: Option<usize>,
        wrap: bool,
    ) -> Option<crate::tui::diff_geometry::DiffGeometry> {
        self.merge.as_ref().map(|merge| {
            crate::tui::diff_geometry::DiffGeometry::new(merge, radius, self.merge_dimensions, wrap)
        })
    }

    /// The one place the unified preview and identical-state are derived: from the Merge's
    /// staged bytes under the Merge's policy, so no caller can refresh them out of order.
    fn refresh_derived(&mut self) {
        if let (Some(merge), crate::tui::DiffKind::Sync { pair }) = (&self.merge, &self.kind) {
            let policy = merge.policy();
            self.identical = policy.identical(&merge.local, &merge.gist);
            self.body.text = policy.diff(
                &format!("local: {}", crate::config::display_path(&pair.local)),
                &merge.local,
                &format!("gist: {} / {}", pair.gist.gist_id, pair.gist.filename),
                &merge.gist,
            );
        }
    }

    /// Install a Merge and derive every fact from it.
    #[cfg(test)]
    pub(crate) fn set_merge(&mut self, merge: crate::merge::Merge) {
        self.merge = Some(merge);
        self.refresh_derived();
    }

    /// Stage the selected hunk (`to_gist`) or undo; false when nothing changed.
    pub(crate) fn stage(&mut self, to_gist: bool) -> bool {
        let changed = self.merge.as_mut().is_some_and(|m| m.stage(to_gist));
        self.refresh_derived();
        changed
    }

    pub(crate) fn undo(&mut self) -> bool {
        let changed = self.merge.as_mut().is_some_and(|m| m.undo());
        self.refresh_derived();
        changed
    }

    /// Compare under the current Sync policy without touching any bytes or undo.
    pub(crate) fn apply_policy(&mut self, policy: crate::sync_content::SyncPolicy) {
        if let Some(merge) = &mut self.merge {
            merge.set_policy(policy);
        }
        self.refresh_derived();
    }

    /// Advance the sides that were written, with the exact bytes written.
    pub(crate) fn apply_saved(&mut self, local: Option<String>, gist: Option<String>) {
        if let Some(merge) = &mut self.merge {
            if local.is_some() || gist.is_some() {
                merge.saved(local, gist);
            }
        }
        self.refresh_derived();
    }

    /// The Gist side as last saved — what `d` writes. Never an unsaved staged buffer.
    pub(crate) fn saved_gist(&self) -> Option<&str> {
        self.merge.as_ref().map(|m| m.baseline_gist.as_str())
    }
}

impl AppState {
    /// A Diff parked under Config (or any other screen) follows the current Sync policy, so
    /// returning to it shows one comparison, preview and identical-state.
    pub(crate) fn apply_sync_policy_to_diffs(&mut self) {
        let policy = self.settings.sync_policy();
        let apply = |screen: &mut crate::tui::Screen| {
            if let crate::tui::Screen::Diff(diff) = screen {
                diff.apply_policy(policy);
            }
        };
        match &mut self.screen {
            crate::tui::Screen::Palette(p) => apply(&mut p.origin_screen),
            screen => apply(screen),
        }
        self.nav_stack.iter_mut().for_each(apply);
    }

    pub(crate) fn apply_diff_viewport(
        &mut self,
        viewport: crate::tui::diff_geometry::DiffViewport,
    ) {
        if let Some(diff) = self.diff_mut().filter(|d| d.merge.is_some()) {
            diff.merge_dimensions = Some(viewport.dimensions);
            diff.body.scroll = viewport.scroll;
        }
    }

    pub(crate) fn reveal_hunk(&mut self) {
        let radius = self.effective_diff_context();
        let wrap = self.diff_wrap;
        if let Some(diff) = self.diff_mut() {
            if let Some(geometry) = diff.geometry(radius, wrap) {
                diff.body.scroll = geometry.reveal(diff.merge.as_ref().unwrap().selected);
            }
        }
    }

    pub(crate) fn apply_navigation_diff(&mut self, action: crate::tui::keys::NavAction) -> bool {
        let radius = self.effective_diff_context();
        let wrapped = self.diff_wrap;
        let Some(diff) = self.diff_mut() else {
            return false;
        };
        let Some(geometry) = diff.geometry(radius, wrapped) else {
            return crate::tui::screens::scroll_navigation(self, action);
        };
        let merge = diff.merge.as_mut().unwrap();
        let position = geometry.navigate(
            action,
            crate::tui::diff_geometry::Position {
                scroll: diff.body.scroll,
                hscroll: diff.body.hscroll,
                selected: merge.selected,
            },
        );
        diff.body.scroll = position.scroll;
        diff.body.hscroll = position.hscroll;
        merge.selected = position.selected;
        true
    }

    fn change_hunk(&mut self, key: char) {
        if let Some(diff) = self.diff_mut() {
            match key {
                'n' | 'N' => {
                    if let Some(merge) = &mut diff.merge {
                        merge.jump(key == 'n');
                    }
                }
                '[' | ']' => {
                    diff.stage(key == ']');
                }
                'z' => {
                    diff.undo();
                }
                _ => {}
            }
        }
        self.reveal_hunk();
        self.status = None;
    }

    pub(crate) fn handle_key_diff(&mut self, code: KeyCode) -> KeyOutcome {
        match code {
            // In the diff, q and Esc return to wherever `enter()` recorded (List, Pins, …).
            KeyCode::Char('q') | KeyCode::Esc => {
                // Diff pairing identity lives on the payload; leaving drops it (not a full
                // `back_to_list()` — that would also discard the rest of `nav_stack`).
                self.request_hunk_back();
            }
            KeyCode::Char(key @ ('n' | 'N' | '[' | ']' | 'z')) if diff_guard(self, code) => {
                self.change_hunk(key);
            }
            KeyCode::Char('s') if diff_guard(self, code) => self.confirm_hunk_save(None),
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
                self.reveal_hunk();
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
                self.reveal_hunk();
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
    if let Some(merge) = state.diff().and_then(|d| d.merge.as_ref()) {
        if state.diff_identical() && !merge.dirty() {
            return format!("Files are identical — nothing to sync  ·  {scroll}  ·  {wrap}  ·  {context}  ·  {back}");
        }
        let hunk = if merge.hunk_count() == 0 {
            "No remaining hunks".to_string()
        } else {
            format!("Hunk {}/{}", merge.selected + 1, merge.hunk_count())
        };
        let actions = if merge.dirty() {
            "s save · z undo"
        } else {
            "d download · u upload"
        };
        return format!("{hunk}  ·  n/N hunks  ·  [ to Local  ·  ] to Gist  ·  {actions}  ·  {wrap}  ·  {context}  ·  {back}");
    }
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
        sides: if state.pending_action().is_some() {
            None
        } else {
            state
                .diff()
                .zip(state.sync_pair())
                .and_then(|(diff, pair)| {
                    let merge = diff.merge.as_ref()?;
                    Some(SideBySideVm {
                        geometry: crate::tui::diff_geometry::DiffGeometry::new(
                            merge,
                            state.effective_diff_context(),
                            diff.merge_dimensions,
                            state.diff_wrap,
                        ),
                        selected: merge.selected,
                        local_title: format!(
                            "Local{} — {}",
                            if merge.local_dirty() { " [staged]" } else { "" },
                            crate::config::display_path(&pair.local)
                        ),
                        gist_title: format!(
                            "Gist{} — {} / {}",
                            if merge.gist_dirty() { " [staged]" } else { "" },
                            pair.gist.gist_id,
                            pair.gist.filename
                        ),
                    })
                })
        },
    }
}

pub(crate) fn render_diff_vm(
    frame: &mut Frame,
    state: &AppState,
    diff: &DiffVm,
    chrome: &ChromeVm,
    layout: &mut crate::tui::MouseFrame,
    feedback: &mut crate::tui::render::RenderFeedback,
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

    feedback.diff_viewport =
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
                    merge: None,
                    merge_dimensions: None,
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
    use crate::tui::test_support::{set_diff_body, set_diff_scroll, set_pending};
    use crate::tui::*;
    use crossterm::event::KeyCode;
    use std::path::PathBuf;

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
        let KeyOutcome::Sync(request) = state.handle_key(KeyCode::Char('u')) else {
            panic!("expected an upload");
        };
        assert_eq!(request.pair.local, PathBuf::from("/tmp/config"));
        assert_eq!(request.pair.gist.filename, "config");
        assert_eq!(
            request.intent,
            crate::tui::sync::SyncIntent::Push { replaces: false }
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
            },
            ..DiffState::default()
        }));

        let KeyOutcome::Sync(request) = state.handle_key(KeyCode::Char('u')) else {
            panic!("expected an upload preview of the compared gist file");
        };
        assert_eq!(request.pair.local, PathBuf::from("/home/u/.zshrc"));
        assert_eq!(
            (
                request.pair.gist.gist_id.as_str(),
                request.pair.gist.filename.as_str()
            ),
            ("g1", "zshrc")
        );
        assert_eq!(
            request.intent,
            crate::tui::sync::SyncIntent::Push { replaces: true }
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
    #[test]
    fn staging_blocks_whole_file_actions_and_revision_writes() {
        let mut state = initial_state();
        crate::tui::test_support::enter_hunk_diff(&mut state, "a\nsame\nb\n", "x\nsame\ny\n");
        state.handle_key(KeyCode::Char(']'));
        state.handle_key(KeyCode::Char('['));
        assert!(state.hunks_dirty());
        assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::None);
        assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
        assert!(diff_footer(&state).contains("s save"));
        state.handle_key(KeyCode::Char('z'));
        state.handle_key(KeyCode::Char('z'));
        assert!(!state.hunks_dirty());
        crate::tui::test_support::enter_revision_diff(&mut state, "-old\n+new\n".into());
        for key in ['[', ']', 's', 'z'] {
            assert_eq!(state.handle_key(KeyCode::Char(key)), KeyOutcome::None);
        }
        assert!(build_diff_vm(&state).sides.is_none());
    }

    #[test]
    fn dirty_back_can_cancel_preview_save_or_discard() {
        let mut state = initial_state();
        crate::tui::test_support::enter_hunk_diff(&mut state, "local\n", "gist\n");
        state.handle_key(KeyCode::Char(']'));
        state.handle_key(KeyCode::Esc);
        assert!(matches!(
            state.pending_action(),
            Some(PendingAction::LeaveHunks(
                crate::tui::hunk_sync::HunkExit::Back
            ))
        ));
        state.handle_key(KeyCode::Char('n'));
        assert!(state.screen.is_diff() && state.hunks_dirty());
        state.handle_key(KeyCode::Char('q'));
        state.handle_key(KeyCode::Char('s'));
        assert!(
            matches!(state.pending_action(), Some(PendingAction::SaveHunks(request)) if request.exit == Some(crate::tui::hunk_sync::HunkExit::Back))
        );
        state.handle_key(KeyCode::Esc);
        assert!(state.hunks_dirty());
        state.handle_key(KeyCode::Esc);
        state.handle_key(KeyCode::Char('d'));
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn hunk_selection_survives_context_wrap_and_scroll_is_bounded() {
        let mut state = initial_state();
        let middle = "same\n".repeat(30);
        crate::tui::test_support::enter_hunk_diff(
            &mut state,
            &format!("a\n{middle}b\n"),
            &format!("x\n{middle}y\n"),
        );
        state.handle_key(KeyCode::Char('n'));
        state.handle_key(KeyCode::Char('c'));
        state.handle_key(KeyCode::Char('w'));
        state.handle_key(KeyCode::Char(']'));
        let merge = state.diff().unwrap().merge.as_ref().unwrap();
        assert_eq!(merge.gist, format!("x\n{middle}b\n"));
        for _ in 0..10 {
            state.handle_key(KeyCode::PageDown);
        }
        let diff = state.diff().unwrap();
        let (geometry, _) = diff
            .geometry(state.effective_diff_context(), state.diff_wrap)
            .unwrap()
            .frame(
                diff.merge_dimensions.unwrap_or_default(),
                diff.body.scroll,
                diff.merge.as_ref().unwrap().selected,
            );
        assert!(usize::from(diff.body.scroll) < geometry.rows().len());
    }

    #[test]
    fn staged_top_navigation_prompts_and_discard_continues_to_target() {
        for pins in [true, false] {
            let mut state = initial_state();
            crate::tui::test_support::enter_hunk_diff(&mut state, "local\n", "gist\n");
            state.handle_key(KeyCode::Char(']'));
            if pins {
                state.open_pins();
            } else {
                state.open_gist_manager();
            }
            assert!(state.screen.is_confirm());
            state.handle_key(KeyCode::Char('d'));
            if pins {
                assert!(state.screen.is_pins());
            } else {
                assert!(state.screen.is_gists());
            }
        }
    }

    #[test]
    fn palette_quit_preserves_staging_until_an_explicit_discard() {
        let mut state = initial_state();
        crate::tui::test_support::enter_hunk_diff(&mut state, "local\n", "gist\n");
        state.handle_key(KeyCode::Char(']'));
        state.open_palette_command();
        let index = state
            .palette_visible_items()
            .iter()
            .position(|item| {
                matches!(
                    item.exec,
                    crate::tui::palette::PaletteExec::Cross(crate::tui::palette::CrossAction::Quit)
                )
            })
            .unwrap();
        state.palette_mut().unwrap().selected = index;
        assert_eq!(state.execute_palette_selection(), KeyOutcome::None);
        assert!(matches!(
            state.pending_action(),
            Some(PendingAction::LeaveHunks(
                crate::tui::hunk_sync::HunkExit::Quit
            ))
        ));
        state.handle_key(KeyCode::Char('n'));
        assert!(state.hunks_dirty());
        assert_eq!(state.request_hunk_quit(), KeyOutcome::None);
        assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::Quit);
    }
    #[test]
    fn first_frame_and_resize_reveal_the_selected_hunk_at_current_width() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = initial_state();
        let middle = format!("{}\nsame\n", "unchanged ".repeat(12));
        crate::tui::test_support::enter_hunk_diff(
            &mut state,
            &format!("local first\n{middle}local second\ntail\ntail\n"),
            &format!("gist first\n{middle}gist second\ntail\ntail\n"),
        );
        state.handle_key(KeyCode::Char('w'));
        state.handle_key(KeyCode::Char('n'));
        for (width, height) in [(81, 12), (81, 8), (49, 12), (115, 12), (24, 12)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut feedback = crate::tui::render::RenderFeedback::default();
            let mut layout = crate::tui::MouseFrame::default();
            terminal
                .draw(|frame| {
                    crate::tui::render::render(frame, &state, &mut layout, &mut feedback);
                })
                .unwrap();
            assert_eq!(
                terminal.backend().buffer()[(1, 2)].symbol(),
                "▶",
                "first frame at {width}x{height}"
            );
            assert_eq!(
                terminal.backend().buffer()[(6, 2)].symbol(),
                "4",
                "reveal the first source row of the selected hunk"
            );
            let viewport = feedback.diff_viewport.unwrap();
            // Drawing stays read-only; feedback applies the exact position that was painted.
            state.apply_diff_viewport(viewport);
            assert_eq!(state.diff().unwrap().body.scroll, viewport.scroll);
            // Leave the hunk before the next resize, including the height-only resize.
            state.handle_key(KeyCode::Down);
            let manual_scroll = state.diff().unwrap().body.scroll;
            terminal
                .draw(|frame| {
                    crate::tui::render::render(frame, &state, &mut layout, &mut feedback);
                })
                .unwrap();
            assert_eq!(
                feedback.diff_viewport.unwrap().scroll,
                manual_scroll,
                "unchanged dimensions preserve manual scrolling"
            );
        }
    }

    #[test]
    fn confirm_keeps_text_background_and_emits_no_diff_viewport_feedback() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = initial_state();
        crate::tui::test_support::enter_hunk_diff(&mut state, "local\nsame\n", "gist\nsame\n");
        state.handle_key(KeyCode::Char(']'));
        state.handle_key(KeyCode::Char('s'));
        let before = state.diff().unwrap().clone();
        let confirm = crate::tui::screens::confirm::build_confirm_vm(&state);
        let crate::tui::screens::confirm::ConfirmBackgroundVm::Diff(background) =
            confirm.background
        else {
            panic!("save preview uses the Diff text background")
        };
        assert!(background.sides.is_none());
        let mut terminal = Terminal::new(TestBackend::new(49, 12)).unwrap();
        let mut feedback = crate::tui::render::RenderFeedback::default();
        let mut layout = crate::tui::MouseFrame::default();
        terminal
            .draw(|frame| {
                crate::tui::render::render(frame, &state, &mut layout, &mut feedback);
            })
            .unwrap();
        assert!(feedback.diff_viewport.is_none());
        assert_eq!(state.diff().unwrap(), &before);
    }

    #[test]
    fn wrapped_rows_scroll_within_a_long_line_and_resize_keeps_the_selected_hunk() {
        let mut state = initial_state();
        let local = format!("{}END\nsame\nsecond local\n", "long ".repeat(80));
        crate::tui::test_support::enter_hunk_diff(&mut state, &local, "short\nsame\nsecond gist\n");
        state.apply_diff_viewport(crate::tui::diff_geometry::DiffViewport {
            dimensions: crate::tui::diff_geometry::Dimensions {
                panes: [27, 27],
                height: 12,
            },
            scroll: 0,
        });
        state.handle_key(KeyCode::Char('w'));
        state.handle_key(KeyCode::PageDown);
        let merge = state.diff().unwrap().merge.as_ref().unwrap();
        let (geometry, _) = state.diff().unwrap().geometry(None, true).unwrap().frame(
            state.diff().unwrap().merge_dimensions.unwrap(),
            state.diff().unwrap().body.scroll,
            merge.selected,
        );
        let row = &geometry.rows()[state.diff().unwrap().body.scroll as usize];
        assert_eq!(
            row.hunk,
            Some(0),
            "page scroll remains inside the wrapped first hunk"
        );
        assert_eq!(
            row.local.as_ref().unwrap().0,
            0,
            "continuation has no repeated source line number"
        );
        state.handle_key(KeyCode::Char('n'));
        state.apply_diff_viewport(crate::tui::diff_geometry::DiffViewport {
            dimensions: crate::tui::diff_geometry::Dimensions {
                panes: [20, 20],
                height: 12,
            },
            scroll: 0,
        });
        state.reveal_hunk();
        state.handle_key(KeyCode::Char(']'));
        let merge = state.diff().unwrap().merge.as_ref().unwrap();
        assert_eq!(merge.gist, "short\nsame\nsecond local\n");
        assert_eq!(
            merge.local, local,
            "resizing never changes the source buffer"
        );
    }

    /// #552: a Config policy change reaches a Diff parked under it; bytes and undo survive.
    #[test]
    fn policy_change_recomputes_parked_diff_without_touching_bytes() {
        let mut state = initial_state();
        state
            .settings
            .adjust(ConfigField::NormalizeLineEndings, true); // default on → off
        crate::tui::test_support::enter_hunk_diff(&mut state, "a\r\nb\r\n", "a\nb\n");
        assert!(!state.diff_identical());
        assert_eq!(
            state.diff().unwrap().merge.as_ref().unwrap().hunk_count(),
            1
        );
        state.handle_key(KeyCode::Char(']'));
        let before = state.diff().unwrap().merge.clone().unwrap();
        let text = state.diff().unwrap().body.text.clone();
        // Config parks the Diff; flipping the policy while parked must still reach it.
        state.nav_stack.push(state.screen.clone());
        state.screen = Screen::Config(Box::default());
        state
            .settings
            .adjust(ConfigField::NormalizeLineEndings, true);
        state.apply_sync_policy_to_diffs();
        state.leave();
        let diff = state.diff().unwrap();
        let merge = diff.merge.as_ref().unwrap();
        assert!(diff.identical && merge.hunk_count() == 0);
        assert_ne!(diff.body.text, text);
        assert_eq!((&merge.local, &merge.gist), (&before.local, &before.gist));
        assert_eq!(merge.can_undo(), before.can_undo());
    }

    /// #552: download is the saved Gist snapshot, never a staged buffer.
    #[test]
    fn download_source_is_the_saved_gist_not_staged() {
        let mut state = initial_state();
        crate::tui::test_support::enter_hunk_diff(&mut state, "local\n", "gist\n");
        state.handle_key(KeyCode::Char(']'));
        let diff = state.diff().unwrap();
        assert_eq!(diff.merge.as_ref().unwrap().gist, "local\n");
        assert_eq!(diff.saved_gist(), Some("gist\n"));
        let mut diff = diff.clone();
        diff.apply_saved(None, Some("local\n".into()));
        assert_eq!(diff.saved_gist(), Some("local\n"));
        assert!(diff.identical);
    }
}
