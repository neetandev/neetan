//! FM-77AV joystick-port mouse tests through the native OPN ports.

mod harness;

use common::JoystickState;
use harness::{build_av_bus_with_synthetic_roms, run_bus_cycles};
use machine_fm7::{BootMode, Fm7Bus};

/// `0xFD15` native OPN command port.
const OPN_COMMAND_PORT: u16 = 0xFD15;
/// `0xFD16` native OPN data port.
const OPN_DATA_PORT: u16 = 0xFD16;

/// Command latching the OPN register address from the data byte.
const OPN_LATCH_ADDRESS: u8 = 3;
/// Command writing the data byte to the latched OPN register.
const OPN_WRITE_DATA: u8 = 2;
/// Command reading the joystick/mouse byte.
const OPN_READ_JOYSTICK: u8 = 9;

/// SSG register index of parallel port B.
const SSG_PORT_B: u8 = 0x0F;

/// Port B values used by the Psy-O-Blade mouse reader.
const PORT_B_STROBE_HIGH: u8 = 0x3F;
const PORT_B_STROBE_LOW: u8 = 0x0F;
const PORT_B_PRESENCE_POLL: u8 = 0x2F;

fn write_opn_register(bus: &mut Fm7Bus, address: u8, value: u8) {
    bus.write_byte(OPN_COMMAND_PORT, OPN_LATCH_ADDRESS);
    bus.write_byte(OPN_DATA_PORT, address);
    bus.write_byte(OPN_COMMAND_PORT, OPN_WRITE_DATA);
    bus.write_byte(OPN_DATA_PORT, value);
}

fn write_port_b(bus: &mut Fm7Bus, value: u8) {
    write_opn_register(bus, SSG_PORT_B, value);
}

fn read_joystick(bus: &mut Fm7Bus) -> u8 {
    bus.write_byte(OPN_COMMAND_PORT, OPN_READ_JOYSTICK);
    bus.read_byte(OPN_DATA_PORT).0
}

#[test]
fn idle_mouse_is_visible_to_the_boot_probe() {
    let mut bus = build_av_bus_with_synthetic_roms(BootMode::Basic, |_| {});

    write_port_b(&mut bus, PORT_B_PRESENCE_POLL);

    assert_eq!(read_joystick(&mut bus), 0xF0);
}

#[test]
fn mouse_delta_reads_negated_high_nibble_first() {
    let mut bus = build_av_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    bus.push_mouse_delta(0x21, -0x12);

    // Wire deltas are negated: X = -0x21 = 0xDF, Y = 0x12.
    write_port_b(&mut bus, PORT_B_STROBE_HIGH);
    assert_eq!(read_joystick(&mut bus), 0xFD);
    write_port_b(&mut bus, PORT_B_STROBE_LOW);
    assert_eq!(read_joystick(&mut bus), 0xFF);
    write_port_b(&mut bus, PORT_B_STROBE_HIGH);
    assert_eq!(read_joystick(&mut bus), 0xF1);
    write_port_b(&mut bus, PORT_B_STROBE_LOW);
    assert_eq!(read_joystick(&mut bus), 0xF2);

    // The accumulators are consumed by the latch; the next sequence is idle.
    write_port_b(&mut bus, PORT_B_STROBE_HIGH);
    assert_eq!(read_joystick(&mut bus), 0xF0);
}

#[test]
fn mouse_buttons_are_active_low_through_the_button_gate() {
    let mut bus = build_av_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    write_port_b(&mut bus, PORT_B_PRESENCE_POLL);

    bus.set_mouse_buttons(true, false);
    assert_eq!(read_joystick(&mut bus), 0xE0);

    bus.set_mouse_buttons(false, true);
    assert_eq!(read_joystick(&mut bus), 0xD0);

    bus.set_mouse_buttons(true, true);
    assert_eq!(read_joystick(&mut bus), 0xC0);

    bus.set_mouse_buttons(false, false);
    assert_eq!(read_joystick(&mut bus), 0xF0);
}

#[test]
fn mouse_timeout_resynchronizes_to_x_high() {
    let mut bus = build_av_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    bus.push_mouse_delta(0x34, 0);

    write_port_b(&mut bus, PORT_B_STROBE_HIGH);
    assert_eq!(read_joystick(&mut bus), 0xFC);
    write_port_b(&mut bus, PORT_B_STROBE_LOW);
    assert_eq!(read_joystick(&mut bus), 0xFC);

    run_bus_cycles(&mut bus, 5_000);
    bus.push_mouse_delta(0x12, 0);

    write_port_b(&mut bus, PORT_B_STROBE_HIGH);
    assert_eq!(read_joystick(&mut bus), 0xFE);
}

#[test]
fn joystick_input_reclaims_the_shared_port() {
    let mut bus = build_av_bus_with_synthetic_roms(BootMode::Basic, |_| {});

    bus.push_mouse_delta(1, 0);
    write_port_b(&mut bus, PORT_B_STROBE_HIGH);
    assert_eq!(read_joystick(&mut bus), 0xFF);

    bus.set_joystick(JoystickState {
        left: true,
        ..JoystickState::default()
    });
    write_port_b(&mut bus, PORT_B_PRESENCE_POLL);

    assert_eq!(read_joystick(&mut bus) & 0x04, 0);
}
