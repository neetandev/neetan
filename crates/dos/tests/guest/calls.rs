//! Guest call mechanism: chained calls, nested entries, far call entries and
//! the save state of a pending call.

use common::Cpu as _;

use crate::harness::{
    BUSY_POLLS, DRIVER_SEGMENT, GuestMachine, IDLE_CHAR, IDLE_PRINT, INT28_ENTRY,
    platform_with_guest_console,
};

/// Returns a booted machine with the guest console attached.
fn machine() -> GuestMachine {
    let mut machine = GuestMachine::new(platform_with_guest_console());
    machine.attach_console();
    machine
}

#[test]
fn one_entry_chains_several_guest_calls() {
    let mut machine = machine();
    machine.write_bytes((0x5000u32 << 4) + 0x0200, b"chain$");
    let traps_before = machine.trap_count;
    // MOV AH,09h; MOV DX,0200h; INT 21h; HLT
    machine.run_program(&[0xB4, 0x09, 0xBA, 0x00, 0x02, 0xCD, 0x21, 0xF4]);
    assert_eq!(machine.console_output(), b"chain");
    // The INT 21h trap and one return trap per byte.
    assert_eq!(machine.trap_count - traps_before, 1 + 5);
    assert_eq!(machine.cpu.sp(), 0xFFFE);
    assert_eq!(machine.cpu.dx(), 0x0200);
}

#[test]
fn int21h_inside_a_guest_call_runs_a_nested_operation() {
    let mut machine = machine();
    machine.set_vector(0x28, DRIVER_SEGMENT, INT28_ENTRY);
    machine.set_driver_byte(IDLE_PRINT, 1);
    machine.set_driver_byte(IDLE_CHAR, b'n');
    machine.set_driver_word(BUSY_POLLS, 2);
    machine.type_input(b"q");
    // MOV AH,08h; INT 21h; HLT
    machine.run_program(&[0xB4, 0x08, 0xCD, 0x21, 0xF4]);
    assert_eq!(machine.cpu.al(), b'q');
    assert_eq!(machine.console_output(), b"nn");
    assert_eq!(machine.indos(), 0);
}

#[test]
fn far_call_entry_keeps_its_return_address_while_waiting() {
    let mut machine = machine();
    machine.set_driver_word(BUSY_POLLS, 2);
    machine.type_input(b"f");
    // XOR AX,AX; MOV DS,AX; MOV AH,08h; PUSHF; CALL FAR [0084h]; HLT
    machine.run_program(&[
        0x31, 0xC0, 0x8E, 0xD8, 0xB4, 0x08, 0x9C, 0xFF, 0x1E, 0x84, 0x00, 0xF4,
    ]);
    assert_eq!(machine.cpu.al(), b'f');
    assert_eq!(machine.cpu.sp(), 0xFFFE);
}

#[test]
fn pending_guest_call_survives_a_save_state_round_trip() {
    let mut machine = machine();
    machine.set_driver_word(BUSY_POLLS, 0xFFFF);
    // MOV AH,01h; INT 21h; HLT
    machine.start_program(&[0xB4, 0x01, 0xCD, 0x21, 0xF4]);
    assert!(!machine.run_steps(5_000));

    let mut encoded = Vec::new();
    save_state::StateEncode::encode_state(&machine.dos, &mut encoded);
    let mut decoder = save_state::StateDecoder::new(&encoded, usize::MAX);
    let restored: dos::NeetanDos = save_state::StateDecode::decode_state(&mut decoder).unwrap();
    machine.dos = restored;

    machine.set_driver_word(BUSY_POLLS, 0);
    machine.type_input(b"s");
    assert!(machine.run_steps(20_000_000));
    assert_eq!(machine.cpu.al(), b's');
    assert_eq!(machine.console_output(), b"s");
    assert_eq!(machine.indos(), 0);
}
