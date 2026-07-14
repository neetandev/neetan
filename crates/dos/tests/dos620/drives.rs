use common::Bus;

use crate::harness;

const IOSYS_BASE: u32 = 0x0600;
const BDA_BOOT_DEVICE: u32 = 0x0584;
const AUTOEXEC_LINES: &[u8] = b"@ECHO OFF\r\n";

fn assert_boot_and_current_drive_are_a(machine: &mut machine_98::Pc9801Ra) {
    let sysvars = harness::get_sysvars_address(machine);
    let boot_drive = harness::read_byte(&machine.bus, sysvars + 0x43);
    assert_eq!(
        boot_drive, 1,
        "Boot drive should be 1 (A:), got {}",
        boot_drive
    );

    let code: &[u8] = &[
        0xB4, 0x19, // MOV AH, 19h
        0xCD, 0x21, // INT 21h
        0xA2, 0x00, 0x01, // MOV [0100h], AL
        0xFA, // CLI
        0xF4, // HLT
    ];
    harness::inject_and_run(machine, code);
    let current_drive = harness::result_byte(&machine.bus, 0);
    assert_eq!(
        current_drive, 0,
        "Current drive should be 0 (A:), got {}",
        current_drive
    );
}

#[test]
fn hdd_gets_lower_drive_letters_when_present() {
    let mut machine = harness::create_hle_machine();

    // Set BDA DISK_EQUIP: bit 0 = 1MB FDD unit 0, bit 8 = HDD unit 0.
    machine.bus.write_byte(0x055C, 0x01);
    machine.bus.write_byte(0x055D, 0x01);

    let mut total_cycles = 0u64;
    loop {
        total_cycles += machine.run_for(1_000_000);
        if harness::hle_prompt_visible(&machine.bus) {
            break;
        }
        assert!(total_cycles < 500_000_000, "HLE DOS did not show prompt");
    }

    // PC-98 convention: HDD gets A:, FDD skips B: and starts at C:.
    let drive_a_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C);
    assert_eq!(
        drive_a_daua, 0x80,
        "Drive A: should be HDD (0x80) when HDDs are present, got {:#04X}",
        drive_a_daua
    );

    let drive_b_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C + 1);
    assert_eq!(
        drive_b_daua, 0x00,
        "Drive B: should be empty (0x00) -- reserved gap, got {:#04X}",
        drive_b_daua
    );

    let drive_c_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C + 2);
    assert_eq!(
        drive_c_daua, 0x90,
        "Drive C: should be FDD (0x90) -- secondary type starts at C:, got {:#04X}",
        drive_c_daua
    );
}

#[test]
fn floppy_gets_lower_drive_letters_without_hdd() {
    let mut machine = harness::create_hle_machine();

    // Set BDA DISK_EQUIP: bit 0 = 1MB FDD unit 0, no HDDs.
    machine.bus.write_byte(0x055C, 0x01);
    machine.bus.write_byte(0x055D, 0x00);

    let mut total_cycles = 0u64;
    loop {
        total_cycles += machine.run_for(1_000_000);
        if harness::hle_prompt_visible(&machine.bus) {
            break;
        }
        assert!(total_cycles < 500_000_000, "HLE DOS did not show prompt");
    }

    // No HDDs: FDD gets A:.
    let drive_a_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C);
    assert_eq!(
        drive_a_daua, 0x90,
        "Drive A: should be FDD (0x90) when no HDDs exist, got {:#04X}",
        drive_a_daua
    );
}

#[test]
fn fdd_autoexec_takes_drive_a_when_hdd_is_also_present() {
    let floppy = harness::create_test_floppy_with_autoexec(AUTOEXEC_LINES);
    let hdd = harness::create_test_hdd_with_autoexec(256, AUTOEXEC_LINES);
    let mut machine = harness::boot_hle_with_forced_os(Some(floppy), Some(hdd));

    let drive_a_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C);
    assert_eq!(
        drive_a_daua, 0x90,
        "Drive A: should be FDD (0x90) when floppy AUTOEXEC.BAT is present, got {:#04X}",
        drive_a_daua
    );

    let drive_b_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C + 1);
    assert_eq!(
        drive_b_daua, 0x00,
        "Drive B: should be reserved when only one floppy is present, got {:#04X}",
        drive_b_daua
    );

    let drive_c_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C + 2);
    assert_eq!(
        drive_c_daua, 0x80,
        "Drive C: should be HDD (0x80) when floppy AUTOEXEC.BAT takes priority, got {:#04X}",
        drive_c_daua
    );

    assert_boot_and_current_drive_are_a(&mut machine);

    let boot_device = harness::read_byte(&machine.bus, BDA_BOOT_DEVICE);
    assert_eq!(
        boot_device, 0x90,
        "BDA boot device should report 2HD floppy boot (0x90), got {:#04X}",
        boot_device
    );
}

#[test]
fn hdd_autoexec_takes_drive_a_when_floppy_has_no_autoexec() {
    let floppy = harness::create_test_floppy();
    let hdd = harness::create_test_hdd_with_autoexec(256, AUTOEXEC_LINES);
    let mut machine = harness::boot_hle_with_forced_os(Some(floppy), Some(hdd));

    let drive_a_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C);
    assert_eq!(
        drive_a_daua, 0x80,
        "Drive A: should be HDD (0x80) when only HDD has AUTOEXEC.BAT, got {:#04X}",
        drive_a_daua
    );

    let boot_device = harness::read_byte(&machine.bus, BDA_BOOT_DEVICE);
    assert_eq!(
        boot_device, 0x80,
        "BDA boot device should report HDD boot (0x80), got {:#04X}",
        boot_device
    );

    let drive_b_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C + 1);
    assert_eq!(
        drive_b_daua, 0x00,
        "Drive B: should be reserved when only one HDD is present, got {:#04X}",
        drive_b_daua
    );

    let drive_c_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C + 2);
    assert_eq!(
        drive_c_daua, 0x90,
        "Drive C: should be FDD (0x90) when HDD AUTOEXEC.BAT takes priority, got {:#04X}",
        drive_c_daua
    );

    assert_boot_and_current_drive_are_a(&mut machine);
}

#[test]
fn hle_boot_sets_bda_boot_device_from_selected_physical_medium() {
    let floppy = harness::create_test_floppy_with_autoexec(AUTOEXEC_LINES);
    let hdd = harness::create_test_hdd_with_autoexec(256, AUTOEXEC_LINES);
    let machine = harness::boot_hle_with_forced_os(Some(floppy), Some(hdd));
    assert_eq!(
        harness::read_byte(&machine.bus, BDA_BOOT_DEVICE),
        0x90,
        "BDA boot device should report the selected floppy DA/UA when floppy AUTOEXEC.BAT wins"
    );

    let floppy = harness::create_test_floppy();
    let hdd = harness::create_test_hdd_with_autoexec(256, AUTOEXEC_LINES);
    let machine = harness::boot_hle_with_forced_os(Some(floppy), Some(hdd));
    assert_eq!(
        harness::read_byte(&machine.bus, BDA_BOOT_DEVICE),
        0x80,
        "BDA boot device should report the selected HDD DA/UA when HDD AUTOEXEC.BAT wins"
    );
}

#[test]
fn hdd_stays_drive_a_when_no_autoexec_exists() {
    let floppy = harness::create_test_floppy();
    let hdd = harness::create_test_hdd(256);
    let mut machine = harness::boot_hle_with_forced_os(Some(floppy), Some(hdd));

    let drive_a_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C);
    assert_eq!(
        drive_a_daua, 0x80,
        "Drive A: should default to HDD (0x80) when no AUTOEXEC.BAT exists, got {:#04X}",
        drive_a_daua
    );

    let drive_c_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C + 2);
    assert_eq!(
        drive_c_daua, 0x90,
        "Drive C: should be FDD (0x90) when HDD defaults to primary, got {:#04X}",
        drive_c_daua
    );

    assert_boot_and_current_drive_are_a(&mut machine);
}

#[test]
fn fdd_stays_drive_a_when_only_floppy_is_present_without_autoexec() {
    let floppy = harness::create_test_floppy();
    let mut machine = harness::boot_hle_with_forced_os(Some(floppy), None);

    let drive_a_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C);
    assert_eq!(
        drive_a_daua, 0x90,
        "Drive A: should be FDD (0x90) when only floppy media is present, got {:#04X}",
        drive_a_daua
    );

    assert_boot_and_current_drive_are_a(&mut machine);
}

#[test]
fn fdd_stays_drive_a_when_only_floppy_is_present_with_autoexec() {
    let floppy = harness::create_test_floppy_with_autoexec(AUTOEXEC_LINES);
    let mut machine = harness::boot_hle_with_forced_os(Some(floppy), None);

    let drive_a_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C);
    assert_eq!(
        drive_a_daua, 0x90,
        "Drive A: should be FDD (0x90) when only floppy AUTOEXEC.BAT is present, got {:#04X}",
        drive_a_daua
    );

    assert_boot_and_current_drive_are_a(&mut machine);
}

#[test]
fn daua_floppy_assignment() {
    let machine = harness::boot_hle();
    // DA/UA table at 0060:006Ch, 16 bytes for A:-P:.
    // HLE boots with 2 built-in 1MB FDD drives: A:=0x90, B:=0x91.
    let first_drive_daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C);
    assert_eq!(
        first_drive_daua, 0x90,
        "Boot drive A: DA/UA should be 0x90 (1MB FDD unit 0), got {:#04X}",
        first_drive_daua
    );
}

#[test]
fn dpb_chain_matches_drives() {
    let mut machine = harness::boot_hle();
    let sysvars = harness::get_sysvars_address(&mut machine);
    let (seg, off) = harness::read_far_ptr(&machine.bus, sysvars);
    let mut dpb_addr = harness::far_to_linear(seg, off);

    let mut drive_numbers = Vec::new();
    for _ in 0..26 {
        if dpb_addr == 0 || dpb_addr >= 0xA0000 {
            break;
        }

        let drive_num = harness::read_byte(&machine.bus, dpb_addr);
        drive_numbers.push(drive_num);

        // Next DPB pointer is at offset +0x19 in the DPB (DOS 4.0+ format).
        // For DOS 3.x it might be at a different offset. Try +0x19 first.
        let (next_seg, next_off) = harness::read_far_ptr(&machine.bus, dpb_addr + 0x19);
        if next_seg == 0xFFFF && next_off == 0xFFFF {
            break;
        }
        dpb_addr = harness::far_to_linear(next_seg, next_off);
    }

    assert!(
        !drive_numbers.is_empty(),
        "DPB chain should contain at least one entry"
    );

    for &drive in &drive_numbers {
        assert!(drive < 26, "DPB drive number should be < 26, got {}", drive);
    }
}

#[test]
fn boot_drive_is_first_drive_when_media_present() {
    let mut machine = harness::create_hle_machine();

    // Set BDA DISK_EQUIP: bit 8 = HDD unit 0.
    machine.bus.write_byte(0x055C, 0x00);
    machine.bus.write_byte(0x055D, 0x01);

    // Insert HDD with media BEFORE boot so sector_size() returns Some.
    let hdd = harness::create_empty_hdd(256);
    machine.bus.insert_hdd(0, hdd, None);

    let mut total_cycles = 0u64;
    loop {
        total_cycles += machine.run_for(1_000_000);
        if harness::hle_prompt_visible(&machine.bus) {
            break;
        }
        assert!(total_cycles < 500_000_000, "HLE DOS did not show prompt");
    }

    // Boot drive should be 1 (A:, 1-based) since HDD has media.
    let sysvars = harness::get_sysvars_address(&mut machine);
    let boot_drive = harness::read_byte(&machine.bus, sysvars + 0x43);
    assert_eq!(
        boot_drive, 1,
        "Boot drive should be 1 (A:) when HDD has media, got {}",
        boot_drive
    );

    // Current drive (INT 21h AH=19h) should be 0 (A:, 0-based).
    let code: &[u8] = &[
        0xB4, 0x19, // MOV AH, 19h
        0xCD, 0x21, // INT 21h
        0xA2, 0x00, 0x01, // MOV [0100h], AL
        0xFA, // CLI
        0xF4, // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let current_drive = harness::result_byte(&machine.bus, 0);
    assert_eq!(
        current_drive, 0,
        "Current drive should be 0 (A:) when HDD has media, got {}",
        current_drive
    );
}

#[test]
fn boot_drive_is_virtual_when_floppy_has_no_media() {
    let mut machine = harness::create_hle_machine();

    // Set BDA DISK_EQUIP: bit 0 = 1MB FDD unit 0, no HDDs.
    machine.bus.write_byte(0x055C, 0x01);
    machine.bus.write_byte(0x055D, 0x00);

    // Do NOT insert floppy media -- drive exists but has no disk.
    let mut total_cycles = 0u64;
    loop {
        total_cycles += machine.run_for(1_000_000);
        if harness::hle_prompt_visible(&machine.bus) {
            break;
        }
        assert!(total_cycles < 500_000_000, "HLE DOS did not show prompt");
    }

    // Boot drive should be 26 (Z:, 1-based) since floppy has no media.
    let sysvars = harness::get_sysvars_address(&mut machine);
    let boot_drive = harness::read_byte(&machine.bus, sysvars + 0x43);
    assert_eq!(
        boot_drive, 26,
        "Boot drive should be 26 (Z:) when floppy has no media, got {}",
        boot_drive
    );
}

#[test]
fn sysvars_max_sector_size() {
    let mut machine = harness::boot_hle();
    let sysvars = harness::get_sysvars_address(&mut machine);
    let max_sector = harness::read_word(&machine.bus, sysvars + 0x10);
    // HDD uses 512-byte sectors, so max should be at least 512.
    assert!(
        max_sector >= 512,
        "SYSVARS max bytes per sector should be >= 512, got {}",
        max_sector
    );
}
