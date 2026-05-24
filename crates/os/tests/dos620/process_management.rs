//! Integration tests for process management (EXEC and terminate).

use common::Bus;

use crate::harness::*;

fn create_test_floppy_with_two_programs(
    first_name: &[u8; 11],
    first_program: &[u8],
    second_name: &[u8; 11],
    second_program: &[u8],
) -> device::floppy::FloppyImage {
    use device::floppy::d88::{D88Disk, D88MediaType, D88Sector};

    const CYLINDERS: usize = 77;
    const HEADS: usize = 2;
    const SECTORS_PER_TRACK: usize = 8;
    const SECTOR_SIZE: usize = 1024;
    const ROOT_DIRECTORY_OFFSET: usize = 5 * SECTOR_SIZE;
    const CLUSTER2_OFFSET: usize = 11 * SECTOR_SIZE;
    const CLUSTER3_OFFSET: usize = 12 * SECTOR_SIZE;
    const FILE_TIME: u16 = 0;
    const FILE_DATE: u16 = 0;

    let total_tracks = CYLINDERS * HEADS;
    let total_sectors = CYLINDERS * HEADS * SECTORS_PER_TRACK;
    let mut disk_data = vec![0u8; total_sectors * SECTOR_SIZE];

    {
        let boot_sector = &mut disk_data[0..SECTOR_SIZE];
        boot_sector[0] = 0xEB;
        boot_sector[1] = 0x3C;
        boot_sector[2] = 0x90;
        boot_sector[3..11].copy_from_slice(b"NEETAN  ");
        boot_sector[11..13].copy_from_slice(&1024u16.to_le_bytes());
        boot_sector[13] = 1;
        boot_sector[14..16].copy_from_slice(&1u16.to_le_bytes());
        boot_sector[16] = 2;
        boot_sector[17..19].copy_from_slice(&192u16.to_le_bytes());
        boot_sector[19..21].copy_from_slice(&1232u16.to_le_bytes());
        boot_sector[21] = 0xFE;
        boot_sector[22..24].copy_from_slice(&2u16.to_le_bytes());
        boot_sector[24..26].copy_from_slice(&8u16.to_le_bytes());
        boot_sector[26..28].copy_from_slice(&2u16.to_le_bytes());
    }

    let fat1_offset = SECTOR_SIZE;
    disk_data[fat1_offset] = 0xFE;
    disk_data[fat1_offset + 1] = 0xFF;
    disk_data[fat1_offset + 2] = 0xFF;
    disk_data[fat1_offset + 3] = 0xFF;
    disk_data[fat1_offset + 4] = 0xFF;
    disk_data[fat1_offset + 5] = 0xFF;

    let fat2_offset = 3 * SECTOR_SIZE;
    let fat1_end = fat1_offset + 2 * SECTOR_SIZE;
    let fat1_copy = disk_data[fat1_offset..fat1_end].to_vec();
    disk_data[fat2_offset..fat2_offset + fat1_copy.len()].copy_from_slice(&fat1_copy);

    {
        let first_entry = &mut disk_data[ROOT_DIRECTORY_OFFSET..ROOT_DIRECTORY_OFFSET + 32];
        first_entry[0..11].copy_from_slice(first_name);
        first_entry[11] = 0x20;
        first_entry[22..24].copy_from_slice(&FILE_TIME.to_le_bytes());
        first_entry[24..26].copy_from_slice(&FILE_DATE.to_le_bytes());
        first_entry[26..28].copy_from_slice(&2u16.to_le_bytes());
        first_entry[28..32].copy_from_slice(&(first_program.len() as u32).to_le_bytes());
    }

    {
        let second_entry = &mut disk_data[ROOT_DIRECTORY_OFFSET + 32..ROOT_DIRECTORY_OFFSET + 64];
        second_entry[0..11].copy_from_slice(second_name);
        second_entry[11] = 0x20;
        second_entry[22..24].copy_from_slice(&FILE_TIME.to_le_bytes());
        second_entry[24..26].copy_from_slice(&FILE_DATE.to_le_bytes());
        second_entry[26..28].copy_from_slice(&3u16.to_le_bytes());
        second_entry[28..32].copy_from_slice(&(second_program.len() as u32).to_le_bytes());
    }

    disk_data[CLUSTER2_OFFSET..CLUSTER2_OFFSET + first_program.len()]
        .copy_from_slice(first_program);
    disk_data[CLUSTER3_OFFSET..CLUSTER3_OFFSET + second_program.len()]
        .copy_from_slice(second_program);

    let mut tracks = Vec::with_capacity(total_tracks);
    for track_index in 0..total_tracks {
        let cylinder = (track_index / HEADS) as u8;
        let head = (track_index % HEADS) as u8;
        let mut sectors = Vec::with_capacity(SECTORS_PER_TRACK);
        for sector_index in 0..SECTORS_PER_TRACK {
            let linear_sector = track_index * SECTORS_PER_TRACK + sector_index;
            let data_offset = linear_sector * SECTOR_SIZE;
            sectors.push(D88Sector {
                cylinder,
                head,
                record: (sector_index + 1) as u8,
                size_code: 3,
                sector_count: SECTORS_PER_TRACK as u16,
                mfm_flag: 0x40,
                deleted: 0,
                status: 0,
                reserved: [0; 5],
                data: disk_data[data_offset..data_offset + SECTOR_SIZE].to_vec(),
                source_offset: None,
            });
        }
        tracks.push(Some(sectors));
    }

    let disk = D88Disk::from_tracks("EXECTEST".to_string(), false, D88MediaType::Disk2HD, tracks);
    device::floppy::FloppyImage::from_d88(disk)
}

/// EXECs the given filename, reads flags and return code via INT 21h/4Dh,
/// and asserts CF=0 and AX matches `expected_return_ax`.
fn exec_file_and_check_return_code(
    machine: &mut machine::Pc9801Ra,
    filename: &[u8],
    expected_return_ax: u16,
) {
    let base = INJECT_CODE_BASE;
    let seg = INJECT_CODE_SEGMENT;

    write_bytes(&mut machine.bus, base + 0x0200, filename);
    machine.bus.write_byte(base + 0x0220, 0x00);
    machine.bus.write_byte(base + 0x0221, 0x0D);

    let seg_lo = (seg & 0xFF) as u8;
    let seg_hi = (seg >> 8) as u8;
    #[rustfmt::skip]
    let param_block: [u8; 14] = [
        0x00, 0x00,
        0x20, 0x02, seg_lo, seg_hi,
        0x30, 0x02, seg_lo, seg_hi,
        0x40, 0x02, seg_lo, seg_hi,
    ];
    write_bytes(&mut machine.bus, base + 0x0210, &param_block);

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    const SEG_LO: u8 = (INJECT_CODE_SEGMENT & 0xFF) as u8;
    const SEG_HI: u8 = (INJECT_CODE_SEGMENT >> 8) as u8;

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,               // MOV DX, 0200h (filename)
        0xBB, 0x10, 0x02,               // MOV BX, 0210h (param block)
        0xB8, 0x00, 0x4B,               // MOV AX, 4B00h (EXEC)
        0xCD, 0x21,                     // INT 21h
        // Reload DS and ES before writing results so this helper does not
        // depend on EXEC register preservation.
        0x9C,                            // PUSHF
        0xB8, SEG_LO, SEG_HI,           // MOV AX, INJECT_CODE_SEGMENT
        0x8E, 0xD8,                     // MOV DS, AX
        0x8E, 0xC0,                     // MOV ES, AX
        0x58,                           // POP AX (saved flags)
        0x89, 0x06, RES_LO, RES_HI,     // MOV [result+0], AX (flags)
        0xB4, 0x4D,                     // MOV AH, 4Dh
        0xCD, 0x21,                     // INT 21h
        0x89, 0x06, RES_LO + 2, RES_HI, // MOV [result+2], AX
        0xFA,                           // CLI
        0xF4,                           // HLT
    ];

    inject_and_run_with_budget(machine, code, INJECT_BUDGET_DISK_IO);

    let flags = result_word(&machine.bus, 0);
    assert_eq!(
        flags & 0x0001,
        0,
        "EXEC should return with CF=0, flags={:#06X}",
        flags
    );

    let return_ax = result_word(&machine.bus, 2);
    assert_eq!(return_ax, expected_return_ax, "unexpected return code");
}

/// EXEC a .COM file on the test floppy and verify the child terminates
/// with the expected return code.
#[test]
fn exec_com_and_get_return_code() {
    let mut machine = boot_hle_with_floppy();

    let base = INJECT_CODE_BASE;
    let seg = INJECT_CODE_SEGMENT;

    // Filename at +0x0200
    write_bytes(&mut machine.bus, base + 0x0200, b"A:\\TEST.COM\x00");

    // Command tail at +0x0220: length=0, CR
    machine.bus.write_byte(base + 0x0220, 0x00);
    machine.bus.write_byte(base + 0x0221, 0x0D);

    // EXEC parameter block at +0x0210
    let seg_lo = (seg & 0xFF) as u8;
    let seg_hi = (seg >> 8) as u8;
    #[rustfmt::skip]
    let param_block: [u8; 14] = [
        0x00, 0x00,                     // env_seg = 0 (inherit)
        0x20, 0x02, seg_lo, seg_hi,     // cmd_tail = seg:0220
        0x30, 0x02, seg_lo, seg_hi,     // fcb1 = seg:0230
        0x40, 0x02, seg_lo, seg_hi,     // fcb2 = seg:0240
    ];
    write_bytes(&mut machine.bus, base + 0x0210, &param_block);

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;

    // Injected code:
    // 1. EXEC A:\TEST.COM (child exits with code 0x42)
    // 2. Reload DS/ES so the test does not depend on register preservation
    // 3. Save carry flag
    // 4. Get return code via INT 21h/4Dh
    // 5. Get current PSP via INT 21h/62h
    // 6. CLI + HLT
    const SEG_LO: u8 = (INJECT_CODE_SEGMENT & 0xFF) as u8;
    const SEG_HI: u8 = (INJECT_CODE_SEGMENT >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        // DS and ES already set to INJECT_CODE_SEGMENT by inject_and_run
        0xBA, 0x00, 0x02,               // MOV DX, 0200h (filename)
        0xBB, 0x10, 0x02,               // MOV BX, 0210h (param block)
        0xB8, 0x00, 0x4B,               // MOV AX, 4B00h (EXEC)
        0xCD, 0x21,                     // INT 21h
        // After child terminates, we return here.
        // Reload DS and ES before writing results so this test does not
        // depend on EXEC register preservation.
        0x9C,                           // PUSHF (save CF from EXEC result)
        0xB8, SEG_LO, SEG_HI,           // MOV AX, INJECT_CODE_SEGMENT
        0x8E, 0xD8,                     // MOV DS, AX
        0x8E, 0xC0,                     // MOV ES, AX
        0x58,                           // POP AX (get saved flags)
        0x89, 0x06, RES_LO, RES_HI,     // MOV [result+0], AX (flags)
        // Get return code
        0xB4, 0x4D,                     // MOV AH, 4Dh
        0xCD, 0x21,                     // INT 21h
        0x89, 0x06, RES_LO + 2, RES_HI, // MOV [result+2], AX
        // Get current PSP
        0xB4, 0x62,                     // MOV AH, 62h
        0xCD, 0x21,                     // INT 21h
        0x89, 0x1E, RES_LO + 4, RES_HI, // MOV [result+4], BX
        0xFA,                           // CLI
        0xF4,                           // HLT
    ];

    inject_and_run_with_budget(&mut machine, code, INJECT_BUDGET_DISK_IO);

    // CF should be clear (success)
    let flags = result_word(&machine.bus, 0);
    assert_eq!(
        flags & 0x0001,
        0,
        "EXEC should return with CF=0 on success, flags={:#06X}",
        flags
    );

    // Return code = 0x42, termination type = 0x00
    let return_ax = result_word(&machine.bus, 2);
    assert_eq!(
        return_ax, 0x0042,
        "INT 21h/4Dh should return AX=0042h (code=42h, type=00h), got {:#06X}",
        return_ax
    );

    // Current PSP should be restored to parent
    let psp_after = result_word(&machine.bus, 4);
    assert_ne!(psp_after, 0, "current PSP should be restored to parent");
}

/// After EXEC + terminate, the child's MCB should be freed and coalesced.
#[test]
fn terminate_frees_child_memory_and_coalesces() {
    let mut machine = boot_hle_with_floppy();

    let base = INJECT_CODE_BASE;
    let seg = INJECT_CODE_SEGMENT;

    write_bytes(&mut machine.bus, base + 0x0200, b"A:\\TEST.COM\x00");
    machine.bus.write_byte(base + 0x0220, 0x00);
    machine.bus.write_byte(base + 0x0221, 0x0D);

    let seg_lo = (seg & 0xFF) as u8;
    let seg_hi = (seg >> 8) as u8;
    #[rustfmt::skip]
    let param_block: [u8; 14] = [
        0x00, 0x00,
        0x20, 0x02, seg_lo, seg_hi,
        0x30, 0x02, seg_lo, seg_hi,
        0x40, 0x02, seg_lo, seg_hi,
    ];
    write_bytes(&mut machine.bus, base + 0x0210, &param_block);

    // Get first MCB before EXEC
    let sysvars = get_sysvars_address(&mut machine);
    let first_mcb_segment = read_word(&machine.bus, sysvars - 2);
    let blocks_before = walk_mcb_chain(&machine.bus, first_mcb_segment);

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;

    const SEG_LO2: u8 = (INJECT_CODE_SEGMENT & 0xFF) as u8;
    const SEG_HI2: u8 = (INJECT_CODE_SEGMENT >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,                      // MOV DX, 0200h
        0xBB, 0x10, 0x02,                      // MOV BX, 0210h
        0xB8, 0x00, 0x4B,                      // MOV AX, 4B00h
        0xCD, 0x21,                            // INT 21h
        // Reload DS before writing results so this test does not depend on
        // EXEC register preservation.
        0xB8, SEG_LO2, SEG_HI2,                // MOV AX, INJECT_CODE_SEGMENT
        0x8E, 0xD8,                            // MOV DS, AX
        0xC6, 0x06, RES_LO, RES_HI, 0x01,      // MOV BYTE [result], 01h
        0xFA,                                  // CLI
        0xF4,                                  // HLT
    ];

    inject_and_run_with_budget(&mut machine, code, INJECT_BUDGET_DISK_IO);

    assert_eq!(
        result_byte(&machine.bus, 0),
        0x01,
        "EXEC should have returned to parent"
    );

    // Walk MCB chain after terminate
    let blocks_after = walk_mcb_chain(&machine.bus, first_mcb_segment);

    // Free memory should be >= before (child blocks freed and coalesced)
    let free_before: u32 = blocks_before
        .iter()
        .filter(|b| b.owner == 0)
        .map(|b| b.size as u32)
        .sum();
    let free_after: u32 = blocks_after
        .iter()
        .filter(|b| b.owner == 0)
        .map(|b| b.size as u32)
        .sum();
    assert!(
        free_after >= free_before,
        "free memory after terminate ({}) should be >= before ({})",
        free_after,
        free_before
    );

    // MCB chain should end with Z block
    assert_eq!(
        blocks_after.last().map(|b| b.block_type),
        Some(0x5A),
        "MCB chain should end with Z block"
    );
}

#[test]
fn exec_restores_parent_registers_for_post_child_dispatch() {
    let first_child_program: &[u8] = &[
        0xB8, 0x01, 0x4C, // MOV AX, 4C01h
        0xCD, 0x21, // INT 21h
    ];
    let second_child_program: &[u8] = &[
        0xB8, 0x42, 0x4C, // MOV AX, 4C42h
        0xCD, 0x21, // INT 21h
    ];
    let floppy = create_test_floppy_with_two_programs(
        b"RET1    COM",
        first_child_program,
        b"PASS    COM",
        second_child_program,
    );
    let mut machine = boot_hle_with_floppy_image(floppy);

    let base = INJECT_CODE_BASE;
    let seg = INJECT_CODE_SEGMENT;
    let seg_lo = (seg & 0xFF) as u8;
    let seg_hi = (seg >> 8) as u8;

    write_bytes(&mut machine.bus, base + 0x0200, b"A:\\RET1.COM\x00");
    write_bytes(&mut machine.bus, base + 0x0210, b"A:\\PASS.COM\x00");
    machine.bus.write_word(base + 0x0220, 0x0000);
    machine.bus.write_word(base + 0x0222, 0x0210);
    machine.bus.write_byte(base + 0x0240, 0x00);
    machine.bus.write_byte(base + 0x0241, 0x0D);

    #[rustfmt::skip]
    let param_block: [u8; 14] = [
        0x00, 0x00,
        0x40, 0x02, seg_lo, seg_hi,
        0x50, 0x02, seg_lo, seg_hi,
        0x60, 0x02, seg_lo, seg_hi,
    ];
    write_bytes(&mut machine.bus, base + 0x0230, &param_block);

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h (RET1.COM)
        0xBB, 0x30, 0x02,                   // MOV BX, 0230h (param block)
        0xB8, 0x00, 0x4B,                   // MOV AX, 4B00h
        0xCD, 0x21,                         // INT 21h
        0xB4, 0x4D,                         // MOV AH, 4Dh
        0xCD, 0x21,                         // INT 21h
        0x2E, 0xA3, 0x00, 0x01,             // MOV CS:[0100], AX
        0x8B, 0xF0,                         // MOV SI, AX
        0xD1, 0xE6,                         // SHL SI, 1
        0x8B, 0x94, 0x20, 0x02,             // MOV DX, [SI+0220]
        0xBB, 0x30, 0x02,                   // MOV BX, 0230h
        0xB8, 0x00, 0x4B,                   // MOV AX, 4B00h
        0xCD, 0x21,                         // INT 21h
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0x2E, 0xA3, 0x02, 0x01,             // MOV CS:[0102], AX
        0xB4, 0x4D,                         // MOV AH, 4Dh
        0xCD, 0x21,                         // INT 21h
        0x2E, 0xA3, 0x04, 0x01,             // MOV CS:[0104], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];

    inject_and_run_with_budget(&mut machine, code, INJECT_BUDGET_DISK_IO);

    assert_eq!(
        result_word(&machine.bus, 0),
        0x0001,
        "the first child should return code 1 to drive the filename table"
    );

    let second_exec_flags = result_word(&machine.bus, 2);
    assert_eq!(
        second_exec_flags & 0x0001,
        0,
        "the second EXEC should succeed when the parent register context is restored"
    );

    assert_eq!(
        result_word(&machine.bus, 4),
        0x0042,
        "post-child filename dispatch should launch PASS.COM and return code 0x42"
    );
}

/// TSR termination keeps the process MCB resident.
///
/// Replaces TEST.COM on the floppy with a TSR program, then EXECs it.
/// The TSR program keeps 32 paragraphs resident (DX=0x0020).
#[test]
fn tsr_keeps_memory_resident() {
    let mut machine = boot_hle_with_floppy();

    let base = INJECT_CODE_BASE;
    let seg = INJECT_CODE_SEGMENT;

    // Replace the floppy with one containing a TSR .COM program.
    // The TSR .COM keeps 32 paragraphs resident (DX=0x0020).
    let tsr_com: &[u8] = &[
        0xBA, 0x20, 0x00, // MOV DX, 0020h (keep 32 paragraphs)
        0xB8, 0x00, 0x31, // MOV AX, 3100h (TSR, exit code 0)
        0xCD, 0x21, // INT 21h
    ];
    machine.bus.eject_floppy(0);
    let floppy = create_test_floppy_with_program(b"TEST    COM", tsr_com);
    machine.bus.insert_floppy(0, floppy, None);

    // Set up EXEC parameter block
    write_bytes(&mut machine.bus, base + 0x0200, b"A:\\TEST.COM\x00");
    machine.bus.write_byte(base + 0x0220, 0x00);
    machine.bus.write_byte(base + 0x0221, 0x0D);

    let seg_lo = (seg & 0xFF) as u8;
    let seg_hi = (seg >> 8) as u8;
    #[rustfmt::skip]
    let param_block: [u8; 14] = [
        0x00, 0x00,
        0x20, 0x02, seg_lo, seg_hi,
        0x30, 0x02, seg_lo, seg_hi,
        0x40, 0x02, seg_lo, seg_hi,
    ];
    write_bytes(&mut machine.bus, base + 0x0210, &param_block);

    // Get first MCB before EXEC
    let sysvars = get_sysvars_address(&mut machine);
    let first_mcb_segment = read_word(&machine.bus, sysvars - 2);

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    const SEG_LO3: u8 = (INJECT_CODE_SEGMENT & 0xFF) as u8;
    const SEG_HI3: u8 = (INJECT_CODE_SEGMENT >> 8) as u8;

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,                      // MOV DX, 0200h (filename)
        0xBB, 0x10, 0x02,                      // MOV BX, 0210h (param block)
        0xB8, 0x00, 0x4B,                      // MOV AX, 4B00h (EXEC)
        0xCD, 0x21,                            // INT 21h
        // Restore DS after EXEC
        0xB8, SEG_LO3, SEG_HI3,                // MOV AX, INJECT_CODE_SEGMENT
        0x8E, 0xD8,                            // MOV DS, AX
        0xC6, 0x06, RES_LO, RES_HI, 0x01,      // MOV BYTE [result+0], 01h
        0xFA,                                  // CLI
        0xF4,                                  // HLT
    ];

    inject_and_run_with_budget(&mut machine, code, INJECT_BUDGET_DISK_IO);

    assert_eq!(
        result_byte(&machine.bus, 0),
        0x01,
        "TSR should have returned to parent"
    );

    // Walk MCB chain after TSR
    let blocks_after = walk_mcb_chain(&machine.bus, first_mcb_segment);

    // Find the child's PSP block. The EXEC allocated the largest block,
    // so the child PSP should be the block right after COMMAND.COM.
    // After TSR, the child's MCB should still be owned (not freed) but resized.
    let tsr_block = blocks_after
        .iter()
        .find(|b| b.owner != 0 && b.owner != 0x0008 && b.size <= 0x0021);
    assert!(
        tsr_block.is_some(),
        "TSR block should exist and be resized to ~32 paragraphs"
    );
    let tsr_block = tsr_block.unwrap();
    assert!(
        tsr_block.size <= 0x0021,
        "TSR block should be resized to ~32 paragraphs, got {}",
        tsr_block.size
    );

    // MCB chain should end with Z block
    assert_eq!(
        blocks_after.last().map(|b| b.block_type),
        Some(0x5A),
        "MCB chain should end with Z block"
    );
}

/// Build a minimal MZ EXE that executes `code_bytes` with the given stack size.
/// init_cs and init_ip are relative to the load segment (image base).
fn build_exe(code_bytes: &[u8], init_cs: u16, init_ip: u16, stack_size: u16) -> Vec<u8> {
    let header_paragraphs: u16 = 2; // 32 bytes = 2 paragraphs
    let header_size = (header_paragraphs as usize) * 16;
    let image_size = code_bytes.len() + stack_size as usize;
    let file_size = header_size + image_size;
    let total_pages = file_size.div_ceil(512) as u16;
    let bytes_last_page = (file_size % 512) as u16;
    // SS:SP relative to load segment. Put stack after code.
    let init_ss: u16 = 0;
    let init_sp = (code_bytes.len() as u16) + stack_size;

    let mut exe = vec![0u8; file_size];
    // MZ header
    exe[0] = 0x4D; // 'M'
    exe[1] = 0x5A; // 'Z'
    exe[2..4].copy_from_slice(&bytes_last_page.to_le_bytes());
    exe[4..6].copy_from_slice(&total_pages.to_le_bytes());
    exe[6..8].copy_from_slice(&0u16.to_le_bytes()); // reloc_count = 0
    exe[8..10].copy_from_slice(&header_paragraphs.to_le_bytes());
    exe[10..12].copy_from_slice(&0u16.to_le_bytes()); // min_alloc
    exe[12..14].copy_from_slice(&0xFFFFu16.to_le_bytes()); // max_alloc
    exe[14..16].copy_from_slice(&init_ss.to_le_bytes());
    exe[16..18].copy_from_slice(&init_sp.to_le_bytes());
    // checksum at [18..20] = 0
    exe[20..22].copy_from_slice(&init_ip.to_le_bytes());
    exe[22..24].copy_from_slice(&init_cs.to_le_bytes());
    exe[24..26].copy_from_slice(&(header_size as u16).to_le_bytes()); // reloc_table_offset
    // Code image
    exe[header_size..header_size + code_bytes.len()].copy_from_slice(code_bytes);
    exe
}

/// EXEC a .EXE file and verify the child terminates with the expected return code.
#[test]
fn exec_exe_and_get_return_code() {
    // Build a minimal MZ EXE: MOV AH,4Ch; MOV AL,42h; INT 21h
    let code: &[u8] = &[0xB4, 0x4C, 0xB0, 0x42, 0xCD, 0x21];
    let exe_data = build_exe(code, 0, 0, 256);

    let mut machine = boot_hle_with_floppy();
    machine.bus.eject_floppy(0);
    let floppy = create_test_floppy_with_program(b"TEST    EXE", &exe_data);
    machine.bus.insert_floppy(0, floppy, None);

    exec_file_and_check_return_code(&mut machine, b"A:\\TEST.EXE\x00", 0x0042);
}

#[test]
fn exec_exe_max_alloc_gets_largest_available_block() {
    let code: &[u8] = &[
        0xA1, 0x02, 0x00, // MOV AX, [0002h] (PSP memory top)
        0x16, // PUSH SS
        0x5B, // POP BX
        0x2B, 0xC3, // SUB AX, BX
        0x3D, 0x00, 0x01, // CMP AX, 0100h
        0xB0, 0x01, // MOV AL, 01h
        0x72, 0x02, // JC short exit
        0xB0, 0x42, // MOV AL, 42h
        0xB4, 0x4C, // MOV AH, 4Ch
        0xCD, 0x21, // INT 21h
    ];
    let exe_data = build_exe(code, 0, 0, 256);

    let mut machine = boot_hle_with_floppy();
    machine.bus.eject_floppy(0);
    let floppy = create_test_floppy_with_program(b"TEST    EXE", &exe_data);
    machine.bus.insert_floppy(0, floppy, None);

    exec_file_and_check_return_code(&mut machine, b"A:\\TEST.EXE\x00", 0x0042);
}

/// EXEC a .EXE with a relocation entry and verify it runs correctly.
#[test]
fn exec_exe_with_relocation() {
    // Build an EXE where the code references a segment that needs relocation.
    // The code does: MOV AX, [relocated_seg]:0000 ; then exits with that value.
    //
    // Layout (relative to load segment):
    //   CS:0000 = code
    //   Segment 0x0001:0000 = data word (0x0042)
    //
    // Code:
    //   MOV AX, 0001h        ; segment to be relocated -> becomes load_seg+1
    //   MOV DS, AX
    //   MOV AL, [0000h]      ; read byte from relocated segment
    //   MOV AH, 4Ch
    //   INT 21h
    let code: &[u8] = &[
        0xB8, 0x01, 0x00, // MOV AX, 0001h (to be relocated)
        0x8E, 0xD8, // MOV DS, AX
        0xA0, 0x00, 0x00, // MOV AL, [0000h]
        0xB4, 0x4C, // MOV AH, 4Ch
        0xCD, 0x21, // INT 21h
    ];
    let header_paragraphs: u16 = 2;
    let header_size = (header_paragraphs as usize) * 16;
    // Image: code at offset 0, data at offset 16 (segment 0x0001 relative)
    let image_size = 16 + 16 + 256; // code paragraph + data paragraph + stack
    let file_size = header_size + image_size;
    let total_pages = file_size.div_ceil(512) as u16;
    let bytes_last_page = (file_size % 512) as u16;

    let mut exe = vec![0u8; file_size];
    exe[0] = 0x4D;
    exe[1] = 0x5A;
    exe[2..4].copy_from_slice(&bytes_last_page.to_le_bytes());
    exe[4..6].copy_from_slice(&total_pages.to_le_bytes());
    exe[6..8].copy_from_slice(&1u16.to_le_bytes()); // 1 relocation
    exe[8..10].copy_from_slice(&header_paragraphs.to_le_bytes());
    exe[10..12].copy_from_slice(&0u16.to_le_bytes()); // min_alloc
    exe[12..14].copy_from_slice(&0xFFFFu16.to_le_bytes()); // max_alloc
    exe[14..16].copy_from_slice(&0u16.to_le_bytes()); // init_ss = 0
    exe[16..18].copy_from_slice(&(image_size as u16).to_le_bytes()); // init_sp
    exe[20..22].copy_from_slice(&0u16.to_le_bytes()); // init_ip = 0
    exe[22..24].copy_from_slice(&0u16.to_le_bytes()); // init_cs = 0
    exe[24..26].copy_from_slice(&(header_size as u16).to_le_bytes()); // reloc offset
    // Relocation table at offset 28 (inside header padding after the 28-byte fields)
    exe[24..26].copy_from_slice(&28u16.to_le_bytes()); // reloc_table_offset = 28
    exe[28] = 0x01; // reloc offset low (byte 1 of MOV AX, imm16)
    exe[29] = 0x00; // reloc offset high
    exe[30] = 0x00; // reloc segment low
    exe[31] = 0x00; // reloc segment high

    // Image at file offset 32
    exe[header_size..header_size + code.len()].copy_from_slice(code);
    // Data at image offset 16 (= segment 0x0001 * 16 relative to load base)
    exe[header_size + 16] = 0x42; // The byte the code reads

    let mut machine = boot_hle_with_floppy();
    machine.bus.eject_floppy(0);
    let floppy = create_test_floppy_with_program(b"TEST    EXE", &exe);
    machine.bus.insert_floppy(0, floppy, None);

    exec_file_and_check_return_code(&mut machine, b"A:\\TEST.EXE\x00", 0x0042);
}

struct McbInfo {
    owner: u16,
    size: u16,
    block_type: u8,
}

fn walk_mcb_chain(bus: &machine::Pc9801Bus, first_mcb: u16) -> Vec<McbInfo> {
    let mut blocks = Vec::new();
    let mut current = first_mcb;
    for _ in 0..4096 {
        let addr = (current as u32) << 4;
        let block_type = bus.read_byte_direct(addr);
        if block_type != 0x4D && block_type != 0x5A {
            break;
        }
        let owner = read_word(bus, addr + 1);
        let size = read_word(bus, addr + 3);
        blocks.push(McbInfo {
            owner,
            size,
            block_type,
        });
        if block_type == 0x5A {
            break;
        }
        current = current + size + 1;
    }
    blocks
}
