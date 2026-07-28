//! Per-screen colocation (issue #287, Phase 2): each `Screen` variant's `handle_key_*`,
//! `build_*_vm`, `render_*_vm`, and `*_palette_items` bodies live in one file here, migrated
//! one screen at a time. The five per-screen registries (`handle_key` dispatch,
//! `build_view_model`, `render_screen_vm`, `build_palette_items`, `HelpTopic::for_screen`)
//! keep their exhaustive-match shape; each arm becomes a one-line call into a screen module.

pub(crate) mod config;
pub(crate) mod detail;
pub(crate) mod diff;
pub(crate) mod gists;
pub(crate) mod help;
pub(crate) mod pins;
pub(crate) mod preview;
pub(crate) mod revisions;
