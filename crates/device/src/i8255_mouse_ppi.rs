//! i8255 Mouse PPI - mouse interface controller.
//!
//! The mouse PPI uses an 8255A-compatible chip at ports 0x7FD9-0x7FDF.
//! Port 0x7FD9 R/W: Port A (mouse buttons and counter data).
//! Port 0x7FDB R/W: Port B (DIP switches).
//! Port 0x7FDD R/W: Port C (mouse control and DIP switches).
//! Port 0x7FDF W:   PPI control/mode register.
//!
//! Boot initialization typically sets the PPI mode register to 0x93,
//! which configures port A input, port C upper output / lower input, port B input.
//!
//! Reference: `undoc98/io_mouse.txt`

/// PPI control register bit 7: mode set flag.
/// When set, the write is a mode configuration word.
/// When clear, the write is a bit set/reset command for port C.
const CTRL_MODE_SET: u8 = 1 << 7;

/// Bit set/reset command bit 0: 1 = set the targeted port C bit, 0 = reset it.
const CTRL_BSR_SET: u8 = 1;

/// PPI mode register bit 4: port A direction (1 = input, 0 = output).
const MODE_PORT_A_INPUT: u8 = 1 << 4;

/// PPI mode register bit 3: port C upper (bits 4-7) direction (1 = input, 0 = output).
const MODE_PORT_C_UPPER_INPUT: u8 = 1 << 3;

/// PPI mode register bit 1: port B direction (1 = input, 0 = output).
const MODE_PORT_B_INPUT: u8 = 1 << 1;

/// PPI mode register bit 0: port C lower (bits 0-3) direction (1 = input, 0 = output).
const MODE_PORT_C_LOWER_INPUT: u8 = 1;

/// Port C bit 7: HC -- latch mouse counter and reset. Rising edge (0->1) latches counters.
const PORTC_HC: u8 = 1 << 7;

/// Port C bit 6: SXY -- selects X/Y axis (0 = X, 1 = Y).
const PORTC_SXY: u8 = 1 << 6;

/// Port C bit 5: SHL -- selects high/low nibble (0 = low, 1 = high).
const PORTC_SHL: u8 = 1 << 5;

/// Port C bit 4: INT# -- mouse timer interrupt mask (0 = enabled, 1 = disabled).
const PORTC_INT_MASK: u8 = 1 << 4;

/// Port C bit 3: MODSW -- normal/high-resolution mode status.
const PORTC_MODSW: u8 = 1 << 3;

/// Default mode register value (0x93): port A input (mode 0), port C upper output,
/// port B input (mode 0), port C lower input.
const MODE_DEFAULT: u8 = 0x93;

/// Default port C value: HC, SXY, SHL, INT_MASK all high (0xF0).
const PORT_C_DEFAULT: u8 = PORTC_HC | PORTC_SXY | PORTC_SHL | PORTC_INT_MASK;

/// Port A value when no mouse is connected: 0x70 (middle button ON for shizuku
/// compatibility per NP21W).
const PORT_A_NO_MOUSE: u8 = 0x70;

/// Port B value when in input mode: 0x40 (RAMKL=1, standard DIP defaults per NP21W).
const PORT_B_INPUT_DEFAULT: u8 = 0x40;

/// Default button state: all buttons released (active-low = bits set).
/// Bit 7 = LEFT off, bit 6 = MIDDLE off, bit 5 = RIGHT off.
const BUTTONS_DEFAULT: u8 = 0xE0;

/// Minimum elapsed cycles before interpolation kicks in (NP21W threshold).
const INTERPOLATION_THRESHOLD: u64 = 2000;

save_state::runtime_state! {
/// Authoritative mouse PPI state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I8255MousePpiState {
    /// PPI mode register (port 0x7FDF bit 7 = 1 writes).
    pub mode: u8,
    /// Port A latch (port 0x7FD9).
    pub port_a: u8,
    /// Port B latch (port 0x7FDB).
    pub port_b: u8,
    /// Port C latch (port 0x7FDD).
    pub port_c: u8,
    /// Live X delta accumulator (counts movement since last latch).
    pub accumulator_x: i16,
    /// Live Y delta accumulator.
    pub accumulator_y: i16,
    /// Remaining X delta not yet consumed by interpolation this frame.
    pub remaining_x: i16,
    /// Remaining Y delta not yet consumed by interpolation this frame.
    pub remaining_y: i16,
    /// Total X movement for the current frame (used as interpolation reference).
    pub sample_x: i16,
    /// Total Y movement for the current frame.
    pub sample_y: i16,
    /// Latched X value (captured on HC rising edge, clamped to -128..127).
    pub latch_x: i16,
    /// Latched Y value.
    pub latch_y: i16,
    /// Button state byte (active-low): bit 7 = LEFT, bit 6 = MIDDLE, bit 5 = RIGHT.
    pub buttons: u8,
    /// CPU cycle at which the last movement interpolation was computed.
    pub last_interpolation_cycle: u64,
    /// Whether a mouse is connected.
    pub mouse_connected: bool,
}}

/// i8255 Mouse PPI controller.
pub struct I8255MousePpi {
    /// Embedded state for save/restore.
    pub state: I8255MousePpiState,
    /// CPU cycles per interpolation unit (cpu_clock_hz / 56400, per NP21W).
    /// Not saved/restored -- derived from clock config.
    move_clock: u32,
}

impl_simple_state_accessors!(I8255MousePpi, I8255MousePpiState);

impl Default for I8255MousePpi {
    fn default() -> Self {
        Self::new()
    }
}

impl I8255MousePpi {
    /// Creates a new mouse PPI in hardware reset state.
    pub fn new() -> Self {
        Self {
            state: I8255MousePpiState {
                mode: MODE_DEFAULT,
                port_a: 0x00,
                port_b: 0x00,
                port_c: PORT_C_DEFAULT,
                accumulator_x: 0,
                accumulator_y: 0,
                remaining_x: 0,
                remaining_y: 0,
                sample_x: 0,
                sample_y: 0,
                latch_x: 0,
                latch_y: 0,
                buttons: BUTTONS_DEFAULT,
                last_interpolation_cycle: 0,
                mouse_connected: true,
            },
            move_clock: 142, // 8 MHz / 56400 ~ 142
        }
    }

    /// Sets the interpolation clock divisor from the CPU clock frequency.
    pub fn set_cpu_clock(&mut self, cpu_clock_hz: u32) {
        self.move_clock = cpu_clock_hz / 56400;
    }

    /// Synchronizes host mouse input at frame boundaries.
    ///
    /// `dx`/`dy` are the raw relative motion deltas accumulated since the previous
    /// frame. Adds any unconsumed remainder from the previous frame to the
    /// accumulators, then stores the new deltas for gradual interpolation.
    pub fn sync_frame(&mut self, dx: i16, dy: i16, current_cycle: u64) {
        self.state.accumulator_x = self
            .state
            .accumulator_x
            .saturating_add(self.state.remaining_x);
        self.state.accumulator_y = self
            .state
            .accumulator_y
            .saturating_add(self.state.remaining_y);

        self.state.sample_x = dx;
        self.state.sample_y = dy;
        self.state.remaining_x = dx;
        self.state.remaining_y = dy;

        self.state.last_interpolation_cycle = current_cycle;
    }

    /// Gradually transfers movement from `remaining` into `accumulator` proportional
    /// to elapsed CPU cycles.
    fn interpolate_movement(&mut self, current_cycle: u64) {
        let elapsed = current_cycle.wrapping_sub(self.state.last_interpolation_cycle);
        if elapsed < INTERPOLATION_THRESHOLD {
            return;
        }

        let ticks = (elapsed / 1000) as i32;
        let move_clock = self.move_clock.max(1) as i32;

        // Interpolate X.
        let sx = self.state.sample_x as i32;
        if sx != 0 {
            let mut dx = sx.abs() * ticks / move_clock;
            if sx < 0 {
                dx = -dx;
            }
            let rx = self.state.remaining_x as i32;
            if sx > 0 {
                dx = dx.min(rx);
            } else {
                dx = dx.max(rx);
            }
            self.state.accumulator_x = self.state.accumulator_x.saturating_add(dx as i16);
            self.state.remaining_x -= dx as i16;
        }

        // Interpolate Y.
        let sy = self.state.sample_y as i32;
        if sy != 0 {
            let mut dy = sy.abs() * ticks / move_clock;
            if sy < 0 {
                dy = -dy;
            }
            let ry = self.state.remaining_y as i32;
            if sy > 0 {
                dy = dy.min(ry);
            } else {
                dy = dy.max(ry);
            }
            self.state.accumulator_y = self.state.accumulator_y.saturating_add(dy as i16);
            self.state.remaining_y -= dy as i16;
        }

        self.state.last_interpolation_cycle += (ticks as u64) * 1000;
    }

    /// Latches the current accumulator values and resets accumulators.
    ///
    /// Called on the rising edge (0 -> 1) of the HC bit (Port C bit 7).
    pub fn latch(&mut self, current_cycle: u64) {
        self.interpolate_movement(current_cycle);

        self.state.latch_x = self.state.accumulator_x.clamp(-128, 127);
        self.state.latch_y = self.state.accumulator_y.clamp(-128, 127);

        self.state.accumulator_x = 0;
        self.state.accumulator_y = 0;
    }

    /// Updates the button state from host input.
    ///
    /// Each parameter: `true` = pressed, `false` = released.
    /// Internally encoded as active-low: pressed = bit clear, released = bit set.
    pub fn set_buttons(&mut self, left: bool, right: bool, middle: bool) {
        let mut buttons: u8 = 0;
        if !left {
            buttons |= 0x80;
        }
        if !middle {
            buttons |= 0x40;
        }
        if !right {
            buttons |= 0x20;
        }
        self.state.buttons = buttons;
    }

    /// Reads port A (port 0x7FD9).
    ///
    /// In input mode: returns button state in bits 7:4 plus the selected counter
    /// nibble in bits 3:0, selected by Port C (HC, SXY, SHL).
    /// In output mode: returns the port A latch.
    pub fn read_port_a(&mut self, current_cycle: u64) -> u8 {
        if self.state.mode & MODE_PORT_A_INPUT == 0 {
            return self.state.port_a;
        }

        if !self.state.mouse_connected {
            return PORT_A_NO_MOUSE;
        }

        self.interpolate_movement(current_cycle);

        // Button byte: active-low buttons with 0x40 forced for shizuku compatibility.
        let mut ret = self.state.buttons & 0xF0;
        ret |= 0x40;

        // Select axis value based on HC (latched vs live) and SXY (X vs Y).
        let portc = self.state.port_c;
        let value = if portc & PORTC_HC != 0 {
            if portc & PORTC_SXY != 0 {
                self.state.latch_y
            } else {
                self.state.latch_x
            }
        } else if portc & PORTC_SXY != 0 {
            self.state.accumulator_y
        } else {
            self.state.accumulator_x
        };

        // Select nibble based on SHL.
        if portc & PORTC_SHL != 0 {
            ret |= ((value >> 4) & 0x0F) as u8;
        } else {
            ret |= (value & 0x0F) as u8;
        }

        ret
    }

    /// Reads port B (port 0x7FDB).
    ///
    /// In input mode: returns 0x40 (RAMKL=1, standard DIP defaults per NP21W).
    /// In output mode: returns the port B latch.
    pub fn read_port_b(&self) -> u8 {
        if self.state.mode & MODE_PORT_B_INPUT != 0 {
            PORT_B_INPUT_DEFAULT
        } else {
            self.state.port_b
        }
    }

    /// Reads port C (port 0x7FDD).
    ///
    /// Combines output bits (echoed from latch) with input bits (DIP switches).
    pub fn read_port_c(&self) -> u8 {
        let mut ret = self.state.port_c;
        if self.state.mode & MODE_PORT_C_UPPER_INPUT != 0 {
            ret &= !(PORTC_HC | PORTC_SXY | PORTC_SHL);
        }
        if self.state.mode & MODE_PORT_C_LOWER_INPUT != 0 {
            ret &= 0xF0;
            ret |= PORTC_MODSW;
        }
        ret
    }

    /// Writes port A (port 0x7FD9).
    pub fn write_port_a(&mut self, value: u8) {
        self.state.port_a = value;
    }

    /// Writes port B (port 0x7FDB).
    pub fn write_port_b(&mut self, value: u8) {
        self.state.port_b = value;
    }

    /// Writes port C (port 0x7FDD) and returns whether HC had a rising edge (0->1).
    ///
    /// The caller should invoke [`latch`] when this returns `true`.
    pub fn write_port_c(&mut self, value: u8) -> bool {
        let old = self.state.port_c;
        self.state.port_c = value;
        (old & PORTC_HC == 0) && (value & PORTC_HC != 0)
    }

    /// Writes the PPI control register (port 0x7FDF) and returns whether HC had
    /// a rising edge (0->1).
    ///
    /// If bit 7 is set, the value is a mode configuration word: stored in `mode`
    /// and port C is reset to 0. This never produces a rising edge.
    /// If bit 7 is clear, the value is a bit set/reset command for port C.
    /// The caller should invoke [`latch`] when this returns `true`.
    pub fn write_ctrl(&mut self, value: u8) -> bool {
        if value & CTRL_MODE_SET != 0 {
            self.state.mode = value;
            self.state.port_c = 0;
            false
        } else {
            let old = self.state.port_c;
            let bit = (value >> 1) & 7;
            if value & CTRL_BSR_SET != 0 {
                self.state.port_c |= 1 << bit;
            } else {
                self.state.port_c &= !(1 << bit);
            }
            (old & PORTC_HC == 0) && (self.state.port_c & PORTC_HC != 0)
        }
    }
}
