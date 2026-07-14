use super::{
    KB_BUFFER_START, KB_COUNT, KB_HEAD, KB_SHIFT_STATE, KB_STATUS_START, KB_TAIL,
    boot_inject_run_f, boot_inject_run_ra, boot_inject_run_vm, boot_inject_run_vx,
    create_machine_f, create_machine_ra, create_machine_vm, create_machine_vx, make_sti_hlt_code,
    read_ivt_vector, read_ram_u16,
};

const KEYBOARD_BUDGET: u64 = 500_000;

// ============================================================================
// §6 INT 09h - Vector Setup
// ============================================================================

#[test]
fn int09h_vector_f() {
    let mut machine = create_machine_f();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x09);
    assert!(
        segment >= 0xFD80,
        "INT 09h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int09h_vector_vm() {
    let mut machine = create_machine_vm();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x09);
    assert!(
        segment >= 0xFD80,
        "INT 09h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int09h_vector_vx() {
    let mut machine = create_machine_vx();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x09);
    assert!(
        segment >= 0xFD80,
        "INT 09h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int09h_vector_ra() {
    let mut machine = create_machine_ra();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x09);
    assert!(
        segment >= 0xFD80,
        "INT 09h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

// ============================================================================
// §6.1 Key Press - Status Bit Set
// ============================================================================

// Scancode 0x1C (Enter make): pos = 0x1C >> 3 = 3, bit = 1 << 4 = 0x10.
// Status byte at 0x052A + 3 = 0x052D.

#[test]
fn key_press_sets_status_bit_f() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_f(&[0x1C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[KB_STATUS_START + 3] & 0x10,
        0,
        "Enter key (0x1C) should set bit 4 in status group 3"
    );
}

#[test]
fn key_press_sets_status_bit_vm() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_vm(&[0x1C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[KB_STATUS_START + 3] & 0x10,
        0,
        "Enter key (0x1C) should set bit 4 in status group 3"
    );
}

#[test]
fn key_press_sets_status_bit_vx() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_vx(&[0x1C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[KB_STATUS_START + 3] & 0x10,
        0,
        "Enter key (0x1C) should set bit 4 in status group 3"
    );
}

#[test]
fn key_press_sets_status_bit_ra() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_ra(&[0x1C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[KB_STATUS_START + 3] & 0x10,
        0,
        "Enter key (0x1C) should set bit 4 in status group 3"
    );
}

// ============================================================================
// §6.1 Key Release - Status Bit Clear
// ============================================================================

#[test]
fn key_release_clears_status_bit_f() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_f(&[0x1C, 0x9C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_STATUS_START + 3] & 0x10,
        0,
        "Enter release (0x9C) should clear bit 4 in status group 3"
    );
}

#[test]
fn key_release_clears_status_bit_vm() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_vm(&[0x1C, 0x9C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_STATUS_START + 3] & 0x10,
        0,
        "Enter release (0x9C) should clear bit 4 in status group 3"
    );
}

#[test]
fn key_release_clears_status_bit_vx() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_vx(&[0x1C, 0x9C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_STATUS_START + 3] & 0x10,
        0,
        "Enter release (0x9C) should clear bit 4 in status group 3"
    );
}

#[test]
fn key_release_clears_status_bit_ra() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_ra(&[0x1C, 0x9C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_STATUS_START + 3] & 0x10,
        0,
        "Enter release (0x9C) should clear bit 4 in status group 3"
    );
}

// ============================================================================
// §6.2 Shift Press - Shift State Set
// ============================================================================

// Scancode 0x70 (left SHIFT make): pos = 14, bit = 0x01.
// Status byte at 0x052A + 14 = 0x0538. Shift state bit 0 at 0x053A.

#[test]
fn shift_press_sets_shift_state_f() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_f(&[0x70], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[KB_SHIFT_STATE] & 0x01,
        0,
        "SHIFT press should set shift state bit 0"
    );
    assert_ne!(
        state.memory.ram[KB_STATUS_START + 14] & 0x01,
        0,
        "SHIFT press should set key status bit"
    );
}

#[test]
fn shift_press_sets_shift_state_vm() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_vm(&[0x70], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[KB_SHIFT_STATE] & 0x01,
        0,
        "SHIFT press should set shift state bit 0"
    );
    assert_ne!(
        state.memory.ram[KB_STATUS_START + 14] & 0x01,
        0,
        "SHIFT press should set key status bit"
    );
}

#[test]
fn shift_press_sets_shift_state_vx() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_vx(&[0x70], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[KB_SHIFT_STATE] & 0x01,
        0,
        "SHIFT press should set shift state bit 0"
    );
    assert_ne!(
        state.memory.ram[KB_STATUS_START + 14] & 0x01,
        0,
        "SHIFT press should set key status bit"
    );
}

#[test]
fn shift_press_sets_shift_state_ra() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_ra(&[0x70], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[KB_SHIFT_STATE] & 0x01,
        0,
        "SHIFT press should set shift state bit 0"
    );
    assert_ne!(
        state.memory.ram[KB_STATUS_START + 14] & 0x01,
        0,
        "SHIFT press should set key status bit"
    );
}

// ============================================================================
// §6.2 Shift Release - Shift State Clear
// ============================================================================

#[test]
fn shift_release_clears_shift_state_f() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_f(&[0x70, 0xF0], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_SHIFT_STATE] & 0x01,
        0,
        "SHIFT release should clear shift state bit 0"
    );
    assert_eq!(
        state.memory.ram[KB_STATUS_START + 14] & 0x01,
        0,
        "SHIFT release should clear key status bit"
    );
}

#[test]
fn shift_release_clears_shift_state_vm() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_vm(&[0x70, 0xF0], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_SHIFT_STATE] & 0x01,
        0,
        "SHIFT release should clear shift state bit 0"
    );
    assert_eq!(
        state.memory.ram[KB_STATUS_START + 14] & 0x01,
        0,
        "SHIFT release should clear key status bit"
    );
}

#[test]
fn shift_release_clears_shift_state_vx() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_vx(&[0x70, 0xF0], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_SHIFT_STATE] & 0x01,
        0,
        "SHIFT release should clear shift state bit 0"
    );
    assert_eq!(
        state.memory.ram[KB_STATUS_START + 14] & 0x01,
        0,
        "SHIFT release should clear key status bit"
    );
}

#[test]
fn shift_release_clears_shift_state_ra() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_ra(&[0x70, 0xF0], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_SHIFT_STATE] & 0x01,
        0,
        "SHIFT release should clear shift state bit 0"
    );
    assert_eq!(
        state.memory.ram[KB_STATUS_START + 14] & 0x01,
        0,
        "SHIFT release should clear key status bit"
    );
}

// ============================================================================
// §6.2 CTRL - Shift State Bit 4
// ============================================================================

// Scancode 0x74 (CTRL make): pos = 14, bit = 0x10. Shift state bit 4 at 0x053A.

#[test]
fn ctrl_press_sets_shift_state_f() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_f(&[0x74], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[KB_SHIFT_STATE] & 0x10,
        0,
        "CTRL press should set shift state bit 4"
    );
}

#[test]
fn ctrl_press_sets_shift_state_vm() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_vm(&[0x74], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[KB_SHIFT_STATE] & 0x10,
        0,
        "CTRL press should set shift state bit 4"
    );
}

#[test]
fn ctrl_press_sets_shift_state_vx() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_vx(&[0x74], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[KB_SHIFT_STATE] & 0x10,
        0,
        "CTRL press should set shift state bit 4"
    );
}

#[test]
fn ctrl_press_sets_shift_state_ra() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_ra(&[0x74], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[KB_SHIFT_STATE] & 0x10,
        0,
        "CTRL press should set shift state bit 4"
    );
}

// ============================================================================
// §6.3 Key Buffer - Single Key Press
// ============================================================================

#[test]
fn key_press_buffers_code_f() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_f(&[0x1C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 1,
        "KB_COUNT should be 1 after one key press"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0504,
        "KB_TAIL should advance by 2"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_HEAD),
        0x0502,
        "KB_HEAD should remain at start"
    );
    let entry = read_ram_u16(&state.memory.ram, KB_BUFFER_START);
    assert_ne!(entry, 0x0000, "Buffer entry should be non-zero");
}

#[test]
fn key_press_buffers_code_vm() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_vm(&[0x1C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 1,
        "KB_COUNT should be 1 after one key press"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0504,
        "KB_TAIL should advance by 2"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_HEAD),
        0x0502,
        "KB_HEAD should remain at start"
    );
    let entry = read_ram_u16(&state.memory.ram, KB_BUFFER_START);
    assert_ne!(entry, 0x0000, "Buffer entry should be non-zero");
}

#[test]
fn key_press_buffers_code_vx() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_vx(&[0x1C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 1,
        "KB_COUNT should be 1 after one key press"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0504,
        "KB_TAIL should advance by 2"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_HEAD),
        0x0502,
        "KB_HEAD should remain at start"
    );
    let entry = read_ram_u16(&state.memory.ram, KB_BUFFER_START);
    assert_ne!(entry, 0x0000, "Buffer entry should be non-zero");
}

#[test]
fn key_press_buffers_code_ra() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_ra(&[0x1C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 1,
        "KB_COUNT should be 1 after one key press"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0504,
        "KB_TAIL should advance by 2"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_HEAD),
        0x0502,
        "KB_HEAD should remain at start"
    );
    let entry = read_ram_u16(&state.memory.ram, KB_BUFFER_START);
    assert_ne!(entry, 0x0000, "Buffer entry should be non-zero");
}

// ============================================================================
// §6.3 Key Buffer - Modifier Does Not Buffer
// ============================================================================

#[test]
fn modifier_press_does_not_buffer_f() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_f(&[0x70], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 0,
        "SHIFT press should not add to keyboard buffer"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0502,
        "KB_TAIL should not advance for modifier key"
    );
}

#[test]
fn modifier_press_does_not_buffer_vm() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_vm(&[0x70], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 0,
        "SHIFT press should not add to keyboard buffer"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0502,
        "KB_TAIL should not advance for modifier key"
    );
}

#[test]
fn modifier_press_does_not_buffer_vx() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_vx(&[0x70], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 0,
        "SHIFT press should not add to keyboard buffer"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0502,
        "KB_TAIL should not advance for modifier key"
    );
}

#[test]
fn modifier_press_does_not_buffer_ra() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_ra(&[0x70], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 0,
        "SHIFT press should not add to keyboard buffer"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0502,
        "KB_TAIL should not advance for modifier key"
    );
}

// ============================================================================
// §6.3 Key Buffer - Release Does Not Buffer
// ============================================================================

#[test]
fn key_release_does_not_buffer_f() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_f(&[0x1C, 0x9C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 1,
        "Only the key press should buffer (not the release)"
    );
}

#[test]
fn key_release_does_not_buffer_vm() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_vm(&[0x1C, 0x9C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 1,
        "Only the key press should buffer (not the release)"
    );
}

#[test]
fn key_release_does_not_buffer_vx() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_vx(&[0x1C, 0x9C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 1,
        "Only the key press should buffer (not the release)"
    );
}

#[test]
fn key_release_does_not_buffer_ra() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_ra(&[0x1C, 0x9C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 1,
        "Only the key press should buffer (not the release)"
    );
}

// ============================================================================
// §6.3 Key Buffer - Multiple Keys
// ============================================================================

#[test]
fn multiple_keys_buffer_correctly_f() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_f(&[0x1C, 0x1E], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 2,
        "KB_COUNT should be 2 after two key presses"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0506,
        "KB_TAIL should advance by 4 (two entries)"
    );
}

#[test]
fn multiple_keys_buffer_correctly_vm() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_vm(&[0x1C, 0x1E], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 2,
        "KB_COUNT should be 2 after two key presses"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0506,
        "KB_TAIL should advance by 4 (two entries)"
    );
}

#[test]
fn multiple_keys_buffer_correctly_vx() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_vx(&[0x1C, 0x1E], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 2,
        "KB_COUNT should be 2 after two key presses"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0506,
        "KB_TAIL should advance by 4 (two entries)"
    );
}

#[test]
fn multiple_keys_buffer_correctly_ra() {
    let code = make_sti_hlt_code(2);
    let machine = boot_inject_run_ra(&[0x1C, 0x1E], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[KB_COUNT], 2,
        "KB_COUNT should be 2 after two key presses"
    );
    assert_eq!(
        read_ram_u16(&state.memory.ram, KB_TAIL),
        0x0506,
        "KB_TAIL should advance by 4 (two entries)"
    );
}

// ============================================================================
// §6 INT 09h - EOI
// ============================================================================

#[test]
fn keyboard_sends_eoi_f() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_f(&[0x1C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[0].isr & 0x02,
        0,
        "IRQ 1 should not be in-service after INT 09h (EOI was sent)"
    );
}

#[test]
fn keyboard_sends_eoi_vm() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_vm(&[0x1C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[0].isr & 0x02,
        0,
        "IRQ 1 should not be in-service after INT 09h (EOI was sent)"
    );
}

#[test]
fn keyboard_sends_eoi_vx() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_vx(&[0x1C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[0].isr & 0x02,
        0,
        "IRQ 1 should not be in-service after INT 09h (EOI was sent)"
    );
}

#[test]
fn keyboard_sends_eoi_ra() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_ra(&[0x1C], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[0].isr & 0x02,
        0,
        "IRQ 1 should not be in-service after INT 09h (EOI was sent)"
    );
}

// ============================================================================
// §6.3 MSW6 - Shift Key Updates Text VRAM LED State
// ============================================================================

// Real BIOS does not update MSW6 LED bits on SHIFT key press.

#[test]
fn shift_does_not_update_msw6_in_text_vram_f() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_f(&[0x70], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.text_vram[0x3FF6] & 0xE0,
        0,
        "SHIFT press should not update MSW6 LED bits in text VRAM"
    );
}

#[test]
fn shift_does_not_update_msw6_in_text_vram_vm() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_vm(&[0x70], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.text_vram[0x3FF6] & 0xE0,
        0,
        "SHIFT press should not update MSW6 LED bits in text VRAM"
    );
}

#[test]
fn shift_does_not_update_msw6_in_text_vram_vx() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_vx(&[0x70], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.text_vram[0x3FF6] & 0xE0,
        0,
        "SHIFT press should not update MSW6 LED bits in text VRAM"
    );
}

#[test]
fn shift_does_not_update_msw6_in_text_vram_ra() {
    let code = make_sti_hlt_code(1);
    let machine = boot_inject_run_ra(&[0x70], &code, KEYBOARD_BUDGET);
    let state = machine.save_state();
    assert_eq!(
        state.memory.text_vram[0x3FF6] & 0xE0,
        0,
        "SHIFT press should not update MSW6 LED bits in text VRAM"
    );
}
