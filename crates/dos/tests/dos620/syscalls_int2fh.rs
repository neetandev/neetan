use crate::harness;

#[test]
fn windows_not_running() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0x16,                   // MOV AX, 1600h
        0xCD, 0x2F,                         // INT 2Fh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
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
fn xms_check() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0x43,                   // MOV AX, 4300h
        0xCD, 0x2F,                         // INT 2Fh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let al = harness::result_byte(&machine.bus, 0);
    assert_eq!(
        al, 0x80,
        "XMS check: AL should be 0x80 (installed), got {:#04X}",
        al
    );
}

#[test]
fn doskey_check() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0x48,                   // MOV AX, 4800h
        0xCD, 0x2F,                         // INT 2Fh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let al = harness::result_byte(&machine.bus, 0);
    assert_eq!(
        al, 0x00,
        "DOSKEY check: AL should be 0x00 (not installed), got {:#04X}",
        al
    );
}

#[test]
fn hma_query() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x01, 0x4A,                   // MOV AX, 4A01h
        0xCD, 0x2F,                         // INT 2Fh
        0x89, 0x1E, 0x00, 0x01,             // MOV [0x0100], BX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let bx = harness::result_word(&machine.bus, 0);
    assert_eq!(
        bx, 0xFFFF,
        "HMA free space should be 0xFFFF (HMA available), got {:#06X}",
        bx
    );
}

#[test]
fn mscdex_installation_check_no_cdrom() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0x15,                   // MOV AX, 1500h
        0xBB, 0xFF, 0xFF,                   // MOV BX, FFFFh (sentinel)
        0xCD, 0x2F,                         // INT 2Fh
        0x89, 0x1E, 0x00, 0x01,             // MOV [0x0100], BX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let bx = harness::result_word(&machine.bus, 0);
    assert_eq!(
        bx, 0x0000,
        "MSCDEX install check: BX should be 0 (no CD-ROM drives), got {:#06X}",
        bx
    );
}

#[test]
fn mscdex_drive_check_no_cdrom() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x0B, 0x15,                   // MOV AX, 150Bh
        0xB9, 0x10, 0x00,                   // MOV CX, 16 (Q:)
        0xCD, 0x2F,                         // INT 2Fh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let ax = harness::result_word(&machine.bus, 0);
    let al = ax as u8;
    assert_eq!(
        al, 0x00,
        "MSCDEX drive check: AL should be 0 (not CD-ROM), got {:#04X}",
        al
    );
}

#[test]
fn mscdex_version() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x0C, 0x15,                   // MOV AX, 150Ch
        0xCD, 0x2F,                         // INT 2Fh
        0x89, 0x1E, 0x00, 0x01,             // MOV [0x0100], BX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let bx = harness::result_word(&machine.bus, 0);
    assert_eq!(
        bx, 0x020A,
        "MSCDEX version: BX should be 020Ah (v2.10), got {:#06X}",
        bx
    );
}

#[test]
fn mscdex_installation_check_with_cdrom() {
    let mut machine = harness::boot_hle_with_cdrom();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0x15,                   // MOV AX, 1500h
        0xCD, 0x2F,                         // INT 2Fh
        0x89, 0x1E, 0x00, 0x01,             // MOV [0x0100], BX
        0x89, 0x0E, 0x02, 0x01,             // MOV [0x0102], CX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let bx = harness::result_word(&machine.bus, 0);
    let cx = harness::result_word(&machine.bus, 2);
    assert_eq!(
        bx, 0x0001,
        "MSCDEX install check: BX should be 1 (one CD-ROM drive), got {:#06X}",
        bx
    );
    assert_eq!(
        cx, 0x0010,
        "MSCDEX install check: CX should be 16 (Q: drive letter), got {:#06X}",
        cx
    );
}

#[test]
fn mscdex_drive_check_with_cdrom() {
    let mut machine = harness::boot_hle_with_cdrom();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x0B, 0x15,                   // MOV AX, 150Bh
        0xB9, 0x10, 0x00,                   // MOV CX, 16 (Q:)
        0xCD, 0x2F,                         // INT 2Fh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let al = harness::result_byte(&machine.bus, 0);
    let bx = harness::result_word(&machine.bus, 2);
    assert_ne!(
        al, 0x00,
        "MSCDEX drive check: AL should be non-zero (is CD-ROM), got {:#04X}",
        al
    );
    assert_eq!(
        bx, 0xADAD,
        "MSCDEX drive check: BX should be ADADh, got {:#06X}",
        bx
    );
}

#[test]
fn mscdex_drive_check_wrong_drive() {
    let mut machine = harness::boot_hle_with_cdrom();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x0B, 0x15,                   // MOV AX, 150Bh
        0xB9, 0x02, 0x00,                   // MOV CX, 2 (C: -- not CD-ROM)
        0xCD, 0x2F,                         // INT 2Fh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let al = harness::result_byte(&machine.bus, 0);
    assert_eq!(
        al, 0x00,
        "MSCDEX drive check for C: should return AL=0 (not CD-ROM), got {:#04X}",
        al
    );
}

#[test]
fn mscdex_get_drive_letters() {
    let mut machine = harness::boot_hle_with_cdrom();
    // Set up a buffer at DS:0200h, then call INT 2Fh AX=150Dh with ES:BX pointing there.
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x02,                   // MOV BX, 0200h
        0xC6, 0x07, 0xFF,                   // MOV BYTE [BX], FFh (sentinel)
        0xB8, 0x0D, 0x15,                   // MOV AX, 150Dh
        0xCD, 0x2F,                         // INT 2Fh
        // Copy result from [0200h] to result area [0100h].
        0x8A, 0x07,                         // MOV AL, [BX]
        0xA2, 0x00, 0x01,                   // MOV [0100h], AL
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let drive_letter = harness::result_byte(&machine.bus, 0);
    assert_eq!(
        drive_letter, 16,
        "MSCDEX get drive letters: should return 16 (Q:), got {}",
        drive_letter
    );
}

#[test]
fn mscdex_ioctl_device_status() {
    let mut machine = harness::boot_hle_with_cdrom();

    // Build a device driver request at DS:0200h for IOCTL Input (cmd 3),
    // with a control block at DS:0220h requesting device status (code 6).
    let base = harness::INJECT_CODE_BASE;
    let req_off: u16 = 0x0200;
    let req_addr = base + req_off as u32;
    let ctl_off: u16 = 0x0220;
    let ctl_addr = base + ctl_off as u32;

    // Request header: length=26, subunit=0, command=3 (IOCTL Input).
    harness::write_bytes(&mut machine.bus, req_addr, &[26, 0, 3]);
    // Status word at +3 (zeroed).
    harness::write_bytes(&mut machine.bus, req_addr + 3, &[0, 0]);
    // Reserved 8 bytes at +5.
    harness::write_bytes(&mut machine.bus, req_addr + 5, &[0; 8]);
    // Transfer address at +14: far pointer to control block.
    let seg = harness::INJECT_CODE_SEGMENT;
    harness::write_bytes(
        &mut machine.bus,
        req_addr + 14,
        &[
            ctl_off as u8,
            (ctl_off >> 8) as u8,
            seg as u8,
            (seg >> 8) as u8,
        ],
    );

    // Control block: code 6 = device status.
    harness::write_bytes(&mut machine.bus, ctl_addr, &[6]);
    // Clear the result area (4 bytes at ctl+1).
    harness::write_bytes(&mut machine.bus, ctl_addr + 1, &[0, 0, 0, 0]);

    // Call INT 2Fh AX=1510h, CX=drive (Q:=16), ES:BX=request header.
    let seg_lo = (seg & 0xFF) as u8;
    let seg_hi = (seg >> 8) as u8;
    let req_lo = (req_off & 0xFF) as u8;
    let req_hi = (req_off >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, seg_lo, seg_hi,               // MOV AX, seg
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, req_lo, req_hi,               // MOV BX, req_off
        0xB9, 0x10, 0x00,                   // MOV CX, 16
        0xB8, 0x10, 0x15,                   // MOV AX, 1510h
        0xCD, 0x2F,                         // INT 2Fh
        // Copy status flags from control block to result area.
        0xA1, ctl_off.wrapping_add(1) as u8,
              (ctl_off.wrapping_add(1) >> 8) as u8, // MOV AX, [ctl+1]
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX
        0xA1, ctl_off.wrapping_add(3) as u8,
              (ctl_off.wrapping_add(3) >> 8) as u8, // MOV AX, [ctl+3]
        0xA3, 0x02, 0x01,                   // MOV [0102h], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let flags_lo = harness::result_word(&machine.bus, 0);
    let flags_hi = harness::result_word(&machine.bus, 2);
    let flags = flags_lo as u32 | ((flags_hi as u32) << 16);

    // Expected: door unlocked (0x02), cooked+raw (0x04), audio (0x10),
    // prefetch (0x80), audio channels (0x100), red book (0x200), disc present (0x800).
    assert!(
        flags & 0x02 != 0,
        "Device status: door unlocked bit should be set, flags={flags:#06X}"
    );
    assert!(
        flags & 0x04 != 0,
        "Device status: cooked+raw bit should be set, flags={flags:#06X}"
    );
    assert!(
        flags & 0x10 != 0,
        "Device status: audio play bit should be set, flags={flags:#06X}"
    );
    assert!(
        flags & 0x200 != 0,
        "Device status: Red Book bit should be set, flags={flags:#06X}"
    );
    assert!(
        flags & 0x800 != 0,
        "Device status: disc present bit should be set, flags={flags:#06X}"
    );
}

#[test]
fn mscdex_ioctl_audio_disk_info() {
    let mut machine = harness::boot_hle_with_cdrom();

    let base = harness::INJECT_CODE_BASE;
    let req_off: u16 = 0x0200;
    let req_addr = base + req_off as u32;
    let ctl_off: u16 = 0x0220;
    let ctl_addr = base + ctl_off as u32;

    // Request header: IOCTL Input (cmd 3).
    harness::write_bytes(&mut machine.bus, req_addr, &[26, 0, 3, 0, 0]);
    harness::write_bytes(&mut machine.bus, req_addr + 5, &[0; 8]);
    // Transfer address -> control block.
    let seg = harness::INJECT_CODE_SEGMENT;
    harness::write_bytes(
        &mut machine.bus,
        req_addr + 14,
        &[
            ctl_off as u8,
            (ctl_off >> 8) as u8,
            seg as u8,
            (seg >> 8) as u8,
        ],
    );

    // Control block: code 10 = audio disk info.
    harness::write_bytes(&mut machine.bus, ctl_addr, &[10]);
    harness::write_bytes(&mut machine.bus, ctl_addr + 1, &[0; 6]);

    let seg_lo = (seg & 0xFF) as u8;
    let seg_hi = (seg >> 8) as u8;
    let req_lo = (req_off & 0xFF) as u8;
    let req_hi = (req_off >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, seg_lo, seg_hi,
        0x8E, 0xC0,
        0xBB, req_lo, req_hi,
        0xB9, 0x10, 0x00,
        0xB8, 0x10, 0x15,
        0xCD, 0x2F,
        // Copy 7 bytes from control block to result area.
        0xBE, ctl_off as u8, (ctl_off >> 8) as u8, // MOV SI, ctl_off
        0xBF, 0x00, 0x01,                           // MOV DI, 0100h
        0xB9, 0x07, 0x00,                           // MOV CX, 7
        0xF3, 0xA4,                                  // REP MOVSB
        0xFA,
        0xF4,
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    // Code byte.
    let code_byte = harness::result_byte(&machine.bus, 0);
    assert_eq!(code_byte, 10, "IOCTL code should still be 10");

    // First track.
    let first_track = harness::result_byte(&machine.bus, 1);
    assert_eq!(first_track, 1, "First track should be 1, got {first_track}");

    // Last track (test image has 2 tracks).
    let last_track = harness::result_byte(&machine.bus, 2);
    assert_eq!(last_track, 2, "Last track should be 2, got {last_track}");
}

#[test]
fn mscdex_ioctl_volume_size() {
    let mut machine = harness::boot_hle_with_cdrom();

    let base = harness::INJECT_CODE_BASE;
    let req_off: u16 = 0x0200;
    let req_addr = base + req_off as u32;
    let ctl_off: u16 = 0x0220;
    let ctl_addr = base + ctl_off as u32;

    // Request header: IOCTL Input (cmd 3).
    harness::write_bytes(&mut machine.bus, req_addr, &[26, 0, 3, 0, 0]);
    harness::write_bytes(&mut machine.bus, req_addr + 5, &[0; 8]);
    let seg = harness::INJECT_CODE_SEGMENT;
    harness::write_bytes(
        &mut machine.bus,
        req_addr + 14,
        &[
            ctl_off as u8,
            (ctl_off >> 8) as u8,
            seg as u8,
            (seg >> 8) as u8,
        ],
    );

    // Control block: code 8 = volume size.
    harness::write_bytes(&mut machine.bus, ctl_addr, &[8]);
    harness::write_bytes(&mut machine.bus, ctl_addr + 1, &[0; 4]);

    let seg_lo = (seg & 0xFF) as u8;
    let seg_hi = (seg >> 8) as u8;
    let req_lo = (req_off & 0xFF) as u8;
    let req_hi = (req_off >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, seg_lo, seg_hi,
        0x8E, 0xC0,
        0xBB, req_lo, req_hi,
        0xB9, 0x10, 0x00,
        0xB8, 0x10, 0x15,
        0xCD, 0x2F,
        // Copy 4 bytes of volume size from ctl+1 to result.
        0xA1, ctl_off.wrapping_add(1) as u8,
              (ctl_off.wrapping_add(1) >> 8) as u8,
        0xA3, 0x00, 0x01,
        0xA1, ctl_off.wrapping_add(3) as u8,
              (ctl_off.wrapping_add(3) >> 8) as u8,
        0xA3, 0x02, 0x01,
        0xFA,
        0xF4,
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let total_lo = harness::result_word(&machine.bus, 0);
    let total_hi = harness::result_word(&machine.bus, 2);
    let total_sectors = total_lo as u32 | ((total_hi as u32) << 16);
    // Test image: 150 data sectors + 50 audio sectors = 200 total.
    assert_eq!(
        total_sectors, 200,
        "Volume size should be 200 sectors, got {total_sectors}"
    );
}

#[test]
fn mscdex_get_drive_device_list() {
    let mut machine = harness::boot_hle_with_cdrom();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x02,                   // MOV BX, 0200h
        0xB8, 0x01, 0x15,                   // MOV AX, 1501h
        0xCD, 0x2F,                         // INT 2Fh
        0xBE, 0x00, 0x02,                   // MOV SI, 0200h
        0xBF, 0x00, 0x01,                   // MOV DI, 0100h
        0xB9, 0x05, 0x00,                   // MOV CX, 5
        0xF3, 0xA4,                         // REP MOVSB
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let subunit = harness::result_byte(&machine.bus, 0);
    let header_off = harness::result_word(&machine.bus, 1);
    let header_seg = harness::result_word(&machine.bus, 3);
    assert_eq!(subunit, 0, "Subunit should be 0, got {subunit}");
    assert_eq!(
        header_off, 0x007E,
        "Device header offset should be DEV_CDROM_OFFSET, got {header_off:#06X}"
    );
    assert_eq!(
        header_seg, 0x0200,
        "Device header segment should be DOS_DATA_SEGMENT, got {header_seg:#06X}"
    );
}

#[test]
fn mscdex_get_copyright_filename() {
    let mut machine = harness::boot_hle_with_cdrom();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x02,                   // MOV BX, 0200h
        0xB9, 0x10, 0x00,                   // MOV CX, 16 (Q:)
        0xB8, 0x02, 0x15,                   // MOV AX, 1502h
        0xCD, 0x2F,                         // INT 2Fh
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX (flags)
        0xBE, 0x00, 0x02,                   // MOV SI, 0200h
        0xBF, 0x02, 0x01,                   // MOV DI, 0102h
        0xB9, 0x26, 0x00,                   // MOV CX, 38
        0xF3, 0xA4,                         // REP MOVSB
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let flags = harness::result_word(&machine.bus, 0);
    assert_eq!(
        flags & 0x0001,
        0,
        "CF should be clear on success, flags={flags:#06X}"
    );
    let expected = b"COPYRIGHT.TXT;1";
    for (i, &expected_byte) in expected.iter().enumerate() {
        let actual = harness::result_byte(&machine.bus, 2 + i as u32);
        assert_eq!(
            actual, expected_byte,
            "Copyright byte {i}: expected {:#04X}, got {actual:#04X}",
            expected_byte
        );
    }
    let null_term = harness::result_byte(&machine.bus, 2 + 37);
    assert_eq!(null_term, 0, "Null terminator at byte 37, got {null_term}");
}

#[test]
fn mscdex_get_abstract_filename() {
    let mut machine = harness::boot_hle_with_cdrom();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x02,                   // MOV BX, 0200h
        0xB9, 0x10, 0x00,                   // MOV CX, 16 (Q:)
        0xB8, 0x03, 0x15,                   // MOV AX, 1503h
        0xCD, 0x2F,                         // INT 2Fh
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX (flags)
        0xBE, 0x00, 0x02,                   // MOV SI, 0200h
        0xBF, 0x02, 0x01,                   // MOV DI, 0102h
        0xB9, 0x26, 0x00,                   // MOV CX, 38
        0xF3, 0xA4,                         // REP MOVSB
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let flags = harness::result_word(&machine.bus, 0);
    assert_eq!(
        flags & 0x0001,
        0,
        "CF should be clear on success, flags={flags:#06X}"
    );
    let expected = b"ABSTRACT.TXT;1";
    for (i, &expected_byte) in expected.iter().enumerate() {
        let actual = harness::result_byte(&machine.bus, 2 + i as u32);
        assert_eq!(
            actual, expected_byte,
            "Abstract byte {i}: expected {:#04X}, got {actual:#04X}",
            expected_byte
        );
    }
}

#[test]
fn mscdex_get_bibliographic_filename() {
    let mut machine = harness::boot_hle_with_cdrom();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x02,                   // MOV BX, 0200h
        0xB9, 0x10, 0x00,                   // MOV CX, 16 (Q:)
        0xB8, 0x04, 0x15,                   // MOV AX, 1504h
        0xCD, 0x2F,                         // INT 2Fh
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX (flags)
        0xBE, 0x00, 0x02,                   // MOV SI, 0200h
        0xBF, 0x02, 0x01,                   // MOV DI, 0102h
        0xB9, 0x26, 0x00,                   // MOV CX, 38
        0xF3, 0xA4,                         // REP MOVSB
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let flags = harness::result_word(&machine.bus, 0);
    assert_eq!(
        flags & 0x0001,
        0,
        "CF should be clear on success, flags={flags:#06X}"
    );
    let expected = b"BIBLIO.TXT;1";
    for (i, &expected_byte) in expected.iter().enumerate() {
        let actual = harness::result_byte(&machine.bus, 2 + i as u32);
        assert_eq!(
            actual, expected_byte,
            "Bibliographic byte {i}: expected {:#04X}, got {actual:#04X}",
            expected_byte
        );
    }
}

#[test]
fn mscdex_get_copyright_invalid_drive() {
    let mut machine = harness::boot_hle_with_cdrom();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x02,                   // MOV BX, 0200h
        0xB9, 0x00, 0x00,                   // MOV CX, 0 (A: -- invalid)
        0xB8, 0x02, 0x15,                   // MOV AX, 1502h
        0xCD, 0x2F,                         // INT 2Fh
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX (error code)
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0102h], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let ax = harness::result_word(&machine.bus, 0);
    let flags = harness::result_word(&machine.bus, 2);
    assert_ne!(
        flags & 0x0001,
        0,
        "CF should be set on invalid drive, flags={flags:#06X}"
    );
    assert_eq!(ax, 15, "AX should be 15 (invalid drive), got {ax}");
}

#[test]
fn mscdex_read_vtoc_pvd() {
    let mut machine = harness::boot_hle_with_cdrom();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x02,                   // MOV BX, 0200h
        0xB9, 0x10, 0x00,                   // MOV CX, 16 (Q:)
        0xBA, 0x00, 0x00,                   // MOV DX, 0 (sector index 0 = LBA 16)
        0xB8, 0x05, 0x15,                   // MOV AX, 1505h
        0xCD, 0x2F,                         // INT 2Fh
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX (VD type)
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0102h], AX (flags)
        // Copy first 6 bytes of buffer to result area.
        0xBE, 0x00, 0x02,                   // MOV SI, 0200h
        0xBF, 0x04, 0x01,                   // MOV DI, 0104h
        0xB9, 0x06, 0x00,                   // MOV CX, 6
        0xF3, 0xA4,                         // REP MOVSB
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let ax = harness::result_word(&machine.bus, 0);
    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(ax, 1, "AX should be 1 (standard VD), got {ax}");
    assert_eq!(
        flags & 0x0001,
        0,
        "CF should be clear on success, flags={flags:#06X}"
    );
    let vd_type = harness::result_byte(&machine.bus, 4);
    assert_eq!(vd_type, 1, "VD type byte should be 1 (PVD), got {vd_type}");
    let expected_id = b"CD001";
    for (i, &expected_byte) in expected_id.iter().enumerate() {
        let actual = harness::result_byte(&machine.bus, 5 + i as u32);
        assert_eq!(
            actual, expected_byte,
            "CD001 byte {i}: expected {:#04X}, got {actual:#04X}",
            expected_byte
        );
    }
}

#[test]
fn mscdex_read_vtoc_terminator() {
    let mut machine = harness::boot_hle_with_cdrom();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x02,                   // MOV BX, 0200h
        0xB9, 0x10, 0x00,                   // MOV CX, 16 (Q:)
        0xBA, 0x01, 0x00,                   // MOV DX, 1 (sector index 1 = LBA 17)
        0xB8, 0x05, 0x15,                   // MOV AX, 1505h
        0xCD, 0x2F,                         // INT 2Fh
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0102h], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let ax = harness::result_word(&machine.bus, 0);
    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(ax, 0x00FF, "AX should be 00FFh (terminator), got {ax:#06X}");
    assert_eq!(
        flags & 0x0001,
        0,
        "CF should be clear on success, flags={flags:#06X}"
    );
}

#[test]
fn mscdex_absolute_disk_read() {
    let mut machine = harness::boot_hle_with_cdrom();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x02,                   // MOV BX, 0200h
        0xB9, 0x10, 0x00,                   // MOV CX, 16 (Q:)
        0xBA, 0x01, 0x00,                   // MOV DX, 1 (1 sector)
        0x31, 0xF6,                         // XOR SI, SI (start LBA high = 0)
        0x31, 0xFF,                         // XOR DI, DI (start LBA low = 0)
        0xB8, 0x08, 0x15,                   // MOV AX, 1508h
        0xCD, 0x2F,                         // INT 2Fh
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX (flags)
        0xA0, 0x00, 0x02,                   // MOV AL, [0200h]
        0xA2, 0x02, 0x01,                   // MOV [0102h], AL (first data byte)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let flags = harness::result_word(&machine.bus, 0);
    assert_eq!(
        flags & 0x0001,
        0,
        "CF should be clear on success, flags={flags:#06X}"
    );
    let first_byte = harness::result_byte(&machine.bus, 2);
    assert_eq!(
        first_byte, 0x11,
        "First data byte should be 0x11 (sector 0 fill), got {first_byte:#04X}"
    );
}

#[test]
fn mscdex_absolute_disk_read_multi() {
    let mut machine = harness::boot_hle_with_cdrom();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x02,                   // MOV BX, 0200h
        0xB9, 0x10, 0x00,                   // MOV CX, 16 (Q:)
        0xBA, 0x02, 0x00,                   // MOV DX, 2 (2 sectors)
        0x31, 0xF6,                         // XOR SI, SI
        0x31, 0xFF,                         // XOR DI, DI
        0xB8, 0x08, 0x15,                   // MOV AX, 1508h
        0xCD, 0x2F,                         // INT 2Fh
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX (flags)
        // First byte of sector 0.
        0xA0, 0x00, 0x02,                   // MOV AL, [0200h]
        0xA2, 0x02, 0x01,                   // MOV [0102h], AL
        // First byte of sector 1 (at buffer + 2048 = 0200h + 0800h = 0A00h).
        0xA0, 0x00, 0x0A,                   // MOV AL, [0A00h]
        0xA2, 0x03, 0x01,                   // MOV [0103h], AL
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let flags = harness::result_word(&machine.bus, 0);
    assert_eq!(
        flags & 0x0001,
        0,
        "CF should be clear on success, flags={flags:#06X}"
    );
    let byte_sector0 = harness::result_byte(&machine.bus, 2);
    let byte_sector1 = harness::result_byte(&machine.bus, 3);
    assert_eq!(
        byte_sector0, 0x11,
        "Sector 0 first byte should be 0x11, got {byte_sector0:#04X}"
    );
    assert_eq!(
        byte_sector1, 0x11,
        "Sector 1 first byte should be 0x11, got {byte_sector1:#04X}"
    );
}

#[test]
fn mscdex_absolute_disk_read_invalid_drive() {
    let mut machine = harness::boot_hle_with_cdrom();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, 0x00, 0x02,                   // MOV BX, 0200h
        0xB9, 0x00, 0x00,                   // MOV CX, 0 (A: -- invalid)
        0xBA, 0x01, 0x00,                   // MOV DX, 1
        0x31, 0xF6,                         // XOR SI, SI
        0x31, 0xFF,                         // XOR DI, DI
        0xB8, 0x08, 0x15,                   // MOV AX, 1508h
        0xCD, 0x2F,                         // INT 2Fh
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX (error code)
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0102h], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let ax = harness::result_word(&machine.bus, 0);
    let flags = harness::result_word(&machine.bus, 2);
    assert_ne!(
        flags & 0x0001,
        0,
        "CF should be set on invalid drive, flags={flags:#06X}"
    );
    assert_eq!(ax, 15, "AX should be 15 (invalid drive), got {ax}");
}
