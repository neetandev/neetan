//! Memory banking tests: the base-X1 ROM/RAM toggle over the bottom 32 KiB.

mod harness;

use harness::{build_machine, build_machine_with_synthetic_roms};
use machinex1::{MonitorTiming, X1Model};

#[test]
fn rom_ram_toggle_selects_the_bottom_half() {
    // A distinctive IPL byte at 0x0000 (0xC3 = JP) distinguishes ROM from the
    // zeroed power-on RAM.
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1, |roms| {
        roms.ipl[0] = 0xC3;
    });

    // Reset state: ROM is mapped in the bottom half.
    assert!(machine.bus.rom_selected());
    assert_eq!(machine.bus.peek_byte(0x0000), 0xC3);

    // A write to 0x1E00 selects work RAM (zeroed) in the bottom half.
    machine.bus.poke_byte(0x0000, 0x00);
    machine.bus.io_write(0x1E00, 0x00);
    assert!(!machine.bus.rom_selected());
    assert_eq!(machine.bus.peek_byte(0x0000), 0x00);

    // A write to 0x1D00 restores ROM.
    machine.bus.io_write(0x1D00, 0x00);
    assert!(machine.bus.rom_selected());
    assert_eq!(machine.bus.peek_byte(0x0000), 0xC3);
}

#[test]
fn upper_half_is_always_ram() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1, |roms| {
        roms.ipl.fill(0xFF);
    });
    // 0xC000 is above the 32 KiB ROM window, so writes land in RAM and read back
    // regardless of the ROM/RAM latch.
    machine.bus.poke_byte(0xC000, 0x5A);
    assert_eq!(machine.bus.peek_byte(0xC000), 0x5A);
    assert!(machine.bus.rom_selected());
}

/// Selects flat RAM bank `bank` via the turbo bank register (`0x0B00`): BMCS (bit
/// 4) clear, bank index in the low nibble.
fn select_bank(machine: &mut machinex1::X1Machine, bank: u8) {
    machine.bus.io_write(0x0B00, bank & 0x0F);
}

#[test]
fn turbo_bank_register_selects_independent_64k_windows() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |roms| {
        roms.ipl.fill(0xFF);
    });

    // Reset selects the base map (BMCS set), so the IPL is visible.
    assert_eq!(machine.bus.io_read(0x0B00), 0x10);
    assert!(machine.bus.rom_selected());

    // Write a distinctive byte at the same address in three different banks.
    select_bank(&mut machine, 0);
    machine.bus.poke_byte(0x0000, 0xA0);
    select_bank(&mut machine, 1);
    machine.bus.poke_byte(0x0000, 0xA1);
    select_bank(&mut machine, 15);
    machine.bus.poke_byte(0x0000, 0xAF);

    // Each bank reads back its own byte; banks do not alias.
    select_bank(&mut machine, 0);
    assert_eq!(machine.bus.peek_byte(0x0000), 0xA0);
    select_bank(&mut machine, 1);
    assert_eq!(machine.bus.peek_byte(0x0000), 0xA1);
    select_bank(&mut machine, 15);
    assert_eq!(machine.bus.peek_byte(0x0000), 0xAF);

    // The bank register reads back the six stored bits.
    assert_eq!(machine.bus.io_read(0x0B00), 0x0F);
}

#[test]
fn turbo_flat_bank_has_no_rom_overlay() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |roms| {
        roms.ipl[0] = 0xC3; // distinctive IPL byte
    });

    // Base map: the IPL shows through the bottom of the address space.
    assert_eq!(machine.bus.peek_byte(0x0000), 0xC3);

    // Selecting a flat bank removes the ROM overlay; the bottom is plain RAM.
    select_bank(&mut machine, 0);
    assert_eq!(machine.bus.peek_byte(0x0000), 0x00);
    machine.bus.poke_byte(0x0000, 0x5A);
    assert_eq!(machine.bus.peek_byte(0x0000), 0x5A);
}

#[test]
fn turbo_base_map_toggle_still_works() {
    // With BMCS set (the reset default), the turbo behaves exactly like the base
    // X1: the 0x1D00/0x1E00 ROM/RAM toggle drives the bottom 32 KiB.
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |roms| {
        roms.ipl[0] = 0xC3;
    });

    assert!(machine.bus.rom_selected());
    assert_eq!(machine.bus.peek_byte(0x0000), 0xC3);

    machine.bus.io_write(0x1E00, 0x00);
    assert!(!machine.bus.rom_selected());
    assert_eq!(machine.bus.peek_byte(0x0000), 0x00);

    machine.bus.io_write(0x1D00, 0x00);
    assert!(machine.bus.rom_selected());
    assert_eq!(machine.bus.peek_byte(0x0000), 0xC3);
}

#[test]
fn base_x1_ignores_the_turbo_bank_register() {
    // The base X1 has no bank register: 0x0B00 is open bus and never banks memory.
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1, |roms| {
        roms.ipl[0] = 0xC3;
    });
    machine.bus.io_write(0x0B00, 0x00); // would select bank 0 on turbo
    assert_eq!(machine.bus.io_read(0x0B00), 0xFF); // open bus
    assert_eq!(machine.bus.peek_byte(0x0000), 0xC3); // IPL still mapped
}

#[test]
fn turbo_dip_switch_reports_monitor_type() {
    let mut turbo = build_machine(X1Model::X1Turbo);
    // Default (Auto) reports a high-resolution 24 kHz monitor: bit 0 = 0.
    assert_eq!(turbo.bus.io_read(0x1FF0), 0x00);

    turbo.bus.set_monitor_timing(MonitorTiming::Fixed15kHz);
    assert_eq!(turbo.bus.io_read(0x1FF0), 0x01); // standard 15 kHz monitor

    turbo.bus.set_monitor_timing(MonitorTiming::Fixed24kHz);
    assert_eq!(turbo.bus.io_read(0x1FF0), 0x00);

    // The base X1 has no DIP port.
    let mut base = build_machine(X1Model::X1);
    assert_eq!(base.bus.io_read(0x1FF0), 0xFF);
}
