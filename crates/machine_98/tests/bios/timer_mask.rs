use common::Bus;

use super::{
    TEST_CALLBACK, boot_and_run_ra, boot_and_run_vm, boot_and_run_vx, create_machine_ra,
    read_ivt_vector,
    v86_monitor::{
        Register, assert_monitor_saw_imr_update, load_register, make_v86_call_code, run_real_mode,
        run_under_v86_monitor, store_word,
    },
    write_bytes,
};

/// Address of the master IMR that the real mode code stores.
const RESULT: u32 = 0x0600;
/// Address of the counter that the INT 07h callback increments.
const CALLBACK_COUNT: u32 = 0x0630;
/// CPU cycle budget of one test run.
const TIMER_MASK_BUDGET: u64 = 2_000_000;

/// Master PIC IMR before a call that unmasks IR0.
const IMR_IR0_MASKED: u8 = 0xA5;
/// Master PIC IMR before a call that masks IR0.
const IMR_IR0_UNMASKED: u8 = 0x5A;

/// Interval timer count that keeps the timer running for the whole test.
const LONG_COUNT: u16 = 0x1000;
/// Interval timer count that the next INT 08h decrements to zero.
const EXPIRING_COUNT: u16 = 1;
/// Interval timer count that the next INT 08h decrements to a non-zero value.
const RUNNING_COUNT: u16 = 5;

/// INT 07h callback: INC BYTE [0630h]; IRET.
const COUNTING_CALLBACK: &[u8] = &[0xFE, 0x06, 0x30, 0x06, 0xCF];

/// Real mode code: sets the master IMR, calls INT `vector` with AH=`function`,
/// CX=`count` and ES:BX=0000:2000, then stores the master IMR at RESULT.
#[rustfmt::skip]
fn make_real_mode_call_code(imr: u8, vector: u8, function: u8, count: u16) -> Vec<u8> {
    vec![
        0xFA,                                       // CLI
        0xB0, imr,                                  // MOV AL, imr
        0xE6, 0x02,                                 // OUT 0x02, AL
        0x31, 0xC0,                                 // XOR AX, AX
        0x8E, 0xC0,                                 // MOV ES, AX
        0xBB, 0x00, 0x20,                           // MOV BX, 0x2000
        0xB9, count as u8, (count >> 8) as u8,      // MOV CX, count
        0xB4, function,                             // MOV AH, function
        0xCD, vector,                               // INT vector
        0xE4, 0x02,                                 // IN AL, 0x02
        0xA2, 0x00, 0x06,                           // MOV [RESULT], AL
        0xF4,                                       // HLT
    ]
}

/// Real mode code: starts the interval timer with `count`, sets the master
/// IMR, calls INT 08h and stores the master IMR at RESULT.
#[rustfmt::skip]
fn make_real_mode_tick_code(imr: u8, count: u16) -> Vec<u8> {
    vec![
        0xFA,                                       // CLI
        0x31, 0xC0,                                 // XOR AX, AX
        0x8E, 0xC0,                                 // MOV ES, AX
        0xBB, 0x00, 0x20,                           // MOV BX, 0x2000
        0xB9, count as u8, (count >> 8) as u8,      // MOV CX, count
        0xB4, 0x02,                                 // MOV AH, 0x02
        0xCD, 0x1C,                                 // INT 0x1C
        0xB0, imr,                                  // MOV AL, imr
        0xE6, 0x02,                                 // OUT 0x02, AL
        0xCD, 0x08,                                 // INT 0x08
        0xE4, 0x02,                                 // IN AL, 0x02
        0xA2, 0x00, 0x06,                           // MOV [RESULT], AL
        0xF4,                                       // HLT
    ]
}

/// Asserts the master IMR that the real mode code stored at RESULT.
fn assert_stored_imr(bus: &mut impl Bus, expected: u8, case: &str) {
    let imr = bus.read_byte(RESULT);
    assert_eq!(
        imr, expected,
        "{case}: master IMR {imr:#04X}, expected {expected:#04X}"
    );
}

macro_rules! real_mode_imr_tests {
    ($boot_and_run:ident, $set:ident, $continue:ident, $tick:ident, $expiry:ident) => {
        #[test]
        fn $set() {
            let code = make_real_mode_call_code(IMR_IR0_MASKED, 0x1C, 0x02, LONG_COUNT);
            let (mut machine, _) = $boot_and_run(&code, COUNTING_CALLBACK, TIMER_MASK_BUDGET);
            assert_stored_imr(&mut machine.bus, IMR_IR0_MASKED & !0x01, "INT 1Ch AH=02h");
        }

        #[test]
        fn $continue() {
            let code = make_real_mode_call_code(IMR_IR0_MASKED, 0x1C, 0x03, 0);
            let (mut machine, _) = $boot_and_run(&code, COUNTING_CALLBACK, TIMER_MASK_BUDGET);
            assert_stored_imr(&mut machine.bus, IMR_IR0_MASKED & !0x01, "INT 1Ch AH=03h");
        }

        #[test]
        fn $tick() {
            let code = make_real_mode_tick_code(IMR_IR0_MASKED, RUNNING_COUNT);
            let (mut machine, _) = $boot_and_run(&code, COUNTING_CALLBACK, TIMER_MASK_BUDGET);
            assert_stored_imr(&mut machine.bus, IMR_IR0_MASKED & !0x01, "INT 08h tick");
            assert_eq!(machine.bus.read_byte(CALLBACK_COUNT), 0);
        }

        #[test]
        fn $expiry() {
            let code = make_real_mode_tick_code(IMR_IR0_UNMASKED, EXPIRING_COUNT);
            let (mut machine, _) = $boot_and_run(&code, COUNTING_CALLBACK, TIMER_MASK_BUDGET);
            assert_stored_imr(&mut machine.bus, IMR_IR0_UNMASKED | 0x01, "INT 08h expiry");
            assert_eq!(machine.bus.read_byte(CALLBACK_COUNT), 1);
        }
    };
}

real_mode_imr_tests!(
    boot_and_run_vm,
    int1ch_set_interval_timer_unmasks_only_ir0_vm,
    int1ch_continue_interval_timer_unmasks_only_ir0_vm,
    int08h_tick_unmasks_only_ir0_vm,
    int08h_expiry_masks_only_ir0_vm
);

real_mode_imr_tests!(
    boot_and_run_vx,
    int1ch_set_interval_timer_unmasks_only_ir0_vx,
    int1ch_continue_interval_timer_unmasks_only_ir0_vx,
    int08h_tick_unmasks_only_ir0_vx,
    int08h_expiry_masks_only_ir0_vx
);

real_mode_imr_tests!(
    boot_and_run_ra,
    int1ch_set_interval_timer_unmasks_only_ir0_ra,
    int1ch_continue_interval_timer_unmasks_only_ir0_ra,
    int08h_tick_unmasks_only_ir0_ra,
    int08h_expiry_masks_only_ir0_ra
);

/// Boots an RA and returns it with the IVT entry of `vector`.
fn boot_ra_for_vector(vector: u8) -> (machine_98::Pc9801Ra, (u16, u16)) {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CALLBACK, COUNTING_CALLBACK);
    let state = machine.inspection_state();
    let handler = read_ivt_vector(&state.memory.ram, vector);
    (machine, handler)
}

/// BIOS work area word of the interval timer count.
const INTERVAL_TIMER_COUNT: u16 = 0x058A;

/// V86 code: optionally writes the interval timer count, loads AX, BX and CX
/// and calls the handler at `handler` like an interrupt.
fn make_timer_v86_call_code(
    timer_count: Option<u16>,
    ax: u16,
    bx: u16,
    cx: u16,
    handler: (u16, u16),
) -> Vec<u8> {
    let mut setup = Vec::new();
    if let Some(count) = timer_count {
        setup.extend_from_slice(&store_word(INTERVAL_TIMER_COUNT, count));
    }
    setup.extend_from_slice(&load_register(Register::Ax, ax));
    setup.extend_from_slice(&load_register(Register::Bx, bx));
    setup.extend_from_slice(&load_register(Register::Cx, cx));
    make_v86_call_code(&setup, handler)
}

/// Starts the interval timer in real mode with a count that does not expire.
fn start_interval_timer(machine: &mut machine_98::Pc9801Ra) {
    let code = make_real_mode_call_code(0xFF, 0x1C, 0x02, LONG_COUNT);
    run_real_mode(machine, &code);
}

#[test]
fn int1ch_set_interval_timer_imr_write_traps_in_v86_ra() {
    let (mut machine, handler) = boot_ra_for_vector(0x1C);
    let code = make_timer_v86_call_code(None, 0x0200, TEST_CALLBACK as u16, LONG_COUNT, handler);

    let accesses = run_under_v86_monitor(&mut machine, IMR_IR0_MASKED, &code);

    assert_monitor_saw_imr_update(
        &mut machine,
        &accesses,
        IMR_IR0_MASKED,
        IMR_IR0_MASKED & !0x01,
    );
}

#[test]
fn int1ch_continue_interval_timer_imr_write_traps_in_v86_ra() {
    let (mut machine, handler) = boot_ra_for_vector(0x1C);
    let code = make_timer_v86_call_code(None, 0x0300, 0, 0, handler);

    let accesses = run_under_v86_monitor(&mut machine, IMR_IR0_MASKED, &code);

    assert_monitor_saw_imr_update(
        &mut machine,
        &accesses,
        IMR_IR0_MASKED,
        IMR_IR0_MASKED & !0x01,
    );
}

#[test]
fn int1ch_get_datetime_leaves_imr_untouched_in_v86_ra() {
    let (mut machine, handler) = boot_ra_for_vector(0x1C);
    let code = make_timer_v86_call_code(None, 0x0000, RESULT as u16, 0, handler);

    let accesses = run_under_v86_monitor(&mut machine, IMR_IR0_MASKED, &code);

    assert_eq!(accesses, []);
    assert_eq!(machine.inspection_state().pic.chips[0].imr, IMR_IR0_MASKED);
}

#[test]
fn int08h_tick_imr_write_traps_in_v86_ra() {
    let (mut machine, handler) = boot_ra_for_vector(0x08);
    start_interval_timer(&mut machine);
    let code = make_timer_v86_call_code(Some(RUNNING_COUNT), 0, 0, 0, handler);

    let accesses = run_under_v86_monitor(&mut machine, IMR_IR0_MASKED, &code);

    assert_monitor_saw_imr_update(
        &mut machine,
        &accesses,
        IMR_IR0_MASKED,
        IMR_IR0_MASKED & !0x01,
    );
    assert_eq!(machine.bus.read_byte(CALLBACK_COUNT), 0);
}

#[test]
fn int08h_expiry_imr_write_traps_in_v86_ra() {
    let (mut machine, handler) = boot_ra_for_vector(0x08);
    start_interval_timer(&mut machine);
    let code = make_timer_v86_call_code(Some(EXPIRING_COUNT), 0, 0, 0, handler);

    let accesses = run_under_v86_monitor(&mut machine, IMR_IR0_UNMASKED, &code);

    assert_monitor_saw_imr_update(
        &mut machine,
        &accesses,
        IMR_IR0_UNMASKED,
        IMR_IR0_UNMASKED | 0x01,
    );
    assert_eq!(machine.bus.read_byte(CALLBACK_COUNT), 1);
}
