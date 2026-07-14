//! FM-77AV MMR paging and relocatable-window translation tests.

mod harness;

use harness::build_av_bus_with_synthetic_roms;
use machine_fm7::{BootMode, Fm7Bus};

/// `0xFD93` control value enabling MMR translation.
const MMR_ENABLE: u8 = 0x80;
/// `0xFD93` control value enabling the relocatable window.
const WINDOW_ENABLE: u8 = 0x40;
/// `0xFD10` write value disabling the initiator overlay.
const INITIATOR_DISABLE: u8 = 0x02;

/// Builds an AV bus with the initiator overlay handed off, so low addresses are
/// governed by MMR and the window rather than the initiator ROM.
fn av_bus_after_handoff() -> Fm7Bus {
    let mut bus = build_av_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    bus.write_byte(0xFD10, INITIATOR_DISABLE);
    bus
}

#[test]
fn mmr_page_register_selects_the_physical_bank() {
    let mut bus = av_bus_after_handoff();
    bus.write_byte(0xFD93, MMR_ENABLE);
    bus.write_byte(0xFD90, 0x00);

    // Block 0 (0x0000-0x0FFF) -> physical bank 2 (MMR bank-0 DRAM offset 0x2000).
    bus.write_byte(0xFD80, 0x02);
    bus.poke_byte(0x0000, 0x77);
    assert_eq!(bus.peek_byte(0x0000), 0x77);

    // Remapping block 0 to a different bank exposes distinct storage.
    bus.write_byte(0xFD80, 0x03);
    assert_eq!(bus.peek_byte(0x0000), 0x00);
    bus.poke_byte(0x0000, 0x88);

    // Switching back reveals the original bank-2 byte unchanged.
    bus.write_byte(0xFD80, 0x02);
    assert_eq!(bus.peek_byte(0x0000), 0x77);
}

#[test]
fn unfitted_page_two_reads_open_bus() {
    let mut bus = av_bus_after_handoff();
    bus.write_byte(0xFD93, MMR_ENABLE);
    bus.write_byte(0xFD90, 0x00);
    bus.write_byte(0xFD80, 0x20);
    assert_eq!(bus.peek_byte(0x0000), 0xFF);
}

#[test]
fn mmr_disabled_uses_the_straight_compatible_view() {
    let mut bus = av_bus_after_handoff();
    // With MMR on, block 0 maps to a separate physical bank.
    bus.write_byte(0xFD93, MMR_ENABLE);
    bus.write_byte(0xFD90, 0x00);
    bus.write_byte(0xFD80, 0x02);
    bus.poke_byte(0x0000, 0x77);

    // Disabling MMR falls back to the FM-7-compatible RAM at 0x0000.
    bus.write_byte(0xFD93, 0x00);
    bus.poke_byte(0x0000, 0x11);
    assert_eq!(bus.peek_byte(0x0000), 0x11);

    // The MMR-banked byte is untouched by the compatible-view write.
    bus.write_byte(0xFD93, MMR_ENABLE);
    assert_eq!(bus.peek_byte(0x0000), 0x77);
}

#[test]
fn window_relocates_into_bank_zero_by_offset() {
    let mut bus = av_bus_after_handoff();
    bus.write_byte(0xFD93, WINDOW_ENABLE);

    // Offset 0x10 maps 0x7C00 into MMR bank-0 DRAM at 0x1000 + 0x7C00.
    bus.write_byte(0xFD92, 0x10);
    bus.poke_byte(0x7C00, 0x99);
    assert_eq!(bus.peek_byte(0x7C00), 0x99);

    // Changing the offset addresses a different, still-zero location.
    bus.write_byte(0xFD92, 0x20);
    assert_eq!(bus.peek_byte(0x7C00), 0x00);

    // Restoring the offset reveals the original byte.
    bus.write_byte(0xFD92, 0x10);
    assert_eq!(bus.peek_byte(0x7C00), 0x99);
}

#[test]
fn page_registers_are_independent_per_segment() {
    let mut bus = av_bus_after_handoff();

    bus.write_byte(0xFD90, 0x00);
    bus.write_byte(0xFD80, 0x05);
    bus.write_byte(0xFD90, 0x03);
    bus.write_byte(0xFD80, 0x06);

    bus.write_byte(0xFD90, 0x00);
    assert_eq!(bus.read_byte(0xFD80), 0x05);
    bus.write_byte(0xFD90, 0x03);
    assert_eq!(bus.read_byte(0xFD80), 0x06);
}
