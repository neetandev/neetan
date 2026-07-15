//! Bus-level tests for the COM1 serial mouse and the analog game port.
//!
//! These build an `AtBus` with placeholder ROM images (the CPU never runs) and
//! drive the devices through the I/O ports, advancing the scheduler by hand.

use common::{Bus, JoystickState, NoTrace};
use machine_at::{AtBus, LoadedRoms};

// COM1 UART register ports.
const COM1_RBR: u16 = 0x03F8;
const COM1_MCR: u16 = 0x03FC;
const COM1_LSR: u16 = 0x03FD;

// Game port (any address in 0x200-0x207 mirrors 0x201).
const GAME_PORT: u16 = 0x0201;

// Modem control bits.
const MCR_DTR: u8 = 0x01;
const MCR_RTS: u8 = 0x02;
const MCR_OUT2: u8 = 0x08;

/// Builds a bus with placeholder ROMs and a 1 MHz-ish clock for simple timing.
fn bus() -> AtBus<NoTrace> {
    let roms = LoadedRoms {
        system_bios: vec![0xFF; 0x1_0000],
        vga_bios: vec![0xFF; 0x8000],
    };
    AtBus::<NoTrace>::new(1_152_000, 16 << 20, roms, 48_000)
}

/// Advances the scheduler clock to `cycle`, firing due events.
fn advance(bus: &mut AtBus<NoTrace>, cycle: u64) {
    bus.set_current_cycle(cycle);
}

#[test]
fn power_on_reset_sends_identification_byte() {
    let mut bus = bus();
    // Raising DTR and RTS powers on the mouse, which answers with 'M'.
    bus.io_write_byte(COM1_MCR, MCR_DTR | MCR_RTS | MCR_OUT2);
    advance(&mut bus, 1000);
    assert_ne!(bus.io_read_byte(COM1_LSR) & 0x01, 0);
    assert_eq!(bus.io_read_byte(COM1_RBR), b'M');
}

#[test]
fn motion_packet_streams_three_bytes() {
    let mut bus = bus();
    // Power on and drain the identification byte.
    bus.io_write_byte(COM1_MCR, MCR_DTR | MCR_RTS | MCR_OUT2);
    advance(&mut bus, 200);
    assert_eq!(bus.io_read_byte(COM1_RBR), b'M');

    // Inject movement; the packet paces out one frame (70 cycles) apart.
    bus.push_mouse_delta(5, -3);
    advance(&mut bus, 300);
    assert_eq!(bus.io_read_byte(COM1_RBR), 0x4C);
    advance(&mut bus, 400);
    assert_eq!(bus.io_read_byte(COM1_RBR), 0x05);
    advance(&mut bus, 500);
    assert_eq!(bus.io_read_byte(COM1_RBR), 0x3D);
}

#[test]
fn game_port_reads_absent_without_a_gamepad() {
    let mut bus = bus();
    bus.io_write_byte(GAME_PORT, 0);
    assert_eq!(bus.io_read_byte(GAME_PORT), 0xF0);
}

#[test]
fn game_port_reports_axes_and_buttons() {
    let mut bus = bus();
    // A connected gamepad forwards analog axes (marking the stick present)
    // and its buttons.
    bus.set_joystick_axes(0, Some((0, 0)));
    bus.set_joystick(
        0,
        JoystickState {
            trigger1: true,
            ..JoystickState::default()
        },
    );

    // Immediately after a write, the axis one-shots read high and the button
    // lines hold released.
    bus.io_write_byte(GAME_PORT, 0);
    let fired = bus.io_read_byte(GAME_PORT);
    assert_eq!(fired & 0x03, 0x03);
    assert_eq!(fired & 0xF0, 0xF0);

    // After the axes discharge, the pressed button 1 reads low.
    let now = bus.current_cycle();
    advance(&mut bus, now + 5000);
    let settled = bus.io_read_byte(GAME_PORT);
    assert_eq!(settled & 0x0F, 0);
    assert_eq!(settled & 0x10, 0);
    assert_ne!(settled & 0x20, 0);
}

#[test]
fn disconnecting_gamepad_clears_analog_port_presence() {
    let mut bus = bus();
    bus.set_joystick_axes(0, Some((0, 0)));
    bus.set_joystick(
        0,
        JoystickState {
            trigger1: true,
            ..JoystickState::default()
        },
    );
    bus.set_joystick_axes(0, None);

    bus.io_write_byte(GAME_PORT, 0);
    let settled_cycle = bus.current_cycle() + 5000;
    advance(&mut bus, settled_cycle);
    assert_eq!(bus.io_read_byte(GAME_PORT), 0xF0);
}
