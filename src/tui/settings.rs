//! Runtime ownership for persisted TUI settings and CLI force-off overrides.

use super::Theme;
use crate::config::{AppConfig, Preferences, ThemeChoice};

/// Declares the Settings-screen fields in display order. Each field has a label, a one-line
/// description (the Settings row), and help text (its `?` help line). `ALL` is generated from
/// the same list, so a field can't be declared and then left off the screen or out of help.
macro_rules! config_fields {
    ($($variant:ident => $label:literal, $description:literal, $help:literal;)+) => {
        /// Fields shown on the Settings screen, in display order.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum ConfigField {
            $($variant),+
        }

        impl ConfigField {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn label(self) -> &'static str {
                match self {
                    $(Self::$variant => $label),+
                }
            }

            pub fn description(self) -> &'static str {
                match self {
                    $(Self::$variant => $description),+
                }
            }

            /// The field's line in the Config topic of `?` help.
            pub fn help(self) -> &'static str {
                match self {
                    $(Self::$variant => $help),+
                }
            }
        }
    };
}

config_fields! {
    Theme => "Theme", "terminal colours", "dark / light (also global T)";
    Mouse => "Mouse support", "click and wheel input",
        "on / off (session still respects --no-mouse)";
    CheckUpdates => "Check for updates", "daily GitHub version check",
        "on / off (session still respects --no-update-check)";
    DiffShowFull => "Show full diff", "open Diff expanded",
        "on / off (opens Diff expanded; c still toggles it)";
    IgnoreTrailingNewline => "Ignore trailing newline", "hide newline-only diffs",
        "on / off (diff + overwrite confirm)";
    NormalizeLineEndings => "Normalize line endings", "force LF on upload/create/download",
        "on / off (LF in what upload, create, and download send / write)";
    ScanDepth => "Recursive scan depth", "directory levels to scan",
        "0–20 (r recursive discovery)";
    DiffContext => "Diff context lines", "unchanged lines around edits",
        "0–50 (c in Diff still toggles full vs this radius)";
}

impl ConfigField {
    pub fn is_numeric(self) -> bool {
        matches!(self, Self::ScanDepth | Self::DiffContext)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsEffect {
    SyncMouseCapture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsChange {
    pub effect: Option<SettingsEffect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSettings {
    prefs: Preferences,
    no_mouse: bool,
    no_update_check: bool,
}

impl RuntimeSettings {
    pub fn from_config(config: &AppConfig, no_mouse: bool, no_update_check: bool) -> Self {
        Self {
            prefs: config.prefs.clone(),
            no_mouse,
            no_update_check,
        }
    }

    /// The on/off preference behind `field`, if it is one.
    fn flag_mut(&mut self, field: ConfigField) -> Option<&mut bool> {
        let p = &mut self.prefs;
        match field {
            ConfigField::Mouse => Some(&mut p.mouse),
            ConfigField::CheckUpdates => Some(&mut p.check_updates),
            ConfigField::DiffShowFull => Some(&mut p.diff_show_full),
            ConfigField::IgnoreTrailingNewline => Some(&mut p.ignore_trailing_newline),
            ConfigField::NormalizeLineEndings => Some(&mut p.normalize_line_endings),
            ConfigField::Theme | ConfigField::ScanDepth | ConfigField::DiffContext => None,
        }
    }

    pub fn adjust(&mut self, field: ConfigField, forward: bool) -> Option<SettingsChange> {
        if let Some(flag) = self.flag_mut(field) {
            *flag = !*flag;
            let effect = (field == ConfigField::Mouse).then_some(SettingsEffect::SyncMouseCapture);
            return Some(SettingsChange { effect });
        }
        let p = &mut self.prefs;
        let (value, max) = match field {
            ConfigField::Theme => {
                p.theme = match p.theme {
                    ThemeChoice::Dark => ThemeChoice::Light,
                    ThemeChoice::Light => ThemeChoice::Dark,
                };
                return Some(SettingsChange { effect: None });
            }
            ConfigField::ScanDepth => (&mut p.scan_depth, 20),
            ConfigField::DiffContext => (&mut p.diff_context, 50),
            ConfigField::Mouse
            | ConfigField::CheckUpdates
            | ConfigField::DiffShowFull
            | ConfigField::IgnoreTrailingNewline
            | ConfigField::NormalizeLineEndings => unreachable!("handled as a flag above"),
        };
        let next = if forward {
            value.saturating_add(1).min(max)
        } else {
            value.saturating_sub(1)
        };
        if next == *value {
            return None;
        }
        *value = next;
        Some(SettingsChange { effect: None })
    }

    /// Write every runtime-owned preference back into `config`, leaving pins, skip
    /// directories, and anything else in it alone.
    pub fn apply_to_config(&self, config: &mut AppConfig) {
        config.prefs = self.prefs.clone();
    }

    pub fn field_value(&self, field: ConfigField) -> String {
        let p = &self.prefs;
        let on_off = |on: bool| if on { "on" } else { "off" }.to_string();
        match field {
            ConfigField::Theme => match p.theme {
                ThemeChoice::Dark => "dark",
                ThemeChoice::Light => "light",
            }
            .into(),
            ConfigField::Mouse => on_off(p.mouse),
            ConfigField::CheckUpdates => on_off(p.check_updates),
            ConfigField::DiffShowFull => on_off(p.diff_show_full),
            ConfigField::IgnoreTrailingNewline => on_off(p.ignore_trailing_newline),
            ConfigField::NormalizeLineEndings => on_off(p.normalize_line_endings),
            ConfigField::ScanDepth => p.scan_depth.to_string(),
            ConfigField::DiffContext => p.diff_context.to_string(),
        }
    }

    pub fn theme_choice(&self) -> ThemeChoice {
        self.prefs.theme
    }
    pub fn theme(&self) -> Theme {
        Theme::for_choice(self.prefs.theme)
    }
    pub fn mouse_enabled(&self) -> bool {
        self.prefs.mouse && !self.no_mouse
    }
    pub fn update_check_enabled(&self) -> bool {
        self.prefs.check_updates && !self.no_update_check
    }
    pub fn diff_show_full(&self) -> bool {
        self.prefs.diff_show_full
    }
    pub fn ignore_trailing_newline(&self) -> bool {
        self.prefs.ignore_trailing_newline
    }
    pub fn normalize_line_endings(&self) -> bool {
        self.prefs.normalize_line_endings
    }
    /// The Sync policy these settings describe.
    pub fn sync_policy(&self) -> crate::sync_content::SyncPolicy {
        crate::sync_content::SyncPolicy {
            normalize_line_endings: self.prefs.normalize_line_endings,
            ignore_trailing_newline: self.prefs.ignore_trailing_newline,
        }
    }
    pub fn scan_depth(&self) -> u32 {
        self.prefs.scan_depth
    }
    pub fn diff_context(&self) -> u32 {
        self.prefs.diff_context
    }

    pub fn effective_diff_context(&self) -> Option<usize> {
        (!self.prefs.diff_show_full).then_some(self.prefs.diff_context as usize)
    }
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self::from_config(&AppConfig::default(), false, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every Settings field, changed on screen, survives a save and a reload — the path that
    /// used to lose a field silently when one of its hand-written copies was missed.
    #[test]
    fn every_field_change_survives_save_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for &field in ConfigField::ALL {
            let mut settings = RuntimeSettings::default();
            assert!(
                settings.adjust(field, true).is_some(),
                "{field:?} didn't change"
            );
            let changed = settings.field_value(field);

            let mut config = AppConfig::default();
            settings.apply_to_config(&mut config);
            crate::config::save_config(&path, &config).unwrap();
            let reloaded = RuntimeSettings::from_config(
                &crate::config::load_config(&path).unwrap(),
                false,
                false,
            );

            assert_eq!(reloaded.field_value(field), changed, "{field:?}");
            assert_ne!(
                RuntimeSettings::default().field_value(field),
                changed,
                "{field:?} must differ from its default"
            );
        }
    }

    #[test]
    fn cli_overrides_force_effective_values_off_without_changing_preferences() {
        let config = AppConfig::default();
        let settings = RuntimeSettings::from_config(&config, true, true);
        assert!(!settings.mouse_enabled());
        assert!(!settings.update_check_enabled());
        let mut saved = AppConfig::default();
        settings.apply_to_config(&mut saved);
        assert!(saved.prefs.mouse);
        assert!(saved.prefs.check_updates);
    }

    #[test]
    fn numeric_adjustments_clamp_and_report_no_change_at_bounds() {
        let mut config = AppConfig {
            prefs: Preferences {
                scan_depth: 20,
                diff_context: 0,
                ..Default::default()
            },
            ..AppConfig::default()
        };
        let mut settings = RuntimeSettings::from_config(&config, false, false);
        assert!(settings.adjust(ConfigField::ScanDepth, true).is_none());
        assert!(settings.adjust(ConfigField::DiffContext, false).is_none());
        assert!(settings.adjust(ConfigField::ScanDepth, false).is_some());
        assert_eq!(settings.scan_depth(), 19);
        settings.apply_to_config(&mut config);
        assert_eq!(config.prefs.diff_context, 0);
    }

    #[test]
    fn only_mouse_adjustment_requests_external_effect() {
        let mut settings = RuntimeSettings::default();
        let change = settings.adjust(ConfigField::Mouse, true).unwrap();
        assert_eq!(change.effect, Some(SettingsEffect::SyncMouseCapture));
        let change = settings.adjust(ConfigField::Theme, true).unwrap();
        assert_eq!(change.effect, None);
    }

    #[test]
    fn projection_updates_every_owned_field_and_preserves_the_rest() {
        let source = AppConfig {
            prefs: Preferences {
                theme: ThemeChoice::Light,
                mouse: false,
                check_updates: false,
                diff_show_full: true,
                ignore_trailing_newline: false,
                scan_depth: 5,
                diff_context: 7,
                ..Default::default()
            },
            ..AppConfig::default()
        };
        let settings = RuntimeSettings::from_config(&source, false, false);
        let mut target = AppConfig::default();
        target.pinned.push(crate::domain::PinnedMapping::fixture(
            "/tmp/keep",
            "g1",
            "a.txt",
        ));
        target.skip_dirs = vec!["keep-me".into()];
        settings.apply_to_config(&mut target);
        assert_eq!(
            target,
            AppConfig {
                pinned: target.pinned.clone(),
                skip_dirs: vec!["keep-me".into()],
                ..source
            }
        );
    }
}
