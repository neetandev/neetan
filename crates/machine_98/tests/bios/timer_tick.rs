use common::Bus;

use super::{
    CALLBACK_IRET, boot_and_run_f, boot_and_run_ra, boot_and_run_vm, boot_and_run_vx,
    create_machine_f, create_machine_ra, create_machine_vm, create_machine_vx, read_ivt_vector,
};

const RESULT: u32 = 0x0600;
const TIMER_BUDGET: u64 = 2_000_000;

// Callback that writes a marker byte (0xAA) to 0x0700, then IRET.
const CALLBACK_MARKER: &[u8] = &[
    0xC6, 0x06, 0x00, 0x07, 0xAA, // MOV BYTE [0x0700], 0xAA
    0xCF, // IRET
];

// Test code: set interval timer with given CX, wait for 1 tick, store CA_TIM_CNT.
#[rustfmt::skip]
fn make_single_tick_code(tick_count: u8) -> Vec<u8> {
    vec![
        // Mask all IRQs except IRQ0 (timer) so HLT only wakes on timer tick.
        0xB0, 0xFE,                         // MOV AL, 0xFE  (mask all except IRQ0)
        0xE6, 0x02,                         // OUT 0x02, AL  (write master PIC IMR)
        0x31, 0xC0,                         // XOR AX, AX
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x20,                   // MOV BX, 0x2000  (callback address)
        0xB9, tick_count, 0x00,             // MOV CX, tick_count
        0xB4, 0x02,                         // MOV AH, 0x02
        0xCD, 0x1C,                         // INT 0x1C  (set interval timer)
        0xFB,                               // STI
        0xF4,                               // HLT  (wait for 1 tick)
        0xFA,                               // CLI  (prevent further ticks)
        0xA1, 0x8A, 0x05,                   // MOV AX, [0x058A]
        0xA3, 0x00, 0x06,                   // MOV [RESULT], AX
        0xF4,                               // HLT
    ]
}

// Test code: set interval timer, wait for N ticks, store CA_TIM_CNT and marker.
#[rustfmt::skip]
fn make_multi_tick_code(tick_count: u8) -> Vec<u8> {
    let mut code = vec![
        // Mask all IRQs except IRQ0 (timer) so HLT only wakes on timer tick.
        0xB0, 0xFE,                         // MOV AL, 0xFE  (mask all except IRQ0)
        0xE6, 0x02,                         // OUT 0x02, AL  (write master PIC IMR)
        0x31, 0xC0,                         // XOR AX, AX
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x20,                   // MOV BX, 0x2000  (callback address)
        0xB9, tick_count, 0x00,             // MOV CX, tick_count
        0xB4, 0x02,                         // MOV AH, 0x02
        0xCD, 0x1C,                         // INT 0x1C  (set interval timer)
        0xFB,                               // STI
    ];
    code.extend(std::iter::repeat_n(0xF4, tick_count as usize)); // HLT (wait for tick)
    code.extend_from_slice(&[
        0xFA,                               // CLI  (prevent further ticks)
        0xA1, 0x8A, 0x05,                   // MOV AX, [0x058A]
        0xA3, 0x00, 0x06,                   // MOV [RESULT], AX
        0xA0, 0x00, 0x07,                   // MOV AL, [MARKER]
        0xA2, 0x02, 0x06,                   // MOV [RESULT+2], AL
        0xF4,                               // HLT
    ]);
    code
}

// ============================================================================
// §5 INT 08h - Timer Tick Vector Setup
// ============================================================================

#[test]
fn int08h_vector_f() {
    let mut machine = create_machine_f();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x08);
    assert!(
        segment >= 0xFD80,
        "INT 08h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int08h_vector_vm() {
    let mut machine = create_machine_vm();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x08);
    assert!(
        segment >= 0xFD80,
        "INT 08h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int08h_vector_vx() {
    let mut machine = create_machine_vx();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x08);
    assert!(
        segment >= 0xFD80,
        "INT 08h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int08h_vector_ra() {
    let mut machine = create_machine_ra();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x08);
    assert!(
        segment >= 0xFD80,
        "INT 08h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

// ============================================================================
// §5 Timer Tick - Single Decrement
// ============================================================================

#[test]
fn timer_tick_decrements_count_f() {
    let code = make_single_tick_code(5);
    let (mut machine, _cycles) = boot_and_run_f(&code, CALLBACK_IRET, TIMER_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 4, "CA_TIM_CNT should be 5-1=4 after one tick");
}

#[test]
fn timer_tick_decrements_count_vm() {
    let code = make_single_tick_code(5);
    let (mut machine, _cycles) = boot_and_run_vm(&code, CALLBACK_IRET, TIMER_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 4, "CA_TIM_CNT should be 5-1=4 after one tick");
}

#[test]
fn timer_tick_decrements_count_vx() {
    let code = make_single_tick_code(5);
    let (mut machine, _cycles) = boot_and_run_vx(&code, CALLBACK_IRET, TIMER_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 4, "CA_TIM_CNT should be 5-1=4 after one tick");
}

#[test]
fn timer_tick_decrements_count_ra() {
    let code = make_single_tick_code(5);
    let (mut machine, _cycles) = boot_and_run_ra(&code, CALLBACK_IRET, TIMER_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 4, "CA_TIM_CNT should be 5-1=4 after one tick");
}

// ============================================================================
// §5 Timer Tick - Multiple Decrements to Zero
// ============================================================================

#[test]
fn timer_tick_counts_down_to_zero_f() {
    let code = make_multi_tick_code(3);
    let (mut machine, _cycles) = boot_and_run_f(&code, CALLBACK_IRET, TIMER_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 0, "CA_TIM_CNT should be 0 after 3 ticks with CX=3");
}

#[test]
fn timer_tick_counts_down_to_zero_vm() {
    let code = make_multi_tick_code(3);
    let (mut machine, _cycles) = boot_and_run_vm(&code, CALLBACK_IRET, TIMER_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 0, "CA_TIM_CNT should be 0 after 3 ticks with CX=3");
}

#[test]
fn timer_tick_counts_down_to_zero_vx() {
    let code = make_multi_tick_code(3);
    let (mut machine, _cycles) = boot_and_run_vx(&code, CALLBACK_IRET, TIMER_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 0, "CA_TIM_CNT should be 0 after 3 ticks with CX=3");
}

#[test]
fn timer_tick_counts_down_to_zero_ra() {
    let code = make_multi_tick_code(3);
    let (mut machine, _cycles) = boot_and_run_ra(&code, CALLBACK_IRET, TIMER_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 0, "CA_TIM_CNT should be 0 after 3 ticks with CX=3");
}

// ============================================================================
// §5 Timer Tick - Callback Invocation
// ============================================================================

#[test]
fn timer_tick_fires_callback_at_zero_f() {
    let code = make_multi_tick_code(1);
    let (mut machine, _cycles) = boot_and_run_f(&code, CALLBACK_MARKER, TIMER_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 0, "CA_TIM_CNT should be 0 after callback");

    let marker = machine.bus.read_byte(RESULT + 2);
    assert_eq!(marker, 0xAA, "Callback should have written marker 0xAA");
}

#[test]
fn timer_tick_fires_callback_at_zero_vm() {
    let code = make_multi_tick_code(1);
    let (mut machine, _cycles) = boot_and_run_vm(&code, CALLBACK_MARKER, TIMER_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 0, "CA_TIM_CNT should be 0 after callback");

    let marker = machine.bus.read_byte(RESULT + 2);
    assert_eq!(marker, 0xAA, "Callback should have written marker 0xAA");
}

#[test]
fn timer_tick_fires_callback_at_zero_vx() {
    let code = make_multi_tick_code(1);
    let (mut machine, _cycles) = boot_and_run_vx(&code, CALLBACK_MARKER, TIMER_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 0, "CA_TIM_CNT should be 0 after callback");

    let marker = machine.bus.read_byte(RESULT + 2);
    assert_eq!(marker, 0xAA, "Callback should have written marker 0xAA");
}

#[test]
fn timer_tick_fires_callback_at_zero_ra() {
    let code = make_multi_tick_code(1);
    let (mut machine, _cycles) = boot_and_run_ra(&code, CALLBACK_MARKER, TIMER_BUDGET);

    let count = machine.bus.read_word(RESULT);
    assert_eq!(count, 0, "CA_TIM_CNT should be 0 after callback");

    let marker = machine.bus.read_byte(RESULT + 2);
    assert_eq!(marker, 0xAA, "Callback should have written marker 0xAA");
}

// ============================================================================
// §5 Timer Tick - EOI
// ============================================================================

#[test]
fn timer_tick_sends_eoi_f() {
    let code = make_single_tick_code(5);
    let (machine, _cycles) = boot_and_run_f(&code, CALLBACK_IRET, TIMER_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[0].isr & 0x01,
        0,
        "IRQ 0 should not be in-service after INT 08h (EOI was sent)"
    );
}

#[test]
fn timer_tick_sends_eoi_vm() {
    let code = make_single_tick_code(5);
    let (machine, _cycles) = boot_and_run_vm(&code, CALLBACK_IRET, TIMER_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[0].isr & 0x01,
        0,
        "IRQ 0 should not be in-service after INT 08h (EOI was sent)"
    );
}

#[test]
fn timer_tick_sends_eoi_vx() {
    let code = make_single_tick_code(5);
    let (machine, _cycles) = boot_and_run_vx(&code, CALLBACK_IRET, TIMER_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[0].isr & 0x01,
        0,
        "IRQ 0 should not be in-service after INT 08h (EOI was sent)"
    );
}

#[test]
fn timer_tick_sends_eoi_ra() {
    let code = make_single_tick_code(5);
    let (machine, _cycles) = boot_and_run_ra(&code, CALLBACK_IRET, TIMER_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[0].isr & 0x01,
        0,
        "IRQ 0 should not be in-service after INT 08h (EOI was sent)"
    );
}

// ============================================================================
// §5 Timer Tick - PIT Reload
// ============================================================================

#[test]
fn timer_tick_reloads_pit_f() {
    let code = make_single_tick_code(5);
    let (machine, _cycles) = boot_and_run_f(&code, CALLBACK_IRET, TIMER_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pit.channels[0].ctrl, 0x36,
        "PIT ch0 ctrl should be 0x36 (mode 3) after INT 08h reload"
    );
    // F BIOS uses 8MHz PIT clock lineage -> divider 0x4E00.
    assert_eq!(
        state.pit.channels[0].value, 0x4E00,
        "PIT ch0 value should be 0x4E00 (8MHz-lineage 10ms divider)"
    );
}

#[test]
fn timer_tick_reloads_pit_vm() {
    let code = make_single_tick_code(5);
    let (machine, _cycles) = boot_and_run_vm(&code, CALLBACK_IRET, TIMER_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pit.channels[0].ctrl, 0x36,
        "PIT ch0 ctrl should be 0x36 (mode 3) after INT 08h reload"
    );
    // VM BIOS uses 5MHz PIT clock lineage -> divider 0x6000.
    assert_eq!(
        state.pit.channels[0].value, 0x6000,
        "PIT ch0 value should be 0x6000 (5MHz-lineage 10ms divider)"
    );
}

#[test]
fn timer_tick_reloads_pit_vx() {
    let code = make_single_tick_code(5);
    let (machine, _cycles) = boot_and_run_vx(&code, CALLBACK_IRET, TIMER_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pit.channels[0].ctrl, 0x36,
        "PIT ch0 ctrl should be 0x36 (mode 3) after INT 08h reload"
    );
    // VX BIOS uses 5MHz PIT clock lineage -> divider 0x6000.
    assert_eq!(
        state.pit.channels[0].value, 0x6000,
        "PIT ch0 value should be 0x6000 (5MHz-lineage 10ms divider)"
    );
}

#[test]
fn timer_tick_reloads_pit_ra() {
    let code = make_single_tick_code(5);
    let (machine, _cycles) = boot_and_run_ra(&code, CALLBACK_IRET, TIMER_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pit.channels[0].ctrl, 0x36,
        "PIT ch0 ctrl should be 0x36 (mode 3) after INT 08h reload"
    );
    // RA BIOS uses 8MHz PIT clock lineage -> divider 0x4E00.
    assert_eq!(
        state.pit.channels[0].value, 0x4E00,
        "PIT ch0 value should be 0x4E00 (8MHz-lineage 10ms divider)"
    );
}

const PIT_PRESERVE_BUDGET: u64 = 5_000_000;

// Test code that programs PIT ch0 to a custom rate via direct I/O (simulating a
// game sound driver), then waits for a timer tick. When the interval timer
// expires (count reaches 0), INT 08H masks IRQ 0 and returns early without
// reloading PIT, so the custom rate is preserved.
#[rustfmt::skip]
fn make_pit_preserve_code(custom_divider: u16) -> Vec<u8> {
    let lo = custom_divider as u8;
    let hi = (custom_divider >> 8) as u8;
    vec![
        // First set up interval timer via INT 1CH so IRQ 0 is unmasked.
        0x31, 0xC0,                         // XOR AX, AX
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x20,                   // MOV BX, 0x2000  (callback address)
        0xB9, 0x01, 0x00,                   // MOV CX, 1
        0xB4, 0x02,                         // MOV AH, 0x02
        0xCD, 0x1C,                         // INT 0x1C  (set interval timer)

        // Now reprogram PIT ch0 to custom rate (simulating a game sound driver).
        0xB0, 0x36,                         // MOV AL, 0x36
        0xE6, 0x77,                         // OUT 0x77, AL  (PIT control: mode 3, LSB/MSB)
        0xB0, lo,                           // MOV AL, lo
        0xE6, 0x71,                         // OUT 0x71, AL  (PIT ch0 low byte)
        0xB0, hi,                           // MOV AL, hi
        0xE6, 0x71,                         // OUT 0x71, AL  (PIT ch0 high byte)

        // Wait for the timer tick (BIOS INT 08H handler will fire).
        0xFB,                               // STI
        0xF4,                               // HLT  (wait for timer tick)
        0xFA,                               // CLI  (prevent further ticks)

        0xA1, 0x8A, 0x05,                   // MOV AX, [0x058A]  -> CA_TIM_CNT
        0xA3, 0x00, 0x06,                   // MOV [RESULT], AX

        0xF4,                               // HLT
    ]
}

#[test]
fn int08h_preserves_custom_pit_rate_f() {
    let code = make_pit_preserve_code(0x2800);
    let (machine, _cycles) = boot_and_run_f(&code, CALLBACK_IRET, PIT_PRESERVE_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pit.channels[0].value, 0x2800,
        "INT 08H must preserve custom PIT ch0 divider on expiry (got {:#06X})",
        state.pit.channels[0].value
    );
    assert_eq!(
        state.pit.channels[0].ctrl, 0x36,
        "PIT ch0 ctrl should remain 0x36 (mode 3)"
    );
}

#[test]
fn int08h_preserves_custom_pit_rate_vm() {
    let code = make_pit_preserve_code(0x2800);
    let (machine, _cycles) = boot_and_run_vm(&code, CALLBACK_IRET, PIT_PRESERVE_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pit.channels[0].value, 0x2800,
        "INT 08H must preserve custom PIT ch0 divider on expiry (got {:#06X})",
        state.pit.channels[0].value
    );
    assert_eq!(
        state.pit.channels[0].ctrl, 0x36,
        "PIT ch0 ctrl should remain 0x36 (mode 3)"
    );
}

#[test]
fn int08h_preserves_custom_pit_rate_vx() {
    let code = make_pit_preserve_code(0x2800);
    let (machine, _cycles) = boot_and_run_vx(&code, CALLBACK_IRET, PIT_PRESERVE_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pit.channels[0].value, 0x2800,
        "INT 08H must preserve custom PIT ch0 divider on expiry (got {:#06X})",
        state.pit.channels[0].value
    );
    assert_eq!(
        state.pit.channels[0].ctrl, 0x36,
        "PIT ch0 ctrl should remain 0x36 (mode 3)"
    );
}

// Test that INT 08H masks IRQ 0 after the interval timer expires (count
// reaches 0). The callback or subsequent code will call INT 1CH AH=02H
// to restart the interval timer and unmask IRQ 0.
#[rustfmt::skip]
fn make_irq0_mask_on_expiry_code() -> Vec<u8> {
    vec![
        // Set up interval timer with count=1 (expires on first tick).
        0x31, 0xC0,                         // XOR AX, AX
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x20,                   // MOV BX, 0x2000  (callback address)
        0xB9, 0x01, 0x00,                   // MOV CX, 1
        0xB4, 0x02,                         // MOV AH, 0x02
        0xCD, 0x1C,                         // INT 0x1C  (set interval timer)

        // Wait for first tick (timer expires, callback fires).
        0xFB,                               // STI
        0xF4,                               // HLT
        0xFA,                               // CLI  (prevent further ticks)

        // Read master PIC IMR to check that IRQ 0 is now masked.
        0xE4, 0x02,                         // IN AL, 0x02
        0xA2, 0x00, 0x06,                   // MOV [RESULT], AL
        0xF4,                               // HLT
    ]
}

#[test]
fn int08h_masks_irq0_on_expiry_f() {
    let code = make_irq0_mask_on_expiry_code();
    let (mut machine, _cycles) = boot_and_run_f(&code, CALLBACK_IRET, PIT_PRESERVE_BUDGET);

    let imr = machine.bus.read_byte(RESULT);
    assert_ne!(
        imr & 0x01,
        0,
        "IRQ 0 should be masked in PIC IMR after interval timer expired (IMR={imr:#04X})"
    );
}

#[test]
fn int08h_masks_irq0_on_expiry_vm() {
    let code = make_irq0_mask_on_expiry_code();
    let (mut machine, _cycles) = boot_and_run_vm(&code, CALLBACK_IRET, PIT_PRESERVE_BUDGET);

    let imr = machine.bus.read_byte(RESULT);
    assert_ne!(
        imr & 0x01,
        0,
        "IRQ 0 should be masked in PIC IMR after interval timer expired (IMR={imr:#04X})"
    );
}

#[test]
fn int08h_masks_irq0_on_expiry_vx() {
    let code = make_irq0_mask_on_expiry_code();
    let (mut machine, _cycles) = boot_and_run_vx(&code, CALLBACK_IRET, PIT_PRESERVE_BUDGET);

    let imr = machine.bus.read_byte(RESULT);
    assert_ne!(
        imr & 0x01,
        0,
        "IRQ 0 should be masked in PIC IMR after interval timer expired (IMR={imr:#04X})"
    );
}

// ============================================================================
// §5 Timer Tick - PIT FLAG_I After Non-Expired Tick
// ============================================================================

// After a non-expired tick (count > 1), the INT 08H handler reloads PIT ch0 and
// must set FLAG_I so that the PIT continues to fire IRQ 0.
#[test]
fn int08h_pit_flag_i_after_tick_f() {
    let code = make_single_tick_code(5);
    let (machine, _cycles) = boot_and_run_f(&code, CALLBACK_IRET, TIMER_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.pit.channels[0].flag & 0x20,
        0,
        "PIT ch0 FLAG_I (0x20) should be set after non-expired tick (flag={:#04X})",
        state.pit.channels[0].flag
    );
}

#[test]
fn int08h_pit_flag_i_after_tick_vm() {
    let code = make_single_tick_code(5);
    let (machine, _cycles) = boot_and_run_vm(&code, CALLBACK_IRET, TIMER_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.pit.channels[0].flag & 0x20,
        0,
        "PIT ch0 FLAG_I (0x20) should be set after non-expired tick (flag={:#04X})",
        state.pit.channels[0].flag
    );
}

#[test]
fn int08h_pit_flag_i_after_tick_vx() {
    let code = make_single_tick_code(5);
    let (machine, _cycles) = boot_and_run_vx(&code, CALLBACK_IRET, TIMER_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.pit.channels[0].flag & 0x20,
        0,
        "PIT ch0 FLAG_I (0x20) should be set after non-expired tick (flag={:#04X})",
        state.pit.channels[0].flag
    );
}

#[test]
fn int08h_pit_flag_i_after_tick_ra() {
    let code = make_single_tick_code(5);
    let (machine, _cycles) = boot_and_run_ra(&code, CALLBACK_IRET, TIMER_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.pit.channels[0].flag & 0x20,
        0,
        "PIT ch0 FLAG_I (0x20) should be set after non-expired tick (flag={:#04X})",
        state.pit.channels[0].flag
    );
}

// ============================================================================
// §5 Timer Tick - Callback Preserves Caller Stack
// ============================================================================

// When INT 08H fires the callback (count expires), the caller's stack values
// must be preserved. The IRET frame chaining writes BELOW the existing frame.
#[rustfmt::skip]
fn make_callback_stack_preserve_code() -> Vec<u8> {
    vec![
        // Push sentinel values onto the stack.
        0xB8, 0xBE, 0xBA,                  // MOV AX, 0xBABE
        0x50,                              // PUSH AX
        0xB8, 0xFE, 0xCA,                  // MOV AX, 0xCAFE
        0x50,                              // PUSH AX

        // Mask all IRQs except IRQ0 (timer) so HLT only wakes on timer tick.
        0xB0, 0xFE,                         // MOV AL, 0xFE
        0xE6, 0x02,                         // OUT 0x02, AL  (write master PIC IMR)

        // Set interval timer with count=1 (expires on first tick).
        0x31, 0xC0,                         // XOR AX, AX
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x20,                   // MOV BX, 0x2000  (callback address)
        0xB9, 0x01, 0x00,                   // MOV CX, 1
        0xB4, 0x02,                         // MOV AH, 0x02
        0xCD, 0x1C,                         // INT 0x1C  (set interval timer)

        // Wait for first tick (timer expires, callback fires).
        0xFB,                               // STI
        0xF4,                               // HLT
        0xFA,                               // CLI  (prevent further ticks)

        // Pop sentinel values and store them in RESULT area.
        0x58,                               // POP AX  (should be 0xCAFE)
        0xA3, 0x00, 0x06,                   // MOV [RESULT], AX
        0x58,                               // POP AX  (should be 0xBABE)
        0xA3, 0x02, 0x06,                   // MOV [RESULT+2], AX
        0xF4,                               // HLT
    ]
}

#[test]
fn int08h_callback_preserves_caller_stack_f() {
    let code = make_callback_stack_preserve_code();
    let (mut machine, _cycles) = boot_and_run_f(&code, CALLBACK_MARKER, PIT_PRESERVE_BUDGET);

    let marker = machine.bus.read_byte(0x0700);
    assert_eq!(marker, 0xAA, "Callback should have fired (marker=0xAA)");

    let val1 = machine.bus.read_word(RESULT);
    let val2 = machine.bus.read_word(RESULT + 2);
    assert_eq!(
        val1, 0xCAFE,
        "First popped value should be 0xCAFE (got {val1:#06X})"
    );
    assert_eq!(
        val2, 0xBABE,
        "Second popped value should be 0xBABE (got {val2:#06X})"
    );
}

#[test]
fn int08h_callback_preserves_caller_stack_vm() {
    let code = make_callback_stack_preserve_code();
    let (mut machine, _cycles) = boot_and_run_vm(&code, CALLBACK_MARKER, PIT_PRESERVE_BUDGET);

    let marker = machine.bus.read_byte(0x0700);
    assert_eq!(marker, 0xAA, "Callback should have fired (marker=0xAA)");

    let val1 = machine.bus.read_word(RESULT);
    let val2 = machine.bus.read_word(RESULT + 2);
    assert_eq!(
        val1, 0xCAFE,
        "First popped value should be 0xCAFE (got {val1:#06X})"
    );
    assert_eq!(
        val2, 0xBABE,
        "Second popped value should be 0xBABE (got {val2:#06X})"
    );
}

#[test]
fn int08h_callback_preserves_caller_stack_vx() {
    let code = make_callback_stack_preserve_code();
    let (mut machine, _cycles) = boot_and_run_vx(&code, CALLBACK_MARKER, PIT_PRESERVE_BUDGET);

    let marker = machine.bus.read_byte(0x0700);
    assert_eq!(marker, 0xAA, "Callback should have fired (marker=0xAA)");

    let val1 = machine.bus.read_word(RESULT);
    let val2 = machine.bus.read_word(RESULT + 2);
    assert_eq!(
        val1, 0xCAFE,
        "First popped value should be 0xCAFE (got {val1:#06X})"
    );
    assert_eq!(
        val2, 0xBABE,
        "Second popped value should be 0xBABE (got {val2:#06X})"
    );
}

#[test]
fn int08h_callback_preserves_caller_stack_ra() {
    let code = make_callback_stack_preserve_code();
    let (mut machine, _cycles) = boot_and_run_ra(&code, CALLBACK_MARKER, PIT_PRESERVE_BUDGET);

    let marker = machine.bus.read_byte(0x0700);
    assert_eq!(marker, 0xAA, "Callback should have fired (marker=0xAA)");

    let val1 = machine.bus.read_word(RESULT);
    let val2 = machine.bus.read_word(RESULT + 2);
    assert_eq!(
        val1, 0xCAFE,
        "First popped value should be 0xCAFE (got {val1:#06X})"
    );
    assert_eq!(
        val2, 0xBABE,
        "Second popped value should be 0xBABE (got {val2:#06X})"
    );
}
