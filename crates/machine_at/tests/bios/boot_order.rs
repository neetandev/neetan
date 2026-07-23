//! Bootstrap boot order: the CMOS boot sequence selects the device tried
//! first, and the bootstrap falls through to the other device on failure.

use common::Cpu;
use machine_at::AtBootDevice;

use super::{
    BOOT_SECTOR_ADDRESS, HALT_BOOT_CODE, create_machine_dx50, make_halt_boot_floppy,
    make_halt_boot_hdd, make_unsigned_boot_hdd, read_ram_u8,
};

/// Asserts the CLI;HLT boot sector landed at 0000:7C00 and DL holds the
/// boot drive.
fn assert_booted(machine: &machine_at::AtMachine<common::NoTrace>, drive: u8, label: &str) {
    for (index, &byte) in HALT_BOOT_CODE.iter().enumerate() {
        assert_eq!(
            read_ram_u8(machine, BOOT_SECTOR_ADDRESS + index as u32),
            byte,
            "{label}: boot sector byte {index}"
        );
    }
    assert_eq!(machine.cpu.dx() & 0xFF, u16::from(drive), "{label}: DL");
}

#[test]
fn floppy_first_boots_the_floppy() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_halt_boot_floppy(), None)
        .expect("insert boot floppy");
    machine
        .bus
        .insert_hdd(0, make_halt_boot_hdd(), None)
        .expect("insert boot hard disk");
    boot_to_halt!(machine);
    assert_booted(&machine, 0x00, "floppy first");
}

#[test]
fn hdd_first_boots_the_hard_disk() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_halt_boot_floppy(), None)
        .expect("insert boot floppy");
    machine
        .bus
        .insert_hdd(0, make_halt_boot_hdd(), None)
        .expect("insert boot hard disk");
    machine.bus.set_boot_device(AtBootDevice::HddFirst);
    boot_to_halt!(machine);
    assert_booted(&machine, 0x80, "hard disk first");
}

#[test]
fn hdd_first_falls_back_to_the_floppy() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_halt_boot_floppy(), None)
        .expect("insert boot floppy");
    machine
        .bus
        .insert_hdd(0, make_unsigned_boot_hdd(), None)
        .expect("insert unsigned hard disk");
    machine.bus.set_boot_device(AtBootDevice::HddFirst);
    boot_to_halt!(machine);
    assert_booted(&machine, 0x00, "unsigned hard disk falls back");
}

#[test]
fn floppy_first_falls_through_to_the_hard_disk() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_hdd(0, make_halt_boot_hdd(), None)
        .expect("insert boot hard disk");
    boot_to_halt!(machine);
    assert_booted(&machine, 0x80, "empty floppy drive falls through");
}
