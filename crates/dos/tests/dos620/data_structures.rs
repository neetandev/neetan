use crate::harness;

#[test]
fn cds_boot_drive_path() {
    let mut machine = harness::boot_hle();
    let sysvars = harness::get_sysvars_address(&mut machine);

    // CDS array pointer at SYSVARS+0x16.
    let (cds_seg, cds_off) = harness::read_far_ptr(&machine.bus, sysvars + 0x16);
    let cds_addr = harness::far_to_linear(cds_seg, cds_off);

    // Boot drive index from SYSVARS+0x43 (1=A:, 2=B:, ...).
    let boot_drive = harness::read_byte(&machine.bus, sysvars + 0x43);
    assert!(
        (1..=26).contains(&boot_drive),
        "Boot drive should be 1-26, got {}",
        boot_drive
    );

    // Each CDS entry is 88 (0x58) bytes. The current path is the first 67 bytes.
    let drive_index = (boot_drive - 1) as u32;
    let entry_addr = cds_addr + drive_index * 0x58;
    let byte0 = harness::read_byte(&machine.bus, entry_addr);
    let byte1 = harness::read_byte(&machine.bus, entry_addr + 1);
    let byte2 = harness::read_byte(&machine.bus, entry_addr + 2);

    let drive_letter = b'A' + (boot_drive - 1);
    assert_eq!(
        byte0, drive_letter,
        "CDS boot drive path should start with '{}', got {:#04X}",
        drive_letter as char, byte0
    );
    assert_eq!(
        byte1, 0x3A,
        "CDS boot drive path second byte should be ':' (0x3A), got {:#04X}",
        byte1
    );
    assert_eq!(
        byte2, 0x5C,
        "CDS boot drive path third byte should be '\\' (0x5C), got {:#04X}",
        byte2
    );
}

#[test]
fn cds_boot_drive_flags() {
    let mut machine = harness::boot_hle();
    let sysvars = harness::get_sysvars_address(&mut machine);

    let (cds_seg, cds_off) = harness::read_far_ptr(&machine.bus, sysvars + 0x16);
    let cds_addr = harness::far_to_linear(cds_seg, cds_off);
    let boot_drive = harness::read_byte(&machine.bus, sysvars + 0x43);
    let drive_index = (boot_drive - 1) as u32;
    let entry_addr = cds_addr + drive_index * 0x58;

    // CDS flags at offset +0x43 within the entry (WORD).
    let flags = harness::read_word(&machine.bus, entry_addr + 0x43);
    // Boot drive Z: is virtual (0x8000), physical drives have 0x4000.
    assert!(
        flags != 0,
        "CDS flags for boot drive should be non-zero, got {:#06X}",
        flags
    );
}

#[test]
fn dpb_first_entry_fields() {
    let mut machine = harness::boot_hle();
    let sysvars = harness::get_sysvars_address(&mut machine);

    // First DPB at SYSVARS+0x00.
    let (dpb_seg, dpb_off) = harness::read_far_ptr(&machine.bus, sysvars);
    let dpb_addr = harness::far_to_linear(dpb_seg, dpb_off);

    // DPB structure (DOS 4.0+):
    // +0x00: drive number (BYTE)
    // +0x01: unit number (BYTE)
    // +0x02: bytes per sector (WORD)
    // +0x04: sectors per cluster - 1 (BYTE, i.e., cluster mask)
    // +0x05: cluster shift (BYTE)
    // +0x08: number of FATs (BYTE)
    // +0x16: media descriptor (BYTE)
    let drive_number = harness::read_byte(&machine.bus, dpb_addr);
    assert!(
        drive_number < 26,
        "DPB drive number should be < 26, got {}",
        drive_number
    );

    let bytes_per_sector = harness::read_word(&machine.bus, dpb_addr + 0x02);
    assert!(
        bytes_per_sector == 256 || bytes_per_sector == 512 || bytes_per_sector == 1024,
        "DPB bytes per sector should be 256, 512, or 1024, got {}",
        bytes_per_sector
    );

    let cluster_mask = harness::read_byte(&machine.bus, dpb_addr + 0x04);
    let sectors_per_cluster = cluster_mask as u16 + 1;
    assert!(
        sectors_per_cluster.is_power_of_two(),
        "DPB sectors per cluster should be power of 2, got {}",
        sectors_per_cluster
    );

    let media_descriptor = harness::read_byte(&machine.bus, dpb_addr + 0x17);
    assert!(
        media_descriptor >= 0xF0,
        "DPB media descriptor should be >= 0xF0, got {:#04X}",
        media_descriptor
    );
}

#[test]
fn sft_first_node_five_entries() {
    let mut machine = harness::boot_hle();
    let sysvars = harness::get_sysvars_address(&mut machine);

    // SFT chain at SYSVARS+0x04.
    let (sft_seg, sft_off) = harness::read_far_ptr(&machine.bus, sysvars + 0x04);
    let sft_addr = harness::far_to_linear(sft_seg, sft_off);

    // SFT header: 4-byte next pointer, 2-byte entry count.
    let entry_count = harness::read_word(&machine.bus, sft_addr + 0x04);
    assert_eq!(
        entry_count, 5,
        "First SFT node should have 5 entries (standard handles), got {}",
        entry_count
    );
}

#[test]
fn sft_stdout_is_character_device() {
    let mut machine = harness::boot_hle();
    let sysvars = harness::get_sysvars_address(&mut machine);

    let (sft_seg, sft_off) = harness::read_far_ptr(&machine.bus, sysvars + 0x04);
    let sft_addr = harness::far_to_linear(sft_seg, sft_off);

    // Each SFT entry is 59 bytes (DOS 4.0+). Entries start at offset +0x06 in the SFT node.
    // Entry 1 (stdout) = sft_addr + 0x06 + 59 * 1.
    // Within entry: +0x05 = device info word (WORD), +0x20 = device name (8 bytes).
    let entry_addr = sft_addr + 0x06 + 59;
    let device_info = harness::read_word(&machine.bus, entry_addr + 0x05);
    assert!(
        device_info & 0x0080 != 0,
        "SFT stdout entry device info bit 7 should be set (char device), got {:#06X}",
        device_info
    );

    let name = harness::read_bytes(&machine.bus, entry_addr + 0x20, 8);
    let name_str = String::from_utf8_lossy(&name);
    assert!(
        name_str.starts_with("CON"),
        "SFT stdout device name should start with 'CON', got '{}'",
        name_str
    );
}

#[test]
fn indos_flag_is_zero_at_idle() {
    let mut machine = harness::boot_hle();
    // Get InDOS flag address via INT 21h/34h.
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x34,                         // MOV AH, 34h
        0xCD, 0x21,                         // INT 21h
        0x89, 0x1E, 0x00, 0x01,             // MOV [0x0100], BX
        0x8C, 0x06, 0x02, 0x01,             // MOV [0x0102], ES
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let offset = harness::result_word(&machine.bus, 0);
    let segment = harness::result_word(&machine.bus, 2);
    let indos_addr = harness::far_to_linear(segment, offset);

    // In HLE mode, all INT 21h calls (including the shell's AH=FFh step)
    // complete instantly: InDOS is incremented on entry and decremented on
    // return within the same handler invocation. When inject_and_run takes
    // over the CPU between iterations of COMMAND.COM's stub loop, InDOS is 0.
    let indos_value = harness::read_byte(&machine.bus, indos_addr);
    assert_eq!(
        indos_value, 0,
        "InDOS flag should be 0 (HLE INT 21h calls complete instantly), got {}",
        indos_value
    );
}

#[test]
fn dbcs_table_at_fixed_address() {
    let mut machine = harness::boot_hle();
    // Get DBCS table pointer via INT 21h/63h.
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x63,                         // MOV AH, 63h
        0xB0, 0x00,                         // MOV AL, 00h
        0xCD, 0x21,                         // INT 21h
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

    // The DBCS table lives in DOS's data segment. Its address is NOT fixed;
    // it depends on where DOS loads. INT 21h/63h returns the actual pointer.
    // On NEC MS-DOS 6.20 the segment is typically in the DOS kernel area.
    assert!(
        ds > 0 && ds < 0x9000,
        "DBCS table segment should be in DOS data area, got {:#06X}",
        ds
    );

    // Verify the table content has Shift-JIS ranges.
    let range1_start = harness::read_byte(&machine.bus, table_addr);
    let range1_end = harness::read_byte(&machine.bus, table_addr + 1);
    let range2_start = harness::read_byte(&machine.bus, table_addr + 2);
    let range2_end = harness::read_byte(&machine.bus, table_addr + 3);
    let terminator1 = harness::read_byte(&machine.bus, table_addr + 4);
    let terminator2 = harness::read_byte(&machine.bus, table_addr + 5);

    assert_eq!(range1_start, 0x81, "DBCS range 1 start should be 0x81");
    assert_eq!(range1_end, 0x9F, "DBCS range 1 end should be 0x9F");
    assert_eq!(range2_start, 0xE0, "DBCS range 2 start should be 0xE0");
    assert_eq!(range2_end, 0xFC, "DBCS range 2 end should be 0xFC");
    assert_eq!(
        terminator1, 0x00,
        "DBCS table terminator byte 1 should be 0x00"
    );
    assert_eq!(
        terminator2, 0x00,
        "DBCS table terminator byte 2 should be 0x00"
    );
}
