use common::Bus;

use super::{
    boot_and_run_ra, boot_and_run_vm, boot_and_run_vx, create_machine_ra, read_ivt_vector,
    v86_monitor::{
        Register, assert_monitor_saw_imr_update, load_register, make_v86_call_code, run_real_mode,
        run_under_v86_monitor,
    },
};

/// Address of the master IMR that the real mode code stores.
const RESULT_IMR: u32 = 0x0600;
/// Address of the AH return code that the real mode code stores.
const RESULT_AH: u32 = 0x0601;
/// CPU cycle budget of one real mode run.
const RS232C_MASK_BUDGET: u64 = 2_000_000;

/// IR4 bit of the RS-232C receive interrupt in the master IMR.
const IR4: u8 = 0x10;
/// Master PIC IMR before a call that unmasks IR4.
const IMR_IR4_MASKED: u8 = 0xB5;
/// Master PIC IMR before a call that masks IR4.
const IMR_IR4_UNMASKED: u8 = 0x4A;

/// Offset of the buffer control block in segment 0000h.
const CONTROL_BLOCK: u16 = 0x8000;
/// Receive buffer size in bytes.
const BUFFER_SIZE: u16 = 0x0100;
/// AL of the initialization: 1200 bps.
const RATE_1200: u8 = 0x04;
/// 8251A mode word: asynchronous, x16, 8 data bits, no parity, 1 stop bit.
const MODE_8N1: u8 = 0x4E;
/// 8251A command word with receive enable: RTS, ER, RXE, DTR and TXE.
const COMMAND_RECEIVE_ON: u8 = 0x37;
/// 8251A command word without receive enable: RTS, ER, DTR and TXE.
const COMMAND_RECEIVE_OFF: u8 = 0x33;

/// Machine code that loads the registers of INT 19h AH=00h with `command`
/// and the control block at 0000:8000.
fn initialize_registers(command: u8) -> Vec<u8> {
    let mut code = Vec::new();
    code.extend_from_slice(&load_register(Register::Ax, u16::from(RATE_1200)));
    code.extend_from_slice(&load_register(Register::Bx, 0));
    code.extend_from_slice(&load_register(
        Register::Cx,
        u16::from_le_bytes([command, MODE_8N1]),
    ));
    code.extend_from_slice(&load_register(Register::Dx, BUFFER_SIZE));
    code.extend_from_slice(&load_register(Register::Di, CONTROL_BLOCK));
    code
}

/// Machine code that loads the registers of INT 19h AH=05h with `command`.
fn command_registers(command: u8) -> Vec<u8> {
    load_register(Register::Ax, 0x0500 | u16::from(command)).to_vec()
}

/// Real mode code: clears ES, runs `before`, sets the master IMR, loads
/// `call` and executes INT 19h, then stores the master IMR and AH.
#[rustfmt::skip]
fn make_real_mode_code(before: &[u8], imr: u8, call: &[u8]) -> Vec<u8> {
    let mut code = vec![
        0xFA,                   // CLI
        0x31, 0xC0,             // XOR AX, AX
        0x8E, 0xC0,             // MOV ES, AX
    ];
    code.extend_from_slice(before);
    code.extend_from_slice(&[
        0xB0, imr,              // MOV AL, imr
        0xE6, 0x02,             // OUT 0x02, AL
    ]);
    code.extend_from_slice(call);
    code.extend_from_slice(&[
        0xCD, 0x19,             // INT 0x19
        0x88, 0x26, 0x01, 0x06, // MOV [RESULT_AH], AH
        0xE4, 0x02,             // IN AL, 0x02
        0xA2, 0x00, 0x06,       // MOV [RESULT_IMR], AL
        0xF4,                   // HLT
    ]);
    code
}

/// Machine code that initializes the RS-232C BIOS with receive enable.
fn initialize_call() -> Vec<u8> {
    let mut code = initialize_registers(COMMAND_RECEIVE_ON);
    code.extend_from_slice(&[0xCD, 0x19]);
    code
}

/// Asserts the master IMR and the AH return code that the real mode code stored.
fn assert_stored_result(bus: &mut impl Bus, expected_imr: u8, case: &str) {
    let imr = bus.read_byte(RESULT_IMR);
    assert_eq!(
        imr, expected_imr,
        "{case}: master IMR {imr:#04X}, expected {expected_imr:#04X}"
    );
    assert_eq!(bus.read_byte(RESULT_AH), 0x00, "{case}: return code");
}

macro_rules! real_mode_imr_tests {
    ($boot_and_run:ident, $init_on:ident, $init_off:ident, $command_on:ident, $command_off:ident) => {
        #[test]
        fn $init_on() {
            let code = make_real_mode_code(
                &[],
                IMR_IR4_MASKED,
                &initialize_registers(COMMAND_RECEIVE_ON),
            );
            let (mut machine, _) = $boot_and_run(&code, &[], RS232C_MASK_BUDGET);
            assert_stored_result(&mut machine.bus, IMR_IR4_MASKED & !IR4, "AH=00h RXE=1");
        }

        #[test]
        fn $init_off() {
            let code = make_real_mode_code(
                &[],
                IMR_IR4_UNMASKED,
                &initialize_registers(COMMAND_RECEIVE_OFF),
            );
            let (mut machine, _) = $boot_and_run(&code, &[], RS232C_MASK_BUDGET);
            assert_stored_result(&mut machine.bus, IMR_IR4_UNMASKED | IR4, "AH=00h RXE=0");
        }

        #[test]
        fn $command_on() {
            let code = make_real_mode_code(
                &initialize_call(),
                IMR_IR4_MASKED,
                &command_registers(COMMAND_RECEIVE_ON),
            );
            let (mut machine, _) = $boot_and_run(&code, &[], RS232C_MASK_BUDGET);
            assert_stored_result(&mut machine.bus, IMR_IR4_MASKED & !IR4, "AH=05h RXE=1");
        }

        #[test]
        fn $command_off() {
            let code = make_real_mode_code(
                &initialize_call(),
                IMR_IR4_UNMASKED,
                &command_registers(COMMAND_RECEIVE_OFF),
            );
            let (mut machine, _) = $boot_and_run(&code, &[], RS232C_MASK_BUDGET);
            assert_stored_result(&mut machine.bus, IMR_IR4_UNMASKED | IR4, "AH=05h RXE=0");
        }
    };
}

real_mode_imr_tests!(
    boot_and_run_vm,
    int19h_initialize_receive_on_unmasks_only_ir4_vm,
    int19h_initialize_receive_off_masks_only_ir4_vm,
    int19h_command_receive_on_unmasks_only_ir4_vm,
    int19h_command_receive_off_masks_only_ir4_vm
);

real_mode_imr_tests!(
    boot_and_run_vx,
    int19h_initialize_receive_on_unmasks_only_ir4_vx,
    int19h_initialize_receive_off_masks_only_ir4_vx,
    int19h_command_receive_on_unmasks_only_ir4_vx,
    int19h_command_receive_off_masks_only_ir4_vx
);

real_mode_imr_tests!(
    boot_and_run_ra,
    int19h_initialize_receive_on_unmasks_only_ir4_ra,
    int19h_initialize_receive_off_masks_only_ir4_ra,
    int19h_command_receive_on_unmasks_only_ir4_ra,
    int19h_command_receive_off_masks_only_ir4_ra
);

/// Boots an RA and returns it with the IVT entry of INT 19h.
fn boot_ra_for_int19h() -> (machine_98::Pc9801Ra, (u16, u16)) {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    let state = machine.inspection_state();
    let handler = read_ivt_vector(&state.memory.ram, 0x19);
    (machine, handler)
}

/// Initializes the RS-232C BIOS in real mode with receive enable.
fn initialize_in_real_mode(machine: &mut machine_98::Pc9801Ra) {
    let code = make_real_mode_code(&[], 0xFF, &initialize_registers(COMMAND_RECEIVE_ON));
    run_real_mode(machine, &code);
    assert_eq!(machine.bus.read_byte(RESULT_AH), 0x00);
}

#[test]
fn int19h_initialize_imr_write_traps_in_v86_ra() {
    let (mut machine, handler) = boot_ra_for_int19h();
    let code = make_v86_call_code(&initialize_registers(COMMAND_RECEIVE_ON), handler);

    let accesses = run_under_v86_monitor(&mut machine, IMR_IR4_MASKED, &code);

    assert_monitor_saw_imr_update(
        &mut machine,
        &accesses,
        IMR_IR4_MASKED,
        IMR_IR4_MASKED & !IR4,
    );
}

#[test]
fn int19h_initialize_without_receive_imr_write_traps_in_v86_ra() {
    let (mut machine, handler) = boot_ra_for_int19h();
    let code = make_v86_call_code(&initialize_registers(COMMAND_RECEIVE_OFF), handler);

    let accesses = run_under_v86_monitor(&mut machine, IMR_IR4_UNMASKED, &code);

    assert_monitor_saw_imr_update(
        &mut machine,
        &accesses,
        IMR_IR4_UNMASKED,
        IMR_IR4_UNMASKED | IR4,
    );
}

#[test]
fn int19h_command_imr_write_traps_in_v86_ra() {
    let (mut machine, handler) = boot_ra_for_int19h();
    initialize_in_real_mode(&mut machine);
    let code = make_v86_call_code(&command_registers(COMMAND_RECEIVE_OFF), handler);

    let accesses = run_under_v86_monitor(&mut machine, IMR_IR4_UNMASKED, &code);

    assert_monitor_saw_imr_update(
        &mut machine,
        &accesses,
        IMR_IR4_UNMASKED,
        IMR_IR4_UNMASKED | IR4,
    );
}

#[test]
fn int19h_received_count_leaves_imr_untouched_in_v86_ra() {
    let (mut machine, handler) = boot_ra_for_int19h();
    initialize_in_real_mode(&mut machine);
    let code = make_v86_call_code(&load_register(Register::Ax, 0x0200), handler);

    let accesses = run_under_v86_monitor(&mut machine, IMR_IR4_MASKED, &code);

    assert_eq!(accesses, []);
    assert_eq!(machine.inspection_state().pic.chips[0].imr, IMR_IR4_MASKED);
}
