//! FM-77AV boot RAM and initiator ROM overlay tests.

mod harness;

use harness::build_av_bus_with_synthetic_roms;
use machinefm7::BootMode;

/// Initiator-ROM offset seeding the boot RAM in BASIC boot mode.
const INITIATOR_BASIC_SEED: usize = 0x1800;
/// Initiator-ROM offset seeding the boot RAM in DOS boot mode.
const INITIATOR_DOS_SEED: usize = 0x1A00;
/// Initiator-ROM offset of the reset vector fetched while the initiator is
/// active.
const INITIATOR_RESET_VECTOR: usize = 0x1FFE;

#[test]
fn boot_ram_is_seeded_from_the_initiator_per_boot_mode() {
    let basic_bus = build_av_bus_with_synthetic_roms(BootMode::Basic, |roms| {
        let initiator = roms.initiate.as_mut().expect("AV initiator ROM present");
        initiator[INITIATOR_BASIC_SEED] = 0xB1;
        initiator[INITIATOR_DOS_SEED] = 0xD0;
    });
    // 0xFE00 reads the boot RAM, seeded from the BASIC-mode initiator offset.
    assert_eq!(basic_bus.peek_byte(0xFE00), 0xB1);

    let dos_bus = build_av_bus_with_synthetic_roms(BootMode::Dos, |roms| {
        let initiator = roms.initiate.as_mut().expect("AV initiator ROM present");
        initiator[INITIATOR_BASIC_SEED] = 0xB1;
        initiator[INITIATOR_DOS_SEED] = 0xD0;
    });
    assert_eq!(dos_bus.peek_byte(0xFE00), 0xD0);
}

#[test]
fn fd10_switches_the_reset_vector_source() {
    let mut bus = build_av_bus_with_synthetic_roms(BootMode::Basic, |roms| {
        let initiator = roms.initiate.as_mut().expect("AV initiator ROM present");
        initiator[INITIATOR_RESET_VECTOR] = 0x12;
        initiator[INITIATOR_RESET_VECTOR + 1] = 0x34;
    });

    // While the initiator is enabled the reset vector comes from the initiator.
    assert!(bus.initiator_enabled());
    assert_eq!(bus.peek_byte(0xFFFE), 0x12);
    assert_eq!(bus.peek_byte(0xFFFF), 0x34);

    // Writing 0xFD10 bit 1 hands off to the boot RAM, whose reset vector is forced
    // to the boot entry 0xFE00.
    bus.write_byte(0xFD10, 0x02);
    assert!(!bus.initiator_enabled());
    assert_eq!(bus.peek_byte(0xFFFE), 0xFE);
    assert_eq!(bus.peek_byte(0xFFFF), 0x00);
}

#[test]
fn fd93_bit0_gates_boot_ram_writes() {
    let mut bus = build_av_bus_with_synthetic_roms(BootMode::Basic, |_| {});

    // Boot-RAM writes are enabled out of reset.
    bus.write_byte(0xFE00, 0x55);
    assert_eq!(bus.peek_byte(0xFE00), 0x55);

    // Clearing 0xFD93 bit 0 write-protects the boot RAM.
    bus.write_byte(0xFD93, 0x00);
    bus.write_byte(0xFE00, 0xAA);
    assert_eq!(bus.peek_byte(0xFE00), 0x55);

    // Re-enabling it restores writes.
    bus.write_byte(0xFD93, 0x01);
    bus.write_byte(0xFE00, 0xAA);
    assert_eq!(bus.peek_byte(0xFE00), 0xAA);
}
