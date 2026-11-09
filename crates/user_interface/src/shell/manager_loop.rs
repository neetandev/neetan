//! The event loop of the game manager. It waits for events and renders a
//! frame only after a pass of the user interface or a window change.

use std::{
    mem,
    time::{Duration, Instant},
};

use common::error;
use gui::component::run;
use sdl3::{
    event::{Event, EventPump},
    keyboard::Scancode,
};

use super::Shell;
use crate::{
    gui_host::{GuiEvent, GuiHost, keys::pointer_or_text_event, run_pending_passes},
    manager::{Launch, ShellEffect},
    texts::LanguageCatalog,
};

/// The longest wait for an event while nothing repeats.
const IDLE_WAIT: Duration = Duration::from_secs(1);

/// The wait before the next try of a stale frame.
const FRAME_RETRY_WAIT: Duration = Duration::from_millis(30);

/// The most stale frames in a row that the manager retries after a window
/// event.
pub(super) const MAXIMUM_STALE_FRAMES: u32 = 8;

/// Returns the longest wait for the next event at `now`: the time to the
/// next repeat `next_repeat` of a held gamepad direction or of the edge
/// repeat of a text drag, or the idle wait. A pass that the user interface
/// asked for runs without a wait. A pending frame in a visible window
/// shortens the wait to the retry wait. A window that cannot show a frame
/// waits for its next window event.
fn manager_wait(
    pass_pending: bool,
    frame_pending: bool,
    window_visible: bool,
    next_repeat: Option<Instant>,
    now: Instant,
) -> Duration {
    if pass_pending {
        return Duration::ZERO;
    }
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

/// Returns whether the chain of manager passes runs another pass: the
/// manager is open in `manager_open`, no session runs in `session_runs` and
/// the user interface asks for a pass in `pass_pending`.
fn manager_pass_due(manager_open: bool, session_runs: bool, pass_pending: bool) -> bool {
    manager_open && !session_runs && pass_pending
}

/// What the effects of the manager change in the shell.
trait ManagerEffectTarget {
    /// Whether a session runs.
    fn session_runs(&self) -> bool;

    /// Starts the game of `launch`.
    fn start_game(&mut self, launch: Launch);

    /// Switches the user interface to `catalog`.
    fn switch_catalog(&mut self, catalog: LanguageCatalog);
}

/// Applies `effects` to `target` in their order and leaves the buffer empty
/// with its capacity: a launch starts the game while no session runs, and a
/// catalog switch changes the language of the user interface.
fn apply_effects(target: &mut impl ManagerEffectTarget, effects: &mut Vec<ShellEffect>) {
    for effect in effects.drain(..) {
        match effect {
            ShellEffect::Launch(launch) => {
                if !target.session_runs() {
                    target.start_game(launch);
                }
            }
            ShellEffect::SwitchCatalog(catalog) => target.switch_catalog(catalog),
        }
    }
}

impl ManagerEffectTarget for Shell {
    fn session_runs(&self) -> bool {
        self.session.is_some()
    }

    fn start_game(&mut self, launch: Launch) {
        self.launch(launch);
    }

    fn switch_catalog(&mut self, catalog: LanguageCatalog) {
        if let Some(gui) = self.host.gui.as_mut() {
            gui.set_catalog(catalog);
        }
    }
}

impl Shell {
    /// Runs the manager until the application quits or a game starts.
    ///
    /// The events that wait from the end of a session reach the manager
    /// before its first frame, so the frame has the restored window size.
    /// The passes that the user interface asks for run before each frame, so
    /// the frame shows the settled state. A started game ends the chain of
    /// passes.
    pub(super) fn run_manager_loop(&mut self, event_pump: &mut EventPump) {
        for event in event_pump.poll_iter() {
            self.handle_event(&event);
        }
        self.host.update_gui_window();
        self.frame_pending = true;
        loop {
            if self.should_quit || self.session.is_some() {
                return;
            }
            if let Some(gui) = self.host.gui.as_mut()
                && let Some(repeat) = gui.gamepad.poll(Instant::now())
            {
                self.manager_gui_event(repeat);
            }
            if let Some(gui) = self.host.gui.as_mut() {
                gui.poll_edge_repeat(Instant::now());
            }
            run_pending_passes(|| {
                let due = manager_pass_due(
                    self.manager.is_some(),
                    ManagerEffectTarget::session_runs(self),
                    self.host.gui.as_ref().is_some_and(GuiHost::pending),
                );
                if due {
                    self.manager_pass();
                }
                due
            });
            if self.session.is_some() {
                return;
            }
            if self.frame_pending {
                let window_visible = self.window_visible;
                let mut stale_frames = self.stale_frames;
                self.frame_pending = frame_still_pending(window_visible, &mut stale_frames, || {
                    self.render_manager_frame()
                });
                self.stale_frames = stale_frames;
            }
            let next_repeat = self.host.gui.as_ref().and_then(|gui| {
                [gui.gamepad.next_repeat(), gui.next_edge_repeat()]
                    .into_iter()
                    .flatten()
                    .min()
            });
            let pass_pending = self.host.gui.as_ref().is_some_and(GuiHost::pending);
            let wait = manager_wait(
                pass_pending,
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
    /// Right Ctrl + Enter toggles fullscreen and Right Ctrl + Esc quits.
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
                        Scancode::Escape if !repeat => self.should_quit = true,
                        _ => {}
                    }
                    return;
                }
                self.host
                    .gui
                    .as_ref()
                    .and_then(|gui| gui.key_event(scancode, keymod, *repeat))
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
            _ => pointer_or_text_event(event, self.host.shift_held),
        };
        if let Some(gui_event) = gui_event {
            self.manager_gui_event(gui_event);
        }
    }

    /// Adds `gui_event` to the input of the next pass and runs the pass when
    /// the event is discrete.
    fn manager_gui_event(&mut self, gui_event: GuiEvent) {
        if self.manager.is_none() {
            return;
        }
        let Some(gui) = self.host.gui.as_mut() else {
            return;
        };
        if gui.apply(gui_event) {
            self.manager_pass();
        }
    }

    /// Runs one pass of the manager over the whole window and applies its
    /// effects.
    fn manager_pass(&mut self) {
        let (Some(gui), Some(manager)) = (self.host.gui.as_mut(), self.manager.as_mut()) else {
            return;
        };
        let effects = &mut self.manager_effects;
        gui.pass(|context| {
            let area = context.bounds();
            run(manager, context, area, (), &mut |effect| {
                effects.push(effect)
            })
        });
        gui.sync_text_input(&self.host.window);
        self.frame_pending = true;
        self.apply_manager_effects();
    }

    /// Applies the effects of the last manager pass in their order: a launch
    /// starts the game while no session runs, and a catalog switch changes
    /// the language of the user interface. A failed launch opens its error
    /// dialog before the next pass.
    fn apply_manager_effects(&mut self) {
        let mut effects = mem::take(&mut self.manager_effects);
        apply_effects(self, &mut effects);
        self.manager_effects = effects;
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
    use stack_vec::StackVec;

    use super::*;
    use crate::texts::test_catalog;

    /// Verifies that a pending frame retries soon only while the window can show it.
    #[test]
    fn a_pending_frame_retries_only_in_a_visible_window() {
        let now = Instant::now();
        assert_eq!(manager_wait(false, false, true, None, now), IDLE_WAIT);
        assert_eq!(manager_wait(false, true, true, None, now), FRAME_RETRY_WAIT);
        assert_eq!(manager_wait(false, true, false, None, now), IDLE_WAIT);
        assert_eq!(manager_wait(false, false, false, None, now), IDLE_WAIT);
    }

    /// Verifies that a pass that the user interface asked for runs without a wait.
    #[test]
    fn a_pending_pass_does_not_wait() {
        let now = Instant::now();
        assert_eq!(manager_wait(true, false, true, None, now), Duration::ZERO);
        assert_eq!(manager_wait(true, true, false, None, now), Duration::ZERO);
    }

    /// Verifies that a held gamepad direction shortens the wait to its next repeat.
    #[test]
    fn a_gamepad_repeat_shortens_the_wait() {
        let now = Instant::now();
        let soon = now + Duration::from_millis(5);
        assert_eq!(
            manager_wait(false, false, true, Some(soon), now),
            Duration::from_millis(5)
        );
        assert_eq!(
            manager_wait(false, true, true, Some(now + IDLE_WAIT), now),
            FRAME_RETRY_WAIT
        );
        assert_eq!(
            manager_wait(false, false, false, Some(now), now),
            Duration::ZERO
        );
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

    /// A call that [`RecordingShell`] received.
    #[derive(Debug, PartialEq)]
    enum Call {
        /// A game of this model started.
        StartGame(&'static str),
        /// The catalog switched to the English catalog.
        SwitchToEnglish,
        /// The catalog switched to a loaded catalog.
        SwitchToLoaded,
    }

    /// A shell that records the effects of the manager. A started game runs
    /// a session.
    #[derive(Default)]
    struct RecordingShell {
        /// The recorded calls in their order.
        calls: StackVec<Call, 4>,
        /// Whether a session runs.
        session_runs: bool,
    }

    impl ManagerEffectTarget for RecordingShell {
        fn session_runs(&self) -> bool {
            self.session_runs
        }

        fn start_game(&mut self, launch: Launch) {
            self.calls.push(Call::StartGame(launch.model));
            self.session_runs = true;
        }

        fn switch_catalog(&mut self, catalog: LanguageCatalog) {
            self.calls.push(match catalog {
                LanguageCatalog::English => Call::SwitchToEnglish,
                LanguageCatalog::Loaded(_) => Call::SwitchToLoaded,
            });
        }
    }

    /// A launch of a game of the model `model`.
    fn launch_effect(model: &'static str) -> ShellEffect {
        let system = machine_factory::config::systems::compiled_systems()
            .next()
            .expect("one system is compiled");
        ShellEffect::Launch(Launch {
            request: crate::config::LaunchRequest::default(),
            title: String::new(),
            system,
            model,
        })
    }

    /// Verifies that a launch starts its game.
    #[test]
    fn a_launch_starts_the_game() {
        let mut shell = RecordingShell::default();
        let mut effects = vec![launch_effect("Touhou")];
        apply_effects(&mut shell, &mut effects);
        assert_eq!(*shell.calls, [Call::StartGame("Touhou")]);
        assert!(shell.session_runs);
    }

    /// Verifies that a second launch in the same buffer is dropped while the session of the first runs.
    #[test]
    fn a_second_launch_is_dropped_while_the_first_session_runs() {
        let mut shell = RecordingShell::default();
        let mut effects = vec![launch_effect("First"), launch_effect("Second")];
        apply_effects(&mut shell, &mut effects);
        assert_eq!(*shell.calls, [Call::StartGame("First")]);
    }

    /// Verifies that the catalog switches in the order of the effects.
    #[test]
    fn a_catalog_switch_is_applied_in_its_order() {
        let mut shell = RecordingShell::default();
        let mut effects = vec![
            ShellEffect::SwitchCatalog(LanguageCatalog::Loaded(Box::new(test_catalog("")))),
            launch_effect("Game"),
            ShellEffect::SwitchCatalog(LanguageCatalog::English),
        ];
        apply_effects(&mut shell, &mut effects);
        assert_eq!(
            *shell.calls,
            [
                Call::SwitchToLoaded,
                Call::StartGame("Game"),
                Call::SwitchToEnglish,
            ]
        );
    }

    /// Verifies that the buffer is empty after the effects and keeps its capacity.
    #[test]
    fn the_buffer_is_empty_afterwards_and_keeps_its_capacity() {
        let mut shell = RecordingShell::default();
        let mut effects = Vec::with_capacity(8);
        effects.push(ShellEffect::SwitchCatalog(LanguageCatalog::English));
        effects.push(launch_effect("Game"));
        let capacity = effects.capacity();
        apply_effects(&mut shell, &mut effects);
        assert!(effects.is_empty());
        assert_eq!(effects.capacity(), capacity);
    }

    /// Runs the chain of manager passes over `shell` while the user interface
    /// asks for `requested_passes` passes. Each pass emits the effects of
    /// `pass_effects`. Returns the count of passes.
    fn run_manager_chain(
        shell: &mut RecordingShell,
        requested_passes: u32,
        mut pass_effects: impl FnMut(&mut Vec<ShellEffect>),
    ) -> u32 {
        let mut effects = Vec::new();
        let mut requests_left = requested_passes;
        run_pending_passes(|| {
            let due = manager_pass_due(true, shell.session_runs(), requests_left > 0);
            if due {
                requests_left -= 1;
                pass_effects(&mut effects);
                apply_effects(shell, &mut effects);
            }
            due
        })
    }

    /// Verifies that a pass that starts a game ends the chain of passes after that pass.
    #[test]
    fn a_pass_that_starts_a_game_ends_the_chain() {
        let mut shell = RecordingShell::default();
        let passes = run_manager_chain(&mut shell, 5, |effects| {
            effects.push(launch_effect("Game"));
        });
        assert_eq!(passes, 1);
        assert_eq!(*shell.calls, [Call::StartGame("Game")]);
    }

    /// Verifies that passes without a game keep the chain going while the user interface asks for one.
    #[test]
    fn a_pass_without_a_game_keeps_the_chain_going() {
        let mut shell = RecordingShell::default();
        let passes = run_manager_chain(&mut shell, 3, |effects| {
            effects.push(ShellEffect::SwitchCatalog(LanguageCatalog::English));
        });
        assert_eq!(passes, 3);
        assert_eq!(
            *shell.calls,
            [
                Call::SwitchToEnglish,
                Call::SwitchToEnglish,
                Call::SwitchToEnglish,
            ]
        );
        assert!(!shell.session_runs);
    }

    /// Verifies that the chain of manager passes runs a pass only while the manager is open, no session runs and the user interface asks for one.
    #[test]
    fn a_manager_pass_is_due_only_for_an_open_manager_without_a_session() {
        assert!(manager_pass_due(true, false, true));
        assert!(!manager_pass_due(false, false, true));
        assert!(!manager_pass_due(true, true, true));
        assert!(!manager_pass_due(true, false, false));
    }
}
