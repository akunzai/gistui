//! Terminal event loop: draw, poll input, absorb background results, dispatch key outcomes.
//! Heavy IO helpers live in [`super::bg`] and [`super::dispatch`] (issue #225).
//! Background work is owned by the [`Jobs`] registry (issue #243).

use super::bg::{spawn_update_check, Jobs, LoopFlow};
use super::dispatch::dispatch_outcome;
use super::*;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

pub(super) fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    no_mouse: bool,
    no_update_check: bool,
) -> Result<()> {
    let mut state = load_startup_state(no_mouse, no_update_check)?;
    if state.mouse_enabled {
        execute!(terminal.backend_mut(), EnableMouseCapture)?;
    }
    let mut mouse_layout = MouseLayout::default();
    let mut last_click: Option<(u16, u16, std::time::Instant)> = None;

    // Background "is a newer release available?" check — off-thread, throttled to once a day,
    // silent on failure. The result (if any) is absorbed in the loop below.
    let update_check_path = crate::update_check::state_path().ok();
    let mut update_rx: Option<std::sync::mpsc::Receiver<crate::update_check::UpdateCheckOutcome>> =
        None;
    if state.update_check_enabled {
        let due = update_check_path.as_ref().is_none_or(|path| {
            crate::update_check::should_check(
                crate::update_check::load_state(path).last_check,
                crate::update_check::now_secs(),
            )
        });
        if due {
            update_rx = Some(spawn_update_check());
        }
    }
    let mut jobs = Jobs::startup(update_rx, true);
    // Pins presentation cache: refresh on enter/return to Pins (or Pins-as-palette-bg) and
    // whenever the dirty flag is set — never every frame while already on Pins (#241).
    let mut pin_sync_screen_active = false;

    loop {
        let pins_active =
            state.screen.is_pins() || state.palette().is_some_and(|p| p.origin_screen.is_pins());
        if pins_active
            && (state.pin_sync_cache_dirty
                || !pin_sync_screen_active
                || state.pin_sync_cache.len() != state.pinned.len())
        {
            state.refresh_pin_sync_cache();
        }
        pin_sync_screen_active = pins_active;

        terminal.draw(|frame| render(frame, &state, &mut mouse_layout))?;
        if state.detail().is_some_and(|d| d.comments_scroll_to_bottom) {
            if let Some(max) = mouse_layout.comments_max_scroll {
                if let Some(d) = state.detail_mut() {
                    d.scroll = max;
                }
            }
            if let Some(d) = state.detail_mut() {
                d.comments_scroll_to_bottom = false;
            }
        }
        // Advance the spinner once per iteration; the poll below caps the loop at ~150ms, so
        // in-progress states (scanning/loading/working) animate even with no input.
        state.spinner_frame = state.spinner_frame.wrapping_add(1);

        match jobs.absorb(&mut state, &update_check_path)? {
            LoopFlow::Quit => break,
            LoopFlow::SkipIteration => continue,
            LoopFlow::Proceed => {}
        }

        // Poll so the loop also wakes to check the background fetches, not only on input.
        if !event::poll(std::time::Duration::from_millis(150))? {
            continue;
        }
        let outcome = match event::read()? {
            Event::Key(key) => {
                // Windows reports both Press and Release (and Repeat) for each
                // keystroke, while Unix terminals report only Press. Without this
                // filter every key fires twice on Windows — Tab toggles focus back
                // to where it started and Up/Down jump two rows. See ratatui#347.
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if state.bg_task_msg.is_some() {
                    if key.code == KeyCode::Esc {
                        jobs.cancel_action(&mut state);
                    }
                    continue;
                }
                state.handle_key_with(key.code, key.modifiers)
            }
            Event::Mouse(m) if state.mouse_enabled => {
                if state.bg_task_msg.is_some() {
                    continue; // ignore mouse while a background task overlay is up, mirroring keys
                }
                let input = match m.kind {
                    MouseEventKind::ScrollUp => Some(super::MouseInput::ScrollUp),
                    MouseEventKind::ScrollDown => Some(super::MouseInput::ScrollDown),
                    MouseEventKind::Down(MouseButton::Left) => {
                        let prev = last_click.map(|(c, r, _)| (c, r));
                        let elapsed = last_click
                            .map(|(_, _, t)| t.elapsed().as_millis())
                            .unwrap_or(u128::MAX);
                        let classified = super::classify_click(prev, elapsed, m.column, m.row);
                        last_click = Some((m.column, m.row, std::time::Instant::now()));
                        Some(classified)
                    }
                    MouseEventKind::Down(MouseButton::Right) => {
                        Some(super::MouseInput::RightClick {
                            col: m.column,
                            row: m.row,
                        })
                    }
                    _ => None,
                };
                match input {
                    Some(i) => state.handle_mouse(i, &mouse_layout),
                    None => KeyOutcome::None,
                }
            }
            _ => KeyOutcome::None,
        };
        if let LoopFlow::Quit = dispatch_outcome(outcome, &mut state, terminal, &mut jobs)? {
            break;
        }
    }

    Ok(())
}
