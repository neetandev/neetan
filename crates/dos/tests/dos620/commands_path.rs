use crate::harness::*;

fn submit(machine: &mut machine::Pc9801Ra, command: &[u8]) {
    type_string_long(machine, command);
    run_until_prompt(machine);
}

fn assert_screen_contains(machine: &machine::Pc9801Ra, text: &str, message: &str) {
    assert!(
        find_row_containing(&machine.bus, text).is_some(),
        "{message}"
    );
}

fn switch_to_a(machine: &mut machine::Pc9801Ra) {
    submit(machine, b"A:\r");
}

fn print_message_com(message: &[u8]) -> Vec<u8> {
    let message_offset = 0x010C_u16;
    let mut program = vec![
        0xB4,
        0x09, // MOV AH, 09h
        0xBA,
        (message_offset & 0x00FF) as u8,
        (message_offset >> 8) as u8, // MOV DX, message
        0xCD,
        0x21, // INT 21h
        0xB8,
        0x00,
        0x4C, // MOV AX, 4C00h
        0xCD,
        0x21, // INT 21h
    ];
    program.extend_from_slice(message);
    program.push(b'$');
    program
}

#[test]
fn path_displays_default_empty_path() {
    let mut machine = boot_hle();

    submit(&mut machine, b"CLS\r");
    submit(&mut machine, b"PATH\r");

    assert_screen_contains(
        &machine,
        "No Path",
        "default empty PATH should show No Path",
    );
}

#[test]
fn path_sets_and_displays_value() {
    let mut machine = boot_hle_with_floppy();
    switch_to_a(&mut machine);

    submit(&mut machine, b"PATH A:\\BIN;Z:\\TOOLS\r");
    submit(&mut machine, b"CLS\r");
    submit(&mut machine, b"PATH\r");

    assert_screen_contains(
        &machine,
        "PATH=A:\\BIN;Z:\\TOOLS",
        "PATH should display the configured search path",
    );

    submit(&mut machine, b"ECHO ECHO %PATH% > SHOWPATH.BAT\r");
    submit(&mut machine, b"SHOWPATH\r");

    assert_screen_contains(
        &machine,
        "A:\\BIN;Z:\\TOOLS",
        "batch expansion should see the PATH environment value",
    );
}

#[test]
fn path_equals_without_space_sets_value() {
    let mut machine = boot_hle_with_floppy();
    switch_to_a(&mut machine);

    submit(&mut machine, b"PATH=A:\\BIN\r");
    submit(&mut machine, b"CLS\r");
    submit(&mut machine, b"PATH\r");

    assert_screen_contains(
        &machine,
        "PATH=A:\\BIN",
        "PATH= syntax without a space should set PATH",
    );
}

#[test]
fn path_semicolon_clears_value() {
    let mut machine = boot_hle_with_floppy();
    switch_to_a(&mut machine);

    submit(&mut machine, b"PATH A:\\BIN\r");
    submit(&mut machine, b"PATH;\r");
    submit(&mut machine, b"CLS\r");
    submit(&mut machine, b"PATH\r");

    assert_screen_contains(&machine, "No Path", "PATH; should clear PATH");
}

#[test]
fn path_searches_com_programs() {
    let program = print_message_com(b"PATHRUN");
    let floppy = create_test_floppy_with_program(b"TEST    COM", &program);
    let mut machine = boot_hle_with_floppy_image(floppy);
    switch_to_a(&mut machine);

    submit(&mut machine, b"MD BIN\r");
    submit(&mut machine, b"COPY TEST.COM BIN\\RUNME.COM\r");
    submit(&mut machine, b"PATH A:\\BIN\r");
    submit(&mut machine, b"CLS\r");
    submit(&mut machine, b"RUNME\r");

    assert_screen_contains(
        &machine,
        "PATHRUN",
        "shell should execute a COM program found through PATH",
    );
}

#[test]
fn path_searches_batch_files() {
    let mut machine = boot_hle_with_floppy();
    switch_to_a(&mut machine);

    submit(&mut machine, b"MD BIN\r");
    submit(&mut machine, b"ECHO ECHO PATHBAT > BIN\\RUNBAT.BAT\r");
    submit(&mut machine, b"PATH A:\\BIN\r");
    submit(&mut machine, b"CLS\r");
    submit(&mut machine, b"RUNBAT\r");

    assert_screen_contains(
        &machine,
        "PATHBAT",
        "shell should execute a BAT file found through PATH",
    );
}

#[test]
fn call_searches_batch_files_through_path() {
    let mut machine = boot_hle_with_floppy();
    switch_to_a(&mut machine);

    submit(&mut machine, b"MD BIN\r");
    submit(&mut machine, b"ECHO ECHO CHILDOK > BIN\\CHILD.BAT\r");
    submit(&mut machine, b"ECHO CALL CHILD > PARENT.BAT\r");
    submit(&mut machine, b"ECHO ECHO PARENTOK >> PARENT.BAT\r");
    submit(&mut machine, b"PATH A:\\BIN\r");
    submit(&mut machine, b"CLS\r");
    submit(&mut machine, b"PARENT\r");

    assert_screen_contains(
        &machine,
        "CHILDOK",
        "CALL should find a child BAT file through PATH",
    );
    assert_screen_contains(
        &machine,
        "PARENTOK",
        "CALL should return to the parent batch after the PATH child",
    );
}
