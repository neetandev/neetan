//! FM-7 joystick-port mouse.
//!
//! The mouse plugs into a joystick port and is read through the OPN SSG
//! parallel ports. Port B carries the strobe and button gate lines, while port
//! A delivers one nibble of the latched movement per strobe edge.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MousePhase {
    XHigh,
    XLow,
    YHigh,
    YLow,
}

const BUTTON_GATE_MASK: u8 = 0x03;
const BUTTON_LEFT: u8 = 0x10;
const BUTTON_RIGHT: u8 = 0x20;
const READOUT_HIGH_BITS: u8 = 0xC0;
const DELTA_LIMIT: i32 = 127;

/// FM-7 joystick-port mouse.
pub struct MouseFm7 {
    accum_x: i32,
    accum_y: i32,
    latch_x: u8,
    latch_y: u8,
    phase: MousePhase,
    strobe: bool,
    x_high_latched: bool,
    button_left: bool,
    button_right: bool,
}

impl MouseFm7 {
    /// Creates an idle mouse.
    pub fn new() -> Self {
        Self {
            accum_x: 0,
            accum_y: 0,
            latch_x: 0,
            latch_y: 0,
            phase: MousePhase::XHigh,
            strobe: false,
            x_high_latched: false,
            button_left: false,
            button_right: false,
        }
    }

    /// Accumulates relative movement reported by the host.
    pub fn push_delta(&mut self, delta_x: i16, delta_y: i16) {
        self.accum_x = (self.accum_x + i32::from(delta_x)).clamp(-DELTA_LIMIT, DELTA_LIMIT);
        self.accum_y = (self.accum_y + i32::from(delta_y)).clamp(-DELTA_LIMIT, DELTA_LIMIT);
    }

    /// Sets the button state.
    pub fn set_buttons(&mut self, left: bool, right: bool) {
        self.button_left = left;
        self.button_right = right;
    }

    /// Applies a strobe level and reports whether the level changed.
    pub fn update_strobe(&mut self, strobe: bool) -> bool {
        if strobe == self.strobe {
            return false;
        }
        self.strobe = strobe;
        self.phase = match (self.phase, strobe) {
            (MousePhase::XHigh, false) => MousePhase::XLow,
            (MousePhase::XLow, true) => MousePhase::YHigh,
            (MousePhase::YHigh, false) => MousePhase::YLow,
            (MousePhase::YLow, true) => {
                self.x_high_latched = false;
                MousePhase::XHigh
            }
            (MousePhase::XHigh, true) => {
                self.x_high_latched = false;
                MousePhase::XHigh
            }
            (phase, _) => phase,
        };
        true
    }

    fn latch(&mut self) {
        self.latch_x = (-self.accum_x) as u8;
        self.latch_y = (-self.accum_y) as u8;
        self.accum_x = 0;
        self.accum_y = 0;
    }

    /// Resynchronizes the sequence after the strobe stalls mid-read.
    pub fn timeout(&mut self) {
        self.phase = MousePhase::XHigh;
        self.x_high_latched = false;
    }

    /// Returns the port A readout for the current nibble phase.
    pub fn read(&mut self, port_b: u8) -> u8 {
        if self.phase == MousePhase::XHigh && self.strobe && !self.x_high_latched {
            self.latch();
            self.x_high_latched = true;
        }
        let nibble = match self.phase {
            MousePhase::XHigh => self.latch_x >> 4,
            MousePhase::XLow => self.latch_x & 0x0F,
            MousePhase::YHigh => self.latch_y >> 4,
            MousePhase::YLow => self.latch_y & 0x0F,
        };
        let mut buttons = BUTTON_LEFT | BUTTON_RIGHT;
        if self.button_left {
            buttons &= !BUTTON_LEFT;
        }
        if self.button_right {
            buttons &= !BUTTON_RIGHT;
        }
        let gate = (port_b & BUTTON_GATE_MASK) << 4;
        nibble | (buttons & gate) | READOUT_HIGH_BITS
    }
}

impl Default for MouseFm7 {
    fn default() -> Self {
        Self::new()
    }
}
