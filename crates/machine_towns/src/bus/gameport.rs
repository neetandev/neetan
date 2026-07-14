//! FM Towns game port (I/O 0x04D0/0x04D2/0x04D6).
//!
//! Port 0 (0x04D0) carries a 2/6-button pad; port 1 (0x04D2) carries the
//! MSX-style relative mouse. The output port (0x04D6) drives each port's COM
//! strobe and trigger lines.
//!
//! The mouse reports a signed 8-bit X/Y delta as four nibbles clocked out by
//! toggling the COM line: X-high, X-low, Y-high, Y-low. The pad reports its
//! directions and buttons active-low; the 6-button pad multiplexes the extra
//! buttons on the COM line.

use common::JoystickState;

use crate::config::TownsPadType;

/// Reset-to-first-nibble timeout: an idle gap returns the read state to X-high.
const MOUSE_RESET_TIMEOUT_NANOS: u64 = 1_000_000;
/// Number of nanoseconds in one second.
const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// Mouse nibble read phases, advanced by COM toggles.
const MOUSE_STATE_X_HIGH: u8 = 0;
const MOUSE_STATE_X_LOW: u8 = 1;
const MOUSE_STATE_Y_HIGH: u8 = 2;
const MOUSE_STATE_Y_LOW: u8 = 3;

/// Read-data bits.
const BIT_COM: u8 = 0x40;
const BIT_BUTTON_LEFT: u8 = 0x10;
const BIT_BUTTON_RIGHT: u8 = 0x20;
const NIBBLE_MASK: u8 = 0x0F;
/// Base mask applied before the trigger lines gate the button bits.
const READ_BUTTON_GATE_BASE: u8 = 0xCF;

/// Output-port bit assignments (0x04D6 write).
const OUTPUT_PORT0_COM: u8 = 0x10;
const OUTPUT_PORT1_COM: u8 = 0x20;
const OUTPUT_TRIG_MASK: u8 = 0x03;
const OUTPUT_PORT0_TRIG_SHIFT: u8 = 0;
const OUTPUT_PORT1_TRIG_SHIFT: u8 = 2;

/// Pad read-data bits (active low: a released input reads 1).
const PAD_UP: u8 = 0x01;
const PAD_DOWN: u8 = 0x02;
const PAD_LEFT: u8 = 0x04;
const PAD_RIGHT: u8 = 0x08;
const PAD_BUTTON_A: u8 = 0x10;
const PAD_BUTTON_B: u8 = 0x20;
const PAD_COM_ECHO: u8 = 0x40;
const PAD_SIXBUTTON: u8 = 0x80;
/// All four directions plus the two face buttons.
const PAD_DIRECTION_BUTTON_MASK: u8 = 0x3F;
/// Base mask before the trigger lines gate the two face-button bits.
const PAD_BUTTON_GATE_BASE: u8 = 0xCF;
/// Run/Start forces the left+right lines; Select forces up+down.
const PAD_RUN_CHORD: u8 = PAD_LEFT | PAD_RIGHT;
const PAD_SELECT_CHORD: u8 = PAD_UP | PAD_DOWN;

/// Extra 6-button pad bits, read while COM is high.
const PAD_BUTTON_Z: u8 = 0x01;
const PAD_BUTTON_Y: u8 = 0x02;
const PAD_BUTTON_X: u8 = 0x04;
const PAD_BUTTON_C: u8 = 0x08;

/// A digital pad on game port 0.
struct PadPort {
    kind: TownsPadType,
    state: JoystickState,
    com: bool,
    trigger: u8,
}

impl PadPort {
    fn new() -> Self {
        Self {
            kind: TownsPadType::default(),
            state: JoystickState::default(),
            com: false,
            trigger: 0,
        }
    }

    /// Latches the COM strobe and trigger lines driven by the output port.
    fn write(&mut self, com: bool, trigger: u8) {
        self.com = com;
        self.trigger = trigger;
    }

    /// Reads the pad byte for the current COM/trigger lines.
    fn read(&self) -> u8 {
        match self.kind {
            TownsPadType::TwoButton => self.read_two_button(),
            TownsPadType::SixButton => self.read_six_button(),
        }
    }

    /// 2-button pad: directions and A/B active-low, the face buttons gated by
    /// the trigger lines. Bit 0 is up and bit 1 is down (swapped vs the
    /// databook, per the Errata).
    fn read_two_button(&self) -> u8 {
        let mut data = PAD_DIRECTION_BUTTON_MASK;
        if self.com {
            data |= PAD_COM_ECHO;
        }
        if self.state.up {
            data &= !PAD_UP;
        }
        if self.state.down {
            data &= !PAD_DOWN;
        }
        if self.state.left {
            data &= !PAD_LEFT;
        }
        if self.state.right {
            data &= !PAD_RIGHT;
        }
        if self.state.trigger1 {
            data &= !PAD_BUTTON_A;
        }
        if self.state.trigger2 {
            data &= !PAD_BUTTON_B;
        }
        data & (PAD_BUTTON_GATE_BASE | (self.trigger << 4))
    }

    /// 6-button pad: COM low reports the 2-button layout (with Run/Select
    /// chords); COM high reports the extra buttons Z/Y/X/C. Bit 7 is always set
    /// so software can distinguish it from a 2-button pad.
    fn read_six_button(&self) -> u8 {
        let mut pressed = 0u8;
        if self.com {
            if self.state.button_z {
                pressed |= PAD_BUTTON_Z;
            }
            if self.state.button_y {
                pressed |= PAD_BUTTON_Y;
            }
            if self.state.button_x {
                pressed |= PAD_BUTTON_X;
            }
            if self.state.button_c {
                pressed |= PAD_BUTTON_C;
            }
        } else {
            if self.state.up {
                pressed |= PAD_UP;
            }
            if self.state.down {
                pressed |= PAD_DOWN;
            }
            if self.state.left {
                pressed |= PAD_LEFT;
            }
            if self.state.right {
                pressed |= PAD_RIGHT;
            }
            if self.state.run {
                pressed |= PAD_RUN_CHORD;
            }
            if self.state.select {
                pressed |= PAD_SELECT_CHORD;
            }
        }
        if self.state.trigger1 {
            pressed |= PAD_BUTTON_A;
        }
        if self.state.trigger2 {
            pressed |= PAD_BUTTON_B;
        }
        let mut data = (!pressed) & PAD_DIRECTION_BUTTON_MASK;
        if self.com {
            data |= PAD_COM_ECHO;
        }
        data | PAD_SIXBUTTON
    }
}

/// MSX-style relative mouse on game port 1.
struct MousePort {
    com: bool,
    trigger: u8,
    state: u8,
    accumulator_x: i32,
    accumulator_y: i32,
    latched_x: u8,
    latched_y: u8,
    button_left: bool,
    button_right: bool,
    last_access_cycle: u64,
}

impl MousePort {
    fn new() -> Self {
        Self {
            com: false,
            trigger: 0,
            state: MOUSE_STATE_X_HIGH,
            accumulator_x: 0,
            accumulator_y: 0,
            latched_x: 0,
            latched_y: 0,
            button_left: false,
            button_right: false,
            last_access_cycle: 0,
        }
    }

    /// Advances the nibble read state on a COM edge.
    fn write(&mut self, now: u64, com: bool, trigger: u8) {
        if self.com != com {
            match self.state {
                MOUSE_STATE_X_HIGH | MOUSE_STATE_Y_HIGH if !com => self.state += 1,
                MOUSE_STATE_X_LOW | MOUSE_STATE_Y_LOW if com => self.state += 1,
                _ => {}
            }
            if self.state > MOUSE_STATE_Y_LOW {
                self.state = MOUSE_STATE_X_HIGH;
            }
        }
        self.com = com;
        self.trigger = trigger;
        self.last_access_cycle = now;
    }

    /// Reads the current nibble plus the COM and button bits.
    fn read(&mut self, now: u64, reset_timeout_cycles: u64) -> u8 {
        let mut data = 0;
        if self.com {
            data |= BIT_COM;
        }
        if now.saturating_sub(self.last_access_cycle) > reset_timeout_cycles {
            self.state = MOUSE_STATE_X_HIGH;
        }
        if !self.button_left {
            data |= BIT_BUTTON_LEFT;
        }
        if !self.button_right {
            data |= BIT_BUTTON_RIGHT;
        }
        match self.state {
            MOUSE_STATE_X_HIGH => {
                self.latched_x = clamp_to_signed_byte(self.accumulator_x);
                self.latched_y = clamp_to_signed_byte(self.accumulator_y);
                data |= (self.latched_x >> 4) & NIBBLE_MASK;
            }
            MOUSE_STATE_X_LOW => data |= self.latched_x & NIBBLE_MASK,
            MOUSE_STATE_Y_HIGH => data |= (self.latched_y >> 4) & NIBBLE_MASK,
            MOUSE_STATE_Y_LOW => {
                data |= self.latched_y & NIBBLE_MASK;
                self.accumulator_x = 0;
                self.accumulator_y = 0;
            }
            _ => data |= NIBBLE_MASK,
        }
        data &= READ_BUTTON_GATE_BASE | (self.trigger << 4);
        self.last_access_cycle = now;
        data
    }
}

/// FM Towns game port.
pub(crate) struct TownsGamePort {
    pad: PadPort,
    mouse: MousePort,
    output_latch: u8,
    reset_timeout_cycles: u64,
}

impl TownsGamePort {
    /// Creates the game port for the given CPU clock (used for the mouse timeout).
    pub(crate) fn new(cpu_clock_hz: u32) -> Self {
        let reset_timeout_cycles =
            (u64::from(cpu_clock_hz) * MOUSE_RESET_TIMEOUT_NANOS / NANOS_PER_SECOND).max(1);
        Self {
            pad: PadPort::new(),
            mouse: MousePort::new(),
            output_latch: 0,
            reset_timeout_cycles,
        }
    }

    /// Reads game port 0 (0x04D0): the pad.
    pub(crate) fn read_port_a(&self) -> u8 {
        self.pad.read()
    }

    /// Updates the pad direction and button state from the host.
    pub(crate) fn set_pad(&mut self, state: JoystickState) {
        self.pad.state = state;
    }

    /// Selects the pad type on game port 0.
    pub(crate) fn set_pad_type(&mut self, kind: TownsPadType) {
        self.pad.kind = kind;
    }

    /// Reads game port 1 (0x04D2): the mouse.
    pub(crate) fn read_port_b(&mut self, now: u64) -> u8 {
        self.mouse.read(now, self.reset_timeout_cycles)
    }

    /// Reads the output-port latch (0x04D6).
    pub(crate) fn read_output(&self) -> u8 {
        self.output_latch
    }

    /// Writes the output port (0x04D6): COM and trigger lines for both ports.
    pub(crate) fn write_output(&mut self, now: u64, value: u8) {
        self.output_latch = value;
        let pad_com = value & OUTPUT_PORT0_COM != 0;
        let pad_trigger = (value >> OUTPUT_PORT0_TRIG_SHIFT) & OUTPUT_TRIG_MASK;
        self.pad.write(pad_com, pad_trigger);
        let mouse_com = value & OUTPUT_PORT1_COM != 0;
        let mouse_trigger = (value >> OUTPUT_PORT1_TRIG_SHIFT) & OUTPUT_TRIG_MASK;
        self.mouse.write(now, mouse_com, mouse_trigger);
    }

    /// Accumulates a relative mouse movement from the host.
    pub(crate) fn push_mouse_delta(&mut self, dx: i16, dy: i16) {
        self.mouse.accumulator_x -= i32::from(dx);
        self.mouse.accumulator_y -= i32::from(dy);
    }

    /// Updates the mouse button state.
    pub(crate) fn set_mouse_buttons(&mut self, left: bool, right: bool) {
        self.mouse.button_left = left;
        self.mouse.button_right = right;
    }
}

/// Clamps an accumulated delta to a signed byte and returns its two's-complement
/// bit pattern for nibble extraction.
fn clamp_to_signed_byte(value: i32) -> u8 {
    value.clamp(-128, 127) as i8 as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPU_HZ: u32 = 66_000_000;

    #[test]
    fn mouse_clocks_out_four_nibbles() {
        let mut port = TownsGamePort::new(CPU_HZ);
        port.push_mouse_delta(-0x12, 0x01); // X = +0x12 (both axes inverted; Y unused here)
        port.set_mouse_buttons(false, false);

        // Full trigger gating so the button bits are visible.
        let trig = 0x0C; // both trigger lines high.
        // X-high nibble.
        port.write_output(1, trig);
        let x_high = port.read_port_b(1) & NIBBLE_MASK;
        // Toggle COM low -> X-low.
        port.write_output(2, trig & !OUTPUT_PORT1_COM);
        // Actually drive the COM strobe explicitly.
        port.write_output(2, OUTPUT_PORT1_COM | trig);
        port.write_output(3, trig);
        let x_low = port.read_port_b(3) & NIBBLE_MASK;
        assert_eq!(x_high, 0x1);
        assert_eq!(x_low, 0x2);
    }

    #[test]
    fn mouse_buttons_are_active_low() {
        let mut port = TownsGamePort::new(CPU_HZ);
        // Trigger lines high so button bits pass through.
        port.write_output(1, OUTPUT_PORT1_COM | 0x0C);
        // Nothing pressed: both button bits set (released).
        let released = port.read_port_b(1);
        assert_eq!(released & BIT_BUTTON_LEFT, BIT_BUTTON_LEFT);
        assert_eq!(released & BIT_BUTTON_RIGHT, BIT_BUTTON_RIGHT);

        port.set_mouse_buttons(true, true);
        let pressed = port.read_port_b(1);
        assert_eq!(pressed & BIT_BUTTON_LEFT, 0);
        assert_eq!(pressed & BIT_BUTTON_RIGHT, 0);
    }

    #[test]
    fn idle_timeout_resets_to_first_nibble() {
        let mut port = TownsGamePort::new(CPU_HZ);
        port.push_mouse_delta(-0x30, -0x40);
        // Advance to Y-low.
        port.write_output(10, OUTPUT_PORT1_COM);
        port.write_output(20, 0);
        port.write_output(30, OUTPUT_PORT1_COM);
        port.write_output(40, 0);
        // A long idle gap returns the read state to X-high.
        let far = 40 + u64::from(CPU_HZ); // one second later.
        let value = port.read_port_b(far) & NIBBLE_MASK;
        assert_eq!(value, 0x3); // X-high nibble of 0x30.
    }

    #[test]
    fn two_button_pad_direction_bits_are_swapped() {
        let mut port = TownsGamePort::new(CPU_HZ);
        port.set_pad_type(TownsPadType::TwoButton);
        let state = JoystickState {
            up: true,
            ..JoystickState::default()
        };
        port.set_pad(state);
        // Drive port 0 COM and both trigger lines high.
        port.write_output(0, OUTPUT_PORT0_COM | OUTPUT_TRIG_MASK);
        let data = port.read_port_a();
        // Bit 0 is up (pressed -> 0), bit 1 is down (released -> 1).
        assert_eq!(data & PAD_UP, 0);
        assert_eq!(data & PAD_DOWN, PAD_DOWN);
    }

    #[test]
    fn two_button_face_buttons_gated_by_trigger() {
        let mut port = TownsGamePort::new(CPU_HZ);
        port.set_pad_type(TownsPadType::TwoButton);
        let state = JoystickState {
            trigger1: true,
            ..JoystickState::default()
        };
        port.set_pad(state);

        // Trigger lines high: button A is visible (pressed -> 0).
        port.write_output(0, OUTPUT_PORT0_COM | OUTPUT_TRIG_MASK);
        assert_eq!(port.read_port_a() & PAD_BUTTON_A, 0);

        // Trigger lines low: the face-button bits are masked out (read as 0).
        port.write_output(0, OUTPUT_PORT0_COM);
        assert_eq!(port.read_port_a() & PAD_BUTTON_A, 0);
        // A released with triggers low still reads 0 (masked), confirming gating.
        let released = JoystickState::default();
        port.set_pad(released);
        assert_eq!(port.read_port_a() & PAD_BUTTON_A, 0);
        // With triggers high and released, the bit reads 1.
        port.write_output(0, OUTPUT_PORT0_COM | OUTPUT_TRIG_MASK);
        assert_eq!(port.read_port_a() & PAD_BUTTON_A, PAD_BUTTON_A);
    }

    #[test]
    fn six_button_pad_multiplexes_on_com() {
        let mut port = TownsGamePort::new(CPU_HZ);
        port.set_pad_type(TownsPadType::SixButton);
        let state = JoystickState {
            button_z: true,
            up: true,
            ..JoystickState::default()
        };
        port.set_pad(state);

        // COM high: the extra buttons are reported; Z is pressed (bit 0 -> 0).
        port.write_output(0, OUTPUT_PORT0_COM);
        let high = port.read_port_a();
        assert_eq!(high & PAD_SIXBUTTON, PAD_SIXBUTTON);
        assert_eq!(high & PAD_BUTTON_Z, 0);

        // COM low: the directions are reported; up is pressed (bit 0 -> 0), and
        // the Z press does not leak into the direction bits.
        port.write_output(0, 0);
        let low = port.read_port_a();
        assert_eq!(low & PAD_SIXBUTTON, PAD_SIXBUTTON);
        assert_eq!(low & PAD_UP, 0);
    }
}
