use crate::harness::*;

#[test]
fn edit_without_args_opens_file_selection_mode() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"EDIT\r");
    machine.run_for(20_000_000);

    let title = text_vram_row_to_string(&machine.bus, 0);
    let path_line = text_vram_row_to_string(&machine.bus, 1);
    let function_bar = text_vram_row_to_string(&machine.bus, 24);

    assert!(
        title.contains("edit - Open File"),
        "title row should show file selection mode, got: {title:?}"
    );
    assert!(
        path_line.contains("Path: A:\\"),
        "path row should show current directory, got: {path_line:?}"
    );
    assert!(
        function_bar.contains("F2 Open"),
        "function key bar should expose open action, got: {function_bar:?}"
    );
    assert!(
        function_bar.contains("F5 Del"),
        "function key bar should expose delete action, got: {function_bar:?}"
    );
    let footer = text_vram_row_to_string(&machine.bus, 22);
    assert!(
        !footer.contains("Action:Open"),
        "picker footer should not show a static action label, got: {footer:?}"
    );
}

#[test]
fn edit_existing_file_opens_editor_mode() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"EDIT TESTFILE.TXT\r");
    machine.run_for(20_000_000);

    let title = text_vram_row_to_string(&machine.bus, 0);
    let status = text_vram_row_to_string(&machine.bus, 22);

    assert!(
        title.contains("edit - A:\\TESTFILE.TXT"),
        "editor title should show full file path, got: {title:?}"
    );
    assert!(
        find_string_in_text_vram(
            &machine.bus,
            &[
                0x0048, 0x0045, 0x004C, 0x004C, 0x004F, 0x0020, 0x0057, 0x004F, 0x0052, 0x004C,
                0x0044
            ]
        ),
        "editor should render the file contents"
    );
    assert!(
        status.contains("Enc:SJIS"),
        "status line should expose Shift-JIS encoding, got: {status:?}"
    );
}

#[test]
fn edit_can_create_save_and_exit_new_file() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"EDIT NEWFILE.TXT\r");
    machine.run_for(20_000_000);

    type_string(&mut machine.bus, b"XYZ");
    machine.run_for(5_000_000);
    type_special_key(&mut machine, 0x63);
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"\x1B");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"\x1B");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string_long(&mut machine, b"TYPE NEWFILE.TXT\r");
    run_until_prompt(&mut machine);

    assert!(
        find_string_in_text_vram(&machine.bus, &[0x0058, 0x0059, 0x005A]),
        "saved file should contain the inserted text"
    );
}

#[test]
fn edit_escape_returns_to_file_selection_mode() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"EDIT TESTFILE.TXT\r");
    machine.run_for(20_000_000);

    type_string(&mut machine.bus, b"\x1B");
    machine.run_for(20_000_000);

    let title = text_vram_row_to_string(&machine.bus, 0);
    let path_line = text_vram_row_to_string(&machine.bus, 1);

    assert!(
        title.contains("edit - Open File"),
        "ESC should return to the file picker, got title: {title:?}"
    );
    assert!(
        path_line.contains("Path: A:\\"),
        "file picker should reopen in the file directory, got: {path_line:?}"
    );
}

#[test]
fn edit_new_opens_name_dialog() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"EDIT\r");
    machine.run_for(20_000_000);

    type_special_key(&mut machine, 0x64);
    machine.run_for(20_000_000);
    type_string(&mut machine.bus, b"NEWFILE.TXT");
    machine.run_for(20_000_000);

    let dialog_title = text_vram_row_to_string(&machine.bus, 9);
    let dialog_path = text_vram_row_to_string(&machine.bus, 10);

    assert!(
        dialog_title.contains("New File"),
        "F3 should open the new-file dialog, got: {dialog_title:?}"
    );
    assert!(
        dialog_path.contains("Path:"),
        "new-file dialog should show a path field, got: {dialog_path:?}"
    );
    assert!(
        dialog_path.contains("A:\\NEWFILE.TXT"),
        "new-file dialog should redraw typed text, got: {dialog_path:?}"
    );
}

#[test]
fn edit_new_rejects_existing_file_name() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"EDIT\r");
    machine.run_for(20_000_000);

    type_string(&mut machine.bus, b"\tTESTFILE.TXT");
    machine.run_for(10_000_000);
    type_special_key(&mut machine, 0x64);
    machine.run_for(20_000_000);
    type_string(&mut machine.bus, b"\r");
    machine.run_for(20_000_000);

    let title = text_vram_row_to_string(&machine.bus, 0);
    let dialog_title = text_vram_row_to_string(&machine.bus, 9);
    let message = text_vram_row_to_string(&machine.bus, 23);

    assert!(
        title.contains("edit - Open File"),
        "existing-file collision should keep the picker active, got: {title:?}"
    );
    assert!(
        dialog_title.contains("New File"),
        "collision should keep the new-file dialog open, got: {dialog_title:?}"
    );
    assert!(
        message.contains("Already exists"),
        "collision should report an error, got: {message:?}"
    );
}

#[test]
fn edit_save_as_dialog_updates_while_typing() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"EDIT TESTFILE.TXT\r");
    machine.run_for(20_000_000);

    type_special_key(&mut machine, 0x64);
    machine.run_for(20_000_000);
    type_string(&mut machine.bus, b"2");
    machine.run_for(20_000_000);

    let dialog_title = text_vram_row_to_string(&machine.bus, 9);
    let dialog_path = text_vram_row_to_string(&machine.bus, 10);

    assert!(
        dialog_title.contains("Save As"),
        "F3 in editor should open the save-as dialog, got: {dialog_title:?}"
    );
    assert!(
        dialog_path.contains("TESTFILE.TXT2"),
        "save-as dialog should redraw typed text, got: {dialog_path:?}"
    );
}

#[test]
fn edit_f5_deletes_file_after_confirmation() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"COPY TESTFILE.TXT ZZZDEL.TXT\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"EDIT\r");
    machine.run_for(20_000_000);

    type_special_key(&mut machine, 0x3F);
    machine.run_for(10_000_000);
    type_special_key(&mut machine, 0x66);
    machine.run_for(20_000_000);

    let dialog_title = text_vram_row_to_string(&machine.bus, 9);
    let dialog_name = text_vram_row_to_string(&machine.bus, 11);
    assert!(dialog_title.contains("Delete"));
    assert!(dialog_name.contains("ZZZDEL.TXT"));

    type_string(&mut machine.bus, b"\r");
    machine.run_for(20_000_000);
    type_string(&mut machine.bus, b"\x1B");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string_long(&mut machine, b"TYPE ZZZDEL.TXT\r");
    run_until_prompt(&mut machine);

    assert!(
        find_row_containing(&machine.bus, "File not found").is_some(),
        "deleted file should no longer be readable"
    );
}

#[test]
fn edit_f5_deletes_empty_directory_after_confirmation() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"MD 0EMPTY\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"EDIT\r");
    machine.run_for(20_000_000);

    type_special_key(&mut machine, 0x3D);
    machine.run_for(10_000_000);
    type_special_key(&mut machine, 0x66);
    machine.run_for(20_000_000);
    type_string(&mut machine.bus, b"\r");
    machine.run_for(20_000_000);
    type_string(&mut machine.bus, b"\x1B");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt(&mut machine);

    assert!(
        !find_string_in_text_vram(
            &machine.bus,
            &[0x0030, 0x0045, 0x004D, 0x0050, 0x0054, 0x0059]
        ),
        "deleted directory should no longer appear in DIR"
    );
}

#[test]
fn edit_f5_rejects_non_empty_directory() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"MD 0NONEMPTY\r");
    run_until_prompt(&mut machine);
    type_string_long(&mut machine, b"COPY TESTFILE.TXT 0NONEMPTY\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"EDIT\r");
    machine.run_for(20_000_000);

    type_special_key(&mut machine, 0x3D);
    machine.run_for(10_000_000);
    type_special_key(&mut machine, 0x66);
    machine.run_for(20_000_000);
    type_string(&mut machine.bus, b"\r");
    machine.run_for(20_000_000);

    let title = text_vram_row_to_string(&machine.bus, 0);
    let message = text_vram_row_to_string(&machine.bus, 23);

    assert!(
        title.contains("edit - Open File"),
        "failed directory delete should keep the picker active, got: {title:?}"
    );
    assert!(
        message.contains("Directory not empty"),
        "non-empty directory delete should show a clear error, got: {message:?}"
    );
}
