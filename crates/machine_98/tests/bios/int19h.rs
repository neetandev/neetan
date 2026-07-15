use super::{
    TEST_CODE, create_machine_f, create_machine_ra, create_machine_vm, create_machine_vx,
    read_ivt_vector, read_ram_u16, write_bytes,
};

const INT19H_BUDGET: u64 = 500_000;
const RESULT: u32 = 0x0600;

/// RS-232C buffer control block address used by test code.
const RS_BUF: usize = 0x3000;

// RSBIOS field offsets
const R_INT: usize = 0x00;
const R_BFLG: usize = 0x01;
const R_FLAG: usize = 0x02;
const R_CMD: usize = 0x03;
const R_STIME: usize = 0x04;
const R_RTIME: usize = 0x05;
const R_XOFF: usize = 0x06;
const R_XON: usize = 0x08;
const R_HEADP: usize = 0x0A;
const R_TAILP: usize = 0x0C;
const R_CNT: usize = 0x0E;
const R_PUTP: usize = 0x10;
const R_GETP: usize = 0x12;
const DATA_BUF_OFFSET: usize = 0x14;
const DATA_BUF_START: usize = RS_BUF + DATA_BUF_OFFSET;

/// BDA addresses for RS-232C buffer pointer.
const RS_CH0_OFST: usize = 0x0556;
const RS_CH0_SEG: usize = 0x0558;

/// Serial init preamble: sets up registers and calls INT 19h.
/// ES=0, DI=0x3000, DX=0x0100, BH=0x04, BL=0x40, CH=0x4E, CL=0x27, AL=0x07.
#[rustfmt::skip]
fn serial_init_preamble(ah: u8) -> Vec<u8> {
    vec![
        0x31, 0xC0,             // XOR AX, AX
        0x8E, 0xC0,             // MOV ES, AX
        0xBF, 0x00, 0x30,       // MOV DI, 0x3000
        0xBA, 0x00, 0x01,       // MOV DX, 0x0100
        0xBB, 0x40, 0x04,       // MOV BX, 0x0440
        0xB9, 0x27, 0x4E,       // MOV CX, 0x4E27
        0xB8, 0x07, ah,         // MOV AX, ah:07
        0xCD, 0x19,             // INT 0x19
    ]
}

/// Init (AH=00h) + store AX to RESULT + HLT.
#[rustfmt::skip]
fn make_init_and_store() -> Vec<u8> {
    let mut code = serial_init_preamble(0x00);
    code.extend_from_slice(&[
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0xF4,                   // HLT
    ]);
    code
}

/// Init with flow control (AH=01h) + store AX to RESULT + HLT.
#[rustfmt::skip]
fn make_init_flow_and_store() -> Vec<u8> {
    let mut code = serial_init_preamble(0x01);
    code.extend_from_slice(&[
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0xF4,                   // HLT
    ]);
    code
}

/// Init + call INT 19h function + store AX and CX to RESULT + HLT.
#[rustfmt::skip]
fn make_init_then_call_store(ah: u8, al: u8) -> Vec<u8> {
    let mut code = serial_init_preamble(0x00);
    code.extend_from_slice(&[
        0xB8, al, ah,           // MOV AX, ah:al
        0xCD, 0x19,             // INT 0x19
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0x89, 0x0E, 0x02, 0x06, // MOV [RESULT+2], CX
        0xF4,                   // HLT
    ]);
    code
}

/// Call INT 19h function WITHOUT init + store AX and CX to RESULT + HLT.
#[rustfmt::skip]
fn make_no_init_call_store(ah: u8, al: u8) -> Vec<u8> {
    vec![
        0xB8, al, ah,           // MOV AX, ah:al
        0xCD, 0x19,             // INT 0x19
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0x89, 0x0E, 0x02, 0x06, // MOV [RESULT+2], CX
        0xF4,                   // HLT
    ]
}

/// Init (AH=00h) + STI + HLTs for receiving serial bytes.
/// Same pattern as `serial_receive.rs` `make_serial_init_code`.
fn make_serial_init_code(num_interrupts: usize) -> Vec<u8> {
    let mut code = serial_init_preamble(0x00);
    code.push(0xFB); // STI
    code.extend(std::iter::repeat_n(0xF4_u8, num_interrupts + 1)); // HLTs
    code
}

/// Query code: call INT 19h function + store AX and CX to RESULT + HLT.
#[rustfmt::skip]
fn make_query_store(ah: u8, al: u8) -> Vec<u8> {
    vec![
        0xB8, al, ah,           // MOV AX, ah:al
        0xCD, 0x19,             // INT 0x19
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0x89, 0x0E, 0x02, 0x06, // MOV [RESULT+2], CX
        0xF4,                   // HLT
    ]
}

/// Two-phase test helper for F: Phase 1 receives serial bytes, Phase 2 runs query code.
fn boot_serial_receive_then_query_f(serial_bytes: &[u8], query_code: &[u8]) -> machine_98::Pc9801F {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);

    let receive_code = make_serial_init_code(serial_bytes.len());
    for &byte in serial_bytes {
        machine.bus.push_serial_byte(byte);
    }
    write_bytes(&mut machine.bus, TEST_CODE, &receive_code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT19H_BUDGET);

    write_bytes(&mut machine.bus, TEST_CODE, query_code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT19H_BUDGET);
    machine
}

/// Two-phase test helper for VM: Phase 1 receives serial bytes, Phase 2 runs query code.
fn boot_serial_receive_then_query_vm(
    serial_bytes: &[u8],
    query_code: &[u8],
) -> machine_98::Pc9801Vm {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);

    // Phase 1: init serial port and receive bytes via IRQ.
    let receive_code = make_serial_init_code(serial_bytes.len());
    for &byte in serial_bytes {
        machine.bus.push_serial_byte(byte);
    }
    write_bytes(&mut machine.bus, TEST_CODE, &receive_code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT19H_BUDGET);

    // Phase 2: run query code.
    write_bytes(&mut machine.bus, TEST_CODE, query_code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT19H_BUDGET);
    machine
}

/// Two-phase test helper for VX.
fn boot_serial_receive_then_query_vx(
    serial_bytes: &[u8],
    query_code: &[u8],
) -> machine_98::Pc9801Vx {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);

    let receive_code = make_serial_init_code(serial_bytes.len());
    for &byte in serial_bytes {
        machine.bus.push_serial_byte(byte);
    }
    write_bytes(&mut machine.bus, TEST_CODE, &receive_code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT19H_BUDGET);

    write_bytes(&mut machine.bus, TEST_CODE, query_code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT19H_BUDGET);
    machine
}

/// Two-phase test helper for RA.
fn boot_serial_receive_then_query_ra(
    serial_bytes: &[u8],
    query_code: &[u8],
) -> machine_98::Pc9801Ra {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);

    let receive_code = make_serial_init_code(serial_bytes.len());
    for &byte in serial_bytes {
        machine.bus.push_serial_byte(byte);
    }
    write_bytes(&mut machine.bus, TEST_CODE, &receive_code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT19H_BUDGET);

    write_bytes(&mut machine.bus, TEST_CODE, query_code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT19H_BUDGET);
    machine
}

// ============================================================================
// IVT Vector
// ============================================================================

#[test]
fn int19h_vector_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    let state = machine.inspection_state();
    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x19);
    assert!(
        segment >= 0xFD80,
        "INT 19h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int19h_vector_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    let state = machine.inspection_state();
    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x19);
    assert!(
        segment >= 0xFD80,
        "INT 19h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int19h_vector_vx() {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    let state = machine.inspection_state();
    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x19);
    assert!(
        segment >= 0xFD80,
        "INT 19h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int19h_vector_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    let state = machine.inspection_state();
    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x19);
    assert!(
        segment >= 0xFD80,
        "INT 19h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

// ============================================================================
// AH=00h Initialize - Return Value
// ============================================================================

#[test]
fn int19h_init_returns_ok_f() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 after init (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_init_returns_ok_vm() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 after init (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_init_returns_ok_vx() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_vx(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 after init (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_init_returns_ok_ra() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_ra(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 after init (got {:#04X})",
        ax >> 8
    );
}

// ============================================================================
// AH=00h Initialize - Buffer Control Block Fields
// ============================================================================

fn assert_init_buffer_fields(ram: &[u8; 0xA0000]) {
    let flag = ram[RS_BUF + R_FLAG];
    assert_eq!(
        flag & 0x80,
        0x80,
        "FLAG should have RFLAG_INIT (0x80) set (got {flag:#04X})"
    );

    let cmd = ram[RS_BUF + R_CMD];
    assert_eq!(cmd, 0x27, "CMD should be 0x27 from CL (got {cmd:#04X})");

    let stime = ram[RS_BUF + R_STIME];
    assert_eq!(
        stime, 0x04,
        "STIME should be 0x04 from BH (got {stime:#04X})"
    );

    let rtime = ram[RS_BUF + R_RTIME];
    assert_eq!(
        rtime, 0x40,
        "RTIME should be 0x40 from BL (got {rtime:#04X})"
    );

    let headp = read_ram_u16(ram, RS_BUF + R_HEADP);
    assert_eq!(
        headp, DATA_BUF_START as u16,
        "HEADP should be {:#06X} (got {headp:#06X})",
        DATA_BUF_START
    );

    let putp = read_ram_u16(ram, RS_BUF + R_PUTP);
    assert_eq!(
        putp, DATA_BUF_START as u16,
        "PUTP should be {:#06X} (got {putp:#06X})",
        DATA_BUF_START
    );

    let getp = read_ram_u16(ram, RS_BUF + R_GETP);
    assert_eq!(
        getp, DATA_BUF_START as u16,
        "GETP should be {:#06X} (got {getp:#06X})",
        DATA_BUF_START
    );

    // TAILP = data buffer start + DX (0x0100)
    let tailp = read_ram_u16(ram, RS_BUF + R_TAILP);
    assert_eq!(
        tailp,
        (DATA_BUF_START + 0x0100) as u16,
        "TAILP should be {:#06X} (got {tailp:#06X})",
        DATA_BUF_START + 0x0100
    );

    let cnt = read_ram_u16(ram, RS_BUF + R_CNT);
    assert_eq!(cnt, 0, "CNT should be 0 after init (got {cnt})");

    // XOFF = DX >> 3 = 0x0100 >> 3 = 0x0020
    let xoff = read_ram_u16(ram, RS_BUF + R_XOFF);
    assert_eq!(xoff, 0x0020, "XOFF should be 0x0020 (got {xoff:#06X})");

    // XON = XOFF + DX >> 2 = 0x0020 + 0x0040 = 0x0060
    let xon = read_ram_u16(ram, RS_BUF + R_XON);
    assert_eq!(xon, 0x0060, "XON should be 0x0060 (got {xon:#06X})");
}

#[test]
fn int19h_init_buffer_fields_f() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_init_buffer_fields(&state.memory.ram);
}

#[test]
fn int19h_init_buffer_fields_vm() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_init_buffer_fields(&state.memory.ram);
}

#[test]
fn int19h_init_buffer_fields_vx() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_vx(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_init_buffer_fields(&state.memory.ram);
}

#[test]
fn int19h_init_buffer_fields_ra() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_ra(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_init_buffer_fields(&state.memory.ram);
}

// ============================================================================
// AH=00h Initialize - R_INT and R_BFLG Cleared
// ============================================================================

fn assert_init_clears_header(ram: &[u8; 0xA0000]) {
    let r_int = ram[RS_BUF + R_INT];
    assert_eq!(
        r_int, 0x00,
        "R_INT should be 0x00 after init (got {r_int:#04X})"
    );
    let r_bflg = ram[RS_BUF + R_BFLG];
    assert_eq!(
        r_bflg, 0x00,
        "R_BFLG should be 0x00 after init (got {r_bflg:#04X})"
    );
}

#[test]
fn int19h_init_clears_header_f() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_init_clears_header(&state.memory.ram);
}

#[test]
fn int19h_init_clears_header_vm() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_init_clears_header(&state.memory.ram);
}

#[test]
fn int19h_init_clears_header_vx() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_vx(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_init_clears_header(&state.memory.ram);
}

#[test]
fn int19h_init_clears_header_ra() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_ra(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_init_clears_header(&state.memory.ram);
}

// ============================================================================
// AH=00h Initialize - BDA Pointer
// ============================================================================

fn assert_bda_pointer(ram: &[u8; 0xA0000]) {
    let ofst = read_ram_u16(ram, RS_CH0_OFST);
    let seg = read_ram_u16(ram, RS_CH0_SEG);
    assert_eq!(
        ofst, 0x3000,
        "BDA RS_CH0_OFST should be 0x3000 (got {ofst:#06X})"
    );
    assert_eq!(
        seg, 0x0000,
        "BDA RS_CH0_SEG should be 0x0000 (got {seg:#06X})"
    );
}

#[test]
fn int19h_init_stores_bda_pointer_f() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_bda_pointer(&state.memory.ram);
}

#[test]
fn int19h_init_stores_bda_pointer_vm() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_bda_pointer(&state.memory.ram);
}

#[test]
fn int19h_init_stores_bda_pointer_vx() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_vx(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_bda_pointer(&state.memory.ram);
}

#[test]
fn int19h_init_stores_bda_pointer_ra() {
    let code = make_init_and_store();
    let (machine, _) = super::boot_and_run_ra(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_bda_pointer(&state.memory.ram);
}

// ============================================================================
// AH=01h Init with Flow Control
// ============================================================================

#[test]
fn int19h_init_flow_flag_f() {
    let code = make_init_flow_and_store();
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let flag = state.memory.ram[RS_BUF + R_FLAG];
    assert_eq!(
        flag, 0x90,
        "FLAG should be 0x90 (RFLAG_INIT | RFLAG_XON) (got {flag:#04X})"
    );
}

#[test]
fn int19h_init_flow_flag_vm() {
    let code = make_init_flow_and_store();
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let flag = state.memory.ram[RS_BUF + R_FLAG];
    assert_eq!(
        flag, 0x90,
        "FLAG should be 0x90 (RFLAG_INIT | RFLAG_XON) (got {flag:#04X})"
    );
}

#[test]
fn int19h_init_flow_flag_vx() {
    let code = make_init_flow_and_store();
    let (machine, _) = super::boot_and_run_vx(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let flag = state.memory.ram[RS_BUF + R_FLAG];
    assert_eq!(
        flag, 0x90,
        "FLAG should be 0x90 (RFLAG_INIT | RFLAG_XON) (got {flag:#04X})"
    );
}

#[test]
fn int19h_init_flow_flag_ra() {
    let code = make_init_flow_and_store();
    let (machine, _) = super::boot_and_run_ra(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let flag = state.memory.ram[RS_BUF + R_FLAG];
    assert_eq!(
        flag, 0x90,
        "FLAG should be 0x90 (RFLAG_INIT | RFLAG_XON) (got {flag:#04X})"
    );
}

// ============================================================================
// AH=02h RX Char Count - Not Initialized
// ============================================================================

#[test]
fn int19h_rx_count_not_init_f() {
    let code = make_no_init_call_store(0x02, 0x00);
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_rx_count_not_init_vm() {
    let code = make_no_init_call_store(0x02, 0x00);
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_rx_count_not_init_vx() {
    let code = make_no_init_call_store(0x02, 0x00);
    let (machine, _) = super::boot_and_run_vx(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_rx_count_not_init_ra() {
    let code = make_no_init_call_store(0x02, 0x00);
    let (machine, _) = super::boot_and_run_ra(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

// ============================================================================
// AH=02h RX Char Count - Empty Buffer
// ============================================================================

#[test]
fn int19h_rx_count_zero_f() {
    let code = make_init_then_call_store(0x02, 0x00);
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(ax >> 8, 0x00, "AH should be 0x00 (got {:#04X})", ax >> 8);
    assert_eq!(cx, 0x0000, "CX should be 0 (got {cx:#06X})");
}

#[test]
fn int19h_rx_count_zero_vm() {
    let code = make_init_then_call_store(0x02, 0x00);
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(ax >> 8, 0x00, "AH should be 0x00 (got {:#04X})", ax >> 8);
    assert_eq!(cx, 0x0000, "CX should be 0 (got {cx:#06X})");
}

#[test]
fn int19h_rx_count_zero_vx() {
    let code = make_init_then_call_store(0x02, 0x00);
    let (machine, _) = super::boot_and_run_vx(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(ax >> 8, 0x00, "AH should be 0x00 (got {:#04X})", ax >> 8);
    assert_eq!(cx, 0x0000, "CX should be 0 (got {cx:#06X})");
}

#[test]
fn int19h_rx_count_zero_ra() {
    let code = make_init_then_call_store(0x02, 0x00);
    let (machine, _) = super::boot_and_run_ra(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(ax >> 8, 0x00, "AH should be 0x00 (got {:#04X})", ax >> 8);
    assert_eq!(cx, 0x0000, "CX should be 0 (got {cx:#06X})");
}

// ============================================================================
// AH=02h RX Char Count - After Receiving Data
// ============================================================================

#[test]
fn int19h_rx_count_after_receive_f() {
    let query = make_query_store(0x02, 0x00);
    let machine = boot_serial_receive_then_query_f(&[0x41, 0x42], &query);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(ax >> 8, 0x00, "AH should be 0x00 (got {:#04X})", ax >> 8);
    assert_eq!(cx, 0x0002, "CX should be 2 (got {cx:#06X})");
}

#[test]
fn int19h_rx_count_after_receive_vm() {
    let query = make_query_store(0x02, 0x00);
    let machine = boot_serial_receive_then_query_vm(&[0x41, 0x42], &query);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(ax >> 8, 0x00, "AH should be 0x00 (got {:#04X})", ax >> 8);
    assert_eq!(cx, 0x0002, "CX should be 2 (got {cx:#06X})");
}

#[test]
fn int19h_rx_count_after_receive_vx() {
    let query = make_query_store(0x02, 0x00);
    let machine = boot_serial_receive_then_query_vx(&[0x41, 0x42], &query);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(ax >> 8, 0x00, "AH should be 0x00 (got {:#04X})", ax >> 8);
    assert_eq!(cx, 0x0002, "CX should be 2 (got {cx:#06X})");
}

#[test]
fn int19h_rx_count_after_receive_ra() {
    let query = make_query_store(0x02, 0x00);
    let machine = boot_serial_receive_then_query_ra(&[0x41, 0x42], &query);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(ax >> 8, 0x00, "AH should be 0x00 (got {:#04X})", ax >> 8);
    assert_eq!(cx, 0x0002, "CX should be 2 (got {cx:#06X})");
}

// ============================================================================
// AH=03h Send Char - Not Initialized
// ============================================================================

#[test]
fn int19h_send_not_init_f() {
    let code = make_no_init_call_store(0x03, 0x41);
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_send_not_init_vm() {
    let code = make_no_init_call_store(0x03, 0x41);
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_send_not_init_vx() {
    let code = make_no_init_call_store(0x03, 0x41);
    let (machine, _) = super::boot_and_run_vx(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_send_not_init_ra() {
    let code = make_no_init_call_store(0x03, 0x41);
    let (machine, _) = super::boot_and_run_ra(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

// ============================================================================
// AH=03h Send Char - Success
// ============================================================================

#[test]
fn int19h_send_char_f() {
    let code = make_init_then_call_store(0x03, 0x41);
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_send_char_vm() {
    let code = make_init_then_call_store(0x03, 0x41);
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_send_char_vx() {
    let code = make_init_then_call_store(0x03, 0x41);
    let (machine, _) = super::boot_and_run_vx(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_send_char_ra() {
    let code = make_init_then_call_store(0x03, 0x41);
    let (machine, _) = super::boot_and_run_ra(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
}

// ============================================================================
// AH=04h Receive Char - Not Initialized
// ============================================================================

#[test]
fn int19h_recv_not_init_f() {
    let code = make_no_init_call_store(0x04, 0x00);
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_recv_not_init_vm() {
    let code = make_no_init_call_store(0x04, 0x00);
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_recv_not_init_vx() {
    let code = make_no_init_call_store(0x04, 0x00);
    let (machine, _) = super::boot_and_run_vx(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_recv_not_init_ra() {
    let code = make_no_init_call_store(0x04, 0x00);
    let (machine, _) = super::boot_and_run_ra(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

// ============================================================================
// AH=04h Receive Char - Success (read back received byte)
// ============================================================================

#[test]
fn int19h_recv_char_f() {
    let query = make_query_store(0x04, 0x00);
    let machine = boot_serial_receive_then_query_f(&[0x41], &query);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
    assert_eq!(
        cx >> 8,
        0x41,
        "CH should be 0x41 (received byte) (got {:#04X})",
        cx >> 8
    );

    let cnt = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(cnt, 0, "CNT should be 0 after reading the byte (got {cnt})");

    let getp = read_ram_u16(&state.memory.ram, RS_BUF + R_GETP) as usize;
    assert_eq!(
        getp,
        DATA_BUF_START + 2,
        "GETP should advance by 2 (got {getp:#06X})"
    );
}

#[test]
fn int19h_recv_char_vm() {
    let query = make_query_store(0x04, 0x00);
    let machine = boot_serial_receive_then_query_vm(&[0x41], &query);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
    assert_eq!(
        cx >> 8,
        0x41,
        "CH should be 0x41 (received byte) (got {:#04X})",
        cx >> 8
    );

    let cnt = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(cnt, 0, "CNT should be 0 after reading the byte (got {cnt})");

    let getp = read_ram_u16(&state.memory.ram, RS_BUF + R_GETP) as usize;
    assert_eq!(
        getp,
        DATA_BUF_START + 2,
        "GETP should advance by 2 (got {getp:#06X})"
    );
}

#[test]
fn int19h_recv_char_vx() {
    let query = make_query_store(0x04, 0x00);
    let machine = boot_serial_receive_then_query_vx(&[0x41], &query);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
    assert_eq!(
        cx >> 8,
        0x41,
        "CH should be 0x41 (received byte) (got {:#04X})",
        cx >> 8
    );

    let cnt = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(cnt, 0, "CNT should be 0 after reading the byte (got {cnt})");

    let getp = read_ram_u16(&state.memory.ram, RS_BUF + R_GETP) as usize;
    assert_eq!(
        getp,
        DATA_BUF_START + 2,
        "GETP should advance by 2 (got {getp:#06X})"
    );
}

#[test]
fn int19h_recv_char_ra() {
    let query = make_query_store(0x04, 0x00);
    let machine = boot_serial_receive_then_query_ra(&[0x41], &query);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
    assert_eq!(
        cx >> 8,
        0x41,
        "CH should be 0x41 (received byte) (got {:#04X})",
        cx >> 8
    );

    let cnt = read_ram_u16(&state.memory.ram, RS_BUF + R_CNT);
    assert_eq!(cnt, 0, "CNT should be 0 after reading the byte (got {cnt})");

    let getp = read_ram_u16(&state.memory.ram, RS_BUF + R_GETP) as usize;
    assert_eq!(
        getp,
        DATA_BUF_START + 2,
        "GETP should advance by 2 (got {getp:#06X})"
    );
}

// ============================================================================
// AH=04h Receive Char - Timeout (empty buffer)
// ============================================================================

#[test]
fn int19h_recv_timeout_f() {
    let code = make_init_then_call_store(0x04, 0x00);
    let (machine, _) = super::boot_and_run_f(&code, &[], 200_000_000);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x03,
        "AH should be 0x03 (timeout) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_recv_timeout_vm() {
    let code = make_init_then_call_store(0x04, 0x00);
    let (machine, _) = super::boot_and_run_vm(&code, &[], 200_000_000);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x03,
        "AH should be 0x03 (timeout) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_recv_timeout_vx() {
    let code = make_init_then_call_store(0x04, 0x00);
    let (machine, _) = super::boot_and_run_vx(&code, &[], 500_000_000);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x03,
        "AH should be 0x03 (timeout) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_recv_timeout_ra() {
    let code = make_init_then_call_store(0x04, 0x00);
    let (machine, _) = super::boot_and_run_ra(&code, &[], 500_000_000);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x03,
        "AH should be 0x03 (timeout) (got {:#04X})",
        ax >> 8
    );
}

// ============================================================================
// AH=05h Command Output - Not Initialized
// ============================================================================

#[test]
fn int19h_cmd_not_init_f() {
    let code = make_no_init_call_store(0x05, 0x37);
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_cmd_not_init_vm() {
    let code = make_no_init_call_store(0x05, 0x37);
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_cmd_not_init_vx() {
    let code = make_no_init_call_store(0x05, 0x37);
    let (machine, _) = super::boot_and_run_vx(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_cmd_not_init_ra() {
    let code = make_no_init_call_store(0x05, 0x37);
    let (machine, _) = super::boot_and_run_ra(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

// ============================================================================
// AH=05h Command Output - Success
// ============================================================================

#[test]
fn int19h_cmd_output_f() {
    let code = make_init_then_call_store(0x05, 0x37);
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
    let cmd = state.memory.ram[RS_BUF + R_CMD];
    assert_eq!(
        cmd, 0x37,
        "CMD field should be updated to 0x37 (got {cmd:#04X})"
    );
}

#[test]
fn int19h_cmd_output_vm() {
    let code = make_init_then_call_store(0x05, 0x37);
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
    let cmd = state.memory.ram[RS_BUF + R_CMD];
    assert_eq!(
        cmd, 0x37,
        "CMD field should be updated to 0x37 (got {cmd:#04X})"
    );
}

#[test]
fn int19h_cmd_output_vx() {
    let code = make_init_then_call_store(0x05, 0x37);
    let (machine, _) = super::boot_and_run_vx(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
    let cmd = state.memory.ram[RS_BUF + R_CMD];
    assert_eq!(
        cmd, 0x37,
        "CMD field should be updated to 0x37 (got {cmd:#04X})"
    );
}

#[test]
fn int19h_cmd_output_ra() {
    let code = make_init_then_call_store(0x05, 0x37);
    let (machine, _) = super::boot_and_run_ra(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
    let cmd = state.memory.ram[RS_BUF + R_CMD];
    assert_eq!(
        cmd, 0x37,
        "CMD field should be updated to 0x37 (got {cmd:#04X})"
    );
}

// ============================================================================
// AH=06h Status Read - Not Initialized
// ============================================================================

#[test]
fn int19h_status_not_init_f() {
    let code = make_no_init_call_store(0x06, 0x00);
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_status_not_init_vm() {
    let code = make_no_init_call_store(0x06, 0x00);
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_status_not_init_vx() {
    let code = make_no_init_call_store(0x06, 0x00);
    let (machine, _) = super::boot_and_run_vx(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn int19h_status_not_init_ra() {
    let code = make_no_init_call_store(0x06, 0x00);
    let (machine, _) = super::boot_and_run_ra(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        0x01,
        "AH should be 0x01 (not init) (got {:#04X})",
        ax >> 8
    );
}

// ============================================================================
// AH=06h Status Read - Success
// ============================================================================

#[test]
fn int19h_status_read_f() {
    let code = make_init_then_call_store(0x06, 0x00);
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
    let ch = (cx >> 8) as u8;
    assert_eq!(
        ch & 0x85,
        0x85,
        "CH should have TxRDY|TxEMPTY|DSR (0x85) set (got {ch:#04X})"
    );
    let cl = (cx & 0xFF) as u8;
    assert_eq!(
        cl & 0xE8,
        0xE8,
        "CL should have CI#|CS#|CD#|CRTT (0xE8) from port 0x33 (got {cl:#04X})"
    );
}

#[test]
fn int19h_status_read_vm() {
    let code = make_init_then_call_store(0x06, 0x00);
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
    let ch = (cx >> 8) as u8;
    assert_eq!(
        ch & 0x85,
        0x85,
        "CH should have TxRDY|TxEMPTY|DSR (0x85) set (got {ch:#04X})"
    );
    let cl = (cx & 0xFF) as u8;
    assert_eq!(
        cl & 0xE8,
        0xE8,
        "CL should have CI#|CS#|CD#|CRTT (0xE8) from port 0x33 (got {cl:#04X})"
    );
}

#[test]
fn int19h_status_read_vx() {
    let code = make_init_then_call_store(0x06, 0x00);
    let (machine, _) = super::boot_and_run_vx(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
    let ch = (cx >> 8) as u8;
    assert_eq!(
        ch & 0x85,
        0x85,
        "CH should have TxRDY|TxEMPTY|DSR (0x85) set (got {ch:#04X})"
    );
    let cl = (cx & 0xFF) as u8;
    assert_eq!(
        cl & 0xE8,
        0xE8,
        "CL should have CI#|CS#|CD#|CRTT (0xE8) from port 0x33 (got {cl:#04X})"
    );
}

#[test]
fn int19h_status_read_ra() {
    let code = make_init_then_call_store(0x06, 0x00);
    let (machine, _) = super::boot_and_run_ra(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(
        ax >> 8,
        0x00,
        "AH should be 0x00 (success) (got {:#04X})",
        ax >> 8
    );
    let ch = (cx >> 8) as u8;
    assert_eq!(
        ch & 0x85,
        0x85,
        "CH should have TxRDY|TxEMPTY|DSR (0x85) set (got {ch:#04X})"
    );
    let cl = (cx & 0xFF) as u8;
    assert_eq!(
        cl & 0xE8,
        0xE8,
        "CL should have CI#|CS#|CD#|CRTT (0xE8) from port 0x33 (got {cl:#04X})"
    );
}

// ============================================================================
// AH=00h Init with IR bit set - should NOT set RFLAG_INIT
// ============================================================================

/// Init preamble with custom CL value.
#[rustfmt::skip]
fn serial_init_preamble_cl(ah: u8, cl: u8) -> Vec<u8> {
    vec![
        0x31, 0xC0,             // XOR AX, AX
        0x8E, 0xC0,             // MOV ES, AX
        0xBF, 0x00, 0x30,       // MOV DI, 0x3000
        0xBA, 0x00, 0x01,       // MOV DX, 0x0100
        0xBB, 0x40, 0x04,       // MOV BX, 0x0440
        0xB9, cl, 0x4E,         // MOV CX, 0x4E:cl
        0xB8, 0x07, ah,         // MOV AX, ah:07
        0xCD, 0x19,             // INT 0x19
    ]
}

#[test]
fn int19h_init_with_ir_clears_init_flag_f() {
    // CL=0x67: IR bit (0x40) set + RXE (0x04) set.
    let mut code = serial_init_preamble_cl(0x00, 0x67);
    code.extend_from_slice(&[0xF4]); // HLT
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let flag = state.memory.ram[RS_BUF + R_FLAG];
    assert_eq!(
        flag & 0x80,
        0,
        "FLAG should NOT have RFLAG_INIT when IR bit is set in CL (got {flag:#04X})"
    );
}

#[test]
fn int19h_init_with_ir_clears_init_flag_vm() {
    // CL=0x67: IR bit (0x40) set + RXE (0x04) set.
    let mut code = serial_init_preamble_cl(0x00, 0x67);
    code.extend_from_slice(&[0xF4]); // HLT
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    let flag = state.memory.ram[RS_BUF + R_FLAG];
    assert_eq!(
        flag & 0x80,
        0,
        "FLAG should NOT have RFLAG_INIT when IR bit is set in CL (got {flag:#04X})"
    );
}

#[test]
fn int19h_init_without_rxe_does_not_unmask_irq4_f() {
    // CL=0x23: no IR, no RXE - IRQ4 should remain masked.
    let mut code = serial_init_preamble_cl(0x00, 0x23);
    code.extend_from_slice(&[0xF4]); // HLT
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_ne!(
        state.pic.chips[0].imr & 0x10,
        0,
        "IRQ4 should remain masked when RXE is not set in CL"
    );
}

#[test]
fn int19h_init_without_rxe_does_not_unmask_irq4_vm() {
    // CL=0x23: no IR, no RXE - IRQ4 should remain masked.
    let mut code = serial_init_preamble_cl(0x00, 0x23);
    code.extend_from_slice(&[0xF4]); // HLT
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_ne!(
        state.pic.chips[0].imr & 0x10,
        0,
        "IRQ4 should remain masked when RXE is not set in CL"
    );
}

#[test]
fn int19h_init_with_rxe_unmasks_irq4_f() {
    // CL=0x27: no IR, RXE set - IRQ4 should be unmasked.
    let mut code = serial_init_preamble_cl(0x00, 0x27);
    code.extend_from_slice(&[0xF4]); // HLT
    let (machine, _) = super::boot_and_run_f(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_eq!(
        state.pic.chips[0].imr & 0x10,
        0,
        "IRQ4 should be unmasked when RXE is set in CL"
    );
}

#[test]
fn int19h_init_with_rxe_unmasks_irq4_vm() {
    // CL=0x27: no IR, RXE set - IRQ4 should be unmasked.
    let mut code = serial_init_preamble_cl(0x00, 0x27);
    code.extend_from_slice(&[0xF4]); // HLT
    let (machine, _) = super::boot_and_run_vm(&code, &[], INT19H_BUDGET);
    let state = machine.inspection_state();
    assert_eq!(
        state.pic.chips[0].imr & 0x10,
        0,
        "IRQ4 should be unmasked when RXE is set in CL"
    );
}
