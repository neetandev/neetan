use crate::harness::{self, *};

const IOSYS_CURSOR_Y: u32 = 0x0600 + 0x0110;
const IOSYS_CURSOR_X: u32 = 0x0600 + 0x011C;

#[test]
fn system_identification() {
    let mut machine = harness::boot_hle();
    // INT DCh CL=12h: System identification.
    // AX returns product number from 0060:0020h.
    // DX returns machine type (0003h = normal-mode PC-98).
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x12, 0x00,                   // MOV CX, 0012h (CL=12h)
        0xB8, 0x00, 0x00,                   // MOV AX, 0000h
        0xCD, 0xDC,                         // INT DCh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let expected_product = harness::read_word(&machine.bus, 0x0600 + 0x0020);
    let ax = harness::result_word(&machine.bus, 0);
    let dx = harness::result_word(&machine.bus, 2);
    assert_eq!(
        ax, expected_product,
        "INT DCh CL=12h: AX should be product number {:#06X}, got {:#06X}",
        expected_product, ax
    );
    assert_eq!(
        dx, 0x0003,
        "INT DCh CL=12h: DX should be 0x0003 (normal-mode PC-98), got {:#06X}",
        dx
    );
}

#[test]
fn daua_mapping_buffer() {
    let mut machine = harness::boot_hle();
    // INT DCh CL=13h: Fill 96-byte DA/UA buffer at DS:DX.
    let buffer_offset: u16 = harness::INJECT_RESULT_OFFSET + 0x10;
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xB9, 0x13, 0x00,                                     // MOV CX, 0013h (CL=13h)
        0xBA, buffer_offset as u8, (buffer_offset >> 8) as u8, // MOV DX, buffer_offset
        0xCD, 0xDC,                                            // INT DCh
        0xFA,                                                  // CLI
        0xF4,                                                  // HLT
    ];
    harness::inject_and_run(&mut machine, &code);

    // First 16 bytes of the buffer should match DA/UA mapping at 0060:006Ch.
    let buffer_addr = harness::INJECT_RESULT_BASE + 0x10;
    let iosys_daua_addr = 0x0600 + 0x006C;

    for i in 0..16u32 {
        let from_buffer = harness::read_byte(&machine.bus, buffer_addr + i);
        let from_iosys = harness::read_byte(&machine.bus, iosys_daua_addr + i);
        assert_eq!(
            from_buffer, from_iosys,
            "DA/UA buffer byte {} ({:#04X}) should match IO.SYS table ({:#04X})",
            i, from_buffer, from_iosys
        );
    }
}

#[test]
fn internal_revision() {
    let mut machine = harness::boot_hle();
    // INT DCh CL=15h: Returns internal revision from 0060:0022h in AL.
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x15, 0x00,                   // MOV CX, 0015h (CL=15h)
        0xCD, 0xDC,                         // INT DCh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let expected = harness::read_byte(&machine.bus, 0x0600 + 0x0022);
    let al = harness::result_byte(&machine.bus, 0);
    assert_eq!(
        al, expected,
        "INT DCh CL=15h: AL should be revision {:#04X}, got {:#04X}",
        expected, al
    );
}

#[test]
fn extended_memory_query() {
    let mut machine = harness::boot_hle();
    // INT DCh CL=81h: Returns extended memory size from 0060:0031h in AL.
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x81, 0x00,                   // MOV CX, 0081h (CL=81h)
        0xCD, 0xDC,                         // INT DCh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let expected = harness::read_byte(&machine.bus, 0x0600 + 0x0031);
    let al = harness::result_byte(&machine.bus, 0);
    assert_eq!(
        al, expected,
        "INT DCh CL=81h: AL should be ext mem {:#04X}, got {:#04X}",
        expected, al
    );
}

#[test]
fn noop_functions_00h_through_08h() {
    let mut machine = harness::boot_hle();
    // Call INT DCh with CL=00h through CL=08h. These are documented no-ops.
    // They should return without hanging or crashing.
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB1, 0x00,                         // MOV CL, 00h
        // loop:
        0xB5, 0x00,                         // MOV CH, 00h
        0xCD, 0xDC,                         // INT DCh
        0xFE, 0xC1,                         // INC CL
        0x80, 0xF9, 0x09,                   // CMP CL, 09h
        0x72, 0xF5,                         // JB loop (back to MOV CH, 00h)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    // If we reach here, all 9 calls completed without hanging.
}

#[test]
fn disk_partition_info_80h() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x80, 0x00,                   // MOV CX, 0080h (CL=80h)
        0xB0, 0x00,                         // MOV AL, 00h
        0xB4, 0x00,                         // MOV AH, 00h
        0xCD, 0xDC,                         // INT DCh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    // Verify the call completed without hanging. The return values
    // depend on the drive configuration.
    let _ax = harness::result_word(&machine.bus, 0);
    let _bx = harness::result_word(&machine.bus, 2);
}

#[test]
fn fnkey_write_then_read_roundtrip() {
    let mut machine = harness::boot_hle();
    let base = INJECT_CODE_BASE;

    // Write 16 test bytes at +0x0200 (data to write as F1 key mapping)
    let test_data: [u8; 16] = [
        0x1B, 0x5B, 0x31, 0x7E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];
    write_bytes(&mut machine.bus, base + 0x0200, &test_data);

    // Zero out read buffer at +0x0220
    write_bytes(&mut machine.bus, base + 0x0220, &[0u8; 16]);

    let seg_lo = (INJECT_CODE_SEGMENT & 0xFF) as u8;
    let seg_hi = (INJECT_CODE_SEGMENT >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Write F1 mapping: CL=0Dh, AX=0001h, DS:DX=seg:0200h
        0xB8, seg_lo, seg_hi,               // MOV AX, INJECT_CODE_SEGMENT
        0x8E, 0xD8,                         // MOV DS, AX
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xB8, 0x01, 0x00,                   // MOV AX, 0001h (F1 key specifier)
        0xB9, 0x0D, 0x00,                   // MOV CX, 000Dh (CL=0Dh = write)
        0xCD, 0xDC,                         // INT DCh
        // Read F1 mapping: CL=0Ch, AX=0001h, DS:DX=seg:0220h
        0xBA, 0x20, 0x02,                   // MOV DX, 0220h
        0xB8, 0x01, 0x00,                   // MOV AX, 0001h
        0xB9, 0x0C, 0x00,                   // MOV CX, 000Ch (CL=0Ch = read)
        0xCD, 0xDC,                         // INT DCh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run(&mut machine, code);

    // Verify the read buffer matches what we wrote
    for i in 0..16u32 {
        let expected = test_data[i as usize];
        let actual = machine.bus.read_byte_direct(base + 0x0220 + i);
        assert_eq!(
            actual, expected,
            "fnkey map byte {i}: expected {expected:#04X}, got {actual:#04X}"
        );
    }
}

#[test]
fn intdch_10h_04h_cursor_down_one_line() {
    let mut machine = harness::boot_hle();
    harness::set_cursor_position(&mut machine.bus, 5, 10);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x10, 0x00,       // MOV CX, 0010h (CL=10h)
        0xB4, 0x04,             // MOV AH, 04h
        0xCD, 0xDC,             // INT DCh
        0xFA, 0xF4,             // CLI; HLT
    ];
    inject_and_run(&mut machine, code);
    let row = machine.bus.read_byte_direct(IOSYS_CURSOR_Y);
    let col = machine.bus.read_byte_direct(IOSYS_CURSOR_X);
    assert_eq!(row, 6, "AH=04h should move cursor down 1 row");
    assert_eq!(col, 10, "AH=04h should preserve column");
}

#[test]
fn intdch_10h_05h_cursor_up_one_line() {
    let mut machine = harness::boot_hle();
    harness::set_cursor_position(&mut machine.bus, 5, 10);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x10, 0x00,       // MOV CX, 0010h (CL=10h)
        0xB4, 0x05,             // MOV AH, 05h
        0xCD, 0xDC,             // INT DCh
        0xFA, 0xF4,             // CLI; HLT
    ];
    inject_and_run(&mut machine, code);
    let row = machine.bus.read_byte_direct(IOSYS_CURSOR_Y);
    let col = machine.bus.read_byte_direct(IOSYS_CURSOR_X);
    assert_eq!(row, 4, "AH=05h should move cursor up 1 row");
    assert_eq!(col, 10, "AH=05h should preserve column");
}

#[test]
fn intdch_10h_06h_cursor_up_n_lines() {
    let mut machine = harness::boot_hle();
    harness::set_cursor_position(&mut machine.bus, 10, 5);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x10, 0x00,       // MOV CX, 0010h (CL=10h)
        0xB4, 0x06,             // MOV AH, 06h
        0xB2, 0x03,             // MOV DL, 03h
        0xCD, 0xDC,             // INT DCh
        0xFA, 0xF4,             // CLI; HLT
    ];
    inject_and_run(&mut machine, code);
    let row = machine.bus.read_byte_direct(IOSYS_CURSOR_Y);
    let col = machine.bus.read_byte_direct(IOSYS_CURSOR_X);
    assert_eq!(row, 7, "AH=06h DL=3 should move cursor up 3 rows");
    assert_eq!(col, 5, "AH=06h should preserve column");
}

#[test]
fn intdch_10h_07h_cursor_down_n_lines() {
    let mut machine = harness::boot_hle();
    harness::set_cursor_position(&mut machine.bus, 5, 10);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x10, 0x00,       // MOV CX, 0010h (CL=10h)
        0xB4, 0x07,             // MOV AH, 07h
        0xB2, 0x03,             // MOV DL, 03h
        0xCD, 0xDC,             // INT DCh
        0xFA, 0xF4,             // CLI; HLT
    ];
    inject_and_run(&mut machine, code);
    let row = machine.bus.read_byte_direct(IOSYS_CURSOR_Y);
    let col = machine.bus.read_byte_direct(IOSYS_CURSOR_X);
    assert_eq!(row, 8, "AH=07h DL=3 should move cursor down 3 rows");
    assert_eq!(col, 10, "AH=07h should preserve column");
}

#[test]
fn intdch_10h_08h_cursor_right_n_cols() {
    let mut machine = harness::boot_hle();
    harness::set_cursor_position(&mut machine.bus, 5, 10);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x10, 0x00,       // MOV CX, 0010h (CL=10h)
        0xB4, 0x08,             // MOV AH, 08h
        0xB2, 0x05,             // MOV DL, 05h
        0xCD, 0xDC,             // INT DCh
        0xFA, 0xF4,             // CLI; HLT
    ];
    inject_and_run(&mut machine, code);
    let row = machine.bus.read_byte_direct(IOSYS_CURSOR_Y);
    let col = machine.bus.read_byte_direct(IOSYS_CURSOR_X);
    assert_eq!(row, 5, "AH=08h should preserve row");
    assert_eq!(col, 15, "AH=08h DL=5 should move cursor right 5 columns");
}

#[test]
fn intdch_10h_09h_cursor_left_n_cols() {
    let mut machine = harness::boot_hle();
    harness::set_cursor_position(&mut machine.bus, 5, 10);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x10, 0x00,       // MOV CX, 0010h (CL=10h)
        0xB4, 0x09,             // MOV AH, 09h
        0xB2, 0x03,             // MOV DL, 03h
        0xCD, 0xDC,             // INT DCh
        0xFA, 0xF4,             // CLI; HLT
    ];
    inject_and_run(&mut machine, code);
    let row = machine.bus.read_byte_direct(IOSYS_CURSOR_Y);
    let col = machine.bus.read_byte_direct(IOSYS_CURSOR_X);
    assert_eq!(row, 5, "AH=09h should preserve row");
    assert_eq!(col, 7, "AH=09h DL=3 should move cursor left 3 columns");
}

#[test]
fn intdch_0fh_enable_ctrl_fn() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // AX=0000h: enable CTRL+Fn for application.
        0xB9, 0x0F, 0x00,       // MOV CX, 000Fh (CL=0Fh)
        0xB8, 0x00, 0x00,       // MOV AX, 0000h
        0xCD, 0xDC,             // INT DCh
        // AX=8000h: read CTRL+Fn state.
        0xB8, 0x00, 0x80,       // MOV AX, 8000h
        0xCD, 0xDC,             // INT DCh
        0xA3, 0x00, 0x01,       // MOV [0100h], AX
        0xFA, 0xF4,             // CLI; HLT
    ];
    inject_and_run(&mut machine, code);
    let ax = harness::result_word(&machine.bus, 0);
    assert_eq!(
        ax, 0x0000,
        "after AX=0000h, read-back should be 0000h (app has control)"
    );
}

#[test]
fn intdch_0fh_disable_ctrl_fn() {
    let mut machine = harness::boot_hle();
    // First enable, then disable.
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x0F, 0x00,       // MOV CX, 000Fh
        0xB8, 0x00, 0x00,       // MOV AX, 0000h (enable)
        0xCD, 0xDC,             // INT DCh
        0xB8, 0x01, 0x00,       // MOV AX, 0001h (disable)
        0xCD, 0xDC,             // INT DCh
        0xB8, 0x00, 0x80,       // MOV AX, 8000h (read)
        0xCD, 0xDC,             // INT DCh
        0xA3, 0x00, 0x01,       // MOV [0100h], AX
        0xFA, 0xF4,             // CLI; HLT
    ];
    inject_and_run(&mut machine, code);
    let ax = harness::result_word(&machine.bus, 0);
    assert_eq!(
        ax, 0x0001,
        "after AX=0001h, read-back should be 0001h (DOS has control)"
    );
}

#[test]
fn intdch_0fh_enable_ctrl_xfer() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x0F, 0x00,       // MOV CX, 000Fh
        0xB8, 0x02, 0x00,       // MOV AX, 0002h (enable CTRL+XFER/NFER)
        0xCD, 0xDC,             // INT DCh
        0xB8, 0x02, 0x80,       // MOV AX, 8002h (read CTRL+XFER/NFER state)
        0xCD, 0xDC,             // INT DCh
        0xA3, 0x00, 0x01,       // MOV [0100h], AX
        0xFA, 0xF4,             // CLI; HLT
    ];
    inject_and_run(&mut machine, code);
    let ax = harness::result_word(&machine.bus, 0);
    assert_eq!(
        ax, 0x0000,
        "after AX=0002h, read-back should be 0000h (app has control)"
    );
}

#[test]
fn intdch_0fh_default_state() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x0F, 0x00,       // MOV CX, 000Fh
        0xB8, 0x00, 0x80,       // MOV AX, 8000h (read CTRL+Fn state)
        0xCD, 0xDC,             // INT DCh
        0xA3, 0x00, 0x01,       // MOV [0100h], AX
        0xFA, 0xF4,             // CLI; HLT
    ];
    inject_and_run(&mut machine, code);
    let ax = harness::result_word(&machine.bus, 0);
    assert_eq!(
        ax, 0x0001,
        "default state should be 0001h (DOS has control)"
    );
}

#[test]
fn intdch_0dh_write_extended_read_basic_extracts_correctly() {
    let mut machine = harness::boot_hle();

    // Fill a 786-byte extended-format source buffer at DS:0200h.
    // Extended layout:
    //   0-159:   F1-F10 (160 bytes)
    //   160-239: VF1-VF5 (80 bytes, skipped on read-back)
    //   240-399: Shift+F1-F10 (160 bytes)
    //   400-479: Shift+VF1-VF5 (80 bytes, skipped)
    //   480-545: Editing keys (66 bytes)
    //   546-705: Ctrl+F1-F10 (160 bytes, skipped)
    //   706-785: Ctrl+VF1-VF5 (80 bytes, skipped)
    let src_base = INJECT_CODE_BASE + 0x0200;
    let pattern: Vec<u8> = (0..786).map(|i| ((i % 251) + 1) as u8).collect();
    harness::write_bytes(&mut machine.bus, src_base, &pattern);

    // Write with AX=0xFF (extended), read back with AX=0x0000 (basic 386 bytes).
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x0D, 0x00,       // MOV CX, 000Dh
        0xB8, 0xFF, 0x00,       // MOV AX, 00FFh
        0xBA, 0x00, 0x02,       // MOV DX, 0200h
        0xCD, 0xDC,             // INT DCh
        0xB9, 0x0C, 0x00,       // MOV CX, 000Ch
        0xB8, 0x00, 0x00,       // MOV AX, 0000h
        0xBA, 0x00, 0x05,       // MOV DX, 0500h
        0xCD, 0xDC,             // INT DCh
        0xFA, 0xF4,             // CLI; HLT
    ];
    inject_and_run(&mut machine, code);

    let dst_base = INJECT_CODE_BASE + 0x0500;

    // Basic layout read-back (386 bytes):
    //   0-159: F1-F10 -- should match extended offset 0-159
    for i in 0..160u32 {
        let expected = pattern[i as usize]; // ext offset 0-159
        let actual = machine.bus.read_byte_direct(dst_base + i);
        assert_eq!(actual, expected, "F1-F10 byte {i}");
    }

    // 160-319: Shift+F1-F10 -- should match extended offset 240-399
    for i in 0..160u32 {
        let expected = pattern[(240 + i) as usize]; // ext offset 240+
        let actual = machine.bus.read_byte_direct(dst_base + 160 + i);
        assert_eq!(actual, expected, "Shift+F1-F10 byte {i}");
    }

    // 320-385: Editing keys -- should match extended offset 480-545
    for i in 0..66u32 {
        let expected = pattern[(480 + i) as usize]; // ext offset 480+
        let actual = machine.bus.read_byte_direct(dst_base + 320 + i);
        assert_eq!(actual, expected, "Editing key byte {i}");
    }
}
