//! Shared fixtures for the `tui` test suites. Test-only: every screen module's
//! `mod tests` builds its `AppState` from here instead of re-deriving one.

use super::*;
use std::path::PathBuf;

pub(super) fn gist_file_ref(gist_id: &str, filename: &str) -> GistFileRef {
    GistFileRef::new(gist_id, filename, None)
}

/// Test helper: mutable HelpState when on Help (creates empty Help if needed).
pub(super) fn help_mut(state: &mut AppState) -> &mut HelpState {
    if !state.screen.is_help() {
        state.screen = Screen::Help(Box::default());
    }
    match &mut state.screen {
        Screen::Help(h) => h.as_mut(),
        _ => unreachable!(),
    }
}

pub(super) fn help_ref(state: &AppState) -> &HelpState {
    state.help().expect("expected Screen::Help")
}

pub(super) fn revision_ref(state: &AppState) -> &RevisionState {
    state.revision().expect("expected Screen::Revisions")
}

pub(super) fn pins_mut(state: &mut AppState) -> &mut PinsState {
    if !state.screen.is_pins() {
        state.screen = Screen::Pins(Box::default());
    }
    state.pins_mut().expect("expected Screen::Pins")
}

pub(super) fn pins_ref(state: &AppState) -> &PinsState {
    state.pins().expect("expected Screen::Pins")
}

pub(super) fn gists_mut(state: &mut AppState) -> &mut GistsManagerState {
    if !state.screen.is_gists() {
        state.screen = Screen::Gists(Box::default());
    }
    state.gist_manager_mut().expect("expected Screen::Gists")
}

pub(super) fn detail_mut(state: &mut AppState) -> &mut DetailState {
    if !state.screen.is_gist_detail() {
        state.screen = Screen::GistDetail(Box::default());
    }
    state.detail_mut().expect("expected Screen::GistDetail")
}

/// Open Confirm with the given action (keeps existing body text when already on Diff/Confirm).
/// Mirrors production `enter_confirm`/`enter_confirm_from_diff` for synchronous tests.
pub(super) fn set_pending(state: &mut AppState, action: PendingAction) {
    if state.screen.is_confirm() {
        if let Some(c) = state.confirm_mut() {
            c.action = action;
        }
        return;
    }
    if state.screen.is_diff() {
        state.enter_confirm_from_diff(action);
        return;
    }
    state.enter_confirm(action, String::new());
}

pub(super) fn set_diff_body(state: &mut AppState, text: impl Into<String>) {
    let text = text.into();
    if matches!(state.screen, Screen::Diff(_) | Screen::Confirm(_)) {
        if let Some(body) = state.scroll_body_mut() {
            body.text = text;
            return;
        }
    }
    // Ensure a Diff payload exists for tests that set body before navigating.
    state.screen = Screen::Diff(Box::new(DiffState {
        body: ScrollBody {
            text,
            ..ScrollBody::default()
        },
        ..DiffState::default()
    }));
}

pub(super) fn set_diff_scroll(state: &mut AppState, scroll: u16) {
    if matches!(state.screen, Screen::Diff(_) | Screen::Confirm(_)) {
        if let Some(body) = state.scroll_body_mut() {
            body.scroll = scroll;
            return;
        }
    }
    state.screen = Screen::Diff(Box::new(DiffState {
        body: ScrollBody {
            scroll,
            ..ScrollBody::default()
        },
        ..DiffState::default()
    }));
}

pub(super) fn state_with_gists() -> AppState {
    let mut state = initial_state();
    state.gists = vec![
        GistFile {
            description: "demo".into(),
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
            ..GistFile::fixture("g1", "a.txt")
        },
        GistFile {
            description: "demo".into(),
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
            ..GistFile::fixture("g1", "b.txt")
        },
    ];
    gists_mut(&mut state).cursor.index = 0;
    state
}

pub(super) fn state_with_local_paths(paths: &[&str]) -> AppState {
    let mut state = initial_state();
    state.cwd = PathBuf::from("/cwd");
    state.locals = paths
        .iter()
        .map(|p| LocalCandidate {
            path: PathBuf::from(p),
            pinned: false,
            modified: None,
        })
        .collect();
    state
}

pub(super) fn list_state_with_matches() -> AppState {
    let mut state = initial_state();
    state.locals = vec![
        LocalCandidate {
            path: std::path::PathBuf::from("/cwd/settings.json"),
            pinned: false,
            modified: None,
        },
        LocalCandidate {
            path: std::path::PathBuf::from("/cwd/other.txt"),
            pinned: false,
            modified: None,
        },
    ];
    state.gists = vec![
        GistFile {
            description: "Zed".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "settings.json")
        },
        GistFile {
            description: "misc".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("b", "zzz.txt")
        },
    ];
    state.local_index = 0;
    state.gist_index = 0;
    state
}

pub(super) fn state_with_two_gists() -> AppState {
    let mut state = initial_state();
    state.gists = vec![
        GistFile {
            description: "My Ghostty config".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "config.ghostty")
        },
        GistFile {
            description: "SSH config".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("b", "ssh_config")
        },
    ];
    state.focus = FocusPane::Gist;
    state
}

pub(super) fn state_with_selection() -> AppState {
    let mut state = initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("/tmp/settings.json"),
        pinned: false,
        modified: None,
    }];
    state.gists = vec![GistFile {
        description: "settings".into(),
        updated_at: "x".into(),
        created_at: "x".into(),
        ..GistFile::fixture("a", "settings.json")
    }];
    state.focus = FocusPane::Gist;
    state
}

pub(super) fn state_ready_to_create() -> AppState {
    let mut state = initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("/tmp/config.toml"),
        pinned: false,
        modified: None,
    }];
    state
}

pub(super) fn pins_state_with_long_home_path() -> AppState {
    let mut state = initial_state();
    state.screen = Screen::Pins(Box::default());
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
    state.pinned = vec![PinnedMapping {
        local_path: home.join("code/very/deeply/nested/project/config.json"),
        gist_id: "g1".into(),
        gist_filename: "config.json".into(),
        direction: None,
        last_seen_hash: None,
    }];
    pins_mut(&mut state).cursor.index = 0;
    state
}
