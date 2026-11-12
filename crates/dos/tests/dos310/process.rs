use crate::harness::*;

/// Path of the test program.
const PROGRAM_PATH: &[u8] = b"A:\\TEST.COM\0";
/// Offset of the EXEC parameter block in the inject segment.
const PARAMETER_BLOCK_OFFSET: u32 = 0x0210;
/// Offset of the empty command tail in the inject segment.
const COMMAND_TAIL_OFFSET: u32 = 0x0220;
/// Carriage return that ends a command tail.
const COMMAND_TAIL_END: u8 = 0x0D;
/// Carry flag in the flags word.
const CARRY_FLAG: u16 = 0x0001;
/// `INT 21h AH=4Dh` result of a normal exit with code 42h.
const NORMAL_EXIT_WITH_CODE_42H: u16 = 0x0042;

#[test]
fn exec_runs_a_program_and_returns_its_exit_code() {
    let mut machine = boot_with_floppy(create_floppy(&standard_files()));
    write_bytes(&mut machine.bus, INJECT_DATA_BASE, PROGRAM_PATH);
    write_bytes(
        &mut machine.bus,
        INJECT_CODE_BASE + COMMAND_TAIL_OFFSET,
        &[0x00, COMMAND_TAIL_END],
    );
    let [segment_low, segment_high] = INJECT_CODE_SEGMENT.to_le_bytes();
    #[rustfmt::skip]
    let parameter_block: [u8; 14] = [
        0x00, 0x00,                             // inherit the environment
        0x20, 0x02, segment_low, segment_high,  // command tail
        0x30, 0x02, segment_low, segment_high,  // first FCB
        0x40, 0x02, segment_low, segment_high,  // second FCB
    ];
    write_bytes(
        &mut machine.bus,
        INJECT_CODE_BASE + PARAMETER_BLOCK_OFFSET,
        &parameter_block,
    );
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,                 // MOV DX, 0200h
        0xBB, 0x10, 0x02,                 // MOV BX, 0210h
        0xB8, 0x00, 0x4B,                 // MOV AX, 4B00h
        0xCD, 0x21,                       // INT 21h
        0x9C,                             // PUSHF
        0xB8, segment_low, segment_high,  // MOV AX, inject segment
        0x8E, 0xD8,                       // MOV DS, AX
        0x8E, 0xC0,                       // MOV ES, AX
        0x58,                             // POP AX
        0xA3, 0x00, 0x01,                 // MOV [0100h], AX
        0xB4, 0x4D,                       // MOV AH, 4Dh
        0xCD, 0x21,                       // INT 21h
        0xA3, 0x02, 0x01,                 // MOV [0102h], AX
        0xFA,                             // CLI
        0xF4,                             // HLT
    ];
    inject_and_run_with_budget(&mut machine, code, INJECT_BUDGET_DISK_IO);
    assert_eq!(result_word(&machine.bus, 0) & CARRY_FLAG, 0, "EXEC failed");
    assert_eq!(result_word(&machine.bus, 2), NORMAL_EXIT_WITH_CODE_42H);
}
