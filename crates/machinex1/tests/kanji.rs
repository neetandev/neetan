//! Turbo kanji data-port tests (0x0E80-0x0E82).

mod harness;

use harness::build_machine_with_synthetic_roms;
use machinex1::X1Model;

const KANJI_DATA_LOW: u16 = 0x0E80;
const KANJI_DATA_HIGH: u16 = 0x0E81;
const KANJI_ADDR_LOW: u16 = 0x0E80;
const KANJI_ADDR_HIGH: u16 = 0x0E81;
const KANJI_SELECT: u16 = 0x0E82;

/// Latches `address` into the kanji address register via the select-port edge.
fn latch_address(bus: &mut machinex1::X1Bus, address: u16) {
    bus.io_write(KANJI_ADDR_LOW, (address & 0xFF) as u8);
    bus.io_write(KANJI_ADDR_HIGH, (address >> 8) as u8);
    bus.io_write(KANJI_SELECT, 0x00); // deselect
    bus.io_write(KANJI_SELECT, 0x01); // 0 -> 1 edge latches the address
}

#[test]
fn kanji_port_reads_the_mapped_rom_byte() {
    // jis_convert(0x0E00) maps to kanji ROM offset 0x0E00; the low port reads
    // that byte directly (row 0 of the left half).
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |roms| {
        let kanji = roms.kanji.as_mut().unwrap();
        kanji[0x0E00] = 0xAB;
        kanji[0x0E00 + 0x10] = 0xCD; // right half, same row
    });
    let bus = &mut machine.bus;

    latch_address(bus, 0x0E00);
    assert_eq!(bus.io_read(KANJI_DATA_LOW), 0xAB);
    // The high port returns the right half of the same row.
    assert_eq!(bus.io_read(KANJI_DATA_HIGH), 0xCD);
}

#[test]
fn base_x1_has_no_kanji_port() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1, |_| {});
    let bus = &mut machine.bus;
    assert_eq!(bus.io_read(KANJI_DATA_LOW), 0xFF);
    assert_eq!(bus.io_read(KANJI_DATA_HIGH), 0xFF);
}
