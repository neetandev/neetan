use common::Bus;

use super::{
    TEST_CODE, boot_and_run_vm, create_machine_pc9821as_hdd, create_machine_vm_hdd,
    v86_monitor::{Register, load_register, make_rom_entry_call},
    write_bytes,
};

/// Address of the master IMR that the real mode code stores.
const RESULT_IMR: u32 = 0x0600;
/// CPU cycle budget of one real mode run.
const DISK_INIT_BUDGET: u64 = 2_000_000;
/// Master PIC IMR before a disk initialization.
const IMR_IR0_MASKED: u8 = 0xA5;
/// Real mode address of the INT 1Bh entry of the SASI expansion ROM.
const SASI_ROM_INT1BH_ENTRY: (u16, u16) = (0xD700, 0x0018);
/// AX of INT 1Bh AH=03h for the first SASI or IDE drive.
const HARD_DISK_INITIALIZE: u16 = 0x0380;
/// AX of INT 1Bh AH=03h for the 1 MB floppy drives.
const FLOPPY_INITIALIZE: u16 = 0x0390;

/// Size of the code that `make_real_mode_code` places before the call.
const REAL_MODE_PREFIX_SIZE: u16 = 5;

/// Real mode code: sets the master IMR, runs `call` and stores the master IMR.
#[rustfmt::skip]
fn make_real_mode_code(call: &[u8]) -> Vec<u8> {
    let mut code = vec![
        0xFA,                   // CLI
        0xB0, IMR_IR0_MASKED,   // MOV AL, IMR_IR0_MASKED
        0xE6, 0x02,             // OUT 0x02, AL
    ];
    code.extend_from_slice(call);
    code.extend_from_slice(&[
        0xFA,                   // CLI
        0xE4, 0x02,             // IN AL, 0x02
        0xA2, 0x00, 0x06,       // MOV [RESULT_IMR], AL
        0xF4,                   // HLT
    ]);
    code
}

/// Machine code of INT 1Bh with `ax`.
fn int1bh_call(ax: u16) -> Vec<u8> {
    let mut code = load_register(Register::Ax, ax).to_vec();
    code.extend_from_slice(&[0xCD, 0x1B]);
    code
}

/// Machine code at the call position of `make_real_mode_code` that calls the
/// INT 1Bh entry of the SASI expansion ROM with the register frame of the
/// system BIOS and AX=`ax`.
fn sasi_rom_entry_call(ax: u16) -> Vec<u8> {
    make_rom_entry_call(
        &load_register(Register::Ax, ax),
        SASI_ROM_INT1BH_ENTRY,
        TEST_CODE as u16 + REAL_MODE_PREFIX_SIZE,
    )
}

/// Asserts the master IMR that the real mode code stored.
fn assert_stored_imr(bus: &mut impl Bus, expected: u8, case: &str) {
    let imr = bus.read_byte(RESULT_IMR);
    assert_eq!(
        imr, expected,
        "{case}: master IMR {imr:#04X}, expected {expected:#04X}"
    );
}

/// Boots a VM with a SASI drive and runs `code` in real mode at TEST_CODE.
fn run_vm_hdd(code: &[u8]) -> machine_98::Pc9801Vm {
    let mut machine = create_machine_vm_hdd();
    boot_to_halt_hdd!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut state = cpu::V30State::default();
        state.ip = TEST_CODE as u16;
        state.initialize_cold_frontend();
        state.set_sp(0x4000);
        state
    });
    machine.run_for(DISK_INIT_BUDGET);
    machine
}

#[test]
fn int1bh_floppy_initialize_leaves_imr_unchanged_vm() {
    let code = make_real_mode_code(&int1bh_call(FLOPPY_INITIALIZE));
    let (mut machine, _) = boot_and_run_vm(&code, &[], DISK_INIT_BUDGET);
    assert_stored_imr(&mut machine.bus, IMR_IR0_MASKED, "floppy AH=03h");
}

#[test]
fn int1bh_sasi_initialize_leaves_imr_unchanged_vm() {
    let code = make_real_mode_code(&int1bh_call(HARD_DISK_INITIALIZE));
    let mut machine = run_vm_hdd(&code);
    assert_stored_imr(&mut machine.bus, IMR_IR0_MASKED, "SASI AH=03h");
}

#[test]
fn int1bh_ide_initialize_leaves_imr_unchanged_pc9821as() {
    let code = make_real_mode_code(&int1bh_call(HARD_DISK_INITIALIZE));
    let mut machine = create_machine_pc9821as_hdd();
    boot_to_halt_hdd!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut state = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        state.set_esp(0x4000);
        state
    });
    machine.run_for(DISK_INIT_BUDGET);
    assert_stored_imr(&mut machine.bus, IMR_IR0_MASKED, "IDE AH=03h");
}

#[test]
fn sasi_rom_initialize_leaves_imr_unchanged_vm() {
    let code = make_real_mode_code(&sasi_rom_entry_call(HARD_DISK_INITIALIZE));
    let mut machine = run_vm_hdd(&code);
    assert_stored_imr(&mut machine.bus, IMR_IR0_MASKED, "SASI ROM AH=03h");
}
