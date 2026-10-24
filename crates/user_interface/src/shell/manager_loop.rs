//! The event loop of the game manager. It waits for events and renders a
//! frame only after a pass of the user interface or a window change.

use std::time::{Duration, Instant};

use common::error;
use sdl3::{
    event::{Event, EventPump},
    keyboard::Scancode,
};

use super::Shell;
use crate::{
    gui_host::{GuiEvent, GuiHost, key_event, pointer_or_text_event},
    manager::Launch,
};

/// The longest wait for an event while no gamepad direction repeats.
const IDLE_WAIT: Duration = Duration::from_secs(1);

/// The wait before the next try of a stale frame.
const FRAME_RETRY_WAIT: Duration = Duration::from_millis(30);

/// The most stale frames in a row that the manager retries after a window
/// event.
pub(super) const MAXIMUM_STALE_FRAMES: u32 = 8;

/// Returns the longest wait for the next event at `now`: the time to the
/// next repeat of a held gamepad direction, or the idle wait. A pending
/// frame in a visible window shortens the wait to the retry wait. A window
/// that cannot show a frame waits for its next window event.
fn manager_wait(
    frame_pending: bool,
    window_visible: bool,
    next_repeat: Option<Instant>,
    now: Instant,
) -> Duration {
    let longest = if frame_pending && window_visible {
        FRAME_RETRY_WAIT
    } else {
        IDLE_WAIT
    };
    next_repeat.map_or(longest, |next| {
        next.saturating_duration_since(now).min(longest)
    })
}

/// Returns whether the manager still needs a frame after a render try.
///
/// A hidden window skips the render and keeps the frame pending for its
/// return. `render` draws the frame and returns whether it is stale. A
/// stale frame stays pending until `stale_frames`, the count of stale
/// frames in a row, reaches [`MAXIMUM_STALE_FRAMES`]. A frame at the size
/// of the window resets the count.
fn frame_still_pending(
    window_visible: bool,
    stale_frames: &mut u32,
    render: impl FnOnce() -> bool,
) -> bool {
    if !window_visible {
        return true;
    }
    if !render() {
        *stale_frames = 0;
        return false;
    }
    *stale_frames += 1;
    *stale_frames < MAXIMUM_STALE_FRAMES
}

impl Shell {
    /// Runs the manager until the application quits or the user starts a
    /// game. Returns the game to start, or `None` when the application quits.
    ///
    /// The events that wait from the end of a session reach the manager
    /// before its first frame, so the frame has the restored window size.
    pub(super) fn run_manager_loop(&mut self, event_pump: &mut EventPump) -> Option<Launch> {
        for event in event_pump.poll_iter() {
            self.handle_event(&event);
        }
        self.host.update_gui_window();
        self.frame_pending = true;
        loop {
            if self.should_quit {
                return None;
            }
            if let Some(gui) = self.host.gui.as_mut()
                && let Some(repeat) = gui.gamepad.poll(Instant::now())
            {
                self.manager_gui_event(repeat);
            }
            if self.host.gui.as_ref().is_some_and(GuiHost::pending) {
                self.manager_pass();
            }
            if let Some(launch) = self.pending_launch.take() {
                return Some(launch);
            }
            if self.frame_pending {
                let window_visible = self.window_visible;
                let mut stale_frames = self.stale_frames;
                self.frame_pending = frame_still_pending(window_visible, &mut stale_frames, || {
                    self.render_manager_frame()
                });
                self.stale_frames = stale_frames;
            }
            let next_repeat = self
                .host
                .gui
                .as_ref()
                .and_then(|gui| gui.gamepad.next_repeat());
            let wait = manager_wait(
                self.frame_pending,
                self.window_visible,
                next_repeat,
                Instant::now(),
            );
            if let Some(event) = event_pump.wait_event_timeout(wait) {
                self.handle_event(&event);
                for event in event_pump.poll_iter() {
                    self.handle_event(&event);
                }
            }
        }
    }

    /// Handles a keyboard, text, mouse or gamepad event of the manager.
    /// Right Ctrl + Enter toggles fullscreen and Right Ctrl + Q quits.
    pub(super) fn handle_manager_input(&mut self, event: &Event) {
        let gui_event = match event {
            Event::KeyDown {
                scancode,
                keymod,
                repeat,
                ..
            } => {
                let Some(scancode) = *scancode else {
                    return;
                };
                if keymod.rctrl() {
                    match scancode {
                        Scancode::Return if !repeat => {
                            let fullscreen = !self.host.fullscreen;
                            self.host.set_fullscreen(fullscreen);
                        }
                        Scancode::Q if !repeat => self.should_quit = true,
                        _ => {}
                    }
                    return;
                }
                key_event(scancode, keymod, *repeat)
            }
            Event::TextInput { .. } if self.host.right_control_held => None,
            Event::GamepadButtonDown { which, button }
            | Event::GamepadButtonUp { which, button } => {
                if !self.host.is_active_gamepad(*which) {
                    return;
                }
                let pressed = matches!(event, Event::GamepadButtonDown { .. });
                self.host
                    .gui
                    .as_mut()
                    .and_then(|gui| gui.gamepad.button(*button, pressed, Instant::now()))
            }
            Event::GamepadAxisMotion { which, axis, value } => {
                if !self.host.is_active_gamepad(*which) {
                    return;
                }
                self.host
                    .gui
                    .as_mut()
                    .and_then(|gui| gui.gamepad.axis(*axis, *value, Instant::now()))
            }
            _ => pointer_or_text_event(event),
        };
        if let Some(gui_event) = gui_event {
            self.manager_gui_event(gui_event);
        }
    }

    /// Adds `gui_event` to the input of the next pass and runs the pass when
    /// the event is discrete.
    fn manager_gui_event(&mut self, gui_event: GuiEvent) {
        let accepted = self
            .manager
            .as_ref()
            .is_some_and(|manager| manager.accepts(&gui_event));
        if !accepted {
            return;
        }
        let Some(gui) = self.host.gui.as_mut() else {
            return;
        };
        if gui.apply(gui_event) {
            self.manager_pass();
        }
    }

    /// Runs one pass of the manager and keeps the game it asks to start.
    fn manager_pass(&mut self) {
        let (Some(gui), Some(manager)) = (self.host.gui.as_mut(), self.manager.as_mut()) else {
            return;
        };
        let launch = gui.pass(|context| manager.show(context));
        gui.sync_text_input(&self.host.window);
        self.frame_pending = true;
        if launch.is_some() {
            self.pending_launch = launch;
        }
    }

    /// Renders the scene of the last pass of the manager over black.
    /// Returns whether the frame is stale. A failed frame counts as done,
    /// so an engine error ends in one log line for each pass.
    fn render_manager_frame(&mut self) -> bool {
        let host = &mut self.host;
        let overlay = host.gui.as_ref().map(GuiHost::overlay);
        host.graphics_engine
            .render_frame(&host.window, None, overlay)
            .unwrap_or_else(|error| {
                error!("Failed to render the manager frame: {error:#}");
                false
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that a pending frame retries soon only while the window can show it.
    #[test]
    fn a_pending_frame_retries_only_in_a_visible_window() {
        let now = Instant::now();
        assert_eq!(manager_wait(false, true, None, now), IDLE_WAIT);
        assert_eq!(manager_wait(true, true, None, now), FRAME_RETRY_WAIT);
        assert_eq!(manager_wait(true, false, None, now), IDLE_WAIT);
        assert_eq!(manager_wait(false, false, None, now), IDLE_WAIT);
    }

    /// Verifies that a held gamepad direction shortens the wait to its next repeat.
    #[test]
    fn a_gamepad_repeat_shortens_the_wait() {
        let now = Instant::now();
        let soon = now + Duration::from_millis(5);
        assert_eq!(
            manager_wait(false, true, Some(soon), now),
            Duration::from_millis(5)
        );
        assert_eq!(
            manager_wait(true, true, Some(now + IDLE_WAIT), now),
            FRAME_RETRY_WAIT
        );
        assert_eq!(manager_wait(false, false, Some(now), now), Duration::ZERO);
    }

    /// Verifies that a hidden window does not render and keeps its frame pending.
    #[test]
    fn a_hidden_window_keeps_its_frame_pending() {
        let mut stale_frames = 0;
        let mut rendered = false;
        assert!(frame_still_pending(false, &mut stale_frames, || {
            rendered = true;
            false
        }));
        assert!(!rendered);
        assert_eq!(stale_frames, 0);
    }

    /// Verifies that stale frames retry up to the bound and that a frame at the window size resets the count.
    #[test]
    fn stale_frames_retry_up_to_the_bound() {
        let mut stale_frames = 0;
        for retry in 1..MAXIMUM_STALE_FRAMES {
            assert!(frame_still_pending(true, &mut stale_frames, || true));
            assert_eq!(stale_frames, retry);
        }
        assert!(!frame_still_pending(true, &mut stale_frames, || true));
        assert!(!frame_still_pending(true, &mut stale_frames, || true));
        assert!(!frame_still_pending(true, &mut stale_frames, || false));
        assert_eq!(stale_frames, 0);
        assert!(frame_still_pending(true, &mut stale_frames, || true));
    }
}
