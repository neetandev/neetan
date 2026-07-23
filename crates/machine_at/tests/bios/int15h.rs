//! INT 15h system services: extended memory size, the ROM configuration
//! table and the A20 gate.

use common::Bus;

use super::{
    RESULT, boot_and_run, create_machine_dx50, inject_and_run, read_ram_u8, read_ram_u16,
    write_bytes,
};

/// FLAGS bit 0: carry.
const FLAGS_CARRY: u16 = 0x0001;
/// Physical base of the system BIOS stub ROM.
const SYSTEM_BIOS_BASE: u32 = 0xF_0000;
/// Extended memory KiB of the 16 MiB AT models, capped at 15 MiB.
const EXTENDED_MEMORY_KIB: u16 = 0x3C00;
/// CMOS register: extended memory KiB, AMI mirror low byte.
const CMOS_EXTENDED_MEMORY_LOW: usize = 0x30;
/// CMOS register: extended memory KiB, AMI mirror high byte.
const CMOS_EXTENDED_MEMORY_HIGH: usize = 0x31;

/// AH=88h: stores AX and the returned FLAGS.
#[rustfmt::skip]
const EXTENDED_MEMORY_CODE: &[u8] = &[
    0xB4, 0x88,             // MOV AH, 0x88
    0xCD, 0x15,             // INT 15h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x02, 0x06,       // MOV [0x0602], AX
    0xF4,                   // HLT
];

/// AH=C0h: stores BX, ES and the returned FLAGS.
#[rustfmt::skip]
const CONFIGURATION_TABLE_CODE: &[u8] = &[
    0xB4, 0xC0,             // MOV AH, 0xC0
    0xCD, 0x15,             // INT 15h
    0x89, 0x1E, 0x00, 0x06, // MOV [0x0600], BX
    0x8C, 0xC0,             // MOV AX, ES
    0xA3, 0x02, 0x06,       // MOV [0x0602], AX
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x04, 0x06,       // MOV [0x0604], AX
    0xF4,                   // HLT
];

/// AH=24h walk: status, enable, write above 1 MiB, status, disable, wrapped
/// write, status, enable, read back, support bitmap.
#[rustfmt::skip]
const A20_GATE_CODE: &[u8] = &[
    0xB8, 0x02, 0x24,                   // MOV AX, 0x2402 (status)
    0xCD, 0x15,                         // INT 15h
    0xA2, 0x00, 0x06,                   // MOV [0x0600], AL (expect 0 after POST)
    0xB8, 0x01, 0x24,                   // MOV AX, 0x2401 (enable)
    0xCD, 0x15,                         // INT 15h
    0xB8, 0xFF, 0xFF,                   // MOV AX, 0xFFFF
    0x8E, 0xC0,                         // MOV ES, AX
    0x26, 0xC6, 0x06, 0x10, 0x05, 0x55, // MOV BYTE ES:[0x0510], 0x55 (linear 0x100500)
    0xB8, 0x02, 0x24,                   // MOV AX, 0x2402 (status)
    0xCD, 0x15,                         // INT 15h
    0xA2, 0x01, 0x06,                   // MOV [0x0601], AL (expect 1)
    0xB8, 0x00, 0x24,                   // MOV AX, 0x2400 (disable)
    0xCD, 0x15,                         // INT 15h
    0x26, 0xC6, 0x06, 0x10, 0x05, 0xAA, // MOV BYTE ES:[0x0510], 0xAA (wraps to 0x500)
    0xB8, 0x02, 0x24,                   // MOV AX, 0x2402 (status)
    0xCD, 0x15,                         // INT 15h
    0xA2, 0x02, 0x06,                   // MOV [0x0602], AL (expect 0)
    0xB8, 0x01, 0x24,                   // MOV AX, 0x2401 (enable)
    0xCD, 0x15,                         // INT 15h
    0x26, 0xA0, 0x10, 0x05,             // MOV AL, ES:[0x0510]
    0xA2, 0x03, 0x06,                   // MOV [0x0603], AL (expect 0x55)
    0xB8, 0x03, 0x24,                   // MOV AX, 0x2403 (support)
    0xCD, 0x15,                         // INT 15h
    0x89, 0x1E, 0x04, 0x06,             // MOV [0x0604], BX (expect 0x0003)
    0xF4,                               // HLT
];

/// AH=89h (switch to protected mode, unsupported): stores AH and FLAGS.
#[rustfmt::skip]
const UNSUPPORTED_FUNCTION_CODE: &[u8] = &[
    0xB4, 0x89,             // MOV AH, 0x89
    0xCD, 0x15,             // INT 15h
    0x88, 0x26, 0x00, 0x06, // MOV [0x0600], AH
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x02, 0x06,       // MOV [0x0602], AX
    0xF4,                   // HLT
];

#[test]
fn extended_memory_size_matches_cmos() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, EXTENDED_MEMORY_CODE, &[], 1_000_000);

    let cmos_kib = u16::from(machine.bus.cmos_byte(CMOS_EXTENDED_MEMORY_LOW))
        | (u16::from(machine.bus.cmos_byte(CMOS_EXTENDED_MEMORY_HIGH)) << 8);
    assert_eq!(read_ram_u16(&machine, RESULT), cmos_kib, "AX from CMOS");
    assert_eq!(cmos_kib, EXTENDED_MEMORY_KIB, "16 MiB models report 15 MiB");
    assert_eq!(
        read_ram_u16(&machine, RESULT + 2) & FLAGS_CARRY,
        0,
        "carry clear"
    );
}

#[test]
fn configuration_table_is_served_from_the_stub_rom() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, CONFIGURATION_TABLE_CODE, &[], 1_000_000);

    let table_offset = read_ram_u16(&machine, RESULT);
    assert_eq!(read_ram_u16(&machine, RESULT + 2), 0xF000, "ES");
    assert_eq!(
        read_ram_u16(&machine, RESULT + 4) & FLAGS_CARRY,
        0,
        "carry clear"
    );

    let table = SYSTEM_BIOS_BASE + u32::from(table_offset);
    assert_eq!(read_ram_u16(&machine, table), 8, "table length");
    assert_eq!(read_ram_u8(&machine, table + 2), 0xFC, "machine model");
    assert_eq!(read_ram_u8(&machine, table + 3), 0x01, "submodel");
    assert_eq!(read_ram_u8(&machine, table + 4), 0x00, "BIOS revision");
    assert_eq!(
        read_ram_u8(&machine, table + 5),
        0x70,
        "feature byte: second 8259, RTC, keyboard intercept"
    );
}

// Known divergence from the real AMI BIOS: it does not implement INT 15h
// AH=24h at all (AH=86h and CF=1 for every sub-function, A20 untouched). The
// HLE BIOS implements the IBM-documented gate control so software with an
// INT 15h A20 path works. HIMEM falls back to the KBC and port 0x92 either
// way.
#[test]
fn a20_gate_toggles_the_wrap() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, A20_GATE_CODE, &[], 2_000_000);

    assert_eq!(read_ram_u8(&machine, RESULT), 0, "A20 disabled after POST");
    assert_eq!(read_ram_u8(&machine, RESULT + 1), 1, "A20 on after enable");
    assert_eq!(
        read_ram_u8(&machine, RESULT + 2),
        0,
        "A20 off after disable"
    );
    assert_eq!(
        read_ram_u8(&machine, RESULT + 3),
        0x55,
        "extended memory byte intact after the gate cycle"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT + 4),
        0x0003,
        "support bitmap: KBC and port 0x92"
    );
    assert_eq!(
        read_ram_u8(&machine, 0x500),
        0xAA,
        "wrapped write landed at physical 0x500"
    );
    assert_eq!(
        read_ram_u8(&machine, 0x10_0500),
        0x55,
        "unwrapped write landed above 1 MiB"
    );
}

#[test]
fn unsupported_returns_ah_86_carry() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, UNSUPPORTED_FUNCTION_CODE, &[], 1_000_000);

    assert_eq!(read_ram_u8(&machine, RESULT), 0x86, "AH error code");
    assert_eq!(
        read_ram_u16(&machine, RESULT + 2) & FLAGS_CARRY,
        FLAGS_CARRY,
        "carry set"
    );
}

/// AH=86h wait for 100,000 microseconds: stores the returned FLAGS.
#[rustfmt::skip]
const WAIT_100MS_CODE: &[u8] = &[
    0xB9, 0x01, 0x00,       // MOV CX, 0x0001
    0xBA, 0xA0, 0x86,       // MOV DX, 0x86A0 (CX:DX = 100,000 us)
    0xB4, 0x86,             // MOV AH, 0x86
    0xCD, 0x15,             // INT 15h
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xF4,                   // HLT
];

/// AH=83h AL=00h arming a 50 ms interval on the flag byte at 0000:0700,
/// then idling so the RTC periodic interrupt runs it down.
#[rustfmt::skip]
const EVENT_WAIT_CODE: &[u8] = &[
    0xB8, 0x00, 0x00,       // MOV AX, 0
    0x8E, 0xC0,             // MOV ES, AX
    0xBB, 0x00, 0x07,       // MOV BX, 0x0700
    0xB9, 0x00, 0x00,       // MOV CX, 0
    0xBA, 0x50, 0xC3,       // MOV DX, 0xC350 (50,000 us)
    0xB8, 0x00, 0x83,       // MOV AX, 0x8300
    0xCD, 0x15,             // INT 15h
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xFB,                   // STI
    0xF4,                   // HLT
    0xEB, 0xFD,             // JMP to the HLT
];

/// Arms AH=83h twice back to back, storing both returned FLAGS.
#[rustfmt::skip]
const EVENT_WAIT_DOUBLE_ARM_CODE: &[u8] = &[
    0xB8, 0x00, 0x00,       // MOV AX, 0
    0x8E, 0xC0,             // MOV ES, AX
    0xBB, 0x00, 0x07,       // MOV BX, 0x0700
    0xB9, 0x00, 0x00,       // MOV CX, 0
    0xBA, 0x50, 0xC3,       // MOV DX, 0xC350 (50,000 us)
    0xB8, 0x00, 0x83,       // MOV AX, 0x8300
    0xCD, 0x15,             // INT 15h
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xB8, 0x00, 0x83,       // MOV AX, 0x8300
    0xCD, 0x15,             // INT 15h (interval already active)
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x02, 0x06,       // MOV [0x0602], AX
    0xF4,                   // HLT
];

/// Arms AH=83h, cancels it with AL=01h, then idles past the interval.
#[rustfmt::skip]
const EVENT_WAIT_CANCEL_CODE: &[u8] = &[
    0xB8, 0x00, 0x00,       // MOV AX, 0
    0x8E, 0xC0,             // MOV ES, AX
    0xBB, 0x00, 0x07,       // MOV BX, 0x0700
    0xB9, 0x00, 0x00,       // MOV CX, 0
    0xBA, 0x50, 0xC3,       // MOV DX, 0xC350 (50,000 us)
    0xB8, 0x00, 0x83,       // MOV AX, 0x8300
    0xCD, 0x15,             // INT 15h
    0xB8, 0x01, 0x83,       // MOV AX, 0x8301 (cancel)
    0xCD, 0x15,             // INT 15h
    0xFB,                   // STI
    0xF4,                   // HLT
    0xEB, 0xFD,             // JMP to the HLT
];

/// AH=49h/C1h/C2h (unsupported) and AH=90h/91h (default hooks): stores AH
/// and FLAGS per call.
#[rustfmt::skip]
const SIMPLE_ARMS_CODE: &[u8] = &[
    0xB4, 0x49,             // MOV AH, 0x49 (DBCS BIOS check)
    0xCD, 0x15,             // INT 15h
    0x88, 0x26, 0x00, 0x06, // MOV [0x0600], AH
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x02, 0x06,       // MOV [0x0602], AX
    0xB4, 0xC1,             // MOV AH, 0xC1 (EBDA segment)
    0xCD, 0x15,             // INT 15h
    0x88, 0x26, 0x04, 0x06, // MOV [0x0604], AH
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x06, 0x06,       // MOV [0x0606], AX
    0xB4, 0xC2,             // MOV AH, 0xC2 (PS/2 mouse)
    0xCD, 0x15,             // INT 15h
    0x88, 0x26, 0x08, 0x06, // MOV [0x0608], AH
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x0A, 0x06,       // MOV [0x060A], AX
    0xB4, 0x90,             // MOV AH, 0x90 (device busy)
    0xCD, 0x15,             // INT 15h
    0x88, 0x26, 0x0C, 0x06, // MOV [0x060C], AH
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x0E, 0x06,       // MOV [0x060E], AX
    0xB4, 0x91,             // MOV AH, 0x91 (interrupt complete)
    0xCD, 0x15,             // INT 15h
    0x88, 0x26, 0x10, 0x06, // MOV [0x0610], AH
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x12, 0x06,       // MOV [0x0612], AX
    0xF4,                   // HLT
];

/// BIOS data area: event wait active flag.
const BDA_WAIT_ACTIVE: u32 = 0x4A0;
/// Event wait user flag byte used by the tests.
const EVENT_WAIT_FLAG: u32 = 0x700;

#[test]
fn wait_86h_duration_within_tolerance() {
    use common::Cpu;
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, WAIT_100MS_CODE, &[], 50_000);

    // 100,000 us at 50 MHz is 5M cycles; the refresh toggle granularity and
    // its 15.086 us period against the counted 15 us add under one percent.
    let mut cycles: u64 = 50_000;
    while !machine.cpu.halted() {
        machine.run_for(50_000);
        cycles += 50_000;
        assert!(cycles < 20_000_000, "wait did not finish");
    }
    assert!(
        (4_500_000..=6_500_000).contains(&cycles),
        "wait consumed {cycles} cycles, expected about 5M"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT) & FLAGS_CARRY,
        0,
        "carry clear"
    );
}

#[test]
fn event_wait_sets_the_user_flag() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, EVENT_WAIT_CODE, &[], 10_000_000);

    assert_eq!(
        read_ram_u16(&machine, RESULT) & FLAGS_CARRY,
        0,
        "arming reports success"
    );
    assert_eq!(
        read_ram_u8(&machine, EVENT_WAIT_FLAG) & 0x80,
        0x80,
        "user flag bit 7 set after the interval"
    );
    assert_eq!(
        read_ram_u8(&machine, BDA_WAIT_ACTIVE),
        0,
        "wait no longer active"
    );
}

#[test]
fn event_wait_rejects_a_second_interval() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, EVENT_WAIT_DOUBLE_ARM_CODE, &[], 1_000_000);

    assert_eq!(
        read_ram_u16(&machine, RESULT) & FLAGS_CARRY,
        0,
        "first arm succeeds"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT + 2) & FLAGS_CARRY,
        FLAGS_CARRY,
        "second arm fails while active"
    );
}

#[test]
fn cancelled_event_wait_never_fires() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, EVENT_WAIT_CANCEL_CODE, &[], 10_000_000);

    assert_eq!(
        read_ram_u8(&machine, EVENT_WAIT_FLAG),
        0,
        "user flag untouched after cancel"
    );
    assert_eq!(read_ram_u8(&machine, BDA_WAIT_ACTIVE), 0, "wait not active");
}

#[test]
fn simple_arms_return_their_contracts() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, SIMPLE_ARMS_CODE, &[], 1_000_000);

    for (index, label) in [(0u32, "AH=49h"), (4, "AH=C1h"), (8, "AH=C2h")] {
        assert_eq!(
            read_ram_u8(&machine, RESULT + index),
            0x86,
            "{label} error code"
        );
        assert_eq!(
            read_ram_u16(&machine, RESULT + index + 2) & FLAGS_CARRY,
            FLAGS_CARRY,
            "{label} carry set"
        );
    }
    for (index, label) in [(12u32, "AH=90h"), (16, "AH=91h")] {
        assert_eq!(read_ram_u8(&machine, RESULT + index), 0x00, "{label} AH");
        assert_eq!(
            read_ram_u16(&machine, RESULT + index + 2) & FLAGS_CARRY,
            0,
            "{label} carry clear"
        );
    }
}

/// FLAGS bit 6: zero.
const FLAGS_ZERO: u16 = 0x0040;
/// Above 1 MiB target address of the extended memory move test.
const HIGH_MEMORY_TARGET: u32 = 0x0010_1000;

/// AH=87h: copies 16 words through the descriptor table at 0300:0000 and
/// stores AX and the returned FLAGS.
#[rustfmt::skip]
const EXTENDED_MEMORY_MOVE_CODE: &[u8] = &[
    0xB8, 0x00, 0x03,       // MOV AX, 0x0300
    0x8E, 0xC0,             // MOV ES, AX
    0xBE, 0x00, 0x00,       // MOV SI, 0x0000
    0xB9, 0x10, 0x00,       // MOV CX, 0x0010
    0xB4, 0x87,             // MOV AH, 0x87
    0xCD, 0x15,             // INT 15h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x02, 0x06,       // MOV [0x0602], AX
    0xF4,                   // HLT
];

/// Builds an AH=87h descriptor table copying between the two linear
/// addresses (source descriptor at +10h, target descriptor at +18h).
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

#[test]
fn extended_memory_move_copies_across_one_megabyte() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);
    let pattern: Vec<u8> = (0..32u32).map(|index| (index as u8) * 3 + 1).collect();
    write_bytes(&mut machine, 0x3100, &pattern);

    // Copy from low memory to above 1 MiB. The A20 gate starts disabled, so
    // the handler must enable it for the copy and restore it afterwards.
    let table = move_descriptor_table(0x3100, HIGH_MEMORY_TARGET);
    write_bytes(&mut machine, 0x3000, &table);
    inject_and_run(&mut machine, EXTENDED_MEMORY_MOVE_CODE, &[], 4_000_000);
    // The real BIOS returns AX=0000 with the carry clear and zero set.
    assert_eq!(read_ram_u16(&machine, RESULT), 0x0000);
    let flags = read_ram_u16(&machine, RESULT + 2);
    assert_eq!(flags & FLAGS_CARRY, 0);
    assert_eq!(flags & FLAGS_ZERO, FLAGS_ZERO);
    for (index, &expected) in pattern.iter().enumerate() {
        assert_eq!(
            read_ram_u8(&machine, HIGH_MEMORY_TARGET + index as u32),
            expected,
            "high memory byte {index}"
        );
    }
    assert_eq!(machine.bus.io_read_byte(0x0092) & 0x02, 0x00);

    // And back down into low memory.
    let table = move_descriptor_table(HIGH_MEMORY_TARGET, 0x3200);
    write_bytes(&mut machine, 0x3000, &table);
    inject_and_run(&mut machine, EXTENDED_MEMORY_MOVE_CODE, &[], 4_000_000);
    for (index, &expected) in pattern.iter().enumerate() {
        assert_eq!(
            read_ram_u8(&machine, 0x3200 + index as u32),
            expected,
            "low memory byte {index}"
        );
    }
}
