//! Guest console backend: CON output, input and status through the synthetic CON driver.

use common::Cpu as _;

use crate::harness::{
    BUSY_POLLS, CON_HEADER, DRIVER_SEGMENT, FLUSH_COUNT, GuestMachine, IDLE_COUNT, INT28_ENTRY,
    PROGRAM_SEGMENT, platform_with_guest_console,
};

/// Offset of the data of a test program.
const DATA_OFFSET: u32 = 0x0200;

/// Returns a booted machine with the guest console attached.
fn machine() -> GuestMachine {
    let mut machine = GuestMachine::new(platform_with_guest_console());
    machine.attach_console();
    machine
}

/// Writes program data at `PROGRAM_SEGMENT:0200h`.
fn write_data(machine: &mut GuestMachine, data: &[u8]) {
    machine.write_bytes((u32::from(PROGRAM_SEGMENT) << 4) + DATA_OFFSET, data);
}

#[test]
fn display_string_goes_through_int29() {
    let mut machine = machine();
    write_data(&mut machine, b"OK$");
    // MOV AH,09h; MOV DX,0200h; INT 21h; HLT
    machine.run_program(&[0xB4, 0x09, 0xBA, 0x00, 0x02, 0xCD, 0x21, 0xF4]);
    assert_eq!(machine.console_output(), b"OK");
    assert_eq!(machine.cpu.al(), b'$');
    assert!(machine.command_log().is_empty());
}

#[test]
fn handle_write_without_fast_console_uses_write_requests() {
    let mut machine = GuestMachine::new(platform_with_guest_console());
    machine.write_word(GuestMachine::driver_address(CON_HEADER + 4), 0x8003);
    machine.attach_console();
    write_data(&mut machine, b"abc");
    // MOV AH,40h; MOV BX,1; MOV CX,3; MOV DX,0200h; INT 21h; HLT
    machine.run_program(&[
        0xB4, 0x40, 0xBB, 0x01, 0x00, 0xB9, 0x03, 0x00, 0xBA, 0x00, 0x02, 0xCD, 0x21, 0xF4,
    ]);
    assert_eq!(machine.console_output(), b"abc");
    assert_eq!(machine.command_log(), [0x08]);
    assert_eq!(machine.cpu.ax(), 3);
}

#[test]
fn keyboard_input_with_echo_reads_through_status_and_read() {
    let mut machine = machine();
    machine.type_input(b"x");
    // MOV AH,01h; INT 21h; HLT
    machine.run_program(&[0xB4, 0x01, 0xCD, 0x21, 0xF4]);
    assert_eq!(machine.cpu.al(), b'x');
    assert_eq!(machine.console_output(), b"x");
    assert_eq!(machine.command_log(), [0x05, 0x04]);
}

#[test]
fn keyboard_status_reports_the_driver_state() {
    let mut machine = machine();
    // MOV AH,0Bh; INT 21h; HLT
    let program = [0xB4, 0x0B, 0xCD, 0x21, 0xF4];
    machine.run_program(&program);
    assert_eq!(machine.cpu.al(), 0x00);

    machine.type_input(b"y");
    machine.run_program(&program);
    assert_eq!(machine.cpu.al(), 0xFF);
}

#[test]
fn direct_console_input_without_a_key_sets_zero_flag() {
    let mut machine = machine();
    // MOV AH,06h; MOV DL,FFh; INT 21h; HLT
    let program = [0xB4, 0x06, 0xB2, 0xFF, 0xCD, 0x21, 0xF4];
    machine.run_program(&program);
    assert_ne!(machine.cpu.flags() & 0x0040, 0);

    machine.type_input(b"z");
    machine.run_program(&program);
    assert_eq!(machine.cpu.flags() & 0x0040, 0);
    assert_eq!(machine.cpu.al(), b'z');
}

#[test]
fn blocking_input_idles_until_the_driver_has_a_key() {
    let mut machine = machine();
    machine.type_input(b"k");
    machine.set_driver_word(BUSY_POLLS, 3);
    machine.set_vector(0x28, DRIVER_SEGMENT, INT28_ENTRY);
    // MOV AH,08h; INT 21h; HLT
    machine.run_program(&[0xB4, 0x08, 0xCD, 0x21, 0xF4]);
    assert_eq!(machine.cpu.al(), b'k');
    let commands = machine.command_log();
    assert_eq!(commands, [0x05, 0x05, 0x05, 0x05, 0x04]);
    assert_eq!(
        machine.read_byte(GuestMachine::driver_address(IDLE_COUNT)),
        3
    );
    assert_eq!(machine.indos(), 0);
}

#[test]
fn buffered_input_echoes_and_ends_at_carriage_return() {
    let mut machine = machine();
    machine.type_input(b"hi\r");
    write_data(&mut machine, &[8]);
    // MOV AH,0Ah; MOV DX,0200h; INT 21h; HLT
    machine.run_program(&[0xB4, 0x0A, 0xBA, 0x00, 0x02, 0xCD, 0x21, 0xF4]);
    let base = (u32::from(PROGRAM_SEGMENT) << 4) + DATA_OFFSET;
    assert_eq!(machine.read_byte(base + 1), 2);
    assert_eq!(machine.read_byte(base + 2), b'h');
    assert_eq!(machine.read_byte(base + 3), b'i');
    assert_eq!(machine.console_output(), b"hi\r\n");
}

#[test]
fn flush_and_read_sends_the_input_flush() {
    let mut machine = machine();
    machine.type_input(b"old");
    // MOV AX,0C01h; INT 21h; HLT, run with input that arrives after the flush
    machine.start_program(&[0xB8, 0x01, 0x0C, 0xCD, 0x21, 0xF4]);
    assert!(!machine.run_steps(20_000));
    assert_eq!(machine.driver_word(FLUSH_COUNT), 1);
    machine.type_input(b"n");
    assert!(machine.run_steps(20_000_000));
    assert_eq!(machine.cpu.al(), b'n');
}
