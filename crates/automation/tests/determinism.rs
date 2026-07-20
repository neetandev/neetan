//! Repeatable machine execution and session accounting: two identical runs
//! produce byte-identical framebuffers, tick counts, and frame counts, and the
//! session tracks epochs, totals, and budgets across resets. ROM-free families
//! (PC-98 HLE and MSX) are used so no external fixtures are required.

#[path = "common/harness.rs"]
mod harness;

use std::path::Path;

use automation::{RunError, SessionBudgets};
use common::{AutomatedMachine, RunRequest, RunTarget, StopReason};
use harness::{build_msx, build_pc98, make_session};

#[test]
fn execution_options_contract_holds() {
    let run = harness::run_committed_script("execution-options.scm", 60);
    harness::assert_completed_ok(&run.termination);
}

/// A summary of machine state used to compare two identical runs.
#[derive(PartialEq, Eq, Debug)]
struct RunFingerprint {
    epoch_ticks: u128,
    epoch_frames: u128,
    dimensions: (u32, u32),
    framebuffer: Vec<u8>,
}

/// Drives a machine through a fixed frame-then-tick sequence and fingerprints it.
fn fingerprint(mut machine: Box<dyn AutomatedMachine>) -> RunFingerprint {
    let frames = machine.run_automation(RunRequest {
        target: RunTarget::Frames(30),
        max_ticks: 50_000_000,
        audio_drain_interval_ticks: 1_000_000,
    });
    assert!(matches!(
        frames.stop_reason,
        StopReason::TargetReached | StopReason::TickLimit
    ));
    machine.run_automation(RunRequest {
        target: RunTarget::Ticks(500_000),
        max_ticks: 0,
        audio_drain_interval_ticks: 100_000,
    });
    let timeline = machine.automation_timeline();
    RunFingerprint {
        epoch_ticks: timeline.epoch_ticks,
        epoch_frames: timeline.epoch_frames,
        dimensions: machine.display_dimensions(),
        framebuffer: machine.display_framebuffer().to_vec(),
    }
}

#[test]
fn pc98_two_runs_are_identical() {
    assert_eq!(fingerprint(build_pc98()), fingerprint(build_pc98()));
}

#[test]
fn msx_two_runs_are_identical() {
    assert_eq!(fingerprint(build_msx()), fingerprint(build_msx()));
}

#[test]
fn frame_target_yields_at_the_exact_presentation_boundary() {
    let mut machine = build_pc98();
    let first = machine.run_automation(RunRequest {
        target: RunTarget::Frames(30),
        max_ticks: 50_000_000,
        audio_drain_interval_ticks: 1_000_000,
    });
    // The presentation-boundary yield must fire, so the run stops exactly at the
    // requested frame count rather than exhausting the tick fallback.
    assert_eq!(first.stop_reason, StopReason::TargetReached);
    assert_eq!(first.frames, 30);
    assert_eq!(machine.automation_timeline().epoch_frames, 30);

    // A second frame target advances the counter by exactly the requested amount.
    let second = machine.run_automation(RunRequest {
        target: RunTarget::Frames(15),
        max_ticks: 50_000_000,
        audio_drain_interval_ticks: 1_000_000,
    });
    assert_eq!(second.stop_reason, StopReason::TargetReached);
    assert_eq!(second.frames, 15);
    assert_eq!(machine.automation_timeline().epoch_frames, 45);
}

#[test]
fn msx_frame_target_yields_at_the_exact_presentation_boundary() {
    let mut machine = build_msx();
    let outcome = machine.run_automation(RunRequest {
        target: RunTarget::Frames(20),
        max_ticks: 50_000_000,
        audio_drain_interval_ticks: 1_000_000,
    });
    assert_eq!(outcome.stop_reason, StopReason::TargetReached);
    assert_eq!(outcome.frames, 20);
    assert_eq!(machine.automation_timeline().epoch_frames, 20);
}

#[test]
fn tick_run_advances_the_exact_budget() {
    let mut machine = build_pc98();
    let outcome = machine.run_automation(RunRequest {
        target: RunTarget::Ticks(100_000),
        max_ticks: 0,
        audio_drain_interval_ticks: 50_000,
    });
    // A tick target reaches at least its budget and overshoots by at most one
    // indivisible CPU operation.
    assert!(outcome.ticks >= 100_000);
    assert_eq!(outcome.ticks - 100_000, outcome.overshoot_ticks);
}

#[test]
fn session_run_matches_across_two_sessions() {
    fn drive() -> (u128, u128, u128) {
        let (mut session, _receiver) = make_session(Path::new("."), Path::new("."));
        session.install_machine(build_pc98());
        session
            .run(RunTarget::Frames(20), 50_000_000, 1_000_000)
            .expect("frame run");
        session
            .run(RunTarget::Ticks(250_000), 250_000, 100_000)
            .expect("tick run");
        let timeline = session.timeline();
        (
            timeline.session_ticks,
            timeline.session_frames,
            session.emulated_time_ns(),
        )
    }
    assert_eq!(drive(), drive());
}

#[test]
fn reconstruct_increments_epoch_and_keeps_totals_monotonic() {
    let (mut session, _receiver) = make_session(Path::new("."), Path::new("."));
    session.install_machine(build_pc98());
    session
        .run(RunTarget::Frames(10), 50_000_000, 1_000_000)
        .expect("frame run");
    let before = session.timeline();
    assert_eq!(before.epoch, 0);
    assert!(before.session_frames >= before.epoch_frames);

    session.reconstruct_machine(build_pc98());
    let after = session.timeline();
    assert_eq!(after.epoch, 1);
    // Epoch-relative counters reset on reconstruction.
    assert_eq!(after.epoch_ticks, 0);
    assert_eq!(after.epoch_frames, 0);
    // Session totals stay monotonic across the reset.
    assert!(after.session_ticks >= before.session_ticks);
    assert!(after.session_frames >= before.session_frames);
}

#[test]
fn frame_budget_exhaustion_stops_the_run() {
    let (mut session, _receiver) = make_session(Path::new("."), Path::new("."));
    session.install_machine(build_pc98());
    session.set_budgets(SessionBudgets {
        frames: Some(3),
        ..SessionBudgets::default()
    });
    let outcome = session
        .run(RunTarget::Frames(100), 50_000_000, 1_000_000)
        .expect("frame run");
    assert_eq!(outcome.stop_reason, StopReason::CounterExhausted);
    assert!(outcome.frames <= 3);
}

#[test]
fn run_without_machine_reports_no_machine() {
    let (mut session, _receiver) = make_session(Path::new("."), Path::new("."));
    let error = session.run(RunTarget::Ticks(1), 0, 1).unwrap_err();
    assert_eq!(error, RunError::NoMachine);
}
