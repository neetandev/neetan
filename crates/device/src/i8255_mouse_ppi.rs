//! PC-98 bus mouse interface on an 8255A at ports 0x7FD9-0x7FDF.
//!
//! Port A carries the button lines in bits 7-5 and one counter nibble in
//! bits 3-0. Port B and the low half of port C carry DIP switch states. The
//! high half of port C drives the interface: HC (bit 7) latches and clears the
//! movement counters, SXY (bit 6) selects the axis, SHL (bit 5) selects the
//! nibble and INT# (bit 4) masks the mouse timer interrupt.
//!
//! The rotary encoders of a real mouse step the counters continuously. The
//! host reports one movement total per frame. This model feeds the counts of
//! a frame into the counters at a fixed rate: every 1000 CPU cycles add the
//! frame total times 1000 divided by the feed divisor. The feed divisor is the
//! CPU clock divided by 56400.
//!
//! Reference: `undoc98/io_mouse.txt` (I/O 7FD9h-7FDFh).

/// Control word bit 7: 1 = mode set, 0 = port C bit set/reset.
const CONTROL_MODE_SET: u8 = 0x80;

/// Bit set/reset word bit 0: 1 = set, 0 = reset.
const CONTROL_BIT_VALUE: u8 = 0x01;

/// Bit set/reset word bits 3-1: port C bit number.
const CONTROL_BIT_NUMBER_MASK: u8 = 0x0E;

/// Mode word bit 4: port A is an input.
const MODE_PORT_A_INPUT: u8 = 0x10;

/// Mode word bit 3: port C bits 7-4 are inputs.
const MODE_PORT_C_HIGH_INPUT: u8 = 0x08;

/// Mode word bit 1: port B is an input.
const MODE_PORT_B_INPUT: u8 = 0x02;

/// Mode word bit 0: port C bits 3-0 are inputs.
const MODE_PORT_C_LOW_INPUT: u8 = 0x01;

/// Mode word written by the system BIOS: port A input, port B input, port C
/// high output and port C low input.
const MODE_BIOS_DEFAULT: u8 = 0x93;

/// Port C bit 7: HC, a rising edge latches and clears the counters.
const PORT_C_HC: u8 = 0x80;

/// Port C bit 6: SXY, 1 = Y counter, 0 = X counter.
const PORT_C_SXY: u8 = 0x40;

/// Port C bit 5: SHL, 1 = high nibble, 0 = low nibble.
const PORT_C_SHL: u8 = 0x20;

/// Port C bit 4: INT#, 1 = mouse timer interrupt masked.
const PORT_C_INT_MASK: u8 = 0x10;

/// Port C bits 7-4.
const PORT_C_HIGH_HALF: u8 = 0xF0;

/// Port C bits 3-0.
const PORT_C_LOW_HALF: u8 = 0x0F;

/// Port C input bit 3: MODSW, 1 = normal display mode.
const PORT_C_MODSW_NORMAL: u8 = 0x08;

/// Port B input bit 6: RAMKL, 1 = the internal RAM at 80000h-9FFFFh is used.
const PORT_B_RAMKL_ENABLED: u8 = 0x40;

/// Port A bit 7: left button, 0 = pressed.
const BUTTON_LEFT_RELEASED: u8 = 0x80;

/// Port A bit 6: middle button, 0 = pressed.
const BUTTON_MIDDLE_RELEASED: u8 = 0x40;

/// Port A bit 5: right button, 0 = pressed.
const BUTTON_RIGHT_RELEASED: u8 = 0x20;

/// Port A button lines with no button pressed.
const BUTTONS_RELEASED: u8 = BUTTON_LEFT_RELEASED | BUTTON_MIDDLE_RELEASED | BUTTON_RIGHT_RELEASED;

/// Port A bits 3-0: MD3-MD0, the selected counter nibble.
const COUNTER_NIBBLE_MASK: u8 = 0x0F;

/// Shift of the high counter nibble that SHL = 1 selects.
const COUNTER_HIGH_NIBBLE_SHIFT: u32 = 4;

/// Port A bits 7-4, the half that carries the button lines.
const PORT_A_UPPER_LINES: u8 = 0xF0;

/// Port A value of an interface without a mouse.
const PORT_A_NO_MOUSE: u8 = 0x70;

/// CPU clock divided by this rate gives the feed divisor.
const FEED_RATE_HZ: u32 = 56_400;

/// CPU cycles of one feed step.
const FEED_STEP_CYCLES: u64 = 1000;

/// Smallest distance in CPU cycles between two feed updates.
const FEED_MINIMUM_CYCLES: u64 = 2000;

/// Feed divisor of an 8 MHz CPU clock.
const FEED_DIVISOR_DEFAULT: u32 = 8_000_000 / FEED_RATE_HZ;

/// Lowest value of an 8-bit movement counter.
const COUNTER_MIN: i16 = -128;

/// Highest value of an 8-bit movement counter.
const COUNTER_MAX: i16 = 127;

save_state::runtime_state! {
/// Mouse interface state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I8255MousePpiState {
    /// 8255A mode word.
    pub mode: u8,
    /// Port A output latch.
    pub port_a: u8,
    /// Port B output latch.
    pub port_b: u8,
    /// Port C output latch.
    pub port_c: u8,
    /// Live X movement counter.
    pub counter_x: i16,
    /// Live Y movement counter.
    pub counter_y: i16,
    /// X counter value latched at the last HC rising edge.
    pub latch_x: i16,
    /// Y counter value latched at the last HC rising edge.
    pub latch_y: i16,
    /// X movement reported for the current frame.
    pub frame_delta_x: i16,
    /// Y movement reported for the current frame.
    pub frame_delta_y: i16,
    /// X movement of the current frame not yet fed into the counter.
    pub pending_x: i16,
    /// Y movement of the current frame not yet fed into the counter.
    pub pending_y: i16,
    /// CPU cycle up to which the feed has added movement to the counters.
    pub feed_cycle: u64,
    /// Button lines in port A layout (active low).
    pub buttons: u8,
    /// Whether a mouse is attached to the interface.
    pub mouse_connected: bool,
}}

/// PC-98 bus mouse interface.
pub struct I8255MousePpi {
    /// Embedded state for save/restore.
    pub state: I8255MousePpiState,
    /// CPU cycles per feed unit, derived from the CPU clock.
    feed_divisor: u32,
}

impl_simple_state_accessors!(I8255MousePpi, I8255MousePpiState);

impl Default for I8255MousePpi {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the movement that `steps` feed steps add for a frame total of
/// `total`. The movement never exceeds the `pending` movement and never runs
/// against it.
fn feed_amount(total: i16, pending: i16, steps: u64, feed_divisor: u32) -> i16 {
    let due = i64::from(total) * steps as i64 / i64::from(feed_divisor.max(1));
    let pending = i64::from(pending);
    due.clamp(pending.min(0), pending.max(0)) as i16
}

/// One axis of the movement feed: the live counter, the movement of the frame
/// still to feed and the frame total.
struct AxisFeed<'a> {
    counter: &'a mut i16,
    pending: &'a mut i16,
    frame_total: &'a mut i16,
}

impl AxisFeed<'_> {
    /// Adds the pending movement to the counter at once and starts a new
    /// frame with the total `delta`.
    fn start_frame(&mut self, delta: i16) {
        *self.counter = self.counter.saturating_add(*self.pending);
        *self.frame_total = delta;
        *self.pending = delta;
    }

    /// Moves the movement of `steps` feed steps from the pending movement
    /// into the counter.
    fn feed(&mut self, steps: u64, feed_divisor: u32) {
        let amount = feed_amount(*self.frame_total, *self.pending, steps, feed_divisor);
        *self.counter = self.counter.saturating_add(amount);
        *self.pending -= amount;
    }
}

/// Counter that a port A read selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CounterSource {
    /// The value latched by the last HC rising edge.
    Latched,
    /// The live counter.
    Live,
}

/// Returns the counter value limited to the 8-bit counter range.
fn counter_value(counter: i16) -> i16 {
    counter.clamp(COUNTER_MIN, COUNTER_MAX)
}

impl I8255MousePpi {
    /// Creates a mouse interface in the state the system BIOS leaves it.
    pub fn new() -> Self {
        Self {
            state: Self::reset_state(PORT_C_HIGH_HALF),
            feed_divisor: FEED_DIVISOR_DEFAULT,
        }
    }

    /// Returns the state after a reset with the given port C latch.
    fn reset_state(port_c: u8) -> I8255MousePpiState {
        I8255MousePpiState {
            mode: MODE_BIOS_DEFAULT,
            port_a: 0,
            port_b: 0,
            port_c,
            counter_x: 0,
            counter_y: 0,
            latch_x: 0,
            latch_y: 0,
            frame_delta_x: 0,
            frame_delta_y: 0,
            pending_x: 0,
            pending_y: 0,
            feed_cycle: 0,
            buttons: BUTTONS_RELEASED,
            mouse_connected: true,
        }
    }

    /// Resets the interface to the BIOS mode word and the given port C latch.
    pub fn reset(&mut self, port_c: u8) {
        self.state = Self::reset_state(port_c);
    }

    /// Derives the feed divisor from the CPU clock.
    pub fn set_cpu_clock(&mut self, cpu_clock_hz: u32) {
        self.feed_divisor = cpu_clock_hz / FEED_RATE_HZ;
    }

    /// Returns whether INT# (port C bit 4) enables the mouse timer interrupt.
    pub fn timer_interrupt_enabled(&self) -> bool {
        self.state.port_c & PORT_C_INT_MASK == 0
    }

    /// Returns the X and Y axes of the movement feed.
    fn axes(&mut self) -> [AxisFeed<'_>; 2] {
        let state = &mut self.state;
        [
            AxisFeed {
                counter: &mut state.counter_x,
                pending: &mut state.pending_x,
                frame_total: &mut state.frame_delta_x,
            },
            AxisFeed {
                counter: &mut state.counter_y,
                pending: &mut state.pending_y,
                frame_total: &mut state.frame_delta_y,
            },
        ]
    }

    /// Reports the host movement of one frame.
    ///
    /// Movement of the previous frame that is still pending enters the
    /// counters at once. The new movement enters at the feed rate.
    pub fn sync_frame(&mut self, delta_x: i16, delta_y: i16, current_cycle: u64) {
        for (mut axis, delta) in self.axes().into_iter().zip([delta_x, delta_y]) {
            axis.start_frame(delta);
        }
        self.state.feed_cycle = current_cycle;
    }

    /// Returns the whole feed steps due at `current_cycle`, or `None` while
    /// less than the minimum feed distance has elapsed.
    fn due_feed_steps(&self, current_cycle: u64) -> Option<u64> {
        let elapsed = current_cycle.wrapping_sub(self.state.feed_cycle);
        (elapsed >= FEED_MINIMUM_CYCLES).then_some(elapsed / FEED_STEP_CYCLES)
    }

    /// Feeds the movement due at `current_cycle` into the counters.
    fn advance_counters(&mut self, current_cycle: u64) {
        let Some(steps) = self.due_feed_steps(current_cycle) else {
            return;
        };
        let feed_divisor = self.feed_divisor;
        for mut axis in self.axes() {
            axis.feed(steps, feed_divisor);
        }
        self.state.feed_cycle += steps * FEED_STEP_CYCLES;
    }

    /// Latches both counters limited to 8 bits and clears them.
    pub fn latch(&mut self, current_cycle: u64) {
        self.advance_counters(current_cycle);
        self.state.latch_x = counter_value(self.state.counter_x);
        self.state.latch_y = counter_value(self.state.counter_y);
        self.state.counter_x = 0;
        self.state.counter_y = 0;
    }

    /// Updates the button lines. `true` means pressed.
    pub fn set_buttons(&mut self, left: bool, right: bool, middle: bool) {
        let mut buttons = BUTTONS_RELEASED;
        if left {
            buttons &= !BUTTON_LEFT_RELEASED;
        }
        if middle {
            buttons &= !BUTTON_MIDDLE_RELEASED;
        }
        if right {
            buttons &= !BUTTON_RIGHT_RELEASED;
        }
        self.state.buttons = buttons;
    }

    /// Returns the counter value that SXY selects from `source`.
    fn selected_counter(&self, source: CounterSource) -> i16 {
        let y_axis = self.state.port_c & PORT_C_SXY != 0;
        match (source, y_axis) {
            (CounterSource::Latched, false) => self.state.latch_x,
            (CounterSource::Latched, true) => self.state.latch_y,
            (CounterSource::Live, false) => self.state.counter_x,
            (CounterSource::Live, true) => self.state.counter_y,
        }
    }

    /// Reads port A (0x7FD9): the button lines and the counter nibble selected
    /// by SXY and SHL. HC = 1 selects the latched value, HC = 0 the live counter.
    /// Bit 6 reads 1 at all times and leaves the middle button line unread.
    pub fn read_port_a(&mut self, current_cycle: u64) -> u8 {
        if self.state.mode & MODE_PORT_A_INPUT == 0 {
            return self.state.port_a;
        }
        if !self.state.mouse_connected {
            return PORT_A_NO_MOUSE;
        }
        self.advance_counters(current_cycle);

        let source = if self.state.port_c & PORT_C_HC != 0 {
            CounterSource::Latched
        } else {
            CounterSource::Live
        };
        let nibble_shift = if self.state.port_c & PORT_C_SHL != 0 {
            COUNTER_HIGH_NIBBLE_SHIFT
        } else {
            0
        };
        let nibble = (self.selected_counter(source) >> nibble_shift) as u8 & COUNTER_NIBBLE_MASK;
        let lines = self.state.buttons & PORT_A_UPPER_LINES;
        lines | BUTTON_MIDDLE_RELEASED | nibble
    }

    /// Reads port B (0x7FDB). As an input it shows RAMKL = 1.
    pub fn read_port_b(&self) -> u8 {
        if self.state.mode & MODE_PORT_B_INPUT != 0 {
            PORT_B_RAMKL_ENABLED
        } else {
            self.state.port_b
        }
    }

    /// Reads port C (0x7FDD). Output halves return the latch. The low input
    /// half shows MODSW = 1 (normal mode), CPUSW = 0 and RS-1/RS-2 = 0. The high
    /// input half has no driver and reads 0.
    pub fn read_port_c(&self) -> u8 {
        let high = if self.state.mode & MODE_PORT_C_HIGH_INPUT != 0 {
            0
        } else {
            self.state.port_c & PORT_C_HIGH_HALF
        };
        let low = if self.state.mode & MODE_PORT_C_LOW_INPUT != 0 {
            PORT_C_MODSW_NORMAL
        } else {
            self.state.port_c & PORT_C_LOW_HALF
        };
        high | low
    }

    /// Writes the port A latch (0x7FD9).
    pub fn write_port_a(&mut self, value: u8) {
        self.state.port_a = value;
    }

    /// Writes the port B latch (0x7FDB).
    pub fn write_port_b(&mut self, value: u8) {
        self.state.port_b = value;
    }

    /// Writes the port C latch (0x7FDD). A rising HC edge latches the
    /// counters. Returns whether the write latched the counters.
    pub fn write_port_c(&mut self, value: u8, current_cycle: u64) -> bool {
        self.update_port_c(value, current_cycle)
    }

    /// Writes the control register (0x7FDF). Returns whether the write
    /// latched the counters.
    ///
    /// A mode word clears all output latches. A bit set/reset word changes one
    /// port C bit, and a rising HC edge latches the counters.
    pub fn write_control(&mut self, value: u8, current_cycle: u64) -> bool {
        if value & CONTROL_MODE_SET != 0 {
            self.state.mode = value;
            self.state.port_a = 0;
            self.state.port_b = 0;
            self.state.port_c = 0;
            return false;
        }
        let bit = 1u8 << ((value & CONTROL_BIT_NUMBER_MASK) >> 1);
        let port_c = if value & CONTROL_BIT_VALUE != 0 {
            self.state.port_c | bit
        } else {
            self.state.port_c & !bit
        };
        self.update_port_c(port_c, current_cycle)
    }

    /// Stores a new port C latch and latches the counters on a rising HC edge.
    fn update_port_c(&mut self, value: u8, current_cycle: u64) -> bool {
        let rising = value & !self.state.port_c & PORT_C_HC != 0;
        self.state.port_c = value;
        if rising {
            self.latch(current_cycle);
        }
        rising
    }
}
