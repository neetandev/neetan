use crate::harness::*;

#[test]
fn diskcopy_between_drives() {
    let mut machine = boot_hle_with_two_floppies();

    type_string_long(&mut machine, b"DISKCOPY A: B:\r");
    // Wait for "Copy another (Y/N)?" prompt
    machine.run_for(500_000_000);
    // Answer N to the "Copy another" prompt
    type_string(&mut machine.bus, b"N");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    // Verify: DIR B: should show TESTFILE.TXT from the source
    type_string(&mut machine.bus, b"DIR B:\r");
    run_until_prompt(&mut machine);

    let testfile = [
        0x0054, 0x0045, 0x0053, 0x0054, 0x0046, 0x0049, 0x004C, 0x0045,
    ]; // "TESTFILE"
    assert!(
        find_string_in_text_vram(&machine.bus, &testfile),
        "DIR B: after DISKCOPY should show TESTFILE"
    );
}

#[test]
fn diskcopy_with_verify() {
    let mut machine = boot_hle_with_two_floppies();

    type_string_long(&mut machine, b"DISKCOPY A: B: /V\r");
    machine.run_for(500_000_000);
    type_string(&mut machine.bus, b"N");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    // Check it completed - use DIR to verify copy worked
    type_string(&mut machine.bus, b"DIR B:\r");
    run_until_prompt(&mut machine);

    let testfile = [
        0x0054, 0x0045, 0x0053, 0x0054, 0x0046, 0x0049, 0x004C, 0x0045,
    ]; // "TESTFILE"
    assert!(
        find_string_in_text_vram(&machine.bus, &testfile),
        "DIR B: after DISKCOPY /V should show TESTFILE"
    );
}

#[test]
fn diskcopy_no_arguments() {
    let mut machine = boot_hle_with_floppy();

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DISKCOPY\r");
    run_until_prompt(&mut machine);

    let help = [
        0x0044, 0x0049, 0x0053, 0x004B, 0x0043, 0x004F, 0x0050, 0x0059,
    ]; // "DISKCOPY"
    assert!(
        find_string_in_text_vram(&machine.bus, &help),
        "DISKCOPY with no args should show help text"
    );
}
