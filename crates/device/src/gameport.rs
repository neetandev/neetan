//! Standard analog PC game port (0x200-0x207).
//!
//! Behavioral model of the classic IBM/Creative game port with its four 558
//! one-shot axis timers. A write to the port fires all present axis one-shots;
//! each axis bit then reads high for a time proportional to that axis'
//! potentiometer position, after which it drops low. Buttons are active low in
//! bits 4-7. Button lines read released while any axis is still discharging.
//!
//! Two two-axis, two-button sticks are supported: stick 0 drives axes 0-1 and
//! button bits 0x10/0x20, stick 1 drives axes 2-3 and button bits 0x40/0x80.
//! Axes of an absent stick read low and its buttons read released.

/// Base discharge time of an axis one-shot in microseconds (potentiometer 0).
const AXIS_BASE_MICROS: i64 = 24;

/// Builds the analog discharge time of one axis in core cycles.
///
/// Shift the signed axis into 0..65535, scale to ohms, then to the 558 timing,
/// and add the fixed base. Full scale is about 1.1 ms.
fn axis_discharge_cycles(cpu_clock_hz: u32, value: i32) -> u64 {
    let ohms = (i64::from(value) + 32768) * 100 / 65;
    let micros = ohms * 11 / 1000 + AXIS_BASE_MICROS;
    (i64::from(cpu_clock_hz) * micros / 1_000_000).max(0) as u64
}

/// Standard analog game port.
pub struct GamePort {
    /// Core clock in hertz, for converting discharge times to cycles.
    cpu_clock_hz: u32,
    /// Cycle of the most recent port write (one-shot fire time).
    fire_cycle: u64,
    /// Per-axis discharge duration in cycles, `None` when the stick is absent.
    axis_discharge: [Option<u64>; 4],
    /// Latest analog value per axis (-32768..32767).
    axis_value: [i32; 4],
    /// Button state per stick pair: [s0b1, s0b2, s1b1, s1b2], true = pressed.
    buttons: [bool; 4],
    /// Whether each of the two sticks is connected.
    present: [bool; 2],
}

impl GamePort {
    /// Builds a game port with no stick connected.
    pub fn new(cpu_clock_hz: u32) -> Self {
        Self {
            cpu_clock_hz,
            fire_cycle: 0,
            axis_discharge: [None; 4],
            axis_value: [0; 4],
            buttons: [false; 4],
            present: [false; 2],
        }
    }

    /// Returns the port to its power-on state, keeping the clock.
    pub fn reset(&mut self) {
        let clock = self.cpu_clock_hz;
        *self = Self::new(clock);
    }

    /// Fires the axis one-shots (any write to the port triggers them).
    pub fn write(&mut self, now: u64) {
        self.fire_cycle = now;
        for axis in 0..4 {
            self.axis_discharge[axis] = if self.present[axis / 2] {
                Some(axis_discharge_cycles(
                    self.cpu_clock_hz,
                    self.axis_value[axis],
                ))
            } else {
                None
            };
        }
    }

    /// Reads the port: axis bits 0-3 plus active-low button bits 4-7.
    pub fn read(&self, now: u64) -> u8 {
        let elapsed = now.saturating_sub(self.fire_cycle);
        let mut state = 0u8;
        for axis in 0..4 {
            if let Some(duration) = self.axis_discharge[axis]
                && elapsed < duration
            {
                state |= 1 << axis;
            }
        }
        let mut buttons = self.read_buttons();
        // Button lines read released while any axis one-shot is discharging.
        if state & 0x0F != 0 {
            buttons = 0xF0;
        }
        state | buttons
    }

    /// Sets the analog axes of stick `index` (0 or 1).
    pub fn set_axes(&mut self, index: usize, x: i16, y: i16) {
        if index >= 2 {
            return;
        }
        self.axis_value[index * 2] = i32::from(x);
        self.axis_value[index * 2 + 1] = i32::from(y);
    }

    /// Sets the two buttons of stick `index` (0 or 1).
    pub fn set_buttons(&mut self, index: usize, button1: bool, button2: bool) {
        if index >= 2 {
            return;
        }
        self.buttons[index * 2] = button1;
        self.buttons[index * 2 + 1] = button2;
    }

    /// Marks stick `index` (0 or 1) connected or absent.
    pub fn set_present(&mut self, index: usize, present: bool) {
        if index >= 2 {
            return;
        }
        self.present[index] = present;
    }

    /// Builds the active-low button nibble in bits 4-7.
    fn read_buttons(&self) -> u8 {
        let mut ret = 0xF0u8;
        if self.present[0] {
            if self.buttons[0] {
                ret &= !0x10;
            }
            if self.buttons[1] {
                ret &= !0x20;
            }
        }
        if self.present[1] {
            if self.buttons[2] {
                ret &= !0x40;
            }
            if self.buttons[3] {
                ret &= !0x80;
            }
        }
        ret
    }
}
