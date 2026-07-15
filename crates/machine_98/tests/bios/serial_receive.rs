use common::Bus;

use super::{
    TEST_CODE, create_machine_f, create_machine_ra, create_machine_vm, create_machine_vx,
    read_ivt_vector, read_ram_u16, write_bytes,
};

const SERIAL_BUDGET: u64 = 500_000;

/// RS-232C buffer control block address used by test code.
const RS_BUF: usize = 0x3000;

// RSBIOS structure offsets.
const R_CNT: usize = 0x0E;
const R_PUTP: usize = 0x10;

/// Initial data-buffer start: RS_BUF + sizeof(RSBIOS) = 0x3000 + 0x14.
const DATA_BUF_START: usize = RS_BUF + 0x14;

/// Generates test code that initializes the serial port via INT 19h AH=00h,
/// then executes STI followed by `num_interrupts + 1` HLT instructions.
#[rustfmt::skip]
fn make_serial_init_code(num_interrupts: usize) -> Vec<u8> {
    let mut code = vec![
        0x31, 0xC0,                         // XOR AX, AX
        0x8E, 0xC0,                         // MOV ES, AX
        0xBF, 0x00, 0x30,                   // MOV DI, 0x3000  (buffer control block)
        0xBA, 0x00, 0x01,                   // MOV DX, 0x0100  (256-byte data buffer)
        0xBB, 0x40, 0x04,                   // MOV BX, 0x0440  (BH=send timeout, BL=recv timeout)
        0xB9, 0x27, 0x4E,                   // MOV CX, 0x4E27  (CH=mode 8N1 16x, CL=cmd TxEN|DTR|RxEN|RTS)
        0xB8, 0x07, 0x00,                   // MOV AX, 0x0007  (AH=00 init, AL=07 9600 baud)
        0xCD, 0x19,                         // INT 0x19  (initialize RS-232C)
        0xFB,                               // STI
    ];
    code.extend(std::iter::repeat_n(0xF4_u8, num_interrupts + 1)); // HLTs
    code
}

fn boot_inject_run_f(serial_bytes: &[u8], code: &[u8]) -> machine_98::Pc9801F {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    for &byte in serial_bytes {
        machine.bus.push_serial_byte(byte);
    }
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(SERIAL_BUDGET);
    machine
}

fn boot_inject_run_vm(serial_bytes: &[u8], code: &[u8]) -> machine_98::Pc9801Vm {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    for &byte in serial_bytes {
        machine.bus.push_serial_byte(byte);
    }
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(SERIAL_BUDGET);
    machine
}

fn boot_inject_run_vx(serial_bytes: &[u8], code: &[u8]) -> machine_98::Pc9801Vx {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    for &byte in serial_bytes {
        machine.bus.push_serial_byte(byte);
    }
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(SERIAL_BUDGET);
    machine
}

fn boot_inject_run_ra(serial_bytes: &[u8], code: &[u8]) -> machine_98::Pc9801Ra {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    for &byte in serial_bytes {
        machine.bus.push_serial_byte(byte);
    }
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(SERIAL_BUDGET);
    machine
}

// ============================================================================
// §7 INT 0Ch - Vector Setup
// ============================================================================

#[test]
fn int0ch_vector_f() {
    let mut machine = create_machine_f();
    let _cycles = boot_to_halt!(machine);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x0C);
    assert!(
        segment >= 0xFD80,
        "INT 0Ch segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int0ch_vector_vm() {
    let mut machine = create_machine_vm();
    let _cycles = boot_to_halt!(machine);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x0C);
    assert!(
        segment >= 0xFD80,
        "INT 0Ch segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int0ch_vector_vx() {
    let mut machine = create_machine_vx();
    let _cycles = boot_to_halt!(machine);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x0C);
    assert!(
        segment >= 0xFD80,
        "INT 0Ch segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int0ch_vector_ra() {
    let mut machine = create_machine_ra();
    let _cycles = boot_to_halt!(machine);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x0C);
    assert!(
        segment >= 0xFD80,
        "INT 0Ch segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

// ============================================================================
// §7 Serial Receive - Single Byte
// ============================================================================

#[test]
fn serial_receive_single_byte_f() {
    let code = make_serial_init_code(1);
    let machine = boot_inject_run_f(&[0x41], &code);
    let state = machine.inspection_state();

    let count = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(
        count, 1,
        "RS buffer count should be 1 after one received byte"
    );

    let putp = read_ram_u16(&state.memory.ram, RS_BUF + R_PUTP) as usize;
    assert_eq!(putp, DATA_BUF_START + 2, "RS PUTP should advance by 2");

    let entry = read_ram_u16(&state.memory.ram, DATA_BUF_START);
    assert_eq!(
        (entry >> 8) as u8,
        0x41,
        "High byte of buffer entry should contain the received data byte 0x41"
    );
}

#[test]
fn serial_receive_single_byte_vm() {
    let code = make_serial_init_code(1);
    let machine = boot_inject_run_vm(&[0x41], &code);
    let state = machine.inspection_state();

    let count = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(
        count, 1,
        "RS buffer count should be 1 after one received byte"
    );

    let putp = read_ram_u16(&state.memory.ram, RS_BUF + R_PUTP) as usize;
    assert_eq!(putp, DATA_BUF_START + 2, "RS PUTP should advance by 2");

    let entry = read_ram_u16(&state.memory.ram, DATA_BUF_START);
    assert_eq!(
        (entry >> 8) as u8,
        0x41,
        "High byte of buffer entry should contain the received data byte 0x41"
    );
}

#[test]
fn serial_receive_single_byte_vx() {
    let code = make_serial_init_code(1);
    let machine = boot_inject_run_vx(&[0x41], &code);
    let state = machine.inspection_state();

    let count = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(
        count, 1,
        "RS buffer count should be 1 after one received byte"
    );

    let putp = read_ram_u16(&state.memory.ram, RS_BUF + R_PUTP) as usize;
    assert_eq!(putp, DATA_BUF_START + 2, "RS PUTP should advance by 2");

    let entry = read_ram_u16(&state.memory.ram, DATA_BUF_START);
    assert_eq!(
        (entry >> 8) as u8,
        0x41,
        "High byte of buffer entry should contain the received data byte 0x41"
    );
}

#[test]
fn serial_receive_single_byte_ra() {
    let code = make_serial_init_code(1);
    let machine = boot_inject_run_ra(&[0x41], &code);
    let state = machine.inspection_state();

    let count = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(
        count, 1,
        "RS buffer count should be 1 after one received byte"
    );

    let putp = read_ram_u16(&state.memory.ram, RS_BUF + R_PUTP) as usize;
    assert_eq!(putp, DATA_BUF_START + 2, "RS PUTP should advance by 2");

    let entry = read_ram_u16(&state.memory.ram, DATA_BUF_START);
    assert_eq!(
        (entry >> 8) as u8,
        0x41,
        "High byte of buffer entry should contain the received data byte 0x41"
    );
}

// ============================================================================
// §7 Serial Receive - Multiple Bytes
// ============================================================================

#[test]
fn serial_receive_multiple_bytes_f() {
    let code = make_serial_init_code(2);
    let machine = boot_inject_run_f(&[0x41, 0x42], &code);
    let state = machine.inspection_state();

    let count = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(
        count, 2,
        "RS buffer count should be 2 after two received bytes"
    );

    let putp = read_ram_u16(&state.memory.ram, RS_BUF + R_PUTP) as usize;
    assert_eq!(
        putp,
        DATA_BUF_START + 4,
        "RS PUTP should advance by 4 (two entries)"
    );
}

#[test]
fn serial_receive_multiple_bytes_vm() {
    let code = make_serial_init_code(2);
    let machine = boot_inject_run_vm(&[0x41, 0x42], &code);
    let state = machine.inspection_state();

    let count = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(
        count, 2,
        "RS buffer count should be 2 after two received bytes"
    );

    let putp = read_ram_u16(&state.memory.ram, RS_BUF + R_PUTP) as usize;
    assert_eq!(
        putp,
        DATA_BUF_START + 4,
        "RS PUTP should advance by 4 (two entries)"
    );
}

#[test]
fn serial_receive_multiple_bytes_vx() {
    let code = make_serial_init_code(2);
    let machine = boot_inject_run_vx(&[0x41, 0x42], &code);
    let state = machine.inspection_state();

    let count = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(
        count, 2,
        "RS buffer count should be 2 after two received bytes"
    );

    let putp = read_ram_u16(&state.memory.ram, RS_BUF + R_PUTP) as usize;
    assert_eq!(
        putp,
        DATA_BUF_START + 4,
        "RS PUTP should advance by 4 (two entries)"
    );
}

#[test]
fn serial_receive_multiple_bytes_ra() {
    let code = make_serial_init_code(2);
    let machine = boot_inject_run_ra(&[0x41, 0x42], &code);
    let state = machine.inspection_state();

    let count = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(
        count, 2,
        "RS buffer count should be 2 after two received bytes"
    );

    let putp = read_ram_u16(&state.memory.ram, RS_BUF + R_PUTP) as usize;
    assert_eq!(
        putp,
        DATA_BUF_START + 4,
        "RS PUTP should advance by 4 (two entries)"
    );
}

// ============================================================================
// §7 Serial Receive - EOI
// ============================================================================

#[test]
fn serial_receive_sends_eoi_f() {
    let code = make_serial_init_code(1);
    let machine = boot_inject_run_f(&[0x41], &code);
    let state = machine.inspection_state();

    assert_eq!(
        state.pic.chips[0].isr & 0x10,
        0,
        "IRQ 4 should not be in-service after INT 0Ch (EOI was sent)"
    );
}

#[test]
fn serial_receive_sends_eoi_vm() {
    let code = make_serial_init_code(1);
    let machine = boot_inject_run_vm(&[0x41], &code);
    let state = machine.inspection_state();

    assert_eq!(
        state.pic.chips[0].isr & 0x10,
        0,
        "IRQ 4 should not be in-service after INT 0Ch (EOI was sent)"
    );
}

#[test]
fn serial_receive_sends_eoi_vx() {
    let code = make_serial_init_code(1);
    let machine = boot_inject_run_vx(&[0x41], &code);
    let state = machine.inspection_state();

    assert_eq!(
        state.pic.chips[0].isr & 0x10,
        0,
        "IRQ 4 should not be in-service after INT 0Ch (EOI was sent)"
    );
}

#[test]
fn serial_receive_sends_eoi_ra() {
    let code = make_serial_init_code(1);
    let machine = boot_inject_run_ra(&[0x41], &code);
    let state = machine.inspection_state();

    assert_eq!(
        state.pic.chips[0].isr & 0x10,
        0,
        "IRQ 4 should not be in-service after INT 0Ch (EOI was sent)"
    );
}

// ============================================================================
// §7 Serial Receive - SI/SO Character Conversion
// ============================================================================

#[test]
fn serial_receive_so_not_buffered_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    machine.bus.write_byte(0x055B, 0x80);
    machine.bus.push_serial_byte(0x0E);
    let code = make_serial_init_code(1);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(SERIAL_BUDGET);
    let state = machine.inspection_state();
    let count = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(
        count, 0,
        "SO (0x0E) should not be buffered when SI/SO conversion is enabled"
    );
    assert_eq!(
        state.memory.ram[0x055B] & 0x10,
        0x10,
        "RS_S_FLAG bit 4 (shift-out) should be set after receiving SO"
    );
}

#[test]
fn serial_receive_so_not_buffered_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    machine.bus.write_byte(0x055B, 0x80);
    machine.bus.push_serial_byte(0x0E);
    let code = make_serial_init_code(1);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(SERIAL_BUDGET);
    let state = machine.inspection_state();
    let count = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(
        count, 0,
        "SO (0x0E) should not be buffered when SI/SO conversion is enabled"
    );
    assert_eq!(
        state.memory.ram[0x055B] & 0x10,
        0x10,
        "RS_S_FLAG bit 4 (shift-out) should be set after receiving SO"
    );
}

#[test]
fn serial_receive_si_clears_shift_out_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    machine.bus.write_byte(0x055B, 0x90);
    machine.bus.push_serial_byte(0x0F);
    let code = make_serial_init_code(1);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(SERIAL_BUDGET);
    let state = machine.inspection_state();
    let count = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(
        count, 0,
        "SI (0x0F) should not be buffered when SI/SO conversion is enabled"
    );
    assert_eq!(
        state.memory.ram[0x055B] & 0x10,
        0x00,
        "RS_S_FLAG bit 4 (shift-out) should be cleared after receiving SI"
    );
}

#[test]
fn serial_receive_si_clears_shift_out_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    machine.bus.write_byte(0x055B, 0x90);
    machine.bus.push_serial_byte(0x0F);
    let code = make_serial_init_code(1);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(SERIAL_BUDGET);
    let state = machine.inspection_state();
    let count = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(
        count, 0,
        "SI (0x0F) should not be buffered when SI/SO conversion is enabled"
    );
    assert_eq!(
        state.memory.ram[0x055B] & 0x10,
        0x00,
        "RS_S_FLAG bit 4 (shift-out) should be cleared after receiving SI"
    );
}

#[test]
fn serial_receive_so_sets_bit7_on_data_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    machine.bus.write_byte(0x055B, 0x90);
    machine.bus.push_serial_byte(0x41);
    let code = make_serial_init_code(1);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(SERIAL_BUDGET);
    let state = machine.inspection_state();
    let entry = read_ram_u16(&state.memory.ram, DATA_BUF_START);
    assert_eq!(
        (entry >> 8) as u8,
        0xC1,
        "After SO, data byte 0x41 should have bit 7 set (0xC1)"
    );
}

#[test]
fn serial_receive_so_sets_bit7_on_data_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    machine.bus.write_byte(0x055B, 0x90);
    machine.bus.push_serial_byte(0x41);
    let code = make_serial_init_code(1);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(SERIAL_BUDGET);
    let state = machine.inspection_state();
    let entry = read_ram_u16(&state.memory.ram, DATA_BUF_START);
    assert_eq!(
        (entry >> 8) as u8,
        0xC1,
        "After SO, data byte 0x41 should have bit 7 set (0xC1)"
    );
}

#[test]
fn serial_receive_si_clears_bit7_on_data_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    machine.bus.write_byte(0x055B, 0x80);
    machine.bus.push_serial_byte(0x41);
    let code = make_serial_init_code(1);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(SERIAL_BUDGET);
    let state = machine.inspection_state();
    let entry = read_ram_u16(&state.memory.ram, DATA_BUF_START);
    assert_eq!(
        (entry >> 8) as u8,
        0x41,
        "Without SO (SI mode), data byte 0x41 should have bit 7 clear"
    );
}

#[test]
fn serial_receive_si_clears_bit7_on_data_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    machine.bus.write_byte(0x055B, 0x80);
    machine.bus.push_serial_byte(0x41);
    let code = make_serial_init_code(1);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(SERIAL_BUDGET);
    let state = machine.inspection_state();
    let entry = read_ram_u16(&state.memory.ram, DATA_BUF_START);
    assert_eq!(
        (entry >> 8) as u8,
        0x41,
        "Without SO (SI mode), data byte 0x41 should have bit 7 clear"
    );
}
