use common::{Bus, HostDateTime, Machine as _};

use super::{
    CALLBACK_IRET, TEST_CALLBACK, TEST_CODE, boot_and_run_f, boot_and_run_ra, boot_and_run_vm,
    boot_and_run_vx, create_machine_f, create_machine_ra, create_machine_vm, create_machine_vx,
    read_ivt_vector, write_bytes,
};

const RESULT: u32 = 0x0600;
const MARKER: u32 = 0x0700;
const INT1CH_BUDGET: u64 = 2_000_000;

fn test_local_time() -> HostDateTime {
    HostDateTime {
        year: 2026,
        month: 3,
        day: 3,
        day_of_week: 2,
        hour: 14,
        minute: 30,
        second: 45,
    }
}

const SET_DATE: [u8; 6] = [0x95, 0xA3, 0x15, 0x10, 0x30, 0x45];

fn verify_datetime_buffer(bus: &mut impl Bus, base: u32) {
    let expected = test_local_time().to_bcd_bytes();
    for (i, &exp) in expected.iter().enumerate() {
        assert_eq!(
            bus.read_byte(base + i as u32),
            exp,
            "Calendar byte {i} mismatch (expected {exp:#04X})"
        );
    }
}

// Get date/time: ES:BX = 0000:0600, AH=00h, INT 1Ch.
#[rustfmt::skip]
const GET_DATETIME_CODE: &[u8] = &[
    0x31, 0xC0,       // XOR AX, AX
    0x8E, 0xC0,       // MOV ES, AX
    0xBB, 0x00, 0x06, // MOV BX, 0x0600
    0xB4, 0x00,       // MOV AH, 0x00
    0xCD, 0x1C,       // INT 0x1C
    0xF4,             // HLT
];

// Get date/time through the BIOS HLE trap with a protected-mode ES base that
// differs from selector<<4. This must use the cached segment base, not the
// real-mode formula.
#[rustfmt::skip]
const GET_DATETIME_HLE_TRAP_CODE: &[u8] = &[
    0xBB, 0x00, 0x00, // MOV BX, 0x0000
    0xB4, 0x00,       // MOV AH, 0x00
    0x50,             // PUSH AX
    0x52,             // PUSH DX
    0xBA, 0xF0, 0x07, // MOV DX, 0x07F0
    0xB0, 0x1C,       // MOV AL, 0x1C
    0xEE,             // OUT DX, AL
    0xF4,             // HLT
];

// Set date/time: ES:BX = 0000:0600 (buffer), AH=01h, then write marker.
#[rustfmt::skip]
const SET_DATETIME_CODE: &[u8] = &[
    0x31, 0xC0,                   // XOR AX, AX
    0x8E, 0xC0,                   // MOV ES, AX
    0xBB, 0x00, 0x06,             // MOV BX, 0x0600
    0xB4, 0x01,                   // MOV AH, 0x01
    0xCD, 0x1C,                   // INT 0x1C
    0xC6, 0x06, 0x00, 0x07, 0xAA, // MOV BYTE [0x0700], 0xAA
    0xF4,                         // HLT
];

// Set interval timer: ES:BX = 0000:2000 (callback), CX=5, AH=02h.
#[rustfmt::skip]
const SETUP_TIMER_CODE: &[u8] = &[
    0x31, 0xC0,       // XOR AX, AX
    0x8E, 0xC0,       // MOV ES, AX
    0xBB, 0x00, 0x20, // MOV BX, 0x2000
    0xB9, 0x05, 0x00, // MOV CX, 5
    0xB4, 0x02,       // MOV AH, 0x02
    0xCD, 0x1C,       // INT 0x1C
    0xF4,             // HLT
];

// Set interval timer with CLI, stores CA_TIM_CNT to RESULT.
#[rustfmt::skip]
fn make_setup_counter_code(count: u8) -> Vec<u8> {
    vec![
        0xFA,                   // CLI
        0x31, 0xC0,             // XOR AX, AX
        0x8E, 0xC0,             // MOV ES, AX
        0xBB, 0x00, 0x20,       // MOV BX, 0x2000
        0xB9, count, 0x00,      // MOV CX, count
        0xB4, 0x02,             // MOV AH, 0x02
        0xCD, 0x1C,             // INT 0x1C
        0xA1, 0x8A, 0x05,       // MOV AX, [0x058A]
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0xEB, 0xFE,             // JMP $
    ]
}

// ============================================================================
// §13 INT 1Ch - Vector Setup
// ============================================================================

#[test]
fn int1ch_vector_f() {
    let mut machine = create_machine_f();
    let _cycles = boot_to_halt!(machine);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1C);
    assert!(
        segment >= 0xFD80,
        "INT 1Ch segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int1ch_vector_vm() {
    let mut machine = create_machine_vm();
    let _cycles = boot_to_halt!(machine);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1C);
    assert!(
        segment >= 0xFD80,
        "INT 1Ch segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int1ch_vector_vx() {
    let mut machine = create_machine_vx();
    let _cycles = boot_to_halt!(machine);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1C);
    assert!(
        segment >= 0xFD80,
        "INT 1Ch segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int1ch_vector_ra() {
    let mut machine = create_machine_ra();
    let _cycles = boot_to_halt!(machine);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1C);
    assert!(
        segment >= 0xFD80,
        "INT 1Ch segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

// ============================================================================
// §13 AH=00h - Get Date/Time
// ============================================================================

// The VM uses a µPD1990A calendar LSI which has no year register. The VM BIOS
// reads the year from Memory Switch 8 (text VRAM at A000:3FFEh) instead.
#[test]
fn get_datetime_reads_calendar_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    machine.set_host_date_time_source(std::sync::Arc::new(common::FixedHostDateTime(
        test_local_time(),
    )));
    write_bytes(&mut machine.bus, TEST_CODE, GET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    verify_datetime_buffer(&mut machine.bus, RESULT);
}

#[test]
fn get_datetime_reads_calendar_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    machine.set_host_date_time_source(std::sync::Arc::new(common::FixedHostDateTime(
        test_local_time(),
    )));
    write_bytes(&mut machine.bus, TEST_CODE, GET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    verify_datetime_buffer(&mut machine.bus, RESULT);
}

#[test]
fn get_datetime_reads_calendar_vx() {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    machine.set_host_date_time_source(std::sync::Arc::new(common::FixedHostDateTime(
        test_local_time(),
    )));
    write_bytes(&mut machine.bus, TEST_CODE, GET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    verify_datetime_buffer(&mut machine.bus, RESULT);
}

#[test]
fn get_datetime_reads_calendar_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    machine.set_host_date_time_source(std::sync::Arc::new(common::FixedHostDateTime(
        test_local_time(),
    )));
    write_bytes(&mut machine.bus, TEST_CODE, GET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    verify_datetime_buffer(&mut machine.bus, RESULT);
}

#[test]
fn get_datetime_uses_segment_base_in_protected_mode_ra() {
    const ES_SELECTOR: u16 = 0x1234;
    const WRONG_RESULT: u32 = (ES_SELECTOR as u32) << 4;

    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    machine.set_host_date_time_source(std::sync::Arc::new(common::FixedHostDateTime(
        test_local_time(),
    )));

    for i in 0..6 {
        machine.bus.write_byte(RESULT + i, 0x00);
        machine.bus.write_byte(WRONG_RESULT + i, 0x00);
    }

    write_bytes(&mut machine.bus, TEST_CODE, GET_DATETIME_HLE_TRAP_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            cr0: 0x0000_0001,
            ..Default::default()
        };
        s.set_esp(0x4000);

        s.set_cs(0x0008);
        s.seg_bases[cpu::SegReg32::CS as usize] = 0x0000_0000;
        s.seg_limits[cpu::SegReg32::CS as usize] = 0x0000_FFFF;
        s.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;

        s.set_ss(0x0010);
        s.seg_bases[cpu::SegReg32::SS as usize] = 0x0000_0000;
        s.seg_limits[cpu::SegReg32::SS as usize] = 0x0000_FFFF;
        s.seg_rights[cpu::SegReg32::SS as usize] = 0x93;

        s.set_ds(0x0010);
        s.seg_bases[cpu::SegReg32::DS as usize] = 0x0000_0000;
        s.seg_limits[cpu::SegReg32::DS as usize] = 0x0000_FFFF;
        s.seg_rights[cpu::SegReg32::DS as usize] = 0x93;

        s.set_es(ES_SELECTOR);
        s.seg_bases[cpu::SegReg32::ES as usize] = RESULT;
        s.seg_limits[cpu::SegReg32::ES as usize] = 0x0000_FFFF;
        s.seg_rights[cpu::SegReg32::ES as usize] = 0x93;

        s.seg_valid = [true, true, true, true, false, false];
        s
    });
    machine.run_for(INT1CH_BUDGET);

    verify_datetime_buffer(&mut machine.bus, RESULT);

    for i in 0..6 {
        assert_eq!(
            machine.bus.read_byte(WRONG_RESULT + i),
            0x00,
            "INT 1Ch AH=00h must not use selector<<4 in protected mode"
        );
    }
}

// ============================================================================
// §13 AH=01h - Set Date/Time
// ============================================================================

#[test]
fn set_datetime_completes_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, RESULT, &SET_DATE);
    write_bytes(&mut machine.bus, TEST_CODE, SET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    assert_eq!(
        machine.bus.read_byte(MARKER),
        0xAA,
        "AH=01h should complete and execution should continue"
    );
}

#[test]
fn set_datetime_completes_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, RESULT, &SET_DATE);
    write_bytes(&mut machine.bus, TEST_CODE, SET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    assert_eq!(
        machine.bus.read_byte(MARKER),
        0xAA,
        "AH=01h should complete and execution should continue"
    );
}

#[test]
fn set_datetime_completes_vx() {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, RESULT, &SET_DATE);
    write_bytes(&mut machine.bus, TEST_CODE, SET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    assert_eq!(
        machine.bus.read_byte(MARKER),
        0xAA,
        "AH=01h should complete and execution should continue"
    );
}

#[test]
fn set_datetime_completes_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, RESULT, &SET_DATE);
    write_bytes(&mut machine.bus, TEST_CODE, SET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    assert_eq!(
        machine.bus.read_byte(MARKER),
        0xAA,
        "AH=01h should complete and execution should continue"
    );
}

// ============================================================================
// §13 AH=02h - Set Interval Timer: IVT Callback
// ============================================================================

#[test]
fn setup_timer_stores_ivt_callback_f() {
    let (machine, _cycles) = boot_and_run_f(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x07);
    assert_eq!(segment, 0x0000, "Callback segment should be 0x0000 (ES=0)");
    assert_eq!(
        offset, TEST_CALLBACK as u16,
        "Callback offset should be TEST_CALLBACK (0x2000)"
    );
}

#[test]
fn setup_timer_stores_ivt_callback_vm() {
    let (machine, _cycles) = boot_and_run_vm(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x07);
    assert_eq!(segment, 0x0000, "Callback segment should be 0x0000 (ES=0)");
    assert_eq!(
        offset, TEST_CALLBACK as u16,
        "Callback offset should be TEST_CALLBACK (0x2000)"
    );
}

#[test]
fn setup_timer_stores_ivt_callback_vx() {
    let (machine, _cycles) = boot_and_run_vx(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x07);
    assert_eq!(segment, 0x0000, "Callback segment should be 0x0000 (ES=0)");
    assert_eq!(
        offset, TEST_CALLBACK as u16,
        "Callback offset should be TEST_CALLBACK (0x2000)"
    );
}

#[test]
fn setup_timer_stores_ivt_callback_ra() {
    let (machine, _cycles) = boot_and_run_ra(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x07);
    assert_eq!(segment, 0x0000, "Callback segment should be 0x0000 (ES=0)");
    assert_eq!(
        offset, TEST_CALLBACK as u16,
        "Callback offset should be TEST_CALLBACK (0x2000)"
    );
}

// ============================================================================
// §13 AH=02h - Set Interval Timer: Counter
// ============================================================================

#[test]
fn setup_timer_stores_counter_f() {
    let code = make_setup_counter_code(5);
    let (mut machine, _cycles) = boot_and_run_f(&code, CALLBACK_IRET, INT1CH_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 5, "CA_TIM_CNT should be 5 (CX=5)");
}

#[test]
fn setup_timer_stores_counter_vm() {
    let code = make_setup_counter_code(5);
    let (mut machine, _cycles) = boot_and_run_vm(&code, CALLBACK_IRET, INT1CH_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 5, "CA_TIM_CNT should be 5 (CX=5)");
}

#[test]
fn setup_timer_stores_counter_vx() {
    let code = make_setup_counter_code(5);
    let (mut machine, _cycles) = boot_and_run_vx(&code, CALLBACK_IRET, INT1CH_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 5, "CA_TIM_CNT should be 5 (CX=5)");
}

#[test]
fn setup_timer_stores_counter_ra() {
    let code = make_setup_counter_code(5);
    let (mut machine, _cycles) = boot_and_run_ra(&code, CALLBACK_IRET, INT1CH_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 5, "CA_TIM_CNT should be 5 (CX=5)");
}

// ============================================================================
// §13 AH=02h - Set Interval Timer: PIT Configuration
// ============================================================================

#[test]
fn setup_timer_programs_pit_f() {
    let (machine, _cycles) = boot_and_run_f(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    assert_eq!(
        state.pit.channels[0].ctrl, 0x36,
        "PIT ch0 ctrl should be 0x36 (mode 3) after INT 1Ch AH=02h"
    );
    assert_eq!(
        state.pit.channels[0].value, 0x4E00,
        "PIT ch0 value should be 0x4E00 (8MHz-lineage 10ms divider)"
    );
}

#[test]
fn setup_timer_programs_pit_vm() {
    let (machine, _cycles) = boot_and_run_vm(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    assert_eq!(
        state.pit.channels[0].ctrl, 0x36,
        "PIT ch0 ctrl should be 0x36 (mode 3) after INT 1Ch AH=02h"
    );
    assert_eq!(
        state.pit.channels[0].value, 0x6000,
        "PIT ch0 value should be 0x6000 (5MHz-lineage 10ms divider)"
    );
}

#[test]
fn setup_timer_programs_pit_vx() {
    let (machine, _cycles) = boot_and_run_vx(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    assert_eq!(
        state.pit.channels[0].ctrl, 0x36,
        "PIT ch0 ctrl should be 0x36 (mode 3) after INT 1Ch AH=02h"
    );
    assert_eq!(
        state.pit.channels[0].value, 0x6000,
        "PIT ch0 value should be 0x6000 (5MHz-lineage 10ms divider)"
    );
}

#[test]
fn setup_timer_programs_pit_ra() {
    let (machine, _cycles) = boot_and_run_ra(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    assert_eq!(
        state.pit.channels[0].ctrl, 0x36,
        "PIT ch0 ctrl should be 0x36 (mode 3) after INT 1Ch AH=02h"
    );
    assert_eq!(
        state.pit.channels[0].value, 0x4E00,
        "PIT ch0 value should be 0x4E00 (8MHz-lineage 10ms divider)"
    );
}

// ============================================================================
// §13 AH=01h - Set Date/Time: MSW8 Year Byte
// ============================================================================

#[test]
fn set_datetime_stores_year_in_msw8_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, RESULT, &SET_DATE);
    write_bytes(&mut machine.bus, TEST_CODE, SET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    let state = machine.inspection_state();
    assert_eq!(
        state.memory.text_vram[0x3FFE], SET_DATE[0],
        "AH=01h should store year byte ({:#04X}) in MSW8 (text VRAM 0x3FFE)",
        SET_DATE[0]
    );
}

#[test]
fn set_datetime_stores_year_in_msw8_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, RESULT, &SET_DATE);
    write_bytes(&mut machine.bus, TEST_CODE, SET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    let state = machine.inspection_state();
    assert_eq!(
        state.memory.text_vram[0x3FFE], SET_DATE[0],
        "AH=01h should store year byte ({:#04X}) in MSW8 (text VRAM 0x3FFE)",
        SET_DATE[0]
    );
}

#[test]
fn set_datetime_stores_year_in_msw8_vx() {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, RESULT, &SET_DATE);
    write_bytes(&mut machine.bus, TEST_CODE, SET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    let state = machine.inspection_state();
    assert_eq!(
        state.memory.text_vram[0x3FFE], SET_DATE[0],
        "AH=01h should store year byte ({:#04X}) in MSW8 (text VRAM 0x3FFE)",
        SET_DATE[0]
    );
}

#[test]
fn set_datetime_stores_year_in_msw8_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, RESULT, &SET_DATE);
    write_bytes(&mut machine.bus, TEST_CODE, SET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    let state = machine.inspection_state();
    assert_eq!(
        state.memory.text_vram[0x3FFE], SET_DATE[0],
        "AH=01h should store year byte ({:#04X}) in MSW8 (text VRAM 0x3FFE)",
        SET_DATE[0]
    );
}

// ============================================================================
// §13 AH=01h - Set Date/Time: All 6 Bytes Written
// ============================================================================

// Set date with distinct bytes, then verify completion.
// The BIOS should write all 6 BCD bytes to the RTC and the year to MSW8.
// We already test MSW8 above; here we verify different input bytes all complete.
const SET_DATE_DISTINCT: [u8; 6] = [0x99, 0xC7, 0x31, 0x23, 0x59, 0x58];

#[test]
fn set_datetime_distinct_bytes_completes_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, RESULT, &SET_DATE_DISTINCT);
    write_bytes(&mut machine.bus, TEST_CODE, SET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    let state = machine.inspection_state();
    assert_eq!(
        machine.bus.read_byte(MARKER),
        0xAA,
        "AH=01h with distinct BCD bytes should complete"
    );
    assert_eq!(
        state.memory.text_vram[0x3FFE], SET_DATE_DISTINCT[0],
        "AH=01h should store year byte 0x99 in MSW8"
    );
}

#[test]
fn set_datetime_distinct_bytes_completes_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, RESULT, &SET_DATE_DISTINCT);
    write_bytes(&mut machine.bus, TEST_CODE, SET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    let state = machine.inspection_state();
    assert_eq!(
        machine.bus.read_byte(MARKER),
        0xAA,
        "AH=01h with distinct BCD bytes should complete"
    );
    assert_eq!(
        state.memory.text_vram[0x3FFE], SET_DATE_DISTINCT[0],
        "AH=01h should store year byte 0x99 in MSW8"
    );
}

#[test]
fn set_datetime_distinct_bytes_completes_vx() {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, RESULT, &SET_DATE_DISTINCT);
    write_bytes(&mut machine.bus, TEST_CODE, SET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    let state = machine.inspection_state();
    assert_eq!(machine.bus.read_byte(MARKER), 0xAA);
    assert_eq!(state.memory.text_vram[0x3FFE], SET_DATE_DISTINCT[0]);
}

#[test]
fn set_datetime_distinct_bytes_completes_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, RESULT, &SET_DATE_DISTINCT);
    write_bytes(&mut machine.bus, TEST_CODE, SET_DATETIME_CODE);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT1CH_BUDGET);

    let state = machine.inspection_state();
    assert_eq!(machine.bus.read_byte(MARKER), 0xAA);
    assert_eq!(state.memory.text_vram[0x3FFE], SET_DATE_DISTINCT[0]);
}

// ============================================================================
// §13 AH=02h - Set Interval Timer: PIT FLAG_I
// ============================================================================

// After INT 1Ch AH=02h programs the PIT, FLAG_I (0x20) must be set so that
// on_timer0_event fires IRQ 0 when the counter reaches zero.
#[test]
fn setup_timer_sets_pit_flag_i_f() {
    let (machine, _cycles) = boot_and_run_f(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    assert_ne!(
        state.pit.channels[0].flag & 0x20,
        0,
        "PIT ch0 FLAG_I (0x20) should be set after INT 1Ch AH=02h (flag={:#04X})",
        state.pit.channels[0].flag
    );
}

#[test]
fn setup_timer_sets_pit_flag_i_vm() {
    let (machine, _cycles) = boot_and_run_vm(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    assert_ne!(
        state.pit.channels[0].flag & 0x20,
        0,
        "PIT ch0 FLAG_I (0x20) should be set after INT 1Ch AH=02h (flag={:#04X})",
        state.pit.channels[0].flag
    );
}

#[test]
fn setup_timer_sets_pit_flag_i_vx() {
    let (machine, _cycles) = boot_and_run_vx(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    assert_ne!(
        state.pit.channels[0].flag & 0x20,
        0,
        "PIT ch0 FLAG_I (0x20) should be set after INT 1Ch AH=02h (flag={:#04X})",
        state.pit.channels[0].flag
    );
}

#[test]
fn setup_timer_sets_pit_flag_i_ra() {
    let (machine, _cycles) = boot_and_run_ra(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    assert_ne!(
        state.pit.channels[0].flag & 0x20,
        0,
        "PIT ch0 FLAG_I (0x20) should be set after INT 1Ch AH=02h (flag={:#04X})",
        state.pit.channels[0].flag
    );
}

// ============================================================================
// §13 AH=02h - Set Interval Timer: IRQ 0 Unmasked
// ============================================================================

// After INT 1Ch AH=02h, IRQ 0 must be unmasked in the master PIC IMR.
#[test]
fn setup_timer_unmasks_irq0_f() {
    let (machine, _cycles) = boot_and_run_f(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    assert_eq!(
        state.pic.chips[0].imr & 0x01,
        0,
        "IRQ 0 should be unmasked after INT 1Ch AH=02h (IMR={:#04X})",
        state.pic.chips[0].imr
    );
}

#[test]
fn setup_timer_unmasks_irq0_vm() {
    let (machine, _cycles) = boot_and_run_vm(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    assert_eq!(
        state.pic.chips[0].imr & 0x01,
        0,
        "IRQ 0 should be unmasked after INT 1Ch AH=02h (IMR={:#04X})",
        state.pic.chips[0].imr
    );
}

#[test]
fn setup_timer_unmasks_irq0_vx() {
    let (machine, _cycles) = boot_and_run_vx(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    assert_eq!(
        state.pic.chips[0].imr & 0x01,
        0,
        "IRQ 0 should be unmasked after INT 1Ch AH=02h (IMR={:#04X})",
        state.pic.chips[0].imr
    );
}

#[test]
fn setup_timer_unmasks_irq0_ra() {
    let (machine, _cycles) = boot_and_run_ra(SETUP_TIMER_CODE, CALLBACK_IRET, INT1CH_BUDGET);
    let state = machine.inspection_state();

    assert_eq!(
        state.pic.chips[0].imr & 0x01,
        0,
        "IRQ 0 should be unmasked after INT 1Ch AH=02h (IMR={:#04X})",
        state.pic.chips[0].imr
    );
}
