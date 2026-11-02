use crate::harness::*;

fn submit(machine: &mut machine_98::Pc9801Ra, command: &[u8]) {
    type_string_long(machine, command);
    run_until_prompt(machine);
}

#[test]
fn prompt_sets_the_prompt_text_and_resets_it() {
    let mut machine = boot_hle_with_floppy();
    submit(&mut machine, b"A:\r");

    submit(&mut machine, b"PROMPT [$P]$G\r");
    assert!(
        find_row_containing(&machine.bus, "[A:\\]>").is_some(),
        "PROMPT should change the prompt text"
    );

    submit(&mut machine, b"PROMPT\r");
    submit(&mut machine, b"CLS\r");
    assert!(
        find_row_containing(&machine.bus, "[A:\\]>").is_none(),
        "PROMPT without text should restore the default prompt"
    );
    assert!(
        find_row_containing(&machine.bus, "A:\\>").is_some(),
        "the default prompt shows the current directory"
    );
}
