use std::fs;

use crate::{file_copy_harness::*, harness::*};

fn boot_with_batch_file(batch_content: &[u8]) -> machine::Pc9801Ra {
    let floppy_image = create_test_floppy_with_program(b"TEST    BAT", batch_content);
    boot_hle_with_floppy_image(floppy_image)
}

fn submit_command(machine: &mut machine::Pc9801Ra, command: &[u8]) {
    type_string(&mut machine.bus, command);
    run_until_prompt(machine);
}

fn submit_long_command(machine: &mut machine::Pc9801Ra, command: &[u8]) {
    type_string_long(machine, command);
    run_until_prompt(machine);
}

fn format_drive_a(machine: &mut machine::Pc9801Ra) {
    type_string_long(machine, b"FORMAT A:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(machine);
}

fn prepare_test_batch(machine: &mut machine::Pc9801Ra) {
    submit_command(machine, b"A:\r");
    submit_command(machine, b"CLS\r");
}

fn run_batch_command(mut machine: machine::Pc9801Ra, command: &[u8]) -> machine::Pc9801Ra {
    prepare_test_batch(&mut machine);
    submit_command(&mut machine, command);
    machine
}

fn run_test_batch(batch_content: &[u8]) -> machine::Pc9801Ra {
    run_batch_command(boot_with_batch_file(batch_content), b"TEST\r")
}

fn run_test_batch_with_long_command(batch_content: &[u8], command: &[u8]) -> machine::Pc9801Ra {
    let mut machine = boot_with_batch_file(batch_content);
    prepare_test_batch(&mut machine);
    submit_long_command(&mut machine, command);
    machine
}

fn run_test_batch_with_two_floppy_images(
    first_floppy_image: device::floppy::FloppyImage,
    second_floppy_image: device::floppy::FloppyImage,
) -> machine::Pc9801Ra {
    run_batch_command(
        boot_hle_with_two_floppy_images(first_floppy_image, second_floppy_image),
        b"TEST\r",
    )
}

fn screen_contains(machine: &machine::Pc9801Ra, text: &str) -> bool {
    assert!(text.is_ascii(), "screen text helper only supports ASCII");
    let characters = text.bytes().map(u16::from).collect::<Vec<_>>();
    find_string_in_text_vram(&machine.bus, &characters)
}

fn assert_screen_contains(machine: &machine::Pc9801Ra, text: &str, message: &str) {
    assert!(screen_contains(machine, text), "{message}");
}

fn assert_screen_lacks(machine: &machine::Pc9801Ra, text: &str, message: &str) {
    assert!(!screen_contains(machine, text), "{message}");
}

fn run_until_screen_contains(machine: &mut machine::Pc9801Ra, text: &str, message: &str) {
    let max_cycles: u64 = 500_000_000;
    let check_interval: u64 = 100_000;
    let mut total_cycles = 0u64;
    loop {
        total_cycles += machine.run_for(check_interval);
        if screen_contains(machine, text) {
            break;
        }
        assert!(total_cycles < max_cycles, "{message}");
    }
}

fn current_shell_program_segment_prefix(machine: &mut machine::Pc9801Ra) -> u16 {
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

#[test]
fn batch_echo() {
    let machine = run_test_batch(b"ECHO BATCH OUTPUT\r\n");
    assert_screen_contains(
        &machine,
        "BATCH OUTPUT",
        "batch file should display 'BATCH OUTPUT'",
    );
}

#[test]
fn batch_echoed_command_includes_prompt() {
    let machine = run_test_batch(b"ECHO BATCH OUTPUT\r\n");
    assert_screen_contains(
        &machine,
        "A:\\>ECHO BATCH OUTPUT",
        "echoed batch command should include the active prompt",
    );
}

#[test]
fn batch_multi_line() {
    let machine = run_test_batch(b"ECHO ALPHA\r\nECHO BETA\r\n");
    assert_screen_contains(&machine, "ALPHA", "batch should display 'ALPHA'");
    assert_screen_contains(&machine, "BETA", "batch should display 'BETA'");
}

#[test]
fn batch_echo_off() {
    let machine = run_test_batch(b"@ECHO OFF\r\nECHO QUIET\r\n");
    assert_screen_contains(&machine, "QUIET", "batch should display 'QUIET'");
}

#[test]
fn batch_echo_backslash_prints_blank_line() {
    let machine = run_test_batch(b"ECHO\\\r\nECHO AFTER\r\n");
    assert_screen_contains(&machine, "AFTER", "batch should continue after ECHO\\");
    assert_screen_lacks(
        &machine,
        "Bad command",
        "ECHO\\ should be accepted as an ECHO command",
    );
}

#[test]
fn batch_goto() {
    let machine = run_test_batch(b"GOTO SKIP\r\nECHO BAD\r\n:SKIP\r\nECHO GOOD\r\n");
    assert_screen_contains(
        &machine,
        "GOOD",
        "GOTO should skip to label and display 'GOOD'",
    );
    assert_screen_lacks(&machine, "BAD", "'BAD' should not be displayed after GOTO");
}

#[test]
fn batch_goto_accepts_colon_and_label_comments() {
    let machine = run_test_batch(
        b"GOTO :SKIP trailing note\r\nECHO BAD\r\n:SKIP target label\r\nECHO GOOD\r\n",
    );
    assert_screen_contains(
        &machine,
        "GOOD",
        "GOTO should accept a leading colon and ignore text after the label",
    );
    assert_screen_lacks(&machine, "BAD", "GOTO should skip the false branch");
}

#[test]
fn batch_goto_missing_label_reports_error() {
    let machine = run_test_batch(b"GOTO NOWHERE\r\nECHO BAD\r\n");
    assert_screen_contains(
        &machine,
        "Label not found",
        "missing GOTO labels should report an error",
    );
    assert_screen_lacks(&machine, "BAD", "batch should stop after a missing label");
}

#[test]
fn batch_if_exist() {
    let machine = run_test_batch(b"IF EXIST TESTFILE.TXT ECHO FOUND\r\n");
    assert_screen_contains(
        &machine,
        "FOUND",
        "IF EXIST should detect TESTFILE.TXT and display 'FOUND'",
    );
}

#[test]
fn batch_if_exist_false_skips_command() {
    let machine = run_test_batch(b"@ECHO OFF\r\nIF EXIST NOFILE.TXT ECHO BAD\r\nECHO AFTER\r\n");
    assert_screen_contains(
        &machine,
        "AFTER",
        "false IF EXIST should continue with the next line",
    );
    assert_screen_lacks(
        &machine,
        "BAD",
        "false IF EXIST should skip the conditional command",
    );
}

#[test]
fn batch_if_not_exist() {
    let machine = run_test_batch(b"IF NOT EXIST NOFILE.TXT ECHO MISSING\r\n");
    assert_screen_contains(
        &machine,
        "MISSING",
        "IF NOT EXIST should display 'MISSING' for nonexistent file",
    );
}

#[test]
fn batch_if_errorlevel() {
    let machine = run_test_batch(b"NOSUCHCMD\r\nIF ERRORLEVEL 1 ECHO ERROR\r\n");
    assert_screen_contains(
        &machine,
        "ERROR",
        "IF ERRORLEVEL should detect non-zero exit code and display 'ERROR'",
    );
}

#[test]
fn batch_if_not_errorlevel_equals_with_spaces() {
    let machine = run_test_batch(
        b"NOSUCHCMD\r\nIF NOT ERRORLEVEL == 2 ECHO BELOW\r\nIF ERRORLEVEL == 1 ECHO HIT\r\n",
    );
    assert_screen_contains(
        &machine,
        "BELOW",
        "IF NOT ERRORLEVEL == n should accept spaces around ==",
    );
    assert_screen_contains(
        &machine,
        "HIT",
        "IF ERRORLEVEL == n should accept spaces around ==",
    );
}

#[test]
fn batch_if_errorlevel_equals_is_exact() {
    let first_floppy_image = create_test_floppy_with_program(
        b"TEST    BAT",
        b"@ECHO OFF\r\nB:\\RUNME\r\nIF ERRORLEVEL==65 ECHO BAD\r\nIF ERRORLEVEL==66 ECHO EXITOK\r\n",
    );
    let second_floppy_image = create_test_floppy_with_program(b"RUNME   COM", TEST_COM_PROGRAM);
    let machine = run_test_batch_with_two_floppy_images(first_floppy_image, second_floppy_image);
    assert_screen_contains(
        &machine,
        "EXITOK",
        "IF ERRORLEVEL==n should match an exact return code",
    );
    assert_screen_lacks(
        &machine,
        "BAD",
        "IF ERRORLEVEL==n should not use threshold matching",
    );
}

#[test]
fn batch_if_string_compare() {
    let machine = run_test_batch_with_long_command(
        b"@ECHO OFF\r\nIF \"%1\"==\"FOO\" ECHO MATCH\r\nIF \"%2\"==\"NOPE\" ECHO BAD\r\n",
        b"TEST FOO BAR\r",
    );
    assert_screen_contains(
        &machine,
        "MATCH",
        "IF string comparison should run the matching command",
    );
    assert_screen_lacks(
        &machine,
        "BAD",
        "IF string comparison should skip non-matching commands",
    );
}

#[test]
fn batch_if_not_string_compare() {
    let machine = run_test_batch_with_long_command(
        b"IF NOT \"%1\"==\"BAR\" ECHO MISMATCH\r\n",
        b"TEST FOO\r",
    );
    assert_screen_contains(
        &machine,
        "MISMATCH",
        "IF NOT string comparison should invert the condition",
    );
}

#[test]
fn batch_if_errorlevel_after_external_program() {
    let first_floppy_image = create_test_floppy_with_program(
        b"TEST    BAT",
        b"B:\\RUNME\r\nIF ERRORLEVEL 66 ECHO EXITOK\r\n",
    );
    let second_floppy_image = create_test_floppy_with_program(b"RUNME   COM", TEST_COM_PROGRAM);
    let machine = run_test_batch_with_two_floppy_images(first_floppy_image, second_floppy_image);
    assert_screen_contains(
        &machine,
        "EXITOK",
        "batch should wait for external programs before evaluating following lines",
    );
}

#[test]
fn batch_echo_preserves_errorlevel_after_external_program() {
    let first_floppy_image = create_test_floppy_with_program(
        b"TEST    BAT",
        b"B:\\RUNME\r\nECHO.\r\nIF ERRORLEVEL 66 ECHO EXITOK\r\n",
    );
    let second_floppy_image = create_test_floppy_with_program(b"RUNME   COM", TEST_COM_PROGRAM);
    let machine = run_test_batch_with_two_floppy_images(first_floppy_image, second_floppy_image);
    assert_screen_contains(
        &machine,
        "EXITOK",
        "ECHO should not clear ERRORLEVEL from an external program",
    );
}

#[test]
fn batch_if_errorlevel_equals_goto_after_external_program() {
    let first_floppy_image = create_test_floppy_with_program(
        b"TEST    BAT",
        b"B:\\RUNME\r\nIF ERRORLEVEL==66 GOTO EXITOK   66=EXTERNAL\r\nECHO BAD\r\n:EXITOK\r\nECHO EXITOK\r\n",
    );
    let second_floppy_image = create_test_floppy_with_program(b"RUNME   COM", TEST_COM_PROGRAM);
    let machine = run_test_batch_with_two_floppy_images(first_floppy_image, second_floppy_image);
    assert_screen_contains(
        &machine,
        "EXITOK",
        "IF ERRORLEVEL==n should jump to a batch label after an external program",
    );
    assert_screen_lacks(&machine, "BAD", "inline GOTO should skip the false branch");
}

#[test]
fn batch_regular_command_supports_inline_output_redirection() {
    let machine = run_test_batch(b"@ECHO OFF\r\nECHO HIDDEN>NUL\r\nECHO VISIBLE\r\n");
    assert_screen_contains(
        &machine,
        "VISIBLE",
        "batch should continue after a redirected command",
    );
    assert_screen_lacks(
        &machine,
        "HIDDEN",
        "regular batch commands should parse inline output redirection",
    );
}

#[test]
fn batch_external_command_supports_inline_output_redirection() {
    let first_floppy_image = create_test_floppy_with_program(
        b"TEST    BAT",
        b"@ECHO OFF\r\nB:\\RUNME.COM>NUL\r\nIF ERRORLEVEL 66 ECHO EXITOK\r\n",
    );
    let second_floppy_image = create_test_floppy_with_program(b"RUNME   COM", TEST_COM_PROGRAM);
    let machine = run_test_batch_with_two_floppy_images(first_floppy_image, second_floppy_image);
    assert_screen_contains(
        &machine,
        "EXITOK",
        "external batch commands should parse inline output redirection",
    );
    assert_screen_lacks(
        &machine,
        "Bad command",
        "inline output redirection must not be part of the executable name",
    );
}

#[test]
fn batch_external_command_output_redirects_to_nul() {
    #[rustfmt::skip]
    let print_and_exit_com: &[u8] = &[
        0xBA, 0x0C, 0x01,       // MOV DX, 010Ch
        0xB4, 0x09,             // MOV AH, 09h
        0xCD, 0x21,             // INT 21h
        0xB8, 0x42, 0x4C,       // MOV AX, 4C42h
        0xCD, 0x21,             // INT 21h
        b'L', b'O', b'U', b'D', b'$',
    ];
    let first_floppy_image = create_test_floppy_with_program(
        b"TEST    BAT",
        b"@ECHO OFF\r\nB:\\RUNME.COM>NUL\r\nIF ERRORLEVEL 66 ECHO EXITOK\r\n",
    );
    let second_floppy_image = create_test_floppy_with_program(b"RUNME   COM", print_and_exit_com);
    let machine = run_test_batch_with_two_floppy_images(first_floppy_image, second_floppy_image);
    assert_screen_contains(
        &machine,
        "EXITOK",
        "redirected external command should still return its exit code",
    );
    assert_screen_lacks(
        &machine,
        "LOUD",
        "external program stdout should be discarded by >NUL",
    );
}

#[test]
fn batch_external_command_output_redirect_restores_stdout_for_next_external_command() {
    #[rustfmt::skip]
    let print_and_exit_com: &[u8] = &[
        0xBA, 0x0C, 0x01,       // MOV DX, 010Ch
        0xB4, 0x09,             // MOV AH, 09h
        0xCD, 0x21,             // INT 21h
        0xB8, 0x42, 0x4C,       // MOV AX, 4C42h
        0xCD, 0x21,             // INT 21h
        b'L', b'O', b'U', b'D', b'$',
    ];
    let first_floppy_image = create_test_floppy_with_program(
        b"TEST    BAT",
        b"@ECHO OFF\r\nB:\\RUNME.COM>NUL\r\nB:\\RUNME.COM\r\n",
    );
    let second_floppy_image = create_test_floppy_with_program(b"RUNME   COM", print_and_exit_com);
    let machine = run_test_batch_with_two_floppy_images(first_floppy_image, second_floppy_image);
    assert_screen_contains(
        &machine,
        "LOUD",
        "external command stdout should be restored after a preceding >NUL child",
    );
}

#[test]
fn batch_if_can_run_external_program() {
    let first_floppy_image = create_test_floppy_with_program(
        b"TEST    BAT",
        b"IF EXIST TESTFILE.TXT B:\\RUNME\r\nIF ERRORLEVEL 66 ECHO EXITOK\r\n",
    );
    let second_floppy_image = create_test_floppy_with_program(b"RUNME   COM", TEST_COM_PROGRAM);
    let machine = run_test_batch_with_two_floppy_images(first_floppy_image, second_floppy_image);
    assert_screen_contains(
        &machine,
        "EXITOK",
        "IF should wait for external programs before continuing",
    );
}

#[test]
fn batch_if_can_replace_with_another_batch() {
    let first_floppy_image = create_test_floppy_with_program(
        b"TEST    BAT",
        b"IF EXIST B:\\CHILD.BAT B:\\CHILD\r\nECHO BAD\r\n",
    );
    let second_floppy_image = create_test_floppy_with_program(b"CHILD   BAT", b"ECHO CHILD\r\n");
    let machine = run_test_batch_with_two_floppy_images(first_floppy_image, second_floppy_image);
    assert_screen_contains(
        &machine,
        "CHILD",
        "IF should be able to replace the current batch with another batch",
    );
    assert_screen_lacks(
        &machine,
        "BAD",
        "batch replacement should not return to the parent without CALL",
    );
}

#[test]
fn batch_replaced_child_inherits_echo_off() {
    let first_floppy_image =
        create_test_floppy_with_program(b"TEST    BAT", b"@ECHO OFF\r\nB:\\CHILD\r\n");
    let second_floppy_image =
        create_test_floppy_with_program(b"CHILD   BAT", b"ECHO OFF\r\nECHO CHILD\r\n");
    let machine = run_test_batch_with_two_floppy_images(first_floppy_image, second_floppy_image);
    assert_screen_contains(&machine, "CHILD", "child batch should run");
    assert_screen_lacks(
        &machine,
        "ECHO OFF",
        "child batch should inherit ECHO OFF from its parent",
    );
}

#[test]
fn batch_call_runs_child_and_returns_to_parent_batch() {
    let first_floppy_image =
        create_test_floppy_with_program(b"TEST    BAT", b"CALL B:\\CHILD ARG\r\nECHO PARENT\r\n");
    let second_floppy_image = create_test_floppy_with_program(b"CHILD   BAT", b"ECHO CHILD %1\r\n");
    let machine = run_test_batch_with_two_floppy_images(first_floppy_image, second_floppy_image);
    assert_screen_contains(&machine, "CHILD", "CALL should run the child batch");
    assert_screen_contains(
        &machine,
        "ARG",
        "CALL should pass arguments to the child batch",
    );
    assert_screen_contains(&machine, "PARENT", "CALL should return to the parent batch");
}

#[test]
fn batch_call_missing_child_reports_error_and_continues() {
    let machine = run_test_batch(b"CALL NOFILE\r\nECHO AFTER\r\n");
    assert_screen_contains(
        &machine,
        "Batch file not found",
        "CALL should report a missing batch file",
    );
    assert_screen_contains(
        &machine,
        "AFTER",
        "CALL should continue after a missing child batch",
    );
}

#[test]
fn batch_if_can_run_immediate_shell_command() {
    let machine = run_test_batch(b"IF EXIST TESTFILE.TXT A:\r\nECHO AFTER\r\n");
    assert_screen_contains(
        &machine,
        "AFTER",
        "IF should advance after immediate shell commands",
    );
}

#[test]
fn batch_if_command_supports_redirection() {
    let machine = run_test_batch(b"IF EXIST TESTFILE.TXT ECHO REDIR > OUT.TXT\r\nTYPE OUT.TXT\r\n");
    assert_screen_contains(
        &machine,
        "REDIR",
        "IF command output redirection should be written and readable",
    );
}

#[test]
fn batch_if_unknown_condition_continues() {
    let machine = run_test_batch(b"@ECHO OFF\r\nIF UNKNOWN ECHO BAD\r\nECHO AFTER\r\n");
    assert_screen_contains(
        &machine,
        "AFTER",
        "unknown IF conditions should continue with the next line",
    );
    assert_screen_lacks(
        &machine,
        "BAD",
        "unknown IF conditions should not run their command",
    );
}

#[test]
fn batch_params() {
    let machine = run_test_batch_with_long_command(b"ECHO %1 %2\r\n", b"TEST FOO BAR\r");
    assert_screen_contains(&machine, "FOO", "batch params should substitute 'FOO'");
    assert_screen_contains(&machine, "BAR", "batch params should substitute 'BAR'");
}

#[test]
fn batch_env_var() {
    let machine = run_test_batch(b"ECHO %COMSPEC%\r\n");
    assert_screen_contains(
        &machine,
        "COMMAND",
        "batch %COMSPEC% should display COMMAND.COM path",
    );
}

#[test]
fn batch_rem() {
    let machine = run_test_batch(b"REM This is a comment\r\nECHO VISIBLE\r\n");
    assert_screen_contains(
        &machine,
        "VISIBLE",
        "batch should display 'VISIBLE' after REM",
    );
}

#[test]
fn batch_pause() {
    let mut machine = boot_with_batch_file(b"ECHO BEFORE\r\nPAUSE\r\nECHO AFTER\r\n");
    prepare_test_batch(&mut machine);
    type_string(&mut machine.bus, b"TEST\r");

    run_until_screen_contains(
        &mut machine,
        "Press",
        "PAUSE should display 'Press any key'",
    );
    submit_command(&mut machine, b" ");

    assert_screen_contains(
        &machine,
        "AFTER",
        "batch should continue after PAUSE and display 'AFTER'",
    );
}

#[test]
fn batch_drive_change_applies_before_following_lines() {
    let first_floppy_image = create_test_floppy_with_program(
        b"SWITCH  BAT",
        b"B:\r\nIF EXIST TARGET.TXT ECHO FOUND\r\n",
    );
    let second_floppy_image = create_test_floppy_with_program(b"TARGET  TXT", b"B-ONLY\r\n");
    let machine = run_batch_command(
        boot_hle_with_two_floppy_images(first_floppy_image, second_floppy_image),
        b"SWITCH\r",
    );
    assert_screen_contains(
        &machine,
        "FOUND",
        "batch drive change should affect relative paths used by following lines",
    );
}

#[test]
fn batch_if_exist_sees_swapped_floppy_after_pause() {
    let first_floppy_image = create_test_floppy_with_program(
        b"SWAP    BAT",
        b"@ECHO OFF\r\nPAUSE\r\nIF EXIST A:\\ADISK.DAT ECHO FOUND\r\nIF NOT EXIST A:\\ADISK.DAT ECHO MISSING\r\n",
    );
    let second_floppy_image = create_test_floppy_with_program(b"ADISK   DAT", b"SECOND\r\n");
    let mut machine = boot_hle_with_floppy_image(first_floppy_image);

    prepare_test_batch(&mut machine);
    type_string(&mut machine.bus, b"SWAP\r");
    run_until_screen_contains(
        &mut machine,
        "Press",
        "PAUSE should display before the floppy swap",
    );

    machine.bus.insert_floppy(0, second_floppy_image, None);
    submit_command(&mut machine, b" ");

    assert_screen_contains(
        &machine,
        "FOUND",
        "IF EXIST should see files on the newly inserted floppy",
    );
    assert_screen_lacks(
        &machine,
        "MISSING",
        "IF NOT EXIST should not match files on the newly inserted floppy",
    );
}

#[test]
fn batch_if_exist_sees_swapped_floppy_on_drive_c_with_hdd_present() {
    let first_floppy_image = create_test_floppy_with_program(
        b"SWAP    BAT",
        b"@ECHO OFF\r\nPAUSE\r\nIF EXIST C:\\ADISK.DAT ECHO FOUND\r\nIF NOT EXIST C:\\ADISK.DAT ECHO MISSING\r\n",
    );
    let second_floppy_image = create_test_floppy_with_program(b"ADISK   DAT", b"SECOND\r\n");
    let hdd_path = make_temp_hdd_path("batch-swap-c");
    fs::write(&hdd_path, create_empty_hdd(256).to_bytes()).expect("write temp HDD image");
    let mut machine = boot_hle_with_temp_hdd_and_floppy(&hdd_path, first_floppy_image);

    submit_command(&mut machine, b"C:\r");
    submit_command(&mut machine, b"CLS\r");
    type_string(&mut machine.bus, b"SWAP\r");
    run_until_screen_contains(
        &mut machine,
        "Press",
        "PAUSE should display before the C: floppy swap",
    );

    machine.bus.insert_floppy(0, second_floppy_image, None);
    submit_command(&mut machine, b" ");

    assert_screen_contains(
        &machine,
        "FOUND",
        "IF EXIST should see files on the newly inserted C: floppy",
    );
    assert_screen_lacks(
        &machine,
        "MISSING",
        "IF NOT EXIST should not match files on the newly inserted C: floppy",
    );
}

#[test]
fn batch_copy_wildcard_from_swapped_drive_c_to_hdd_directory() {
    let first_floppy_image = create_test_floppy_with_program(
        b"SWAP    BAT",
        b"@ECHO OFF\r\nPAUSE\r\nCOPY C:\\*.* A:\\ALI_ONLY > NUL\r\nIF EXIST A:\\ALI_ONLY\\ADISK.DAT ECHO COPIED\r\nIF NOT EXIST A:\\ALI_ONLY\\ADISK.DAT ECHO MISSING\r\n",
    );
    let second_floppy_image = create_test_floppy_with_program(b"ADISK   DAT", b"SECOND\r\n");
    let hdd_path = make_temp_hdd_path("batch-copy-c-wildcard");
    fs::write(&hdd_path, create_empty_hdd(256).to_bytes()).expect("write temp HDD image");
    let mut machine = boot_hle_with_temp_hdd_and_floppy(&hdd_path, first_floppy_image);

    format_drive_a(&mut machine);
    submit_command(&mut machine, b"MD A:\\ALI_ONLY\r");
    submit_command(&mut machine, b"C:\r");
    submit_command(&mut machine, b"CLS\r");
    type_string(&mut machine.bus, b"SWAP\r");
    run_until_screen_contains(
        &mut machine,
        "Press",
        "PAUSE should display before wildcard COPY from C:",
    );

    machine.bus.insert_floppy(0, second_floppy_image, None);
    submit_command(&mut machine, b" ");

    assert_screen_contains(
        &machine,
        "COPIED",
        "COPY C:\\*.* A:\\ALI_ONLY should copy files from the newly inserted C: floppy",
    );
    assert_screen_lacks(
        &machine,
        "MISSING",
        "destination file check should not fail after wildcard COPY from C:",
    );
}

#[test]
fn batch_command_c_resumes_parent_batch() {
    let machine = run_test_batch(b"COMMAND /C ECHO CHILD\r\nECHO PARENT\r\n");
    assert_screen_contains(
        &machine,
        "CHILD",
        "COMMAND /C inside a batch file should execute the child command",
    );
    assert_screen_contains(
        &machine,
        "PARENT",
        "batch execution should resume after COMMAND /C returns",
    );
}

#[test]
fn batch_command_c_preserves_external_program_errorlevel() {
    let machine = run_test_batch(b"COMMAND /C A:\\TEST.COM\r\nIF ERRORLEVEL 66 ECHO EXITOK\r\n");
    assert_screen_contains(
        &machine,
        "EXITOK",
        "COMMAND /C should preserve the external program exit code for the parent batch",
    );
}

#[test]
fn batch_command_c_bomb_reports_oom_and_returns_to_root_shell() {
    let mut machine = boot_with_batch_file(b"COMMAND /C TEST\r\n");
    let root_program_segment_prefix = current_shell_program_segment_prefix(&mut machine);

    prepare_test_batch(&mut machine);
    submit_command(&mut machine, b"TEST\r");

    assert_screen_contains(
        &machine,
        "Error loading program (8)",
        "Recursive COMMAND /C should stop with insufficient-memory error",
    );

    let restored_program_segment_prefix = current_shell_program_segment_prefix(&mut machine);
    assert_eq!(
        restored_program_segment_prefix, root_program_segment_prefix,
        "COMMAND /C bomb should unwind back to the root shell after OOM"
    );

    submit_command(&mut machine, b"VER\r");
    assert_screen_contains(
        &machine,
        "6.20",
        "Root shell should still accept commands after recursive COMMAND /C OOM",
    );
}
