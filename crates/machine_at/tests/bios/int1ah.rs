//! INT 1Ah time services against the fixed harness clock
//! (2026-07-12 12:00:00, 24-hour BCD RTC).

use super::{
    RESULT, boot_and_run, create_machine_dx50, inject_and_run, read_ram_u8, read_ram_u16,
    read_ram_u32, write_bytes,
};

/// BIOS data area: timer tick count (dword).
const BDA_TIMER_COUNT: u32 = 0x46C;
/// BIOS data area: timer 24-hour rollover flag.
const BDA_TIMER_OVERFLOW: u32 = 0x470;
/// FLAGS bit 0: carry.
const FLAGS_CARRY: u16 = 0x0001;

/// AH=00h: stores CX, DX, AL and the returned FLAGS.
#[rustfmt::skip]
const READ_TICK_COUNT_CODE: &[u8] = &[
    0xB4, 0x00,             // MOV AH, 0x00
    0xCD, 0x1A,             // INT 1Ah
    0x89, 0x0E, 0x00, 0x06, // MOV [0x0600], CX
    0x89, 0x16, 0x02, 0x06, // MOV [0x0602], DX
    0xA2, 0x04, 0x06,       // MOV [0x0604], AL
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x06, 0x06,       // MOV [0x0606], AX
    0xF4,                   // HLT
];

/// AH=01h: sets the tick count to 0x1234:0x5678.
#[rustfmt::skip]
const SET_TICK_COUNT_CODE: &[u8] = &[
    0xB9, 0x34, 0x12,       // MOV CX, 0x1234
    0xBA, 0x78, 0x56,       // MOV DX, 0x5678
    0xB4, 0x01,             // MOV AH, 0x01
    0xCD, 0x1A,             // INT 1Ah
    0xF4,                   // HLT
];

/// AH=02h: stores CX, DX and the returned FLAGS.
#[rustfmt::skip]
const READ_RTC_TIME_CODE: &[u8] = &[
    0xB4, 0x02,             // MOV AH, 0x02
    0xCD, 0x1A,             // INT 1Ah
    0x89, 0x0E, 0x00, 0x06, // MOV [0x0600], CX
    0x89, 0x16, 0x02, 0x06, // MOV [0x0602], DX
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x04, 0x06,       // MOV [0x0604], AX
    0xF4,                   // HLT
];

/// AH=03h with 23:59:58, then AH=02h storing the read-back.
#[rustfmt::skip]
const SET_THEN_READ_RTC_TIME_CODE: &[u8] = &[
    0xB9, 0x59, 0x23,       // MOV CX, 0x2359
    0xBA, 0x00, 0x58,       // MOV DX, 0x5800
    0xB4, 0x03,             // MOV AH, 0x03
    0xCD, 0x1A,             // INT 1Ah
    0xB4, 0x02,             // MOV AH, 0x02
    0xCD, 0x1A,             // INT 1Ah
    0x89, 0x0E, 0x00, 0x06, // MOV [0x0600], CX
    0x89, 0x16, 0x02, 0x06, // MOV [0x0602], DX
    0xF4,                   // HLT
];

/// AH=04h: stores CX, DX and the returned FLAGS.
#[rustfmt::skip]
const READ_RTC_DATE_CODE: &[u8] = &[
    0xB4, 0x04,             // MOV AH, 0x04
    0xCD, 0x1A,             // INT 1Ah
    0x89, 0x0E, 0x00, 0x06, // MOV [0x0600], CX
    0x89, 0x16, 0x02, 0x06, // MOV [0x0602], DX
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x04, 0x06,       // MOV [0x0604], AX
    0xF4,                   // HLT
];

/// AH=05h with 1999-12-31, then AH=04h storing the read-back.
#[rustfmt::skip]
const SET_THEN_READ_RTC_DATE_CODE: &[u8] = &[
    0xB9, 0x99, 0x19,       // MOV CX, 0x1999
    0xBA, 0x31, 0x12,       // MOV DX, 0x1231
    0xB4, 0x05,             // MOV AH, 0x05
    0xCD, 0x1A,             // INT 1Ah
    0xB4, 0x04,             // MOV AH, 0x04
    0xCD, 0x1A,             // INT 1Ah
    0x89, 0x0E, 0x00, 0x06, // MOV [0x0600], CX
    0x89, 0x16, 0x02, 0x06, // MOV [0x0602], DX
    0xF4,                   // HLT
];

/// AH=B1h (PCI BIOS, not implemented): stores the returned FLAGS.
#[rustfmt::skip]
const UNSUPPORTED_FUNCTION_CODE: &[u8] = &[
    0xB4, 0xB1,             // MOV AH, 0xB1
    0xCD, 0x1A,             // INT 1Ah
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xF4,                   // HLT
];

#[test]
fn read_tick_count_clears_the_midnight_flag() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);

    write_bytes(&mut machine, BDA_TIMER_COUNT, &[0x59, 0x00, 0x0C, 0x00]);
    write_bytes(&mut machine, BDA_TIMER_OVERFLOW, &[0x01]);
    inject_and_run(&mut machine, READ_TICK_COUNT_CODE, &[], 1_000_000);

    assert_eq!(read_ram_u16(&machine, RESULT), 0x000C, "CX high word");
    assert_eq!(read_ram_u16(&machine, RESULT + 2), 0x0059, "DX low word");
    assert_eq!(read_ram_u8(&machine, RESULT + 4), 1, "AL midnight flag");
    assert_eq!(
        read_ram_u16(&machine, RESULT + 6) & FLAGS_CARRY,
        0,
        "carry clear"
    );
    assert_eq!(
        read_ram_u8(&machine, BDA_TIMER_OVERFLOW),
        0,
        "midnight flag cleared by the read"
    );
}

#[test]
fn set_tick_count_writes_the_bda() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);

    write_bytes(&mut machine, BDA_TIMER_OVERFLOW, &[0x01]);
    inject_and_run(&mut machine, SET_TICK_COUNT_CODE, &[], 1_000_000);

    assert_eq!(read_ram_u32(&machine, BDA_TIMER_COUNT), 0x1234_5678);
    assert_eq!(
        read_ram_u8(&machine, BDA_TIMER_OVERFLOW),
        0,
        "midnight flag cleared by the set"
    );
}

#[test]
fn read_rtc_time_returns_the_seeded_clock() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, READ_RTC_TIME_CODE, &[], 1_000_000);

    // Noon from the fixed 2026-07-12 12:00:00 seed, BCD.
    assert_eq!(
        read_ram_u16(&machine, RESULT),
        0x1200,
        "CH hours, CL minutes"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT + 2),
        0x0000,
        "DH seconds, DL daylight savings"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT + 4) & FLAGS_CARRY,
        0,
        "carry clear"
    );
}

#[test]
fn set_then_read_rtc_time_round_trips() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, SET_THEN_READ_RTC_TIME_CODE, &[], 1_000_000);

    assert_eq!(
        read_ram_u16(&machine, RESULT),
        0x2359,
        "CH hours, CL minutes"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT + 2),
        0x5800,
        "DH seconds, DL daylight savings"
    );
}

#[test]
fn read_rtc_date_returns_the_seeded_clock() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, READ_RTC_DATE_CODE, &[], 1_000_000);

    assert_eq!(
        read_ram_u16(&machine, RESULT),
        0x2026,
        "CH century, CL year"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT + 2),
        0x0712,
        "DH month, DL day"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT + 4) & FLAGS_CARRY,
        0,
        "carry clear"
    );
}

#[test]
fn set_then_read_rtc_date_round_trips() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, SET_THEN_READ_RTC_DATE_CODE, &[], 1_000_000);

    assert_eq!(
        read_ram_u16(&machine, RESULT),
        0x1999,
        "CH century, CL year"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT + 2),
        0x1231,
        "DH month, DL day"
    );
}

#[test]
fn unsupported_function_sets_carry() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, UNSUPPORTED_FUNCTION_CODE, &[], 1_000_000);

    assert_eq!(
        read_ram_u16(&machine, RESULT) & FLAGS_CARRY,
        FLAGS_CARRY,
        "carry set for the unimplemented PCI BIOS functions"
    );
}

/// Slave PIC interrupt mask register port.
const SLAVE_IMR_PORT: u16 = 0xA1;

/// Installs IVT vector 4Ah = 0000:2000, arms the alarm for 12:00:02 through
/// AH=06h, stores the returned FLAGS, then idles on timer wakeups.
#[rustfmt::skip]
const ARM_ALARM_AND_IDLE_CODE: &[u8] = &[
    0xC7, 0x06, 0x28, 0x01, 0x00, 0x20, // MOV WORD [0x0128], 0x2000
    0xC7, 0x06, 0x2A, 0x01, 0x00, 0x00, // MOV WORD [0x012A], 0x0000
    0xB5, 0x12,                         // MOV CH, 0x12 (hours, BCD)
    0xB1, 0x00,                         // MOV CL, 0x00 (minutes)
    0xB6, 0x02,                         // MOV DH, 0x02 (seconds)
    0xB4, 0x06,                         // MOV AH, 0x06
    0xCD, 0x1A,                         // INT 1Ah
    0x9C, 0x58,                         // PUSHF; POP AX
    0xA3, 0x04, 0x06,                   // MOV [0x0604], AX
    0xFB,                               // STI
    0xF4,                               // HLT
    0xEB, 0xFD,                         // JMP short back to the HLT
];

/// Arms the alarm twice back to back, storing both returned FLAGS.
#[rustfmt::skip]
const DOUBLE_ARM_ALARM_CODE: &[u8] = &[
    0xB5, 0x12,                         // MOV CH, 0x12 (hours, BCD)
    0xB1, 0x00,                         // MOV CL, 0x00 (minutes)
    0xB6, 0x10,                         // MOV DH, 0x10 (seconds)
    0xB4, 0x06,                         // MOV AH, 0x06
    0xCD, 0x1A,                         // INT 1Ah
    0x9C, 0x58,                         // PUSHF; POP AX
    0xA3, 0x00, 0x06,                   // MOV [0x0600], AX
    0xB6, 0x11,                         // MOV DH, 0x11 (seconds)
    0xB4, 0x06,                         // MOV AH, 0x06
    0xCD, 0x1A,                         // INT 1Ah (alarm already armed)
    0x9C, 0x58,                         // PUSHF; POP AX
    0xA3, 0x02, 0x06,                   // MOV [0x0602], AX
    0xF4,                               // HLT
];

/// Installs the INT 4Ah hook, arms the alarm for 12:00:02, cancels it with
/// AH=07h, then idles past the alarm time.
#[rustfmt::skip]
const CANCEL_ALARM_AND_IDLE_CODE: &[u8] = &[
    0xC7, 0x06, 0x28, 0x01, 0x00, 0x20, // MOV WORD [0x0128], 0x2000
    0xC7, 0x06, 0x2A, 0x01, 0x00, 0x00, // MOV WORD [0x012A], 0x0000
    0xB5, 0x12,                         // MOV CH, 0x12 (hours, BCD)
    0xB1, 0x00,                         // MOV CL, 0x00 (minutes)
    0xB6, 0x02,                         // MOV DH, 0x02 (seconds)
    0xB4, 0x06,                         // MOV AH, 0x06
    0xCD, 0x1A,                         // INT 1Ah
    0xB4, 0x07,                         // MOV AH, 0x07
    0xCD, 0x1A,                         // INT 1Ah (cancel)
    0xFB,                               // STI
    0xF4,                               // HLT
    0xEB, 0xFD,                         // JMP short back to the HLT
];

/// INT 4Ah callback: increments the counter at the result address.
#[rustfmt::skip]
const COUNT_ALARMS_CALLBACK: &[u8] = &[
    0xFE, 0x06, 0x00, 0x06, // INC BYTE [0x0600]
    0xCF,                   // IRET
];

#[test]
fn alarm_fires_the_int_4ah_hook_once() {
    use common::Bus;
    let mut machine = create_machine_dx50();
    // The harness clock starts at 12:00:00; four emulated seconds pass the
    // 12:00:02 alarm with margin.
    boot_and_run(
        &mut machine,
        ARM_ALARM_AND_IDLE_CODE,
        COUNT_ALARMS_CALLBACK,
        200_000_000,
    );

    assert_eq!(
        read_ram_u16(&machine, RESULT + 4) & FLAGS_CARRY,
        0,
        "arming reports success"
    );
    assert_eq!(
        read_ram_u8(&machine, RESULT),
        1,
        "the hook ran exactly once"
    );
    assert_eq!(
        machine.bus.io_read_byte(SLAVE_IMR_PORT) & 0x01,
        0,
        "IRQ 8 unmasked while the alarm is armed"
    );
}

#[test]
fn second_alarm_while_armed_fails() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, DOUBLE_ARM_ALARM_CODE, &[], 1_000_000);

    assert_eq!(
        read_ram_u16(&machine, RESULT) & FLAGS_CARRY,
        0,
        "first arm succeeds"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT + 2) & FLAGS_CARRY,
        FLAGS_CARRY,
        "second arm fails while armed"
    );
}

#[test]
fn cancelled_alarm_does_not_fire() {
    use common::Bus;
    let mut machine = create_machine_dx50();
    boot_and_run(
        &mut machine,
        CANCEL_ALARM_AND_IDLE_CODE,
        COUNT_ALARMS_CALLBACK,
        200_000_000,
    );

    assert_eq!(read_ram_u8(&machine, RESULT), 0, "the hook never ran");
    assert_eq!(
        machine.bus.io_read_byte(SLAVE_IMR_PORT) & 0x01,
        0x01,
        "IRQ 8 masked again after the cancel"
    );
}
