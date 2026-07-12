//! Host key injection round trip: a set-1 key id pushed into the bus comes
//! back out of the 8042 as the same set-1 byte after the set-2 expansion and
//! the controller's set-2-to-set-1 translation.

use common::Bus;
use machineat::{AT_KEY_CURSOR_UP, AT_KEY_DELETE, AT_KEY_RIGHT_ALT, AtBus, LoadedRoms};

/// Builds a bus with dummy ROM images and translation enabled.
fn test_bus() -> AtBus<common::NoTracing> {
    let roms = LoadedRoms {
        system_bios: vec![0u8; 0x1_0000],
        vga_bios: vec![0u8; 0x8000],
    };
    let mut bus = AtBus::<common::NoTracing>::new(66_000_000, 16 * 1024 * 1024, roms, 48_000);
    // Command byte: translation on, IRQ1 on.
    bus.io_write_byte(0x64, 0x60);
    bus.io_write_byte(0x60, 0x41);
    bus
}

/// Advances the bus far enough to deliver every pending keyboard byte and
/// drains the delivered set-1 bytes from the data port.
fn drain_keyboard_bytes(bus: &mut AtBus<common::NoTracing>) -> Vec<u8> {
    let mut bytes = Vec::new();
    for _ in 0..16 {
        let next = bus.current_cycle() + 66_000_000 / 1_000;
        bus.set_current_cycle(next);
        // Status bit 0 is the output buffer full flag.
        if bus.io_read_byte(0x64) & 0x01 != 0 {
            bytes.push(bus.io_read_byte(0x60));
        }
    }
    bytes
}

#[test]
fn ordinary_keys_round_trip_through_the_translation() {
    let mut bus = test_bus();
    // Every set-1 make code of the 106-key layout's ordinary keys.
    let mut ids: Vec<u8> = (0x01..=0x58).collect();
    ids.extend([0x70, 0x73, 0x79, 0x7B, 0x7D]);
    for id in ids {
        bus.push_key_scancode(id);
        let delivered = drain_keyboard_bytes(&mut bus);
        assert_eq!(delivered, vec![id], "make code for id {id:#04X}");
        bus.push_key_scancode(id | 0x80);
        let delivered = drain_keyboard_bytes(&mut bus);
        assert_eq!(delivered, vec![id | 0x80], "break code for id {id:#04X}");
    }
}

#[test]
fn extended_keys_carry_the_e0_prefix() {
    let mut bus = test_bus();
    bus.push_key_scancode(AT_KEY_CURSOR_UP);
    assert_eq!(drain_keyboard_bytes(&mut bus), vec![0xE0, 0x48]);
    bus.push_key_scancode(AT_KEY_CURSOR_UP | 0x80);
    assert_eq!(drain_keyboard_bytes(&mut bus), vec![0xE0, 0xC8]);
    bus.push_key_scancode(AT_KEY_DELETE);
    assert_eq!(drain_keyboard_bytes(&mut bus), vec![0xE0, 0x53]);
    bus.push_key_scancode(AT_KEY_RIGHT_ALT);
    assert_eq!(drain_keyboard_bytes(&mut bus), vec![0xE0, 0x38]);
}
