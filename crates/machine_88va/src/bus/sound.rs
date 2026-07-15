//! PC-88VA2 sound board (YM2608 / OPNA) I/O and the mouse or joystick data
//! presented on the chip's SSG I/O ports.
//!
//! The VA "Sound board 2" is a bare YM2608 at I/O 0x044-0x047. Its timer interrupt
//! routes to the 8259 slave IR4 (master-relative IRQ 12), gated by the FM interrupt
//! mask in system port 0x032 bit 7 (set = masked). The mouse and joystick share
//! SSG registers 0x0E (data nibble / directions) and 0x0F (buttons / triggers):
//! the VA connects one device at a time, so the port reports either the mouse or
//! the joystick, picked by whichever device last got input.

use common::JoystickState;
use device::opn_fm::FmTimerAction;

use super::{
    JOYSTICK_DOWN, JOYSTICK_LEFT, JOYSTICK_RIGHT, JOYSTICK_TRIGGER1, JOYSTICK_TRIGGER2,
    JOYSTICK_UP, Pc88VaBus,
};
use crate::scheduler::Event88Va;

impl<T: common::TraceSink> Pc88VaBus<T> {
    /// Reads the OPNA low-bank status (port 0x044).
    pub(crate) fn read_opn_status(&mut self) -> u8 {
        self.soundboard.read_status(self.current_cycle)
    }

    /// Reads the addressed OPNA low-bank register (port 0x045). SSG registers
    /// 0x0E/0x0F return the mouse nibble and buttons.
    pub(crate) fn read_opn_data(&mut self) -> u8 {
        self.update_joyport();
        self.soundboard.read_data(self.current_cycle)
    }

    /// Reads the OPNA high-bank status (port 0x046).
    pub(crate) fn read_opn_status_hi(&mut self) -> u8 {
        self.soundboard.read_status_hi(self.current_cycle)
    }

    /// Reads the addressed OPNA high-bank register (port 0x047).
    pub(crate) fn read_opn_data_hi(&mut self) -> u8 {
        self.soundboard.read_data_hi(self.current_cycle)
    }

    /// Latches the OPNA low-bank register address (port 0x044 write).
    pub(crate) fn write_opn_address(&mut self, value: u8) {
        self.soundboard.write_address(value, self.current_cycle);
        self.apply_sound_timers();
    }

    /// Writes the addressed OPNA low-bank register (port 0x045 write).
    pub(crate) fn write_opn_data(&mut self, value: u8) {
        self.soundboard.write_data(value, self.current_cycle);
        self.apply_sound_timers();
    }

    /// Latches the OPNA high-bank register address (port 0x046 write).
    pub(crate) fn write_opn_address_hi(&mut self, value: u8) {
        self.soundboard.write_address_hi(value, self.current_cycle);
        self.apply_sound_timers();
    }

    /// Writes the addressed OPNA high-bank register (port 0x047 write).
    pub(crate) fn write_opn_data_hi(&mut self, value: u8) {
        self.soundboard.write_data_hi(value, self.current_cycle);
        self.apply_sound_timers();
    }

    /// Presents the selected device's lines on the chip (SSG registers
    /// 0x0E/0x0F). Port A carries the mouse readout nibble or the joystick
    /// directions; port B the mouse buttons or the joystick triggers, depending
    /// on which device is currently connected to the shared port.
    pub(crate) fn update_joyport(&mut self) {
        let (port_a, port_b) = if self.joystick_selected {
            (self.joystick_port_a, self.joystick_port_b)
        } else {
            (
                self.mouse.data_nibble() | 0xF0,
                self.mouse.button_bits() | 0xFC,
            )
        };
        self.soundboard.set_joyport(port_a, port_b);
    }

    /// Steps the mouse nibble machine from a system-port-0x040 strobe (bit 6) and
    /// refreshes the SSG readback.
    pub(crate) fn mouse_strobe(&mut self, port040: u8) {
        self.mouse.strobe((port040 >> 6) & 0x01);
        self.update_joyport();
    }

    /// Feeds a relative mouse movement reported by the host. Real motion selects
    /// the mouse as the device on the shared port.
    pub(crate) fn push_mouse_delta(&mut self, delta_x: i16, delta_y: i16) {
        if delta_x != 0 || delta_y != 0 {
            self.joystick_selected = false;
        }
        self.mouse.push_delta(delta_x, delta_y);
    }

    /// Sets the mouse button state (the VA mouse has no middle button). A press
    /// selects the mouse as the device on the shared port.
    pub(crate) fn set_mouse_buttons(&mut self, left: bool, right: bool) {
        if left || right {
            self.joystick_selected = false;
        }
        self.mouse.set_buttons(left, right);
        self.update_joyport();
    }

    /// Sets the digital joystick state. The joystick shares the OPN port A/B read
    /// lines with the mouse (the same physical connector); any held direction or
    /// trigger selects the joystick as the device on the shared port.
    pub(crate) fn set_joystick(&mut self, state: JoystickState) {
        let mut port_a = 0xFFu8;
        if state.up {
            port_a &= !JOYSTICK_UP;
        }
        if state.down {
            port_a &= !JOYSTICK_DOWN;
        }
        if state.left {
            port_a &= !JOYSTICK_LEFT;
        }
        if state.right {
            port_a &= !JOYSTICK_RIGHT;
        }
        let mut port_b = 0xFFu8;
        if state.trigger1 {
            port_b &= !JOYSTICK_TRIGGER1;
        }
        if state.trigger2 {
            port_b &= !JOYSTICK_TRIGGER2;
        }
        self.joystick_port_a = port_a;
        self.joystick_port_b = port_b;
        if state != JoystickState::default() {
            self.joystick_selected = true;
        }
        self.update_joyport();
    }

    /// Drains the chip's pending FM-timer schedule/cancel requests onto the
    /// scheduler and reconciles the OPNA IRQ output, mirroring `machine_88`.
    pub(crate) fn apply_sound_timers(&mut self) {
        let timers: [Option<FmTimerAction>; 2] = {
            let actions = self.soundboard.drain_timers();
            let mut slots: [Option<FmTimerAction>; 2] = [None, None];
            for action in actions {
                let id = match action {
                    FmTimerAction::Schedule { timer_id, .. } => *timer_id,
                    FmTimerAction::Cancel { timer_id } => *timer_id,
                };
                if (id as usize) < slots.len() {
                    slots[id as usize] = Some(*action);
                }
            }
            slots
        };
        for action in timers.into_iter().flatten() {
            let (timer_id, fire_cycle) = match action {
                FmTimerAction::Schedule {
                    timer_id,
                    fire_cycle,
                } => (timer_id, Some(fire_cycle)),
                FmTimerAction::Cancel { timer_id } => (timer_id, None),
            };
            let kind = if timer_id == 0 {
                Event88Va::OpnaTimerA
            } else {
                Event88Va::OpnaTimerB
            };
            match fire_cycle {
                Some(cycle) => self.scheduler.schedule(kind, cycle),
                None => self.scheduler.cancel(kind),
            }
        }
        if self.soundboard.take_irq_change().is_some() {
            self.recompute_sound_irq();
        }
        self.update_next_event_cycle();
    }

    /// Routes the OPNA IRQ output to the 8259 slave IR4 (IRQ 12), masked when the
    /// FM interrupt mask (system port 0x032 bit 7) is set.
    pub(crate) fn recompute_sound_irq(&mut self) {
        let mask_open = self.sysport.port032 & 0x80 == 0;
        if self.soundboard.irq_asserted() && mask_open {
            self.pic.set_irq(12);
        } else {
            self.pic.clear_irq(12);
        }
    }

    /// Generates resampled OPNA audio into `output` and reconciles the timers.
    pub(crate) fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        self.soundboard.generate_samples(
            self.current_cycle,
            self.clocks.main_clock_hz,
            volume,
            output,
        );
        self.apply_sound_timers();
        output.len()
    }
}
