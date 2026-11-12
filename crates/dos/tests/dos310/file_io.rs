use crate::harness::*;

/// Path of the file that the round trip creates.
const FILE_PATH: &[u8] = b"A:\\NEW310.TXT\0";
/// Data that the round trip writes.
const FILE_DATA: &[u8; 10] = b"DOS 3.10!!";
/// File position of the second read.
const SEEK_POSITION: u16 = 6;
/// Offset of the data to write in the inject segment.
const WRITE_BUFFER_OFFSET: u16 = 0x0240;
/// Offset of the buffer of the first read in the inject segment.
const READ_BUFFER_OFFSET: u16 = 0x0280;
/// Offset of the buffer of the second read in the inject segment.
const SEEK_READ_BUFFER_OFFSET: u16 = 0x02A0;
/// Carry flag in the flags word.
const CARRY_FLAG: u16 = 0x0001;
/// DOS error code of a missing file.
const ERROR_FILE_NOT_FOUND: u16 = 0x0002;

/// Results of the file round trip.
struct RoundTrip {
    create_flags: u16,
    bytes_written: u16,
    write_flags: u16,
    rewind_position: u16,
    first_read_count: u16,
    first_read: Vec<u8>,
    seek_position: u16,
    second_read_count: u16,
    second_read: Vec<u8>,
    close_flags: u16,
    delete_flags: u16,
    reopen_result: u16,
    reopen_flags: u16,
}

/// Creates, writes, rewinds, reads, seeks, reads, closes and deletes
/// `FILE_PATH`, then tries to open it again.
fn run_round_trip(machine: &mut Machine) -> RoundTrip {
    let data_length = FILE_DATA.len() as u8;
    let seek_read_length = data_length - SEEK_POSITION as u8;
    write_bytes(&mut machine.bus, INJECT_DATA_BASE, FILE_PATH);
    write_bytes(
        &mut machine.bus,
        INJECT_CODE_BASE + u32::from(WRITE_BUFFER_OFFSET),
        FILE_DATA,
    );
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        // Create
        0xBA, 0x00, 0x02,             // MOV DX, 0200h
        0xB9, 0x00, 0x00,             // MOV CX, 0000h
        0xB4, 0x3C,                   // MOV AH, 3Ch
        0xCD, 0x21,                   // INT 21h
        0xA3, 0x00, 0x01,             // MOV [0100h], AX
        0x9C, 0x58,                   // PUSHF; POP AX
        0xA3, 0x02, 0x01,             // MOV [0102h], AX
        // Write
        0x8B, 0x1E, 0x00, 0x01,       // MOV BX, [0100h]
        0xB9, data_length, 0x00,      // MOV CX, length
        0xBA, 0x40, 0x02,             // MOV DX, 0240h
        0xB4, 0x40,                   // MOV AH, 40h
        0xCD, 0x21,                   // INT 21h
        0xA3, 0x04, 0x01,             // MOV [0104h], AX
        0x9C, 0x58,                   // PUSHF; POP AX
        0xA3, 0x06, 0x01,             // MOV [0106h], AX
        // Rewind
        0x8B, 0x1E, 0x00, 0x01,       // MOV BX, [0100h]
        0xB8, 0x00, 0x42,             // MOV AX, 4200h
        0x31, 0xC9,                   // XOR CX, CX
        0x31, 0xD2,                   // XOR DX, DX
        0xCD, 0x21,                   // INT 21h
        0xA3, 0x08, 0x01,             // MOV [0108h], AX
        // Read everything
        0x8B, 0x1E, 0x00, 0x01,       // MOV BX, [0100h]
        0xB9, data_length, 0x00,      // MOV CX, length
        0xBA, 0x80, 0x02,             // MOV DX, 0280h
        0xB4, 0x3F,                   // MOV AH, 3Fh
        0xCD, 0x21,                   // INT 21h
        0xA3, 0x0A, 0x01,             // MOV [010Ah], AX
        // Seek
        0x8B, 0x1E, 0x00, 0x01,       // MOV BX, [0100h]
        0xB8, 0x00, 0x42,             // MOV AX, 4200h
        0x31, 0xC9,                   // XOR CX, CX
        0xBA, SEEK_POSITION as u8, 0x00, // MOV DX, position
        0xCD, 0x21,                   // INT 21h
        0xA3, 0x0C, 0x01,             // MOV [010Ch], AX
        // Read the tail
        0x8B, 0x1E, 0x00, 0x01,       // MOV BX, [0100h]
        0xB9, seek_read_length, 0x00, // MOV CX, length
        0xBA, 0xA0, 0x02,             // MOV DX, 02A0h
        0xB4, 0x3F,                   // MOV AH, 3Fh
        0xCD, 0x21,                   // INT 21h
        0xA3, 0x0E, 0x01,             // MOV [010Eh], AX
        // Close
        0x8B, 0x1E, 0x00, 0x01,       // MOV BX, [0100h]
        0xB4, 0x3E,                   // MOV AH, 3Eh
        0xCD, 0x21,                   // INT 21h
        0x9C, 0x58,                   // PUSHF; POP AX
        0xA3, 0x10, 0x01,             // MOV [0110h], AX
        // Delete
        0xBA, 0x00, 0x02,             // MOV DX, 0200h
        0xB4, 0x41,                   // MOV AH, 41h
        0xCD, 0x21,                   // INT 21h
        0x9C, 0x58,                   // PUSHF; POP AX
        0xA3, 0x12, 0x01,             // MOV [0112h], AX
        // Open the deleted file
        0xBA, 0x00, 0x02,             // MOV DX, 0200h
        0xB8, 0x00, 0x3D,             // MOV AX, 3D00h
        0xCD, 0x21,                   // INT 21h
        0xA3, 0x14, 0x01,             // MOV [0114h], AX
        0x9C, 0x58,                   // PUSHF; POP AX
        0xA3, 0x16, 0x01,             // MOV [0116h], AX
        0xFA,                         // CLI
        0xF4,                         // HLT
    ];
    inject_and_run_with_budget(machine, &code, INJECT_BUDGET_DISK_IO);
    let bus = &machine.bus;
    RoundTrip {
        create_flags: result_word(bus, 0x02),
        bytes_written: result_word(bus, 0x04),
        write_flags: result_word(bus, 0x06),
        rewind_position: result_word(bus, 0x08),
        first_read_count: result_word(bus, 0x0A),
        first_read: read_bytes(
            bus,
            INJECT_CODE_BASE + u32::from(READ_BUFFER_OFFSET),
            FILE_DATA.len(),
        ),
        seek_position: result_word(bus, 0x0C),
        second_read_count: result_word(bus, 0x0E),
        second_read: read_bytes(
            bus,
            INJECT_CODE_BASE + u32::from(SEEK_READ_BUFFER_OFFSET),
            usize::from(seek_read_length),
        ),
        close_flags: result_word(bus, 0x10),
        delete_flags: result_word(bus, 0x12),
        reopen_result: result_word(bus, 0x14),
        reopen_flags: result_word(bus, 0x16),
    }
}

/// Checks every step of the round trip.
fn assert_round_trip(result: &RoundTrip) {
    assert_eq!(result.create_flags & CARRY_FLAG, 0, "create failed");
    assert_eq!(result.write_flags & CARRY_FLAG, 0, "write failed");
    assert_eq!(result.bytes_written, FILE_DATA.len() as u16);
    assert_eq!(result.rewind_position, 0);
    assert_eq!(result.first_read_count, FILE_DATA.len() as u16);
    assert_eq!(result.first_read, FILE_DATA);
    assert_eq!(result.seek_position, SEEK_POSITION);
    assert_eq!(
        result.second_read_count,
        FILE_DATA.len() as u16 - SEEK_POSITION
    );
    assert_eq!(result.second_read, &FILE_DATA[usize::from(SEEK_POSITION)..]);
    assert_eq!(result.close_flags & CARRY_FLAG, 0, "close failed");
    assert_eq!(result.delete_flags & CARRY_FLAG, 0, "delete failed");
    assert_ne!(result.reopen_flags & CARRY_FLAG, 0, "the file still exists");
    assert_eq!(result.reopen_result, ERROR_FILE_NOT_FOUND);
}

#[test]
fn file_round_trip_works_on_a_floppy() {
    let mut machine = boot_with_floppy(create_floppy(&standard_files()));
    let result = run_round_trip(&mut machine);
    assert_round_trip(&result);
}

#[test]
fn file_round_trip_works_on_a_hard_disk() {
    let mut machine = boot_with_hdd(create_hdd(&standard_files()));
    let result = run_round_trip(&mut machine);
    assert_round_trip(&result);
}
