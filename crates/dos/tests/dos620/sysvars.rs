use crate::harness;

fn boot_and_get_sysvars() -> (machine::Pc9801Ra, u32) {
    let mut machine = harness::boot_hle();
    let sysvars = harness::get_sysvars_address(&mut machine);
    (machine, sysvars)
}

#[test]
fn int21h_52h_returns_valid_pointer() {
    let (_machine, sysvars) = boot_and_get_sysvars();
    assert!(
        sysvars < 0xA0000,
        "SYSVARS pointer should be in conventional memory, got {:#010X}",
        sysvars
    );
    assert!(
        sysvars >= 0x0600,
        "SYSVARS pointer should be above IVT/BDA, got {:#010X}",
        sysvars
    );
}

#[test]
fn first_mcb_segment() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let mcb_segment = harness::read_word(&machine.bus, sysvars - 2);
    let mcb_linear = harness::far_to_linear(mcb_segment, 0);
    let mcb_type = harness::read_byte(&machine.bus, mcb_linear);
    assert!(
        mcb_type == 0x4D || mcb_type == 0x5A,
        "First MCB at segment {:#06X} should start with 'M' (0x4D) or 'Z' (0x5A), got {:#04X}",
        mcb_segment,
        mcb_type
    );
}

#[test]
fn dpb_pointer_valid() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let (segment, offset) = harness::read_far_ptr(&machine.bus, sysvars);
    let linear = harness::far_to_linear(segment, offset);
    assert!(
        linear < 0xA0000 && linear > 0,
        "DPB pointer should be in conventional memory, got {:#010X}",
        linear
    );
    // First byte of DPB is the drive number (0=A:, 1=B:, etc.)
    let drive_number = harness::read_byte(&machine.bus, linear);
    assert!(
        drive_number < 26,
        "DPB drive number should be < 26, got {}",
        drive_number
    );
}

#[test]
fn sft_pointer_valid() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let (segment, offset) = harness::read_far_ptr(&machine.bus, sysvars + 0x04);
    let linear = harness::far_to_linear(segment, offset);
    assert!(
        linear < 0xA0000 && linear > 0,
        "SFT pointer should be in conventional memory, got {:#010X}",
        linear
    );
}

#[test]
fn clock_device_pointer() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let (segment, offset) = harness::read_far_ptr(&machine.bus, sysvars + 0x08);
    let linear = harness::far_to_linear(segment, offset);
    let name = harness::read_device_name(&machine.bus, linear);
    let trimmed = name.trim_end();
    assert!(
        trimmed == "CLOCK$" || trimmed == "CLOCK",
        "SYSVARS+0x08 should point to CLOCK device, got '{}'",
        trimmed
    );
}

#[test]
fn con_device_pointer() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let (segment, offset) = harness::read_far_ptr(&machine.bus, sysvars + 0x0C);
    let linear = harness::far_to_linear(segment, offset);
    let name = harness::read_device_name(&machine.bus, linear);
    assert_eq!(
        name.trim_end(),
        "CON",
        "SYSVARS+0x0C should point to CON device, got '{}'",
        name
    );
}

#[test]
fn max_bytes_per_sector() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let max_sector = harness::read_word(&machine.bus, sysvars + 0x10);
    assert!(
        max_sector == 512 || max_sector == 1024,
        "Max bytes per sector should be 512 or 1024, got {}",
        max_sector
    );
}

#[test]
fn disk_buffer_pointer_valid() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let (segment, offset) = harness::read_far_ptr(&machine.bus, sysvars + 0x12);
    let linear = harness::far_to_linear(segment, offset);
    assert!(
        linear < 0xA0000 && linear > 0,
        "Disk buffer pointer should be in conventional memory, got {:#010X}",
        linear
    );
}

#[test]
fn cds_pointer_valid() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let (segment, offset) = harness::read_far_ptr(&machine.bus, sysvars + 0x16);
    let linear = harness::far_to_linear(segment, offset);
    assert!(
        linear < 0xA0000 && linear > 0,
        "CDS pointer should be in conventional memory, got {:#010X}",
        linear
    );
}

#[test]
fn fcb_sft_pointer_valid() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let (segment, offset) = harness::read_far_ptr(&machine.bus, sysvars + 0x1A);
    let linear = harness::far_to_linear(segment, offset);
    assert!(
        linear < 0xA0000 && linear > 0,
        "FCB-SFT pointer should be in conventional memory, got {:#010X}",
        linear
    );
}

#[test]
fn block_device_count() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let count = harness::read_byte(&machine.bus, sysvars + 0x20);
    // HLE boot with no media has 0 block devices; with media it would be >= 1.
    assert!(
        count <= 26,
        "Block device count should be <= 26, got {}",
        count
    );
}

#[test]
fn lastdrive_value() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let lastdrive = harness::read_byte(&machine.bus, sysvars + 0x21);
    assert!(
        (5..=26).contains(&lastdrive),
        "LASTDRIVE should be between 5 and 26, got {}",
        lastdrive
    );
}

#[test]
fn nul_device_header() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let name = harness::read_device_name(&machine.bus, sysvars + 0x22);
    assert_eq!(
        name.trim_end(),
        "NUL",
        "NUL device header at SYSVARS+0x22 should have name 'NUL', got '{}'",
        name
    );
}

#[test]
fn device_chain_order() {
    let (machine, sysvars) = boot_and_get_sysvars();
    // Walk device chain starting from NUL at SYSVARS+0x22.
    // NEETAN DOS chain: NUL -> CON
    // CLOCK is NOT in the chain (only referenced by SYSVARS+0x08 pointer).
    let expected_names = ["NUL", "CON"];
    let mut addr = sysvars + 0x22;
    let mut found_names = Vec::new();

    for _ in 0..20 {
        let name = harness::read_device_name(&machine.bus, addr);
        let trimmed = name.trim_end().to_string();
        found_names.push(trimmed);

        // Read next pointer (first 4 bytes of device header).
        let (next_seg, next_off) = harness::read_far_ptr(&machine.bus, addr);
        if next_seg == 0xFFFF && next_off == 0xFFFF {
            break;
        }
        addr = harness::far_to_linear(next_seg, next_off);
    }

    for expected in &expected_names {
        assert!(
            found_names.iter().any(|name| name == expected),
            "Device chain should contain '{}', found: {:?}",
            expected,
            found_names
        );
    }

    // Verify NUL is first.
    assert_eq!(
        found_names[0], "NUL",
        "First device in chain should be NUL, got '{}'",
        found_names[0]
    );
}

#[test]
fn join_drives_zero() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let join_count = harness::read_word(&machine.bus, sysvars + 0x34);
    assert_eq!(
        join_count, 0,
        "Number of JOIN'ed drives should be 0, got {}",
        join_count
    );
}

#[test]
fn buffers_value() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let buffers = harness::read_word(&machine.bus, sysvars + 0x3F);
    assert!(
        (1..=99).contains(&buffers),
        "BUFFERS value should be in range 1-99, got {}",
        buffers
    );
}

#[test]
fn boot_drive() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let boot_drive = harness::read_byte(&machine.bus, sysvars + 0x43);
    // Boot drive: 1=A:, 2=B:, 3=C:, etc.
    assert!(
        (1..=26).contains(&boot_drive),
        "Boot drive should be between 1 and 26, got {}",
        boot_drive
    );
}

#[test]
fn cpu_386_flag() {
    let (machine, sysvars) = boot_and_get_sysvars();
    let flag = harness::read_byte(&machine.bus, sysvars + 0x44);
    assert_eq!(
        flag, 0x01,
        "386+ CPU flag should be 0x01 (DWORD moves available), got {:#04X}",
        flag
    );
}
