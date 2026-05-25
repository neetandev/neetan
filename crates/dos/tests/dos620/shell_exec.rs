use crate::harness::*;

/// Build a minimal MZ EXE that terminates with exit code 0x00.
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
    exe[0] = 0x4D; // 'M'
    exe[1] = 0x5A; // 'Z'
    exe[2..4].copy_from_slice(&bytes_last_page.to_le_bytes());
    exe[4..6].copy_from_slice(&total_pages.to_le_bytes());
    exe[6..8].copy_from_slice(&0u16.to_le_bytes()); // reloc_count
    exe[8..10].copy_from_slice(&header_paragraphs.to_le_bytes());
    exe[10..12].copy_from_slice(&0u16.to_le_bytes()); // min_alloc
    exe[12..14].copy_from_slice(&0xFFFFu16.to_le_bytes()); // max_alloc
    exe[14..16].copy_from_slice(&0u16.to_le_bytes()); // init_ss
    exe[16..18].copy_from_slice(&init_sp.to_le_bytes());
    exe[20..22].copy_from_slice(&0u16.to_le_bytes()); // init_ip
    exe[22..24].copy_from_slice(&0u16.to_le_bytes()); // init_cs
    exe[24..26].copy_from_slice(&(header_size as u16).to_le_bytes());
    exe[header_size..header_size + code.len()].copy_from_slice(code);
    exe
}

fn ascii_to_vram_chars(s: &[u8]) -> Vec<u16> {
    s.iter().map(|&b| b as u16).collect()
}

fn current_shell_psp(machine: &mut machine::Pc9801Ra) -> u16 {
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x62,                         // MOV AH, 62h
        0xCD, 0x21,                         // INT 21h
        0x89, 0x1E, 0x00, 0x01,             // MOV [0100h], BX
        0xC3,                               // RET
    ];
    inject_and_run_via_int28(machine, code, INJECT_BUDGET_DISK_IO);
    result_word(&machine.bus, 0)
}

/// Typing "install" on A: drive should find and execute INSTALL.EXE
/// without MCB corruption (error 7).
///
/// Regression test: exec_from_shell writes the filename at PSP:0108h.
/// If COMMAND.COM's MCB allocation is too small, the filename overflows
/// into the next MCB header, corrupting the chain.
#[test]
fn shell_exec_install_exe_by_name() {
    let exe_data = build_simple_exe();
    let floppy = create_test_floppy_with_program(b"INSTALL EXE", &exe_data);
    let mut machine = boot_hle_with_floppy_image(floppy);

    // Switch to drive A:
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    // Clear screen so we can check for error messages from the command
    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    // Type "install" (no extension) - shell should find INSTALL.EXE and exec it
    type_string(&mut machine.bus, b"install\r");
    run_until_prompt(&mut machine);

    // "Error loading program (7)" should NOT appear (MCB corruption)
    let error_str = ascii_to_vram_chars(b"Error");
    assert!(
        !find_string_in_text_vram(&machine.bus, &error_str),
        "exec_from_shell should not corrupt MCB chain (error 7)"
    );
}

/// Typing "install.exe" should execute the EXE, not load it as a batch file.
///
/// Regression test: the shell appends .BAT to get "INSTALL.EXE.BAT", and
/// name_to_fcb truncates that to FCB "INSTALL EXE" which matches the real
/// INSTALL.EXE. The binary is then incorrectly loaded as a batch file.
#[test]
fn shell_exec_install_exe_with_extension() {
    let exe_data = build_simple_exe();
    let floppy = create_test_floppy_with_program(b"INSTALL EXE", &exe_data);
    let mut machine = boot_hle_with_floppy_image(floppy);

    // Switch to drive A:
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    // Clear screen
    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    // Type "install.exe" (with extension)
    type_string(&mut machine.bus, b"install.exe\r");
    run_until_prompt(&mut machine);

    // "Bad command or file name" should NOT appear
    let bad_cmd = ascii_to_vram_chars(b"Bad command");
    assert!(
        !find_string_in_text_vram(&machine.bus, &bad_cmd),
        "install.exe should execute as EXE, not be loaded as batch file"
    );

    // "Error" should NOT appear either
    let error_str = ascii_to_vram_chars(b"Error");
    assert!(
        !find_string_in_text_vram(&machine.bus, &error_str),
        "install.exe should execute without errors"
    );
}

#[test]
fn shell_exec_command_starts_nested_shell_until_exit() {
    let mut machine = boot_hle_with_floppy();
    let root_psp = current_shell_psp(&mut machine);

    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"COMMAND\r");
    run_until_prompt(&mut machine);

    let child_psp = current_shell_psp(&mut machine);
    assert_ne!(
        child_psp, root_psp,
        "COMMAND should switch the active shell PSP to a child command processor"
    );
    let child_parent_psp = read_word(&machine.bus, far_to_linear(child_psp, 0) + 0x16);
    assert_eq!(
        child_parent_psp, root_psp,
        "Nested COMMAND PSP should reference the original shell as its parent"
    );

    type_string(&mut machine.bus, b"EXIT\r");
    run_until_prompt(&mut machine);

    let restored_psp = current_shell_psp(&mut machine);
    assert_eq!(
        restored_psp, root_psp,
        "EXIT should restore the original command processor PSP"
    );
}

#[test]
fn shell_exec_command_com_c_echo_returns_to_parent_shell() {
    let mut machine = boot_hle_with_floppy();

    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"A:\\COMMAND.COM /C \"ECHO CHILD\"\r");
    run_until_prompt(&mut machine);

    let child = ascii_to_vram_chars(b"CHILD");
    assert!(
        find_string_in_text_vram(&machine.bus, &child),
        "COMMAND.COM /C should execute the requested built-in command"
    );
}
