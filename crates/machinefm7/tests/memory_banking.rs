//! Main-memory banking tests.

mod harness;

use harness::build_bus_with_synthetic_roms;
use machinefm7::BootMode;

#[test]
fn fd0f_read_maps_rom_and_write_maps_ram() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |roms| {
        roms.fbasic[0] = 0xA5;
    });

    assert!(bus.basic_rom_mapped());
    assert_eq!(bus.read_byte(0x8000), 0xA5);

    bus.write_byte(0x8000, 0x11);
    bus.write_byte(0xFD0F, 0x00);
    assert!(!bus.basic_rom_mapped());
    assert_eq!(bus.read_byte(0x8000), 0x00);

    bus.write_byte(0x8000, 0x22);
    assert_eq!(bus.read_byte(0x8000), 0x22);

    assert_eq!(bus.read_byte(0xFD0F), 0xFF);
    assert!(bus.basic_rom_mapped());
    assert_eq!(bus.read_byte(0x8000), 0xA5);

    bus.write_byte(0x8000, 0x33);
    bus.write_byte(0xFD0F, 0x00);
    assert_eq!(bus.read_byte(0x8000), 0x22);
}

#[test]
fn boot_mode_selects_default_basic_rom_mapping() {
    let basic = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    let dos = build_bus_with_synthetic_roms(BootMode::Dos, |_| {});

    assert!(basic.basic_rom_mapped());
    assert!(!dos.basic_rom_mapped());
}

#[test]
fn shared_window_returns_safe_defaults_until_sub_cpu_exists() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});

    assert_eq!(bus.read_byte(0xFC80), 0xFF);
    bus.write_byte(0xFC80, 0x42);
    assert_eq!(bus.read_byte(0xFC80), 0xFF);
}
