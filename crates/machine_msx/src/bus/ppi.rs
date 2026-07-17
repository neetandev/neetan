//! MSX i8255 wiring.

use device::i8255::{I8255, I8255Write};

/// Mode 0 with ports A and C as outputs and port B as input.
const MODE_MSX: u8 = 0x82;
/// Port C cassette motor bit.
const CASSETTE_MOTOR_BIT: u8 = 0x10;
/// Port C cassette output bit.
const CASSETTE_OUTPUT_BIT: u8 = 0x20;
/// Port C Caps LED bit.
const CAPS_LED_BIT: u8 = 0x40;
/// Port C keyboard click bit.
const KEYBOARD_CLICK_BIT: u8 = 0x80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PpiEffect {
    pub(crate) primary_slots: Option<u8>,
    pub(crate) motor_changed: bool,
    pub(crate) click_changed: bool,
}

pub(crate) struct MsxPpi {
    ppi: I8255,
    selected_row: u8,
    cassette_motor: bool,
    cassette_output: bool,
    caps_led: bool,
    keyboard_click: bool,
}

save_state::runtime_state! {
/// Mutable MSX PPI and external output state.
#[derive(Clone)]
pub(crate) struct MsxPpiState {
    ppi: device::i8255::I8255State,
    selected_row: u8,
    cassette_motor: bool,
    cassette_output: bool,
    caps_led: bool,
    keyboard_click: bool,
}}

impl MsxPpi {
    pub(crate) fn new() -> Self {
        let mut ppi = I8255::new();
        ppi.set_port_a(0);
        ppi.set_port_b(0xFF);
        ppi.set_port_c(0xFF);
        Self {
            ppi,
            selected_row: 0x0F,
            cassette_motor: false,
            cassette_output: false,
            caps_led: false,
            keyboard_click: false,
        }
    }

    pub(crate) fn read(&mut self, offset: u8, keyboard_row: u8) -> u8 {
        self.ppi.set_port_b(keyboard_row);
        self.ppi.read(offset)
    }

    pub(crate) fn write(&mut self, offset: u8, value: u8) -> PpiEffect {
        let changed = self.ppi.write(offset, value);
        let mut effect = PpiEffect::default();
        if matches!(changed, I8255Write::PortA | I8255Write::Mode)
            && self.ppi.state.read_mask_a == 0
        {
            effect.primary_slots = Some(self.ppi.port_a());
        }
        if matches!(changed, I8255Write::PortC | I8255Write::Mode) {
            self.apply_port_c(&mut effect);
        }
        effect
    }

    pub(crate) const fn selected_row(&self) -> u8 {
        self.selected_row
    }

    pub(crate) const fn cassette_motor(&self) -> bool {
        self.cassette_motor
    }

    pub(crate) const fn cassette_output(&self) -> bool {
        self.cassette_output
    }

    pub(crate) const fn caps_led(&self) -> bool {
        self.caps_led
    }

    pub(crate) const fn keyboard_click(&self) -> bool {
        self.keyboard_click
    }

    pub(crate) fn select_primary_slots_for_synthetic_program(&mut self, value: u8) {
        self.write(3, MODE_MSX);
        self.write(0, value);
    }

    /// Captures the PPI and its decoded output signals.
    pub(crate) fn capture_state(&self) -> MsxPpiState {
        MsxPpiState {
            ppi: self.ppi.state.clone(),
            selected_row: self.selected_row,
            cassette_motor: self.cassette_motor,
            cassette_output: self.cassette_output,
            caps_led: self.caps_led,
            keyboard_click: self.keyboard_click,
        }
    }

    /// Restores the PPI and its decoded output signals.
    pub(crate) fn restore_state(
        &mut self,
        state: MsxPpiState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.selected_row > 0x0F {
            return Err(save_state::StateValidationError::new(
                "MSX PPI keyboard row is invalid",
            ));
        }
        self.ppi.state = state.ppi;
        self.selected_row = state.selected_row;
        self.cassette_motor = state.cassette_motor;
        self.cassette_output = state.cassette_output;
        self.caps_led = state.caps_led;
        self.keyboard_click = state.keyboard_click;
        Ok(())
    }

    fn apply_port_c(&mut self, effect: &mut PpiEffect) {
        let output_mask = !self.ppi.state.read_mask_c;
        let value = self.ppi.port_c();
        if output_mask & 0x0F != 0 {
            self.selected_row = value & 0x0F;
        }
        if output_mask & CASSETTE_MOTOR_BIT != 0 {
            let cassette_motor = value & CASSETTE_MOTOR_BIT == 0;
            effect.motor_changed = cassette_motor != self.cassette_motor;
            self.cassette_motor = cassette_motor;
        }
        if output_mask & CASSETTE_OUTPUT_BIT != 0 {
            self.cassette_output = value & CASSETTE_OUTPUT_BIT != 0;
        }
        if output_mask & CAPS_LED_BIT != 0 {
            self.caps_led = value & CAPS_LED_BIT == 0;
        }
        if output_mask & KEYBOARD_CLICK_BIT != 0 {
            let keyboard_click = value & KEYBOARD_CLICK_BIT != 0;
            effect.click_changed = keyboard_click != self.keyboard_click;
            self.keyboard_click = keyboard_click;
        }
    }
}

impl Default for MsxPpi {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_a_changes_slots_only_while_configured_as_output() {
        let mut ppi = MsxPpi::new();
        assert_eq!(ppi.write(0, 0xE4).primary_slots, None);
        assert_eq!(ppi.write(3, MODE_MSX).primary_slots, Some(0));
        assert_eq!(ppi.write(0, 0xE4).primary_slots, Some(0xE4));
        assert_eq!(ppi.read(0, 0xFF), 0xE4);
    }

    #[test]
    fn port_b_reads_the_selected_keyboard_row() {
        let mut ppi = MsxPpi::new();
        ppi.write(3, MODE_MSX);
        ppi.write(2, 0x03);
        assert_eq!(ppi.selected_row(), 3);
        assert_eq!(ppi.read(1, 0xA5), 0xA5);
    }

    #[test]
    fn port_c_controls_cassette_and_leds() {
        let mut ppi = MsxPpi::new();
        let effect = ppi.write(3, MODE_MSX);
        assert!(effect.motor_changed);
        assert!(ppi.cassette_motor());
        assert!(ppi.caps_led());
        ppi.write(2, 0xF0);
        assert!(!ppi.cassette_motor());
        assert!(ppi.cassette_output());
        assert!(!ppi.caps_led());
        assert!(ppi.keyboard_click());
    }
}
