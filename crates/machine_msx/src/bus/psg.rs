//! MSX YM2149 wiring.

use device::psg::Ym2149;

use super::controller::{MsxControllerDevice, MsxControllerPort};

/// Number of MSX controller ports.
const CONTROLLER_PORT_COUNT: usize = 2;
/// Mouse sequence timeout.
const MOUSE_TIMEOUT_MICROSECONDS: u64 = 1_500;

pub(crate) struct MsxPsg {
    psg: Ym2149,
    controllers: [MsxControllerPort; CONTROLLER_PORT_COUNT],
    keyboard_layout_bit: u8,
    kana_led: bool,
    mouse_timeout_cycles: u64,
}

save_state::runtime_state! {
/// Mutable MSX PSG and controller-port state.
#[derive(Clone)]
pub(crate) struct MsxPsgState {
    psg: device::psg::PsgState,
    controllers: [crate::bus::controller::MsxControllerPortState; CONTROLLER_PORT_COUNT],
    kana_led: bool,
}}

impl MsxPsg {
    /// Creates the machine PSG and its two controller ports.
    pub(crate) fn new(
        keyboard_layout_bit: u8,
        input_clock_numerator: u64,
        input_clock_denominator: u32,
        cpu_clock_hz: u32,
        sample_rate: u32,
    ) -> Self {
        let mut psg = Ym2149::new();
        psg.configure_audio_rational(
            input_clock_numerator,
            input_clock_denominator,
            cpu_clock_hz,
            sample_rate,
        );
        Self {
            psg,
            controllers: [MsxControllerPort::new(), MsxControllerPort::new()],
            keyboard_layout_bit,
            kana_led: false,
            mouse_timeout_cycles: u64::from(cpu_clock_hz) * MOUSE_TIMEOUT_MICROSECONDS / 1_000_000,
        }
    }

    /// Selects the PSG register addressed by subsequent accesses.
    pub(crate) fn address_write(&mut self, value: u8) {
        self.psg.address_w(value);
    }

    /// Writes the selected PSG register.
    pub(crate) fn data_write(&mut self, value: u8, current_cycle: u64) {
        self.psg.data_w_at(value, current_cycle);
        self.apply_port_b_output(current_cycle);
    }

    /// Reads the selected PSG register.
    pub(crate) fn data_read(&mut self, cassette_level: bool) -> u8 {
        let selected = usize::from((self.psg.port_b_output() >> 6) & 1);
        let port_a = self.controllers[selected].read_inputs()
            | self.keyboard_layout_bit
            | (u8::from(cassette_level) << 7);
        self.psg.set_port_a_input(port_a);
        self.psg.set_port_b_input(0xFF);
        self.psg.data_r()
    }

    /// Connects a device to one controller port.
    pub(crate) fn set_controller(&mut self, port: usize, device: MsxControllerDevice) {
        if let Some(controller) = self.controllers.get_mut(port) {
            controller.set_device(device);
        }
    }

    /// Updates a joystick on one controller port.
    pub(crate) fn set_joystick(&mut self, port: usize, state: super::controller::MsxJoystickState) {
        if let Some(controller) = self.controllers.get_mut(port) {
            controller.set_joystick(state);
        }
    }

    /// Accumulates host mouse movement on controller port A.
    pub(crate) fn push_mouse_delta(&mut self, delta_x: i16, delta_y: i16) {
        self.controllers[0].push_mouse_delta(delta_x, delta_y);
    }

    /// Updates mouse buttons on controller port A.
    pub(crate) fn set_mouse_buttons(&mut self, left: bool, right: bool) {
        self.controllers[0].set_mouse_buttons(left, right);
    }

    /// Returns whether the Kana LED is lit.
    pub(crate) const fn kana_led(&self) -> bool {
        self.kana_led
    }

    /// Captures PSG, controller, and LED state.
    pub(crate) fn capture_state(&self) -> MsxPsgState {
        MsxPsgState {
            psg: self.psg.capture_state(),
            controllers: self
                .controllers
                .each_ref()
                .map(MsxControllerPort::capture_state),
            kana_led: self.kana_led,
        }
    }

    /// Restores PSG, controller, and LED state.
    pub(crate) fn restore_state(
        &mut self,
        state: MsxPsgState,
    ) -> Result<(), save_state::StateValidationError> {
        self.psg.restore_state(state.psg)?;
        for (controller, controller_state) in self.controllers.iter_mut().zip(state.controllers) {
            controller.restore_state(controller_state)?;
        }
        self.kana_led = state.kana_led;
        Ok(())
    }

    /// Generates stereo PSG samples through the current machine cycle.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn generate_samples(
        &mut self,
        frame_end_cycle: u64,
        input_clock_numerator: u64,
        input_clock_denominator: u32,
        cpu_clock_hz: u32,
        sample_rate: u32,
        volume: f32,
        output: &mut [f32],
    ) -> usize {
        self.psg.generate_samples_rational(
            frame_end_cycle,
            input_clock_numerator,
            input_clock_denominator,
            cpu_clock_hz,
            sample_rate,
            volume,
            output,
        )
    }

    /// Applies PSG port B to both controller ports.
    fn apply_port_b_output(&mut self, current_cycle: u64) {
        if !self.psg.port_b_is_output() {
            return;
        }
        let value = self.psg.port_b_output();
        self.controllers[0].write_output(
            (value & 0x03) | ((value >> 2) & 0x04),
            current_cycle,
            self.mouse_timeout_cycles,
        );
        self.controllers[1].write_output(
            ((value >> 2) & 0x03) | ((value >> 3) & 0x04),
            current_cycle,
            self.mouse_timeout_cycles,
        );
        self.kana_led = value & 0x80 == 0;
    }
}
