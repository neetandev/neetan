//! INT 76h (IRQ 14) hard disk completion flag for direct hardware users.

use super::{create_machine_dx50, inject_and_run, make_pattern_hdd, read_ram_u8};

/// BIOS data area: hard disk operation-complete interrupt flag.
const BDA_HDD_INTERRUPT_FLAG: u32 = 0x48E;
/// Cycle budget for the polled IDE command round trip.
const IDE_IRQ_BUDGET: u64 = 40_000_000;

/// Unmasks IRQ 14, issues an IDE READ SECTORS command for CHS 0/0/1 through
/// the task file with interrupts enabled and polls BDA 40:8E until the
/// completion flag appears, then stores the byte.
#[rustfmt::skip]
const IDE_READ_POLL_CODE: &[u8] = &[
    0xFB,                   // STI
    0xE4, 0xA1,             // IN AL, 0xA1
    0x24, 0xBF,             // AND AL, 0xBF (unmask IRQ 14)
    0xE6, 0xA1,             // OUT 0xA1, AL
    0xBA, 0xF6, 0x01,       // MOV DX, 0x1F6
    0xB0, 0xA0,             // MOV AL, 0xA0 (drive 0, head 0, CHS)
    0xEE,                   // OUT DX, AL
    0xBA, 0xF2, 0x01,       // MOV DX, 0x1F2
    0xB0, 0x01,             // MOV AL, 1 (one sector)
    0xEE,                   // OUT DX, AL
    0xBA, 0xF3, 0x01,       // MOV DX, 0x1F3
    0xB0, 0x01,             // MOV AL, 1 (sector 1)
    0xEE,                   // OUT DX, AL
    0xBA, 0xF4, 0x01,       // MOV DX, 0x1F4
    0xB0, 0x00,             // MOV AL, 0 (cylinder low)
    0xEE,                   // OUT DX, AL
    0xBA, 0xF5, 0x01,       // MOV DX, 0x1F5
    0xB0, 0x00,             // MOV AL, 0 (cylinder high)
    0xEE,                   // OUT DX, AL
    0xBA, 0xF7, 0x01,       // MOV DX, 0x1F7
    0xB0, 0x20,             // MOV AL, 0x20 (READ SECTORS)
    0xEE,                   // OUT DX, AL
    0xA0, 0x8E, 0x04,       // MOV AL, [0x048E]
    0x84, 0xC0,             // TEST AL, AL
    0x74, 0xF9,             // JZ back to the poll
    0xA2, 0x00, 0x06,       // MOV [0x0600], AL
    0xF4,                   // HLT
];

#[test]
fn irq14_sets_completion_flag_and_eois() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_hdd(0, make_pattern_hdd(1), None)
        .expect("insert pattern hard disk");
    boot_to_halt!(machine);
    inject_and_run(&mut machine, IDE_READ_POLL_CODE, &[], IDE_IRQ_BUDGET);

    assert_eq!(
        read_ram_u8(&machine, super::RESULT),
        0xFF,
        "completion flag observed by the poller"
    );
    assert_eq!(
        read_ram_u8(&machine, BDA_HDD_INTERRUPT_FLAG),
        0xFF,
        "completion flag set in the BDA"
    );
    let state = machine.inspection_state();
    assert_eq!(
        state.pic.chips[1].isr & 0x40,
        0,
        "IRQ 14 in-service bit cleared by the stub EOI"
    );
    assert_eq!(
        state.pic.chips[0].isr & 0x04,
        0,
        "cascade in-service bit cleared by the stub EOI"
    );
}
