//! MSX general-purpose controller ports.

/// Controller input bits exposed by PSG port A.
const CONTROLLER_INPUT_MASK: u8 = 0x3F;
/// Mouse strobe line on controller pin 8.
const MOUSE_STROBE: u8 = 0x04;
/// Host movement divisor used by an MSX mouse.
const MOUSE_HOST_SCALE: i32 = 2;
/// Minimum movement that the mouse joystick mode reports.
const MOUSE_JOYSTICK_THRESHOLD: i32 = 2;
/// Largest signed movement returned by one mouse scan.
const MOUSE_DELTA_LIMIT: i32 = 127;

/// Active nibble in the two-pass MSX mouse readout sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MousePhase {
    XHighFirst,
    XLowFirst,
    YHighFirst,
    YLowFirst,
    XHighSecond,
    XLowSecond,
    YHighSecond,
    YLowSecond,
}

/// Mutable state of an MSX mouse.
struct MsxMouse {
    accumulated_x: i32,
    accumulated_y: i32,
    joystick_x: i32,
    joystick_y: i32,
    fractional_x: i32,
    fractional_y: i32,
    latched_x: u8,
    latched_y: u8,
    left: bool,
    right: bool,
    phase: MousePhase,
    last_write_cycle: u64,
    native_protocol_seen: bool,
}

impl MsxMouse {
    /// Creates an idle mouse awaiting the first rising strobe.
    const fn new() -> Self {
        Self {
            accumulated_x: 0,
            accumulated_y: 0,
            joystick_x: 0,
            joystick_y: 0,
            fractional_x: 0,
            fractional_y: 0,
            latched_x: 0,
            latched_y: 0,
            left: false,
            right: false,
            phase: MousePhase::YLowSecond,
            last_write_cycle: 0,
            native_protocol_seen: false,
        }
    }

    /// Accumulates relative host movement in MSX mouse coordinates.
    fn push_delta(&mut self, delta_x: i16, delta_y: i16) {
        let movement_x = Self::scale_delta(delta_x, &mut self.fractional_x);
        let movement_y = Self::scale_delta(delta_y, &mut self.fractional_y);
        self.accumulated_x = self.accumulated_x.saturating_add(movement_x);
        self.accumulated_y = self.accumulated_y.saturating_add(movement_y);
        self.joystick_x = self.joystick_x.saturating_add(movement_x);
        self.joystick_y = self.joystick_y.saturating_add(movement_y);
    }

    /// Converts one host delta while retaining half-pixel movement.
    fn scale_delta(delta: i16, fractional: &mut i32) -> i32 {
        let scaled = i32::from(delta) + *fractional;
        *fractional = scaled.rem_euclid(MOUSE_HOST_SCALE);
        -scaled.div_euclid(MOUSE_HOST_SCALE)
    }

    /// Updates the two active-low mouse buttons.
    fn set_buttons(&mut self, left: bool, right: bool) {
        self.left = left;
        self.right = right;
    }

    /// Returns the current nibble and button lines.
    fn read_inputs(&mut self) -> u8 {
        if !self.native_protocol_seen {
            return self.read_joystick_inputs();
        }
        let nibble = match self.phase {
            MousePhase::XHighFirst | MousePhase::XHighSecond => self.latched_x >> 4,
            MousePhase::XLowFirst | MousePhase::XLowSecond => self.latched_x & 0x0F,
            MousePhase::YHighFirst | MousePhase::YHighSecond => self.latched_y >> 4,
            MousePhase::YLowFirst | MousePhase::YLowSecond => self.latched_y & 0x0F,
        };
        let mut value = nibble | 0x30;
        if self.left {
            value &= !0x10;
        }
        if self.right {
            value &= !0x20;
        }
        value
    }

    /// Returns one movement pulse in physical mouse joystick mode.
    fn read_joystick_inputs(&mut self) -> u8 {
        let delta_x = std::mem::take(&mut self.joystick_x);
        let delta_y = std::mem::take(&mut self.joystick_y);
        let absolute_x = i64::from(delta_x).abs();
        let absolute_y = i64::from(delta_y).abs();
        let mut value = 0x30;
        if self.left {
            value &= !0x10;
        }
        if self.right {
            value &= !0x20;
        }
        if absolute_x < i64::from(MOUSE_JOYSTICK_THRESHOLD)
            && absolute_y < i64::from(MOUSE_JOYSTICK_THRESHOLD)
        {
            return value;
        }
        if 12 * absolute_x > 5 * absolute_y {
            value |= if delta_x > 0 { 0x08 } else { 0x04 };
        }
        if 12 * absolute_y > 5 * absolute_x {
            value |= if delta_y > 0 { 0x02 } else { 0x01 };
        }
        value
    }

    /// Applies a controller output change at the current CPU cycle.
    fn write_output(
        &mut self,
        output: u8,
        strobe_changed: bool,
        current_cycle: u64,
        timeout_cycles: u64,
    ) {
        if strobe_changed {
            self.native_protocol_seen = true;
            self.joystick_x = 0;
            self.joystick_y = 0;
        }
        if current_cycle.saturating_sub(self.last_write_cycle) > timeout_cycles {
            self.phase = MousePhase::YLowSecond;
        }
        self.last_write_cycle = current_cycle;
        let strobe = output & MOUSE_STROBE != 0;
        self.phase = match (self.phase, strobe) {
            (MousePhase::XHighFirst, false) => MousePhase::XLowFirst,
            (MousePhase::XLowFirst, true) => MousePhase::YHighFirst,
            (MousePhase::YHighFirst, false) => MousePhase::YLowFirst,
            (MousePhase::YLowFirst, true) => {
                self.latched_x = 0;
                self.latched_y = 0;
                MousePhase::XHighSecond
            }
            (MousePhase::XHighSecond, false) => MousePhase::XLowSecond,
            (MousePhase::XLowSecond, true) => MousePhase::YHighSecond,
            (MousePhase::YHighSecond, false) => MousePhase::YLowSecond,
            (MousePhase::YLowSecond, true) => {
                self.latch_movement();
                MousePhase::XHighFirst
            }
            (phase, _) => phase,
        };
    }

    /// Latches one bounded movement byte per axis.
    fn latch_movement(&mut self) {
        let delta_x = self
            .accumulated_x
            .clamp(-MOUSE_DELTA_LIMIT, MOUSE_DELTA_LIMIT);
        let delta_y = self
            .accumulated_y
            .clamp(-MOUSE_DELTA_LIMIT, MOUSE_DELTA_LIMIT);
        self.latched_x = delta_x as i8 as u8;
        self.latched_y = delta_y as i8 as u8;
        self.accumulated_x -= delta_x;
        self.accumulated_y -= delta_y;
    }
}

/// State of an MSX joystick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MsxJoystickState {
    /// Whether up is pressed.
    pub up: bool,
    /// Whether down is pressed.
    pub down: bool,
    /// Whether left is pressed.
    pub left: bool,
    /// Whether right is pressed.
    pub right: bool,
    /// Whether trigger A is pressed.
    pub trigger_a: bool,
    /// Whether trigger B is pressed.
    pub trigger_b: bool,
}

/// Device connected to an MSX controller port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MsxControllerDevice {
    /// No connected controller.
    #[default]
    Empty,
    /// Standard two-button joystick.
    Joystick(MsxJoystickState),
    /// Standard two-button MSX mouse.
    Mouse,
}

pub(crate) struct MsxControllerPort {
    device: MsxControllerDevice,
    output: u8,
    mouse: MsxMouse,
}

save_state::runtime_state! {
/// Mutable state of one MSX controller port.
#[derive(Clone)]
pub(crate) struct MsxControllerPortState {
    device: u8,
    joystick: [bool; 6],
    output: u8,
    accumulated_x: i32,
    accumulated_y: i32,
    joystick_x: i32,
    joystick_y: i32,
    fractional_x: i32,
    fractional_y: i32,
    latched_x: u8,
    latched_y: u8,
    left: bool,
    right: bool,
    phase: u8,
    last_write_cycle: u64,
    native_protocol_seen: bool,
}}

impl MsxControllerPort {
    /// Creates an empty controller port.
    pub(crate) const fn new() -> Self {
        Self {
            device: MsxControllerDevice::Empty,
            output: 0,
            mouse: MsxMouse::new(),
        }
    }

    /// Replaces the connected controller device.
    pub(crate) fn set_device(&mut self, device: MsxControllerDevice) {
        self.device = device;
    }

    /// Updates the joystick and selects it once a control is engaged.
    pub(crate) fn set_joystick(&mut self, state: MsxJoystickState) {
        let engaged = state.up
            || state.down
            || state.left
            || state.right
            || state.trigger_a
            || state.trigger_b;
        if engaged || matches!(self.device, MsxControllerDevice::Joystick(_)) {
            self.device = MsxControllerDevice::Joystick(state);
        }
    }

    /// Accumulates host movement and selects the mouse.
    pub(crate) fn push_mouse_delta(&mut self, delta_x: i16, delta_y: i16) {
        self.device = MsxControllerDevice::Mouse;
        self.mouse.push_delta(delta_x, delta_y);
    }

    /// Updates mouse buttons and selects the mouse on a button press.
    pub(crate) fn set_mouse_buttons(&mut self, left: bool, right: bool) {
        if left || right || matches!(self.device, MsxControllerDevice::Mouse) {
            self.device = MsxControllerDevice::Mouse;
        }
        self.mouse.set_buttons(left, right);
    }

    /// Returns the six controller input lines.
    pub(crate) fn read_inputs(&mut self) -> u8 {
        match self.device {
            MsxControllerDevice::Empty => CONTROLLER_INPUT_MASK,
            MsxControllerDevice::Joystick(state) => {
                let mut value = CONTROLLER_INPUT_MASK;
                value &= !(u8::from(state.up)
                    | (u8::from(state.down) << 1)
                    | (u8::from(state.left) << 2)
                    | (u8::from(state.right) << 3)
                    | (u8::from(state.trigger_a) << 4)
                    | (u8::from(state.trigger_b) << 5));
                value
            }
            MsxControllerDevice::Mouse => self.mouse.read_inputs(),
        }
    }

    /// Captures the connected device and mouse protocol progress.
    pub(crate) fn capture_state(&self) -> MsxControllerPortState {
        let (device, joystick) = match self.device {
            MsxControllerDevice::Empty => (0, [false; 6]),
            MsxControllerDevice::Joystick(state) => (
                1,
                [
                    state.up,
                    state.down,
                    state.left,
                    state.right,
                    state.trigger_a,
                    state.trigger_b,
                ],
            ),
            MsxControllerDevice::Mouse => (2, [false; 6]),
        };
        MsxControllerPortState {
            device,
            joystick,
            output: self.output,
            accumulated_x: self.mouse.accumulated_x,
            accumulated_y: self.mouse.accumulated_y,
            joystick_x: self.mouse.joystick_x,
            joystick_y: self.mouse.joystick_y,
            fractional_x: self.mouse.fractional_x,
            fractional_y: self.mouse.fractional_y,
            latched_x: self.mouse.latched_x,
            latched_y: self.mouse.latched_y,
            left: self.mouse.left,
            right: self.mouse.right,
            phase: self.mouse.phase as u8,
            last_write_cycle: self.mouse.last_write_cycle,
            native_protocol_seen: self.mouse.native_protocol_seen,
        }
    }

    /// Restores the connected device and mouse protocol progress.
    pub(crate) fn restore_state(
        &mut self,
        state: MsxControllerPortState,
    ) -> Result<(), save_state::StateValidationError> {
        let joystick = MsxJoystickState {
            up: state.joystick[0],
            down: state.joystick[1],
            left: state.joystick[2],
            right: state.joystick[3],
            trigger_a: state.joystick[4],
            trigger_b: state.joystick[5],
        };
        self.device = match state.device {
            0 => MsxControllerDevice::Empty,
            1 => MsxControllerDevice::Joystick(joystick),
            2 => MsxControllerDevice::Mouse,
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX controller device is invalid",
                ));
            }
        };
        self.output = state.output;
        self.mouse.accumulated_x = state.accumulated_x;
        self.mouse.accumulated_y = state.accumulated_y;
        self.mouse.joystick_x = state.joystick_x;
        self.mouse.joystick_y = state.joystick_y;
        self.mouse.fractional_x = state.fractional_x;
        self.mouse.fractional_y = state.fractional_y;
        self.mouse.latched_x = state.latched_x;
        self.mouse.latched_y = state.latched_y;
        self.mouse.left = state.left;
        self.mouse.right = state.right;
        self.mouse.phase = match state.phase {
            0 => MousePhase::XHighFirst,
            1 => MousePhase::XLowFirst,
            2 => MousePhase::YHighFirst,
            3 => MousePhase::YLowFirst,
            4 => MousePhase::XHighSecond,
            5 => MousePhase::XLowSecond,
            6 => MousePhase::YHighSecond,
            7 => MousePhase::YLowSecond,
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX mouse phase is invalid",
                ));
            }
        };
        self.mouse.last_write_cycle = state.last_write_cycle;
        self.mouse.native_protocol_seen = state.native_protocol_seen;
        Ok(())
    }

    /// Updates the three controller output lines.
    pub(crate) fn write_output(&mut self, output: u8, current_cycle: u64, timeout_cycles: u64) {
        let output = output & 0x07;
        if output == self.output {
            return;
        }
        let strobe_changed = (output ^ self.output) & MOUSE_STROBE != 0;
        self.output = output;
        self.mouse
            .write_output(output, strobe_changed, current_cycle, timeout_cycles);
    }
}
