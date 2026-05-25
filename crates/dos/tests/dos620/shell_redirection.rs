use common::JisChar;

use crate::harness::*;

#[test]
fn redirect_output_overwrite() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"ECHO HELLO >OUTPUT.TXT\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"TYPE OUTPUT.TXT\r");
    run_until_prompt(&mut machine);

    let hello = [0x0048, 0x0045, 0x004C, 0x004C, 0x004F]; // "HELLO"
    assert!(
        find_string_in_text_vram(&machine.bus, &hello),
        "TYPE of redirected file should show 'HELLO'"
    );
}

#[test]
fn redirect_output_append() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"ECHO LINE1 >OUT.TXT\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"ECHO LINE2 >>OUT.TXT\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"TYPE OUT.TXT\r");
    run_until_prompt(&mut machine);

    let line1 = [0x004C, 0x0049, 0x004E, 0x0045, 0x0031]; // "LINE1"
    let line2 = [0x004C, 0x0049, 0x004E, 0x0045, 0x0032]; // "LINE2"
    assert!(
        find_string_in_text_vram(&machine.bus, &line1),
        "appended file should contain 'LINE1'"
    );
    assert!(
        find_string_in_text_vram(&machine.bus, &line2),
        "appended file should contain 'LINE2'"
    );
}

#[test]
fn redirect_dir_to_file() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"DIR >DIROUT.TXT\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"TYPE DIROUT.TXT\r");
    run_until_prompt(&mut machine);

    // DIR output should contain "COMMAND" somewhere
    let command = [0x0043, 0x004F, 0x004D, 0x004D, 0x0041, 0x004E, 0x0044]; // "COMMAND"
    assert!(
        find_string_in_text_vram(&machine.bus, &command),
        "DIR redirected output should contain 'COMMAND'"
    );
}

#[test]
fn redirect_type_preserves_shift_jis_bytes() {
    let floppy = create_test_floppy_with_program(b"JAPAN   TXT", b"\x82\xA0\x82\xA2\r\n");
    let mut machine = boot_hle_with_floppy_image(floppy);
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"TYPE JAPAN.TXT >OUT.TXT\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"TYPE OUT.TXT\r");
    run_until_prompt(&mut machine);

    let chars = [JisChar::from_u16(0x2422), JisChar::from_u16(0x2424)];
    assert!(
        find_jis_string_in_text_vram(&machine.bus, &chars),
        "redirected TYPE output should preserve raw Shift-JIS bytes"
    );
}

#[test]
fn pipe_echo_more() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"ECHO PIPED | MORE\r");
    run_until_prompt(&mut machine);

    let piped = [0x0050, 0x0049, 0x0050, 0x0045, 0x0044]; // "PIPED"
    assert!(
        find_string_in_text_vram(&machine.bus, &piped),
        "piped ECHO through MORE should show 'PIPED'"
    );
}

#[test]
fn sequence_commands() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    // Type two ECHO commands separated by ASCII 0x14 (paragraph mark)
    let mut cmd = Vec::new();
    cmd.extend_from_slice(b"ECHO FIRST");
    cmd.push(0x14);
    cmd.extend_from_slice(b"ECHO SECOND\r");
    type_string_long(&mut machine, &cmd);
    run_until_prompt(&mut machine);

    let first = [0x0046, 0x0049, 0x0052, 0x0053, 0x0054]; // "FIRST"
    let second = [0x0053, 0x0045, 0x0043, 0x004F, 0x004E, 0x0044]; // "SECOND"
    assert!(
        find_string_in_text_vram(&machine.bus, &first),
        "first sequenced command should show 'FIRST'"
    );
    assert!(
        find_string_in_text_vram(&machine.bus, &second),
        "second sequenced command should show 'SECOND'"
    );
}
