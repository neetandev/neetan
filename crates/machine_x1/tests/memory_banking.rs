//! Memory tests: the base-X1 ROM/RAM toggle and X1turbo lower-window banking.

mod harness;

use harness::{build_machine, build_machine_with_synthetic_roms};
use machine_x1::{MonitorTiming, X1Model};

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

/// Selects lower-window RAM bank `bank` via the turbo bank register (`0x0B00`).
/// BMCS (bit 4) is clear; BMNO aliases physical storage by parity.
fn select_bank(machine: &mut machine_x1::X1Machine, bank: u8) {
    machine.bus.io_write(0x0B00, bank & 0x0F);
}

#[test]
fn turbo_has_64k_main_ram() {
    assert_eq!(X1Model::X1Turbo.work_ram_size(), 0x1_0000);
}

#[test]
fn turbo_bank_register_selects_two_physical_lower_windows() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |roms| {
        roms.ipl.fill(0xFF);
    });

    // Reset selects the base map (BMCS set), so the IPL is visible.
    assert_eq!(machine.bus.io_read(0x0B00), 0x10);
    assert!(machine.bus.rom_selected());

    // Bank 0 and bank 1 are independent. The upper half stays normal main RAM
    // and is shared across bank selections.
    select_bank(&mut machine, 0);
    machine.bus.poke_byte(0x0000, 0xA0);
    machine.bus.poke_byte(0xC000, 0xC0);

    select_bank(&mut machine, 1);
    machine.bus.poke_byte(0x0000, 0xA1);
    assert_eq!(machine.bus.peek_byte(0xC000), 0xC0);
    machine.bus.poke_byte(0xC000, 0xC1);

    select_bank(&mut machine, 0);
    assert_eq!(machine.bus.peek_byte(0x0000), 0xA0);
    assert_eq!(machine.bus.peek_byte(0xC000), 0xC1);

    select_bank(&mut machine, 1);
    assert_eq!(machine.bus.peek_byte(0x0000), 0xA1);
    assert_eq!(machine.bus.peek_byte(0xC000), 0xC1);

    // Higher BMNO values alias the two physical banks by parity.
    select_bank(&mut machine, 2);
    assert_eq!(machine.bus.peek_byte(0x0000), 0xA0);
    machine.bus.poke_byte(0x0000, 0xA2);

    select_bank(&mut machine, 0);
    assert_eq!(machine.bus.peek_byte(0x0000), 0xA2);

    select_bank(&mut machine, 15);
    assert_eq!(machine.bus.io_read(0x0B00), 0x0F);
    assert_eq!(machine.bus.peek_byte(0x0000), 0xA1);
    machine.bus.poke_byte(0x0000, 0xAF);
    assert_eq!(machine.bus.peek_byte(0xC000), 0xC1);

    select_bank(&mut machine, 1);
    assert_eq!(machine.bus.peek_byte(0x0000), 0xAF);
    assert_eq!(machine.bus.peek_byte(0xC000), 0xC1);

    // The bank register reads back the stored byte.
    assert_eq!(machine.bus.io_read(0x0B00), 0x01);
}

#[test]
fn turbo_bank_register_preserves_high_bits_but_maps_bank_parity() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |roms| {
        roms.ipl[0] = 0xC3;
    });

    machine.bus.io_write(0x0B00, 0xE1);
    assert_eq!(machine.bus.io_read(0x0B00), 0xE1);
    machine.bus.poke_byte(0x0000, 0xA1);

    machine.bus.io_write(0x0B00, 0xF1);
    assert_eq!(machine.bus.io_read(0x0B00), 0xF1);
    assert_eq!(machine.bus.peek_byte(0x0000), 0xC3);

    machine.bus.io_write(0x0B00, 0xE3);
    assert_eq!(machine.bus.io_read(0x0B00), 0xE3);
    assert_eq!(machine.bus.peek_byte(0x0000), 0xA1);
}

#[test]
fn turbo_lower_bank_has_no_rom_overlay() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |roms| {
        roms.ipl[0] = 0xC3; // distinctive IPL byte
    });

    // Base map: the IPL shows through the bottom of the address space.
    assert_eq!(machine.bus.peek_byte(0x0000), 0xC3);
    machine.bus.io_write(0x1E00, 0x00);
    machine.bus.poke_byte(0x0000, 0x33);
    assert_eq!(machine.bus.peek_byte(0x0000), 0x33);

    // Selecting a lower-window bank removes the ROM overlay and uses separate
    // storage from main RAM.
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
fn rom_ram_toggle_decodes_full_io_pages() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1, |roms| {
        roms.ipl[0] = 0xC3;
    });

    machine.bus.io_write(0x1E7F, 0x00);
    assert!(!machine.bus.rom_selected());
    machine.bus.poke_byte(0x0000, 0x42);
    assert_eq!(machine.bus.peek_byte(0x0000), 0x42);

    machine.bus.io_write(0x1D80, 0x00);
    assert!(machine.bus.rom_selected());
    assert_eq!(machine.bus.peek_byte(0x0000), 0xC3);

    machine.bus.io_write(0x1EFF, 0x00);
    assert!(!machine.bus.rom_selected());
    assert_eq!(machine.bus.peek_byte(0x0000), 0x42);
}

#[test]
fn read_1exx_selects_ram_and_returns_open_bus() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1, |roms| {
        roms.ipl[0] = 0xC3;
    });

    machine.bus.poke_byte(0x0000, 0x5A);
    assert!(machine.bus.rom_selected());
    assert_eq!(machine.bus.peek_byte(0x0000), 0xC3);

    assert_eq!(machine.bus.io_read(0x1E40), 0xFF);
    assert!(!machine.bus.rom_selected());
    assert_eq!(machine.bus.peek_byte(0x0000), 0x5A);
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
