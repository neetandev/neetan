//! Integration tests for the uPD72065 FDC wrapper.

use device::{
    upd765a_fdc::{
        FdcAction, FdcCommand, FdcPhase, ST0_ABNORMAL_TERMINATION, ST0_EQUIPMENT_CHECK,
        ST0_SEEK_END,
    },
    upd72065_fdc::Upd72065Fdc,
};

/// Issues a READ DATA command up to the start of the execution phase.
fn issue_read_data(fdc: &mut Upd72065Fdc) -> FdcAction {
    fdc.write_data(0x06);
    let mut action = FdcAction::None;
    for parameter in [0x00u8, 0, 0, 1, 3, 8, 0x74, 0xFF] {
        action = fdc.write_data(parameter);
    }
    action
}

/// Issues a SCAN command and returns the final action.
fn issue_scan(fdc: &mut Upd72065Fdc, command: u8, eot: u8, stp: u8) -> FdcAction {
    fdc.write_data(command);
    let mut action = FdcAction::None;
    for parameter in [0x00u8, 0, 0, 1, 0, eot, 0x74, stp] {
        action = fdc.write_data(parameter);
    }
    action
}

#[test]
fn auxiliary_reset_aborts_command() {
    let mut fdc = Upd72065Fdc::new();
    fdc.set_drive_ready_mask(0x01);
    fdc.state.drive_cylinder[0] = 42;

    let action = issue_read_data(&mut fdc);
    assert_eq!(action, FdcAction::StartReadData);
    assert_eq!(fdc.state.phase, FdcPhase::Execution);

    fdc.write_auxiliary_command(0x36);
    assert_eq!(fdc.state.phase, FdcPhase::Idle);
    assert_eq!(fdc.state.active_command, FdcCommand::None);
    assert_eq!(fdc.state.drive_cylinder[0], 42, "head position survives");
}

#[test]
fn standby_set_and_reset_gate_command_writes() {
    let mut fdc = Upd72065Fdc::new();
    assert!(!fdc.standby());

    fdc.write_auxiliary_command(0x35);
    assert!(fdc.standby());
    assert_eq!(fdc.write_data(0x06), FdcAction::None);
    assert_eq!(fdc.state.phase, FdcPhase::Idle);

    fdc.write_auxiliary_command(0x34);
    assert!(!fdc.standby());
    fdc.write_data(0x06);
    assert_eq!(fdc.state.phase, FdcPhase::Command);
}

#[test]
fn unknown_auxiliary_commands_are_ignored() {
    let mut fdc = Upd72065Fdc::new();
    fdc.write_auxiliary_command(0x00);
    fdc.write_auxiliary_command(0xFF);
    assert!(!fdc.standby());
    assert_eq!(fdc.state.phase, FdcPhase::Idle);
}

#[test]
fn scan_equal_hit_reports_satisfied_sector() {
    let mut fdc = Upd72065Fdc::new();
    assert_eq!(issue_scan(&mut fdc, 0x11, 8, 1), FdcAction::StartScan);
    assert!(fdc.is_scan_equal());

    fdc.begin_scan_sector(&[0x01, 0x02]);
    fdc.write_data(0x01);
    fdc.write_data(0x02);
    assert!(fdc.execution_sector_done());
    assert!(fdc.scan_sector_satisfied());
}

#[test]
fn scan_low_or_equal_and_high_or_equal_verdicts() {
    let mut fdc = Upd72065Fdc::new();
    issue_scan(&mut fdc, 0x19, 8, 1);
    assert!(!fdc.is_scan_equal());
    fdc.begin_scan_sector(&[0x10, 0x30]);
    fdc.write_data(0x20);
    fdc.write_data(0x30);
    assert!(fdc.scan_sector_satisfied(), "disk <= host holds");

    // Read the result away so a new command can start.
    fdc.core_mut().complete_success();
    while fdc.state.phase == FdcPhase::Result {
        fdc.read_data();
    }

    issue_scan(&mut fdc, 0x1D, 8, 1);
    fdc.begin_scan_sector(&[0x10, 0x30]);
    fdc.write_data(0x20);
    fdc.write_data(0x30);
    assert!(
        !fdc.scan_sector_satisfied(),
        "disk >= host fails on 0x10 < 0x20"
    );
}

#[test]
fn scan_step_two_reports_alternate_stepping() {
    let mut fdc = Upd72065Fdc::new();
    issue_scan(&mut fdc, 0x11, 7, 2);
    assert_eq!(fdc.scan_step(), 2);
}

#[test]
fn recalibrate_reaches_track_zero_within_255_steps() {
    let mut fdc = Upd72065Fdc::new();
    fdc.state.drive_cylinder[0] = 200;

    fdc.write_data(0x07);
    fdc.write_data(0x00);
    assert_eq!(fdc.state.drive_cylinder[0], 0);
    assert_eq!(fdc.state.drive_st0[0], ST0_SEEK_END);
}

#[test]
fn recalibrate_over_255_sets_equipment_check() {
    let mut fdc = Upd72065Fdc::new();
    // Seek beyond the limit first (SEEK accepts any cylinder byte).
    fdc.state.drive_cylinder[0] = 255;
    fdc.state.recalibrate_step_limit = 100;

    fdc.write_data(0x07);
    fdc.write_data(0x00);
    assert_eq!(fdc.state.drive_cylinder[0], 155);
    assert_eq!(
        fdc.state.drive_st0[0],
        ST0_ABNORMAL_TERMINATION | ST0_SEEK_END | ST0_EQUIPMENT_CHECK
    );
}

#[test]
fn dma_execution_fifo_serves_bytes_without_ndm() {
    let mut fdc = Upd72065Fdc::new();
    fdc.set_drive_ready_mask(0x01);
    issue_read_data(&mut fdc);

    fdc.begin_execution_read(&[0xDE, 0xAD]);
    assert_eq!(fdc.read_status(), 0x10, "MSR shows only controller busy");
    assert_eq!(fdc.read_data(), 0xDE);
    assert_eq!(fdc.read_data(), 0xAD);
    assert!(fdc.execution_sector_done());
}

#[test]
fn ready_mask_reflects_motor_and_media() {
    let mut fdc = Upd72065Fdc::new();
    fdc.set_drive_ready_mask(0x02);
    assert_eq!(fdc.state.drive_has_disk, 0x02);
    fdc.set_drive_write_protected_mask(0x01);
    assert_eq!(fdc.state.drive_write_protected, 0x01);
}
