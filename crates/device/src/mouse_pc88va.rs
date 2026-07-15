//! PC-88VA2 bus mouse.
//!
//! The mouse is read through the YM2608 SSG I/O ports (register 0x0E delivers a
//! data nibble, register 0x0F the buttons), advanced by a strobe on system port
//! 0x040 bit 6. A four-state machine (XH, XL, YH, YL) walks the latched delta
//! one nibble per strobe edge; wrapping past YL latches a fresh delta.

const STATE_XH: u8 = 0;
const STATE_XL: u8 = 1;
const STATE_YH: u8 = 2;
const STATE_YL: u8 = 3;

/// Mouse interface state.
pub struct MouseVa {
    /// Accumulated horizontal movement since the last latch.
    accum_x: i32,
    /// Accumulated vertical movement since the last latch.
    accum_y: i32,
    /// Latched horizontal delta, clamped to [-128, 127].
    latch_x: i32,
    /// Latched vertical delta, clamped to [-128, 127].
    latch_y: i32,
    /// Nibble phase (XH/XL/YH/YL).
    state: u8,
    /// Last observed strobe level (bit 6 of port 0x040).
    last_strobe: u8,
    /// Button state, np2 convention: bit5 = left, bit7 = right (set = pressed).
    buttons: u8,
}

impl Default for MouseVa {
    fn default() -> Self {
        Self {
            accum_x: 0,
            accum_y: 0,
            latch_x: 0,
            latch_y: 0,
            // Reset leaves the machine at the last state, so the first strobe
            // wraps to XH and latches (matching `mouseifva_reset`).
            state: STATE_YL,
            last_strobe: 0,
            buttons: 0,
        }
    }
}

impl MouseVa {
    /// Accumulates a relative movement reported by the host.
    pub fn push_delta(&mut self, delta_x: i16, delta_y: i16) {
        self.accum_x += i32::from(delta_x);
        self.accum_y += i32::from(delta_y);
    }

    /// Sets the button state (left/right; the VA mouse has no middle button).
    pub fn set_buttons(&mut self, left: bool, right: bool) {
        self.buttons = (if left { 0x20 } else { 0 }) | (if right { 0x80 } else { 0 });
    }

    /// Advances the nibble machine on a strobe-level change, latching a fresh
    /// delta when the phase wraps from YL back to XH.
    pub fn strobe(&mut self, strobe: u8) {
        if strobe == self.last_strobe {
            return;
        }
        self.last_strobe = strobe;
        self.state += 1;
        if self.state > STATE_YL {
            self.state = STATE_XH;
            self.latch();
        }
    }

    fn latch(&mut self) {
        self.latch_x = self.accum_x.clamp(-128, 127);
        self.latch_y = self.accum_y.clamp(-128, 127);
        self.accum_x = 0;
        self.accum_y = 0;
    }

    /// The data nibble (register 0x0E) for the current phase. The delta is
    /// negated to match the hardware sign convention.
    pub fn data_nibble(&self) -> u8 {
        let x = (-self.latch_x) as u8;
        let y = (-self.latch_y) as u8;
        match self.state {
            STATE_XL => x & 0x0F,
            STATE_XH => (x >> 4) & 0x0F,
            STATE_YL => y & 0x0F,
            STATE_YH => (y >> 4) & 0x0F,
            _ => 0,
        }
    }

    /// The button bits (register 0x0F): bit0 = left, bit1 = right. The MSX-style
    /// trigger lines are active low, so a pressed button pulls its bit low and an
    /// idle mouse reads both bits high.
    pub fn button_bits(&self) -> u8 {
        let mut value = 0x03;
        if self.buttons & 0x20 != 0 {
            value &= !0x01;
        }
        if self.buttons & 0x80 != 0 {
            value &= !0x02;
        }
        value
    }
}
