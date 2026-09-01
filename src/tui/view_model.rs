//! Pure presentation seam: `AppState` (+ pin-sync cache) → immutable view models.
//!
//! The draw path builds a [`ViewModel`] once per frame and paints from it for every screen.
//! Builders never touch the filesystem or network (issues #241 / #250).

use super::screens::{lookup, ScreenVm};
use super::AppState;

/// Full-frame presentation contract produced by [`build_view_model`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ViewModel {
    pub chrome: ChromeVm,
    pub screen: ScreenVm,
}

/// Cross-screen chrome facts shared by every body (top bar / overlays).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChromeVm {
    /// Whether mouse hit targets (close button, list hits) should be recorded.
    pub mouse_enabled: bool,
    /// Background task overlay message, if any.
    pub bg_task_msg: Option<String>,
    pub spinner_frame: usize,
}

/// Pure chrome facts shared across screens (and palette backgrounds).
pub(crate) fn build_chrome(state: &AppState) -> ChromeVm {
    ChromeVm {
        mouse_enabled: state.settings.mouse_enabled(),
        bg_task_msg: state.bg_task_msg.clone(),
        spinner_frame: state.spinner_frame,
    }
}

/// Pure: map app state (+ pin sync cache) into a view model. No FS / network / mutation.
pub(crate) fn build_view_model(state: &AppState) -> ViewModel {
    ViewModel {
        chrome: build_chrome(state),
        screen: (lookup(&state.screen).build_vm)(state),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::screens::list::ListFooterVm;
    use crate::tui::{initial_state, Screen};

    #[test]
    fn key_dense_screens_expose_contextual_action_hints() {
        // One of two smoke tests kept here: it is the only assertion that three different
        // screens all reach their contextual hints through the full `build_view_model`
        // path, so it stays whole rather than being split across three modules.
        let mut state = initial_state();
        let ScreenVm::List(list) = build_view_model(&state).screen else {
            panic!("expected List");
        };
        let ListFooterVm::Hints { text } = list.footer else {
            panic!("expected List hints");
        };
        assert!(text.contains("Enter diff") && text.contains("d download"));

        state.screen = Screen::Pins(Box::default());
        let ScreenVm::Pins(pins) = build_view_model(&state).screen else {
            panic!("expected Pins");
        };
        assert!(pins.footer.contains("s sync") && pins.footer.contains("x unpin"));
        assert!(
            pins.footer_title.contains("✓ synced") && pins.footer_title.contains("↓ remote newer")
        );

        state.screen = Screen::Gists(Box::default());
        let ScreenVm::Gists(gists) = build_view_model(&state).screen else {
            panic!("expected Gists");
        };
        assert!(gists.footer.contains("Enter detail") && gists.footer.contains("H revisions"));
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
