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
    /// The Settings-screen preferences, stored as top-level keys.
    #[serde(flatten)]
    pub prefs: Preferences,
}

/// The user preferences the Settings screen edits. Their defaults live only in
/// [`Preferences::default`]. A key missing from `config.toml` takes that default, and a
/// value equal to it is left out when the file is saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// Maximum directory depth for recursive local file discovery (r key).
    pub scan_depth: u32,
    /// Unchanged context lines kept around each change in the diff view (`c` toggles
    /// between this radius and the full file).
    pub diff_context: u32,
    /// Remembered state of the diff view's context toggle: `true` shows the full file,
    /// `false` collapses to `diff_context` lines. Persisted when the user presses `c`.
    pub diff_show_full: bool,
    /// Built-in colour theme: `"dark"` (default) or `"light"`.
    pub theme: ThemeChoice,
    /// Enable mouse support (wheel scroll, click-to-focus/select). Default `true`;
    /// set `false` to opt out (the `--no-mouse` CLI flag also forces it off).
    pub mouse: bool,
    /// Check GitHub on startup for a newer release and show a hint if one exists. Default
    /// `true`; set `false` to opt out (the `--no-update-check` CLI flag also forces it off).
    pub check_updates: bool,
    /// Treat a difference that is *only* a file-final newline as "no difference": the diff
    /// view hides the phantom change and the overwrite-confirm gate counts the sides as
    /// identical. Default `true`; set `false` for strict, byte-exact diffs.
    pub ignore_trailing_newline: bool,
    /// Rewrite CRLF/lone-CR line endings to LF in the bytes actually sent/written on
    /// upload and download. Unlike `ignore_trailing_newline`, this changes real content,
    /// not just the diff view. Default `true`; set `false` to preserve a file's original
    /// line-ending style through upload/download untouched.
    pub normalize_line_endings: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            scan_depth: 2,
            diff_context: 3,
            diff_show_full: false,
            theme: ThemeChoice::Dark,
            mouse: true,
            check_updates: true,
            ignore_trailing_newline: true,
            normalize_line_endings: true,
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            pinned: Vec::new(),
            skip_dirs: default_skip_dirs(),
            prefs: Preferences::default(),
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
    parse_config(&raw).with_context(|| format!("parse {}", path.display()))
}

fn parse_config(raw: &str) -> Result<AppConfig> {
    let mut config: AppConfig = toml::from_str(raw)?;
    default_gist_filenames(&mut config.pinned)?;
    Ok(config)
}

/// A hand-written pin may leave `gist_filename` out; it then names the gist file after the
/// local file (issue #469).
fn default_gist_filenames(pinned: &mut [PinnedMapping]) -> Result<()> {
    for pin in pinned.iter_mut().filter(|p| p.gist_filename.is_empty()) {
        let Some(name) = pin.local_path.file_name().and_then(|n| n.to_str()) else {
            anyhow::bail!(
                "pinned entry {} has no gist_filename, and no file name to default it to",
                pin.local_path.display()
            );
        };
        pin.gist_filename = name.to_string();
    }
    Ok(())
}

/// Save `config` to `path`, editing the existing file rather than rewriting it (issue #484).
///
/// A key whose value this save changes is written, or removed when it changes back to its
/// default. Every other line is left as the user wrote it: comments, key order, keys gistui
/// doesn't know, and values that happen to equal a default. `[[pinned]]` entries are matched
/// by their key and updated in place. A file that doesn't exist yet is written fresh with
/// only the non-default keys.
pub fn save_config(path: &Path, config: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = match fs::read_to_string(path) {
        Ok(existing) => edit_saved_toml(&existing, config)
            .with_context(|| format!("update {}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => to_saved_toml(config)?,
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    fs::write(path, text).with_context(|| format!("write {}", path.display()))
}

/// A fresh file: every key whose value differs from the default, plus `pinned` (always
/// present). A value equal to its default is left out, so a later change of default reaches
/// users who never set it.
fn to_saved_toml(config: &AppConfig) -> Result<String> {
    let mut table = toml::Table::try_from(config)?;
    let defaults = toml::Table::try_from(AppConfig::default())?;
    table.retain(|key, value| key == "pinned" || defaults.get(key) != Some(value));
    Ok(toml::to_string_pretty(&table)?)
}

/// `existing` with only what differs between it and `config` changed.
fn edit_saved_toml(existing: &str, config: &AppConfig) -> Result<String> {
    let mut doc: toml_edit::DocumentMut = existing.parse()?;
    let before = toml::Table::try_from(parse_config(existing)?)?;
    let after = toml::Table::try_from(config)?;
    let defaults = toml::Table::try_from(AppConfig::default())?;

    for (key, value) in after.iter().filter(|(key, _)| *key != "pinned") {
        if before.get(key) == Some(value) {
            continue;
        }
        if defaults.get(key) == Some(value) {
            remove_keeping_comments(doc.as_table_mut(), key);
        } else {
            set_keeping_comments(doc.as_table_mut(), key, edit_value(value)?);
        }
    }
    if before.get("pinned") != after.get("pinned") {
        edit_pins(&mut doc, &parse_config(existing)?.pinned, &config.pinned)?;
    }
    // `toml_edit` writes LF; a CRLF file (the norm on Windows) stays CRLF.
    let text = doc.to_string();
    Ok(if existing.contains("\r\n") {
        text.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        text
    })
}

fn edit_value(value: &toml::Value) -> Result<toml_edit::Value> {
    Ok(value.to_string().parse()?)
}

/// Set `key` to `value`, keeping a trailing comment on the line it replaces.
fn set_keeping_comments(table: &mut toml_edit::Table, key: &str, mut value: toml_edit::Value) {
    if let Some(old) = table.get(key).and_then(toml_edit::Item::as_value) {
        *value.decor_mut() = old.decor().clone();
    }
    table[key] = toml_edit::Item::Value(value);
}

/// Remove `key`. The comment lines above it move to the key that follows, so a file header
/// or section comment isn't lost with the line.
fn remove_keeping_comments(table: &mut toml_edit::Table, key: &str) {
    let prefix = table
        .key(key)
        .and_then(|k| k.leaf_decor().prefix())
        .and_then(|p| p.as_str())
        .filter(|p| !p.trim().is_empty())
        .map(str::to_string);
    let next = {
        let mut keys = table.iter().map(|(k, _)| k.to_string());
        keys.by_ref().find(|k| k == key);
        keys.next()
    };
    table.remove(key);
    if let (Some(prefix), Some(next)) = (prefix, next) {
        if let Some(mut next_key) = table.key_mut(&next) {
            let kept = next_key
                .leaf_decor()
                .prefix()
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string();
            next_key
                .leaf_decor_mut()
                .set_prefix(format!("{prefix}{kept}"));
        }
    }
}

/// Rebuild `[[pinned]]` from `new`, reusing the file's own table (and so its comments) for
/// every pin that is still there, with only its changed fields rewritten.
fn edit_pins(
    doc: &mut toml_edit::DocumentMut,
    old: &[PinnedMapping],
    new: &[PinnedMapping],
) -> Result<()> {
    let identity = |p: &PinnedMapping| {
        (
            p.local_path.clone(),
            p.gist_id.clone(),
            p.gist_filename.clone(),
        )
    };
    let mut existing: Vec<Option<toml_edit::Table>> = match doc.get("pinned") {
        Some(toml_edit::Item::ArrayOfTables(tables)) => tables.iter().cloned().map(Some).collect(),
        _ => Vec::new(),
    };
    let mut pinned = toml_edit::ArrayOfTables::new();
    for pin in new {
        let fields = toml::Table::try_from(pin)?;
        let reused = old
            .iter()
            .zip(existing.iter_mut())
            .find(|(o, t)| t.is_some() && identity(o) == identity(pin))
            .and_then(|(o, t)| Some((o, t.take()?)));
        let table = match reused {
            Some((old_pin, mut table)) => {
                let old_fields = toml::Table::try_from(old_pin)?;
                for (key, _) in old_fields.iter().filter(|(k, _)| !fields.contains_key(*k)) {
                    remove_keeping_comments(&mut table, key);
                }
                for (key, value) in &fields {
                    if old_fields.get(key) != Some(value) {
                        set_keeping_comments(&mut table, key, edit_value(value)?);
                    }
                }
                table
            }
            None => {
                let mut table = toml_edit::Table::new();
                for (key, value) in &fields {
                    table[key.as_str()] = toml_edit::value(edit_value(value)?);
                }
                table
            }
        };
        pinned.push(table);
    }
    doc["pinned"] = toml_edit::Item::ArrayOfTables(pinned);
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::domain::{PinnedMapping, SyncDirection};
    use std::env;
    use std::ffi::OsString;
    use std::sync::Mutex;

    /// Shared with other modules' tests that mutate `XDG_CONFIG_HOME` (e.g. tui config open).
    pub(crate) static ENV_MUTEX: Mutex<()> = Mutex::new(());

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
                direction: Some(SyncDirection::Upload),
                baseline: crate::sync_baseline::SyncBaseline {
                    local_sha256: Some("hash".into()),
                    ..Default::default()
                },
                ..PinnedMapping::fixture("/tmp/settings.json", "abc123", "settings.json")
            }],
            skip_dirs: default_skip_dirs(),
            prefs: Preferences::default(),
        };

        save_config(&path, &config).unwrap();
        assert_eq!(load_config(&path).unwrap(), config);
    }

    #[test]
    fn save_config_omits_defaults_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = AppConfig::default();

        save_config(&path, &config).unwrap();

        let saved: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let table = saved.as_table().unwrap();
        assert_eq!(table.len(), 1);
        assert_eq!(table.get("pinned"), Some(&toml::Value::Array(Vec::new())));
        assert_eq!(load_config(&path).unwrap(), config);
    }

    /// Pins the saved file's exact shape — key order, omitted defaults, `pinned` always
    /// present — with every preference off its default.
    #[test]
    fn save_config_writes_every_custom_value_in_a_stable_layout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = AppConfig {
            skip_dirs: vec!["x".into()],
            prefs: Preferences {
                scan_depth: 5,
                diff_context: 7,
                diff_show_full: true,
                theme: ThemeChoice::Light,
                mouse: false,
                check_updates: false,
                ignore_trailing_newline: false,
                normalize_line_endings: false,
            },
            ..Default::default()
        };

        save_config(&path, &config).unwrap();

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "pinned = []\nskip_dirs = [\"x\"]\nscan_depth = 5\ndiff_context = 7\n\
             diff_show_full = true\ntheme = \"light\"\nmouse = false\ncheck_updates = false\n\
             ignore_trailing_newline = false\nnormalize_line_endings = false\n"
        );
        assert_eq!(load_config(&path).unwrap(), config);
    }

    fn edit(existing: &str, change: impl FnOnce(&mut AppConfig)) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, existing).unwrap();
        let mut config = load_config(&path).unwrap();
        change(&mut config);
        save_config(&path, &config).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert_eq!(load_config(&path).unwrap(), config, "round-trips:\n{saved}");
        saved
    }

    /// Issue #484: a save edits the file — comments, unknown keys, and values the user wrote
    /// out (even when they equal a default) stay; only the changed key moves.
    #[test]
    fn save_config_keeps_comments_unknown_keys_and_untouched_values() {
        let existing = "\
# my gistui settings
theme = \"light\" # I like it bright

# written out on purpose, though it is the default
mouse = true
future_option = 42
";
        let saved = edit(existing, |c| c.prefs.scan_depth = 5);
        assert_eq!(saved, format!("{existing}scan_depth = 5\n"));
    }

    /// The common case behind #484: a config copied from `config.example.toml` keeps every
    /// line but the one a setting changed.
    #[test]
    fn save_config_keeps_a_copied_example_config_intact() {
        let example = include_str!("../config.example.toml");
        let saved = edit(example, |c| c.prefs.mouse = false);
        assert_eq!(saved, example.replacen("mouse = true", "mouse = false", 1));
    }

    /// A CRLF config (as Windows writes, and as git checks this repo's example out there)
    /// stays CRLF throughout, not just on the changed line.
    #[test]
    fn save_config_keeps_crlf_line_endings() {
        let existing = "# settings\r\nmouse = true\r\ntheme = \"light\"\r\n";
        let saved = edit(existing, |c| c.prefs.mouse = false);
        assert_eq!(
            saved,
            "# settings\r\nmouse = false\r\ntheme = \"light\"\r\n"
        );
    }

    #[test]
    fn save_config_removes_a_key_changed_back_to_its_default() {
        let existing = "# keep me\nmouse = false # was off\ntheme = \"light\"\n";
        let saved = edit(existing, |c| c.prefs.mouse = true);
        assert_eq!(saved, "# keep me\ntheme = \"light\"\n");
    }

    #[test]
    fn save_config_updates_a_value_in_place() {
        let existing = "# depth\nscan_depth = 4 # deep enough\nmouse = false\n";
        let saved = edit(existing, |c| c.prefs.scan_depth = 6);
        assert_eq!(
            saved,
            "# depth\nscan_depth = 6 # deep enough\nmouse = false\n"
        );
    }

    /// `[[pinned]]` entries keep their comments across a sync record, a new pin, and the
    /// removal of another pin; a hand-written pin without `gist_filename` stays that way.
    #[test]
    fn save_config_edits_pins_in_place() {
        let existing = "\
mouse = false

# dotfiles
[[pinned]]
local_path = \"~/.zshrc\" # shell
gist_id = \"abc\"

# going away
[[pinned]]
local_path = \"~/old.txt\"
gist_id = \"def\"
gist_filename = \"old.txt\"
";
        let saved = edit(existing, |c| {
            c.pinned[0].baseline.local_sha256 = Some("h".into());
            c.pinned.remove(1);
            c.pinned
                .push(PinnedMapping::fixture("new.txt", "ghi", "new.txt"));
        });
        assert!(saved.starts_with("mouse = false\n\n# dotfiles\n[[pinned]]\nlocal_path = \"~/.zshrc\" # shell\ngist_id = \"abc\"\nlast_seen_hash = \"h\"\n"), "{saved}");
        assert!(
            !saved.contains("old.txt") && !saved.contains("going away"),
            "{saved}"
        );
        assert!(!saved.contains("gist_filename = \".zshrc\""), "{saved}");
        assert!(saved.contains("gist_id = \"ghi\""), "{saved}");
    }

    #[test]
    fn save_config_keeps_only_custom_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = AppConfig {
            pinned: vec![PinnedMapping::fixture(
                "/tmp/settings.json",
                "abc",
                "settings.json",
            )],
            prefs: Preferences {
                scan_depth: 4,
                mouse: false,
                ..Default::default()
            },
            ..Default::default()
        };
        config.prefs.scan_depth = Preferences::default().scan_depth;

        save_config(&path, &config).unwrap();

        let saved: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let table = saved.as_table().unwrap();
        assert_eq!(table.len(), 2);
        assert!(table.contains_key("pinned"));
        assert_eq!(table.get("mouse"), Some(&toml::Value::Boolean(false)));
        assert!(!table.contains_key("scan_depth"));
        assert_eq!(load_config(&path).unwrap(), config);
    }

    #[test]
    fn load_config_defaults_a_missing_gist_filename_to_the_local_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "mouse = false\n\n[[pinned]]\nlocal_path = \"~/.zshrc\"\ngist_id = \"abc\"\n",
        )
        .unwrap();

        let config = load_config(&path).unwrap();

        assert!(!config.prefs.mouse, "the rest of the config still loads");
        assert_eq!(config.pinned.len(), 1);
        assert_eq!(config.pinned[0].gist_filename, ".zshrc");
    }

    #[test]
    fn load_config_rejects_a_pin_with_no_name_to_default_to() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[[pinned]]\nlocal_path = \"/\"\ngist_id = \"abc\"\n").unwrap();

        let err = load_config(&path).unwrap_err();
        assert!(format!("{err:#}").contains("no gist_filename"), "{err:#}");
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
        // Host-absolute path (Windows rejects bare "/tmp/..." as absolute).
        let absolute = env::temp_dir().join("gistui-normalize-absolute-settings.json");
        assert!(absolute.is_absolute());
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

    #[test]
    fn mouse_defaults_to_true_when_absent() {
        // A config file with no `mouse` key must load as enabled.
        let toml = "scan_depth = 4\n";
        let config: AppConfig = toml::from_str(toml).unwrap();
        assert!(config.prefs.mouse);
    }

    #[test]
    fn mouse_round_trips() {
        let config = AppConfig {
            prefs: Preferences {
                mouse: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let text = toml::to_string(&config).unwrap();
        let parsed: AppConfig = toml::from_str(&text).unwrap();
        assert!(!parsed.prefs.mouse);
    }

    #[test]
    fn check_updates_defaults_to_true_when_absent() {
        // A config file with no `check_updates` key must load as enabled.
        let toml = "scan_depth = 4\n";
        let config: AppConfig = toml::from_str(toml).unwrap();
        assert!(config.prefs.check_updates);
    }

    #[test]
    fn ignore_trailing_newline_defaults_to_true_when_absent() {
        // A config file with no `ignore_trailing_newline` key must load as enabled.
        let toml = "scan_depth = 4\n";
        let config: AppConfig = toml::from_str(toml).unwrap();
        assert!(config.prefs.ignore_trailing_newline);
    }

    #[test]
    fn ignore_trailing_newline_round_trips() {
        let config = AppConfig {
            prefs: Preferences {
                ignore_trailing_newline: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let text = toml::to_string(&config).unwrap();
        let parsed: AppConfig = toml::from_str(&text).unwrap();
        assert!(!parsed.prefs.ignore_trailing_newline);
    }

    #[test]
    fn normalize_line_endings_defaults_to_true_when_absent() {
        // A config file with no `normalize_line_endings` key must load as enabled.
        let toml = "scan_depth = 4\n";
        let config: AppConfig = toml::from_str(toml).unwrap();
        assert!(config.prefs.normalize_line_endings);
    }

    #[test]
    fn normalize_line_endings_round_trips() {
        let config = AppConfig {
            prefs: Preferences {
                normalize_line_endings: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let text = toml::to_string(&config).unwrap();
        let parsed: AppConfig = toml::from_str(&text).unwrap();
        assert!(!parsed.prefs.normalize_line_endings);
    }
}
