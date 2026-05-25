use crate::harness;

#[test]
fn default_files_count() {
    let mut machine = harness::boot_hle();
    let sysvars = harness::get_sysvars_address(&mut machine);

    // Walk the SFT chain from SYSVARS+0x04 and count total file entries.
    let (seg, off) = harness::read_far_ptr(&machine.bus, sysvars + 0x04);
    let mut sft_addr = harness::far_to_linear(seg, off);
    let mut total_entries = 0u32;

    for _ in 0..20 {
        if sft_addr == 0 || sft_addr >= 0xA0000 {
            break;
        }

        // SFT header: 4 bytes next pointer, 2 bytes entry count.
        let entry_count = harness::read_word(&machine.bus, sft_addr + 4);
        total_entries += entry_count as u32;

        let (next_seg, next_off) = harness::read_far_ptr(&machine.bus, sft_addr);
        if next_seg == 0xFFFF && next_off == 0xFFFF {
            break;
        }
        sft_addr = harness::far_to_linear(next_seg, next_off);
    }

    assert!(
        total_entries >= 20,
        "Total SFT entries should be >= 20 (default FILES=20), got {}",
        total_entries
    );
}

#[test]
fn default_buffers() {
    let mut machine = harness::boot_hle();
    let sysvars = harness::get_sysvars_address(&mut machine);
    let buffers = harness::read_word(&machine.bus, sysvars + 0x3F);
    assert_eq!(buffers, 15, "Default BUFFERS should be 15, got {}", buffers);
}

#[test]
fn default_lastdrive() {
    let mut machine = harness::boot_hle();
    let sysvars = harness::get_sysvars_address(&mut machine);
    let lastdrive = harness::read_byte(&machine.bus, sysvars + 0x21);
    assert_eq!(
        lastdrive, 26,
        "Default LASTDRIVE should be 26 (Z), got {}",
        lastdrive
    );
}

#[test]
fn default_break_off() {
    let mut machine = harness::boot_hle();

    // INT 21h AH=33h AL=00h: Get BREAK flag
    const RES_LO: u8 = (harness::INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (harness::INJECT_RESULT_OFFSET >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x33,                         // MOV AH, 33h
        0xB0, 0x00,                         // MOV AL, 00h (get)
        0xCD, 0x21,                         // INT 21h
        0x88, 0x16, RES_LO, RES_HI,         // MOV [result], DL
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let break_flag = harness::result_byte(&machine.bus, 0);
    assert_eq!(
        break_flag, 0,
        "Default BREAK should be OFF (0), got {}",
        break_flag
    );
}

#[test]
fn sft_chain_has_two_blocks() {
    let mut machine = harness::boot_hle();
    let sysvars = harness::get_sysvars_address(&mut machine);

    let (seg, off) = harness::read_far_ptr(&machine.bus, sysvars + 0x04);
    let sft1_addr = harness::far_to_linear(seg, off);
    let sft1_count = harness::read_word(&machine.bus, sft1_addr + 4);
    assert_eq!(sft1_count, 5, "First SFT block should have 5 entries");

    // Follow chain to second block
    let (seg2, off2) = harness::read_far_ptr(&machine.bus, sft1_addr);
    assert!(
        !(seg2 == 0xFFFF && off2 == 0xFFFF),
        "SFT chain should have a second block"
    );
    let sft2_addr = harness::far_to_linear(seg2, off2);
    let sft2_count = harness::read_word(&machine.bus, sft2_addr + 4);
    assert!(
        sft2_count >= 15,
        "Second SFT block should have >= 15 entries (FILES=20 default), got {}",
        sft2_count
    );

    // Total should be >= 20
    assert!(
        sft1_count + sft2_count >= 20,
        "Total SFT entries should be >= 20, got {}",
        sft1_count + sft2_count
    );
}

#[test]
fn shell_exec_com_program() {
    let mut machine = harness::boot_hle_with_floppy();

    // Type "A:\TEST.COM" and press Enter. TEST.COM exits with code 0x42 (66).
    harness::type_string_long(&mut machine, b"A:\\TEST.COM\r");
    harness::run_until_prompt(&mut machine);

    // The shell should return to prompt after the child terminates.
    // Verify by typing another command.
    harness::type_string(&mut machine.bus, b"VER\r");
    harness::run_until_prompt(&mut machine);

    // "6.20" should be in VRAM
    let version = [0x0036, 0x002E, 0x0032, 0x0030]; // "6.20"
    assert!(
        harness::find_string_in_text_vram(&machine.bus, &version),
        "VER should work after external program execution"
    );
}

#[test]
fn shell_exec_errorlevel() {
    let mut machine = harness::boot_hle_with_floppy();

    // Run TEST.COM (exits with code 0x42 = 66), then check ERRORLEVEL via batch IF.
    // We use a one-liner: first run the program, then check errorlevel.
    harness::type_string_long(&mut machine, b"A:\\TEST.COM\r");
    harness::run_until_prompt(&mut machine);

    // Now check errorlevel using IF
    harness::type_string_long(&mut machine, b"IF ERRORLEVEL 66 ECHO EXITOK\r");
    harness::run_until_prompt(&mut machine);

    let exitok = [0x0045, 0x0058, 0x0049, 0x0054, 0x004F, 0x004B]; // "EXITOK"
    assert!(
        harness::find_string_in_text_vram(&machine.bus, &exitok),
        "IF ERRORLEVEL 66 should match after TEST.COM (exit code 0x42)"
    );
}

#[test]
fn shell_bad_command() {
    let mut machine = harness::boot_hle();

    harness::type_string_long(&mut machine, b"NONEXISTENT\r");
    harness::run_until_prompt(&mut machine);

    let bad = [0x0042, 0x0061, 0x0064]; // "Bad"
    assert!(
        harness::find_string_in_text_vram(&machine.bus, &bad),
        "Unknown command should print 'Bad command or file name'"
    );
}

#[test]
fn boot_banner_displayed() {
    let machine = harness::boot_hle();
    let neetan = [0x004E, 0x0065, 0x0065, 0x0074, 0x0061, 0x006E]; // "Neetan"
    assert!(
        harness::find_string_in_text_vram(&machine.bus, &neetan),
        "Boot banner should be displayed"
    );
    let version = [0x0036, 0x002E, 0x0032, 0x0030]; // "6.20"
    assert!(
        harness::find_string_in_text_vram(&machine.bus, &version),
        "Version 6.20 should be in boot banner"
    );
}

#[test]
fn prompt_shows_drive_letter() {
    let machine = harness::boot_hle();
    // Default prompt is $P$G which shows "Z:\>" (boot drive is Z:)
    let prompt = [0x005A, 0x003A, 0x005C, 0x003E]; // "Z:\>"
    assert!(
        harness::find_string_in_text_vram(&machine.bus, &prompt),
        "Default prompt should show 'Z:\\>'"
    );
}
