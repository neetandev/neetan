//! A fake machine and a session over it for the session tests.

use std::{cell::RefCell, rc::Rc, time::Instant};

use common::{JoystickState, Machine, MediaBacking, MediaImage};

use super::Session;
use crate::{
    config::{EmulatorConfig, ScalingMode},
    input::{KeyOverrides, KeyboardForwardingState},
};

/// The calls a fake machine records.
pub(super) type CallLog = Rc<RefCell<Vec<&'static str>>>;

/// A machine that records its flushes and panics on request.
pub(super) struct FakeMachine {
    /// The recorded calls.
    pub(super) log: CallLog,
    /// The frame of the machine.
    pub(super) framebuffer: Vec<u8>,
    /// Whether flushing the floppy disks panics.
    pub(super) panic_on_floppy_flush: bool,
    /// Whether reading the frame panics.
    pub(super) panic_on_framebuffer: bool,
}

impl Machine for FakeMachine {
    fn cpu_clock_hz(&self) -> f64 {
        8_000_000.0
    }

    fn run_for(&mut self, budget: u64) -> u64 {
        budget
    }

    fn shutdown_requested(&self) -> bool {
        false
    }

    fn display_framebuffer(&self) -> &[u8] {
        assert!(!self.panic_on_framebuffer, "the frame is unreadable");
        &self.framebuffer
    }

    fn display_dimensions(&self) -> (u32, u32) {
        (2, 1)
    }

    fn push_keyboard_scancode(&mut self, _code: u8) {}

    fn generate_audio_samples(&mut self, _volume: f32, _output: &mut [f32]) -> usize {
        0
    }

    fn insert_floppy(
        &mut self,
        _drive: usize,
        _image: MediaImage<'_>,
        _backing: MediaBacking,
    ) -> Result<String, String> {
        Err("no floppy drive".to_owned())
    }

    fn eject_floppy(&mut self, _drive: usize) {}

    fn flush_floppies(&mut self) {
        self.log.borrow_mut().push("floppies");
        assert!(!self.panic_on_floppy_flush, "the floppy image is gone");
    }

    fn flush_hdds(&mut self) {
        self.log.borrow_mut().push("hard disks");
    }

    fn flush_cartridges(&mut self) {
        self.log.borrow_mut().push("cartridges");
    }

    fn flush_printer(&mut self) {
        self.log.borrow_mut().push("printer");
    }
}

/// A session over `machine` without media.
pub(super) fn session_with(machine: FakeMachine) -> Session {
    let config = EmulatorConfig::default();
    Session {
        machine: Box::new(machine),
        runtime_snapshot: None,
        cpu_hz: 8_000_000.0,
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
        key_overrides: KeyOverrides::new(),
        keyboard_forwarding_state: KeyboardForwardingState::new(),
        fdd1_entries: Vec::new(),
        fdd1_index: None,
        fdd2_entries: Vec::new(),
        fdd2_index: None,
        cdrom_entries: Vec::new(),
        cdrom_index: None,
        media_dialog: None,
        crt_enabled: false,
        composite_phase: 0,
        scaling: ScalingMode::Nearest,
        fast_forward: false,
        last_emulation_tick: Instant::now(),
        exit: None,
        config,
    }
}

/// A fake machine that records into `log`.
pub(super) fn fake_machine(log: &CallLog) -> FakeMachine {
    FakeMachine {
        log: Rc::clone(log),
        framebuffer: vec![0; 8],
        panic_on_floppy_flush: false,
        panic_on_framebuffer: false,
    }
}
