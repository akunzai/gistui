use crate::domain::PinnedMapping;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Which built-in colour theme to use. Set `theme = "light"` in `config.toml` for
/// light-background terminals; the default `"dark"` suits dark-background terminals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
}

fn default_scan_depth() -> u32 {
    2
}

fn default_diff_context() -> u32 {
    3
}

fn default_skip_dirs() -> Vec<String> {
    [
        "node_modules",
        "target",
        "dist",
        "build",
        ".next",
        "__pycache__",
        "vendor",
        ".cache",
        "venv",
        ".venv",
        "env",
        ".tox",
        "coverage",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub pinned: Vec<PinnedMapping>,
    /// Directory names skipped during recursive local file discovery.
    #[serde(default = "default_skip_dirs")]
    pub skip_dirs: Vec<String>,
    /// Maximum directory depth for recursive local file discovery (r key).
    #[serde(default = "default_scan_depth")]
    pub scan_depth: u32,
    /// Unchanged context lines kept around each change in the diff view (`c` toggles
    /// between this radius and the full file).
    #[serde(default = "default_diff_context")]
    pub diff_context: u32,
    /// Remembered state of the diff view's context toggle: `true` shows the full file,
    /// `false` collapses to `diff_context` lines. Persisted when the user presses `c`.
    #[serde(default)]
    pub diff_show_full: bool,
    /// Built-in colour theme: `"dark"` (default) or `"light"`.
    #[serde(default)]
    pub theme: ThemeChoice,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            pinned: Vec::new(),
            skip_dirs: default_skip_dirs(),
            scan_depth: default_scan_depth(),
            diff_context: default_diff_context(),
            diff_show_full: false,
            theme: ThemeChoice::Dark,
        }
    }
}

pub fn normalize_path(path: &Path) -> Result<PathBuf> {
    let expanded = if let Ok(stripped) = path.strip_prefix("~") {
        dirs::home_dir()
            .context("home directory not found")?
            .join(stripped)
    } else {
        path.to_path_buf()
    };

    if expanded.is_absolute() {
        Ok(expanded)
    } else {
        Ok(std::env::current_dir()?.join(expanded))
    }
}

/// Human-friendly rendering of a local path for TUI display. Replaces the user's
/// home-directory prefix with `~` and normalizes the suffix to `/` separators (so the
/// form is short and consistent on every platform, matching git / gh conventions). It is
/// the display-side symmetry to `normalize_path`'s `~` expansion. Paths outside home, or
/// when no home is known, render unchanged. This thin wrapper feeds the real home into the
/// pure [`display_path_with_home`] so the logic stays unit-testable.
pub fn display_path(path: &Path) -> String {
    display_path_with_home(path, dirs::home_dir().as_deref())
}

/// Pure core of [`display_path`] with the home directory injected, so home handling
/// (including Windows-style separators) is deterministically testable.
pub fn display_path_with_home(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home {
        if let Ok(suffix) = path.strip_prefix(home) {
            if suffix.as_os_str().is_empty() {
                return "~".to_string();
            }
            let suffix = suffix.to_string_lossy().replace('\\', "/");
            return format!("~/{}", suffix.trim_start_matches('/'));
        }
    }
    path.display().to_string()
}

/// Resolve the working directory to operate in. `None` keeps the current directory;
/// `Some(path)` must point at an existing directory (a `~` prefix is expanded) — a missing
/// path or a non-directory is an error, so the caller can report it and exit before the TUI.
pub fn resolve_working_dir(path: Option<PathBuf>) -> Result<PathBuf> {
    match path {
        None => std::env::current_dir().context("could not determine the current directory"),
        Some(path) => {
            let path = normalize_path(&path)?;
            if !path.exists() {
                anyhow::bail!("path does not exist: {}", path.display());
            }
            if !path.is_dir() {
                anyhow::bail!("not a directory: {}", path.display());
            }
            Ok(path)
        }
    }
}

pub fn config_path() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .context("home directory not found")
                .map(|home| home.join(".config"))
        })?;
    Ok(base.join("gistui").join("config.toml"))
}

pub fn load_config(path: &Path) -> Result<AppConfig> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

pub fn save_config(path: &Path, config: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = toml::to_string_pretty(config)?;
    fs::write(path, raw).with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{PinnedMapping, SyncDirection};
    use std::env;
    use std::ffi::OsString;
    use std::sync::Mutex;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    struct EnvVarRestore {
        name: &'static str,
        value: Option<OsString>,
    }

    impl EnvVarRestore {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                value: env::var_os(name),
            }
        }
    }

    impl Drop for EnvVarRestore {
        fn drop(&mut self) {
            match &self.value {
                Some(value) => env::set_var(self.name, value),
                None => env::remove_var(self.name),
            }
        }
    }

    #[test]
    fn missing_config_loads_default() {
        let dir = tempfile::tempdir().unwrap();
        let config = load_config(&dir.path().join("missing.toml")).unwrap();
        assert!(config.pinned.is_empty());
    }

    #[test]
    fn saves_and_loads_pinned_mapping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = AppConfig {
            pinned: vec![PinnedMapping {
                local_path: PathBuf::from("/tmp/settings.json"),
                gist_id: "abc123".into(),
                gist_filename: "settings.json".into(),
                direction: Some(SyncDirection::Upload),
                last_seen_hash: Some("hash".into()),
            }],
            skip_dirs: default_skip_dirs(),
            scan_depth: default_scan_depth(),
            diff_context: default_diff_context(),
            diff_show_full: false,
            theme: ThemeChoice::Dark,
        };

        save_config(&path, &config).unwrap();
        assert_eq!(load_config(&path).unwrap(), config);
    }

    #[test]
    fn config_path_uses_xdg_config_home_when_set() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _restore = EnvVarRestore::new("XDG_CONFIG_HOME");
        let dir = tempfile::tempdir().unwrap();

        env::set_var("XDG_CONFIG_HOME", dir.path());
        let path = config_path().unwrap();

        assert_eq!(path, dir.path().join("gistui").join("config.toml"));
    }

    #[test]
    fn config_path_falls_back_to_home_config_when_xdg_config_home_is_unset() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _restore = EnvVarRestore::new("XDG_CONFIG_HOME");

        env::remove_var("XDG_CONFIG_HOME");
        let path = config_path().unwrap();

        assert_eq!(
            path,
            dirs::home_dir()
                .unwrap()
                .join(".config")
                .join("gistui")
                .join("config.toml")
        );
    }

    #[test]
    fn config_path_falls_back_to_home_config_when_xdg_config_home_is_empty() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _restore = EnvVarRestore::new("XDG_CONFIG_HOME");

        env::set_var("XDG_CONFIG_HOME", "");
        let path = config_path().unwrap();

        assert_eq!(
            path,
            dirs::home_dir()
                .unwrap()
                .join(".config")
                .join("gistui")
                .join("config.toml")
        );
    }

    #[test]
    fn normalize_path_joins_relative_path_to_current_dir() {
        let relative = PathBuf::from("settings.json");
        assert_eq!(
            normalize_path(&relative).unwrap(),
            env::current_dir().unwrap().join(relative)
        );
    }

    #[test]
    fn normalize_path_expands_home_prefix() {
        assert_eq!(
            normalize_path(Path::new("~/settings.json")).unwrap(),
            dirs::home_dir().unwrap().join("settings.json")
        );
    }

    #[test]
    fn normalize_path_preserves_absolute_path() {
        let absolute = PathBuf::from("/tmp/settings.json");
        assert_eq!(normalize_path(&absolute).unwrap(), absolute);
    }

    #[test]
    fn resolve_working_dir_defaults_to_current_dir_when_none() {
        assert_eq!(
            resolve_working_dir(None).unwrap(),
            env::current_dir().unwrap()
        );
    }

    #[test]
    fn resolve_working_dir_accepts_an_existing_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_working_dir(Some(dir.path().to_path_buf())).unwrap(),
            dir.path()
        );
    }

    #[test]
    fn resolve_working_dir_rejects_a_missing_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        let err = resolve_working_dir(Some(missing)).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn resolve_working_dir_rejects_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "x").unwrap();
        let err = resolve_working_dir(Some(file)).unwrap_err();
        assert!(err.to_string().contains("not a directory"));
    }

    #[test]
    fn display_path_with_home_is_tilde_for_home_root() {
        let home = Path::new("/Users/alice");
        assert_eq!(display_path_with_home(home, Some(home)), "~");
    }

    #[test]
    fn display_path_with_home_shortens_path_under_home() {
        let home = Path::new("/Users/alice");
        let p = home.join("code").join("gistui");
        assert_eq!(display_path_with_home(&p, Some(home)), "~/code/gistui");
    }

    #[test]
    fn display_path_with_home_preserves_spaces() {
        let home = Path::new("/Users/alice");
        let p = home.join("My Docs").join("a b.txt");
        assert_eq!(display_path_with_home(&p, Some(home)), "~/My Docs/a b.txt");
    }

    #[test]
    fn display_path_with_home_uses_forward_slashes_in_suffix() {
        // On Windows the stripped suffix carries `\` separators; the display form
        // normalizes them to `/`. We exercise the replacement portably by placing a
        // literal backslash inside a single path component.
        let home = Path::new("/Users/alice");
        let p = home.join(r"sub\dir");
        assert_eq!(display_path_with_home(&p, Some(home)), "~/sub/dir");
    }

    #[test]
    fn display_path_with_home_keeps_paths_outside_home() {
        let home = Path::new("/Users/alice");
        let p = Path::new("/etc/hosts");
        assert_eq!(display_path_with_home(p, Some(home)), "/etc/hosts");
    }

    #[test]
    fn display_path_with_home_falls_back_without_home() {
        let p = Path::new("/Users/alice/code");
        assert_eq!(display_path_with_home(p, None), "/Users/alice/code");
    }

    #[test]
    fn display_path_matches_helper_with_real_home() {
        // The thin wrapper just feeds dirs::home_dir() into the pure helper.
        if let Some(home) = dirs::home_dir() {
            let p = home.join("code").join("gistui");
            assert_eq!(display_path(&p), display_path_with_home(&p, Some(&home)));
            assert_eq!(display_path(&p), "~/code/gistui");
        }
    }
}
