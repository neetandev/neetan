use crate::harness;

/// Fixed test time: 2026-03-15 14:30:45, Sunday (dow=0).
/// BCD format: [year, month<<4|dow, day, hour, minute, second]
fn test_time() -> [u8; 6] {
    [0x26, 0x30, 0x15, 0x14, 0x30, 0x45]
}

fn boot_hle_with_fixed_time() -> machine::Pc9801Ra {
    harness::boot_hle_with_time(Some(test_time))
}

#[test]
fn get_date_returns_host_time() {
    let mut machine = boot_hle_with_fixed_time();

    // AH=2Ah: Get Date
    // Returns: CX=year, DH=month, DL=day, AL=day-of-week
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x2A,                         // MOV AH, 2Ah
        0xCD, 0x21,                         // INT 21h
        0x89, 0x0E, 0x00, 0x01,             // MOV [0x0100], CX (year)
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX (DH=month, DL=day)
        0xA2, 0x04, 0x01,                   // MOV [0x0104], AL (day-of-week)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let year = harness::result_word(&machine.bus, 0);
    let dx = harness::result_word(&machine.bus, 2);
    let month = (dx >> 8) as u8;
    let day = (dx & 0xFF) as u8;
    let dow = harness::result_byte(&machine.bus, 4);

    assert_eq!(year, 2026, "year should be 2026, got {}", year);
    assert_eq!(month, 3, "month should be 3, got {}", month);
    assert_eq!(day, 15, "day should be 15, got {}", day);
    assert_eq!(dow, 0, "day-of-week should be 0 (Sunday), got {}", dow);
}

#[test]
fn get_time_returns_host_time() {
    let mut machine = boot_hle_with_fixed_time();

    // AH=2Ch: Get Time
    // Returns: CH=hour, CL=minute, DH=second, DL=hundredths
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x2C,                         // MOV AH, 2Ch
        0xCD, 0x21,                         // INT 21h
        0x89, 0x0E, 0x00, 0x01,             // MOV [0x0100], CX (CH=hour, CL=minute)
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX (DH=second, DL=hundredths)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let cx = harness::result_word(&machine.bus, 0);
    let dx = harness::result_word(&machine.bus, 2);
    let hour = (cx >> 8) as u8;
    let minute = (cx & 0xFF) as u8;
    let second = (dx >> 8) as u8;
    let hundredths = (dx & 0xFF) as u8;

    assert_eq!(hour, 14, "hour should be 14, got {}", hour);
    assert_eq!(minute, 30, "minute should be 30, got {}", minute);
    assert_eq!(second, 45, "second should be 45, got {}", second);
    assert_eq!(hundredths, 0, "hundredths should be 0, got {}", hundredths);
}

#[test]
fn set_date_returns_success() {
    let mut machine = boot_hle_with_fixed_time();

    // AH=2Bh: Set Date (no-op, should return AL=0)
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x2B,                         // MOV AH, 2Bh
        0xB9, 0xE8, 0x07,                   // MOV CX, 2024
        0xBA, 0x06, 0x0F,                   // MOV DX, 0x060F (DH=6, DL=15)
        0xCD, 0x21,                         // INT 21h
        0xA2, 0x00, 0x01,                   // MOV [0x0100], AL
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let result = harness::result_byte(&machine.bus, 0);
    assert_eq!(
        result, 0,
        "Set Date should return AL=0 (success), got {}",
        result
    );
}

#[test]
fn set_time_returns_success() {
    let mut machine = boot_hle_with_fixed_time();

    // AH=2Dh: Set Time (no-op, should return AL=0)
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x2D,                         // MOV AH, 2Dh
        0xB9, 0x0A, 0x1E,                   // MOV CX, 0x1E0A (CH=30, CL=10)
        0xBA, 0x00, 0x00,                   // MOV DX, 0x0000
        0xCD, 0x21,                         // INT 21h
        0xA2, 0x00, 0x01,                   // MOV [0x0100], AL
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let result = harness::result_byte(&machine.bus, 0);
    assert_eq!(
        result, 0,
        "Set Time should return AL=0 (success), got {}",
        result
    );
}

#[test]
fn get_date_after_set_still_returns_host_time() {
    let mut machine = boot_hle_with_fixed_time();

    // Call Set Date, then Get Date - should still return host time (set is no-op)
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Set Date to 2024-06-15
        0xB4, 0x2B,                         // MOV AH, 2Bh
        0xB9, 0xE8, 0x07,                   // MOV CX, 2024
        0xBA, 0x06, 0x0F,                   // MOV DX, 0x060F
        0xCD, 0x21,                         // INT 21h
        // Get Date
        0xB4, 0x2A,                         // MOV AH, 2Ah
        0xCD, 0x21,                         // INT 21h
        0x89, 0x0E, 0x00, 0x01,             // MOV [0x0100], CX (year)
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let year = harness::result_word(&machine.bus, 0);
    let dx = harness::result_word(&machine.bus, 2);
    let month = (dx >> 8) as u8;
    let day = (dx & 0xFF) as u8;

    assert_eq!(
        year, 2026,
        "year should still be 2026 after set, got {}",
        year
    );
    assert_eq!(month, 3, "month should still be 3, got {}", month);
    assert_eq!(day, 15, "day should still be 15, got {}", day);
}
