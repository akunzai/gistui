//! Background "is there a newer release?" check shown on startup.
//!
//! Reuses the [`crate::upgrade`] release client and version helpers. The network fetch is a
//! thin IO boundary; the comparison/throttle/hint logic is pure and unit-tested. The check
//! fails silently (offline, rate-limited, parse error) and is throttled to once per day.

use crate::upgrade::{normalize_tag, upgrade_hint, version_cmp, InstallMethod, ReleaseClient};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Minimum gap between startup checks (once per day), to be polite to the GitHub API
/// (unauthenticated `api.github.com` is 60 requests/hour/IP).
pub const CHECK_INTERVAL_SECS: u64 = 86_400;

/// Outcome of an attempted background check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCheckOutcome {
    /// A newer release is available (normalized version, e.g. `"0.14.0"`).
    Newer(String),
    /// Checked successfully; already on the latest version.
    UpToDate,
    /// The check could not complete (offline / rate-limited / parse error). Stay silent and
    /// retry on the next launch — do NOT record it against the throttle.
    Failed,
}

/// Persisted throttle state (`$XDG_CACHE_HOME/gistui/update_check.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCheckState {
    /// Unix seconds of the last completed check.
    #[serde(default)]
    pub last_check: u64,
    /// Latest release tag seen at the last check (normalized); lets the hint persist across
    /// launches within the throttle window without re-hitting the network.
    #[serde(default)]
    pub latest_seen: String,
}

pub fn state_path() -> Result<PathBuf> {
    let dir = dirs::cache_dir().context("locate cache directory")?;
    Ok(dir.join("gistui").join("update_check.json"))
}

pub fn load_state(path: &Path) -> UpdateCheckState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_state(path: &Path, state: &UpdateCheckState) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(path, json);
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Whether enough time has passed since `last_check` to run another network check.
pub fn should_check(last_check: u64, now: u64) -> bool {
    now.saturating_sub(last_check) >= CHECK_INTERVAL_SECS
}

/// `Some(normalized)` when `latest` is a strictly newer version than `current`, else `None`.
pub fn is_newer(latest: &str, current: &str) -> Option<String> {
    let latest = normalize_tag(latest);
    if latest.is_empty() {
        return None;
    }
    (version_cmp(&latest, current) == Ordering::Greater).then_some(latest)
}

/// Footer hint for an available update, tailored to how the binary was installed.
pub fn update_hint(latest: &str, method: &InstallMethod) -> String {
    let cmd = upgrade_hint(method)
        .map(str::to_string)
        .or_else(|| match method {
            InstallMethod::Standalone => Some("gistui --upgrade".to_string()),
            _ => None,
        });
    match cmd {
        Some(cmd) => format!("⬆ v{latest} available — run {cmd}"),
        None => format!("⬆ v{latest} available"),
    }
}

/// Run the network check against `client`, comparing the latest tag with `current`.
/// Thin IO boundary; pure classification lives in [`is_newer`].
pub fn check(client: &impl ReleaseClient, current: &str) -> UpdateCheckOutcome {
    match client.fetch_latest_tag() {
        Ok(tag) => match is_newer(&tag, current) {
            Some(v) => UpdateCheckOutcome::Newer(v),
            None => UpdateCheckOutcome::UpToDate,
        },
        Err(_) => UpdateCheckOutcome::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_check_respects_the_daily_interval() {
        assert!(should_check(0, CHECK_INTERVAL_SECS));
        assert!(should_check(0, CHECK_INTERVAL_SECS + 10));
        assert!(!should_check(100, 100));
        assert!(!should_check(100, 100 + CHECK_INTERVAL_SECS - 1));
    }

    #[test]
    fn is_newer_only_for_strictly_greater() {
        assert_eq!(is_newer("v0.14.0", "0.13.0"), Some("0.14.0".to_string()));
        assert_eq!(is_newer("0.13.0", "0.13.0"), None);
        assert_eq!(is_newer("v0.12.0", "0.13.0"), None);
        assert_eq!(is_newer("", "0.13.0"), None);
    }

    #[test]
    fn update_hint_is_install_method_aware() {
        assert_eq!(
            update_hint("0.14.0", &InstallMethod::Homebrew),
            "⬆ v0.14.0 available — run brew upgrade gistui"
        );
        assert_eq!(
            update_hint("0.14.0", &InstallMethod::Standalone),
            "⬆ v0.14.0 available — run gistui --upgrade"
        );
        assert_eq!(
            update_hint("0.14.0", &InstallMethod::Refuse { hint: "x".into() }),
            "⬆ v0.14.0 available"
        );
    }

    #[test]
    fn check_classifies_outcomes_via_a_fake_client() {
        struct Fake {
            tag: Option<&'static str>,
        }
        impl ReleaseClient for Fake {
            fn fetch_latest_tag(&self) -> Result<String> {
                self.tag
                    .map(str::to_string)
                    .ok_or_else(|| anyhow::anyhow!("offline"))
            }
            fn download(&self, _url: &str) -> Result<Vec<u8>> {
                unreachable!("update check never downloads")
            }
        }
        assert_eq!(
            check(
                &Fake {
                    tag: Some("v0.14.0")
                },
                "0.13.0"
            ),
            UpdateCheckOutcome::Newer("0.14.0".to_string())
        );
        assert_eq!(
            check(
                &Fake {
                    tag: Some("v0.13.0")
                },
                "0.13.0"
            ),
            UpdateCheckOutcome::UpToDate
        );
        assert_eq!(
            check(&Fake { tag: None }, "0.13.0"),
            UpdateCheckOutcome::Failed
        );
    }

    #[test]
    fn state_round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("gistui-uctest-{}", std::process::id()));
        let path = dir.join("update_check.json");
        let state = UpdateCheckState {
            last_check: 12345,
            latest_seen: "0.14.0".to_string(),
        };
        save_state(&path, &state);
        assert_eq!(load_state(&path), state);
        // Missing file loads the default.
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load_state(&path), UpdateCheckState::default());
    }
}
