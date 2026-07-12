use common::{Bus, Cpu};
use device::upd7220_gdc::DISPLAY_MODE_GRAPHICS;

use super::{
    KB_COUNT, KB_HEAD, KB_TAIL, TEST_CODE, boot_inject_run_f, boot_inject_run_pc9821as,
    boot_inject_run_ra, boot_inject_run_vm, boot_inject_run_vx, create_machine_f,
    create_machine_ra, create_machine_vm, create_machine_vx, read_ivt_vector, read_ram_u16,
    write_bytes,
};

const RESULT: u32 = 0x0600;
const DATA_TABLE: u32 = 0x0700;
const INT18H_BUDGET: u64 = 500_000;

// Pattern A: AH-only call, assert on device state.
fn make_int18h_call(ah: u8) -> Vec<u8> {
    vec![0xB4, ah, 0xCD, 0x18, 0xF4]
}

// Pattern B: AH+AL call (e.g. cursor blink mode).
fn make_int18h_call_al(ah: u8, al: u8) -> Vec<u8> {
    vec![0xB4, ah, 0xB0, al, 0xCD, 0x18, 0xF4]
}

// Pattern C: AH+DX call (cursor position, VRAM init).
fn make_int18h_call_dx(ah: u8, dx: u16) -> Vec<u8> {
    vec![
        0xB4,
        ah,
        0xBA,
        (dx & 0xFF) as u8,
        (dx >> 8) as u8,
        0xCD,
        0x18,
        0xF4,
    ]
}

// Pattern D: AH+DS:BX call (palette, border).
#[rustfmt::skip]
fn make_int18h_call_ds_bx(ah: u8, bx: u16) -> Vec<u8> {
    vec![
        0x31, 0xC0,                                     // XOR AX, AX
        0x8E, 0xD8,                                     // MOV DS, AX
        0xBB, (bx & 0xFF) as u8, (bx >> 8) as u8,      // MOV BX, bx
        0xB4, ah,                                        // MOV AH, ah
        0xCD, 0x18,                                      // INT 18h
        0xF4,                                            // HLT
    ]
}

// Two sequential AH-only INT 18h calls.
fn make_int18h_two_calls(ah1: u8, ah2: u8) -> Vec<u8> {
    vec![0xB4, ah1, 0xCD, 0x18, 0xB4, ah2, 0xCD, 0x18, 0xF4]
}

// AH-only call, store AX to RESULT.
#[rustfmt::skip]
fn make_int18h_call_store(ah: u8) -> Vec<u8> {
    vec![
        0xB4, ah,               // MOV AH, ah
        0xCD, 0x18,             // INT 18h
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0xF4,                   // HLT
    ]
}

// AH-only call, store AX and BX to RESULT.
#[rustfmt::skip]
fn make_int18h_call_store_ax_bx(ah: u8) -> Vec<u8> {
    vec![
        0xB4, ah,               // MOV AH, ah
        0xCD, 0x18,             // INT 18h
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0x89, 0x1E, 0x02, 0x06, // MOV [RESULT+2], BX
        0xF4,                   // HLT
    ]
}

// Pattern E: Wait for N IRQs, then call INT 18h AH=ah/AL=al, store AX+BX to RESULT.
#[rustfmt::skip]
fn make_kb_int18h(ah: u8, al: u8, num_irqs: usize) -> Vec<u8> {
    let mut code = vec![0xFB]; // STI
    code.extend(std::iter::repeat_n(0xF4_u8, num_irqs)); // HLTs
    code.extend_from_slice(&[
        0xB4, ah,               // MOV AH, ah
        0xB0, al,               // MOV AL, al
        0xCD, 0x18,             // INT 18h
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0x89, 0x1E, 0x02, 0x06, // MOV [RESULT+2], BX
        0xF4,                   // HLT
    ]);
    code
}

#[rustfmt::skip]
fn make_dosshell_chained_int09h_program() -> Vec<u8> {
    vec![
        0xFB,                   // STI
        0xF4,                   // HLT (wait for key IRQ)
        0xB4, 0x02,             // MOV AH,02h
        0xCD, 0x18,             // INT 18h
        0xB4, 0x01,             // MOV AH,01h
        0xCD, 0x18,             // INT 18h
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0x89, 0x1E, 0x02, 0x06, // MOV [RESULT+2], BX
        0xF4,                   // HLT
    ]
}

#[rustfmt::skip]
fn make_dosshell_int09h_prehandler(original_segment: u16, original_offset: u16) -> Vec<u8> {
    vec![
        0x50,                    // PUSH AX
        0x52,                    // PUSH DX
        0xBA, 0x43, 0x00,        // MOV DX,0043h
        0xEC,                    // IN AL,DX
        0xBA, 0x41, 0x00,        // MOV DX,0041h
        0xEC,                    // IN AL,DX
        0x5A,                    // POP DX
        0x58,                    // POP AX
        0xEA,                    // JMP FAR original INT 09h
        (original_offset & 0xFF) as u8,
        (original_offset >> 8) as u8,
        (original_segment & 0xFF) as u8,
        (original_segment >> 8) as u8,
    ]
}

// Pattern F: AH+CH call (display area set, draw mode set).
#[rustfmt::skip]
fn make_int18h_call_ch(ah: u8, ch: u8) -> Vec<u8> {
    vec![
        0xB4, ah,               // MOV AH, ah
        0xB5, ch,               // MOV CH, ch
        0xCD, 0x18,             // INT 18h
        0xF4,                   // HLT
    ]
}

// Pattern G: AH+BX+CX+DX call (font read, user char define, multi display area).
#[rustfmt::skip]
fn make_int18h_call_bx_cx_dx(ah: u8, bx: u16, cx: u16, dx: u16) -> Vec<u8> {
    vec![
        0xBB, (bx & 0xFF) as u8, (bx >> 8) as u8,     // MOV BX, bx
        0xB9, (cx & 0xFF) as u8, (cx >> 8) as u8,      // MOV CX, cx
        0xBA, (dx & 0xFF) as u8, (dx >> 8) as u8,      // MOV DX, dx
        0xB4, ah,                                        // MOV AH, ah
        0xCD, 0x18,                                      // INT 18h
        0xF4,                                            // HLT
    ]
}

// Drawing test code: AH=40h (graphics start) -> AH=42h CH=0xC0 (640x400) -> draw call with DS:BX.
#[rustfmt::skip]
fn make_draw_test_code(draw_ah: u8) -> Vec<u8> {
    let dt_lo = (DATA_TABLE & 0xFF) as u8;
    let dt_hi = ((DATA_TABLE >> 8) & 0xFF) as u8;
    vec![
        0xB4, 0x40,             // MOV AH, 0x40 (graphics display start)
        0xCD, 0x18,             // INT 18h
        0xB4, 0x42,             // MOV AH, 0x42 (display area set)
        0xB5, 0xC0,             // MOV CH, 0xC0 (640x400)
        0xCD, 0x18,             // INT 18h
        0x31, 0xC0,             // XOR AX, AX
        0x8E, 0xD8,             // MOV DS, AX
        0xBB, dt_lo, dt_hi,     // MOV BX, DATA_TABLE
        0xB4, draw_ah,          // MOV AH, draw_ah
        0xCD, 0x18,             // INT 18h
        0xF4,                   // HLT
    ]
}

const DRAW_BUDGET: u64 = 1_000_000;

fn run_draw_test_f(ucw_data: &[u8], draw_ah: u8) -> machine::MachineState {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, DATA_TABLE, ucw_data);
    let code = make_draw_test_code(draw_ah);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    machine.save_state()
}

fn run_draw_test_vm(ucw_data: &[u8], draw_ah: u8) -> machine::MachineState {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, DATA_TABLE, ucw_data);
    let code = make_draw_test_code(draw_ah);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    machine.save_state()
}

fn run_draw_test_vx(ucw_data: &[u8], draw_ah: u8) -> machine::MachineState {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, DATA_TABLE, ucw_data);
    let code = make_draw_test_code(draw_ah);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    machine.save_state()
}

fn run_draw_test_ra(ucw_data: &[u8], draw_ah: u8) -> machine::MachineState {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, DATA_TABLE, ucw_data);
    let code = make_draw_test_code(draw_ah);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    machine.save_state()
}

// ============================================================================
// §9 INT 18h - Vector Setup
// ============================================================================

#[test]
fn int18h_vector_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x18);
    assert!(
        segment >= 0xFD80,
        "INT 18h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int18h_vector_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x18);
    assert!(
        segment >= 0xFD80,
        "INT 18h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int18h_vector_vx() {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x18);
    assert!(
        segment >= 0xFD80,
        "INT 18h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int18h_vector_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x18);
    assert!(
        segment >= 0xFD80,
        "INT 18h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

// ============================================================================
// §9.1 AH=00h - Key Read
// ============================================================================

#[test]
fn key_read_f() {
    let code = make_kb_int18h(0x00, 0x00, 1);
    let mut machine = boot_inject_run_f(&[0x1C], &code, INT18H_BUDGET);
    let ax = machine.bus.read_word(RESULT);
    assert_ne!(
        ax, 0x0000,
        "AH=00h should return non-zero key code for Enter"
    );
}

#[test]
fn key_read_vm() {
    let code = make_kb_int18h(0x00, 0x00, 1);
    let mut machine = boot_inject_run_vm(&[0x1C], &code, INT18H_BUDGET);
    let ax = machine.bus.read_word(RESULT);
    assert_ne!(
        ax, 0x0000,
        "AH=00h should return non-zero key code for Enter"
    );
}

#[test]
fn key_read_vx() {
    let code = make_kb_int18h(0x00, 0x00, 1);
    let mut machine = boot_inject_run_vx(&[0x1C], &code, INT18H_BUDGET);
    let ax = machine.bus.read_word(RESULT);
    assert_ne!(
        ax, 0x0000,
        "AH=00h should return non-zero key code for Enter"
    );
}

#[test]
fn key_read_ra() {
    let code = make_kb_int18h(0x00, 0x00, 1);
    let mut machine = boot_inject_run_ra(&[0x1C], &code, INT18H_BUDGET);
    let ax = machine.bus.read_word(RESULT);
    assert_ne!(
        ax, 0x0000,
        "AH=00h should return non-zero key code for Enter"
    );
}

// ============================================================================
// §9.1 AH=01h - Buffer Sense (Empty)
// ============================================================================

#[test]
fn buffer_sense_empty_f() {
    let code = make_int18h_call_store_ax_bx(0x01);
    let mut machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let bh = machine.bus.read_byte(RESULT + 3);
    assert_eq!(
        bh, 0x00,
        "AH=01h should return BH=0x00 when buffer is empty"
    );
}

#[test]
fn buffer_sense_empty_vm() {
    let code = make_int18h_call_store_ax_bx(0x01);
    let mut machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let bh = machine.bus.read_byte(RESULT + 3);
    assert_eq!(
        bh, 0x00,
        "AH=01h should return BH=0x00 when buffer is empty"
    );
}

#[test]
fn buffer_sense_empty_vx() {
    let code = make_int18h_call_store_ax_bx(0x01);
    let mut machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let bh = machine.bus.read_byte(RESULT + 3);
    assert_eq!(
        bh, 0x00,
        "AH=01h should return BH=0x00 when buffer is empty"
    );
}

#[test]
fn buffer_sense_empty_ra() {
    let code = make_int18h_call_store_ax_bx(0x01);
    let mut machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let bh = machine.bus.read_byte(RESULT + 3);
    assert_eq!(
        bh, 0x00,
        "AH=01h should return BH=0x00 when buffer is empty"
    );
}

// ============================================================================
// §9.1 AH=01h - Buffer Sense (Data Available)
// ============================================================================

#[test]
fn buffer_sense_data_f() {
    let code = make_kb_int18h(0x01, 0x00, 1);
    let mut machine = boot_inject_run_f(&[0x1C], &code, INT18H_BUDGET);
    let ax = machine.bus.read_word(RESULT);
    let bh = machine.bus.read_byte(RESULT + 3);
    let state = machine.save_state();

    assert_eq!(bh, 0x01, "AH=01h should return BH=0x01 when data available");
    assert_ne!(ax, 0x0000, "AH=01h should return key code in AX");
    assert_eq!(
        state.memory.ram[KB_COUNT], 1,
        "AH=01h should not remove key from buffer"
    );
}

#[test]
fn buffer_sense_data_vm() {
    let code = make_kb_int18h(0x01, 0x00, 1);
    let mut machine = boot_inject_run_vm(&[0x1C], &code, INT18H_BUDGET);
    let ax = machine.bus.read_word(RESULT);
    let bh = machine.bus.read_byte(RESULT + 3);
    let state = machine.save_state();

    assert_eq!(bh, 0x01, "AH=01h should return BH=0x01 when data available");
    assert_ne!(ax, 0x0000, "AH=01h should return key code in AX");
    assert_eq!(
        state.memory.ram[KB_COUNT], 1,
        "AH=01h should not remove key from buffer"
    );
}

#[test]
fn buffer_sense_data_vx() {
    let code = make_kb_int18h(0x01, 0x00, 1);
    let mut machine = boot_inject_run_vx(&[0x1C], &code, INT18H_BUDGET);
    let ax = machine.bus.read_word(RESULT);
    let bh = machine.bus.read_byte(RESULT + 3);
    let state = machine.save_state();

    assert_eq!(bh, 0x01, "AH=01h should return BH=0x01 when data available");
    assert_ne!(ax, 0x0000, "AH=01h should return key code in AX");
    assert_eq!(
        state.memory.ram[KB_COUNT], 1,
        "AH=01h should not remove key from buffer"
    );
}

#[test]
fn buffer_sense_data_ra() {
    let code = make_kb_int18h(0x01, 0x00, 1);
    let mut machine = boot_inject_run_ra(&[0x1C], &code, INT18H_BUDGET);
    let ax = machine.bus.read_word(RESULT);
    let bh = machine.bus.read_byte(RESULT + 3);
    let state = machine.save_state();

    assert_eq!(bh, 0x01, "AH=01h should return BH=0x01 when data available");
    assert_ne!(ax, 0x0000, "AH=01h should return key code in AX");
    assert_eq!(
        state.memory.ram[KB_COUNT], 1,
        "AH=01h should not remove key from buffer"
    );
}

#[test]
fn buffer_sense_after_chained_int09h_prehandler_vm() {
    const HANDLER: u32 = DATA_TABLE;

    let mut machine = create_machine_vm();
    boot_to_halt!(machine);

    let state = machine.save_state();
    let (original_segment, original_offset) = read_ivt_vector(&state.memory.ram, 0x09);
    let handler = make_dosshell_int09h_prehandler(original_segment, original_offset);
    write_bytes(&mut machine.bus, HANDLER, &handler);
    write_bytes(
        &mut machine.bus,
        0x09 * 4,
        &[
            (HANDLER & 0xFF) as u8,
            ((HANDLER >> 8) & 0xFF) as u8,
            0x00,
            0x00,
        ],
    );

    let code = make_dosshell_chained_int09h_program();
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.bus.push_keyboard_scancode(0x4B);
    machine.cpu.load_state(&{
        let mut state = cpu::V30State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        state.set_sp(0x4000);
        state
    });

    machine.run_for(INT18H_BUDGET);

    assert!(machine.cpu.halted(), "guest program should halt");
    let ax = machine.bus.read_word(RESULT);
    let bh = machine.bus.read_byte(RESULT + 3);
    let state = machine.save_state();

    assert_eq!(
        bh, 0x01,
        "AH=01h should see the cursor key buffered by the chained INT 09h path"
    );
    assert_ne!(ax, 0x0000, "AH=01h should return a buffered key code");
    assert_eq!(
        state.memory.ram[KB_COUNT], 1,
        "AH=01h should not consume the buffered key"
    );
    assert!(
        !state.keyboard.rx_ready && state.keyboard.rx_fifo.is_empty(),
        "the chained INT 09h path should not leave the controller byte pending"
    );
}

// ============================================================================
// §9.1 AH=02h - Shift Status
// ============================================================================

#[test]
fn shift_status_f() {
    let code = make_kb_int18h(0x02, 0x00, 1);
    let mut machine = boot_inject_run_f(&[0x70], &code, INT18H_BUDGET);
    let al = machine.bus.read_byte(RESULT);
    assert_ne!(al & 0x01, 0, "AH=02h should return SHIFT bit 0 set in AL");
}

#[test]
fn shift_status_vm() {
    let code = make_kb_int18h(0x02, 0x00, 1);
    let mut machine = boot_inject_run_vm(&[0x70], &code, INT18H_BUDGET);
    let al = machine.bus.read_byte(RESULT);
    assert_ne!(al & 0x01, 0, "AH=02h should return SHIFT bit 0 set in AL");
}

#[test]
fn shift_status_vx() {
    let code = make_kb_int18h(0x02, 0x00, 1);
    let mut machine = boot_inject_run_vx(&[0x70], &code, INT18H_BUDGET);
    let al = machine.bus.read_byte(RESULT);
    assert_ne!(al & 0x01, 0, "AH=02h should return SHIFT bit 0 set in AL");
}

#[test]
fn shift_status_ra() {
    let code = make_kb_int18h(0x02, 0x00, 1);
    let mut machine = boot_inject_run_ra(&[0x70], &code, INT18H_BUDGET);
    let al = machine.bus.read_byte(RESULT);
    assert_ne!(al & 0x01, 0, "AH=02h should return SHIFT bit 0 set in AL");
}

// ============================================================================
// §9.1 AH=03h - Keyboard Init
// ============================================================================

#[test]
fn kb_init_f() {
    // Inject a key, then call AH=03h to reset the buffer.
    #[rustfmt::skip]
    let code = vec![
        0xFB,                   // STI
        0xF4,                   // HLT (wait for key IRQ)
        0xB4, 0x03,             // MOV AH, 0x03
        0xCD, 0x18,             // INT 18h
        0xF4,                   // HLT
    ];
    let machine = boot_inject_run_f(&[0x1C], &code, INT18H_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 0,
        "AH=03h should clear KB_COUNT"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_HEAD),
        0x0502,
        "AH=03h should reset KB_HEAD to 0x0502"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0502,
        "AH=03h should reset KB_TAIL to 0x0502"
    );
}

#[test]
fn kb_init_vm() {
    // Inject a key, then call AH=03h to reset the buffer.
    #[rustfmt::skip]
    let code = vec![
        0xFB,                   // STI
        0xF4,                   // HLT (wait for key IRQ)
        0xB4, 0x03,             // MOV AH, 0x03
        0xCD, 0x18,             // INT 18h
        0xF4,                   // HLT
    ];
    let machine = boot_inject_run_vm(&[0x1C], &code, INT18H_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 0,
        "AH=03h should clear KB_COUNT"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_HEAD),
        0x0502,
        "AH=03h should reset KB_HEAD to 0x0502"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0502,
        "AH=03h should reset KB_TAIL to 0x0502"
    );
}

#[test]
fn kb_init_vx() {
    #[rustfmt::skip]
    let code = vec![
        0xFB, 0xF4,
        0xB4, 0x03, 0xCD, 0x18,
        0xF4,
    ];
    let machine = boot_inject_run_vx(&[0x1C], &code, INT18H_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 0,
        "AH=03h should clear KB_COUNT"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_HEAD),
        0x0502,
        "AH=03h should reset KB_HEAD to 0x0502"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0502,
        "AH=03h should reset KB_TAIL to 0x0502"
    );
}

#[test]
fn kb_init_ra() {
    #[rustfmt::skip]
    let code = vec![
        0xFB, 0xF4,
        0xB4, 0x03, 0xCD, 0x18,
        0xF4,
    ];
    let machine = boot_inject_run_ra(&[0x1C], &code, INT18H_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 0,
        "AH=03h should clear KB_COUNT"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_HEAD),
        0x0502,
        "AH=03h should reset KB_HEAD to 0x0502"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0502,
        "AH=03h should reset KB_TAIL to 0x0502"
    );
}

#[test]
fn kb_init_clears_key_status_and_sets_table_pointers_f() {
    #[rustfmt::skip]
    let code = vec![
        0xFB, 0xF4,
        0xB4, 0x03, 0xCD, 0x18,
        0xF4,
    ];
    let machine = boot_inject_run_f(&[0x1C], &code, INT18H_BUDGET);
    let state = machine.save_state();

    // Key status area (0x0529..0x053B) should be cleared.
    for addr in 0x0529..0x053B {
        assert_eq!(
            state.memory.ram[addr], 0,
            "AH=03h should clear key status byte at {addr:#06X}"
        );
    }
    // KB_SHIFT_TBL at 0x0522.
    assert_eq!(
        read_ram_u16(&state.memory.ram, 0x0522),
        0x0A58,
        "AH=03h should set KB_SHIFT_TBL to 0x0A58"
    );
    // KB_CODE_OFF at 0x05C6.
    assert_eq!(
        read_ram_u16(&state.memory.ram, 0x05C6),
        0x0A58,
        "AH=03h should set KB_CODE_OFF to 0x0A58"
    );
    // KB_CODE_SEG at 0x05C8.
    assert_eq!(
        read_ram_u16(&state.memory.ram, 0x05C8),
        0xFD80,
        "AH=03h should set KB_CODE_SEG to 0xFD80"
    );
}

#[test]
fn kb_init_clears_key_status_and_sets_table_pointers_vm() {
    #[rustfmt::skip]
    let code = vec![
        0xFB, 0xF4,
        0xB4, 0x03, 0xCD, 0x18,
        0xF4,
    ];
    let machine = boot_inject_run_vm(&[0x1C], &code, INT18H_BUDGET);
    let state = machine.save_state();

    // Key status area (0x0529..0x053B) should be cleared.
    for addr in 0x0529..0x053B {
        assert_eq!(
            state.memory.ram[addr], 0,
            "AH=03h should clear key status byte at {addr:#06X}"
        );
    }
    // KB_SHIFT_TBL at 0x0522.
    assert_eq!(
        read_ram_u16(&state.memory.ram, 0x0522),
        0x0B28,
        "AH=03h should set KB_SHIFT_TBL to 0x0B28"
    );
    // KB_CODE_OFF at 0x05C6.
    assert_eq!(
        read_ram_u16(&state.memory.ram, 0x05C6),
        0x0B28,
        "AH=03h should set KB_CODE_OFF to 0x0B28"
    );
    // KB_CODE_SEG at 0x05C8.
    assert_eq!(
        read_ram_u16(&state.memory.ram, 0x05C8),
        0xFD80,
        "AH=03h should set KB_CODE_SEG to 0xFD80"
    );
}

// ============================================================================
// §9.1 AH=04h - Key State Sense
// ============================================================================

// Inject Enter (0x1C): group = 0x1C >> 3 = 3, bit = 1 << (0x1C & 7) = 0x10.
// Query group 3 -> AH should have bit 4 set.

#[test]
fn key_state_sense_f() {
    let code = make_kb_int18h(0x04, 0x03, 1);
    let mut machine = boot_inject_run_f(&[0x1C], &code, INT18H_BUDGET);
    let ah = machine.bus.read_byte(RESULT + 1);
    assert_ne!(
        ah & 0x10,
        0,
        "AH=04h group 3 should have bit 4 set for Enter key"
    );
}

#[test]
fn key_state_sense_vm() {
    let code = make_kb_int18h(0x04, 0x03, 1);
    let mut machine = boot_inject_run_vm(&[0x1C], &code, INT18H_BUDGET);
    let ah = machine.bus.read_byte(RESULT + 1);
    assert_ne!(
        ah & 0x10,
        0,
        "AH=04h group 3 should have bit 4 set for Enter key"
    );
}

#[test]
fn key_state_sense_vx() {
    let code = make_kb_int18h(0x04, 0x03, 1);
    let mut machine = boot_inject_run_vx(&[0x1C], &code, INT18H_BUDGET);
    let ah = machine.bus.read_byte(RESULT + 1);
    assert_ne!(
        ah & 0x10,
        0,
        "AH=04h group 3 should have bit 4 set for Enter key"
    );
}

#[test]
fn key_state_sense_ra() {
    let code = make_kb_int18h(0x04, 0x03, 1);
    let mut machine = boot_inject_run_ra(&[0x1C], &code, INT18H_BUDGET);
    let ah = machine.bus.read_byte(RESULT + 1);
    assert_ne!(
        ah & 0x10,
        0,
        "AH=04h group 3 should have bit 4 set for Enter key"
    );
}

// ============================================================================
// §9.1 AH=05h - Key Code Read (Empty)
// ============================================================================

#[test]
fn key_code_read_empty_f() {
    let code = make_int18h_call_store_ax_bx(0x05);
    let mut machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let bh = machine.bus.read_byte(RESULT + 3);
    assert_eq!(
        bh, 0x00,
        "AH=05h should return BH=0x00 when buffer is empty"
    );
}

#[test]
fn key_code_read_empty_vm() {
    let code = make_int18h_call_store_ax_bx(0x05);
    let mut machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let bh = machine.bus.read_byte(RESULT + 3);
    assert_eq!(
        bh, 0x00,
        "AH=05h should return BH=0x00 when buffer is empty"
    );
}

#[test]
fn key_code_read_empty_vx() {
    let code = make_int18h_call_store_ax_bx(0x05);
    let mut machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let bh = machine.bus.read_byte(RESULT + 3);
    assert_eq!(
        bh, 0x00,
        "AH=05h should return BH=0x00 when buffer is empty"
    );
}

#[test]
fn key_code_read_empty_ra() {
    let code = make_int18h_call_store_ax_bx(0x05);
    let mut machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let bh = machine.bus.read_byte(RESULT + 3);
    assert_eq!(
        bh, 0x00,
        "AH=05h should return BH=0x00 when buffer is empty"
    );
}

// ============================================================================
// §9.1 AH=05h - Key Code Read (Data Available)
// ============================================================================

#[test]
fn key_code_read_data_f() {
    let code = make_kb_int18h(0x05, 0x00, 1);
    let mut machine = boot_inject_run_f(&[0x1C], &code, INT18H_BUDGET);
    let ax = machine.bus.read_word(RESULT);
    let bh = machine.bus.read_byte(RESULT + 3);
    let state = machine.save_state();

    assert_eq!(bh, 0x01, "AH=05h should return BH=0x01 when data available");
    assert_ne!(ax, 0x0000, "AH=05h should return key code in AX");
    assert_eq!(
        state.memory.ram[KB_COUNT], 0,
        "AH=05h should remove key from buffer"
    );
}

#[test]
fn key_code_read_data_vm() {
    let code = make_kb_int18h(0x05, 0x00, 1);
    let mut machine = boot_inject_run_vm(&[0x1C], &code, INT18H_BUDGET);
    let ax = machine.bus.read_word(RESULT);
    let bh = machine.bus.read_byte(RESULT + 3);
    let state = machine.save_state();

    assert_eq!(bh, 0x01, "AH=05h should return BH=0x01 when data available");
    assert_ne!(ax, 0x0000, "AH=05h should return key code in AX");
    assert_eq!(
        state.memory.ram[KB_COUNT], 0,
        "AH=05h should remove key from buffer"
    );
}

#[test]
fn key_code_read_data_vx() {
    let code = make_kb_int18h(0x05, 0x00, 1);
    let mut machine = boot_inject_run_vx(&[0x1C], &code, INT18H_BUDGET);
    let ax = machine.bus.read_word(RESULT);
    let bh = machine.bus.read_byte(RESULT + 3);
    let state = machine.save_state();

    assert_eq!(bh, 0x01, "AH=05h should return BH=0x01 when data available");
    assert_ne!(ax, 0x0000, "AH=05h should return key code in AX");
    assert_eq!(
        state.memory.ram[KB_COUNT], 0,
        "AH=05h should remove key from buffer"
    );
}

#[test]
fn key_code_read_data_ra() {
    let code = make_kb_int18h(0x05, 0x00, 1);
    let mut machine = boot_inject_run_ra(&[0x1C], &code, INT18H_BUDGET);
    let ax = machine.bus.read_word(RESULT);
    let bh = machine.bus.read_byte(RESULT + 3);
    let state = machine.save_state();

    assert_eq!(bh, 0x01, "AH=05h should return BH=0x01 when data available");
    assert_ne!(ax, 0x0000, "AH=05h should return key code in AX");
    assert_eq!(
        state.memory.ram[KB_COUNT], 0,
        "AH=05h should remove key from buffer"
    );
}

// ============================================================================
// §9.2 AH=0Bh - CRT Mode Sense
// ============================================================================

#[test]
fn crt_mode_sense_f() {
    let code = make_int18h_call_store(0x0B);
    let mut machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let al = machine.bus.read_byte(RESULT);
    let state = machine.save_state();
    let expected = state.memory.ram[0x053C];
    assert_eq!(
        al, expected,
        "AH=0Bh should return CRT_STS_FLAG from 0x053C"
    );
}

#[test]
fn crt_mode_sense_vm() {
    let code = make_int18h_call_store(0x0B);
    let mut machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let al = machine.bus.read_byte(RESULT);
    let state = machine.save_state();
    let expected = state.memory.ram[0x053C];
    assert_eq!(
        al, expected,
        "AH=0Bh should return CRT_STS_FLAG from 0x053C"
    );
}

#[test]
fn crt_mode_sense_vx() {
    let code = make_int18h_call_store(0x0B);
    let mut machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let al = machine.bus.read_byte(RESULT);
    let state = machine.save_state();
    let expected = state.memory.ram[0x053C];
    assert_eq!(
        al, expected,
        "AH=0Bh should return CRT_STS_FLAG from 0x053C"
    );
}

#[test]
fn crt_mode_sense_ra() {
    let code = make_int18h_call_store(0x0B);
    let mut machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let al = machine.bus.read_byte(RESULT);
    let state = machine.save_state();
    let expected = state.memory.ram[0x053C];
    assert_eq!(
        al, expected,
        "AH=0Bh should return CRT_STS_FLAG from 0x053C"
    );
}

// ============================================================================
// §9.2 AH=0Dh - Text Display Stop
// ============================================================================

#[test]
fn text_display_stop_f() {
    let code = make_int18h_call(0x0D);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.display_enabled,
        "AH=0Dh should disable text display"
    );
}

#[test]
fn text_display_stop_vm() {
    let code = make_int18h_call(0x0D);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.display_enabled,
        "AH=0Dh should disable text display"
    );
}

#[test]
fn text_display_stop_vx() {
    let code = make_int18h_call(0x0D);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.display_enabled,
        "AH=0Dh should disable text display"
    );
}

#[test]
fn text_display_stop_ra() {
    let code = make_int18h_call(0x0D);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.display_enabled,
        "AH=0Dh should disable text display"
    );
}

// ============================================================================
// §9.2 AH=0Ch - Text Display Start
// ============================================================================

#[test]
fn text_display_start_f() {
    let code = make_int18h_two_calls(0x0D, 0x0C);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_master.display_enabled,
        "AH=0Ch should enable text display"
    );
}

#[test]
fn text_display_start_vm() {
    let code = make_int18h_two_calls(0x0D, 0x0C);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_master.display_enabled,
        "AH=0Ch should enable text display"
    );
}

#[test]
fn text_display_start_vx() {
    let code = make_int18h_two_calls(0x0D, 0x0C);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_master.display_enabled,
        "AH=0Ch should enable text display"
    );
}

#[test]
fn text_display_start_ra() {
    let code = make_int18h_two_calls(0x0D, 0x0C);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_master.display_enabled,
        "AH=0Ch should enable text display"
    );
}

// ============================================================================
// §9.2 AH=10h - Cursor Blink (Blinking)
// ============================================================================

#[test]
fn cursor_blink_blinking_f() {
    let code = make_int18h_call_al(0x10, 0x00);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.cursor_blink,
        "AH=10h AL=0 should set cursor to blinking (cursor_blink=false)"
    );
}

#[test]
fn cursor_blink_blinking_vm() {
    let code = make_int18h_call_al(0x10, 0x00);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.cursor_blink,
        "AH=10h AL=0 should set cursor to blinking (cursor_blink=false)"
    );
}

#[test]
fn cursor_blink_blinking_vx() {
    let code = make_int18h_call_al(0x10, 0x00);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.cursor_blink,
        "AH=10h AL=0 should set cursor to blinking (cursor_blink=false)"
    );
}

#[test]
fn cursor_blink_blinking_ra() {
    let code = make_int18h_call_al(0x10, 0x00);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.cursor_blink,
        "AH=10h AL=0 should set cursor to blinking (cursor_blink=false)"
    );
}

// ============================================================================
// §9.2 AH=10h - Cursor Blink (Steady)
// ============================================================================

#[test]
fn cursor_blink_steady_f() {
    let code = make_int18h_call_al(0x10, 0x01);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_master.cursor_blink,
        "AH=10h AL=1 should set cursor to steady (cursor_blink=true)"
    );
}

#[test]
fn cursor_blink_steady_vm() {
    let code = make_int18h_call_al(0x10, 0x01);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_master.cursor_blink,
        "AH=10h AL=1 should set cursor to steady (cursor_blink=true)"
    );
}

#[test]
fn cursor_blink_steady_vx() {
    let code = make_int18h_call_al(0x10, 0x01);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_master.cursor_blink,
        "AH=10h AL=1 should set cursor to steady (cursor_blink=true)"
    );
}

#[test]
fn cursor_blink_steady_ra() {
    let code = make_int18h_call_al(0x10, 0x01);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_master.cursor_blink,
        "AH=10h AL=1 should set cursor to steady (cursor_blink=true)"
    );
}

// ============================================================================
// §9.2 AH=12h - Cursor Display Stop
// ============================================================================

#[test]
fn cursor_display_stop_f() {
    let code = make_int18h_call(0x12);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.cursor_display,
        "AH=12h should hide cursor"
    );
}

#[test]
fn cursor_display_stop_vm() {
    let code = make_int18h_call(0x12);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.cursor_display,
        "AH=12h should hide cursor"
    );
}

#[test]
fn cursor_display_stop_vx() {
    let code = make_int18h_call(0x12);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.cursor_display,
        "AH=12h should hide cursor"
    );
}

#[test]
fn cursor_display_stop_ra() {
    let code = make_int18h_call(0x12);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.cursor_display,
        "AH=12h should hide cursor"
    );
}

// ============================================================================
// §9.2 AH=11h - Cursor Display Start
// ============================================================================

#[test]
fn cursor_display_start_f() {
    let code = make_int18h_two_calls(0x12, 0x11);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(state.gdc_master.cursor_display, "AH=11h should show cursor");
}

#[test]
fn cursor_display_start_vm() {
    let code = make_int18h_two_calls(0x12, 0x11);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(state.gdc_master.cursor_display, "AH=11h should show cursor");
}

#[test]
fn cursor_display_start_vx() {
    let code = make_int18h_two_calls(0x12, 0x11);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(state.gdc_master.cursor_display, "AH=11h should show cursor");
}

#[test]
fn cursor_display_start_ra() {
    let code = make_int18h_two_calls(0x12, 0x11);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(state.gdc_master.cursor_display, "AH=11h should show cursor");
}

// ============================================================================
// §9.2 AH=13h - Cursor Position Set
// ============================================================================

#[test]
fn cursor_position_set_f() {
    let code = make_int18h_call_dx(0x13, 0x0050);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_master.ead, 0x0028,
        "AH=13h DX=0x0050 should set GDC master EAD to 0x0028 (word address = DX/2)"
    );
}

#[test]
fn cursor_position_set_vm() {
    let code = make_int18h_call_dx(0x13, 0x0050);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_master.ead, 0x0028,
        "AH=13h DX=0x0050 should set GDC master EAD to 0x0028 (word address = DX/2)"
    );
}

#[test]
fn cursor_position_set_vx() {
    let code = make_int18h_call_dx(0x13, 0x0050);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_master.ead, 0x0028,
        "AH=13h DX=0x0050 should set GDC master EAD to 0x0028 (word address = DX/2)"
    );
}

#[test]
fn cursor_position_set_ra() {
    let code = make_int18h_call_dx(0x13, 0x0050);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_master.ead, 0x0028,
        "AH=13h DX=0x0050 should set GDC master EAD to 0x0028 (word address = DX/2)"
    );
}

// ============================================================================
// §9.2 AH=16h - Text VRAM Init
// ============================================================================

#[test]
fn text_vram_init_f() {
    let code = make_int18h_call_dx(0x16, 0xC141);
    let mut machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let char_byte = machine.bus.read_byte(0xA0000);
    let attr_byte = machine.bus.read_byte(0xA2000);
    assert_eq!(
        char_byte, 0x41,
        "AH=16h should fill text VRAM character plane with DL=0x41"
    );
    assert_eq!(
        attr_byte, 0xC1,
        "AH=16h should fill text VRAM attribute plane with DH=0xC1"
    );
}

#[test]
fn text_vram_init_vm() {
    let code = make_int18h_call_dx(0x16, 0xC141);
    let mut machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let char_byte = machine.bus.read_byte(0xA0000);
    let attr_byte = machine.bus.read_byte(0xA2000);
    assert_eq!(
        char_byte, 0x41,
        "AH=16h should fill text VRAM character plane with DL=0x41"
    );
    assert_eq!(
        attr_byte, 0xC1,
        "AH=16h should fill text VRAM attribute plane with DH=0xC1"
    );
}

#[test]
fn text_vram_init_vx() {
    let code = make_int18h_call_dx(0x16, 0xC141);
    let mut machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let char_byte = machine.bus.read_byte(0xA0000);
    let attr_byte = machine.bus.read_byte(0xA2000);
    assert_eq!(
        char_byte, 0x41,
        "AH=16h should fill text VRAM character plane with DL=0x41"
    );
    assert_eq!(
        attr_byte, 0xC1,
        "AH=16h should fill text VRAM attribute plane with DH=0xC1"
    );
}

#[test]
fn text_vram_init_ra() {
    let code = make_int18h_call_dx(0x16, 0xC141);
    let mut machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let char_byte = machine.bus.read_byte(0xA0000);
    let attr_byte = machine.bus.read_byte(0xA2000);
    assert_eq!(
        char_byte, 0x41,
        "AH=16h should fill text VRAM character plane with DL=0x41"
    );
    assert_eq!(
        attr_byte, 0xC1,
        "AH=16h should fill text VRAM attribute plane with DH=0xC1"
    );
}

// ============================================================================
// §9.4 AH=17h - Beep On
// ============================================================================

#[test]
fn beep_on_f() {
    let code = make_int18h_call(0x17);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.beeper.buzzer_enabled,
        "AH=17h should enable the beeper"
    );
}

#[test]
fn beep_on_vm() {
    let code = make_int18h_call(0x17);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.beeper.buzzer_enabled,
        "AH=17h should enable the beeper"
    );
}

#[test]
fn beep_on_vx() {
    let code = make_int18h_call(0x17);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.beeper.buzzer_enabled,
        "AH=17h should enable the beeper"
    );
}

#[test]
fn beep_on_ra() {
    let code = make_int18h_call(0x17);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.beeper.buzzer_enabled,
        "AH=17h should enable the beeper"
    );
}

// ============================================================================
// §9.4 AH=18h - Beep Off
// ============================================================================

#[test]
fn beep_off_f() {
    let code = make_int18h_two_calls(0x17, 0x18);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.beeper.buzzer_enabled,
        "AH=18h should disable the beeper"
    );
}

#[test]
fn beep_off_vm() {
    let code = make_int18h_two_calls(0x17, 0x18);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.beeper.buzzer_enabled,
        "AH=18h should disable the beeper"
    );
}

#[test]
fn beep_off_vx() {
    let code = make_int18h_two_calls(0x17, 0x18);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.beeper.buzzer_enabled,
        "AH=18h should disable the beeper"
    );
}

#[test]
fn beep_off_ra() {
    let code = make_int18h_two_calls(0x17, 0x18);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.beeper.buzzer_enabled,
        "AH=18h should disable the beeper"
    );
}

// ============================================================================
// §9.5 AH=40h - Graphics Display Start
// ============================================================================

#[test]
fn graphics_display_start_f() {
    let code = make_int18h_call(0x40);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_slave.display_enabled,
        "AH=40h should enable graphics display"
    );
}

#[test]
fn graphics_display_start_vm() {
    let code = make_int18h_call(0x40);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_slave.display_enabled,
        "AH=40h should enable graphics display"
    );
}

#[test]
fn graphics_display_start_vx() {
    let code = make_int18h_call(0x40);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_slave.display_enabled,
        "AH=40h should enable graphics display"
    );
}

#[test]
fn graphics_display_start_ra() {
    let code = make_int18h_call(0x40);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_slave.display_enabled,
        "AH=40h should enable graphics display"
    );
}

// ============================================================================
// §9.5 AH=41h - Graphics Display Stop
// ============================================================================

#[test]
fn graphics_display_stop_f() {
    let code = make_int18h_two_calls(0x40, 0x41);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_slave.display_enabled,
        "AH=41h should disable graphics display"
    );
}

#[test]
fn graphics_display_stop_vm() {
    let code = make_int18h_two_calls(0x40, 0x41);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_slave.display_enabled,
        "AH=41h should disable graphics display"
    );
}

#[test]
fn graphics_display_stop_vx() {
    let code = make_int18h_two_calls(0x40, 0x41);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_slave.display_enabled,
        "AH=41h should disable graphics display"
    );
}

#[test]
fn graphics_display_stop_ra() {
    let code = make_int18h_two_calls(0x40, 0x41);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_slave.display_enabled,
        "AH=41h should disable graphics display"
    );
}

// ============================================================================
// §9.5 AH=40h/41h - PRXCRT Bit 7
// ============================================================================

#[test]
fn graphics_display_start_sets_prxcrt_bit7_f() {
    let code = make_int18h_call(0x40);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_ne!(
        state.memory.ram[0x054C] & 0x80,
        0,
        "AH=40h should set PRXCRT bit 7"
    );
}

#[test]
fn graphics_display_start_sets_prxcrt_bit7_vm() {
    let code = make_int18h_call(0x40);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_ne!(
        state.memory.ram[0x054C] & 0x80,
        0,
        "AH=40h should set PRXCRT bit 7"
    );
}

#[test]
fn graphics_display_stop_clears_prxcrt_bit7_f() {
    let code = make_int18h_two_calls(0x40, 0x41);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.ram[0x054C] & 0x80,
        0,
        "AH=41h should clear PRXCRT bit 7"
    );
}

#[test]
fn graphics_display_stop_clears_prxcrt_bit7_vm() {
    let code = make_int18h_two_calls(0x40, 0x41);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.ram[0x054C] & 0x80,
        0,
        "AH=41h should clear PRXCRT bit 7"
    );
}

// ============================================================================
// §9.5 AH=43h - Palette Set
// ============================================================================

// The BIOS reads 4 GBCPC palette bytes from DS:BX+4 and writes them to the
// digital palette registers (I/O ports 0xA8/0xAA/0xAC/0xAE). The BIOS
// transforms GBCPC nibble-pair format into hardware register format.

fn run_palette_set_f(gbcpc: &[u8; 4]) -> machine::MachineState {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, DATA_TABLE + 4, gbcpc);
    let code = make_int18h_call_ds_bx(0x43, DATA_TABLE as u16);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    machine.save_state()
}

fn run_palette_set_vm(gbcpc: &[u8; 4]) -> machine::MachineState {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, DATA_TABLE + 4, gbcpc);
    let code = make_int18h_call_ds_bx(0x43, DATA_TABLE as u16);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    machine.save_state()
}

fn run_palette_set_vx(gbcpc: &[u8; 4]) -> machine::MachineState {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, DATA_TABLE + 4, gbcpc);
    let code = make_int18h_call_ds_bx(0x43, DATA_TABLE as u16);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    machine.save_state()
}

fn run_palette_set_ra(gbcpc: &[u8; 4]) -> machine::MachineState {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, DATA_TABLE + 4, gbcpc);
    let code = make_int18h_call_ds_bx(0x43, DATA_TABLE as u16);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    machine.save_state()
}

#[test]
fn palette_set_f() {
    let default_state = {
        let mut m = create_machine_f();
        boot_to_halt!(m);
        m.save_state()
    };
    let state = run_palette_set_vm(&[0x00, 0x00, 0x00, 0x00]);
    assert_ne!(
        state.palette.digital, default_state.palette.digital,
        "AH=43h should change palette from boot default"
    );
}

#[test]
fn palette_set_vm() {
    let default_state = {
        let mut m = create_machine_vm();
        boot_to_halt!(m);
        m.save_state()
    };
    let state = run_palette_set_vm(&[0x00, 0x00, 0x00, 0x00]);
    assert_ne!(
        state.palette.digital, default_state.palette.digital,
        "AH=43h should change palette from boot default"
    );
}

#[test]
fn palette_set_vx() {
    let default_state = {
        let mut m = create_machine_vx();
        boot_to_halt!(m);
        m.save_state()
    };
    let state = run_palette_set_vx(&[0x00, 0x00, 0x00, 0x00]);
    assert_ne!(
        state.palette.digital, default_state.palette.digital,
        "AH=43h should change palette from boot default"
    );
}

#[test]
fn palette_set_ra() {
    let default_state = {
        let mut m = create_machine_ra();
        boot_to_halt!(m);
        m.save_state()
    };
    let state = run_palette_set_ra(&[0x00, 0x00, 0x00, 0x00]);
    assert_ne!(
        state.palette.digital, default_state.palette.digital,
        "AH=43h should change palette from boot default"
    );
}

// ============================================================================
// §9.2 AH=0Ah - CRT Mode Set (20-line)
// ============================================================================

#[test]
fn crt_mode_set_20_f() {
    let code = make_int18h_call_al(0x0A, 0x01);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.ram[0x053C] & 0x01,
        0x01,
        "AH=0Ah AL=0x01 should set 20-line mode (bit 0 set in CRT_STS_FLAG)"
    );
}

#[test]
fn crt_mode_set_20_vm() {
    let code = make_int18h_call_al(0x0A, 0x01);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.ram[0x053C] & 0x01,
        0x01,
        "AH=0Ah AL=0x01 should set 20-line mode (bit 0 set in CRT_STS_FLAG)"
    );
}

#[test]
fn crt_mode_set_20_vx() {
    let code = make_int18h_call_al(0x0A, 0x01);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.ram[0x053C] & 0x01,
        0x01,
        "AH=0Ah AL=0x01 should set 20-line mode (bit 0 set in CRT_STS_FLAG)"
    );
}

#[test]
fn crt_mode_set_20_ra() {
    let code = make_int18h_call_al(0x0A, 0x01);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.ram[0x053C] & 0x01,
        0x01,
        "AH=0Ah AL=0x01 should set 20-line mode (bit 0 set in CRT_STS_FLAG)"
    );
}

// ============================================================================
// §9.2 AH=0Ah - CRT Mode Set (25-line)
// ============================================================================

#[test]
fn crt_mode_set_25_f() {
    let code = make_int18h_call_al(0x0A, 0x00);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.ram[0x053C] & 0x01,
        0x00,
        "AH=0Ah AL=0x00 should set 25-line mode (bit 0 clear in CRT_STS_FLAG)"
    );
}

#[test]
fn crt_mode_set_25_vm() {
    let code = make_int18h_call_al(0x0A, 0x00);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.ram[0x053C] & 0x01,
        0x00,
        "AH=0Ah AL=0x00 should set 25-line mode (bit 0 clear in CRT_STS_FLAG)"
    );
}

#[test]
fn crt_mode_set_25_vx() {
    let code = make_int18h_call_al(0x0A, 0x00);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.ram[0x053C] & 0x01,
        0x00,
        "AH=0Ah AL=0x00 should set 25-line mode (bit 0 clear in CRT_STS_FLAG)"
    );
}

#[test]
fn crt_mode_set_25_ra() {
    let code = make_int18h_call_al(0x0A, 0x00);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.ram[0x053C] & 0x01,
        0x00,
        "AH=0Ah AL=0x00 should set 25-line mode (bit 0 clear in CRT_STS_FLAG)"
    );
}

// ============================================================================
// §9.2 AH=0Ah - CRT Mode Set hides cursor (CSRFORM bit 7 side effect)
// ============================================================================
//
// The real BIOS rewrites CSRFORM byte 0 without bit 7 during AH=0Ah, which on
// the real GDC clears the cursor-enable flag. Games like Edge rely on this to
// drop the text cursor when switching into graphics mode without issuing an
// explicit AH=12h. Call AH=11h to show the cursor, then AH=0Ah, and assert the
// cursor has been disabled.
#[rustfmt::skip]
fn make_show_cursor_then_crt_mode_set(al: u8) -> Vec<u8> {
    vec![
        0xB4, 0x11,             // MOV AH, 11h  (show cursor)
        0xCD, 0x18,             // INT 18h
        0xB4, 0x0A, 0xB0, al,   // MOV AH, 0Ah; MOV AL, al
        0xCD, 0x18,             // INT 18h
        0xF4,                   // HLT
    ]
}

#[test]
fn crt_mode_set_hides_cursor_f() {
    let code = make_show_cursor_then_crt_mode_set(0x00);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.cursor_display,
        "AH=0Ah should hide cursor via CSRFORM byte 0 rewrite without bit 7"
    );
}

#[test]
fn crt_mode_set_hides_cursor_vm() {
    let code = make_show_cursor_then_crt_mode_set(0x00);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.cursor_display,
        "AH=0Ah should hide cursor via CSRFORM byte 0 rewrite without bit 7"
    );
}

#[test]
fn crt_mode_set_hides_cursor_vx() {
    let code = make_show_cursor_then_crt_mode_set(0x00);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.cursor_display,
        "AH=0Ah should hide cursor via CSRFORM byte 0 rewrite without bit 7"
    );
}

#[test]
fn crt_mode_set_hides_cursor_ra() {
    let code = make_show_cursor_then_crt_mode_set(0x00);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        !state.gdc_master.cursor_display,
        "AH=0Ah should hide cursor via CSRFORM byte 0 rewrite without bit 7"
    );
}

// ============================================================================
// §9.2 AH=0Eh - Single Display Area
// ============================================================================

#[test]
fn single_display_area_f() {
    let code = make_int18h_call_dx(0x0E, 0x0100);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_master.scroll[0].start_address, 0x0080,
        "AH=0Eh DX=0x0100 should set scroll[0] start_address to 0x0080 (DX/2)"
    );
}

#[test]
fn single_display_area_vm() {
    let code = make_int18h_call_dx(0x0E, 0x0100);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_master.scroll[0].start_address, 0x0080,
        "AH=0Eh DX=0x0100 should set scroll[0] start_address to 0x0080 (DX/2)"
    );
}

#[test]
fn single_display_area_vx() {
    let code = make_int18h_call_dx(0x0E, 0x0100);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_master.scroll[0].start_address, 0x0080,
        "AH=0Eh DX=0x0100 should set scroll[0] start_address to 0x0080 (DX/2)"
    );
}

#[test]
fn single_display_area_ra() {
    let code = make_int18h_call_dx(0x0E, 0x0100);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_master.scroll[0].start_address, 0x0080,
        "AH=0Eh DX=0x0100 should set scroll[0] start_address to 0x0080 (DX/2)"
    );
}

// ============================================================================
// §9.3 AH=1Bh - KCG Access Mode (Graphic)
// ============================================================================

#[test]
fn kcg_access_mode_graphic_f() {
    let code = make_int18h_call_al(0x1B, 0x01);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_ne!(
        state.display_control.video_mode & 0x20,
        0,
        "AH=1Bh AL=0x01 should set KAC dot access mode (video_mode bit 5)"
    );
}

#[test]
fn kcg_access_mode_graphic_vm() {
    let code = make_int18h_call_al(0x1B, 0x01);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_ne!(
        state.display_control.video_mode & 0x20,
        0,
        "AH=1Bh AL=0x01 should set KAC dot access mode (video_mode bit 5)"
    );
}

#[test]
fn kcg_access_mode_graphic_vx() {
    let code = make_int18h_call_al(0x1B, 0x01);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_ne!(
        state.display_control.video_mode & 0x20,
        0,
        "AH=1Bh AL=0x01 should set KAC dot access mode (video_mode bit 5)"
    );
}

#[test]
fn kcg_access_mode_graphic_ra() {
    let code = make_int18h_call_al(0x1B, 0x01);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_ne!(
        state.display_control.video_mode & 0x20,
        0,
        "AH=1Bh AL=0x01 should set KAC dot access mode (video_mode bit 5)"
    );
}

// ============================================================================
// §9.3 AH=1Bh - KCG Access Mode (Code)
// ============================================================================

#[test]
fn kcg_access_mode_code_f() {
    // Set graphic mode first, then reset to code mode.
    #[rustfmt::skip]
    let code = vec![
        0xB4, 0x1B, 0xB0, 0x01, 0xCD, 0x18,   // AH=1Bh AL=0x01 (graphic)
        0xB4, 0x1B, 0xB0, 0x00, 0xCD, 0x18,   // AH=1Bh AL=0x00 (code)
        0xF4,                                   // HLT
    ];
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.display_control.video_mode & 0x20,
        0,
        "AH=1Bh AL=0x00 should clear KAC dot access mode (video_mode bit 5)"
    );
}

#[test]
fn kcg_access_mode_code_vm() {
    // Set graphic mode first, then reset to code mode.
    #[rustfmt::skip]
    let code = vec![
        0xB4, 0x1B, 0xB0, 0x01, 0xCD, 0x18,   // AH=1Bh AL=0x01 (graphic)
        0xB4, 0x1B, 0xB0, 0x00, 0xCD, 0x18,   // AH=1Bh AL=0x00 (code)
        0xF4,                                   // HLT
    ];
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.display_control.video_mode & 0x20,
        0,
        "AH=1Bh AL=0x00 should clear KAC dot access mode (video_mode bit 5)"
    );
}

#[test]
fn kcg_access_mode_code_vx() {
    #[rustfmt::skip]
    let code = vec![
        0xB4, 0x1B, 0xB0, 0x01, 0xCD, 0x18,
        0xB4, 0x1B, 0xB0, 0x00, 0xCD, 0x18,
        0xF4,
    ];
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.display_control.video_mode & 0x20,
        0,
        "AH=1Bh AL=0x00 should clear KAC dot access mode (video_mode bit 5)"
    );
}

#[test]
fn kcg_access_mode_code_ra() {
    #[rustfmt::skip]
    let code = vec![
        0xB4, 0x1B, 0xB0, 0x01, 0xCD, 0x18,
        0xB4, 0x1B, 0xB0, 0x00, 0xCD, 0x18,
        0xF4,
    ];
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.display_control.video_mode & 0x20,
        0,
        "AH=1Bh AL=0x00 should clear KAC dot access mode (video_mode bit 5)"
    );
}

// ============================================================================
// §9.5 AH=42h - Display Area Set (640x400)
// ============================================================================

#[test]
fn display_area_set_f() {
    // Set display area, then re-enable graphics (AH=42h may reset GDC slave).
    #[rustfmt::skip]
    let code = vec![
        0xB4, 0x42, 0xB5, 0xC0,    // AH=42h CH=0xC0 (640x400)
        0xCD, 0x18,
        0xB4, 0x40, 0xCD, 0x18,    // AH=40h (graphics display start)
        0xF4,                       // HLT
    ];
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_slave.display_enabled,
        "AH=40h after AH=42h should enable graphics display"
    );
}

#[test]
fn display_area_set_vm() {
    // Set display area, then re-enable graphics (AH=42h may reset GDC slave).
    #[rustfmt::skip]
    let code = vec![
        0xB4, 0x42, 0xB5, 0xC0,    // AH=42h CH=0xC0 (640x400)
        0xCD, 0x18,
        0xB4, 0x40, 0xCD, 0x18,    // AH=40h (graphics display start)
        0xF4,                       // HLT
    ];
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_slave.display_enabled,
        "AH=40h after AH=42h should enable graphics display"
    );
}

#[test]
fn display_area_set_vx() {
    #[rustfmt::skip]
    let code = vec![
        0xB4, 0x42, 0xB5, 0xC0,
        0xCD, 0x18,
        0xB4, 0x40, 0xCD, 0x18,
        0xF4,
    ];
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert!(
        state.gdc_slave.display_enabled,
        "AH=40h after AH=42h should enable graphics display"
    );
}

#[test]
fn display_area_set_ra() {
    // RA BIOS AH=42h performs heavier GDC re-initialization than VM/VX,
    // requiring ~1.5M cycles. Use extended budget so both calls complete.
    #[rustfmt::skip]
    let code = vec![
        0xB4, 0x42, 0xB5, 0xC0,    // AH=42h CH=0xC0 (640x400)
        0xCD, 0x18,
        0xB4, 0x40, 0xCD, 0x18,    // AH=40h (graphics display start)
        0xF4,                       // HLT
    ];
    let machine = boot_inject_run_ra(&[], &code, 2_000_000);
    let state = machine.save_state();
    assert!(
        state.gdc_slave.display_enabled,
        "AH=40h after AH=42h should enable graphics display"
    );
}

// ============================================================================
// §9.5 AH=4Ah - Draw Mode Set
// ============================================================================

#[test]
fn draw_mode_set_f() {
    let code = make_int18h_call_ch(0x4A, 0x16);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    // CH=0x16 has bit 4 set -> PRXDUPD bit 3 should be cleared.
    assert_eq!(
        state.memory.ram[0x054D] & 0x08,
        0,
        "AH=4Ah CH=0x16 (bit 4 set) should clear PRXDUPD bit 3"
    );
}

#[test]
fn draw_mode_set_vm() {
    let code = make_int18h_call_ch(0x4A, 0x16);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    // CH=0x16 has bit 4 set -> PRXDUPD bit 3 should be cleared.
    assert_eq!(
        state.memory.ram[0x054D] & 0x08,
        0,
        "AH=4Ah CH=0x16 (bit 4 set) should clear PRXDUPD bit 3"
    );
}

#[test]
fn draw_mode_set_vx() {
    let code = make_int18h_call_ch(0x4A, 0x16);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.ram[0x054D] & 0x08,
        0,
        "AH=4Ah CH=0x16 (bit 4 set) should clear PRXDUPD bit 3"
    );
}

#[test]
fn draw_mode_set_ra() {
    let code = make_int18h_call_ch(0x4A, 0x16);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.ram[0x054D] & 0x08,
        0,
        "AH=4Ah CH=0x16 (bit 4 set) should clear PRXDUPD bit 3"
    );
}

// ============================================================================
// §9.3 AH=14h - Font Pattern Read (8x8 ANK)
// ============================================================================

#[test]
fn font_pattern_read_ank_f() {
    // Read font for 'A' (0x0041): expect 8x8 header (0x0101) + 8 bytes pattern.
    let code = make_int18h_call_bx_cx_dx(0x14, 0x0000, RESULT as u16, 0x0041);
    let mut machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let header = machine.bus.read_word(RESULT);
    assert_eq!(
        header, 0x0101,
        "AH=14h DX=0x0041 should return 8x8 ANK header (0x0101)"
    );
    let mut pattern_nonzero = false;
    for i in 0..8u32 {
        if machine.bus.read_byte(RESULT + 2 + i) != 0 {
            pattern_nonzero = true;
            break;
        }
    }
    assert!(
        pattern_nonzero,
        "AH=14h should return non-zero pattern data for 'A'"
    );
}

#[test]
fn font_pattern_read_ank_vm() {
    // Read font for 'A' (0x0041): expect 8x8 header (0x0101) + 8 bytes pattern.
    let code = make_int18h_call_bx_cx_dx(0x14, 0x0000, RESULT as u16, 0x0041);
    let mut machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let header = machine.bus.read_word(RESULT);
    assert_eq!(
        header, 0x0101,
        "AH=14h DX=0x0041 should return 8x8 ANK header (0x0101)"
    );
    let mut pattern_nonzero = false;
    for i in 0..8u32 {
        if machine.bus.read_byte(RESULT + 2 + i) != 0 {
            pattern_nonzero = true;
            break;
        }
    }
    assert!(
        pattern_nonzero,
        "AH=14h should return non-zero pattern data for 'A'"
    );
}

#[test]
fn font_pattern_read_ank_vx() {
    let code = make_int18h_call_bx_cx_dx(0x14, 0x0000, RESULT as u16, 0x0041);
    let mut machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let header = machine.bus.read_word(RESULT);
    assert_eq!(
        header, 0x0101,
        "AH=14h DX=0x0041 should return 8x8 ANK header (0x0101)"
    );
    let mut pattern_nonzero = false;
    for i in 0..8u32 {
        if machine.bus.read_byte(RESULT + 2 + i) != 0 {
            pattern_nonzero = true;
            break;
        }
    }
    assert!(
        pattern_nonzero,
        "AH=14h should return non-zero pattern data for 'A'"
    );
}

#[test]
fn font_pattern_read_ank_ra() {
    let code = make_int18h_call_bx_cx_dx(0x14, 0x0000, RESULT as u16, 0x0041);
    let mut machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let header = machine.bus.read_word(RESULT);
    assert_eq!(
        header, 0x0101,
        "AH=14h DX=0x0041 should return 8x8 ANK header (0x0101)"
    );
    let mut pattern_nonzero = false;
    for i in 0..8u32 {
        if machine.bus.read_byte(RESULT + 2 + i) != 0 {
            pattern_nonzero = true;
            break;
        }
    }
    assert!(
        pattern_nonzero,
        "AH=14h should return non-zero pattern data for 'A'"
    );
}

// ============================================================================
// §9.3 AH=14h - Font Pattern Read (16x16 Kanji)
// ============================================================================

#[test]
fn font_pattern_read_kanji_f() {
    // Read font for Kanji code 0x2121 (full-width space): expect 16x16 header (0x0202).
    let code = make_int18h_call_bx_cx_dx(0x14, 0x0000, RESULT as u16, 0x2121);
    let mut machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let header = machine.bus.read_word(RESULT);
    assert_eq!(
        header, 0x0202,
        "AH=14h DX=0x2121 should return 16x16 Kanji header (0x0202)"
    );
}

#[test]
fn font_pattern_read_kanji_vm() {
    // Read font for Kanji code 0x2121 (full-width space): expect 16x16 header (0x0202).
    let code = make_int18h_call_bx_cx_dx(0x14, 0x0000, RESULT as u16, 0x2121);
    let mut machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let header = machine.bus.read_word(RESULT);
    assert_eq!(
        header, 0x0202,
        "AH=14h DX=0x2121 should return 16x16 Kanji header (0x0202)"
    );
}

#[test]
fn font_pattern_read_kanji_vx() {
    let code = make_int18h_call_bx_cx_dx(0x14, 0x0000, RESULT as u16, 0x2121);
    let mut machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let header = machine.bus.read_word(RESULT);
    assert_eq!(
        header, 0x0202,
        "AH=14h DX=0x2121 should return 16x16 Kanji header (0x0202)"
    );
}

#[test]
fn font_pattern_read_kanji_ra() {
    let code = make_int18h_call_bx_cx_dx(0x14, 0x0000, RESULT as u16, 0x2121);
    let mut machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let header = machine.bus.read_word(RESULT);
    assert_eq!(
        header, 0x0202,
        "AH=14h DX=0x2121 should return 16x16 Kanji header (0x0202)"
    );
}

// ============================================================================
// §9.2 AH=0Fh - Multi Display Area
// ============================================================================

fn run_multi_display_area_f() -> machine::MachineState {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    // Scroll parameter data: 2 areas, each 4 bytes (VRAM addr word + line count word).
    #[rustfmt::skip]
    let scroll_data: [u8; 8] = [
        0x00, 0x00, 0x19, 0x00,    // Area 0: VRAM addr=0x0000, 25 rows
        0xA0, 0x00, 0x19, 0x00,    // Area 1: VRAM addr=0x00A0, 25 rows
    ];
    write_bytes(&mut machine.bus, DATA_TABLE, &scroll_data);
    // AH=0Fh: BX=0 (seg), CX=DATA_TABLE (off), DH=0 (start area), DL=2 (count).
    let code = make_int18h_call_bx_cx_dx(0x0F, 0x0000, DATA_TABLE as u16, 0x0002);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    machine.save_state()
}

fn run_multi_display_area_vm() -> machine::MachineState {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    // Scroll parameter data: 2 areas, each 4 bytes (VRAM addr word + line count word).
    #[rustfmt::skip]
    let scroll_data: [u8; 8] = [
        0x00, 0x00, 0x19, 0x00,    // Area 0: VRAM addr=0x0000, 25 rows
        0xA0, 0x00, 0x19, 0x00,    // Area 1: VRAM addr=0x00A0, 25 rows
    ];
    write_bytes(&mut machine.bus, DATA_TABLE, &scroll_data);
    // AH=0Fh: BX=0 (seg), CX=DATA_TABLE (off), DH=0 (start area), DL=2 (count).
    let code = make_int18h_call_bx_cx_dx(0x0F, 0x0000, DATA_TABLE as u16, 0x0002);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    machine.save_state()
}

fn run_multi_display_area_vx() -> machine::MachineState {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    #[rustfmt::skip]
    let scroll_data: [u8; 8] = [
        0x00, 0x00, 0x19, 0x00,
        0xA0, 0x00, 0x19, 0x00,
    ];
    write_bytes(&mut machine.bus, DATA_TABLE, &scroll_data);
    let code = make_int18h_call_bx_cx_dx(0x0F, 0x0000, DATA_TABLE as u16, 0x0002);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    machine.save_state()
}

fn run_multi_display_area_ra() -> machine::MachineState {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    #[rustfmt::skip]
    let scroll_data: [u8; 8] = [
        0x00, 0x00, 0x19, 0x00,
        0xA0, 0x00, 0x19, 0x00,
    ];
    write_bytes(&mut machine.bus, DATA_TABLE, &scroll_data);
    let code = make_int18h_call_bx_cx_dx(0x0F, 0x0000, DATA_TABLE as u16, 0x0002);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    machine.save_state()
}

#[test]
fn multi_display_area_f() {
    let state = run_multi_display_area_f();
    // Area 1 should have a different start_address than area 0.
    assert_ne!(
        state.gdc_master.scroll[0].start_address, state.gdc_master.scroll[1].start_address,
        "AH=0Fh should set different start addresses for scroll areas 0 and 1"
    );
}

#[test]
fn multi_display_area_vm() {
    let state = run_multi_display_area_vm();
    // Area 1 should have a different start_address than area 0.
    assert_ne!(
        state.gdc_master.scroll[0].start_address, state.gdc_master.scroll[1].start_address,
        "AH=0Fh should set different start addresses for scroll areas 0 and 1"
    );
}

#[test]
fn multi_display_area_vx() {
    let state = run_multi_display_area_vx();
    assert_ne!(
        state.gdc_master.scroll[0].start_address, state.gdc_master.scroll[1].start_address,
        "AH=0Fh should set different start addresses for scroll areas 0 and 1"
    );
}

#[test]
fn multi_display_area_ra() {
    let state = run_multi_display_area_ra();
    assert_ne!(
        state.gdc_master.scroll[0].start_address, state.gdc_master.scroll[1].start_address,
        "AH=0Fh should set different start addresses for scroll areas 0 and 1"
    );
}

// ============================================================================
// §9.3 AH=1Ah - User Char Define (Round-trip)
// ============================================================================

fn run_user_char_define_f() -> machine::MachineState {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);

    // Write 2-byte size header + 32-byte pattern at DATA_TABLE for the 16x16 character.
    let mut source = vec![0x02u8, 0x02]; // 2-byte size header (skipped by AH=1Ah)
    source.extend_from_slice(&[0xAA; 32]);
    write_bytes(&mut machine.bus, DATA_TABLE, &source);

    // Code: AH=1Ah (define char 0x7621), then AH=14h (read it back to RESULT).
    #[rustfmt::skip]
    let code = vec![
        0xBB, 0x00, 0x00,                          // MOV BX, 0x0000
        0xB9, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,  // MOV CX, DATA_TABLE
        0xBA, 0x21, 0x76,                           // MOV DX, 0x7621
        0xB4, 0x1A,                                 // MOV AH, 0x1A
        0xCD, 0x18,                                 // INT 18h (define char)
        0xBB, 0x00, 0x00,                           // MOV BX, 0x0000
        0xB9, (RESULT & 0xFF) as u8, ((RESULT >> 8) & 0xFF) as u8,  // MOV CX, RESULT
        0xBA, 0x21, 0x76,                           // MOV DX, 0x7621
        0xB4, 0x14,                                 // MOV AH, 0x14
        0xCD, 0x18,                                 // INT 18h (read back)
        0xF4,                                       // HLT
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    machine.save_state()
}

fn run_user_char_define_vm() -> machine::MachineState {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);

    // Write 2-byte size header + 32-byte pattern at DATA_TABLE for the 16x16 character.
    let mut source = vec![0x02u8, 0x02]; // 2-byte size header (skipped by AH=1Ah)
    source.extend_from_slice(&[0xAA; 32]);
    write_bytes(&mut machine.bus, DATA_TABLE, &source);

    // Code: AH=1Ah (define char 0x7621), then AH=14h (read it back to RESULT).
    #[rustfmt::skip]
    let code = vec![
        0xBB, 0x00, 0x00,                          // MOV BX, 0x0000
        0xB9, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,  // MOV CX, DATA_TABLE
        0xBA, 0x21, 0x76,                           // MOV DX, 0x7621
        0xB4, 0x1A,                                 // MOV AH, 0x1A
        0xCD, 0x18,                                 // INT 18h (define char)
        0xBB, 0x00, 0x00,                           // MOV BX, 0x0000
        0xB9, (RESULT & 0xFF) as u8, ((RESULT >> 8) & 0xFF) as u8,  // MOV CX, RESULT
        0xBA, 0x21, 0x76,                           // MOV DX, 0x7621
        0xB4, 0x14,                                 // MOV AH, 0x14
        0xCD, 0x18,                                 // INT 18h (read back)
        0xF4,                                       // HLT
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    machine.save_state()
}

fn run_user_char_define_vx() -> machine::MachineState {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    let mut source = vec![0x02u8, 0x02];
    source.extend_from_slice(&[0xAA; 32]);
    write_bytes(&mut machine.bus, DATA_TABLE, &source);
    #[rustfmt::skip]
    let code = vec![
        0xBB, 0x00, 0x00,
        0xB9, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x76,
        0xB4, 0x1A,
        0xCD, 0x18,
        0xBB, 0x00, 0x00,
        0xB9, (RESULT & 0xFF) as u8, ((RESULT >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x76,
        0xB4, 0x14,
        0xCD, 0x18,
        0xF4,
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    machine.save_state()
}

fn run_user_char_define_ra() -> machine::MachineState {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    let mut source = vec![0x02u8, 0x02];
    source.extend_from_slice(&[0xAA; 32]);
    write_bytes(&mut machine.bus, DATA_TABLE, &source);
    #[rustfmt::skip]
    let code = vec![
        0xBB, 0x00, 0x00,
        0xB9, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x76,
        0xB4, 0x1A,
        0xCD, 0x18,
        0xBB, 0x00, 0x00,
        0xB9, (RESULT & 0xFF) as u8, ((RESULT >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x76,
        0xB4, 0x14,
        0xCD, 0x18,
        0xF4,
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    machine.save_state()
}

#[test]
fn user_char_define_f() {
    let state = run_user_char_define_f();
    // After the round-trip, RESULT should have the 16x16 header (0x0202).
    let header = u16::from_le_bytes([
        state.memory.ram[RESULT as usize],
        state.memory.ram[RESULT as usize + 1],
    ]);
    assert_eq!(
        header, 0x0202,
        "AH=1Ah/14h round-trip should produce 16x16 header"
    );
    // At least some pattern bytes should be 0xAA (the value we wrote).
    let mut found_aa = false;
    for i in 2..34 {
        if state.memory.ram[RESULT as usize + i] == 0xAA {
            found_aa = true;
            break;
        }
    }
    assert!(
        found_aa,
        "AH=1Ah/14h round-trip should preserve user-defined pattern"
    );
}

#[test]
fn user_char_define_vm() {
    let state = run_user_char_define_vm();
    // After the round-trip, RESULT should have the 16x16 header (0x0202).
    let header = u16::from_le_bytes([
        state.memory.ram[RESULT as usize],
        state.memory.ram[RESULT as usize + 1],
    ]);
    assert_eq!(
        header, 0x0202,
        "AH=1Ah/14h round-trip should produce 16x16 header"
    );
    // At least some pattern bytes should be 0xAA (the value we wrote).
    let mut found_aa = false;
    for i in 2..34 {
        if state.memory.ram[RESULT as usize + i] == 0xAA {
            found_aa = true;
            break;
        }
    }
    assert!(
        found_aa,
        "AH=1Ah/14h round-trip should preserve user-defined pattern"
    );
}

#[test]
fn user_char_define_vx() {
    let state = run_user_char_define_vx();
    let header = u16::from_le_bytes([
        state.memory.ram[RESULT as usize],
        state.memory.ram[RESULT as usize + 1],
    ]);
    assert_eq!(
        header, 0x0202,
        "AH=1Ah/14h round-trip should produce 16x16 header"
    );
    let mut found_aa = false;
    for i in 2..34 {
        if state.memory.ram[RESULT as usize + i] == 0xAA {
            found_aa = true;
            break;
        }
    }
    assert!(
        found_aa,
        "AH=1Ah/14h round-trip should preserve user-defined pattern"
    );
}

#[test]
fn user_char_define_ra() {
    let state = run_user_char_define_ra();
    let header = u16::from_le_bytes([
        state.memory.ram[RESULT as usize],
        state.memory.ram[RESULT as usize + 1],
    ]);
    assert_eq!(
        header, 0x0202,
        "AH=1Ah/14h round-trip should produce 16x16 header"
    );
    let mut found_aa = false;
    for i in 2..34 {
        if state.memory.ram[RESULT as usize + i] == 0xAA {
            found_aa = true;
            break;
        }
    }
    assert!(
        found_aa,
        "AH=1Ah/14h round-trip should preserve user-defined pattern"
    );
}

// ============================================================================
// §9.5 AH=45h - Pattern Fill
// ============================================================================

fn make_ucw_fill() -> [u8; 0x2A] {
    let mut ucw = [0u8; 0x2A];
    ucw[0x00] = 0x07; // GBON_PTN: planes B, R, G enabled
    ucw[0x02] = 0x00; // GBDOTU: replace mode
    ucw[0x03] = 0x00; // GBDSP: direction 0
    // GBCPC: palette colors (white = all planes on).
    ucw[0x04] = 0xFF;
    ucw[0x05] = 0xFF;
    ucw[0x06] = 0xFF;
    ucw[0x07] = 0x00;
    // GBSX1 = 0, GBSY1 = 0 (start at origin).
    // GBLNG1 = 16 (fill 16 pixels wide).
    ucw[0x0C] = 16;
    // GBSY2 = 16, GBLNG2 = 16 (fill height).
    ucw[0x18] = 16;
    ucw[0x1E] = 16;
    // GBMDOTI: solid fill pattern.
    for i in 0..8 {
        ucw[0x20 + i] = 0xFF;
    }
    ucw[0x28] = 0x01; // GBDTYP: rectangle fill
    ucw[0x29] = 0x01; // GBFILL: fill mode
    ucw
}

#[test]
fn pattern_fill_f() {
    let ucw = make_ucw_fill();
    let state = run_draw_test_f(&ucw, 0x45);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=45h pattern fill should write non-zero data to graphics VRAM"
    );
}

#[test]
fn pattern_fill_vm() {
    let ucw = make_ucw_fill();
    let state = run_draw_test_vm(&ucw, 0x45);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=45h pattern fill should write non-zero data to graphics VRAM"
    );
}

#[test]
fn pattern_fill_vx() {
    let ucw = make_ucw_fill();
    let state = run_draw_test_vx(&ucw, 0x45);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=45h pattern fill should write non-zero data to graphics VRAM"
    );
}

#[test]
fn pattern_fill_ra() {
    let ucw = make_ucw_fill();
    let state = run_draw_test_ra(&ucw, 0x45);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=45h pattern fill should write non-zero data to graphics VRAM"
    );
}

// ============================================================================
// §9.5 AH=47h - Line Draw
// ============================================================================

fn make_ucw_line() -> [u8; 0x2A] {
    let mut ucw = [0u8; 0x2A];
    ucw[0x00] = 0x07; // GBON_PTN: planes B, R, G enabled
    ucw[0x02] = 0x00; // GBDOTU: replace mode
    ucw[0x03] = 0x00; // GBDSP: direction 0
    ucw[0x04] = 0xFF;
    ucw[0x05] = 0xFF;
    ucw[0x06] = 0xFF;
    ucw[0x07] = 0x00;
    // GBSX1 = 0, GBSY1 = 0 (start).
    // GBSX2 = 100, GBSY2 = 0 (end: horizontal line 100 pixels).
    ucw[0x16] = 100;
    ucw[0x17] = 0;
    ucw[0x18] = 0;
    ucw[0x19] = 0;
    // GBLNG1 = 100 (line length).
    ucw[0x0C] = 100;
    // GBMDOT = 0xFFFF (solid line pattern).
    ucw[0x1A] = 0xFF;
    ucw[0x1B] = 0xFF;
    // GBMDOTI: solid pattern.
    for i in 0..8 {
        ucw[0x20 + i] = 0xFF;
    }
    ucw[0x28] = 0x01; // GBDTYP: line
    ucw
}

#[test]
fn line_draw_f() {
    let ucw = make_ucw_line();
    let state = run_draw_test_f(&ucw, 0x47);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=47h line draw should write non-zero data to graphics VRAM (any plane/page)"
    );
}

#[test]
fn line_draw_vm() {
    let ucw = make_ucw_line();
    let state = run_draw_test_vm(&ucw, 0x47);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=47h line draw should write non-zero data to graphics VRAM (any plane/page)"
    );
}

#[test]
fn line_draw_vx() {
    let ucw = make_ucw_line();
    let state = run_draw_test_vx(&ucw, 0x47);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=47h line draw should write non-zero data to graphics VRAM"
    );
}

#[test]
fn line_draw_ra() {
    let ucw = make_ucw_line();
    let state = run_draw_test_ra(&ucw, 0x47);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=47h line draw should write non-zero data to graphics VRAM"
    );
}

// ============================================================================
// §9.5 AH=49h - Graphic Char
// ============================================================================

fn make_ucw_gchar() -> [u8; 0x2A] {
    let mut ucw = [0u8; 0x2A];
    ucw[0x00] = 0x07; // GBON_PTN: planes B, R, G enabled
    ucw[0x02] = 0x00; // GBDOTU: replace mode
    ucw[0x03] = 0x00; // GBDSP: direction 0
    ucw[0x04] = 0xFF;
    ucw[0x05] = 0xFF;
    ucw[0x06] = 0xFF;
    ucw[0x07] = 0x00;
    // GBSX1 = 0, GBSY1 = 0.
    // GBMDOTI: 8x8 solid block pattern.
    for i in 0..8 {
        ucw[0x20 + i] = 0xFF;
    }
    // GBLNG2 = 8 (character height).
    ucw[0x1E] = 8;
    ucw
}

#[test]
fn graphic_char_f() {
    let ucw = make_ucw_gchar();
    let state = run_draw_test_f(&ucw, 0x49);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=49h graphic char should write non-zero data to graphics VRAM"
    );
}

#[test]
fn graphic_char_vm() {
    let ucw = make_ucw_gchar();
    let state = run_draw_test_vm(&ucw, 0x49);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=49h graphic char should write non-zero data to graphics VRAM"
    );
}

#[test]
fn graphic_char_vx() {
    let ucw = make_ucw_gchar();
    let state = run_draw_test_vx(&ucw, 0x49);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=49h graphic char should write non-zero data to graphics VRAM"
    );
}

#[test]
fn graphic_char_ra() {
    let ucw = make_ucw_gchar();
    let state = run_draw_test_ra(&ucw, 0x49);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=49h graphic char should write non-zero data to graphics VRAM"
    );
}

// ============================================================================
// §9.5 AH=48h - Circle Draw
// ============================================================================

fn make_ucw_circle() -> [u8; 0x2A] {
    let mut ucw = [0u8; 0x2A];
    ucw[0x00] = 0x07; // GBON_PTN: planes B, R, G enabled
    ucw[0x02] = 0x00; // GBDOTU: replace mode
    ucw[0x03] = 0x00; // GBDSP: direction 0
    ucw[0x04] = 0xFF;
    ucw[0x05] = 0xFF;
    ucw[0x06] = 0xFF;
    ucw[0x07] = 0x00;
    // GBSX1 = 160, GBSY1 = 100 (center).
    ucw[0x08] = 160;
    ucw[0x09] = 0;
    ucw[0x0A] = 100;
    ucw[0x0B] = 0;
    // GBCIR = 50 (radius).
    ucw[0x1C] = 50;
    ucw[0x1D] = 0;
    // GBLNG1 = 50 (DC parameter).
    ucw[0x0C] = 50;
    // GBMDOT = 0xFFFF (solid line pattern).
    ucw[0x1A] = 0xFF;
    ucw[0x1B] = 0xFF;
    for i in 0..8 {
        ucw[0x20 + i] = 0xFF;
    }
    ucw[0x28] = 0x03; // GBDTYP: circle
    ucw
}

#[test]
fn circle_draw_f() {
    let ucw = make_ucw_circle();
    let state = run_draw_test_f(&ucw, 0x48);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=48h circle draw should write pixels to graphics VRAM"
    );
}

#[test]
fn circle_draw_vm() {
    let ucw = make_ucw_circle();
    let state = run_draw_test_vm(&ucw, 0x48);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=48h circle draw should write pixels to graphics VRAM"
    );
}

#[test]
fn circle_draw_vx() {
    let ucw = make_ucw_circle();
    let state = run_draw_test_vx(&ucw, 0x48);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=48h circle draw should write pixels to graphics VRAM"
    );
}

#[test]
fn circle_draw_ra() {
    let ucw = make_ucw_circle();
    let state = run_draw_test_ra(&ucw, 0x48);
    let any_nonzero = state.memory.graphics_vram.iter().any(|&b| b != 0);
    assert!(
        any_nonzero,
        "AH=48h circle draw should write pixels to graphics VRAM"
    );
}

// ============================================================================
// §9.5 AH=46h - Pattern Read
// ============================================================================

const PATTERN_READ_BUFFER: u32 = 0x3000;
const PATTERN_READ_SENTINEL: u8 = 0x55;
const PATTERN_READ_BUFFER_LEN: usize = 80;

fn run_pattern_read_f() -> machine::MachineState {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);

    // Pre-fill plane B at row 0 with known data (0xAA).
    for i in 0..80u32 {
        machine.bus.write_byte(0xA8000 + i, 0xAA);
    }

    // Fill output buffer with sentinel to detect writes.
    for i in 0..PATTERN_READ_BUFFER_LEN as u32 {
        machine
            .bus
            .write_byte(PATTERN_READ_BUFFER + i, PATTERN_READ_SENTINEL);
    }

    // UCW table for pattern read.
    let mut ucw = [0u8; 0x2A];
    ucw[0x08] = 0; // GBSX1 = 0
    ucw[0x0A] = 0; // GBSY1 = 0
    ucw[0x0C] = 16; // GBLNG1 = 16 pixels

    write_bytes(&mut machine.bus, DATA_TABLE, &ucw);

    let es_seg: u16 = (PATTERN_READ_BUFFER >> 4) as u16;
    #[rustfmt::skip]
    let code = vec![
        0xB4, 0x40, 0xCD, 0x18,                                        // Enable graphics
        0x31, 0xC0,                                                    // XOR AX, AX
        0x8E, 0xD8,                                                    // MOV DS, AX
        0xB8, (es_seg & 0xFF) as u8, (es_seg >> 8) as u8,              // MOV AX, es_seg
        0x8E, 0xC0,                                                    // MOV ES, AX
        0xBB, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,  // MOV BX, DATA_TABLE
        0xB5, 0xC0,                                                    // MOV CH, 0xC0 (640x400)
        0xB4, 0x46,                                                    // MOV AH, 0x46
        0xCD, 0x18,                                                    // INT 18h
        0xF4,                                                          // HLT
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    machine.save_state()
}

fn run_pattern_read_vm() -> machine::MachineState {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);

    // Pre-fill plane B at row 0 with known data (0xAA).
    for i in 0..80u32 {
        machine.bus.write_byte(0xA8000 + i, 0xAA);
    }

    // Fill output buffer with sentinel to detect writes.
    for i in 0..PATTERN_READ_BUFFER_LEN as u32 {
        machine
            .bus
            .write_byte(PATTERN_READ_BUFFER + i, PATTERN_READ_SENTINEL);
    }

    // UCW table for pattern read.
    let mut ucw = [0u8; 0x2A];
    ucw[0x08] = 0; // GBSX1 = 0
    ucw[0x0A] = 0; // GBSY1 = 0
    ucw[0x0C] = 16; // GBLNG1 = 16 pixels

    write_bytes(&mut machine.bus, DATA_TABLE, &ucw);

    let es_seg: u16 = (PATTERN_READ_BUFFER >> 4) as u16;
    #[rustfmt::skip]
    let code = vec![
        0xB4, 0x40, 0xCD, 0x18,                                        // Enable graphics
        0x31, 0xC0,                                                    // XOR AX, AX
        0x8E, 0xD8,                                                    // MOV DS, AX
        0xB8, (es_seg & 0xFF) as u8, (es_seg >> 8) as u8,              // MOV AX, es_seg
        0x8E, 0xC0,                                                    // MOV ES, AX
        0xBB, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,  // MOV BX, DATA_TABLE
        0xB5, 0xC0,                                                    // MOV CH, 0xC0 (640x400)
        0xB4, 0x46,                                                    // MOV AH, 0x46
        0xCD, 0x18,                                                    // INT 18h
        0xF4,                                                          // HLT
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    machine.save_state()
}

fn run_pattern_read_vx() -> machine::MachineState {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    for i in 0..80u32 {
        machine.bus.write_byte(0xA8000 + i, 0xAA);
    }
    for i in 0..PATTERN_READ_BUFFER_LEN as u32 {
        machine
            .bus
            .write_byte(PATTERN_READ_BUFFER + i, PATTERN_READ_SENTINEL);
    }
    let mut ucw = [0u8; 0x2A];
    ucw[0x08] = 0;
    ucw[0x0A] = 0;
    ucw[0x0C] = 16;
    write_bytes(&mut machine.bus, DATA_TABLE, &ucw);
    let es_seg: u16 = (PATTERN_READ_BUFFER >> 4) as u16;
    #[rustfmt::skip]
    let code = vec![
        0xB4, 0x40, 0xCD, 0x18,
        0x31, 0xC0,
        0x8E, 0xD8,
        0xB8, (es_seg & 0xFF) as u8, (es_seg >> 8) as u8,
        0x8E, 0xC0,
        0xBB, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,
        0xB5, 0xC0,
        0xB4, 0x46,
        0xCD, 0x18,
        0xF4,
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    machine.save_state()
}

fn run_pattern_read_ra() -> machine::MachineState {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    for i in 0..80u32 {
        machine.bus.write_byte(0xA8000 + i, 0xAA);
    }
    for i in 0..PATTERN_READ_BUFFER_LEN as u32 {
        machine
            .bus
            .write_byte(PATTERN_READ_BUFFER + i, PATTERN_READ_SENTINEL);
    }
    let mut ucw = [0u8; 0x2A];
    ucw[0x08] = 0;
    ucw[0x0A] = 0;
    ucw[0x0C] = 16;
    write_bytes(&mut machine.bus, DATA_TABLE, &ucw);
    let es_seg: u16 = (PATTERN_READ_BUFFER >> 4) as u16;
    #[rustfmt::skip]
    let code = vec![
        0xB4, 0x40, 0xCD, 0x18,
        0x31, 0xC0,
        0x8E, 0xD8,
        0xB8, (es_seg & 0xFF) as u8, (es_seg >> 8) as u8,
        0x8E, 0xC0,
        0xBB, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,
        0xB5, 0xC0,
        0xB4, 0x46,
        0xCD, 0x18,
        0xF4,
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    machine.save_state()
}

fn check_pattern_read_buffer(state: &machine::MachineState) -> bool {
    let base = PATTERN_READ_BUFFER as usize;
    (0..PATTERN_READ_BUFFER_LEN).any(|i| state.memory.ram[base + i] != PATTERN_READ_SENTINEL)
}

#[test]
fn pattern_read_f() {
    let state = run_pattern_read_f();
    assert!(
        check_pattern_read_buffer(&state),
        "AH=46h should write pattern data to the output buffer"
    );
}

#[test]
fn pattern_read_vm() {
    let state = run_pattern_read_vm();
    assert!(
        check_pattern_read_buffer(&state),
        "AH=46h should write pattern data to the output buffer"
    );
}

#[test]
fn pattern_read_vx() {
    let state = run_pattern_read_vx();
    assert!(
        check_pattern_read_buffer(&state),
        "AH=46h should write pattern data to the output buffer"
    );
}

#[test]
fn pattern_read_ra() {
    let state = run_pattern_read_ra();
    assert!(
        check_pattern_read_buffer(&state),
        "AH=46h should write pattern data to the output buffer"
    );
}

// ============================================================================
// §9.1 AH=00h - Key Read Blocks on Empty Buffer
// ============================================================================

#[test]
fn key_read_blocks_on_empty_buffer_f() {
    use common::Cpu as _;
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    // INT 18h AH=00h with no keys - should spin, not halt.
    let code = make_int18h_call(0x00);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    // The CPU should still be running (not halted) - it's busy-waiting.
    assert!(
        !machine.cpu.halted(),
        "AH=00h with empty buffer should not halt (should block/spin)"
    );
}

#[test]
fn key_read_blocks_on_empty_buffer_vm() {
    use common::Cpu as _;
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    // INT 18h AH=00h with no keys - should spin, not halt.
    let code = make_int18h_call(0x00);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    // The CPU should still be running (not halted) - it's busy-waiting.
    assert!(
        !machine.cpu.halted(),
        "AH=00h with empty buffer should not halt (should block/spin)"
    );
}

#[test]
fn key_read_blocks_on_empty_buffer_vx() {
    use common::Cpu as _;
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    let code = make_int18h_call(0x00);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    assert!(
        !machine.cpu.halted(),
        "AH=00h with empty buffer should not halt (should block/spin)"
    );
}

#[test]
fn key_read_blocks_on_empty_buffer_ra() {
    use common::Cpu as _;
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    let code = make_int18h_call(0x00);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    assert!(
        !machine.cpu.halted(),
        "AH=00h with empty buffer should not halt (should block/spin)"
    );
}

// ============================================================================
// §9.5 AH=43h - Palette Set Nibble Unpacking
// ============================================================================

#[test]
fn palette_set_nibble_unpack_f() {
    // GBCPC bytes: col[0]=0xAB, col[1]=0xCD, col[2]=0xEF, col[3]=0x12.
    // Port 0xA8 -> digital[0] = ((col[2] & 0x0F) << 4) | (col[0] & 0x0F) = 0xFB
    // Port 0xAA -> digital[1] = ((col[3] & 0x0F) << 4) | (col[1] & 0x0F) = 0x2D
    // Port 0xAC -> digital[2] = (col[2] & 0xF0) | (col[0] >> 4) = 0xEA
    // Port 0xAE -> digital[3] = (col[3] & 0xF0) | (col[1] >> 4) = 0x1C
    let state = run_palette_set_f(&[0xAB, 0xCD, 0xEF, 0x12]);
    assert_eq!(state.palette.digital[0], 0xFB, "digital[0]");
    assert_eq!(state.palette.digital[1], 0x2D, "digital[1]");
    assert_eq!(state.palette.digital[2], 0xEA, "digital[2]");
    assert_eq!(state.palette.digital[3], 0x1C, "digital[3]");
}

#[test]
fn palette_set_nibble_unpack_vm() {
    // GBCPC bytes: col[0]=0xAB, col[1]=0xCD, col[2]=0xEF, col[3]=0x12.
    // Port 0xA8 -> digital[0] = ((col[2] & 0x0F) << 4) | (col[0] & 0x0F) = 0xFB
    // Port 0xAA -> digital[1] = ((col[3] & 0x0F) << 4) | (col[1] & 0x0F) = 0x2D
    // Port 0xAC -> digital[2] = (col[2] & 0xF0) | (col[0] >> 4) = 0xEA
    // Port 0xAE -> digital[3] = (col[3] & 0xF0) | (col[1] >> 4) = 0x1C
    let state = run_palette_set_vm(&[0xAB, 0xCD, 0xEF, 0x12]);
    assert_eq!(state.palette.digital[0], 0xFB, "digital[0]");
    assert_eq!(state.palette.digital[1], 0x2D, "digital[1]");
    assert_eq!(state.palette.digital[2], 0xEA, "digital[2]");
    assert_eq!(state.palette.digital[3], 0x1C, "digital[3]");
}

#[test]
fn palette_set_nibble_unpack_vx() {
    let state = run_palette_set_vx(&[0xAB, 0xCD, 0xEF, 0x12]);
    assert_eq!(state.palette.digital[0], 0xFB, "digital[0]");
    assert_eq!(state.palette.digital[1], 0x2D, "digital[1]");
    assert_eq!(state.palette.digital[2], 0xEA, "digital[2]");
    assert_eq!(state.palette.digital[3], 0x1C, "digital[3]");
}

#[test]
fn palette_set_nibble_unpack_ra() {
    let state = run_palette_set_ra(&[0xAB, 0xCD, 0xEF, 0x12]);
    assert_eq!(state.palette.digital[0], 0xFB, "digital[0]");
    assert_eq!(state.palette.digital[1], 0x2D, "digital[1]");
    assert_eq!(state.palette.digital[2], 0xEA, "digital[2]");
    assert_eq!(state.palette.digital[3], 0x1C, "digital[3]");
}

// ============================================================================
// §9.2 AH=0Eh - Single Display Area Zeros Other Partitions
// ============================================================================

#[test]
fn single_display_area_does_not_zero_other_partitions_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    // First set multi-display to populate scroll[1].
    #[rustfmt::skip]
    let scroll_data: [u8; 8] = [
        0x00, 0x00, 0x19, 0x00,
        0xA0, 0x00, 0x19, 0x00,
    ];
    write_bytes(&mut machine.bus, DATA_TABLE, &scroll_data);
    #[rustfmt::skip]
    let code = vec![
        // AH=0Fh multi display area (2 areas)
        0xBB, 0x00, 0x00,
        0xB9, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,
        0xBA, 0x02, 0x00,
        0xB4, 0x0F,
        0xCD, 0x18,
        // AH=0Eh single display area DX=0x0000
        0xBA, 0x00, 0x00,
        0xB4, 0x0E,
        0xCD, 0x18,
        0xF4,
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_master.scroll[1].start_address, 80,
        "AH=0Eh should NOT zero scroll partition 1 (real BIOS behavior)"
    );
    assert_ne!(
        state.gdc_master.scroll[0].line_count, 0,
        "AH=0Eh should set non-zero line count on scroll[0]"
    );
    // BDA 0x0548 should be set.
    let bda_addr = read_ram_u16(&state.memory.ram, 0x0548);
    assert_eq!(bda_addr, 0x0000, "AH=0Eh DX=0 should set BDA 0x0548 to 0");
}

#[test]
fn single_display_area_does_not_zero_other_partitions_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    // First set multi-display to populate scroll[1].
    #[rustfmt::skip]
    let scroll_data: [u8; 8] = [
        0x00, 0x00, 0x19, 0x00,
        0xA0, 0x00, 0x19, 0x00,
    ];
    write_bytes(&mut machine.bus, DATA_TABLE, &scroll_data);
    #[rustfmt::skip]
    let code = vec![
        // AH=0Fh multi display area (2 areas)
        0xBB, 0x00, 0x00,
        0xB9, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,
        0xBA, 0x02, 0x00,
        0xB4, 0x0F,
        0xCD, 0x18,
        // AH=0Eh single display area DX=0x0000
        0xBA, 0x00, 0x00,
        0xB4, 0x0E,
        0xCD, 0x18,
        0xF4,
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_master.scroll[1].start_address, 80,
        "AH=0Eh should NOT zero scroll partition 1 (real BIOS behavior)"
    );
    assert_ne!(
        state.gdc_master.scroll[0].line_count, 0,
        "AH=0Eh should set non-zero line count on scroll[0]"
    );
    // BDA 0x0548 should be set.
    let bda_addr = read_ram_u16(&state.memory.ram, 0x0548);
    assert_eq!(bda_addr, 0x0000, "AH=0Eh DX=0 should set BDA 0x0548 to 0");
}

#[test]
fn single_display_area_does_not_zero_other_partitions_vx() {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    #[rustfmt::skip]
    let scroll_data: [u8; 8] = [
        0x00, 0x00, 0x19, 0x00,
        0xA0, 0x00, 0x19, 0x00,
    ];
    write_bytes(&mut machine.bus, DATA_TABLE, &scroll_data);
    #[rustfmt::skip]
    let code = vec![
        0xBB, 0x00, 0x00,
        0xB9, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,
        0xBA, 0x02, 0x00,
        0xB4, 0x0F,
        0xCD, 0x18,
        0xBA, 0x00, 0x00,
        0xB4, 0x0E,
        0xCD, 0x18,
        0xF4,
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_master.scroll[1].start_address, 80,
        "AH=0Eh should NOT zero scroll partition 1 (real BIOS behavior)"
    );
    assert_ne!(
        state.gdc_master.scroll[0].line_count, 0,
        "AH=0Eh should set non-zero line count on scroll[0]"
    );
}

#[test]
fn single_display_area_does_not_zero_other_partitions_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    #[rustfmt::skip]
    let scroll_data: [u8; 8] = [
        0x00, 0x00, 0x19, 0x00,
        0xA0, 0x00, 0x19, 0x00,
    ];
    write_bytes(&mut machine.bus, DATA_TABLE, &scroll_data);
    #[rustfmt::skip]
    let code = vec![
        0xBB, 0x00, 0x00,
        0xB9, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,
        0xBA, 0x02, 0x00,
        0xB4, 0x0F,
        0xCD, 0x18,
        0xBA, 0x00, 0x00,
        0xB4, 0x0E,
        0xCD, 0x18,
        0xF4,
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_master.scroll[1].start_address, 80,
        "AH=0Eh should NOT zero scroll partition 1 (real BIOS behavior)"
    );
    assert_ne!(
        state.gdc_master.scroll[0].line_count, 0,
        "AH=0Eh should set non-zero line count on scroll[0]"
    );
}

// ============================================================================
// §9.2 AH=16h - Text VRAM Attribute Fill Reaches 0x3FE0
// ============================================================================

#[test]
fn text_vram_init_attr_reaches_end_f() {
    let code = make_int18h_call_dx(0x16, 0xE100);
    let mut machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    // Check that attribute byte at 0xA3FDE (last even offset before 0x3FE0) is set.
    let attr_near_end = machine.bus.read_byte(0xA3FDE);
    assert_eq!(
        attr_near_end, 0xE1,
        "AH=16h should fill attributes up to 0xA3FDE (DH=0xE1)"
    );
}

#[test]
fn text_vram_init_attr_reaches_end_vm() {
    let code = make_int18h_call_dx(0x16, 0xE100);
    let mut machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    // Check that attribute byte at 0xA3FDE (last even offset before 0x3FE0) is set.
    let attr_near_end = machine.bus.read_byte(0xA3FDE);
    assert_eq!(
        attr_near_end, 0xE1,
        "AH=16h should fill attributes up to 0xA3FDE (DH=0xE1)"
    );
}

#[test]
fn text_vram_init_attr_reaches_end_vx() {
    let code = make_int18h_call_dx(0x16, 0xE100);
    let mut machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let attr_near_end = machine.bus.read_byte(0xA3FDE);
    assert_eq!(
        attr_near_end, 0xE1,
        "AH=16h should fill attributes up to 0xA3FDE (DH=0xE1)"
    );
}

#[test]
fn text_vram_init_attr_reaches_end_ra() {
    let code = make_int18h_call_dx(0x16, 0xE100);
    let mut machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let attr_near_end = machine.bus.read_byte(0xA3FDE);
    assert_eq!(
        attr_near_end, 0xE1,
        "AH=16h should fill attributes up to 0xA3FDE (DH=0xE1)"
    );
}

// ============================================================================
// §9.3 AH=1Ah - User Char Define Rejects Non-0x76/0x77 Rows
// ============================================================================

#[test]
fn user_char_define_rejects_invalid_row_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    // Write distinct pattern at source.
    let pattern: [u8; 34] = [0xBB; 34];
    write_bytes(&mut machine.bus, DATA_TABLE, &pattern);
    // Try to define char at row 0x21 (not user-definable).
    #[rustfmt::skip]
    let code = vec![
        0xBB, 0x00, 0x00,
        0xB9, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x21,      // DX = 0x2121 (row 0x21, not 0x76/0x77)
        0xB4, 0x1A,
        0xCD, 0x18,
        // Read it back to RESULT.
        0xBB, 0x00, 0x00,
        0xB9, (RESULT & 0xFF) as u8, ((RESULT >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x21,
        0xB4, 0x14,
        0xCD, 0x18,
        0xF4,
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    let state = machine.save_state();
    // The font data should NOT be 0xBB - the define should have been rejected.
    let mut found_bb = false;
    for i in 2..34 {
        if state.memory.ram[RESULT as usize + i] == 0xBB {
            found_bb = true;
            break;
        }
    }
    assert!(
        !found_bb,
        "AH=1Ah should reject row 0x21 - font data should not be modified"
    );
}

#[test]
fn user_char_define_rejects_invalid_row_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    // Write distinct pattern at source.
    let pattern: [u8; 34] = [0xBB; 34];
    write_bytes(&mut machine.bus, DATA_TABLE, &pattern);
    // Try to define char at row 0x21 (not user-definable).
    #[rustfmt::skip]
    let code = vec![
        0xBB, 0x00, 0x00,
        0xB9, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x21,      // DX = 0x2121 (row 0x21, not 0x76/0x77)
        0xB4, 0x1A,
        0xCD, 0x18,
        // Read it back to RESULT.
        0xBB, 0x00, 0x00,
        0xB9, (RESULT & 0xFF) as u8, ((RESULT >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x21,
        0xB4, 0x14,
        0xCD, 0x18,
        0xF4,
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    let state = machine.save_state();
    // The font data should NOT be 0xBB - the define should have been rejected.
    let mut found_bb = false;
    for i in 2..34 {
        if state.memory.ram[RESULT as usize + i] == 0xBB {
            found_bb = true;
            break;
        }
    }
    assert!(
        !found_bb,
        "AH=1Ah should reject row 0x21 - font data should not be modified"
    );
}

// ============================================================================
// §9.3 AH=1Ah - User Char Define Skips 2-byte Header
// ============================================================================

#[test]
fn user_char_define_skips_header_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    // Write 2-byte header (0x02, 0x02) + 32 bytes of interleaved pattern.
    // Format: [left0, right0, left1, right1, ..., left15, right15].
    let mut source = vec![0x02u8, 0x02]; // header
    for _ in 0..16 {
        source.push(0xCC); // left
        source.push(0xDD); // right
    }
    write_bytes(&mut machine.bus, DATA_TABLE, &source);
    // Define char 0x7621 (valid row 0x76).
    #[rustfmt::skip]
    let code = vec![
        0xBB, 0x00, 0x00,
        0xB9, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x76,
        0xB4, 0x1A,
        0xCD, 0x18,
        // Read it back.
        0xBB, 0x00, 0x00,
        0xB9, (RESULT & 0xFF) as u8, ((RESULT >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x76,
        0xB4, 0x14,
        0xCD, 0x18,
        0xF4,
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    let state = machine.save_state();
    // Output header should be 0x0202 (16x16 kanji).
    let header = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(header, 0x0202, "AH=14h header should be 0x0202 for kanji");
    // After 2-byte header, data is interleaved: [left0, right0, left1, right1, ...].
    // Left bytes (even offsets from +2) should be 0xCC.
    let left_byte = state.memory.ram[RESULT as usize + 2];
    assert_eq!(
        left_byte, 0xCC,
        "AH=1Ah should skip 2-byte header: left half should be 0xCC, not the header byte"
    );
    // Right bytes (odd offsets from +2) should be 0xDD.
    let right_byte = state.memory.ram[RESULT as usize + 3];
    assert_eq!(right_byte, 0xDD, "AH=1Ah right half should be 0xDD");
}

#[test]
fn user_char_define_skips_header_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    // Write 2-byte header (0x02, 0x02) + 32 bytes of interleaved pattern.
    // Format: [left0, right0, left1, right1, ..., left15, right15].
    let mut source = vec![0x02u8, 0x02]; // header
    for _ in 0..16 {
        source.push(0xCC); // left
        source.push(0xDD); // right
    }
    write_bytes(&mut machine.bus, DATA_TABLE, &source);
    // Define char 0x7621 (valid row 0x76).
    #[rustfmt::skip]
    let code = vec![
        0xBB, 0x00, 0x00,
        0xB9, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x76,
        0xB4, 0x1A,
        0xCD, 0x18,
        // Read it back.
        0xBB, 0x00, 0x00,
        0xB9, (RESULT & 0xFF) as u8, ((RESULT >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x76,
        0xB4, 0x14,
        0xCD, 0x18,
        0xF4,
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    let state = machine.save_state();
    // Output header should be 0x0202 (16x16 kanji).
    let header = read_ram_u16(&state.memory.ram, RESULT as usize);
    assert_eq!(header, 0x0202, "AH=14h header should be 0x0202 for kanji");
    // After 2-byte header, data is interleaved: [left0, right0, left1, right1, ...].
    // Left bytes (even offsets from +2) should be 0xCC.
    let left_byte = state.memory.ram[RESULT as usize + 2];
    assert_eq!(
        left_byte, 0xCC,
        "AH=1Ah should skip 2-byte header: left half should be 0xCC, not the header byte"
    );
    // Right bytes (odd offsets from +2) should be 0xDD.
    let right_byte = state.memory.ram[RESULT as usize + 3];
    assert_eq!(right_byte, 0xDD, "AH=1Ah right half should be 0xDD");
}

#[test]
fn user_char_define_skips_header_vx() {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    let mut source = vec![0x02u8, 0x02];
    for _ in 0..16 {
        source.push(0xCC);
        source.push(0xDD);
    }
    write_bytes(&mut machine.bus, DATA_TABLE, &source);
    #[rustfmt::skip]
    let code = vec![
        0xBB, 0x00, 0x00,
        0xB9, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x76,
        0xB4, 0x1A,
        0xCD, 0x18,
        0xBB, 0x00, 0x00,
        0xB9, (RESULT & 0xFF) as u8, ((RESULT >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x76,
        0xB4, 0x14,
        0xCD, 0x18,
        0xF4,
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    let state = machine.save_state();
    let left_byte = state.memory.ram[RESULT as usize + 2];
    assert_eq!(left_byte, 0xCC, "AH=1Ah left half should be 0xCC");
    let right_byte = state.memory.ram[RESULT as usize + 3];
    assert_eq!(right_byte, 0xDD, "AH=1Ah right half should be 0xDD");
}

#[test]
fn user_char_define_skips_header_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    let mut source = vec![0x02u8, 0x02];
    for _ in 0..16 {
        source.push(0xCC);
        source.push(0xDD);
    }
    write_bytes(&mut machine.bus, DATA_TABLE, &source);
    #[rustfmt::skip]
    let code = vec![
        0xBB, 0x00, 0x00,
        0xB9, (DATA_TABLE & 0xFF) as u8, ((DATA_TABLE >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x76,
        0xB4, 0x1A,
        0xCD, 0x18,
        0xBB, 0x00, 0x00,
        0xB9, (RESULT & 0xFF) as u8, ((RESULT >> 8) & 0xFF) as u8,
        0xBA, 0x21, 0x76,
        0xB4, 0x14,
        0xCD, 0x18,
        0xF4,
    ];
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    let state = machine.save_state();
    let left_byte = state.memory.ram[RESULT as usize + 2];
    assert_eq!(left_byte, 0xCC, "AH=1Ah left half should be 0xCC");
    let right_byte = state.memory.ram[RESULT as usize + 3];
    assert_eq!(right_byte, 0xDD, "AH=1Ah right half should be 0xDD");
}

// ============================================================================
// §9.2 AH=0Fh - Multi Display Area VRAM Addr >>1 and Raster
// ============================================================================

#[test]
fn multi_display_area_vram_addr_halved_f() {
    let state = run_multi_display_area_f();
    // Area 0: input VRAM addr = 0x0000, after >>1 = 0x0000.
    assert_eq!(
        state.gdc_master.scroll[0].start_address, 0x0000,
        "AH=0Fh area 0: VRAM addr 0x0000 >> 1 = 0"
    );
    // Area 1: input VRAM addr = 0x00A0, after >>1 = 0x0050.
    assert_eq!(
        state.gdc_master.scroll[1].start_address, 0x0050,
        "AH=0Fh area 1: VRAM addr 0x00A0 >> 1 = 0x0050"
    );
}

#[test]
fn multi_display_area_vram_addr_halved_vm() {
    let state = run_multi_display_area_vm();
    // Area 0: input VRAM addr = 0x0000, after >>1 = 0x0000.
    assert_eq!(
        state.gdc_master.scroll[0].start_address, 0x0000,
        "AH=0Fh area 0: VRAM addr 0x0000 >> 1 = 0"
    );
    // Area 1: input VRAM addr = 0x00A0, after >>1 = 0x0050.
    assert_eq!(
        state.gdc_master.scroll[1].start_address, 0x0050,
        "AH=0Fh area 1: VRAM addr 0x00A0 >> 1 = 0x0050"
    );
}

#[test]
fn multi_display_area_vram_addr_halved_vx() {
    let state = run_multi_display_area_vx();
    assert_eq!(state.gdc_master.scroll[0].start_address, 0x0000);
    assert_eq!(
        state.gdc_master.scroll[1].start_address, 0x0050,
        "AH=0Fh area 1: VRAM addr 0x00A0 >> 1 = 0x0050"
    );
}

#[test]
fn multi_display_area_vram_addr_halved_ra() {
    let state = run_multi_display_area_ra();
    assert_eq!(state.gdc_master.scroll[0].start_address, 0x0000);
    assert_eq!(
        state.gdc_master.scroll[1].start_address, 0x0050,
        "AH=0Fh area 1: VRAM addr 0x00A0 >> 1 = 0x0050"
    );
}

#[test]
fn multi_display_area_line_count_has_raster_f() {
    let state = run_multi_display_area_f();
    // Line count should be input_lines * raster.
    // Raster depends on CRT mode. The key check: raw line count (25) is
    // multiplied by the raster factor, so the stored value must be > 25.
    assert!(
        state.gdc_master.scroll[0].line_count > 25,
        "AH=0Fh: line count should be multiplied by raster (got {})",
        state.gdc_master.scroll[0].line_count
    );
}

#[test]
fn multi_display_area_line_count_has_raster_vm() {
    let state = run_multi_display_area_vm();
    // Line count should be input_lines * raster.
    // Raster depends on CRT mode. The key check: raw line count (25) is
    // multiplied by the raster factor, so the stored value must be > 25.
    assert!(
        state.gdc_master.scroll[0].line_count > 25,
        "AH=0Fh: line count should be multiplied by raster (got {})",
        state.gdc_master.scroll[0].line_count
    );
}

#[test]
fn multi_display_area_line_count_has_raster_vx() {
    let state = run_multi_display_area_vx();
    assert!(
        state.gdc_master.scroll[0].line_count > 25,
        "AH=0Fh: line count should be multiplied by raster (got {})",
        state.gdc_master.scroll[0].line_count
    );
}

#[test]
fn multi_display_area_line_count_has_raster_ra() {
    let state = run_multi_display_area_ra();
    assert!(
        state.gdc_master.scroll[0].line_count > 25,
        "AH=0Fh: line count should be multiplied by raster (got {})",
        state.gdc_master.scroll[0].line_count
    );
}

// ============================================================================
// §9.3 AH=14h - Kanji Font Read Interleaved Layout
// ============================================================================

#[test]
fn kanji_font_read_different_codes_f() {
    // Read two different kanji and verify they produce different data.
    let code1 = make_int18h_call_bx_cx_dx(0x14, 0x0000, RESULT as u16, 0x2122);
    let mut m1 = boot_inject_run_f(&[], &code1, INT18H_BUDGET);
    let mut data1 = [0u8; 32];
    for i in 0..32u32 {
        data1[i as usize] = m1.bus.read_byte(RESULT + 2 + i);
    }

    let code2 = make_int18h_call_bx_cx_dx(0x14, 0x0000, RESULT as u16, 0x2130);
    let mut m2 = boot_inject_run_f(&[], &code2, INT18H_BUDGET);
    let mut data2 = [0u8; 32];
    for i in 0..32u32 {
        data2[i as usize] = m2.bus.read_byte(RESULT + 2 + i);
    }

    assert_ne!(
        data1, data2,
        "AH=14h should return different patterns for different kanji codes"
    );
}

#[test]
fn kanji_font_read_different_codes_vm() {
    // Read two different kanji and verify they produce different data.
    let code1 = make_int18h_call_bx_cx_dx(0x14, 0x0000, RESULT as u16, 0x2122);
    let mut m1 = boot_inject_run_vm(&[], &code1, INT18H_BUDGET);
    let mut data1 = [0u8; 32];
    for i in 0..32u32 {
        data1[i as usize] = m1.bus.read_byte(RESULT + 2 + i);
    }

    let code2 = make_int18h_call_bx_cx_dx(0x14, 0x0000, RESULT as u16, 0x2130);
    let mut m2 = boot_inject_run_vm(&[], &code2, INT18H_BUDGET);
    let mut data2 = [0u8; 32];
    for i in 0..32u32 {
        data2[i as usize] = m2.bus.read_byte(RESULT + 2 + i);
    }

    assert_ne!(
        data1, data2,
        "AH=14h should return different patterns for different kanji codes"
    );
}

// ============================================================================
// §9.5 AH=42h - Display Area Set: 400-line Mode (lines_per_row, video_mode)
// ============================================================================

#[test]
fn display_area_set_400line_lines_per_row_f() {
    // CH=0xC0 -> 400-line ALL mode -> lines_per_row should be 1, video_mode bit 4 clear.
    let code = make_int18h_call_ch(0x42, 0xC0);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_slave.lines_per_row, 1,
        "AH=42h CH=0xC0 (400-line) should set lines_per_row=1"
    );
    assert_eq!(
        state.display_control.video_mode & 0x10,
        0,
        "AH=42h CH=0xC0 (400-line) should clear hide_odd_rasters (video_mode bit 4)"
    );
}

#[test]
fn display_area_set_400line_lines_per_row_vm() {
    // CH=0xC0 -> 400-line ALL mode -> lines_per_row should be 1, video_mode bit 4 clear.
    let code = make_int18h_call_ch(0x42, 0xC0);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_slave.lines_per_row, 1,
        "AH=42h CH=0xC0 (400-line) should set lines_per_row=1"
    );
    assert_eq!(
        state.display_control.video_mode & 0x10,
        0,
        "AH=42h CH=0xC0 (400-line) should clear hide_odd_rasters (video_mode bit 4)"
    );
}

#[test]
fn display_area_set_400line_lines_per_row_vx() {
    let code = make_int18h_call_ch(0x42, 0xC0);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.lines_per_row, 1);
    assert_eq!(state.display_control.video_mode & 0x10, 0);
}

#[test]
fn display_area_set_400line_lines_per_row_ra() {
    let code = make_int18h_call_ch(0x42, 0xC0);
    let machine = boot_inject_run_ra(&[], &code, 2_000_000);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.lines_per_row, 1);
    assert_eq!(state.display_control.video_mode & 0x10, 0);
}

// ============================================================================
// §9.5 AH=42h - Display Area Set: 200-line Mode (lines_per_row, video_mode)
// ============================================================================

#[test]
fn display_area_set_200line_lines_per_row_f() {
    // CH=0x80 -> 200-line LOWER mode. PRXCRT bit 6 set (24kHz) -> lines_per_row=2,
    // video_mode bit 4 set (hide_odd_rasters).
    let code = make_int18h_call_ch(0x42, 0x80);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_slave.lines_per_row, 2,
        "AH=42h CH=0x80 (200-line, PRXCRT bit 6 set) should set lines_per_row=2"
    );
    assert_ne!(
        state.display_control.video_mode & 0x10,
        0,
        "AH=42h CH=0x80 (200-line, PRXCRT bit 6 set) should set hide_odd_rasters"
    );
}

#[test]
fn display_area_set_200line_lines_per_row_vm() {
    // CH=0x80 -> 200-line LOWER mode. PRXCRT bit 6 set (24kHz) -> lines_per_row=2,
    // video_mode bit 4 set (hide_odd_rasters).
    let code = make_int18h_call_ch(0x42, 0x80);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_slave.lines_per_row, 2,
        "AH=42h CH=0x80 (200-line, PRXCRT bit 6 set) should set lines_per_row=2"
    );
    assert_ne!(
        state.display_control.video_mode & 0x10,
        0,
        "AH=42h CH=0x80 (200-line, PRXCRT bit 6 set) should set hide_odd_rasters"
    );
}

#[test]
fn display_area_set_200line_lines_per_row_vx() {
    let code = make_int18h_call_ch(0x42, 0x80);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.lines_per_row, 2);
    assert_ne!(state.display_control.video_mode & 0x10, 0);
}

#[test]
fn display_area_set_200line_lines_per_row_ra() {
    let code = make_int18h_call_ch(0x42, 0x80);
    let machine = boot_inject_run_ra(&[], &code, 2_000_000);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.lines_per_row, 2);
    assert_ne!(state.display_control.video_mode & 0x10, 0);
}

// ============================================================================
// §9.5 AH=42h - Display Area Set: Scroll Partition Reset
// ============================================================================

#[test]
fn display_area_set_resets_scroll_f() {
    // AH=42h should reset scroll[0] start_address to 0 and line_count to 0x400.
    let code = make_int18h_call_ch(0x42, 0xC0);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_slave.scroll[0].start_address, 0,
        "AH=42h should reset scroll[0].start_address to 0"
    );
}

#[test]
fn display_area_set_resets_scroll_vm() {
    // AH=42h should reset scroll[0] start_address to 0 and line_count to 0x400.
    let code = make_int18h_call_ch(0x42, 0xC0);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_slave.scroll[0].start_address, 0,
        "AH=42h should reset scroll[0].start_address to 0"
    );
}

#[test]
fn display_area_set_resets_scroll_vx() {
    let code = make_int18h_call_ch(0x42, 0xC0);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.scroll[0].start_address, 0);
}

#[test]
fn display_area_set_resets_scroll_ra() {
    let code = make_int18h_call_ch(0x42, 0xC0);
    let machine = boot_inject_run_ra(&[], &code, 2_000_000);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.scroll[0].start_address, 0);
}

// ============================================================================
// §9.5 AH=42h - Display Area Set: Preserves GDC Slave Sync Params
// ============================================================================

/// AH=42h must NOT load new GDC slave sync parameters unless PRXDUPD has the
/// right bit pattern ((prxdupd & 0x24) == 0x24 for 200-line, or 0x20 for 400-line).
/// After a fresh boot PRXDUPD is 0x00 (VM) / 0x50 (VX and later), so neither condition
/// is met and the slave GDC pitch, AW, AL, and timing must remain unchanged.
#[test]
fn display_area_set_preserves_slave_sync_f() {
    let code = make_int18h_call_ch(0x42, 0x80); // 200-line LOWER
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_slave.pitch, 40,
        "AH=42h should not change GDC slave pitch when PRXDUPD condition is not met"
    );
    assert_eq!(
        state.gdc_slave.aw, 40,
        "AH=42h should not change GDC slave AW when PRXDUPD condition is not met"
    );
    assert_eq!(
        state.gdc_slave.al, 400,
        "AH=42h should not change GDC slave AL when PRXDUPD condition is not met"
    );
}

/// AH=42h must NOT load new GDC slave sync parameters unless PRXDUPD has the
/// right bit pattern ((prxdupd & 0x24) == 0x24 for 200-line, or 0x20 for 400-line).
/// After a fresh boot PRXDUPD is 0x00 (VM) / 0x50 (VX and later), so neither condition
/// is met and the slave GDC pitch, AW, AL, and timing must remain unchanged.
#[test]
fn display_area_set_preserves_slave_sync_vm() {
    let code = make_int18h_call_ch(0x42, 0x80); // 200-line LOWER
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_slave.pitch, 40,
        "AH=42h should not change GDC slave pitch when PRXDUPD condition is not met"
    );
    assert_eq!(
        state.gdc_slave.aw, 40,
        "AH=42h should not change GDC slave AW when PRXDUPD condition is not met"
    );
    assert_eq!(
        state.gdc_slave.al, 400,
        "AH=42h should not change GDC slave AL when PRXDUPD condition is not met"
    );
}

#[test]
fn display_area_set_preserves_slave_sync_vx() {
    let code = make_int18h_call_ch(0x42, 0x80);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.pitch, 40);
    assert_eq!(state.gdc_slave.aw, 40);
    assert_eq!(state.gdc_slave.al, 400);
}

#[test]
fn display_area_set_preserves_slave_sync_ra() {
    let code = make_int18h_call_ch(0x42, 0x80);
    let machine = boot_inject_run_ra(&[], &code, 2_000_000);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.pitch, 40);
    assert_eq!(state.gdc_slave.aw, 40);
    assert_eq!(state.gdc_slave.al, 400);
}

/// Also verify the 400-line ALL mode path (CH=0xC0, crtmode=2) preserves sync
/// params when (prxdupd & 0x24) != 0x20.
#[test]
fn display_area_set_400line_preserves_slave_sync_f() {
    let code = make_int18h_call_ch(0x42, 0xC0); // 400-line ALL
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_slave.pitch, 40,
        "AH=42h 400-line should not change GDC slave pitch when PRXDUPD condition is not met"
    );
    assert_eq!(state.gdc_slave.aw, 40);
    assert_eq!(state.gdc_slave.al, 400);
}

/// Also verify the 400-line ALL mode path (CH=0xC0, crtmode=2) preserves sync
/// params when (prxdupd & 0x24) != 0x20.
#[test]
fn display_area_set_400line_preserves_slave_sync_vm() {
    let code = make_int18h_call_ch(0x42, 0xC0); // 400-line ALL
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_slave.pitch, 40,
        "AH=42h 400-line should not change GDC slave pitch when PRXDUPD condition is not met"
    );
    assert_eq!(state.gdc_slave.aw, 40);
    assert_eq!(state.gdc_slave.al, 400);
}

#[test]
fn display_area_set_400line_preserves_slave_sync_vx() {
    let code = make_int18h_call_ch(0x42, 0xC0);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.pitch, 40);
    assert_eq!(state.gdc_slave.aw, 40);
    assert_eq!(state.gdc_slave.al, 400);
}

#[test]
fn display_area_set_400line_preserves_slave_sync_ra() {
    let code = make_int18h_call_ch(0x42, 0xC0);
    let machine = boot_inject_run_ra(&[], &code, 2_000_000);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.pitch, 40);
    assert_eq!(state.gdc_slave.aw, 40);
    assert_eq!(state.gdc_slave.al, 400);
}

// ============================================================================
// §9.5 AH=4Ah - Draw Mode Set: GDC SYNC Mode Byte
// ============================================================================

#[test]
fn draw_mode_set_sync_mode_byte_f() {
    // AH=4Ah writes CH as the SYNC P1 mode byte to the GDC slave.
    // CH=0x02 -> param_buffer[0] = 0x02, display_mode=0x02.
    let code = make_int18h_call_ch(0x4A, 0x02);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_slave.param_buffer[0], 0x02,
        "AH=4Ah CH=0x02 should write 0x02 as SYNC P1 mode byte"
    );
    assert_eq!(state.gdc_slave.display_mode, DISPLAY_MODE_GRAPHICS);
}

#[test]
fn draw_mode_set_sync_mode_byte_vm() {
    // AH=4Ah writes CH as the SYNC P1 mode byte to the GDC slave.
    // CH=0x02 -> param_buffer[0] = 0x02, display_mode=0x02.
    let code = make_int18h_call_ch(0x4A, 0x02);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.gdc_slave.param_buffer[0], 0x02,
        "AH=4Ah CH=0x02 should write 0x02 as SYNC P1 mode byte"
    );
    assert_eq!(state.gdc_slave.display_mode, DISPLAY_MODE_GRAPHICS);
}

#[test]
fn draw_mode_set_sync_mode_byte_vx() {
    let code = make_int18h_call_ch(0x4A, 0x02);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.param_buffer[0], 0x02);
    assert_eq!(state.gdc_slave.display_mode, DISPLAY_MODE_GRAPHICS);
}

#[test]
fn draw_mode_set_sync_mode_byte_ra() {
    let code = make_int18h_call_ch(0x4A, 0x02);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.param_buffer[0], 0x02);
    assert_eq!(state.gdc_slave.display_mode, DISPLAY_MODE_GRAPHICS);
}

fn assert_draw_mode_preserves_slave_sync(state: &machine::MachineState) {
    assert_eq!(state.gdc_slave.pitch, 40);
    assert_eq!(state.gdc_slave.aw, 40);
    assert_eq!(state.gdc_slave.al, 400);
    assert_ne!(state.gdc_slave.display_period, 0);
}

#[test]
fn draw_mode_set_preserves_slave_sync_f() {
    let code = make_int18h_call_ch(0x4A, 0x02);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    assert_draw_mode_preserves_slave_sync(&machine.save_state());
}

#[test]
fn draw_mode_set_preserves_slave_sync_vm() {
    let code = make_int18h_call_ch(0x4A, 0x02);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    assert_draw_mode_preserves_slave_sync(&machine.save_state());
}

#[test]
fn draw_mode_set_preserves_slave_sync_vx() {
    let code = make_int18h_call_ch(0x4A, 0x02);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    assert_draw_mode_preserves_slave_sync(&machine.save_state());
}

#[test]
fn draw_mode_set_preserves_slave_sync_ra() {
    let code = make_int18h_call_ch(0x4A, 0x02);
    let machine = boot_inject_run_ra(&[], &code, 2_000_000);
    assert_draw_mode_preserves_slave_sync(&machine.save_state());
}

#[test]
fn draw_mode_set_sync_draw_on_retrace_f() {
    // CH=0x12 has bit 4 set -> draw_on_retrace should be true.
    let code = make_int18h_call_ch(0x4A, 0x12);
    let machine = boot_inject_run_f(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.param_buffer[0], 0x12);
    assert!(
        state.gdc_slave.draw_on_retrace,
        "AH=4Ah CH=0x12 (bit 4 set) should set draw_on_retrace"
    );
}

#[test]
fn draw_mode_set_sync_draw_on_retrace_vm() {
    // CH=0x12 has bit 4 set -> draw_on_retrace should be true.
    let code = make_int18h_call_ch(0x4A, 0x12);
    let machine = boot_inject_run_vm(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.param_buffer[0], 0x12);
    assert!(
        state.gdc_slave.draw_on_retrace,
        "AH=4Ah CH=0x12 (bit 4 set) should set draw_on_retrace"
    );
}

#[test]
fn draw_mode_set_sync_draw_on_retrace_vx() {
    let code = make_int18h_call_ch(0x4A, 0x12);
    let machine = boot_inject_run_vx(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.param_buffer[0], 0x12);
    assert!(state.gdc_slave.draw_on_retrace);
}

#[test]
fn draw_mode_set_sync_draw_on_retrace_ra() {
    let code = make_int18h_call_ch(0x4A, 0x12);
    let machine = boot_inject_run_ra(&[], &code, INT18H_BUDGET);
    let state = machine.save_state();
    assert_eq!(state.gdc_slave.param_buffer[0], 0x12);
    assert!(state.gdc_slave.draw_on_retrace);
}

/// AH=40h (graphics start) -> AH=42h CH=0xC0 (640x400) -> set CH explicitly -> draw call.
#[rustfmt::skip]
fn make_draw_test_code_with_ch(draw_ah: u8, draw_ch: u8) -> Vec<u8> {
    let dt_lo = (DATA_TABLE & 0xFF) as u8;
    let dt_hi = ((DATA_TABLE >> 8) & 0xFF) as u8;
    vec![
        0xB4, 0x40,             // MOV AH, 0x40 (graphics display start)
        0xCD, 0x18,             // INT 18h
        0xB4, 0x42,             // MOV AH, 0x42 (display area set)
        0xB5, 0xC0,             // MOV CH, 0xC0 (640x400)
        0xCD, 0x18,             // INT 18h
        0x31, 0xC0,             // XOR AX, AX
        0x8E, 0xD8,             // MOV DS, AX
        0xBB, dt_lo, dt_hi,     // MOV BX, DATA_TABLE
        0xB4, draw_ah,          // MOV AH, draw_ah
        0xB5, draw_ch,          // MOV CH, draw_ch
        0xCD, 0x18,             // INT 18h
        0xF4,                   // HLT
    ]
}

fn run_draw_test_vm_with_ch(ucw_data: &[u8], draw_ah: u8, draw_ch: u8) -> machine::MachineState {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, DATA_TABLE, ucw_data);
    let code = make_draw_test_code_with_ch(draw_ah, draw_ch);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(DRAW_BUDGET);
    machine.save_state()
}

fn make_ucw_horizontal_line_solid(
    gbon_ptn: u8,
    gbdotu: u8,
    x1: u16,
    y1: u16,
    x2: u16,
) -> [u8; 0x2A] {
    let mut ucw = [0u8; 0x2A];
    ucw[0x00] = gbon_ptn;
    ucw[0x02] = gbdotu;
    ucw[0x08] = (x1 & 0xFF) as u8;
    ucw[0x09] = (x1 >> 8) as u8;
    ucw[0x0A] = (y1 & 0xFF) as u8;
    ucw[0x0B] = (y1 >> 8) as u8;
    ucw[0x16] = (x2 & 0xFF) as u8;
    ucw[0x17] = (x2 >> 8) as u8;
    ucw[0x18] = (y1 & 0xFF) as u8;
    ucw[0x19] = (y1 >> 8) as u8;
    for i in 0..8 {
        ucw[0x20 + i] = 0xFF;
    }
    ucw[0x28] = 0x01; // GBDTYP: line
    ucw
}

#[test]
fn line_draw_byte_ordering_left_byte_holds_pixels_0_to_7() {
    let ucw = make_ucw_horizontal_line_solid(0x01, 0x00, 0, 0, 7);
    let state = run_draw_test_vm_with_ch(&ucw, 0x47, 0xC0);

    let mut pattern_found = false;
    for plane_offset in [0x0000usize, 0x8000, 0x10000] {
        let byte0 = state.memory.graphics_vram[plane_offset];
        let byte1 = state.memory.graphics_vram[plane_offset + 1];
        if byte0 == 0 && byte1 == 0 {
            continue;
        }
        assert_eq!(
            (byte0, byte1),
            (0xFF, 0x00),
            "plane at 0x{plane_offset:X}: line x=0..7 must occupy byte 0 (0xFF) leaving byte 1 untouched (0x00); got (0x{byte0:02X}, 0x{byte1:02X}) - high/low byte of word are swapped",
        );
        pattern_found = true;
    }
    assert!(
        pattern_found,
        "AH=47h line draw produced no VRAM writes in any of the B/R/G planes"
    );
}

#[test]
fn line_draw_byte_ordering_right_byte_holds_pixels_8_to_15() {
    let ucw = make_ucw_horizontal_line_solid(0x01, 0x00, 8, 0, 15);
    let state = run_draw_test_vm_with_ch(&ucw, 0x47, 0xC0);

    let mut pattern_found = false;
    for plane_offset in [0x0000usize, 0x8000, 0x10000] {
        let byte0 = state.memory.graphics_vram[plane_offset];
        let byte1 = state.memory.graphics_vram[plane_offset + 1];
        if byte0 == 0 && byte1 == 0 {
            continue;
        }
        assert_eq!(
            (byte0, byte1),
            (0x00, 0xFF),
            "plane at 0x{plane_offset:X}: line x=8..15 must occupy byte 1 (0xFF) leaving byte 0 untouched (0x00); got (0x{byte0:02X}, 0x{byte1:02X})",
        );
        pattern_found = true;
    }
    assert!(pattern_found, "AH=47h line draw produced no VRAM writes");
}

#[test]
fn line_draw_single_plane_set_operation_writes_selected_plane() {
    let ucw = make_ucw_horizontal_line_solid(0x00, 0x03, 0, 0, 7);
    let state = run_draw_test_vm_with_ch(&ucw, 0x47, 0x00);

    assert_eq!(
        state.memory.graphics_vram[0], 0xFF,
        "GBDOTU=3 should set pixels in the selected B plane"
    );
    assert_eq!(
        state.memory.graphics_vram[0x8000], 0x00,
        "CH=0 should not write the R plane"
    );
    assert_eq!(
        state.memory.graphics_vram[0x10000], 0x00,
        "CH=0 should not write the G plane"
    );
}

#[test]
fn line_draw_three_byte_horizontal_has_no_byte_aligned_gaps() {
    let ucw = make_ucw_horizontal_line_solid(0x01, 0x00, 16, 10, 39);
    let state = run_draw_test_vm_with_ch(&ucw, 0x47, 0xC0);

    let row_base = 10 * 80;
    for plane_offset in [0x0000usize, 0x8000, 0x10000] {
        let plane_row = plane_offset + row_base;
        let bytes: [u8; 8] = (0..8)
            .map(|i| state.memory.graphics_vram[plane_row + i])
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        if bytes.iter().all(|&b| b == 0) {
            continue;
        }
        assert_eq!(
            bytes,
            [0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00],
            "line x=16..39 at y=10 must fill exactly bytes 2..4 of the targeted plane; got {bytes:02X?} at plane 0x{plane_offset:X}, row offset 0x{row_base:X}. A byte-swap bug fills bytes 2,3,5 instead, leaving byte 4 empty - the 8-pixel gap visible in the rendered frame.",
        );
        return;
    }
    panic!("line draw produced no writes in any of B/R/G planes");
}

/// Verifies that INT 18h AH=4Dh CH=01h enables PEGC 256-color extended
/// graphics mode and that CH=00h returns to standard graphics mode.
#[test]
fn int18h_ah4d_extended_graphics_toggles_pegc_256_color_mode() {
    #[rustfmt::skip]
    let enable: &[u8] = &[
        0xB4, 0x4D, // MOV AH, 4Dh
        0xB5, 0x01, // MOV CH, 01h (extended graphics mode)
        0xCD, 0x18, // INT 18h
        0xF4,       // HLT
    ];
    let mut machine = boot_inject_run_pc9821as(&[], enable, INT18H_BUDGET);
    assert!(
        machine.cpu.halted(),
        "INT 18h AH=4Dh CH=01h program did not reach HLT",
    );
    assert_eq!(
        machine.bus.read_byte(0x054D) & 0x80,
        0x80,
        "INT 18h AH=4Dh CH=01h must set BDA 054Dh bit 7 (extended graphics mode flag)",
    );

    // With PEGC active, ports 0xA8..0xAE route to the 256-entry PEGC palette.
    // Indices 0x00 and 0x80 share the same low nibble, so a 16-color palette
    // would alias them onto entry 0; the 256-color palette keeps them distinct.
    machine.bus.io_write_byte(0xA8, 0x00);
    machine.bus.io_write_byte(0xAA, 0x11);
    machine.bus.io_write_byte(0xAC, 0x22);
    machine.bus.io_write_byte(0xAE, 0x33);
    machine.bus.io_write_byte(0xA8, 0x80);
    machine.bus.io_write_byte(0xAA, 0x44);
    machine.bus.io_write_byte(0xAC, 0x55);
    machine.bus.io_write_byte(0xAE, 0x66);

    machine.bus.io_write_byte(0xA8, 0x00);
    assert_eq!(machine.bus.io_read_byte(0xAA), 0x11);
    assert_eq!(machine.bus.io_read_byte(0xAC), 0x22);
    assert_eq!(machine.bus.io_read_byte(0xAE), 0x33);
    machine.bus.io_write_byte(0xA8, 0x80);
    assert_eq!(
        machine.bus.io_read_byte(0xAA),
        0x44,
        "palette[0x80] would alias palette[0x00] if PEGC 256-color mode were inactive",
    );
    assert_eq!(machine.bus.io_read_byte(0xAC), 0x55);
    assert_eq!(machine.bus.io_read_byte(0xAE), 0x66);

    // INT 18h AH=4Dh CH=00h returns to standard graphics mode.
    #[rustfmt::skip]
    let disable: &[u8] = &[
        0xB4, 0x4D, // MOV AH, 4Dh
        0xB5, 0x00, // MOV CH, 00h (standard graphics mode)
        0xCD, 0x18, // INT 18h
        0xF4,       // HLT
    ];
    write_bytes(&mut machine.bus, TEST_CODE, disable);
    machine.cpu.load_state(&{
        let mut state = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        state.set_esp(0x4000);
        state
    });
    machine.run_for(INT18H_BUDGET);
    assert!(machine.cpu.halted());
    assert_eq!(
        machine.bus.read_byte(0x054D) & 0x80,
        0x00,
        "INT 18h AH=4Dh CH=00h must clear BDA 054Dh bit 7",
    );
}
