//! Apply handlers for a **gist mutation** — a change to a gist itself, whose async
//! outcome belongs to no single screen (issue #383). List, Gists, GistDetail, and
//! Confirm can all launch one.

use super::bg::{record_pin_sync, LoopFlow};
use super::{AppState, Screen};
use std::path::PathBuf;

fn apply(
    state: &mut AppState,
    result: Result<(), String>,
    verb: &str,
    on_ok: impl FnOnce(&mut AppState) -> String,
) -> LoopFlow {
    match result {
        Ok(()) => {
            let msg = on_ok(state);
            state.set_status(msg);
            state.gist_list_stale = true;
        }
        Err(error) => state.set_status(format!("{verb} failed: {error}")),
    }
    LoopFlow::Proceed
}

/// `UploadReplace` outcome: commit the pin-sync record and return to wherever the upload
/// was initiated from, then re-fetch the gist list.
pub(crate) fn on_upload_replace(
    state: &mut AppState,
    result: Result<(), String>,
    file: crate::domain::GistFileRef,
) -> LoopFlow {
    apply(state, result, "upload", |state| {
        state.gist_content_cache.remove(&file.cache_key());
        if let Some(local_path) = state.upload_local_path() {
            let content = state.content_to_upload();
            record_pin_sync(
                state,
                &local_path,
                &file.gist_id,
                &file.filename,
                &content,
                Some(crate::domain::SyncDirection::Upload),
            );
        }
        // Return to wherever this upload was initiated from (List, or Pins
        // for a pin push) instead of always snapping to List.
        state.leave();
        format!("Uploaded {} to gist {}", file.filename, file.gist_id)
    })
}

/// `CreateGist` outcome. Bespoke: the Err arm resets the screen, unlike [`apply`].
pub(crate) fn on_create_gist(
    state: &mut AppState,
    result: Result<(), String>,
    local_path: PathBuf,
    public: bool,
) -> LoopFlow {
    match result {
        Ok(()) => {
            let visibility = if public { "public" } else { "secret" };
            state.set_status(format!(
                "Created {} gist from {}",
                visibility,
                crate::config::display_path(&local_path)
            ));
            state.description_input.clear();
            state.back_to_list();
            state.gist_list_stale = true;
        }
        Err(error) => {
            state.set_status(format!("create failed: {error}"));
            state.screen = Screen::List;
            state.description_input.clear();
        }
    }
    LoopFlow::Proceed
}

/// `DeleteGist` outcome.
pub(crate) fn on_delete_gist(
    state: &mut AppState,
    result: Result<(), String>,
    gist_id: String,
) -> LoopFlow {
    apply(state, result, "delete", |_| {
        format!("Deleted gist {gist_id}")
    })
}

/// `RemoveFile` outcome.
pub(crate) fn on_remove_file(
    state: &mut AppState,
    result: Result<(), String>,
    gist_id: String,
    filename: String,
) -> LoopFlow {
    apply(state, result, "remove", |state| {
        state
            .gist_content_cache
            .remove(&(gist_id.clone(), filename.clone()));
        format!("Removed {filename} from gist {gist_id}")
    })
}

/// `ApplyDescription` outcome.
pub(crate) fn on_apply_description(
    state: &mut AppState,
    result: Result<(), String>,
    gist_id: String,
) -> LoopFlow {
    apply(state, result, "description update", |_| {
        format!("Updated description for gist {gist_id}")
    })
}

/// `CompactGist` outcome.
pub(crate) fn on_compact_gist(
    state: &mut AppState,
    result: Result<(), String>,
    label: String,
    count: usize,
) -> LoopFlow {
    apply(state, result, "compact", |_| {
        format!("Compacted \"{label}\" ({count} → 1 revision)")
    })
}

/// `GistStarToggle` outcome.
pub(crate) fn on_gist_star_toggle(
    state: &mut AppState,
    result: Result<(), String>,
    gist_id: String,
    starred: bool,
) -> LoopFlow {
    apply(state, result, "star toggle", |state| {
        if starred {
            state.starred_gist_ids.insert(gist_id.clone());
            format!("starred {gist_id}")
        } else {
            state.starred_gist_ids.remove(&gist_id);
            format!("unstarred {gist_id}")
        }
    })
}

/// `ForkGist` outcome.
pub(crate) fn on_fork_gist(
    state: &mut AppState,
    result: Result<(), String>,
    gist_id: String,
) -> LoopFlow {
    apply(state, result, "fork", |_| {
        format!("forked {gist_id} into your account")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{PinnedMapping, SyncDirection};
    use crate::tui::test_support::gist_file_ref;
    use crate::tui::*;

    #[test]
    fn on_upload_replace_err_sets_status() {
        let mut state = initial_state();

        on_upload_replace(&mut state, Err("boom".into()), gist_file_ref("g1", "a.txt"));

        assert_eq!(state.status.as_deref(), Some("upload failed: boom"));
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_upload_replace_ok_records_pin_and_marks_list_stale() {
        let _guard = crate::config::tests::ENV_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", dir.path());

        let local_path = dir.path().join("a.txt");
        std::fs::write(&local_path, "hello").unwrap();
        let mapping = PinnedMapping {
            local_path: local_path.clone(),
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: None,
        };
        let mut config = crate::config::AppConfig::default();
        config.pinned.push(mapping.clone());
        let path = crate::config::config_path().unwrap();
        crate::config::save_config(&path, &config).unwrap();

        let mut state = initial_state();
        state.cwd = dir.path().to_path_buf();
        state.pinned = vec![mapping];
        state
            .gist_content_cache
            .insert(("g1".into(), "a.txt".into()), "stale".into());
        state.enter_confirm(
            PendingAction::Upload {
                gist_id: "g1".into(),
                filename: "a.txt".into(),
                local_path: local_path.clone(),
            },
            String::new(),
        );
        state.upload.original_content = "hello".into();

        on_upload_replace(&mut state, Ok(()), gist_file_ref("g1", "a.txt"));

        assert!(state.gist_list_stale);
        assert_eq!(state.status.as_deref(), Some("Uploaded a.txt to gist g1"));
        assert!(state
            .gist_content_cache
            .get(&("g1".into(), "a.txt".into()))
            .is_none());
        assert_eq!(state.pinned[0].direction, Some(SyncDirection::Upload));
        assert_eq!(
            state.pinned[0].last_seen_hash.as_deref(),
            Some(crate::domain::sha256_hex(b"hello").as_str())
        );

        match prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    #[test]
    fn on_create_gist_err_resets_screen() {
        let mut state = initial_state();
        state.description_input.set("desc");

        on_create_gist(&mut state, Err("boom".into()), PathBuf::from("a.txt"), true);

        assert_eq!(state.status.as_deref(), Some("create failed: boom"));
        assert!(matches!(state.screen, Screen::List));
        assert!(state.description_input.is_empty());
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_create_gist_ok_returns_to_list_and_marks_stale() {
        let mut state = initial_state();
        state.description_input.set("desc");
        state.screen = Screen::Confirm(Box::default());

        on_create_gist(&mut state, Ok(()), PathBuf::from("a.txt"), true);

        assert!(state.gist_list_stale);
        assert_eq!(
            state.status.as_deref(),
            Some("Created public gist from a.txt")
        );
        assert!(matches!(state.screen, Screen::List));
        assert!(state.description_input.is_empty());
    }

    #[test]
    fn on_delete_gist_err_sets_status() {
        let mut state = initial_state();

        on_delete_gist(&mut state, Err("boom".into()), "g1".into());

        assert_eq!(state.status.as_deref(), Some("delete failed: boom"));
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_delete_gist_ok_marks_list_stale() {
        let mut state = initial_state();

        on_delete_gist(&mut state, Ok(()), "g1".into());

        assert!(state.gist_list_stale);
        assert_eq!(state.status.as_deref(), Some("Deleted gist g1"));
    }

    #[test]
    fn on_remove_file_err_sets_status() {
        let mut state = initial_state();

        on_remove_file(&mut state, Err("boom".into()), "g1".into(), "a.txt".into());

        assert_eq!(state.status.as_deref(), Some("remove failed: boom"));
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_remove_file_ok_drops_cache_and_marks_list_stale() {
        let mut state = initial_state();
        state
            .gist_content_cache
            .insert(("g1".into(), "a.txt".into()), "body".into());

        on_remove_file(&mut state, Ok(()), "g1".into(), "a.txt".into());

        assert!(state.gist_list_stale);
        assert_eq!(state.status.as_deref(), Some("Removed a.txt from gist g1"));
        assert!(state
            .gist_content_cache
            .get(&("g1".into(), "a.txt".into()))
            .is_none());
    }

    #[test]
    fn on_apply_description_err_sets_status() {
        let mut state = initial_state();

        on_apply_description(&mut state, Err("boom".into()), "g1".into());

        assert_eq!(
            state.status.as_deref(),
            Some("description update failed: boom")
        );
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_apply_description_ok_marks_list_stale() {
        let mut state = initial_state();

        on_apply_description(&mut state, Ok(()), "g1".into());

        assert!(state.gist_list_stale);
        assert_eq!(
            state.status.as_deref(),
            Some("Updated description for gist g1")
        );
    }

    #[test]
    fn on_compact_gist_err_sets_status() {
        let mut state = initial_state();

        on_compact_gist(&mut state, Err("boom".into()), "demo".into(), 3);

        assert_eq!(state.status.as_deref(), Some("compact failed: boom"));
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_compact_gist_ok_marks_list_stale() {
        let mut state = initial_state();

        on_compact_gist(&mut state, Ok(()), "demo".into(), 3);

        assert!(state.gist_list_stale);
        assert_eq!(
            state.status.as_deref(),
            Some("Compacted \"demo\" (3 → 1 revision)")
        );
    }

    #[test]
    fn on_gist_star_toggle_err_sets_status() {
        let mut state = initial_state();

        on_gist_star_toggle(&mut state, Err("boom".into()), "g1".into(), true);

        assert_eq!(state.status.as_deref(), Some("star toggle failed: boom"));
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_gist_star_toggle_ok_stars_and_marks_list_stale() {
        let mut state = initial_state();

        on_gist_star_toggle(&mut state, Ok(()), "g1".into(), true);

        assert!(state.gist_list_stale);
        assert!(state.starred_gist_ids.contains("g1"));
        assert_eq!(state.status.as_deref(), Some("starred g1"));
    }

    #[test]
    fn on_fork_gist_err_sets_status() {
        let mut state = initial_state();

        on_fork_gist(&mut state, Err("boom".into()), "g1".into());

        assert_eq!(state.status.as_deref(), Some("fork failed: boom"));
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_fork_gist_ok_marks_list_stale() {
        let mut state = initial_state();

        on_fork_gist(&mut state, Ok(()), "g1".into());

        assert!(state.gist_list_stale);
        assert_eq!(state.status.as_deref(), Some("forked g1 into your account"));
    }
}
