use crate::domain::PinnedMapping;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub pinned: Vec<PinnedMapping>,
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
}
