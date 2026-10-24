//! Script-free construction through the machine factory.
//!
//! These tests exercise `initialize_automated_machine` directly. The PC-98 HLE
//! machine builds with no ROM directory, so it is used as the ROM-free family.
//! MSX requires a ROM directory through the factory, so it is used to assert a
//! clean construction error when no ROMs are configured.

#[path = "common/harness.rs"]
mod harness;

use std::sync::Arc;

use common::{FixedHostDateTime, MachineModel, SharedHostDateTimeSource};
use harness::{fixed_date_time, trace_sink};
use machine_factory::{
    InitErrorKind,
    config::{EmulatorConfig, MachineSelection},
    machines::initialize_automated_machine,
};
use machine_msx::MsxModel;

/// Returns the fixed guest real-time-clock source.
fn fixed_rtc() -> SharedHostDateTimeSource {
    Arc::new(FixedHostDateTime(fixed_date_time()))
}

#[test]
fn builds_pc98_hle_without_roms() {
    let config = EmulatorConfig::for_model(MachineSelection::Pc98(MachineModel::PC9801VM));
    let machine = initialize_automated_machine(config, fixed_rtc(), 48_000, trace_sink())
        .expect("PC-98 HLE constructs without a ROM directory");
    let descriptor = machine.automation_descriptor();
    assert_eq!(descriptor.target, "pc98");
    assert!(descriptor.input.keyboard);
}

#[test]
fn msx_without_roms_reports_a_clean_error() {
    let config = EmulatorConfig::for_model(MachineSelection::Msx(MsxModel::Msx));
    let error = match initialize_automated_machine(config, fixed_rtc(), 48_000, trace_sink()) {
        Ok(_) => panic!("MSX must require a ROM directory through the factory"),
        Err(error) => error,
    };
    assert!(
        matches!(error.kind, InitErrorKind::RomMissing | InitErrorKind::Io),
        "expected a ROM-missing or I/O error, got {:?}",
        error.kind
    );
}
