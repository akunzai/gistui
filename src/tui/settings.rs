//! Runtime ownership for persisted TUI settings and CLI force-off overrides.

use super::Theme;
use crate::config::{AppConfig, ThemeChoice};

/// Fields shown on the Settings screen, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigField {
    Theme,
    Mouse,
    CheckUpdates,
    DiffShowFull,
    IgnoreTrailingNewline,
    ScanDepth,
    DiffContext,
}

impl ConfigField {
    pub const ALL: [Self; 7] = [
        Self::Theme,
        Self::Mouse,
        Self::CheckUpdates,
        Self::DiffShowFull,
        Self::IgnoreTrailingNewline,
        Self::ScanDepth,
        Self::DiffContext,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
            Self::Mouse => "Mouse support",
            Self::CheckUpdates => "Check for updates",
            Self::DiffShowFull => "Show full diff",
            Self::IgnoreTrailingNewline => "Ignore trailing newline",
            Self::ScanDepth => "Recursive scan depth",
            Self::DiffContext => "Diff context lines",
        }
    }

    pub fn is_numeric(self) -> bool {
        matches!(self, Self::ScanDepth | Self::DiffContext)
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Theme => "terminal colours",
            Self::Mouse => "click and wheel input",
            Self::CheckUpdates => "daily GitHub version check",
            Self::DiffShowFull => "open Diff expanded",
            Self::IgnoreTrailingNewline => "hide newline-only diffs",
            Self::ScanDepth => "directory levels to scan",
            Self::DiffContext => "unchanged lines around edits",
        }
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
    theme_choice: ThemeChoice,
    mouse: bool,
    check_updates: bool,
    diff_show_full: bool,
    ignore_trailing_newline: bool,
    scan_depth: u32,
    diff_context: u32,
    no_mouse: bool,
    no_update_check: bool,
}

impl RuntimeSettings {
    pub fn from_config(config: &AppConfig, no_mouse: bool, no_update_check: bool) -> Self {
        Self {
            theme_choice: config.theme,
            mouse: config.mouse,
            check_updates: config.check_updates,
            diff_show_full: config.diff_show_full,
            ignore_trailing_newline: config.ignore_trailing_newline,
            scan_depth: config.scan_depth,
            diff_context: config.diff_context,
            no_mouse,
            no_update_check,
        }
    }

    pub fn adjust(&mut self, field: ConfigField, forward: bool) -> Option<SettingsChange> {
        let effect = match field {
            ConfigField::Theme => {
                self.theme_choice = match self.theme_choice {
                    ThemeChoice::Dark => ThemeChoice::Light,
                    ThemeChoice::Light => ThemeChoice::Dark,
                };
                None
            }
            ConfigField::Mouse => {
                self.mouse = !self.mouse;
                Some(SettingsEffect::SyncMouseCapture)
            }
            ConfigField::CheckUpdates => {
                self.check_updates = !self.check_updates;
                None
            }
            ConfigField::DiffShowFull => {
                self.diff_show_full = !self.diff_show_full;
                None
            }
            ConfigField::IgnoreTrailingNewline => {
                self.ignore_trailing_newline = !self.ignore_trailing_newline;
                None
            }
            ConfigField::ScanDepth => {
                let next = if forward {
                    self.scan_depth.saturating_add(1).min(20)
                } else {
                    self.scan_depth.saturating_sub(1)
                };
                if next == self.scan_depth {
                    return None;
                }
                self.scan_depth = next;
                None
            }
            ConfigField::DiffContext => {
                let next = if forward {
                    self.diff_context.saturating_add(1).min(50)
                } else {
                    self.diff_context.saturating_sub(1)
                };
                if next == self.diff_context {
                    return None;
                }
                self.diff_context = next;
                None
            }
        };
        Some(SettingsChange { effect })
    }

    pub fn apply_to_config(&self, config: &mut AppConfig) {
        config.theme = self.theme_choice;
        config.mouse = self.mouse;
        config.check_updates = self.check_updates;
        config.diff_show_full = self.diff_show_full;
        config.ignore_trailing_newline = self.ignore_trailing_newline;
        config.scan_depth = self.scan_depth;
        config.diff_context = self.diff_context;
    }

    pub fn field_value(&self, field: ConfigField) -> String {
        match field {
            ConfigField::Theme => match self.theme_choice {
                ThemeChoice::Dark => "dark",
                ThemeChoice::Light => "light",
            }
            .into(),
            ConfigField::Mouse => if self.mouse { "on" } else { "off" }.into(),
            ConfigField::CheckUpdates => if self.check_updates { "on" } else { "off" }.into(),
            ConfigField::DiffShowFull => if self.diff_show_full { "on" } else { "off" }.into(),
            ConfigField::IgnoreTrailingNewline => if self.ignore_trailing_newline {
                "on"
            } else {
                "off"
            }
            .into(),
            ConfigField::ScanDepth => self.scan_depth.to_string(),
            ConfigField::DiffContext => self.diff_context.to_string(),
        }
    }

    pub fn theme_choice(&self) -> ThemeChoice {
        self.theme_choice
    }
    pub fn theme(&self) -> Theme {
        Theme::for_choice(self.theme_choice)
    }
    pub fn mouse_enabled(&self) -> bool {
        self.mouse && !self.no_mouse
    }
    pub fn update_check_enabled(&self) -> bool {
        self.check_updates && !self.no_update_check
    }
    pub fn diff_show_full(&self) -> bool {
        self.diff_show_full
    }
    pub fn ignore_trailing_newline(&self) -> bool {
        self.ignore_trailing_newline
    }
    pub fn scan_depth(&self) -> u32 {
        self.scan_depth
    }
    pub fn diff_context(&self) -> u32 {
        self.diff_context
    }

    pub fn effective_diff_context(&self) -> Option<usize> {
        (!self.diff_show_full).then_some(self.diff_context as usize)
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

    #[test]
    fn cli_overrides_force_effective_values_off_without_changing_preferences() {
        let config = AppConfig::default();
        let settings = RuntimeSettings::from_config(&config, true, true);
        assert!(!settings.mouse_enabled());
        assert!(!settings.update_check_enabled());
        let mut saved = AppConfig::default();
        settings.apply_to_config(&mut saved);
        assert!(saved.mouse);
        assert!(saved.check_updates);
    }

    #[test]
    fn numeric_adjustments_clamp_and_report_no_change_at_bounds() {
        let mut config = AppConfig {
            scan_depth: 20,
            diff_context: 0,
            ..AppConfig::default()
        };
        let mut settings = RuntimeSettings::from_config(&config, false, false);
        assert!(settings.adjust(ConfigField::ScanDepth, true).is_none());
        assert!(settings.adjust(ConfigField::DiffContext, false).is_none());
        assert!(settings.adjust(ConfigField::ScanDepth, false).is_some());
        assert_eq!(settings.scan_depth(), 19);
        settings.apply_to_config(&mut config);
        assert_eq!(config.diff_context, 0);
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
            theme: ThemeChoice::Light,
            mouse: false,
            check_updates: false,
            diff_show_full: true,
            ignore_trailing_newline: false,
            scan_depth: 5,
            diff_context: 7,
            ..AppConfig::default()
        };
        let settings = RuntimeSettings::from_config(&source, false, false);
        let mut target = AppConfig::default();
        target.pinned.push(crate::domain::PinnedMapping {
            local_path: "/tmp/keep".into(),
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: None,
        });
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
