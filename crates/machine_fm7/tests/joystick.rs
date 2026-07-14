//! Joystick tests: pad state read back through PSG parallel port A.

mod harness;

use common::JoystickState;
use harness::build_bus_with_synthetic_roms;
use machine_fm7::{BootMode, Fm7Bus};

/// PSG register index of parallel port A.
const PSG_PORT_A: u8 = 14;

/// Reads PSG parallel port A through the `0xFD0D`/`0xFD0E` command latch.
fn read_port_a(bus: &mut Fm7Bus) -> u8 {
    bus.write_byte(0xFD0D, 3);
    bus.write_byte(0xFD0E, PSG_PORT_A);
    bus.write_byte(0xFD0D, 1);
    bus.read_byte(0xFD0E).0
}

#[test]
fn idle_pad_reads_all_high() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    bus.set_joystick(JoystickState::default());
    assert_eq!(read_port_a(&mut bus), 0xFF);
}

#[test]
fn each_input_pulls_its_bit_low() {
    let cases = [
        (
            JoystickState {
                up: true,
                ..Default::default()
            },
            0x01u8,
        ),
        (
            JoystickState {
                down: true,
                ..Default::default()
            },
            0x02,
        ),
        (
            JoystickState {
                left: true,
                ..Default::default()
            },
            0x04,
        ),
        (
            JoystickState {
                right: true,
                ..Default::default()
            },
            0x08,
        ),
        (
            JoystickState {
                trigger1: true,
                ..Default::default()
            },
            0x10,
        ),
        (
            JoystickState {
                trigger2: true,
                ..Default::default()
            },
            0x20,
        ),
    ];

    for (state, bit) in cases {
        let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
        bus.set_joystick(state);
        assert_eq!(read_port_a(&mut bus), !bit);
    }
}

#[test]
fn combined_inputs_clear_multiple_bits() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    bus.set_joystick(JoystickState {
        up: true,
        trigger1: true,
        ..Default::default()
    });
    // 0xFF & !0x01 & !0x10 = 0xEE.
    assert_eq!(read_port_a(&mut bus), 0xEE);
}
