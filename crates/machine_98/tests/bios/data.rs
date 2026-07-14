use common::Bus;

use super::{
    create_machine_f, create_machine_ra, create_machine_vm, create_machine_vx, run_f, run_vm,
    write_bytes,
};

const NEC_COPYRIGHT: &[u8] = b"Copyright (C) 1983 by NEC Corporation";
const RESULT: u32 = 0x0500;
const SCRATCH: u32 = 0x0600;
const BDA_BIOS_FLAG2: u32 = 0x0400;
const BDA_EXPMMSZ: u32 = 0x0401;
const BDA_ITF_WORK: u32 = 0x0403;
const BDA_USER_SP: u32 = 0x0404;
const BDA_USER_SS: u32 = 0x0406;
const BDA_SYS_TYPE: u32 = 0x0480;
const BDA_BIOS_FLAG3: u32 = 0x0481;
const BDA_DISK_EQUIPS: u32 = 0x0482;
const BDA_BIOS_FLAG0: u32 = 0x0500;
const BDA_BIOS_FLAG1: u32 = 0x0501;

// ============================================================================
// §3 ROM Data Verification
// ============================================================================

/// Software reads E800:0DD8 (physical 0xE8DD8) to identify NEC hardware.
#[test]
fn nec_copyright_string() {
    let mut machine = create_machine_vm();
    write_bytes(&mut machine.bus, SCRATCH, NEC_COPYRIGHT);

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0xE8,          // MOV AX, 0xE800
        0x8E, 0xC0,                // MOV ES, AX
        0xBF, 0xD8, 0x0D,          // MOV DI, 0x0DD8
        0xBE, 0x00, 0x06,          // MOV SI, SCRATCH
        0xB9, 0x25, 0x00,          // MOV CX, 37
        0xFC,                      // CLD
        0xF3, 0xA6,                // REPE CMPSB
        0x89, 0x0E, 0x00, 0x05,    // MOV [RESULT], CX
        0xF4,                      // HLT
    ];

    let _cycles = run_vm(&mut machine, code, 5000);

    let remaining = machine.bus.read_word(RESULT);
    assert_eq!(
        remaining,
        0,
        "Copyright mismatch at byte {}",
        NEC_COPYRIGHT.len() - remaining as usize
    );
}

#[test]
fn nec_copyright_string_f() {
    let mut machine = create_machine_f();
    write_bytes(&mut machine.bus, SCRATCH, NEC_COPYRIGHT);

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0xE8,          // MOV AX, 0xE800
        0x8E, 0xC0,                // MOV ES, AX
        0xBF, 0xE2, 0x0D,          // MOV DI, 0x0DE2
        0xBE, 0x00, 0x06,          // MOV SI, SCRATCH
        0xB9, 0x25, 0x00,          // MOV CX, 37
        0xFC,                      // CLD
        0xF3, 0xA6,                // REPE CMPSB
        0x89, 0x0E, 0x00, 0x05,    // MOV [RESULT], CX
        0xF4,                      // HLT
    ];

    let _cycles = run_f(&mut machine, code, 5000);

    let remaining = machine.bus.read_word(RESULT);
    assert_eq!(
        remaining,
        0,
        "Copyright mismatch at byte {}",
        NEC_COPYRIGHT.len() - remaining as usize
    );
}

// ============================================================================
// §4.1 Cold Reset Entry - Reset Vector
// ============================================================================

/// At cold reset the CPU reads FFFF:0000 (physical 0xFFFF0). It must be a FAR JMP.
#[test]
fn reset_vector_is_far_jmp() {
    let mut machine = create_machine_vm();

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0xFF, 0xFF,          // MOV AX, 0xFFFF
        0x8E, 0xD8,                // MOV DS, AX
        0xA0, 0x00, 0x00,          // MOV AL, [0x0000]
        0x31, 0xDB,                // XOR BX, BX
        0x8E, 0xDB,                // MOV DS, BX
        0xA2, 0x00, 0x05,          // MOV [RESULT], AL
        0xF4,                      // HLT
    ];

    let _cycles = run_vm(&mut machine, code, 5000);

    let opcode = machine.bus.read_byte(RESULT);
    assert_eq!(
        opcode, 0xEA,
        "Reset vector must be a FAR JMP (0xEA), got {opcode:#04X}"
    );
}

#[test]
fn reset_vector_is_far_jmp_f() {
    let mut machine = create_machine_f();

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0xFF, 0xFF,          // MOV AX, 0xFFFF
        0x8E, 0xD8,                // MOV DS, AX
        0xA0, 0x00, 0x00,          // MOV AL, [0x0000]
        0x31, 0xDB,                // XOR BX, BX
        0x8E, 0xDB,                // MOV DS, BX
        0xA2, 0x00, 0x05,          // MOV [RESULT], AL
        0xF4,                      // HLT
    ];

    let _cycles = run_f(&mut machine, code, 5000);

    let opcode = machine.bus.read_byte(RESULT);
    assert_eq!(
        opcode, 0xEA,
        "Reset vector must be a FAR JMP (0xEA), got {opcode:#04X}"
    );
}

/// After switching to the BIOS bank, the reset vector at FFFF:0000 must
/// target segment 0xFD80 (physical 0xFD800, the BIOS code segment).
/// For single-bank ROMs the bank switch is a no-op and the reset vector
/// already targets FD80.
#[test]
fn bios_reset_vector_targets_bios_segment() {
    let mut machine = create_machine_vm();

    #[rustfmt::skip]
    let code: &[u8] = &[
        // Switch to BIOS bank (port 0x043D = 0x12).
        0xBA, 0x3D, 0x04,          // MOV DX, 0x043D
        0xB0, 0x12,                // MOV AL, 0x12
        0xEE,                      // OUT DX, AL
        // Read reset vector at FFFF:0000.
        0xB8, 0xFF, 0xFF,          // MOV AX, 0xFFFF
        0x8E, 0xD8,                // MOV DS, AX
        0xA0, 0x00, 0x00,          // MOV AL, [0x0000]  ; opcode
        0x8B, 0x1E, 0x03, 0x00,    // MOV BX, [0x0003]  ; target segment
        // Store results to RAM.
        0x31, 0xD2,                // XOR DX, DX
        0x8E, 0xDA,                // MOV DS, DX
        0xA2, 0x00, 0x05,          // MOV [RESULT], AL
        0x89, 0x1E, 0x02, 0x05,    // MOV [RESULT+2], BX
        0xF4,                      // HLT
    ];

    let _cycles = run_vm(&mut machine, code, 5000);

    let opcode = machine.bus.read_byte(RESULT);
    let segment = machine.bus.read_word(RESULT + 2);
    assert_eq!(opcode, 0xEA, "BIOS reset vector must be FAR JMP");
    assert_eq!(
        segment, 0xFD80,
        "BIOS reset vector must target segment 0xFD80"
    );
}

#[test]
fn bios_reset_vector_targets_bios_segment_f() {
    let mut machine = create_machine_f();

    #[rustfmt::skip]
    let code: &[u8] = &[
        // Switch to BIOS bank (port 0x043D = 0x12).
        0xBA, 0x3D, 0x04,          // MOV DX, 0x043D
        0xB0, 0x12,                // MOV AL, 0x12
        0xEE,                      // OUT DX, AL
        // Read reset vector at FFFF:0000.
        0xB8, 0xFF, 0xFF,          // MOV AX, 0xFFFF
        0x8E, 0xD8,                // MOV DS, AX
        0xA0, 0x00, 0x00,          // MOV AL, [0x0000]  ; opcode
        0x8B, 0x1E, 0x03, 0x00,    // MOV BX, [0x0003]  ; target segment
        // Store results to RAM.
        0x31, 0xD2,                // XOR DX, DX
        0x8E, 0xDA,                // MOV DS, DX
        0xA2, 0x00, 0x05,          // MOV [RESULT], AL
        0x89, 0x1E, 0x02, 0x05,    // MOV [RESULT+2], BX
        0xF4,                      // HLT
    ];

    let _cycles = run_f(&mut machine, code, 5000);

    let opcode = machine.bus.read_byte(RESULT);
    let segment = machine.bus.read_word(RESULT + 2);
    assert_eq!(opcode, 0xEA, "BIOS reset vector must be FAR JMP");
    assert_eq!(
        segment, 0xFD80,
        "BIOS reset vector must target segment 0xFD80"
    );
}

// ============================================================================
// §4.5 BDA Setup - System configuration
// ============================================================================

/// PC-9801VM (V30 @ 8 / 10 MHz, single-bank BIOS ROM).
#[test]
fn system_config_vm() {
    let mut machine = create_machine_vm();
    let _cycles = boot_to_halt!(machine);

    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG2), 0x00, "BIOS_FLAG2");
    assert_eq!(machine.bus.read_byte(BDA_EXPMMSZ), 0x00, "EXPMMSZ");
    assert_eq!(machine.bus.read_byte(BDA_ITF_WORK), 0x00, "ITF_WORK");
    assert_eq!(machine.bus.read_word(BDA_USER_SP), 0x0000, "USER_SP");
    assert_eq!(machine.bus.read_word(BDA_USER_SS), 0x0000, "USER_SS");
    assert_eq!(
        machine.bus.read_byte(BDA_SYS_TYPE),
        0x00,
        "SYS_TYPE (V30=0x00)"
    );
    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG3), 0x00, "BIOS_FLAG3");
    assert_eq!(machine.bus.read_byte(BDA_DISK_EQUIPS), 0x00, "DISK_EQUIPS");
    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG0), 0x03, "BIOS_FLAG0");
    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG1), 0x64, "BIOS_FLAG1");
}

/// PC-9801VX (80286 @ 8 / 10 MHz, dual-bank BIOS ROM).
#[test]
fn system_config_vx() {
    let mut machine = create_machine_vx();
    let _cycles = boot_to_halt!(machine);

    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG2), 0x00, "BIOS_FLAG2");
    assert_eq!(machine.bus.read_byte(BDA_EXPMMSZ), 0x20, "EXPMMSZ");
    assert_eq!(machine.bus.read_byte(BDA_ITF_WORK), 0x00, "ITF_WORK");
    assert_eq!(machine.bus.read_word(BDA_USER_SP), 0x0000, "USER_SP");
    assert_eq!(machine.bus.read_word(BDA_USER_SS), 0x0000, "USER_SS");
    assert_eq!(
        machine.bus.read_byte(BDA_SYS_TYPE),
        0x01,
        "SYS_TYPE (80286=0x01)"
    );
    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG3), 0x00, "BIOS_FLAG3");
    assert_eq!(machine.bus.read_byte(BDA_DISK_EQUIPS), 0x00, "DISK_EQUIPS");
    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG0), 0x03, "BIOS_FLAG0");
    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG1), 0x24, "BIOS_FLAG1");
}

/// PC-9801F (8086 @ 8 MHz, dual-bank real BIOS ROM).
#[test]
fn system_config_f() {
    let mut machine = create_machine_f();
    let _cycles = boot_to_halt!(machine);

    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG2), 0x00, "BIOS_FLAG2");
    assert_eq!(machine.bus.read_byte(BDA_EXPMMSZ), 0x00, "EXPMMSZ");
    assert_eq!(machine.bus.read_byte(BDA_ITF_WORK), 0x00, "ITF_WORK");
    assert_eq!(machine.bus.read_word(BDA_USER_SP), 0x0000, "USER_SP");
    assert_eq!(machine.bus.read_word(BDA_USER_SS), 0x0000, "USER_SS");
    assert_eq!(
        machine.bus.read_byte(BDA_SYS_TYPE),
        0x00,
        "SYS_TYPE (8086=0x00)"
    );
    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG3), 0x00, "BIOS_FLAG3");
    assert_eq!(machine.bus.read_byte(BDA_DISK_EQUIPS), 0x00, "DISK_EQUIPS");
    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG0), 0x20, "BIOS_FLAG0");
    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG1), 0xA4, "BIOS_FLAG1");
}

/// PC-9801RA (80386 @ 16 / 20 MHz, dual-bank BIOS ROM).
#[test]
fn system_config_ra() {
    let mut machine = create_machine_ra();
    let _cycles = boot_to_halt!(machine);

    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG2), 0x00, "BIOS_FLAG2");
    assert_eq!(machine.bus.read_byte(BDA_EXPMMSZ), 0x60, "EXPMMSZ");
    assert_eq!(machine.bus.read_byte(BDA_ITF_WORK), 0x00, "ITF_WORK");
    assert_eq!(machine.bus.read_word(BDA_USER_SP), 0x00F8, "USER_SP");
    assert_eq!(machine.bus.read_word(BDA_USER_SS), 0x0030, "USER_SS");
    assert_eq!(machine.bus.read_byte(BDA_SYS_TYPE), 0x4B, "SYS_TYPE");
    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG3), 0x20, "BIOS_FLAG3");
    assert_eq!(machine.bus.read_byte(BDA_DISK_EQUIPS), 0x00, "DISK_EQUIPS");
    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG0), 0x03, "BIOS_FLAG0");
    assert_eq!(machine.bus.read_byte(BDA_BIOS_FLAG1), 0xA4, "BIOS_FLAG1");
}

const BDA_KB_SHIFT_TBL: u32 = 0x0522;
const BDA_F2DD_MODE: u32 = 0x05CA;
const BDA_F2DD_POINTER: u32 = 0x05CC;
const BDA_F2HD_POINTER: u32 = 0x05F8;

#[test]
fn bda_keyboard_shift_table_pointer() {
    let mut machine = create_machine_vm();
    let _cycles = boot_to_halt!(machine);

    let offset = machine.bus.read_word(BDA_KB_SHIFT_TBL);
    assert_eq!(offset, 0x0B28, "KB_SHIFT_TBL should point to 0x0B28");
}

#[test]
fn bda_keyboard_shift_table_pointer_f() {
    let mut machine = create_machine_f();
    let _cycles = boot_to_halt!(machine);

    let offset = machine.bus.read_word(BDA_KB_SHIFT_TBL);
    assert_eq!(offset, 0x0A58, "KB_SHIFT_TBL should point to 0x0A58");
}

#[test]
fn bda_f2dd_mode_initialized() {
    let mut machine = create_machine_vm();
    let _cycles = boot_to_halt!(machine);

    assert_eq!(
        machine.bus.read_byte(BDA_F2DD_MODE),
        0xFF,
        "F2DD_MODE should be 0xFF (all drives normal density)"
    );
}

#[test]
fn bda_f2dd_mode_initialized_f() {
    let mut machine = create_machine_f();
    let _cycles = boot_to_halt!(machine);

    assert_eq!(
        machine.bus.read_byte(BDA_F2DD_MODE),
        0x00,
        "F2DD_MODE should remain 0x00"
    );
}

#[test]
fn bda_f2dd_pointer() {
    let mut machine = create_machine_vm();
    let _cycles = boot_to_halt!(machine);

    let offset = machine.bus.read_word(BDA_F2DD_POINTER);
    let segment = machine.bus.read_word(BDA_F2DD_POINTER + 2);
    assert_eq!(offset, 0x1ADC, "F2DD_POINTER offset");
    assert_eq!(segment, 0xFD80, "F2DD_POINTER segment");
}

#[test]
fn bda_f2dd_pointer_f() {
    let mut machine = create_machine_f();
    let _cycles = boot_to_halt!(machine);

    let offset = machine.bus.read_word(BDA_F2DD_POINTER);
    let segment = machine.bus.read_word(BDA_F2DD_POINTER + 2);
    assert_eq!(offset, 0x0000, "F2DD_POINTER offset");
    assert_eq!(segment, 0x0000, "F2DD_POINTER segment");
}

#[test]
fn bda_f2hd_pointer() {
    let mut machine = create_machine_vm();
    let _cycles = boot_to_halt!(machine);

    let offset = machine.bus.read_word(BDA_F2HD_POINTER);
    let segment = machine.bus.read_word(BDA_F2HD_POINTER + 2);
    assert_eq!(offset, 0x1AB4, "F2HD_POINTER offset");
    assert_eq!(segment, 0xFD80, "F2HD_POINTER segment");
}

#[test]
fn bda_f2hd_pointer_f() {
    let mut machine = create_machine_f();
    let _cycles = boot_to_halt!(machine);

    let offset = machine.bus.read_word(BDA_F2HD_POINTER);
    let segment = machine.bus.read_word(BDA_F2HD_POINTER + 2);
    assert_eq!(offset, 0x0000, "F2HD_POINTER offset");
    assert_eq!(segment, 0x0000, "F2HD_POINTER segment");
}

/// Verify keyboard translation tables are installed at FD80:0B28 in ROM.
/// Reads the first few bytes of each table and spot-checks key entries.
#[test]
fn keyboard_tables_installed_in_rom() {
    let mut machine = create_machine_vm();
    let base: u32 = 0xFD800 + 0x0B28;

    // Table 0 (normal): ESC=0x1B, '1', '2', ...
    assert_eq!(machine.bus.read_byte(base), 0x1B, "Table 0 [0] = ESC");
    assert_eq!(machine.bus.read_byte(base + 1), b'1', "Table 0 [1] = '1'");
    assert_eq!(
        machine.bus.read_byte(base + 0x10),
        b'q',
        "Table 0 [0x10] = 'q'"
    );

    // Table 1 (shift): '!', '"', '#', ...
    let t1 = base + 0x60;
    assert_eq!(machine.bus.read_byte(t1), 0x1B, "Table 1 [0] = ESC");
    assert_eq!(machine.bus.read_byte(t1 + 1), b'!', "Table 1 [1] = '!'");
    assert_eq!(
        machine.bus.read_byte(t1 + 0x10),
        b'Q',
        "Table 1 [0x10] = 'Q'"
    );

    // Table 2 (CAPS): uppercase letters
    let t2 = base + 0xC0;
    assert_eq!(
        machine.bus.read_byte(t2 + 0x10),
        b'Q',
        "Table 2 [0x10] = 'Q'"
    );
    assert_eq!(
        machine.bus.read_byte(t2 + 0x1D),
        b'A',
        "Table 2 [0x1D] = 'A'"
    );

    // Table 3 (shift+CAPS): lowercase letters
    let t3 = base + 0x120;
    assert_eq!(
        machine.bus.read_byte(t3 + 0x10),
        b'q',
        "Table 3 [0x10] = 'q'"
    );
    assert_eq!(
        machine.bus.read_byte(t3 + 0x1D),
        b'a',
        "Table 3 [0x1D] = 'a'"
    );

    // Table 7 (ctrl): Ctrl-A = 0x01, Ctrl-C = 0x03, etc.
    let t7 = base + 0x60 * 7;
    assert_eq!(
        machine.bus.read_byte(t7 + 0x1D),
        0x01,
        "Table 7 [0x1D] = Ctrl-A"
    );
    assert_eq!(
        machine.bus.read_byte(t7 + 0x2B),
        0x03,
        "Table 7 [0x2B] = Ctrl-C"
    );
}

#[test]
fn keyboard_tables_installed_in_rom_f() {
    let mut machine = create_machine_f();
    let base: u32 = 0xFD800 + 0x0A58;

    // Table 0 (normal): ESC=0x1B, '1', '2', ...
    assert_eq!(machine.bus.read_byte(base), 0x1B, "Table 0 [0] = ESC");
    assert_eq!(machine.bus.read_byte(base + 1), b'1', "Table 0 [1] = '1'");
    assert_eq!(
        machine.bus.read_byte(base + 0x10),
        b'q',
        "Table 0 [0x10] = 'q'"
    );

    // Table 1 (shift): '!', '"', '#', ...
    let t1 = base + 0x60;
    assert_eq!(machine.bus.read_byte(t1), 0x1B, "Table 1 [0] = ESC");
    assert_eq!(machine.bus.read_byte(t1 + 1), b'!', "Table 1 [1] = '!'");
    assert_eq!(
        machine.bus.read_byte(t1 + 0x10),
        b'Q',
        "Table 1 [0x10] = 'Q'"
    );

    // Table 2 (CAPS): uppercase letters
    let t2 = base + 0xC0;
    assert_eq!(
        machine.bus.read_byte(t2 + 0x10),
        b'Q',
        "Table 2 [0x10] = 'Q'"
    );
    assert_eq!(
        machine.bus.read_byte(t2 + 0x1D),
        b'A',
        "Table 2 [0x1D] = 'A'"
    );

    // Table 3 (shift+CAPS): lowercase letters
    let t3 = base + 0x120;
    assert_eq!(
        machine.bus.read_byte(t3 + 0x10),
        b'q',
        "Table 3 [0x10] = 'q'"
    );
    assert_eq!(
        machine.bus.read_byte(t3 + 0x1D),
        b'a',
        "Table 3 [0x1D] = 'a'"
    );

    // Table 7 (ctrl): Ctrl-A = 0x01, Ctrl-C = 0x03, etc.
    let t7 = base + 0x60 * 7;
    assert_eq!(
        machine.bus.read_byte(t7 + 0x1D),
        0x01,
        "Table 7 [0x1D] = Ctrl-A"
    );
    assert_eq!(
        machine.bus.read_byte(t7 + 0x2B),
        0x03,
        "Table 7 [0x2B] = Ctrl-C"
    );
}

/// Verify full content of normal (table 0) and shift (table 1) keyboard tables.
#[test]
fn keyboard_table_0_full_content() {
    let mut machine = create_machine_vm();
    let base: u32 = 0xFD800 + 0x0B28;

    #[rustfmt::skip]
    let expected: [u8; 0x60] = [
        0x1b, b'1', b'2', b'3', b'4', b'5', b'6', b'7',
        b'8', b'9', b'0', b'-', b'^', b'\\', 0x08, 0x09,
        b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i',
        b'o', b'p', b'@', b'[', 0x0d, b'a', b's', b'd',
        b'f', b'g', b'h', b'j', b'k', b'l', b';', b':',
        b']', b'z', b'x', b'c', b'v', b'b', b'n', b'm',
        b',', b'.', b'/', 0xff, b' ', 0x35, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3e, 0x00,
        b'-', b'/', b'7', b'8', b'9', b'*', b'4', b'5',
        b'6', b'+', b'1', b'2', b'3', b'=', b'0', b',',
        b'.', 0x51, 0xff, 0xff, 0xff, 0xff, 0x62, 0x63,
        0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b,
    ];

    for (i, &exp) in expected.iter().enumerate() {
        let got = machine.bus.read_byte(base + i as u32);
        assert_eq!(
            got, exp,
            "Keyboard table 0 byte {i:#04X}: expected {exp:#04X}, got {got:#04X}"
        );
    }
}

#[test]
fn keyboard_table_0_full_content_f() {
    let mut machine = create_machine_f();
    let base: u32 = 0xFD800 + 0x0A58;

    #[rustfmt::skip]
    let expected: [u8; 0x60] = [
        0x1b, b'1', b'2', b'3', b'4', b'5', b'6', b'7',
        b'8', b'9', b'0', b'-', b'^', b'\\', 0x08, 0x09,
        b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i',
        b'o', b'p', b'@', b'[', 0x0d, b'a', b's', b'd',
        b'f', b'g', b'h', b'j', b'k', b'l', b';', b':',
        b']', b'z', b'x', b'c', b'v', b'b', b'n', b'm',
        b',', b'.', b'/', 0xff, b' ', 0x35, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3e, 0x00,
        b'-', b'/', b'7', b'8', b'9', b'*', b'4', b'5',
        b'6', b'+', b'1', b'2', b'3', b'=', b'0', b',',
        b'.', 0xff, 0xff, 0xff, 0xff, 0xff, 0x62, 0x63,
        0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b,
    ];

    for (i, &exp) in expected.iter().enumerate() {
        let got = machine.bus.read_byte(base + i as u32);
        assert_eq!(
            got, exp,
            "Keyboard table 0 byte {i:#04X}: expected {exp:#04X}, got {got:#04X}"
        );
    }
}

#[test]
fn keyboard_table_1_full_content() {
    let mut machine = create_machine_vm();
    let base: u32 = 0xFD800 + 0x0B28 + 0x60;

    #[rustfmt::skip]
    let expected: [u8; 0x60] = [
        0x1b, b'!', b'"', b'#', b'$', b'%', b'&', b'\'',
        b'(', b')', b'0', b'=', b'^', b'|', 0x08, 0x09,
        b'Q', b'W', b'E', b'R', b'T', b'Y', b'U', b'I',
        b'O', b'P', b'~', b'{', 0x0d, b'A', b'S', b'D',
        b'F', b'G', b'H', b'J', b'K', b'L', b'+', b'*',
        b'}', b'Z', b'X', b'C', b'V', b'B', b'N', b'M',
        b'<', b'>', b'?', b'_', b' ', 0xa5, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xae, 0x00,
        b'-', b'/', b'7', b'8', b'9', b'*', b'4', b'5',
        b'6', b'+', b'1', b'2', b'3', b'=', b'0', b',',
        b'.', 0xa1, 0xff, 0xff, 0xff, 0xff, 0x82, 0x83,
        0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b,
    ];

    for (i, &exp) in expected.iter().enumerate() {
        let got = machine.bus.read_byte(base + i as u32);
        assert_eq!(
            got, exp,
            "Keyboard table 1 byte {i:#04X}: expected {exp:#04X}, got {got:#04X}"
        );
    }
}

#[test]
fn keyboard_table_1_full_content_f() {
    let mut machine = create_machine_f();
    let base: u32 = 0xFD800 + 0x0A58 + 0x60;

    #[rustfmt::skip]
    let expected: [u8; 0x60] = [
        0x1b, b'!', b'"', b'#', b'$', b'%', b'&', b'\'',
        b'(', b')', b'0', b'=', b'^', b'|', 0x08, 0x09,
        b'Q', b'W', b'E', b'R', b'T', b'Y', b'U', b'I',
        b'O', b'P', b'~', b'{', 0x0d, b'A', b'S', b'D',
        b'F', b'G', b'H', b'J', b'K', b'L', b'+', b'*',
        b'}', b'Z', b'X', b'C', b'V', b'B', b'N', b'M',
        b'<', b'>', b'?', b'_', b' ', 0xa5, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xae, 0x00,
        b'-', b'/', b'7', b'8', b'9', b'*', b'4', b'5',
        b'6', b'+', b'1', b'2', b'3', b'=', b'0', b',',
        b'.', 0xff, 0xff, 0xff, 0xff, 0xff, 0x82, 0x83,
        0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b,
    ];

    for (i, &exp) in expected.iter().enumerate() {
        let got = machine.bus.read_byte(base + i as u32);
        assert_eq!(
            got, exp,
            "Keyboard table 1 byte {i:#04X}: expected {exp:#04X}, got {got:#04X}"
        );
    }
}

/// Verify disk format tables are installed correctly.
/// The F2HD indirection table at FD80:1AB4 has 4 word entries each pointing to 0x1ABC.
#[test]
fn disk_format_table_f2hd_indirection() {
    let mut machine = create_machine_vm();
    let base: u32 = 0xFD800 + 0x1AB4;

    for i in 0..4 {
        let ptr = machine.bus.read_word(base + i * 2);
        assert_eq!(
            ptr, 0x1ABC,
            "F2HD indirection [{i}] should point to 0x1ABC, got {ptr:#06X}"
        );
    }
}

#[test]
fn bios_code_at_vm_f2hd_indirection_location_f() {
    let mut machine = create_machine_f();
    let base: u32 = 0xFD800 + 0x1AB4;

    let expected = [0xB100, 0xD205, 0xE8E4, 0x027D];
    for (i, &expected_word) in expected.iter().enumerate() {
        let word = machine.bus.read_word(base + (i as u32) * 2);
        assert_eq!(word, expected_word, "F BIOS word [{i}]");
    }
}

/// Verify the F2HD format parameter table content at FD80:1ABC.
#[test]
fn disk_format_table_f2hd_content() {
    let mut machine = create_machine_vm();
    let base: u32 = 0xFD800 + 0x1ABC;

    #[rustfmt::skip]
    let expected: [u8; 32] = [
        0x00, 0x00, 0x00, 0x00, 0x1A, 0x07, 0x1A, 0x1B,
        0x1A, 0x0E, 0x1A, 0x36, 0x0F, 0x0E, 0x0F, 0x2A,
        0x0F, 0x1B, 0x0F, 0x54, 0x08, 0x1B, 0x08, 0x3A,
        0x08, 0x35, 0x08, 0x74, 0x00, 0x00, 0x00, 0x00,
    ];

    for (i, &exp) in expected.iter().enumerate() {
        let got = machine.bus.read_byte(base + i as u32);
        assert_eq!(
            got, exp,
            "F2HD table byte {i}: expected {exp:#04X}, got {got:#04X}"
        );
    }
}

#[test]
fn bios_code_at_vm_f2hd_table_location_f() {
    let mut machine = create_machine_f();
    let base: u32 = 0xFD800 + 0x1ABC;

    #[rustfmt::skip]
    let expected: [u8; 32] = [
        0xB4, 0x00, 0xB2, 0x04, 0xE8, 0x76, 0x02, 0xFE,
        0xCA, 0x75, 0xF9, 0xE8, 0x9A, 0x02, 0xF6, 0xC4,
        0xF0, 0x75, 0x08, 0xE8, 0xF6, 0x01, 0x2E, 0x0A,
        0xA7, 0x14, 0x00, 0xC3, 0xB8, 0x40, 0x08, 0xE8,
    ];

    for (i, &exp) in expected.iter().enumerate() {
        let got = machine.bus.read_byte(base + i as u32);
        assert_eq!(
            got, exp,
            "F2HD table byte {i}: expected {exp:#04X}, got {got:#04X}"
        );
    }
}

/// Verify the F2DD indirection table at FD80:1ADC.
#[test]
fn disk_format_table_f2dd_indirection() {
    let mut machine = create_machine_vm();
    let base: u32 = 0xFD800 + 0x1ADC;

    for i in 0..4 {
        let ptr = machine.bus.read_word(base + i * 2);
        assert_eq!(
            ptr, 0x1AE4,
            "F2DD indirection [{i}] should point to 0x1AE4, got {ptr:#06X}"
        );
    }
}

#[test]
fn bios_code_at_vm_f2dd_indirection_location_f() {
    let mut machine = create_machine_f();
    let base: u32 = 0xFD800 + 0x1ADC;

    let expected = [0x006D, 0xFC80, 0x7508, 0xF609];
    for (i, &expected_word) in expected.iter().enumerate() {
        let word = machine.bus.read_word(base + (i as u32) * 2);
        assert_eq!(word, expected_word, "F BIOS word [{i}]");
    }
}

/// Verify the F2DD format parameter table content at FD80:1AE4.
#[test]
fn disk_format_table_f2dd_content() {
    let mut machine = create_machine_vm();
    let base: u32 = 0xFD800 + 0x1AE4;

    #[rustfmt::skip]
    let expected: [u8; 32] = [
        0x00, 0x00, 0x00, 0x00, 0x10, 0x07, 0x10, 0x1B,
        0x10, 0x0E, 0x10, 0x36, 0x09, 0x0E, 0x09, 0x2A,
        0x09, 0x2A, 0x09, 0x50, 0x05, 0x1B, 0x05, 0x3A,
        0x05, 0x35, 0x05, 0x74, 0x00, 0x00, 0x00, 0x00,
    ];

    for (i, &exp) in expected.iter().enumerate() {
        let got = machine.bus.read_byte(base + i as u32);
        assert_eq!(
            got, exp,
            "F2DD table byte {i}: expected {exp:#04X}, got {got:#04X}"
        );
    }
}

#[test]
fn bios_code_at_vm_f2dd_table_location_f() {
    let mut machine = create_machine_f();
    let base: u32 = 0xFD800 + 0x1AE4;

    #[rustfmt::skip]
    let expected: [u8; 32] = [
        0x46, 0x01, 0x20, 0x74, 0x19, 0x80, 0xCC, 0xB0,
        0xC3, 0xB8, 0x44, 0x08, 0xE8, 0x58, 0x00, 0x80,
        0xFC, 0x08, 0x75, 0x09, 0xF6, 0x46, 0x01, 0x20,
        0x74, 0x04, 0x80, 0xCC, 0xB0, 0xC3, 0xB4, 0x40,
    ];

    for (i, &exp) in expected.iter().enumerate() {
        let got = machine.bus.read_byte(base + i as u32);
        assert_eq!(
            got, exp,
            "F2DD table byte {i}: expected {exp:#04X}, got {got:#04X}"
        );
    }
}
