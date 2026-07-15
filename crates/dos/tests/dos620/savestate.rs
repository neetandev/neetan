use common::Machine;

use crate::harness::*;

/// Cycle budget for a built-in command help request to finish.
const COMMAND_COMPLETION_CYCLES: u64 = 1_000_000;

/// Runs until the requested built-in command owns the shell.
fn run_until_command_is_active(machine: &mut machine_98::Pc9801Ra, expected_name: &str) {
    for _ in 0..1_000_000 {
        machine.run_for(1);
        if machine.bus.hle_active_command_name() == Some(expected_name) {
            return;
        }
    }
    panic!("{expected_name} did not become active");
}

/// Submits one command and stops before its first command step.
fn start_command_at_save_point(
    machine: &mut machine_98::Pc9801Ra,
    command_line: &[u8],
    expected_name: &str,
) {
    type_string_long(machine, command_line);
    type_string(&mut machine.bus, b"\r");
    run_until_command_is_active(machine, expected_name);
}

#[test]
fn hle_dos_prompt_state_replays_exactly() {
    let mut machine = boot_hle();
    let initial = machine.capture_state().unwrap();
    type_string_long(&mut machine, b"SET PHASE11=READY\r");
    run_until_prompt(&mut machine);
    let expected = machine.capture_state().unwrap();

    machine.restore_state(&initial).unwrap();
    type_string_long(&mut machine, b"SET PHASE11=READY\r");
    run_until_prompt(&mut machine);
    let replayed = machine.capture_state().unwrap();
    assert_eq!(replayed.payload(), expected.payload());
}

#[test]
fn hle_dos_pending_shell_work_replays_exactly() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR\r");
    machine.run_for(10_000);
    let pending = machine.capture_state().unwrap();

    machine.run_for(5_000_000);
    let expected = machine.capture_state().unwrap();
    machine.restore_state(&pending).unwrap();
    machine.run_for(5_000_000);
    let replayed = machine.capture_state().unwrap();
    assert_eq!(replayed.payload(), expected.payload());
}

#[test]
/// Verifies every registered built-in command save-state tag.
fn every_builtin_command_restores_from_an_active_save_point() {
    let command_cases: &[(&[u8], &str)] = &[
        (b"B3SUM /?", "B3SUM"),
        (b"CLS /?", "CLS"),
        (b"VER /?", "VER"),
        (b"ECHO /?", "ECHO"),
        (b"EDIT /?", "EDIT"),
        (b"REM /?", "REM"),
        (b"CD /?", "CD"),
        (b"SET /?", "SET"),
        (b"PATH /?", "PATH"),
        (b"COPY /?", "COPY"),
        (b"DATE /?", "DATE"),
        (b"DEL /?", "DEL"),
        (b"DIR /?", "DIR"),
        (b"DISKCOPY /?", "DISKCOPY"),
        (b"DOSMOCK /?", "DOSMOCK"),
        (b"FORMAT /?", "FORMAT"),
        (b"MD /?", "MD"),
        (b"MEM /?", "MEM"),
        (b"MORE /?", "MORE"),
        (b"RD /?", "RD"),
        (b"REN /?", "REN"),
        (b"TIME /?", "TIME"),
        (b"TYPE /?", "TYPE"),
        (b"XCOPY /?", "XCOPY"),
    ];
    let mut machine = boot_hle();

    for &(command_line, expected_name) in command_cases {
        start_command_at_save_point(&mut machine, command_line, expected_name);
        let save_point = machine.capture_state().unwrap();
        assert_eq!(machine.bus.hle_active_command_name(), Some(expected_name));

        machine.run_for(COMMAND_COMPLETION_CYCLES);
        assert_eq!(
            machine.bus.hle_active_command_name(),
            None,
            "{expected_name} did not complete"
        );
        let expected = machine.capture_state().unwrap();

        machine.restore_state(&save_point).unwrap();
        assert_eq!(machine.bus.hle_active_command_name(), Some(expected_name));
        machine.run_for(COMMAND_COMPLETION_CYCLES);
        let replayed = machine.capture_state().unwrap();
        assert_eq!(
            replayed.payload(),
            expected.payload(),
            "{expected_name} did not replay exactly"
        );
    }
}

#[test]
/// Verifies the batch interpreter's internal child-wait command tag.
fn batch_child_wait_restores_from_an_active_save_point() {
    let first_floppy_image = create_test_floppy_with_program(
        b"TEST    BAT",
        b"@ECHO OFF\r\nB:\\RUNME\r\nECHO FINISHED\r\n",
    );
    let second_floppy_image = create_test_floppy_with_program(b"RUNME   COM", TEST_COM_PROGRAM);
    let mut machine = boot_hle_with_two_floppy_images(first_floppy_image, second_floppy_image);
    type_string(&mut machine.bus, b"A:\r");
    machine.run_for(COMMAND_COMPLETION_CYCLES);

    start_command_at_save_point(&mut machine, b"TEST", "WAITING_FOR_CHILD");
    let save_point = machine.capture_state().unwrap();
    assert_eq!(
        machine.bus.hle_active_command_name(),
        Some("WAITING_FOR_CHILD")
    );

    machine.run_for(5_000_000);
    let expected = machine.capture_state().unwrap();
    machine.restore_state(&save_point).unwrap();
    assert_eq!(
        machine.bus.hle_active_command_name(),
        Some("WAITING_FOR_CHILD")
    );
    machine.run_for(5_000_000);
    let replayed = machine.capture_state().unwrap();
    assert_eq!(replayed.payload(), expected.payload());
}

#[test]
fn hle_dos_active_host_copy_rebinds_its_source() {
    let source_path = std::env::temp_dir().join(format!(
        "neetan_savestate_host_copy_{}.bin",
        std::process::id()
    ));
    let source_bytes: Vec<u8> = (0..128 * 1024)
        .map(|index| (index as u8).wrapping_mul(37))
        .collect();
    std::fs::write(&source_path, &source_bytes).unwrap();

    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    let mut command = b"COPY host:".to_vec();
    command.extend_from_slice(source_path.to_str().unwrap().as_bytes());
    command.extend_from_slice(b" HOSTCOPY.BIN");
    type_string_long(&mut machine, &command);
    type_string(&mut machine.bus, b"\r");
    machine.run_for(100_000);

    let pending = machine.capture_state().unwrap();
    machine.restore_state(&pending).unwrap();
    run_until_prompt(&mut machine);
    assert!(find_row_containing(&machine.bus, "copied").is_some());

    let _ = std::fs::remove_file(source_path);
}
