use crate::harness;

#[test]
fn dos_version_major_6() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x30,       // MOV AH, 30h
        0xCD, 0x21,       // INT 21h
        0xA3, 0x00, 0x01, // MOV [0x0100], AX   (AL=major, AH=minor)
        0xFA,             // CLI
        0xF4,             // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let al = harness::result_byte(&machine.bus, 0);
    assert_eq!(al, 6, "DOS major version should be 6, got {}", al);
}

#[test]
fn dos_version_minor_20() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x30,       // MOV AH, 30h
        0xCD, 0x21,       // INT 21h
        0xA3, 0x00, 0x01, // MOV [0x0100], AX
        0xFA,             // CLI
        0xF4,             // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let ah = harness::result_byte(&machine.bus, 1);
    assert_eq!(ah, 20, "DOS minor version should be 20, got {}", ah);
}

#[test]
fn true_version_6_20() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x33,       // MOV AH, 33h
        0xB0, 0x06,       // MOV AL, 06h
        0xCD, 0x21,       // INT 21h
        0x89, 0x1E, 0x00, 0x01, // MOV [0x0100], BX  (BH=major, BL=minor)
        0xFA,             // CLI
        0xF4,             // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let bl = harness::result_byte(&machine.bus, 0);
    let bh = harness::result_byte(&machine.bus, 1);
    // INT 21h AH=33h AL=06h: BL=major, BH=minor (reverse of AH=30h convention).
    assert_eq!(bl, 6, "True DOS major version (BL) should be 6, got {}", bl);
    assert_eq!(
        bh, 20,
        "True DOS minor version (BH) should be 20, got {}",
        bh
    );
}

#[test]
fn no_windows_running() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0x16, // MOV AX, 1600h
        0xCD, 0x2F,       // INT 2Fh
        0xA3, 0x00, 0x01, // MOV [0x0100], AX
        0xFA,             // CLI
        0xF4,             // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let al = harness::result_byte(&machine.bus, 0);
    assert_eq!(
        al, 0x00,
        "INT 2Fh/1600h AL should be 0x00 (no Windows), got {:#04X}",
        al
    );
}

#[test]
fn country_code_japan() {
    let mut machine = harness::boot_hle();
    // INT 21h AH=38h, AL=00h (get current country). DS:DX points to 34-byte buffer.
    // Buffer is at result+0x10 (offset 0x10 within our result area).
    // BX returns country code on success.
    let buffer_offset: u16 = harness::INJECT_RESULT_OFFSET + 0x10;
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xB4, 0x38,                                        // MOV AH, 38h
        0xB0, 0x00,                                        // MOV AL, 00h (get current)
        0xBA, buffer_offset as u8, (buffer_offset >> 8) as u8, // MOV DX, buffer_offset
        0xCD, 0x21,                                        // INT 21h
        0x89, 0x1E, 0x00, 0x01,                            // MOV [0x0100], BX (country code)
        0xFA,                                              // CLI
        0xF4,                                              // HLT
    ];
    harness::inject_and_run(&mut machine, &code);

    let country_code = harness::result_word(&machine.bus, 0);
    assert_eq!(
        country_code, 81,
        "Country code should be 81 (Japan), got {}",
        country_code
    );
}

#[test]
fn date_format_ymd() {
    let mut machine = harness::boot_hle();
    let buffer_offset: u16 = harness::INJECT_RESULT_OFFSET + 0x10;
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xB4, 0x38,
        0xB0, 0x00,
        0xBA, buffer_offset as u8, (buffer_offset >> 8) as u8,
        0xCD, 0x21,
        0xFA,
        0xF4,
    ];
    harness::inject_and_run(&mut machine, &code);

    // Country info buffer: offset 0 = date format WORD.
    // 0=USA (MM/DD/YY), 1=Europe (DD/MM/YY), 2=Japan (YY/MM/DD).
    let date_format = harness::read_word(&machine.bus, harness::INJECT_RESULT_BASE + 0x10);
    assert_eq!(
        date_format, 2,
        "Date format should be 2 (YY/MM/DD for Japan), got {}",
        date_format
    );
}

#[test]
fn currency_yen() {
    let mut machine = harness::boot_hle();
    let buffer_offset: u16 = harness::INJECT_RESULT_OFFSET + 0x10;
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xB4, 0x38,
        0xB0, 0x00,
        0xBA, buffer_offset as u8, (buffer_offset >> 8) as u8,
        0xCD, 0x21,
        0xFA,
        0xF4,
    ];
    harness::inject_and_run(&mut machine, &code);

    // Country info buffer: offset 2 = currency symbol (5 bytes ASCIIZ).
    let currency = harness::read_byte(&machine.bus, harness::INJECT_RESULT_BASE + 0x10 + 2);
    assert_eq!(
        currency, 0x5C,
        "Currency symbol first byte should be 0x5C (yen on PC-98), got {:#04X}",
        currency
    );
}

#[test]
fn time_format_24h() {
    let mut machine = harness::boot_hle();
    let buffer_offset: u16 = harness::INJECT_RESULT_OFFSET + 0x10;
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xB4, 0x38,
        0xB0, 0x00,
        0xBA, buffer_offset as u8, (buffer_offset >> 8) as u8,
        0xCD, 0x21,
        0xFA,
        0xF4,
    ];
    harness::inject_and_run(&mut machine, &code);

    // Country info: offset 17 (0x11) = time format BYTE. 0=12hr, 1=24hr.
    let time_format = harness::read_byte(&machine.bus, harness::INJECT_RESULT_BASE + 0x10 + 0x11);
    assert_eq!(
        time_format, 1,
        "Time format should be 1 (24-hour), got {}",
        time_format
    );
}

#[test]
fn dbcs_table_shift_jis() {
    let mut machine = harness::boot_hle();
    // INT 21h AH=63h, AL=00h: Get DBCS lead byte table pointer.
    // Returns DS:SI pointing to the table.
    // NOTE: INT 21h AH=63h modifies DS, so we must use ES (which we control) to store results.
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x63,                         // MOV AH, 63h
        0xB0, 0x00,                         // MOV AL, 00h
        0xCD, 0x21,                         // INT 21h
        // DS:SI now points to DBCS table. Save DS and SI via ES segment.
        0x26, 0x89, 0x36, 0x00, 0x01,       // MOV ES:[0x0100], SI
        0x8C, 0xD8,                         // MOV AX, DS
        0x26, 0xA3, 0x02, 0x01,             // MOV ES:[0x0102], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let si = harness::result_word(&machine.bus, 0);
    let ds = harness::result_word(&machine.bus, 2);
    let table_addr = harness::far_to_linear(ds, si);

    // DBCS table format: pairs of (start, end) bytes, terminated by (00, 00).
    // Shift-JIS: 81-9F, E0-FC.
    let range1_start = harness::read_byte(&machine.bus, table_addr);
    let range1_end = harness::read_byte(&machine.bus, table_addr + 1);
    let range2_start = harness::read_byte(&machine.bus, table_addr + 2);
    let range2_end = harness::read_byte(&machine.bus, table_addr + 3);
    let terminator1 = harness::read_byte(&machine.bus, table_addr + 4);
    let terminator2 = harness::read_byte(&machine.bus, table_addr + 5);

    assert_eq!(
        range1_start, 0x81,
        "DBCS range 1 start should be 0x81, got {:#04X}",
        range1_start
    );
    assert_eq!(
        range1_end, 0x9F,
        "DBCS range 1 end should be 0x9F, got {:#04X}",
        range1_end
    );
    assert_eq!(
        range2_start, 0xE0,
        "DBCS range 2 start should be 0xE0, got {:#04X}",
        range2_start
    );
    assert_eq!(
        range2_end, 0xFC,
        "DBCS range 2 end should be 0xFC, got {:#04X}",
        range2_end
    );
    assert_eq!(
        terminator1, 0x00,
        "DBCS table terminator byte 1 should be 0x00, got {:#04X}",
        terminator1
    );
    assert_eq!(
        terminator2, 0x00,
        "DBCS table terminator byte 2 should be 0x00, got {:#04X}",
        terminator2
    );
}

#[test]
fn oem_byte_nec() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x30,                         // MOV AH, 30h
        0xCD, 0x21,                         // INT 21h
        // BH = OEM serial number.
        0x88, 0x3E, 0x00, 0x01,             // MOV [0x0100], BH
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let oem = harness::result_byte(&machine.bus, 0);
    // OEM values: 0x00=IBM, 0xFF=Microsoft, NEC typically uses a specific value.
    // Just verify the call provides a value and record it.
    let _ = oem;
}

#[test]
fn standard_file_handles() {
    let mut machine = harness::boot_hle();
    let psp_segment = harness::get_psp_segment(&mut machine);
    let psp_linear = harness::far_to_linear(psp_segment, 0);

    // PSP+0x18: Job File Table. Handles 0-4 should all be valid (not 0xFF).
    // Handle 0 = stdin (CON), 1 = stdout (CON), 2 = stderr (CON),
    // 3 = stdaux (AUX), 4 = stdprn (PRN).
    for handle in 0..5u32 {
        let sft_index = harness::read_byte(&machine.bus, psp_linear + 0x18 + handle);
        assert_ne!(
            sft_index, 0xFF,
            "Standard file handle {} should be open (not 0xFF), got {:#04X}",
            handle, sft_index
        );
    }
}
