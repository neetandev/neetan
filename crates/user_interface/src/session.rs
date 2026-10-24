//! One running game: the machine, its media, its input state and its media
//! dialog.

mod display;
mod emulation;
mod input_events;
mod media;
mod panic_guard;
mod removable_media;
mod snapshot;
#[cfg(test)]
mod test_support;

use std::time::Instant;

use common::{Error, JoystickState, Machine, StringError, error, info, warn};

pub(crate) use self::panic_guard::catch_panic;
use crate::{
    Result,
    config::{AspectMode, Backend, EmulatorConfig, ScalingMode, WindowMode},
    host::Host,
    input::{KeyOverrides, KeyboardForwardingState},
    machines::{initialize_machine, sdl_host_date_time_source},
    media_dialog::{ImageEntry, MediaDialog},
};

/// Analog-stick magnitude past which a left-stick axis counts as a held direction.
const GAMEPAD_AXIS_DEADZONE: i16 = 16384;

/// The reason a session ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionExit {
    /// The user pressed Right Ctrl + Q.
    Quit,
    /// The guest requested a system shutdown.
    GuestShutdown,
    /// Emulator code panicked.
    Panic {
        /// The message of the panic.
        message: String,
    },
}

/// One running game.
pub(crate) struct Session {
    /// The configuration of the game.
    config: EmulatorConfig,
    /// The emulated machine.
    machine: Box<dyn Machine>,
    /// The single in-memory runtime snapshot.
    runtime_snapshot: Option<save_state::MachineStateBlob>,
    /// The speed of the CPU on cycles per second.
    cpu_hz: f64,
    /// Tracks CPU cycle overshoot from previous audio steps for precise timing.
    cycle_overshoot: u64,
    /// Accumulated mouse X delta since last frame sync (sub-pixel).
    mouse_dx: f32,
    /// Accumulated mouse Y delta since last frame sync (sub-pixel).
    mouse_dy: f32,
    /// Whether the left mouse button is held.
    mouse_left: bool,
    /// Whether the right mouse button is held.
    mouse_right: bool,
    /// Whether the middle mouse button is held.
    mouse_middle: bool,
    /// Whether relative mouse mode is active (Right Ctrl + M toggles).
    mouse_captured: bool,
    /// Joystick directions and triggers from the gamepad d-pad and face buttons.
    gamepad_buttons: JoystickState,
    /// Left analog stick X position (range -32768 to 32767).
    gamepad_axis_x: i16,
    /// Left analog stick Y position (range -32768 to 32767).
    gamepad_axis_y: i16,
    /// Joystick state from the keyboard fallback (used when no gamepad is connected).
    keyboard_joystick: JoystickState,
    /// Per-scancode native key-code overrides from `key.*` config bindings.
    key_overrides: KeyOverrides,
    /// The keys held on the machine keyboard and the pending key actions.
    keyboard_forwarding_state: KeyboardForwardingState,
    /// Floppy disk image entries for drive 1.
    fdd1_entries: Vec<ImageEntry>,
    /// Current index into fdd1_entries, or `None` if no floppy is loaded.
    fdd1_index: Option<usize>,
    /// Floppy disk image entries for drive 2.
    fdd2_entries: Vec<ImageEntry>,
    /// Current index into fdd2_entries, or `None` if no floppy is loaded.
    fdd2_index: Option<usize>,
    /// CD-ROM disc image entries.
    cdrom_entries: Vec<ImageEntry>,
    /// Current index into cdrom_entries, or `None` if no disc is loaded.
    cdrom_index: Option<usize>,
    /// The open media dialog.
    media_dialog: Option<MediaDialog>,
    /// Whether the CRT effect is enabled.
    crt_enabled: bool,
    /// Composite subcarrier phase select (0..3), cycled with Right Ctrl + P. It
    /// swaps the PC-6001 artifact-color pair of the composite video.
    composite_phase: u32,
    /// Scaling method of the native texture.
    scaling: ScalingMode,
    /// Whether fast forward is currently active (held).
    fast_forward: bool,
    /// Wall-clock instant of the last fast-forward emulation step, used to pace
    /// emulation at a fixed multiple of real time.
    last_emulation_tick: Instant,
    /// The reason the session ends, once one is known.
    exit: Option<SessionExit>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let machine = self.machine.as_mut();
        flush_guarded("printer", || machine.flush_printer());
        flush_guarded("cartridges", || machine.flush_cartridges());
        flush_guarded("floppy disks", || machine.flush_floppies());
        flush_guarded("hard disks", || machine.flush_hdds());
    }
}

/// Runs one flush of the machine and logs a panic of it.
fn flush_guarded(what: &str, flush: impl FnOnce()) {
    if let Err(message) = catch_panic(flush) {
        error!("Flushing the {what} panicked: {message}");
    }
}

impl Session {
    /// Creates the machine of `config` and inserts the first image of every
    /// drive. `backend` decides whether the CRT effect is available.
    pub(crate) fn new(
        config: EmulatorConfig,
        key_overrides: KeyOverrides,
        backend: Backend,
    ) -> Result<Self> {
        let fdd1_entries: Vec<ImageEntry> =
            config.fdd1.iter().cloned().map(ImageEntry::new).collect();
        let fdd2_entries: Vec<ImageEntry> =
            config.fdd2.iter().cloned().map(ImageEntry::new).collect();
        let cdrom_entries: Vec<ImageEntry> =
            config.cdrom.iter().cloned().map(ImageEntry::new).collect();

        let mut machine = initialize_machine(
            &config,
            audio_engine::SAMPLE_RATE as u32,
            sdl_host_date_time_source(),
        )?;

        if config.enable_extractor {
            machine.install_text_extractor(Box::new(text_extractor::ClipboardExtractor::new()));
            info!("Text extractor enabled (clipboard sink)");
        }

        let mut fdd1_index = None;
        if let Some(entry) = fdd1_entries.first() {
            match machine.insert_floppy_from_path(0, &entry.path) {
                Ok(desc) => {
                    info!("Inserted FDD1: {desc} from {}", entry.path.display());
                    fdd1_index = Some(0);
                }
                Err(e) => return Err(Error::from(StringError(e))),
            }
        }
        let mut fdd2_index = None;
        if let Some(entry) = fdd2_entries.first() {
            match machine.insert_floppy_from_path(1, &entry.path) {
                Ok(desc) => {
                    info!("Inserted FDD2: {desc} from {}", entry.path.display());
                    fdd2_index = Some(0);
                }
                Err(e) => return Err(Error::from(StringError(e))),
            }
        }
        let mut cdrom_index = None;
        if let Some(entry) = cdrom_entries.first() {
            match machine.insert_cdrom(&entry.path) {
                Ok(desc) => {
                    info!("Inserted CD-ROM: {desc} from {}", entry.path.display());
                    cdrom_index = Some(0);
                }
                Err(e) => return Err(Error::from(StringError(e))),
            }
        }

        let cpu_hz = machine.cpu_clock_hz();

        Ok(Self {
            machine,
            runtime_snapshot: None,
            cpu_hz,
            cycle_overshoot: 0,
            mouse_dx: 0.0,
            mouse_dy: 0.0,
            mouse_left: false,
            mouse_right: false,
            mouse_middle: false,
            mouse_captured: false,
            gamepad_buttons: JoystickState::default(),
            gamepad_axis_x: 0,
            gamepad_axis_y: 0,
            keyboard_joystick: JoystickState::default(),
            key_overrides,
            keyboard_forwarding_state: KeyboardForwardingState::new(),
            fdd1_entries,
            fdd1_index,
            fdd2_entries,
            fdd2_index,
            cdrom_entries,
            cdrom_index,
            media_dialog: None,
            crt_enabled: config.crt && backend == Backend::Modern,
            composite_phase: config.pc60_composite_phase,
            scaling: config.scaling,
            fast_forward: false,
            last_emulation_tick: Instant::now(),
            exit: None,
            config,
        })
    }

    /// The aspect mode of the game.
    pub(crate) fn aspect_mode(&self) -> AspectMode {
        self.config.aspect_mode
    }

    /// The window mode of the game.
    pub(crate) fn window_mode(&self) -> WindowMode {
        self.config.window_mode
    }

    /// Whether the configuration asks for the CRT effect.
    pub(crate) fn crt_requested(&self) -> bool {
        self.config.crt
    }

    /// Whether the CRT effect is enabled.
    pub(crate) fn crt_enabled(&self) -> bool {
        self.crt_enabled
    }

    /// The scaling method of the native texture.
    pub(crate) fn scaling(&self) -> ScalingMode {
        self.scaling
    }

    /// Whether the media dialog is open.
    pub(crate) fn media_dialog_open(&self) -> bool {
        self.media_dialog.is_some()
    }

    /// The reason the session ends, once one is known.
    pub(crate) fn exit(&self) -> Option<&SessionExit> {
        self.exit.as_ref()
    }

    /// Ends the session with `exit` unless it already has a reason.
    fn request_exit(&mut self, exit: SessionExit) {
        if self.exit.is_none() {
            self.exit = Some(exit);
        }
    }

    /// Whether emulator code of this session panicked.
    fn panicked(&self) -> bool {
        matches!(self.exit, Some(SessionExit::Panic { .. }))
    }

    /// Ends the session after a panic with `message`.
    pub(crate) fn record_panic(&mut self, message: String) {
        error!("The emulation stopped after a panic: {message}");
        self.exit = Some(SessionExit::Panic { message });
    }

    /// Runs `call` on the session and catches a panic of it. A panic ends the
    /// session. After a panic no further call runs and the result is `None`.
    /// The session state after a panic is never used for emulation again.
    /// The only machine calls after it are the flushes of the teardown.
    pub(crate) fn guarded<R>(&mut self, call: impl FnOnce(&mut Self) -> R) -> Option<R> {
        if self.panicked() {
            return None;
        }
        match catch_panic(|| call(self)) {
            Ok(result) => Some(result),
            Err(message) => {
                self.record_panic(message);
                None
            }
        }
    }

    /// Runs the pending pass of the open media dialog and the gamepad key
    /// repeat of the GUI.
    pub(crate) fn update_dialog(&mut self, host: &mut Host) {
        if self.media_dialog.is_none() {
            return;
        }
        let repeat = host
            .gui
            .as_mut()
            .and_then(|gui| gui.gamepad.poll(Instant::now()));
        if let Some(gui_event) = repeat {
            self.handle_gui_event(gui_event, host);
        }
        if host.gui.as_ref().is_some_and(|gui| gui.pending()) {
            self.media_pass(host);
        }
    }

    /// Stops fast forward when the window loses the focus.
    pub(crate) fn focus_lost(&mut self, host: &mut Host) {
        self.set_fast_forward(false, host);
    }

    /// Releases the gamepad state after the gamepad disconnected.
    pub(crate) fn gamepad_disconnected(&mut self, host: &Host) {
        self.gamepad_buttons = JoystickState::default();
        self.gamepad_axis_x = 0;
        self.gamepad_axis_y = 0;
        self.update_joystick(host);
    }

    /// Recomputes the effective joystick state and pushes it to the machine.
    /// A connected gamepad takes precedence over the keyboard fallback.
    fn update_joystick(&mut self, host: &Host) {
        let connected = host.gamepad.is_some();
        let state = if connected {
            let mut state = self.gamepad_buttons;
            state.left |= self.gamepad_axis_x <= -GAMEPAD_AXIS_DEADZONE;
            state.right |= self.gamepad_axis_x >= GAMEPAD_AXIS_DEADZONE;
            state.up |= self.gamepad_axis_y <= -GAMEPAD_AXIS_DEADZONE;
            state.down |= self.gamepad_axis_y >= GAMEPAD_AXIS_DEADZONE;
            state
        } else {
            self.keyboard_joystick
        };
        self.machine.set_joystick(0, state);
        let axes = connected.then_some((self.gamepad_axis_x, self.gamepad_axis_y));
        self.machine.set_joystick_axes(0, axes);
    }

    /// Toggles mouse capture (relative mouse mode) on the window.
    fn toggle_mouse_capture(&mut self, host: &Host) {
        let desired = !self.mouse_captured;

        if let Err(error) = host.window.set_relative_mouse_mode(desired) {
            warn!("Failed to set relative mouse mode: {error}");
            return;
        }

        self.mouse_captured = desired;

        if !self.mouse_captured {
            self.release_mouse_buttons();
        }

        info!(
            "Mouse {}",
            if self.mouse_captured {
                "captured"
            } else {
                "released"
            }
        );
    }

    /// Releases the held mouse buttons of the machine and drops the pending
    /// mouse movement.
    fn release_mouse_buttons(&mut self) {
        self.mouse_left = false;
        self.mouse_right = false;
        self.mouse_middle = false;
        self.machine.set_mouse_buttons(false, false, false);
        self.mouse_dx = 0.0;
        self.mouse_dy = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SessionExit,
        test_support::{CallLog, FakeMachine, fake_machine, session_with},
    };

    #[test]
    fn a_panic_ends_the_session_and_skips_later_calls() {
        let log = CallLog::default();
        let mut session = session_with(fake_machine(&log));

        assert_eq!(session.guarded(|_| 3), Some(3));
        assert_eq!(session.exit(), None);

        let result: Option<()> = session.guarded(|_| panic!("opcode 0x0F is broken"));
        assert_eq!(result, None);
        assert_eq!(
            session.exit(),
            Some(&SessionExit::Panic {
                message: "opcode 0x0F is broken".to_owned()
            })
        );

        let mut ran = false;
        assert_eq!(session.guarded(|_| ran = true), None);
        assert!(!ran);
    }

    #[test]
    fn a_panic_replaces_an_earlier_exit_reason() {
        let log = CallLog::default();
        let mut session = session_with(fake_machine(&log));
        session.request_exit(SessionExit::Quit);
        session.request_exit(SessionExit::GuestShutdown);
        assert_eq!(session.exit(), Some(&SessionExit::Quit));

        let _: Option<()> = session.guarded(|_| panic!("late panic"));
        assert_eq!(
            session.exit(),
            Some(&SessionExit::Panic {
                message: "late panic".to_owned()
            })
        );
    }

    #[test]
    fn dropping_the_session_flushes_every_medium_once() {
        let log = CallLog::default();
        drop(session_with(fake_machine(&log)));
        assert_eq!(
            *log.borrow(),
            ["printer", "cartridges", "floppies", "hard disks"]
        );
    }

    #[test]
    fn a_session_after_a_panic_still_flushes_every_medium() {
        let log = CallLog::default();
        let mut session = session_with(fake_machine(&log));
        let _: Option<()> = session.guarded(|_| panic!("core halted"));
        assert!(session.panicked());
        drop(session);
        assert_eq!(
            *log.borrow(),
            ["printer", "cartridges", "floppies", "hard disks"]
        );
    }

    #[test]
    fn a_panicking_flush_does_not_stop_the_other_flushes() {
        let log = CallLog::default();
        let machine = FakeMachine {
            panic_on_floppy_flush: true,
            ..fake_machine(&log)
        };
        drop(session_with(machine));
        assert_eq!(
            *log.borrow(),
            ["printer", "cartridges", "floppies", "hard disks"]
        );
    }
}
