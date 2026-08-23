//! Per-screen colocation (issue #287): each `Screen` variant's `handle_key_*`,
//! `build_*_vm`, and `render_*_vm` live in one file here.
//!
//! [`lookup`] is the exhaustive match for the data-like per-screen columns
//! (issue #377): help topic, wheel step, key guard, VM builder, key handling,
//! navigation, and list selection. The lookup-before-call pattern is #274's two-phase
//! borrow solution. `render_screen_vm` matches [`ScreenVm`], not [`Screen`].
//! `keymap::for_screen` stays in `keymap.rs` (bindings are that table; putting
//! them here would cycle `screens` ↔ `keymap`).

use super::keys::NavAction;
use super::view_model::ScreenVm;
use super::{AppState, HelpTopic, KeyOutcome, MouseFrame, Screen};
use crossterm::event::{KeyCode, KeyModifiers};

pub(crate) mod config;
pub(crate) mod confirm;
pub(crate) mod detail;
pub(crate) mod diff;
pub(crate) mod gists;
pub(crate) mod help;
pub(crate) mod list;
pub(crate) mod palette;
pub(crate) mod pins;
pub(crate) mod preview;
pub(crate) mod revisions;

/// Per-screen facts selected by [`lookup`]. Columns are `fn` pointers so the
/// screen files stay the adapters — this type does not add a parallel set of
/// dummy types.
pub(crate) struct ScreenLookup {
    pub help_topic: HelpTopic,
    pub wheel_step: fn(&AppState) -> usize,
    pub guard: fn(&AppState, KeyCode) -> bool,
    pub build_vm: fn(&AppState) -> ScreenVm,
    pub handle_key: fn(&mut AppState, KeyCode, KeyModifiers) -> KeyOutcome,
    pub apply_navigation: fn(&mut AppState, NavAction) -> bool,
    pub click_select: fn(&mut AppState, u16, u16, &MouseFrame) -> bool,
}

fn ungated(_: &AppState, _: KeyCode) -> bool {
    true
}

fn no_click(_: &mut AppState, _: u16, _: u16, _: &MouseFrame) -> bool {
    false
}
fn scroll_navigation(state: &mut AppState, action: NavAction) -> bool {
    let Some(body) = state.scroll_body_mut() else {
        return false;
    };
    match action {
        NavAction::Down => body.down(),
        NavAction::Up => body.up(),
        NavAction::PageDown => body.page_down(super::keys::PAGE_SCROLL),
        NavAction::PageUp => body.page_up(super::keys::PAGE_SCROLL),
        NavAction::Right => body.right(),
        NavAction::Left => body.left(),
    }
    true
}

/// Exhaustive map from `Screen` to all lookup columns. A new variant that
/// forgets this match fails to compile; that is the protection the old scattered
/// screen matches duplicated.
pub(crate) fn lookup(screen: &Screen) -> ScreenLookup {
    match screen {
        Screen::List => ScreenLookup {
            help_topic: list::help_topic(),
            wheel_step: |_: &AppState| list::wheel_step(),
            guard: list::list_guard,
            build_vm: |state| ScreenVm::List(list::build_list_vm(state)),
            handle_key: |state, code, _| {
                if state.filtering {
                    state.handle_key_filter(code)
                } else {
                    state.handle_key_list(code)
                }
            },
            apply_navigation: AppState::apply_navigation_list,
            click_select: AppState::click_select_list,
        },
        Screen::Pins(_) => ScreenLookup {
            help_topic: pins::help_topic(),
            wheel_step: |_: &AppState| pins::wheel_step(),
            guard: pins::pins_guard,
            build_vm: |state| ScreenVm::Pins(pins::build_pins_vm(state)),
            handle_key: |state, code, _| state.handle_key_pins(code),
            apply_navigation: AppState::apply_navigation_pins,
            click_select: AppState::click_select_pins,
        },
        Screen::Gists(_) => ScreenLookup {
            help_topic: gists::help_topic(),
            wheel_step: |_: &AppState| gists::wheel_step(),
            guard: gists::gists_guard,
            build_vm: |state| ScreenVm::Gists(gists::build_gists_vm(state)),
            handle_key: |state, code, _| state.handle_key_gists(code),
            apply_navigation: AppState::apply_navigation_gists,
            click_select: AppState::click_select_gists,
        },
        Screen::GistDetail(_) => ScreenLookup {
            help_topic: detail::help_topic(),
            wheel_step: detail::wheel_step,
            guard: detail::detail_guard,
            build_vm: |state| ScreenVm::GistDetail(detail::build_gist_detail_vm(state)),
            handle_key: |state, code, _| state.handle_key_detail(code),
            apply_navigation: AppState::apply_navigation_detail,
            click_select: AppState::click_select_detail,
        },
        Screen::Revisions(_) => ScreenLookup {
            help_topic: revisions::help_topic(),
            wheel_step: |_: &AppState| revisions::wheel_step(),
            guard: revisions::revisions_guard,
            build_vm: |state| ScreenVm::Revisions(revisions::build_revisions_vm(state)),
            handle_key: |state, code, _| state.handle_key_revisions(code),
            apply_navigation: AppState::apply_navigation_revisions,
            click_select: AppState::click_select_revisions,
        },
        Screen::Diff(_) => ScreenLookup {
            help_topic: diff::help_topic(),
            wheel_step: |_: &AppState| diff::wheel_step(),
            guard: diff::diff_guard,
            build_vm: |state| ScreenVm::Diff(diff::build_diff_vm(state)),
            handle_key: |state, code, _| state.handle_key_diff(code),
            apply_navigation: scroll_navigation,
            click_select: no_click,
        },
        Screen::Preview(_) => ScreenLookup {
            help_topic: preview::help_topic(),
            wheel_step: |_: &AppState| preview::wheel_step(),
            guard: ungated,
            build_vm: |state| ScreenVm::Preview(preview::build_preview_vm(state)),
            handle_key: |state, code, _| state.handle_key_preview(code),
            apply_navigation: scroll_navigation,
            click_select: no_click,
        },
        Screen::Help(_) => ScreenLookup {
            help_topic: help::help_topic(),
            wheel_step: |state| state.help().map(help::wheel_step).unwrap_or(1),
            guard: ungated,
            build_vm: |state| ScreenVm::Help(help::build_help_vm(state)),
            handle_key: |state, code, _| state.handle_key_help(code),
            apply_navigation: AppState::apply_navigation_help,
            click_select: AppState::click_select_help,
        },
        Screen::Config(_) => ScreenLookup {
            help_topic: config::help_topic(),
            wheel_step: |_: &AppState| config::wheel_step(),
            guard: ungated,
            build_vm: |state| ScreenVm::Config(config::build_config_vm(state)),
            handle_key: |state, code, _| state.handle_key_config(code),
            apply_navigation: AppState::apply_navigation_config,
            click_select: AppState::click_select_config,
        },
        Screen::Confirm(_) => ScreenLookup {
            help_topic: confirm::help_topic(),
            wheel_step: |_: &AppState| confirm::wheel_step(),
            guard: ungated,
            build_vm: |state| ScreenVm::Confirm(confirm::build_confirm_vm(state)),
            handle_key: |state, code, _| state.handle_key_confirm(code),
            apply_navigation: scroll_navigation,
            click_select: no_click,
        },
        Screen::Palette(_) => ScreenLookup {
            help_topic: palette::help_topic(),
            wheel_step: |_: &AppState| palette::wheel_step(),
            guard: ungated,
            build_vm: |state| ScreenVm::Palette(palette::build_palette_vm(state)),
            handle_key: |state, code, modifiers| state.handle_key_palette(code, modifiers),
            apply_navigation: |state, action| {
                let len = state.palette_visible_items().len();
                if let Some(p) = state.palette_mut() {
                    match action {
                        NavAction::Up => p.selected = p.selected.saturating_sub(1),
                        NavAction::Down if len > 0 && p.selected + 1 < len => p.selected += 1,
                        NavAction::Down => {}
                        _ => return false,
                    }
                }
                true
            },
            click_select: no_click,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_answers_for_every_variant() {
        let cases: &[(&str, Screen, HelpTopic)] = &[
            ("List", Screen::List, list::HELP_TOPIC),
            ("Pins", Screen::Pins(Box::default()), pins::HELP_TOPIC),
            ("Gists", Screen::Gists(Box::default()), gists::HELP_TOPIC),
            (
                "GistDetail",
                Screen::GistDetail(Box::default()),
                detail::HELP_TOPIC,
            ),
            (
                "Revisions",
                Screen::Revisions(Box::default()),
                revisions::HELP_TOPIC,
            ),
            ("Diff", Screen::Diff(Box::default()), diff::HELP_TOPIC),
            (
                "Preview",
                Screen::Preview(Box::default()),
                preview::HELP_TOPIC,
            ),
            ("Help", Screen::Help(Box::default()), help::HELP_TOPIC),
            ("Config", Screen::Config(Box::default()), config::HELP_TOPIC),
            (
                "Confirm",
                Screen::Confirm(Box::default()),
                confirm::HELP_TOPIC,
            ),
            (
                "Palette",
                Screen::Palette(Box::default()),
                palette::HELP_TOPIC,
            ),
        ];
        for (name, screen, topic) in cases {
            let ScreenLookup {
                help_topic: found,
                wheel_step,
                guard,
                build_vm,
                handle_key,
                apply_navigation,
                click_select,
            } = lookup(screen);
            assert_eq!(found, *topic, "{name} help topic");
            let _ = (
                wheel_step,
                guard,
                build_vm,
                handle_key,
                apply_navigation,
                click_select,
            );
        }
    }
}
