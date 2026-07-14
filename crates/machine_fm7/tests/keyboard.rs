//! FM-7 keyboard tests.

mod harness;

use common::Bus;
use harness::{build_bus_with_synthetic_roms, run_bus_cycles};
use machine_fm7::{BootMode, Fm7Bus, SubBusView};

/// Physical scancode of the `Q` key.
const SCANCODE_Q: u8 = 0x11;
/// Physical scancode of the `A` key.
const SCANCODE_A: u8 = 0x1E;
/// Physical scancode of the `1` key.
const SCANCODE_ONE: u8 = 0x02;
/// Physical scancode of the left SHIFT key.
const SCANCODE_SHIFT: u8 = 0x53;
/// Physical scancode of the CAPS key.
const SCANCODE_CAPS: u8 = 0x55;
/// Physical scancode of the KANA key.
const SCANCODE_KANA: u8 = 0x5A;
/// Physical scancode of the F1 key.
const SCANCODE_F1: u8 = 0x5D;
/// Physical scancode of the BREAK key.
const SCANCODE_BREAK: u8 = 0x5C;
/// Release flag OR-ed into a scancode.
const RELEASE: u8 = 0x80;

/// Main-clock cycles that comfortably exceed one 20 ms keyboard latch period.
const ONE_LATCH_PERIOD: u64 = 40_000;

/// `0xFD02` mask bit enabling the keyboard interrupt.
const KEYBOARD_IRQ_ENABLE: u8 = 0x01;

/// Pushes a key press and advances the bus far enough for the latch tick to move
/// the queued keycode into the read latch.
fn press_and_latch(bus: &mut Fm7Bus, scancode: u8) {
    bus.push_keyboard_scancode(scancode);
    run_bus_cycles(bus, ONE_LATCH_PERIOD);
}

/// Reads a sub CPU MMIO byte through a `SubBusView`, honoring read side effects.
fn sub_read(bus: &mut Fm7Bus, address: u16) -> u8 {
    let mut view = SubBusView { bus };
    view.read_byte(u32::from(address))
}

#[test]
fn latches_unmodified_ascii_keycode() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    press_and_latch(&mut bus, SCANCODE_Q);

    assert_eq!(
        bus.read_byte(0xFD00).0 & 0x80,
        0x00,
        "keycode high bit clear"
    );
    assert_eq!(bus.read_byte(0xFD01).0, 0x71, "unmodified Q latches 'q'");
}

#[test]
fn shift_selects_the_shifted_table() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    bus.push_keyboard_scancode(SCANCODE_SHIFT);
    press_and_latch(&mut bus, SCANCODE_Q);

    assert_eq!(bus.read_byte(0xFD01).0, 0x51, "Shift+Q latches 'Q'");
}

#[test]
fn caps_folds_letter_case() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    bus.push_keyboard_scancode(SCANCODE_CAPS);
    press_and_latch(&mut bus, SCANCODE_A);

    assert_eq!(bus.read_byte(0xFD01).0, 0x41, "caps folds 'a' to 'A'");
}

#[test]
fn kana_selects_the_kana_table() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    bus.push_keyboard_scancode(SCANCODE_KANA);
    press_and_latch(&mut bus, SCANCODE_Q);

    assert_eq!(
        bus.read_byte(0xFD01).0,
        0xC0,
        "kana Q latches its katakana code"
    );
}

#[test]
fn function_key_sets_the_ninth_bit() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    press_and_latch(&mut bus, SCANCODE_F1);

    assert_eq!(
        bus.read_byte(0xFD00).0 & 0x80,
        0x80,
        "F1 sets the keycode high bit"
    );
    assert_eq!(
        bus.read_byte(0xFD01).0,
        0x01,
        "F1 low byte is 0x01 (keycode 0x101)"
    );
}

#[test]
fn keycode_reads_the_same_over_main_and_sub_ports() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    press_and_latch(&mut bus, SCANCODE_F1);

    assert_eq!(sub_read(&mut bus, 0xD400), 0xFF, "sub high port bit 7 set");
    assert_eq!(
        sub_read(&mut bus, 0xD401),
        0x01,
        "sub low port matches main"
    );
}

#[test]
fn reading_main_low_port_clears_the_interrupt() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    bus.write_byte(0xFD02, KEYBOARD_IRQ_ENABLE);
    press_and_latch(&mut bus, SCANCODE_Q);

    assert!(
        bus.has_irq(),
        "latched keycode raises the main IRQ when enabled"
    );
    let _ = bus.read_byte(0xFD01).0;
    assert!(!bus.has_irq(), "reading 0xFD01 clears the keyboard IRQ");
}

#[test]
fn reading_sub_low_port_clears_the_interrupt() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    press_and_latch(&mut bus, SCANCODE_Q);

    assert!(
        bus.sub_has_firq(),
        "the sub CPU owns the keyboard FIRQ by default"
    );
    let _ = sub_read(&mut bus, 0xD401);
    assert!(!bus.sub_has_firq(), "reading 0xD401 clears the sub FIRQ");
}

#[test]
fn main_irq_enable_hands_the_keyboard_from_sub_to_main() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    press_and_latch(&mut bus, SCANCODE_ONE);

    // At reset the main keyboard IRQ is masked, so the sub CPU services the key.
    assert!(!bus.has_irq(), "masked main keyboard IRQ stays low");
    assert!(bus.sub_has_firq(), "the sub FIRQ is enabled by default");

    // Claiming the keyboard IRQ on the main side suppresses the sub FIRQ.
    bus.write_byte(0xFD02, KEYBOARD_IRQ_ENABLE);
    assert!(bus.has_irq(), "enabling 0xFD02 bit 0 exposes the main IRQ");
    assert!(
        !bus.sub_has_firq(),
        "the sub FIRQ is suppressed once main claims it"
    );
}

#[test]
fn break_key_drives_the_main_firq() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    assert!(!bus.firq_active(), "FIRQ idle before BREAK");

    bus.push_keyboard_scancode(SCANCODE_BREAK);
    assert!(bus.firq_active(), "BREAK press raises the main FIRQ");
    assert_eq!(
        bus.read_byte(0xFD04).0 & 0x02,
        0x00,
        "0xFD04 bit 1 active-low while held"
    );

    bus.push_keyboard_scancode(SCANCODE_BREAK | RELEASE);
    assert!(!bus.firq_active(), "BREAK release clears the main FIRQ");
    assert_eq!(
        bus.read_byte(0xFD04).0 & 0x02,
        0x02,
        "0xFD04 bit 1 high once released"
    );
}
