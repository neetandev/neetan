//! FM-7 kanji ROM I/O tests.
//!
//! Drives the `0xFD20-0xFD23` address/data ports against a synthetic pattern ROM
//! and checks the FM-7 fallback when no kanji ROM is fitted.

mod harness;

use harness::build_bus_with_synthetic_roms;
use machinefm7::BootMode;

/// Size of the JIS level-1 kanji ROM image.
const KANJI_ROM_SIZE: usize = 0x2_0000;

/// A kanji ROM whose every byte equals its low address byte.
fn pattern_rom() -> Vec<u8> {
    (0..KANJI_ROM_SIZE).map(|index| index as u8).collect()
}

#[test]
fn kanji_ports_return_the_selected_glyph_word() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |roms| {
        roms.kanji = Some(pattern_rom());
    });

    // Character code 0x1234 selects the word at byte offset 0x2468.
    bus.write_byte(0xFD20, 0x12);
    bus.write_byte(0xFD21, 0x34);
    assert_eq!(bus.read_byte(0xFD22), 0x68);
    assert_eq!(bus.read_byte(0xFD23), 0x69);

    // The highest code stays within the 128 KiB ROM.
    bus.write_byte(0xFD20, 0xFF);
    bus.write_byte(0xFD21, 0xFF);
    assert_eq!(bus.read_byte(0xFD22), 0xFE);
    assert_eq!(bus.read_byte(0xFD23), 0xFF);
}

#[test]
fn kanji_reads_are_open_bus_without_a_rom() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});

    bus.write_byte(0xFD20, 0x12);
    bus.write_byte(0xFD21, 0x34);
    assert_eq!(bus.read_byte(0xFD22), 0xFF);
    assert_eq!(bus.read_byte(0xFD23), 0xFF);
}
