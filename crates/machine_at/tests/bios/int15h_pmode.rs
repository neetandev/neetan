//! INT 15h AH=87h extended memory move with a guest-built descriptor table,
//! exercised with the A20 gate disabled and enabled at call time.

use common::Bus;

use super::{
    RESULT, boot_and_run, create_machine_dx50, inject_and_run, read_ram_u8, read_ram_u16,
    write_bytes,
};

/// FLAGS bit 0: carry.
const FLAGS_CARRY: u16 = 0x0001;
/// FLAGS bit 6: zero.
const FLAGS_ZERO: u16 = 0x0040;
/// High memory target of the first guest-built move.
const HIGH_TARGET_FIRST: u32 = 0x0010_1000;
/// High memory target of the second guest-built move.
const HIGH_TARGET_SECOND: u32 = 0x0010_2000;
/// High memory source of the A20-off read test.
const HIGH_SOURCE: u32 = 0x0010_1800;
/// One megabyte wrap alias of `HIGH_SOURCE` when A20 is disabled.
const HIGH_SOURCE_ALIAS: u32 = 0x1800;

/// Builds the 48-byte descriptor table at 0x3000 in guest code (source base
/// 0x3100, target base 0x101000), calls AH=87h with A20 still disabled from
/// POST, and stores AX, FLAGS and port 0x92.
#[rustfmt::skip]
const GUEST_TABLE_MOVE_CODE: &[u8] = &[
    0x31, 0xC0,                         // XOR AX, AX
    0x8E, 0xC0,                         // MOV ES, AX
    0xBF, 0x00, 0x30,                   // MOV DI, 0x3000
    0xB9, 0x18, 0x00,                   // MOV CX, 24
    0xF3, 0xAB,                         // REP STOSW (zero the table)
    0xC7, 0x06, 0x10, 0x30, 0xFF, 0xFF, // MOV WORD [0x3010], 0xFFFF (source limit)
    0xC7, 0x06, 0x12, 0x30, 0x00, 0x31, // MOV WORD [0x3012], 0x3100 (source base 15:0)
    0xC7, 0x06, 0x14, 0x30, 0x00, 0x93, // MOV WORD [0x3014], access 93h, base 23:16
    0xC7, 0x06, 0x18, 0x30, 0xFF, 0xFF, // MOV WORD [0x3018], 0xFFFF (target limit)
    0xC7, 0x06, 0x1A, 0x30, 0x00, 0x10, // MOV WORD [0x301A], 0x1000 (target base 15:0)
    0xC7, 0x06, 0x1C, 0x30, 0x10, 0x93, // MOV WORD [0x301C], access 93h, base 23:16 = 10h
    0xBE, 0x00, 0x30,                   // MOV SI, 0x3000
    0xB9, 0x10, 0x00,                   // MOV CX, 16 (words)
    0xB4, 0x87,                         // MOV AH, 0x87
    0xCD, 0x15,                         // INT 15h
    0xA3, 0x00, 0x06,                   // MOV [0x0600], AX
    0x9C, 0x58,                         // PUSHF; POP AX
    0xA3, 0x02, 0x06,                   // MOV [0x0602], AX
    0xE4, 0x92,                         // IN AL, 0x92
    0xA2, 0x04, 0x06,                   // MOV [0x0604], AL (restored gate)
    0xF4,                               // HLT
];

/// Enables A20 through INT 15h AH=24h, patches the guest-built table
/// (source base 0x3300, target base 0x102000), repeats the move and stores
/// AX, FLAGS and port 0x92.
#[rustfmt::skip]
const GUEST_TABLE_MOVE_A20_ON_CODE: &[u8] = &[
    0xB8, 0x01, 0x24,                   // MOV AX, 0x2401 (A20 enable)
    0xCD, 0x15,                         // INT 15h
    0xC7, 0x06, 0x12, 0x30, 0x00, 0x33, // MOV WORD [0x3012], 0x3300 (source base 15:0)
    0xC7, 0x06, 0x1A, 0x30, 0x00, 0x20, // MOV WORD [0x301A], 0x2000 (target base 15:0)
    0x31, 0xC0,                         // XOR AX, AX
    0x8E, 0xC0,                         // MOV ES, AX
    0xBE, 0x00, 0x30,                   // MOV SI, 0x3000
    0xB9, 0x10, 0x00,                   // MOV CX, 16 (words)
    0xB4, 0x87,                         // MOV AH, 0x87
    0xCD, 0x15,                         // INT 15h
    0xA3, 0x00, 0x06,                   // MOV [0x0600], AX
    0x9C, 0x58,                         // PUSHF; POP AX
    0xA3, 0x02, 0x06,                   // MOV [0x0602], AX
    0xE4, 0x92,                         // IN AL, 0x92
    0xA2, 0x04, 0x06,                   // MOV [0x0604], AL (restored gate)
    0xF4,                               // HLT
];

/// Calls AH=87h through the table at 0x3000 and stores AX and FLAGS.
#[rustfmt::skip]
const HOST_TABLE_MOVE_CODE: &[u8] = &[
    0x31, 0xC0,                         // XOR AX, AX
    0x8E, 0xC0,                         // MOV ES, AX
    0xBE, 0x00, 0x30,                   // MOV SI, 0x3000
    0xB9, 0x10, 0x00,                   // MOV CX, 16 (words)
    0xB4, 0x87,                         // MOV AH, 0x87
    0xCD, 0x15,                         // INT 15h
    0xA3, 0x00, 0x06,                   // MOV [0x0600], AX
    0x9C, 0x58,                         // PUSHF; POP AX
    0xA3, 0x02, 0x06,                   // MOV [0x0602], AX
    0xF4,                               // HLT
];

/// Builds an AH=87h descriptor table copying between two linear addresses.
fn move_descriptor_table(source: u32, target: u32) -> [u8; 48] {
    let mut table = [0u8; 48];
    for (offset, base) in [(0x10usize, source), (0x18, target)] {
        table[offset] = 0xFF;
        table[offset + 1] = 0xFF;
        table[offset + 2] = base as u8;
        table[offset + 3] = (base >> 8) as u8;
        table[offset + 4] = (base >> 16) as u8;
        table[offset + 5] = 0x93;
        table[offset + 7] = (base >> 24) as u8;
    }
    table
}

/// Asserts the stored AH=87h success contract: AX=0, CF clear, ZF set.
fn assert_move_succeeded(machine: &machine_at::AtMachine<common::NoTrace>) {
    assert_eq!(read_ram_u16(machine, RESULT), 0x0000, "AX");
    let flags = read_ram_u16(machine, RESULT + 2);
    assert_eq!(flags & FLAGS_CARRY, 0, "carry clear");
    assert_eq!(flags & FLAGS_ZERO, FLAGS_ZERO, "zero set");
}

#[test]
fn guest_built_table_moves_with_a20_disabled_and_enabled() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);

    // A20 is disabled after POST; the handler must enable it for the copy
    // and restore the disabled state afterwards.
    let first_pattern: Vec<u8> = (0..32u32).map(|index| (index as u8) * 5 + 3).collect();
    write_bytes(&mut machine, 0x3100, &first_pattern);
    inject_and_run(&mut machine, GUEST_TABLE_MOVE_CODE, &[], 4_000_000);
    assert_move_succeeded(&machine);
    assert_eq!(
        read_ram_u8(&machine, RESULT + 4) & 0x02,
        0x00,
        "A20 restored disabled"
    );
    for (index, &expected) in first_pattern.iter().enumerate() {
        assert_eq!(
            read_ram_u8(&machine, HIGH_TARGET_FIRST + index as u32),
            expected,
            "high memory byte {index} with A20 off at call time"
        );
    }

    // Repeat with the gate enabled at call time; it must stay enabled.
    let second_pattern: Vec<u8> = (0..32u32).map(|index| (index as u8) ^ 0xC5).collect();
    write_bytes(&mut machine, 0x3300, &second_pattern);
    inject_and_run(&mut machine, GUEST_TABLE_MOVE_A20_ON_CODE, &[], 4_000_000);
    assert_move_succeeded(&machine);
    assert_eq!(
        read_ram_u8(&machine, RESULT + 4) & 0x02,
        0x02,
        "A20 restored enabled"
    );
    for (index, &expected) in second_pattern.iter().enumerate() {
        assert_eq!(
            read_ram_u8(&machine, HIGH_TARGET_SECOND + index as u32),
            expected,
            "high memory byte {index} with A20 on at call time"
        );
    }
}

#[test]
fn move_reads_high_memory_with_a20_off() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);

    // Seed distinct patterns above 1 MiB and at the wrap alias, with the
    // gate enabled around the high write so it lands unwrapped.
    let high_pattern: Vec<u8> = (0..32u32).map(|index| (index as u8) + 0x40).collect();
    let alias_pattern: Vec<u8> = (0..32u32).map(|index| (index as u8) + 0xA0).collect();
    machine.bus.io_write_byte(0x92, 0x02);
    write_bytes(&mut machine, HIGH_SOURCE, &high_pattern);
    machine.bus.io_write_byte(0x92, 0x00);
    write_bytes(&mut machine, HIGH_SOURCE_ALIAS, &alias_pattern);

    let table = move_descriptor_table(HIGH_SOURCE, 0x3200);
    write_bytes(&mut machine, 0x3000, &table);
    inject_and_run(&mut machine, HOST_TABLE_MOVE_CODE, &[], 4_000_000);
    assert_move_succeeded(&machine);
    for (index, &expected) in high_pattern.iter().enumerate() {
        assert_eq!(
            read_ram_u8(&machine, 0x3200 + index as u32),
            expected,
            "byte {index} must come from high memory, not the wrap alias"
        );
    }
}
