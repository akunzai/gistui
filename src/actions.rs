use crate::domain::GistFile;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    Upload {
        local_path: PathBuf,
        target: GistFile,
    },
    Download {
        source: GistFile,
        local_path: PathBuf,
    },
    Create {
        local_path: PathBuf,
        filename: String,
        public: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedAction {
    pub action: PendingAction,
}

pub fn confirm_action(
    action: Option<PendingAction>,
    diff_previewed: bool,
) -> Option<ConfirmedAction> {
    if diff_previewed {
        action.map(|action| ConfirmedAction { action })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gist_file() -> GistFile {
        GistFile {
            gist_id: "abc123".into(),
            description: "config".into(),
            filename: "settings.json".into(),
            public: false,
            updated_at: "2026-06-08T00:00:00Z".into(),
        }
    }

    #[test]
    fn cannot_confirm_without_diff_preview() {
        let action = PendingAction::Upload {
            local_path: PathBuf::from("/tmp/settings.json"),
            target: gist_file(),
        };

        assert_eq!(confirm_action(Some(action), false), None);
    }

    #[test]
    fn can_confirm_after_diff_preview() {
        let action = PendingAction::Download {
            source: gist_file(),
            local_path: PathBuf::from("/tmp/settings.json"),
        };

        assert!(confirm_action(Some(action), true).is_some());
    }

    #[test]
    fn no_action_yields_none_even_after_preview() {
        assert_eq!(confirm_action(None, true), None);
    }
}
