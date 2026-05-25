use crate::harness;

#[test]
fn ivt_has_valid_int21h_vector() {
    let machine = harness::boot_hle();
    // INT 21h vector is at IVT offset 0x21 * 4 = 0x0084.
    let (segment, offset) = harness::read_far_ptr(&machine.bus, 0x0084);
    let linear = harness::far_to_linear(segment, offset);
    assert_ne!(
        linear, 0,
        "INT 21h vector should point to a valid DOS handler, got 0000:0000"
    );
    // HLE vectors point to BIOS ROM stubs (above 0xA0000 in the ROM range).
    assert!(
        linear < 0x100000,
        "INT 21h vector should be within 1MB address space, got {:#010X}",
        linear
    );
}

#[test]
fn bios_data_area_populated() {
    let machine = harness::boot_hle();
    // BIOS Data Area starts at 0x0400. Check equipment word area at 0x0500.
    let equipment_byte = harness::read_byte(&machine.bus, 0x0500);
    assert_ne!(
        equipment_byte, 0x00,
        "BDA equipment byte at 0x0500 should be non-zero after POST"
    );
}

#[test]
fn dos_data_area_populated() {
    let mut machine = harness::boot_hle();
    // SYSVARS and DOS structures live in the DOS data segment (found via INT 21h/52h).
    // Verify the area around SYSVARS contains significant non-zero data.
    let sysvars = harness::get_sysvars_address(&mut machine);
    let mut nonzero_count = 0;
    for i in 0..0x100 {
        if harness::read_byte(&machine.bus, sysvars + i) != 0 {
            nonzero_count += 1;
        }
    }
    assert!(
        nonzero_count > 10,
        "DOS data area around SYSVARS ({:#010X}) should contain significant non-zero data, found only {} non-zero bytes",
        sysvars,
        nonzero_count
    );
}

#[test]
fn nul_device_header_at_expected_location() {
    let mut machine = harness::boot_hle();
    // Get SYSVARS via INT 21h/52h. NUL device header starts at SYSVARS+0x22.
    let sysvars = harness::get_sysvars_address(&mut machine);
    let name = harness::read_device_name(&machine.bus, sysvars + 0x22);
    assert_eq!(
        name.trim(),
        "NUL",
        "Device header at SYSVARS+0x22 should be NUL device, got '{}'",
        name
    );
}

#[test]
fn conventional_memory_below_a0000() {
    let mut machine = harness::boot_hle();
    // Verify the MCB chain stays within conventional memory (below 0xA0000).
    let sysvars = harness::get_sysvars_address(&mut machine);
    let first_mcb_seg = harness::read_word(&machine.bus, sysvars - 2);
    let mut mcb_addr = harness::far_to_linear(first_mcb_seg, 0);

    for _ in 0..1000 {
        let block_type = harness::read_byte(&machine.bus, mcb_addr);
        let size = harness::read_word(&machine.bus, mcb_addr + 3);
        let mcb_seg = mcb_addr >> 4;
        let end_seg = mcb_seg + size as u32 + 1;
        let end_linear = end_seg << 4;

        assert!(
            end_linear <= 0xA0000,
            "MCB chain must stay below 0xA0000, block at seg {:#06X} extends to {:#010X}",
            mcb_seg,
            end_linear
        );

        if block_type == 0x5A {
            break;
        }
        mcb_addr = end_seg << 4;
    }
}

#[test]
fn ivt_int24h_vector_valid() {
    let machine = harness::boot_hle();
    // INT 24h vector at IVT offset 0x24 * 4 = 0x0090.
    let (segment, offset) = harness::read_far_ptr(&machine.bus, 0x0090);
    let linear = harness::far_to_linear(segment, offset);
    assert_ne!(
        linear, 0,
        "INT 24h vector should be non-zero (critical-error handler)"
    );
    assert!(
        linear < 0x100000,
        "INT 24h vector should be within 1MB address space, got {:#010X}",
        linear
    );
    // Handler must be MOV AL,3 / IRET (return Fail).
    assert_eq!(harness::read_byte(&machine.bus, linear), 0xB0);
    assert_eq!(harness::read_byte(&machine.bus, linear + 1), 0x03);
    assert_eq!(harness::read_byte(&machine.bus, linear + 2), 0xCF);
}

#[test]
fn ivt_int29h_vector_valid() {
    let machine = harness::boot_hle();
    // INT 29h vector at IVT offset 0x29 * 4 = 0x00A4.
    let (segment, offset) = harness::read_far_ptr(&machine.bus, 0x00A4);
    let linear = harness::far_to_linear(segment, offset);
    assert_ne!(
        linear, 0,
        "INT 29h vector should be non-zero (fast console output handler)"
    );
    assert!(
        linear < 0x100000,
        "INT 29h vector should be within 1MB address space, got {:#010X}",
        linear
    );
}

#[test]
fn ivt_int2fh_vector_valid() {
    let machine = harness::boot_hle();
    // INT 2Fh vector at IVT offset 0x2F * 4 = 0x00BC.
    let (segment, offset) = harness::read_far_ptr(&machine.bus, 0x00BC);
    let linear = harness::far_to_linear(segment, offset);
    assert_ne!(
        linear, 0,
        "INT 2Fh vector should be non-zero (multiplex interrupt handler)"
    );
    assert!(
        linear < 0x100000,
        "INT 2Fh vector should be within 1MB address space, got {:#010X}",
        linear
    );
}

#[test]
fn ivt_intdch_vector_valid() {
    let machine = harness::boot_hle();
    // INT DCh vector at IVT offset 0xDC * 4 = 0x0370.
    let (segment, offset) = harness::read_far_ptr(&machine.bus, 0x0370);
    let linear = harness::far_to_linear(segment, offset);
    assert_ne!(
        linear, 0,
        "INT DCh vector should be non-zero (NEC DOS extension handler)"
    );
    assert!(
        linear < 0x100000,
        "INT DCh vector should be within 1MB address space, got {:#010X}",
        linear
    );
}
