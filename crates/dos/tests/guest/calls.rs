//! Guest call mechanism: chained calls, nested entries, far call entries and
//! the save state of a pending call.

use common::{Cpu as _, HleDosVersion};
use dos::{
    DosPlatform,
    tables::{IOSYS_BASE, IOSYS_EXT_DAUA_TABLE_SIZE, IOSYS_OFF_DAUA_PTR, IOSYS_OFF_EXT_DAUA_TABLE},
};

use crate::harness::{
    BUSY_POLLS, DRIVER_SEGMENT, GuestMachine, IDLE_CHAR, IDLE_PRINT, INT28_ENTRY,
    platform_with_guest_console,
};

/// Segment of the `INT 28h` handler that fills the stack of the guest calls.
const STACK_FILL_SEGMENT: u16 = 0x7000;
/// Offset of the word where the handler saves SP.
const STACK_FILL_SAVED_SP: u16 = 0x0040;
/// Offset of the byte that counts the runs of the handler.
const STACK_FILL_COUNT: u16 = 0x0042;
/// Word that the handler pushes.
const STACK_FILL_WORD: u16 = 0xA5A5;
/// Bytes at the start of the guest call area that the fill leaves alone:
/// the routine pointers, the request header, the data buffer and the stub.
const GUEST_CALL_AREA_RESERVE: u16 = 0x0100;
/// Size of the far pointer at `0060:2820` in bytes.
const FAR_POINTER_SIZE: usize = 4;
/// First byte of the pattern that the test writes into the far pointer at
/// `0060:2820`.
const DAUA_POINTER_PATTERN_START: u8 = 0x30;
/// First byte of the pattern that the test writes into the drive table at
/// `0060:2C86`.
const DAUA_TABLE_PATTERN_START: u8 = 0x40;

/// Returns a booted machine with the guest console attached.
fn machine() -> GuestMachine {
    let mut machine = GuestMachine::new(platform_with_guest_console(HleDosVersion::Dos620));
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

/// Returns `size` ascending bytes from `start`.
fn byte_pattern(start: u8, size: usize) -> Vec<u8> {
    (0..size).map(|index| start + index as u8).collect()
}

/// Returns the far pointer at `0060:2820` and the drive table at `0060:2C86`.
fn iosys_drive_tables(machine: &GuestMachine) -> (Vec<u8>, Vec<u8>) {
    let read = |offset: u32, size: usize| {
        let start = (IOSYS_BASE + offset) as usize;
        machine.bus.ram[start..start + size].to_vec()
    };
    (
        read(IOSYS_OFF_DAUA_PTR, FAR_POINTER_SIZE),
        read(IOSYS_OFF_EXT_DAUA_TABLE, IOSYS_EXT_DAUA_TABLE_SIZE as usize),
    )
}

#[test]
fn deep_guest_call_keeps_the_iosys_drive_tables() {
    for version in [HleDosVersion::Dos310, HleDosVersion::Dos620] {
        let platform = DosPlatform::pc98(version);
        let placement = platform.placement;
        let stack_limit = platform
            .data_area
            .layout()
            .swappable_data_area
            .private_tail_offset
            + GUEST_CALL_AREA_RESERVE;
        let mut machine = GuestMachine::new(platform_with_guest_console(version));
        machine.attach_console();
        let drive_tables = (
            byte_pattern(DAUA_POINTER_PATTERN_START, FAR_POINTER_SIZE),
            byte_pattern(DAUA_TABLE_PATTERN_START, IOSYS_EXT_DAUA_TABLE_SIZE as usize),
        );
        machine.write_bytes(IOSYS_BASE + IOSYS_OFF_DAUA_PTR, &drive_tables.0);
        machine.write_bytes(IOSYS_BASE + IOSYS_OFF_EXT_DAUA_TABLE, &drive_tables.1);

        let [saved_low, saved_high] = STACK_FILL_SAVED_SP.to_le_bytes();
        let [count_low, count_high] = STACK_FILL_COUNT.to_le_bytes();
        let [fill_low, fill_high] = STACK_FILL_WORD.to_le_bytes();
        let [limit_low, limit_high] = stack_limit.to_le_bytes();
        #[rustfmt::skip]
        let handler = [
            0x50,                                   // PUSH AX
            0x2E, 0x89, 0x26, saved_low, saved_high, // MOV [CS:saved], SP
            0x2E, 0xFE, 0x06, count_low, count_high, // INC BYTE [CS:count]
            0xB8, fill_low, fill_high,              // MOV AX, fill
            0x50,                                   // fill: PUSH AX
            0x81, 0xFC, limit_low, limit_high,      // CMP SP, limit
            0x77, 0xF9,                             // JA fill
            0x2E, 0x8B, 0x26, saved_low, saved_high, // MOV SP, [CS:saved]
            0x58,                                   // POP AX
            0xCF,                                   // IRET
        ];
        let handler_base = u32::from(STACK_FILL_SEGMENT) << 4;
        machine.write_bytes(handler_base, &handler);
        machine.set_vector(0x28, STACK_FILL_SEGMENT, 0x0000);
        machine.set_driver_word(BUSY_POLLS, 2);
        machine.type_input(b"d");
        // MOV AH,08h; INT 21h; HLT
        machine.run_program(&[0xB4, 0x08, 0xCD, 0x21, 0xF4]);

        assert_eq!(machine.cpu.al(), b'd', "{version:?}");
        assert_ne!(
            machine.read_byte(handler_base + u32::from(STACK_FILL_COUNT)),
            0,
            "{version:?}"
        );
        assert_eq!(
            machine.read_word(placement.dos_data_address(stack_limit)),
            STACK_FILL_WORD,
            "{version:?}"
        );
        assert_eq!(iosys_drive_tables(&machine), drive_tables, "{version:?}");
    }
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
