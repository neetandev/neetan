//! Sub-CPU keyboard and RTC mailbox tests (driven through the I/O mailbox).

mod harness;

use harness::{build_machine, run_bus_cycles};
use machine_x1::{X1KeyboardMode, X1Model};

/// Sends a sub-CPU command byte and lets several poll ticks run.
fn command(bus: &mut machine_x1::X1Bus, value: u8) {
    bus.io_write(0x1900, value);
    run_bus_cycles(bus, 4_000);
}

/// Reads one result byte from the mailbox, then lets a poll tick advance so the
/// next result byte is staged.
fn result(bus: &mut machine_x1::X1Bus) -> u8 {
    let value = bus.io_read(0x1900).0;
    run_bus_cycles(bus, 2_000);
    value
}

#[test]
fn keydata_read_returns_modifier_then_keycode() {
    let mut machine = build_machine(X1Model::X1);
    let bus = &mut machine.bus;

    bus.push_keyboard_scancode(0x41); // press 'A'
    run_bus_cycles(bus, 2_000);

    command(bus, 0xE6); // key-data read
    let modifier = result(bus);
    let keycode = result(bus);

    assert_ne!(modifier, 0x00);
    assert_eq!(keycode, 0x61); // 'a'
}

#[test]
fn game_key_read_reports_live_key_matrix() {
    // Only the mode-B keyboard reports the live matrix.
    let mut machine = build_machine(X1Model::X1Turbo);
    let bus = &mut machine.bus;
    bus.set_keyboard_mode(X1KeyboardMode::ModeB);

    // Press 'Q' (VK 0x51) and 'Z' (VK 0x5A): the game-key scan puts them in bits
    // 0x80 and 0x04 of the first byte.
    bus.push_keyboard_scancode(0x51);
    bus.push_keyboard_scancode(0x5A);
    run_bus_cycles(bus, 2_000);

    command(bus, 0xE3); // game key read (3 result bytes)
    assert_eq!(result(bus), 0x80 | 0x04);
    assert_eq!(result(bus), 0x00);
    assert_eq!(result(bus), 0x00);

    // Releasing 'Q' clears its bit.
    bus.push_keyboard_scancode(0x51 | 0x80);
    run_bus_cycles(bus, 2_000);
    command(bus, 0xE3);
    assert_eq!(result(bus), 0x04);
}

#[test]
fn turbo_uses_mode_b_kana_table() {
    // With kana lock engaged, a turbo keyboard switched to mode B maps the '1'
    // key through the mode-B kana table (0xB1) where mode A would give 0xC7.
    let mut machine = build_machine(X1Model::X1Turbo);
    let bus = &mut machine.bus;
    bus.set_keyboard_mode(X1KeyboardMode::ModeB);

    bus.push_keyboard_scancode(0x15); // toggle kana lock
    run_bus_cycles(bus, 2_000);
    bus.push_keyboard_scancode(0x31); // press '1'
    run_bus_cycles(bus, 2_000);

    command(bus, 0xE6); // key-data read
    let _modifier = result(bus);
    assert_eq!(result(bus), 0xB1);
}

#[test]
fn clock_get_command_returns_bcd_time() {
    let mut machine = build_machine(X1Model::X1);
    let bus = &mut machine.bus;

    // Set time to 12:34:56 (command 0xEE, three BCD parameters).
    bus.io_write(0x1900, 0xEE);
    run_bus_cycles(bus, 2_000);
    bus.io_write(0x1900, 0x12);
    run_bus_cycles(bus, 2_000);
    bus.io_write(0x1900, 0x34);
    run_bus_cycles(bus, 2_000);
    bus.io_write(0x1900, 0x56);
    run_bus_cycles(bus, 2_000);

    command(bus, 0xEF); // get time
    assert_eq!(result(bus), 0x12);
    assert_eq!(result(bus), 0x34);
    assert_eq!(result(bus), 0x56);
}
