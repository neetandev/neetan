//! PPI glue: joystick port reads and ADPCM control on port C.

use common::{JoystickState, Tracing};
use device::i8255::I8255Write;

use super::X68kBus;

/// Port C bit disabling the right ADPCM output line when set.
const PORT_C_ADPCM_RIGHT_OFF: u8 = 0x01;
/// Port C bit disabling the left ADPCM output line when set.
const PORT_C_ADPCM_LEFT_OFF: u8 = 0x02;
/// Shift of the two ADPCM divider bits within port C.
const PORT_C_ADPCM_DIVIDER_SHIFT: u8 = 2;

/// Active-low joystick bit for the up direction.
const JOYSTICK_UP: u8 = 0x01;
/// Active-low joystick bit for the down direction.
const JOYSTICK_DOWN: u8 = 0x02;
/// Active-low joystick bit for the left direction.
const JOYSTICK_LEFT: u8 = 0x04;
/// Active-low joystick bit for the right direction.
const JOYSTICK_RIGHT: u8 = 0x08;
/// Active-low joystick bit for button A.
const JOYSTICK_BUTTON_A: u8 = 0x20;
/// Active-low joystick bit for button B.
const JOYSTICK_BUTTON_B: u8 = 0x40;

/// Encodes a two-button pad as the active-low PPI port byte.
///
/// Opposing directions cancel each other; the select chord pulls both up and
/// down low, and the run chord pulls both left and right low.
fn encode_joystick(state: JoystickState) -> u8 {
    let mut value = 0xFF;
    if state.up && !state.down {
        value &= !JOYSTICK_UP;
    }
    if state.down && !state.up {
        value &= !JOYSTICK_DOWN;
    }
    if state.select {
        value &= !(JOYSTICK_UP | JOYSTICK_DOWN);
    }
    if state.left && !state.right {
        value &= !JOYSTICK_LEFT;
    }
    if state.right && !state.left {
        value &= !JOYSTICK_RIGHT;
    }
    if state.run {
        value &= !(JOYSTICK_LEFT | JOYSTICK_RIGHT);
    }
    if state.trigger1 {
        value &= !JOYSTICK_BUTTON_A;
    }
    if state.trigger2 {
        value &= !JOYSTICK_BUTTON_B;
    }
    value
}

impl<T: Tracing> X68kBus<T> {
    /// Updates the pad state feeding the PPI joystick port at `index`.
    pub fn set_joystick(&mut self, index: usize, state: JoystickState) {
        if let Some(port) = self.joystick_ports.get_mut(index) {
            *port = state;
        }
    }

    /// Reads a PPI register byte at an odd address.
    pub(super) fn read_ppi_register(&mut self, address: u32) -> u8 {
        let select = ((address & 7) >> 1) as u8;
        self.ppi.set_port_a(encode_joystick(self.joystick_ports[0]));
        self.ppi.set_port_b(encode_joystick(self.joystick_ports[1]));
        self.ppi.read(select)
    }

    /// Writes a PPI register byte at an odd address.
    pub(super) fn write_ppi_register(&mut self, address: u32, value: u8) {
        let select = ((address & 7) >> 1) as u8;
        match self.ppi.write(select, value) {
            I8255Write::PortC | I8255Write::Mode => self.apply_ppi_port_c(),
            I8255Write::PortA | I8255Write::PortB | I8255Write::None => {}
        }
    }

    /// Routes the port C latch to the ADPCM divider and output lines.
    fn apply_ppi_port_c(&mut self) {
        let value = self.ppi.port_c();
        let old_period = self.adpcm_byte_period();
        self.adpcm
            .set_divider((value >> PORT_C_ADPCM_DIVIDER_SHIFT) & 3);
        self.retime_adpcm_byte_event(old_period);
        self.adpcm.set_output_enable(
            value & PORT_C_ADPCM_LEFT_OFF == 0,
            value & PORT_C_ADPCM_RIGHT_OFF == 0,
        );
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, JoystickState, M68000AccessSize, M68000FunctionCode};

    use crate::{
        X68kBus, X68kModel,
        bus::test_support::{access, bus},
    };

    fn write_register(bus: &mut X68kBus, address: u32, value: u8) {
        bus.m68000_write(
            access(
                address,
                M68000AccessSize::Byte,
                M68000FunctionCode::SupervisorData,
            ),
            u16::from(value),
        )
        .unwrap();
    }

    fn read_register(bus: &mut X68kBus, address: u32) -> u8 {
        bus.m68000_read(access(
            address,
            M68000AccessSize::Byte,
            M68000FunctionCode::SupervisorData,
        ))
        .unwrap() as u8
    }

    #[test]
    fn port_c_writes_select_the_adpcm_divider_and_output_lines() {
        let mut bus = bus(X68kModel::X68000);
        write_register(&mut bus, 0xE9A007, 0x92);
        write_register(&mut bus, 0xE9A005, 0x0B);
        assert_eq!(read_register(&mut bus, 0xE9A005), 0x0B);
        assert_eq!(bus.adpcm.divider_ratio(), Some(512));
        assert_eq!(bus.adpcm.sampling_rate_hz(), Some(15_625));

        write_register(&mut bus, 0xE9A005, 0x00);
        assert_eq!(bus.adpcm.divider_ratio(), Some(1024));

        write_register(&mut bus, 0xE9A005, 0x04);
        assert_eq!(bus.adpcm.divider_ratio(), Some(768));

        write_register(&mut bus, 0xE9A005, 0x0C);
        assert_eq!(bus.adpcm.divider_ratio(), None);
    }

    #[test]
    fn joystick_ports_report_directions_and_buttons_active_low() {
        let mut bus = bus(X68kModel::X68000);
        write_register(&mut bus, 0xE9A007, 0x92);
        assert_eq!(read_register(&mut bus, 0xE9A001), 0xFF);
        assert_eq!(read_register(&mut bus, 0xE9A003), 0xFF);

        bus.set_joystick(
            0,
            JoystickState {
                up: true,
                trigger1: true,
                ..JoystickState::default()
            },
        );
        bus.set_joystick(
            1,
            JoystickState {
                right: true,
                trigger2: true,
                ..JoystickState::default()
            },
        );
        assert_eq!(read_register(&mut bus, 0xE9A001), 0xDE);
        assert_eq!(read_register(&mut bus, 0xE9A003), 0xB7);

        // The window mirrors every 8 bytes.
        assert_eq!(read_register(&mut bus, 0xE9A009), 0xDE);

        bus.set_joystick(0, JoystickState::default());
        assert_eq!(read_register(&mut bus, 0xE9A001), 0xFF);
    }

    #[test]
    fn opposing_directions_cancel_unless_chorded() {
        let mut bus = bus(X68kModel::X68000);
        write_register(&mut bus, 0xE9A007, 0x92);

        bus.set_joystick(
            0,
            JoystickState {
                up: true,
                down: true,
                left: true,
                right: true,
                ..JoystickState::default()
            },
        );
        assert_eq!(read_register(&mut bus, 0xE9A001), 0xFF);

        // Select pulls up and down low together, run pulls left and right.
        bus.set_joystick(
            0,
            JoystickState {
                select: true,
                ..JoystickState::default()
            },
        );
        assert_eq!(read_register(&mut bus, 0xE9A001), 0xFC);

        bus.set_joystick(
            0,
            JoystickState {
                run: true,
                ..JoystickState::default()
            },
        );
        assert_eq!(read_register(&mut bus, 0xE9A001), 0xF3);
    }

    #[test]
    fn out_of_range_joystick_indices_are_ignored() {
        let mut bus = bus(X68kModel::X68000);
        write_register(&mut bus, 0xE9A007, 0x92);
        bus.set_joystick(
            2,
            JoystickState {
                up: true,
                ..JoystickState::default()
            },
        );
        assert_eq!(read_register(&mut bus, 0xE9A001), 0xFF);
        assert_eq!(read_register(&mut bus, 0xE9A003), 0xFF);
    }

    #[test]
    fn control_bit_set_reset_matches_direct_port_c_writes() {
        let mut bus = bus(X68kModel::X68000);
        write_register(&mut bus, 0xE9A007, 0x92);
        write_register(&mut bus, 0xE9A005, 0x0B);
        // Clear PC1 and PC0 through bit set/reset to enable both outputs.
        write_register(&mut bus, 0xE9A007, 0x02);
        write_register(&mut bus, 0xE9A007, 0x00);
        assert_eq!(read_register(&mut bus, 0xE9A005), 0x08);
        // Set PC2 while PC3 remains set to select the inhibited state.
        write_register(&mut bus, 0xE9A007, 0x05);
        assert_eq!(bus.adpcm.divider_ratio(), None);
        // Clear PC3 to leave the 1/768 selection.
        write_register(&mut bus, 0xE9A007, 0x06);
        assert_eq!(bus.adpcm.divider_ratio(), Some(768));
    }
}
