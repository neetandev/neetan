//! Integration tests for the FM Towns input devices: the serial keyboard packet
//! path and the game-pad port, driven through the public `Machine` host-input
//! API and read back through their I/O ports.

#[path = "common/harness.rs"]
mod harness;

use common::{Bus, JoystickState, Machine};
use harness::machine_mx;

/// FM Towns JIS scancode for the 'A' key.
const SCANCODE_A: u8 = 0x1E;
/// Flag byte prefixing a JIS key-press packet (bit 7 set, JIS type in 5-6).
const FLAG_JIS_PRESS: u8 = 0xA0;

/// A forwarded key event expands into the two-byte serial packet (flag byte then
/// scancode), sets the data-available status, and raises the keyboard interrupt
/// while enabled.
#[test]
fn keyboard_event_expands_into_serial_packet() {
    let mut machine = machine_mx();
    // Enable the keyboard interrupt (0x0604 bit 0).
    machine.bus.io_write_byte(0x0604, 0x01);

    machine.push_keyboard_scancode(SCANCODE_A);

    // Data is available and the interrupt is pending.
    assert_eq!(machine.bus.io_read_byte(0x0602) & 0x01, 0x01);
    assert_eq!(machine.bus.io_read_byte(0x0604) & 0x01, 0x01);

    // The packet reads out flag byte then scancode.
    assert_eq!(machine.bus.io_read_byte(0x0600), FLAG_JIS_PRESS);
    assert_eq!(machine.bus.io_read_byte(0x0600), SCANCODE_A);

    // FIFO drained: status clears.
    assert_eq!(machine.bus.io_read_byte(0x0602) & 0x01, 0x00);
}

/// The game pad reports its directions active-low on port 0 (0x04D0): idle reads
/// the direction bits high, and pressing a direction pulls its bit low.
#[test]
fn game_pad_direction_reads_active_low() {
    let mut machine = machine_mx();

    // Idle: the UP bit (0x01) reads high (released).
    let idle = machine.bus.io_read_byte(0x04D0);
    assert_eq!(
        idle & 0x01,
        0x01,
        "released UP must read high (idle={idle:#04X})"
    );

    // Press UP on port 0.
    machine.set_joystick(
        0,
        JoystickState {
            up: true,
            ..JoystickState::default()
        },
    );

    let pressed = machine.bus.io_read_byte(0x04D0);
    assert_eq!(
        pressed & 0x01,
        0x00,
        "held UP must read low (pressed={pressed:#04X})"
    );
    assert_ne!(
        idle, pressed,
        "the pad read must change when a direction is held"
    );

    // Releasing UP restores the high level.
    machine.set_joystick(0, JoystickState::default());
    assert_eq!(machine.bus.io_read_byte(0x04D0) & 0x01, 0x01);
}
