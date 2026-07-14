use crate::harness;

fn boot_and_get_environment() -> (machine_98::Pc9801Ra, u32) {
    let mut machine = harness::boot_hle();
    let psp_segment = harness::get_psp_segment(&mut machine);
    let psp_linear = harness::far_to_linear(psp_segment, 0);
    let env_segment = harness::read_word(&machine.bus, psp_linear + 0x2C);
    let env_linear = harness::far_to_linear(env_segment, 0);
    (machine, env_linear)
}

fn read_environment_strings(bus: &machine_98::Pc9801Bus, env_addr: u32) -> Vec<String> {
    let mut strings = Vec::new();
    let mut offset = 0u32;
    let max_env_size = 32768u32;

    loop {
        if offset >= max_env_size {
            break;
        }
        let byte = harness::read_byte(bus, env_addr + offset);
        if byte == 0 {
            break;
        }
        let s = harness::read_string(bus, env_addr + offset, (max_env_size - offset) as usize);
        offset += s.len() as u32 + 1;
        if let Ok(str_val) = String::from_utf8(s) {
            strings.push(str_val);
        }
    }

    strings
}

fn read_program_path_from_environment(
    bus: &machine_98::Pc9801Bus,
    env_segment: u16,
) -> (u16, String) {
    let env_addr = harness::far_to_linear(env_segment, 0);
    let mut offset = 0u32;
    let max_size = 32768u32;

    while offset < max_size {
        let byte = harness::read_byte(bus, env_addr + offset);
        if byte == 0 {
            let next = harness::read_byte(bus, env_addr + offset + 1);
            if next == 0 {
                let count_offset = offset + 2;
                let count = harness::read_word(bus, env_addr + count_offset);
                let pathname = harness::read_string(bus, env_addr + count_offset + 2, 128);
                return (count, String::from_utf8_lossy(&pathname).into_owned());
            }
        }
        offset += 1;
    }

    panic!("Could not find double-null terminator in environment block");
}

fn read_psp_command_tail(bus: &machine_98::Pc9801Bus, psp_segment: u16) -> String {
    let psp_addr = harness::far_to_linear(psp_segment, 0);
    let tail_len = harness::read_byte(bus, psp_addr + 0x80) as usize;
    let bytes = harness::read_string(bus, psp_addr + 0x81, tail_len);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn exec_load_only(
    machine: &mut machine_98::Pc9801Ra,
    filename: &[u8],
) -> (u16, u16, u16, u16, u16) {
    let base = harness::INJECT_CODE_BASE;
    harness::write_bytes(&mut machine.bus, base + 0x0200, filename);
    harness::write_bytes(&mut machine.bus, base + 0x0230, &[0x00, 0x0D]);

    let seg = harness::INJECT_CODE_SEGMENT;
    let seg_lo = (seg & 0xFF) as u8;
    let seg_hi = (seg >> 8) as u8;

    #[rustfmt::skip]
    harness::write_bytes(
        &mut machine.bus,
        base + 0x0210,
        &[
            0x00, 0x00,                             // env_seg = 0 (inherit)
            0x30, 0x02, seg_lo, seg_hi,             // cmd_tail far ptr
            0xFF, 0xFF, seg_lo, seg_hi,             // FCB1 far ptr
            0xFF, 0xFF, seg_lo, seg_hi,             // FCB2 far ptr
            0x00, 0x00, 0x00, 0x00,                 // SS:SP output
            0x00, 0x00, 0x00, 0x00,                 // CS:IP output
        ],
    );

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,                       // MOV DX, 0200h
        0xBB, 0x10, 0x02,                       // MOV BX, 0210h
        0xB8, 0x01, 0x4B,                       // MOV AX, 4B01h
        0xCD, 0x21,                             // INT 21h
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x00, 0x01,                       // MOV [0100h], AX  (flags)
        0xA1, 0x1E, 0x02,                       // MOV AX, [021Eh]
        0xA3, 0x02, 0x01,                       // MOV [0102h], AX  (SP)
        0xA1, 0x20, 0x02,                       // MOV AX, [0220h]
        0xA3, 0x04, 0x01,                       // MOV [0104h], AX  (SS)
        0xA1, 0x22, 0x02,                       // MOV AX, [0222h]
        0xA3, 0x06, 0x01,                       // MOV [0106h], AX  (IP)
        0xA1, 0x24, 0x02,                       // MOV AX, [0224h]
        0xA3, 0x08, 0x01,                       // MOV [0108h], AX  (CS)
        0xFA,                                   // CLI
        0xF4,                                   // HLT
    ];
    harness::inject_and_run_with_budget(machine, code, harness::INJECT_BUDGET_DISK_IO);

    (
        harness::result_word(&machine.bus, 0),
        harness::result_word(&machine.bus, 2),
        harness::result_word(&machine.bus, 4),
        harness::result_word(&machine.bus, 6),
        harness::result_word(&machine.bus, 8),
    )
}

fn build_simple_exe() -> Vec<u8> {
    let code: &[u8] = &[
        0xB4, 0x4C, // MOV AH, 4Ch
        0xB0, 0x00, // MOV AL, 00h
        0xCD, 0x21, // INT 21h
    ];

    let header_paragraphs: u16 = 2;
    let header_size = (header_paragraphs as usize) * 16;
    let stack_size: u16 = 256;
    let image_size = code.len() + stack_size as usize;
    let file_size = header_size + image_size;
    let total_pages = file_size.div_ceil(512) as u16;
    let bytes_last_page = (file_size % 512) as u16;
    let init_sp = (code.len() as u16) + stack_size;

    let mut exe = vec![0u8; file_size];
    exe[0] = 0x4D;
    exe[1] = 0x5A;
    exe[2..4].copy_from_slice(&bytes_last_page.to_le_bytes());
    exe[4..6].copy_from_slice(&total_pages.to_le_bytes());
    exe[6..8].copy_from_slice(&0u16.to_le_bytes());
    exe[8..10].copy_from_slice(&header_paragraphs.to_le_bytes());
    exe[10..12].copy_from_slice(&0u16.to_le_bytes());
    exe[12..14].copy_from_slice(&0xFFFFu16.to_le_bytes());
    exe[14..16].copy_from_slice(&0u16.to_le_bytes());
    exe[16..18].copy_from_slice(&init_sp.to_le_bytes());
    exe[20..22].copy_from_slice(&0u16.to_le_bytes());
    exe[22..24].copy_from_slice(&0u16.to_le_bytes());
    exe[24..26].copy_from_slice(&(header_size as u16).to_le_bytes());
    exe[header_size..header_size + code.len()].copy_from_slice(code);
    exe
}

#[test]
fn has_comspec_entry() {
    let (machine, env_addr) = boot_and_get_environment();
    let strings = read_environment_strings(&machine.bus, env_addr);
    let has_comspec = strings.iter().any(|s| s.starts_with("COMSPEC="));
    assert!(
        has_comspec,
        "Environment should contain COMSPEC= entry, found: {:?}",
        strings
    );
}

#[test]
fn has_path_entry() {
    let (machine, env_addr) = boot_and_get_environment();
    let strings = read_environment_strings(&machine.bus, env_addr);
    let has_path = strings.iter().any(|s| s.starts_with("PATH="));
    assert!(
        has_path,
        "Environment should contain PATH= entry, found: {:?}",
        strings
    );
}

#[test]
fn default_root_environment_matches_hle_contract() {
    let (machine, env_addr) = boot_and_get_environment();
    let strings = read_environment_strings(&machine.bus, env_addr);

    assert_eq!(
        strings,
        vec![
            "COMSPEC=Z:\\COMMAND.COM".to_string(),
            "CONFIG=".to_string(),
            "PATH=".to_string(),
            "PROMPT=$P$G".to_string(),
            "TEMP=".to_string(),
        ],
        "root environment strings should match the default HLE DOS contract"
    );
}

#[test]
fn double_null_terminated() {
    let (machine, env_addr) = boot_and_get_environment();
    // Walk to find the double-null terminator.
    let mut offset = 0u32;
    let max_size = 32768u32;
    let mut found_double_null = false;

    while offset < max_size {
        let byte = harness::read_byte(&machine.bus, env_addr + offset);
        if byte == 0 {
            let next = harness::read_byte(&machine.bus, env_addr + offset + 1);
            if next == 0 {
                found_double_null = true;
                break;
            }
        }
        offset += 1;
    }

    assert!(
        found_double_null,
        "Environment block should end with a double-null terminator"
    );
}

#[test]
fn program_name_after_terminator() {
    let (machine, env_addr) = boot_and_get_environment();
    // Find the double-null terminator, then check for WORD count + pathname.
    let mut offset = 0u32;
    let max_size = 32768u32;

    while offset < max_size {
        let byte = harness::read_byte(&machine.bus, env_addr + offset);
        if byte == 0 {
            let next = harness::read_byte(&machine.bus, env_addr + offset + 1);
            if next == 0 {
                // Double-null found at offset. After it: WORD count + pathname.
                let count_offset = offset + 2;
                let count = harness::read_word(&machine.bus, env_addr + count_offset);
                assert!(
                    count >= 1,
                    "Program name count after environment should be >= 1, got {}",
                    count
                );

                let pathname = harness::read_string(&machine.bus, env_addr + count_offset + 2, 128);
                assert!(
                    !pathname.is_empty(),
                    "Program pathname after environment should be non-empty"
                );
                return;
            }
        }
        offset += 1;
    }

    panic!("Could not find double-null terminator in environment block");
}

#[test]
fn prompt_entry_if_present() {
    let (machine, env_addr) = boot_and_get_environment();
    let strings = read_environment_strings(&machine.bus, env_addr);
    // PROMPT may or may not be set depending on AUTOEXEC.BAT. If present, verify it has a value.
    if let Some(prompt) = strings.iter().find(|s| s.starts_with("PROMPT=")) {
        assert!(
            prompt.len() > 7,
            "PROMPT= should have a non-empty value, got '{}'",
            prompt
        );
    }
}

#[test]
fn program_pathname_is_command_com() {
    let mut machine = harness::boot_hle();
    let psp_segment = harness::get_psp_segment(&mut machine);
    let psp_linear = harness::far_to_linear(psp_segment, 0);
    let env_segment = harness::read_word(&machine.bus, psp_linear + 0x2C);
    let env_addr = harness::far_to_linear(env_segment, 0);

    let mut offset = 0u32;
    let max_size = 32768u32;

    // Find the double-null terminator.
    while offset < max_size {
        let byte = harness::read_byte(&machine.bus, env_addr + offset);
        if byte == 0 {
            let next = harness::read_byte(&machine.bus, env_addr + offset + 1);
            if next == 0 {
                let count_offset = offset + 2;
                let count = harness::read_word(&machine.bus, env_addr + count_offset);

                // NEETAN DOS HLE sets the program name for all processes
                // including the root COMMAND.COM, for maximum compatibility
                // with programs that read it (spec section 2.6).
                assert_eq!(
                    count, 0x0001,
                    "NEETAN HLE: COMMAND.COM environment should have count=1"
                );

                let pathname = harness::read_string(&machine.bus, env_addr + count_offset + 2, 128);
                let pathname_str = String::from_utf8_lossy(&pathname);
                assert!(
                    pathname_str.contains("COMMAND.COM"),
                    "Program pathname should contain 'COMMAND.COM', got '{}'",
                    pathname_str
                );
                return;
            }
        }
        offset += 1;
    }
    panic!("Could not find double-null terminator in environment block");
}

/// EXEC a child .COM process and verify it receives its own copied environment
/// block with the child program path in the trailer.
#[test]
fn child_process_program_name() {
    use common::Bus;
    use harness::*;

    // .COM that reads its own env segment from PSP+0x2C, stores it at 0x2000:0x0180,
    // then terminates.
    #[rustfmt::skip]
    let env_reader_com: &[u8] = &[
        0xA1, 0x2C, 0x00,                  // MOV AX, [002Ch]  ; env_seg from PSP
        0xBB, 0x00, 0x20,                  // MOV BX, 2000h
        0x8E, 0xC3,                        // MOV ES, BX
        0x26, 0xA3, 0x80, 0x01,            // MOV [ES:0180h], AX
        0xB4, 0x4C,                        // MOV AH, 4Ch
        0x30, 0xC0,                        // XOR AL, AL
        0xCD, 0x21,                        // INT 21h
    ];

    let mut machine = boot_hle_with_floppy();
    let parent_psp_segment = get_psp_segment(&mut machine);
    let parent_psp_linear = far_to_linear(parent_psp_segment, 0);
    let parent_env_seg = read_word(&machine.bus, parent_psp_linear + 0x2C);
    machine.bus.eject_floppy(0);
    let floppy = create_test_floppy_with_program(b"TEST    COM", env_reader_com);
    machine.bus.insert_floppy(0, floppy, None);

    let base = INJECT_CODE_BASE;
    let seg = INJECT_CODE_SEGMENT;

    // Filename at +0x0200
    write_bytes(&mut machine.bus, base + 0x0200, b"A:\\TEST.COM\x00");

    // Command tail at +0x0220: length=0, CR
    machine.bus.write_byte(base + 0x0220, 0x00);
    machine.bus.write_byte(base + 0x0221, 0x0D);

    // EXEC parameter block at +0x0210 (env_seg=0 -> inherit parent)
    let seg_lo = (seg & 0xFF) as u8;
    let seg_hi = (seg >> 8) as u8;
    #[rustfmt::skip]
    let param_block: [u8; 14] = [
        0x00, 0x00,                         // env_seg = 0 (inherit)
        0x20, 0x02, seg_lo, seg_hi,         // cmd_tail = seg:0220
        0x30, 0x02, seg_lo, seg_hi,         // fcb1 = seg:0230
        0x40, 0x02, seg_lo, seg_hi,         // fcb2 = seg:0240
    ];
    write_bytes(&mut machine.bus, base + 0x0210, &param_block);

    const SEG_LO: u8 = (INJECT_CODE_SEGMENT & 0xFF) as u8;
    const SEG_HI: u8 = (INJECT_CODE_SEGMENT >> 8) as u8;

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,                  // MOV DX, 0200h (filename)
        0xBB, 0x10, 0x02,                  // MOV BX, 0210h (param block)
        0xB8, 0x00, 0x4B,                  // MOV AX, 4B00h (EXEC)
        0xCD, 0x21,                        // INT 21h
        // Restore DS after EXEC (destroys all regs except SS:SP).
        0xB8, SEG_LO, SEG_HI,              // MOV AX, INJECT_CODE_SEGMENT
        0x8E, 0xD8,                        // MOV DS, AX
        0xFA,                              // CLI
        0xF4,                              // HLT
    ];

    inject_and_run_with_budget(&mut machine, code, INJECT_BUDGET_DISK_IO);

    // The child .COM stored its env segment at 0x2000:0x0180.
    let child_env_seg = harness::read_word(&machine.bus, INJECT_CODE_BASE + 0x0180);
    assert!(
        child_env_seg > 0,
        "Child env segment should be non-zero, got {:#06X}",
        child_env_seg
    );
    assert_ne!(
        child_env_seg, parent_env_seg,
        "Child should receive a copied environment block, not reuse the parent one"
    );

    let (count, pathname) = read_program_path_from_environment(&machine.bus, child_env_seg);
    assert_eq!(
        count, 0x0001,
        "Child environment trailer should have count=1"
    );
    assert_eq!(pathname, "A:\\TEST.COM");
}

#[test]
fn shell_config_sets_root_environment_contract() {
    let floppy = harness::create_test_floppy_with_config_and_autoexec(
        b"SHELL = A:COMMAND.COM A: /E:384 /P\r\n",
        b"@ECHO OFF\r\n",
    );
    let mut machine = harness::boot_hle_with_forced_os(Some(floppy), None);
    let psp_segment = harness::get_psp_segment(&mut machine);
    let psp_linear = harness::far_to_linear(psp_segment, 0);
    let env_segment = harness::read_word(&machine.bus, psp_linear + 0x2C);
    let env_linear = harness::far_to_linear(env_segment, 0);

    let strings = read_environment_strings(&machine.bus, env_linear);
    assert!(
        strings.iter().any(|s| s == "COMSPEC=A:COMMAND.COM"),
        "COMSPEC should follow CONFIG.SYS SHELL=, found {:?}",
        strings
    );

    let (count, pathname) = read_program_path_from_environment(&machine.bus, env_segment);
    assert_eq!(
        count, 1,
        "Root environment should store one program pathname"
    );
    assert_eq!(
        pathname, "A:COMMAND.COM",
        "Root environment pathname should follow CONFIG.SYS SHELL="
    );

    let command_tail = read_psp_command_tail(&machine.bus, psp_segment);
    assert_eq!(
        command_tail, "A: /E:384 /P",
        "Root PSP command tail should preserve the SHELL= argument tail"
    );

    let sysvars = harness::get_sysvars_address(&mut machine);
    let first_mcb_segment = harness::read_word(&machine.bus, sysvars - 2);
    let first_mcb_addr = harness::far_to_linear(first_mcb_segment, 0);
    let env_mcb_size = harness::read_word(&machine.bus, first_mcb_addr + 3);
    assert_eq!(
        env_mcb_size, 24,
        "Root environment MCB should honor /E:384 as 24 paragraphs"
    );
}

#[test]
fn exec_load_only_com_uses_child_program_path() {
    let mut machine = harness::boot_hle_with_floppy();
    let parent_psp_segment = harness::get_psp_segment(&mut machine);
    let parent_psp_linear = harness::far_to_linear(parent_psp_segment, 0);
    let parent_env_seg = harness::read_word(&machine.bus, parent_psp_linear + 0x2C);

    let (flags, _sp, _ss, ip, cs) = exec_load_only(&mut machine, b"A:\\TEST.COM\0");
    assert_eq!(flags & 1, 0, "AX=4B01h for COM should return CF=0");
    assert_eq!(ip, 0x0100, "COM load-only should return entry IP 0100h");

    let child_psp = cs;
    let child_psp_linear = harness::far_to_linear(child_psp, 0);
    let child_env_seg = harness::read_word(&machine.bus, child_psp_linear + 0x2C);
    assert_ne!(
        child_env_seg, parent_env_seg,
        "Load-only COM should allocate a separate child environment"
    );
    let child_env_mcb = child_env_seg.wrapping_sub(1);
    let child_env_size = harness::read_word(&machine.bus, harness::far_to_linear(child_env_mcb, 3));
    assert_eq!(
        child_env_size, 5,
        "Child environment should be compact, got {child_env_size} paragraphs"
    );

    let (count, pathname) = read_program_path_from_environment(&machine.bus, child_env_seg);
    assert_eq!(
        count, 0x0001,
        "Child environment trailer should have count=1"
    );
    assert_eq!(pathname, "A:\\TEST.COM");
}

#[test]
fn exec_load_only_exe_uses_child_program_path() {
    let exe_data = build_simple_exe();
    let floppy = harness::create_test_floppy_with_program(b"LOAD    EXE", &exe_data);
    let mut machine = harness::boot_hle_with_floppy_image(floppy);
    let parent_psp_segment = harness::get_psp_segment(&mut machine);
    let parent_psp_linear = harness::far_to_linear(parent_psp_segment, 0);
    let parent_env_seg = harness::read_word(&machine.bus, parent_psp_linear + 0x2C);

    let (flags, _sp, _ss, ip, cs) = exec_load_only(&mut machine, b"A:\\LOAD.EXE\0");
    assert_eq!(flags & 1, 0, "AX=4B01h for EXE should return CF=0");
    assert_eq!(
        ip, 0x0000,
        "EXE load-only should return the header entry IP"
    );

    let child_psp = cs.wrapping_sub(0x0010);
    let child_psp_linear = harness::far_to_linear(child_psp, 0);
    let child_env_seg = harness::read_word(&machine.bus, child_psp_linear + 0x2C);
    assert_ne!(
        child_env_seg, parent_env_seg,
        "Load-only EXE should allocate a separate child environment"
    );

    let (count, pathname) = read_program_path_from_environment(&machine.bus, child_env_seg);
    assert_eq!(
        count, 0x0001,
        "Child environment trailer should have count=1"
    );
    assert_eq!(pathname, "A:\\LOAD.EXE");
}
