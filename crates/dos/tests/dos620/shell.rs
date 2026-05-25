use crate::harness::*;

#[test]
fn shell_prompt_visible() {
    let machine = boot_hle();
    assert!(
        find_char_in_text_vram(&machine.bus, 0x003E),
        "prompt '>' should be visible in VRAM after boot"
    );
}

#[test]
fn shell_banner_displayed() {
    let machine = boot_hle();
    let neetan = [0x004E, 0x0065, 0x0065, 0x0074, 0x0061, 0x006E]; // "Neetan"
    assert!(
        find_string_in_text_vram(&machine.bus, &neetan),
        "boot banner 'Neetan' should be visible in VRAM"
    );
}

#[test]
fn shell_ver_command() {
    let mut machine = boot_hle();
    type_string(&mut machine.bus, b"VER\r");
    run_until_prompt(&mut machine);

    // Check for version string fragments in VRAM
    // "6.20" should appear
    let version = [0x0036, 0x002E, 0x0032, 0x0030]; // "6.20"
    assert!(
        find_string_in_text_vram(&machine.bus, &version),
        "VER command should display version '6.20'"
    );
}

#[test]
fn shell_cls_command() {
    let mut machine = boot_hle();
    // Before CLS, banner text should be present
    let neetan = [0x004E, 0x0065, 0x0065, 0x0074, 0x0061, 0x006E];
    assert!(find_string_in_text_vram(&machine.bus, &neetan));

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    // After CLS, the banner should be gone (screen was cleared)
    // But a new prompt should be present
    assert!(
        find_char_in_text_vram(&machine.bus, 0x003E),
        "prompt should reappear after CLS"
    );
    // The banner from boot should no longer be visible (screen was cleared,
    // only the new prompt remains)
    assert!(
        !find_string_in_text_vram(&machine.bus, &neetan),
        "boot banner should be gone after CLS"
    );
}

#[test]
fn shell_echo_command() {
    let mut machine = boot_hle();
    type_string(&mut machine.bus, b"ECHO HELLO\r");
    run_until_prompt(&mut machine);

    let hello = [0x0048, 0x0045, 0x004C, 0x004C, 0x004F]; // "HELLO"
    assert!(
        find_string_in_text_vram(&machine.bus, &hello),
        "ECHO HELLO should display 'HELLO' in VRAM"
    );
}

#[test]
fn shell_cd_command() {
    let mut machine = boot_hle();
    type_string(&mut machine.bus, b"CD\r");
    run_until_prompt(&mut machine);

    // Should display current path, which starts with the drive letter and backslash
    let backslash = [0x005C]; // "\"
    assert!(
        find_string_in_text_vram(&machine.bus, &backslash),
        "CD should display current path with backslash"
    );
}

#[test]
fn shell_set_command() {
    let mut machine = boot_hle();
    type_string(&mut machine.bus, b"SET\r");
    run_until_prompt(&mut machine);

    // Should display COMSPEC= from the environment
    let comspec = [0x0043, 0x004F, 0x004D, 0x0053, 0x0050, 0x0045, 0x0043]; // "COMSPEC"
    assert!(
        find_string_in_text_vram(&machine.bus, &comspec),
        "SET should display 'COMSPEC' from environment"
    );
}

#[test]
fn shell_bad_command() {
    let mut machine = boot_hle();
    type_string(&mut machine.bus, b"NOSUCHCMD\r");
    run_until_prompt(&mut machine);

    // Unknown commands should just return to prompt (no crash)
    assert!(
        find_char_in_text_vram(&machine.bus, 0x003E),
        "prompt should be visible after bad command"
    );
}

#[test]
fn shell_line_editing() {
    let mut machine = boot_hle();

    // Type "VEER", move left twice, delete the extra 'E', then press Enter.
    // This should produce "VER" which executes the version command.
    type_string(&mut machine.bus, b"VEER");
    type_special_key(&mut machine, SCAN_LEFT);
    type_special_key(&mut machine, SCAN_LEFT);
    type_special_key(&mut machine, SCAN_DELETE);
    type_string(&mut machine.bus, b"\r");
    run_until_prompt(&mut machine);

    let version = [0x0036, 0x002E, 0x0032, 0x0030]; // "6.20"
    assert!(
        find_string_in_text_vram(&machine.bus, &version),
        "line editing should produce working VER command"
    );
}

#[test]
fn shell_history() {
    let mut machine = boot_hle();

    // Execute VER command.
    type_string(&mut machine.bus, b"VER\r");
    run_until_prompt(&mut machine);

    // Clear screen so we can verify the recalled command produces fresh output.
    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    // Press up arrow twice (past CLS to VER), then Enter.
    type_special_key(&mut machine, SCAN_UP);
    type_special_key(&mut machine, SCAN_UP);
    type_string(&mut machine.bus, b"\r");
    run_until_prompt(&mut machine);

    let version = [0x0036, 0x002E, 0x0032, 0x0030]; // "6.20"
    assert!(
        find_string_in_text_vram(&machine.bus, &version),
        "history recall should re-execute VER command"
    );
}

#[test]
fn shell_switch_to_invalid_drive() {
    let mut machine = boot_hle();

    // Switching to E: should fail. boot_hle() configures built-in FDD
    // drives A: and B: plus virtual Z:, but no drive E: exists (no CDS entry).
    type_string(&mut machine.bus, b"E:\r");
    run_until_prompt(&mut machine);

    // "Invalid drive"
    let invalid_drive = [
        0x0049, 0x006E, 0x0076, 0x0061, 0x006C, 0x0069, 0x0064, 0x0020, 0x0064, 0x0072, 0x0069,
        0x0076, 0x0065,
    ];
    assert!(
        find_string_in_text_vram(&machine.bus, &invalid_drive),
        "switching to E: with no such drive should display 'Invalid drive'"
    );
}

#[test]
fn shell_switch_to_drive_no_media() {
    // boot_hle() configures FDD hardware (A: and B:) via BDA init, but no
    // floppy image is inserted. Switching to A: should fail because the
    // drive has no readable media.
    let mut machine = boot_hle();

    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    // "No media in drive A"
    let no_media = [
        0x004E, 0x006F, 0x0020, 0x006D, 0x0065, 0x0064, 0x0069, 0x0061, 0x0020, 0x0069, 0x006E,
        0x0020, 0x0064, 0x0072, 0x0069, 0x0076, 0x0065, 0x0020, 0x0041,
    ];
    assert!(
        find_string_in_text_vram(&machine.bus, &no_media),
        "switching to A: with no media should display 'No media in drive A'"
    );
}

#[test]
fn shell_switch_to_q_drive_with_cdrom() {
    let mut machine = boot_hle_with_cdrom();

    type_string(&mut machine.bus, b"Q:\r");
    run_until_prompt_ap(&mut machine);

    assert!(
        find_row_containing(&machine.bus, "No media in drive Q").is_none(),
        "Q: should not report missing media for the MSCDEX CD-ROM drive"
    );

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x19,                         // MOV AH, 19h
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run_generic_with_budget(&mut machine, code, INJECT_BUDGET);

    let current_drive = result_byte(&machine.bus, 0);
    assert_eq!(
        current_drive, 16,
        "Q: command should switch the current drive to Q:, got index {}",
        current_drive
    );
}

#[test]
fn shell_switch_to_q_drive_without_media() {
    let mut machine = boot_hle_with_cdrom();
    machine.bus.eject_cdrom();

    type_string(&mut machine.bus, b"Q:\r");
    run_until_prompt_ap(&mut machine);

    assert!(
        find_row_containing(&machine.bus, "No media in drive Q").is_some(),
        "Q: should report missing media after the CD-ROM is ejected"
    );
}

#[test]
fn shell_cd_nonexistent_directory() {
    let mut machine = boot_hle_with_floppy();

    // Switch to A: (which has a valid floppy)
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    // CD to a directory that does not exist on the floppy
    type_string(&mut machine.bus, b"CD YUNO\r");
    run_until_prompt(&mut machine);

    // "Invalid directory"
    let inv_dir = [
        0x0049, 0x006E, 0x0076, 0x0061, 0x006C, 0x0069, 0x0064, 0x0020, 0x0064, 0x0069, 0x0072,
        0x0065, 0x0063, 0x0074, 0x006F, 0x0072, 0x0079,
    ];
    assert!(
        find_string_in_text_vram(&machine.bus, &inv_dir),
        "CD YUNO should display 'Invalid directory' error"
    );

    // Clear screen and try again to verify the error is consistent
    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CD YUNO\r");
    run_until_prompt(&mut machine);

    assert!(
        find_string_in_text_vram(&machine.bus, &inv_dir),
        "second CD YUNO should also display 'Invalid directory' error"
    );
}
