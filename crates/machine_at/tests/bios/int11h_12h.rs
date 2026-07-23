//! INT 11h equipment list and INT 12h base memory size.

use machine_at::AtMachine;

use super::{
    RESULT, boot_and_run, create_machine_dx50, create_machine_dx66, inject_and_run, read_ram_u16,
    write_bytes,
};

/// BIOS data area: equipment word.
const BDA_EQUIPMENT: u32 = 0x410;
/// Golden equipment word: FPU, one floppy, color 80-column video, one COM port.
const EQUIPMENT_WORD: u16 = 0x0223;
/// Golden base memory size in KiB.
const BASE_MEMORY_KIB: u16 = 640;

/// Stores the INT 11h and INT 12h results.
#[rustfmt::skip]
const EQUIPMENT_AND_MEMORY_CODE: &[u8] = &[
    0xCD, 0x11,             // INT 11h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xCD, 0x12,             // INT 12h
    0xA3, 0x02, 0x06,       // MOV [0x0602], AX
    0xF4,                   // HLT
];

fn check_equipment_and_memory(mut machine: AtMachine<common::NoTrace>) {
    boot_and_run(&mut machine, EQUIPMENT_AND_MEMORY_CODE, &[], 1_000_000);

    assert_eq!(
        read_ram_u16(&machine, RESULT),
        EQUIPMENT_WORD,
        "INT 11h equipment word"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT + 2),
        BASE_MEMORY_KIB,
        "INT 12h memory size"
    );
}

#[test]
fn equipment_and_memory_dx50() {
    check_equipment_and_memory(create_machine_dx50());
}

#[test]
fn equipment_and_memory_dx66() {
    check_equipment_and_memory(create_machine_dx66());
}

#[test]
fn int11h_rereads_the_bda() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);

    write_bytes(&mut machine, BDA_EQUIPMENT, &[0xCD, 0xAB]);
    inject_and_run(&mut machine, EQUIPMENT_AND_MEMORY_CODE, &[], 1_000_000);

    assert_eq!(
        read_ram_u16(&machine, RESULT),
        0xABCD,
        "INT 11h must return the live BDA equipment word"
    );
}
