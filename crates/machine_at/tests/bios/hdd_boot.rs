//! Hard disk boot: a raw AT flat image with a signed CLI;HLT MBR boots to
//! halt; a missing 0x55AA signature falls through to the failure halt loop.

use common::{Bus, Cpu};
use machine_at::AtMachine;

use super::{
    BOOT_SECTOR_ADDRESS, HALT_BOOT_CODE, create_machine_dx50, create_machine_dx66,
    make_halt_boot_hdd, make_unsigned_boot_hdd, read_ram_u8,
};

fn boot_hdd_to_halt(mut machine: AtMachine<common::NoTrace>) {
    machine
        .bus
        .insert_hdd(0, make_halt_boot_hdd(), None)
        .expect("insert boot hard disk");

    let _cycles = boot_to_halt!(machine);

    for (index, &byte) in HALT_BOOT_CODE.iter().enumerate() {
        assert_eq!(
            read_ram_u8(&machine, BOOT_SECTOR_ADDRESS + index as u32),
            byte,
            "boot sector byte {index} at 0x7C00"
        );
    }
    assert_eq!(machine.cpu.dx() & 0x00FF, 0x0080, "DL is the hard disk");
    assert_eq!(
        machine.cpu.sp(),
        0x7C00,
        "boot sector entered with SP 0x7C00"
    );
}

#[test]
fn hdd_boot_halts_dx50() {
    boot_hdd_to_halt(create_machine_dx50());
}

#[test]
fn hdd_boot_halts_dx66() {
    boot_hdd_to_halt(create_machine_dx66());
}

#[test]
fn hdd_boot_without_signature_reaches_the_halt_loop() {
    let mut machine = create_machine_dx66();
    machine
        .bus
        .insert_hdd(0, make_unsigned_boot_hdd(), None)
        .expect("insert unsigned hard disk");

    let _cycles = boot_to_halt!(machine);

    // The boot sector was never entered.
    assert_eq!(read_ram_u8(&machine, BOOT_SECTOR_ADDRESS), 0x00);
    assert_eq!(machine.cpu.cs(), 0xF000, "halted inside the stub ROM");

    // The failure message reached text VRAM.
    let expected = b"No bootable media.";
    for (index, &character) in expected.iter().enumerate() {
        let address = 0xB8000 + (index as u32) * 2;
        assert_eq!(
            machine.bus.read_byte(address),
            character,
            "failure message character {index}"
        );
    }
}
