use super::{
    TEST_CODE, boot_and_run_f, boot_and_run_ra, boot_and_run_vm, boot_and_run_vx, create_machine_f,
    create_machine_ra, create_machine_vm, create_machine_vx, read_ivt_vector, read_ram_u16,
    write_bytes,
};
const INT1AH_BUDGET: u64 = 2_000_000;
const RESULT: u32 = 0x0600;

/// INT 1Ah call with AH parameter, store AX to RESULT, HLT.
#[rustfmt::skip]
fn make_int1ah_call_store_ax(ah: u8) -> Vec<u8> {
    vec![
        0xB4, ah,               // MOV AH, ah
        0xCD, 0x1A,             // INT 0x1A
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0xF4,                   // HLT
    ]
}

/// INT 1Ah call with AH and AL parameters, store AX to RESULT, HLT.
#[rustfmt::skip]
fn make_int1ah_call_al_store_ax(ah: u8, al: u8) -> Vec<u8> {
    vec![
        0xB8, al, ah,           // MOV AX, ah:al
        0xCD, 0x1A,             // INT 0x1A
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0xF4,                   // HLT
    ]
}

/// AH=30h print buffer: ES:BX=0x0000:0x0700, CX=count, store AX and CX to RESULT, HLT.
#[rustfmt::skip]
fn make_int1ah_print_buffer(count: u16) -> Vec<u8> {
    vec![
        0x31, 0xC0,                                             // XOR AX, AX
        0x8E, 0xC0,                                             // MOV ES, AX
        0xBB, 0x00, 0x07,                                       // MOV BX, 0x0700
        0xB9, (count & 0xFF) as u8, (count >> 8) as u8,         // MOV CX, count
        0xB4, 0x30,                                              // MOV AH, 0x30
        0xCD, 0x1A,                                              // INT 0x1A
        0xA3, 0x00, 0x06,                                       // MOV [RESULT], AX
        0x89, 0x0E, 0x02, 0x06,                                 // MOV [RESULT+2], CX
        0xF4,                                                    // HLT
    ]
}

fn assert_result_ah(ram: &[u8; 0xA0000], expected_ah: u8, label: &str) {
    let ax = read_ram_u16(ram, RESULT as usize);
    assert_eq!(
        ax >> 8,
        expected_ah as u16,
        "{label}: AH should be {expected_ah:#04X} (got {:#04X})",
        ax >> 8
    );
}

// ============================================================================
// IVT Vector
// ============================================================================

#[test]
fn int1ah_vector_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    let state = machine.inspection_state();
    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1A);
    assert!(
        segment >= 0xFD80,
        "INT 1Ah segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int1ah_vector_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    let state = machine.inspection_state();
    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1A);
    assert!(
        segment >= 0xFD80,
        "INT 1Ah segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int1ah_vector_vx() {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    let state = machine.inspection_state();
    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1A);
    assert!(
        segment >= 0xFD80,
        "INT 1Ah segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int1ah_vector_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    let state = machine.inspection_state();
    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1A);
    assert!(
        segment >= 0xFD80,
        "INT 1Ah segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

// ============================================================================
// AH=00h CMT No-op
// ============================================================================

#[test]
fn int1ah_cmt_noop_f() {
    let code = make_int1ah_call_store_ax(0x00);
    let (machine, _) = boot_and_run_f(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=00h no-op");
}

#[test]
fn int1ah_cmt_noop_vm() {
    let code = make_int1ah_call_store_ax(0x00);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=00h no-op");
}

#[test]
fn int1ah_cmt_noop_vx() {
    let code = make_int1ah_call_store_ax(0x00);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=00h no-op");
}

#[test]
fn int1ah_cmt_noop_ra() {
    let code = make_int1ah_call_store_ax(0x00);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=00h no-op");
}

// ============================================================================
// AH=01h CMT Motor Off
// ============================================================================

#[test]
fn int1ah_cmt_motor_off_f() {
    let code = make_int1ah_call_store_ax(0x01);
    let (machine, _) = boot_and_run_f(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=01h motor off");
}

#[test]
fn int1ah_cmt_motor_off_vm() {
    let code = make_int1ah_call_store_ax(0x01);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=01h motor off");
}

#[test]
fn int1ah_cmt_motor_off_vx() {
    let code = make_int1ah_call_store_ax(0x01);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=01h motor off");
}

#[test]
fn int1ah_cmt_motor_off_ra() {
    let code = make_int1ah_call_store_ax(0x01);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=01h motor off");
}

// ============================================================================
// AH=02h CMT Motor On (Read)
// ============================================================================

#[test]
fn int1ah_cmt_motor_on_read_f() {
    let code = make_int1ah_call_al_store_ax(0x02, 0x80);
    let (machine, _) = boot_and_run_f(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=02h motor on read");
}

#[test]
fn int1ah_cmt_motor_on_read_vm() {
    let code = make_int1ah_call_al_store_ax(0x02, 0x80);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=02h motor on read");
}

#[test]
fn int1ah_cmt_motor_on_read_vx() {
    let code = make_int1ah_call_al_store_ax(0x02, 0x80);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=02h motor on read");
}

#[test]
fn int1ah_cmt_motor_on_read_ra() {
    let code = make_int1ah_call_al_store_ax(0x02, 0x80);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=02h motor on read");
}

// ============================================================================
// AH=03h CMT Motor On (Write)
// ============================================================================

#[test]
fn int1ah_cmt_motor_on_write_f() {
    let code = make_int1ah_call_al_store_ax(0x03, 0x80);
    let (machine, _) = boot_and_run_f(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=03h motor on write");
}

#[test]
fn int1ah_cmt_motor_on_write_vm() {
    let code = make_int1ah_call_al_store_ax(0x03, 0x80);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=03h motor on write");
}

#[test]
fn int1ah_cmt_motor_on_write_vx() {
    let code = make_int1ah_call_al_store_ax(0x03, 0x80);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=03h motor on write");
}

#[test]
fn int1ah_cmt_motor_on_write_ra() {
    let code = make_int1ah_call_al_store_ax(0x03, 0x80);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=03h motor on write");
}

// ============================================================================
// AH=04h CMT Data Write (Unsupported)
// ============================================================================
// VM/VX return AH=0x00, RA returns AH=0x02.

#[test]
fn int1ah_cmt_data_write_f() {
    let code = make_int1ah_call_al_store_ax(0x04, 0x41);
    let (machine, _) = boot_and_run_f(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=04h data write");
}

#[test]
fn int1ah_cmt_data_write_vm() {
    let code = make_int1ah_call_al_store_ax(0x04, 0x41);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=04h data write");
}

#[test]
fn int1ah_cmt_data_write_vx() {
    let code = make_int1ah_call_al_store_ax(0x04, 0x41);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=04h data write");
}

#[test]
fn int1ah_cmt_data_write_ra() {
    let code = make_int1ah_call_al_store_ax(0x04, 0x41);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x02, "AH=04h data write");
}

// ============================================================================
// AH=05h CMT Data Read (Unsupported)
// ============================================================================
// VM/VX return AH=0x27, RA returns AH=0x02.

#[test]
fn int1ah_cmt_data_read_f() {
    let code = make_int1ah_call_al_store_ax(0x05, 0x00);
    let (machine, _) = boot_and_run_f(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x27, "AH=05h data read");
}

#[test]
fn int1ah_cmt_data_read_vm() {
    let code = make_int1ah_call_al_store_ax(0x05, 0x00);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x27, "AH=05h data read");
}

#[test]
fn int1ah_cmt_data_read_vx() {
    let code = make_int1ah_call_al_store_ax(0x05, 0x00);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x27, "AH=05h data read");
}

#[test]
fn int1ah_cmt_data_read_ra() {
    let code = make_int1ah_call_al_store_ax(0x05, 0x00);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x02, "AH=05h data read");
}

// ============================================================================
// AH=10h Printer Initialize
// ============================================================================
// All targets return AH=0x01 (ready). On real PC-98 hardware, the BUSY#
// line is high when no printer is connected, so the port always reports ready.

#[test]
fn int1ah_printer_init_f() {
    let code = make_int1ah_call_store_ax(0x10);
    let (machine, _) = boot_and_run_f(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "AH=10h init");
}

#[test]
fn int1ah_printer_init_vm() {
    let code = make_int1ah_call_store_ax(0x10);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "AH=10h init");
}

#[test]
fn int1ah_printer_init_vx() {
    let code = make_int1ah_call_store_ax(0x10);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "AH=10h init");
}

#[test]
fn int1ah_printer_init_ra() {
    let code = make_int1ah_call_store_ax(0x10);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "AH=10h init");
}

// ============================================================================
// AH=11h Printer Print Char
// ============================================================================
// Port always reports ready (BUSY# high). AH=0x01 (ok).

#[test]
fn int1ah_printer_print_char_f() {
    let code = make_int1ah_call_al_store_ax(0x11, 0x41);
    let (machine, _) = boot_and_run_f(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "AH=11h print char");
}

#[test]
fn int1ah_printer_print_char_vm() {
    let code = make_int1ah_call_al_store_ax(0x11, 0x41);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "AH=11h print char");
}

#[test]
fn int1ah_printer_print_char_vx() {
    let code = make_int1ah_call_al_store_ax(0x11, 0x41);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "AH=11h print char");
}

#[test]
fn int1ah_printer_print_char_ra() {
    let code = make_int1ah_call_al_store_ax(0x11, 0x41);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "AH=11h print char");
}

// ============================================================================
// AH=12h Printer Status Read
// ============================================================================
// Port always reports ready (BUSY# high). AH=0x01 (ready).

#[test]
fn int1ah_printer_status_f() {
    let code = make_int1ah_call_store_ax(0x12);
    let (machine, _) = boot_and_run_f(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "AH=12h status");
}

#[test]
fn int1ah_printer_status_vm() {
    let code = make_int1ah_call_store_ax(0x12);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "AH=12h status");
}

#[test]
fn int1ah_printer_status_vx() {
    let code = make_int1ah_call_store_ax(0x12);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "AH=12h status");
}

#[test]
fn int1ah_printer_status_ra() {
    let code = make_int1ah_call_store_ax(0x12);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "AH=12h status");
}

// ============================================================================
// AH=30h Printer Print Buffer - Non-zero Count
// ============================================================================
// No printer attached: returns AH=0x00 (not ready).

fn assert_printer_buffer(ram: &[u8; 0xA0000], expected_ah: u8, expected_cx: u16) {
    let ax = read_ram_u16(ram, RESULT as usize);
    let cx = read_ram_u16(ram, RESULT as usize + 2);
    assert_eq!(
        ax >> 8,
        expected_ah as u16,
        "AH should be {expected_ah:#04X} (got {:#04X})",
        ax >> 8
    );
    assert_eq!(cx, expected_cx, "CX should be {expected_cx} (got {cx})");
}

#[test]
fn int1ah_printer_buffer_f() {
    let code = make_int1ah_print_buffer(3);
    let (machine, _) = boot_and_run_f(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=30h buffer (no printer)");
}

#[test]
fn int1ah_printer_buffer_vm() {
    let code = make_int1ah_print_buffer(3);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=30h buffer (no printer)");
}

#[test]
fn int1ah_printer_buffer_vx() {
    let code = make_int1ah_print_buffer(3);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=30h buffer (no printer)");
}

#[test]
fn int1ah_printer_buffer_ra() {
    let code = make_int1ah_print_buffer(3);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "AH=30h buffer (no printer)");
}

// ============================================================================
// AH=30h Printer Print Buffer - Zero Count
// ============================================================================
// No printer: returns AH=0x00, CX=0.

#[test]
fn int1ah_printer_buffer_zero_count_f() {
    let code = make_int1ah_print_buffer(0);
    let (machine, _) = boot_and_run_f(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_buffer(&state.memory.ram, 0x00, 0);
}

#[test]
fn int1ah_printer_buffer_zero_count_vm() {
    let code = make_int1ah_print_buffer(0);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_buffer(&state.memory.ram, 0x00, 0);
}

#[test]
fn int1ah_printer_buffer_zero_count_vx() {
    let code = make_int1ah_print_buffer(0);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_buffer(&state.memory.ram, 0x00, 0);
}

#[test]
fn int1ah_printer_buffer_zero_count_ra() {
    let code = make_int1ah_print_buffer(0);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_buffer(&state.memory.ram, 0x00, 0);
}

// ============================================================================
// Printer with attached output - helpers
// ============================================================================

fn make_temp_printer_file() -> (std::fs::File, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("neetan_printer_test_{}", std::process::id()));
    let file = std::fs::File::create(&path).expect("failed to create temp file");
    (file, path)
}

fn boot_and_run_printer_f(code: &[u8], budget: u64) -> (machine_98::Pc9801F, std::path::PathBuf) {
    let mut machine = create_machine_f();
    let (file, path) = make_temp_printer_file();
    machine.bus.attach_printer(file);
    boot_to_halt!(machine);
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
    machine.run_for(budget);
    (machine, path)
}

fn boot_and_run_printer_vm(code: &[u8], budget: u64) -> (machine_98::Pc9801Vm, std::path::PathBuf) {
    let mut machine = create_machine_vm();
    let (file, path) = make_temp_printer_file();
    machine.bus.attach_printer(file);
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(budget);
    (machine, path)
}

fn boot_and_run_printer_vx(code: &[u8], budget: u64) -> (machine_98::Pc9801Vx, std::path::PathBuf) {
    let mut machine = create_machine_vx();
    let (file, path) = make_temp_printer_file();
    machine.bus.attach_printer(file);
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(budget);
    (machine, path)
}

fn boot_and_run_printer_ra(code: &[u8], budget: u64) -> (machine_98::Pc9801Ra, std::path::PathBuf) {
    let mut machine = create_machine_ra();
    let (file, path) = make_temp_printer_file();
    machine.bus.attach_printer(file);
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(budget);
    (machine, path)
}

// ============================================================================
// AH=10h Printer Initialize - Printer Attached
// ============================================================================

fn assert_printer_init_attached(ram: &[u8; 0xA0000]) {
    let ax = read_ram_u16(ram, RESULT as usize);
    let ah = (ax >> 8) as u8;
    assert_eq!(ah, 0x01, "AH should be 0x01 (ready) (got {ah:#04X})");
}

#[test]
fn int1ah_printer_init_attached_f() {
    let code = make_int1ah_call_store_ax(0x10);
    let (machine, _path) = boot_and_run_printer_f(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_init_attached(&state.memory.ram);
}

#[test]
fn int1ah_printer_init_attached_vm() {
    let code = make_int1ah_call_store_ax(0x10);
    let (machine, _path) = boot_and_run_printer_vm(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_init_attached(&state.memory.ram);
}

#[test]
fn int1ah_printer_init_attached_vx() {
    let code = make_int1ah_call_store_ax(0x10);
    let (machine, _path) = boot_and_run_printer_vx(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_init_attached(&state.memory.ram);
}

#[test]
fn int1ah_printer_init_attached_ra() {
    let code = make_int1ah_call_store_ax(0x10);
    let (machine, _path) = boot_and_run_printer_ra(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_init_attached(&state.memory.ram);
}

// ============================================================================
// AH=11h Printer Print Char - Printer Attached
// ============================================================================

fn assert_printer_print_char_attached(ram: &[u8; 0xA0000]) {
    let ax = read_ram_u16(ram, RESULT as usize);
    let ah = (ax >> 8) as u8;
    assert_eq!(ah, 0x01, "AH should be 0x01 (ok) (got {ah:#04X})");
}

#[test]
fn int1ah_printer_print_char_attached_f() {
    let code = make_int1ah_call_al_store_ax(0x11, 0x41);
    let (machine, _path) = boot_and_run_printer_f(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_print_char_attached(&state.memory.ram);
}

#[test]
fn int1ah_printer_print_char_attached_vm() {
    let code = make_int1ah_call_al_store_ax(0x11, 0x41);
    let (machine, _path) = boot_and_run_printer_vm(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_print_char_attached(&state.memory.ram);
}

#[test]
fn int1ah_printer_print_char_attached_vx() {
    let code = make_int1ah_call_al_store_ax(0x11, 0x41);
    let (machine, _path) = boot_and_run_printer_vx(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_print_char_attached(&state.memory.ram);
}

#[test]
fn int1ah_printer_print_char_attached_ra() {
    let code = make_int1ah_call_al_store_ax(0x11, 0x41);
    let (machine, _path) = boot_and_run_printer_ra(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_print_char_attached(&state.memory.ram);
}

// ============================================================================
// AH=12h Printer Status - Printer Attached
// ============================================================================

fn assert_printer_status_attached(ram: &[u8; 0xA0000]) {
    let ax = read_ram_u16(ram, RESULT as usize);
    let ah = (ax >> 8) as u8;
    assert_eq!(ah, 0x01, "AH should be 0x01 (ready) (got {ah:#04X})");
}

#[test]
fn int1ah_printer_status_attached_f() {
    let code = make_int1ah_call_store_ax(0x12);
    let (machine, _path) = boot_and_run_printer_f(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_status_attached(&state.memory.ram);
}

#[test]
fn int1ah_printer_status_attached_vm() {
    let code = make_int1ah_call_store_ax(0x12);
    let (machine, _path) = boot_and_run_printer_vm(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_status_attached(&state.memory.ram);
}

#[test]
fn int1ah_printer_status_attached_vx() {
    let code = make_int1ah_call_store_ax(0x12);
    let (machine, _path) = boot_and_run_printer_vx(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_status_attached(&state.memory.ram);
}

#[test]
fn int1ah_printer_status_attached_ra() {
    let code = make_int1ah_call_store_ax(0x12);
    let (machine, _path) = boot_and_run_printer_ra(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_status_attached(&state.memory.ram);
}

// ============================================================================
// AH=30h Printer Print Buffer - Printer Attached
// ============================================================================

#[test]
fn int1ah_printer_buffer_attached_f() {
    let code = make_int1ah_print_buffer(3);
    let (machine, _path) = boot_and_run_printer_f(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_buffer(&state.memory.ram, 0x00, 0);
}

#[test]
fn int1ah_printer_buffer_attached_vm() {
    let code = make_int1ah_print_buffer(3);
    let (machine, _path) = boot_and_run_printer_vm(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_buffer(&state.memory.ram, 0x00, 0);
}

#[test]
fn int1ah_printer_buffer_attached_vx() {
    let code = make_int1ah_print_buffer(3);
    let (machine, _path) = boot_and_run_printer_vx(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_buffer(&state.memory.ram, 0x00, 0);
}

#[test]
fn int1ah_printer_buffer_attached_ra() {
    let code = make_int1ah_print_buffer(3);
    let (machine, _path) = boot_and_run_printer_ra(&code, INT1AH_BUDGET);
    let state = machine.inspection_state();
    assert_printer_buffer(&state.memory.ram, 0x00, 0);
}
