//! Floppy boot: a raw 1.44 MB image with a CLI;HLT boot sector boots to halt
//! on both AT models with no ROM files present.

use common::Cpu;
use machine_at::AtMachine;

use super::{
    BOOT_SECTOR_ADDRESS, HALT_BOOT_CODE, create_machine_dx50, create_machine_dx66,
    make_halt_boot_floppy, read_ram_u8,
};

fn boot_floppy_to_halt(mut machine: AtMachine<common::NoTrace>) {
    machine
        .bus
        .insert_floppy(0, make_halt_boot_floppy(), None)
        .expect("insert boot floppy");

    let _cycles = boot_to_halt!(machine);

    for (index, &byte) in HALT_BOOT_CODE.iter().enumerate() {
        assert_eq!(
            read_ram_u8(&machine, BOOT_SECTOR_ADDRESS + index as u32),
            byte,
            "boot sector byte {index} at 0x7C00"
        );
    }
    assert_eq!(machine.cpu.dx() & 0x00FF, 0x0000, "DL is the floppy drive");
    assert_eq!(
        machine.cpu.sp(),
        0x7C00,
        "boot sector entered with SP 0x7C00"
    );
    assert_eq!(machine.cpu.ss(), 0x0000, "boot sector entered with SS 0");
}

#[test]
fn fdd_boot_halts_dx50() {
    boot_floppy_to_halt(create_machine_dx50());
}

#[test]
fn fdd_boot_halts_dx66() {
    boot_floppy_to_halt(create_machine_dx66());
}
