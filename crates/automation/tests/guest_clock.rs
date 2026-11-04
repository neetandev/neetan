//! Guest real-time clock: the clock that a session hands to its machines starts at
//! the configured guest time and advances with the emulated time of the machine.

#[path = "common/harness.rs"]
mod harness;

use std::{path::Path, sync::Arc};

use automation::guest_clock::GuestClock;
use common::{HostDateTime, HostDateTimeSource, MachineModel, RunTarget};
use harness::{fixed_date_time, make_session_with_guest_clock};
use machine_factory::config::{EmulatorConfig, MachineSelection};

/// Nanoseconds in one second.
const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;

/// Returns the PC-98 configuration that every test opens.
fn pc98_config() -> EmulatorConfig {
    EmulatorConfig::for_model(MachineSelection::Pc98(MachineModel::PC9801VM))
}

/// Returns the start time plus `seconds`.
fn start_plus(seconds: u64) -> HostDateTime {
    HostDateTime::from_unix_seconds(fixed_date_time().to_unix_seconds() + seconds)
}

/// Opens a PC-98, runs it in steps of `frames_per_step` frames and records the
/// elapsed nanoseconds and the guest time after each step.
fn drive(steps: usize, frames_per_step: u64) -> Vec<(u64, HostDateTime)> {
    let clock = Arc::new(GuestClock::new(fixed_date_time()));
    let (mut session, _receiver) =
        make_session_with_guest_clock(Path::new("."), Path::new("."), clock.clone());
    session
        .open_machine(pc98_config(), Vec::new())
        .expect("open machine");
    let mut samples = Vec::with_capacity(steps);
    for _ in 0..steps {
        session
            .run(RunTarget::Frames(frames_per_step), u64::MAX / 4, 1_000_000)
            .expect("frame run");
        samples.push((clock.elapsed_nanoseconds(), clock.now()));
    }
    samples
}

#[test]
fn guest_clock_starts_at_the_configured_time() {
    let clock = Arc::new(GuestClock::new(fixed_date_time()));
    let (mut session, _receiver) =
        make_session_with_guest_clock(Path::new("."), Path::new("."), clock.clone());
    session
        .open_machine(pc98_config(), Vec::new())
        .expect("open machine");
    assert_eq!(clock.elapsed_nanoseconds(), 0);
    assert_eq!(clock.now(), fixed_date_time());
}

#[test]
fn guest_clock_advances_with_emulated_time() {
    let clock = Arc::new(GuestClock::new(fixed_date_time()));
    let (mut session, _receiver) =
        make_session_with_guest_clock(Path::new("."), Path::new("."), clock.clone());
    session
        .open_machine(pc98_config(), Vec::new())
        .expect("open machine");
    session
        .run(RunTarget::Frames(200), u64::MAX / 4, 1_000_000)
        .expect("frame run");
    let elapsed = session.emulated_time_ns();
    assert!(elapsed > 3 * u128::from(NANOSECONDS_PER_SECOND));
    assert_eq!(u128::from(clock.elapsed_nanoseconds()), elapsed);
    let elapsed_seconds = u64::try_from(elapsed).unwrap() / NANOSECONDS_PER_SECOND;
    assert_eq!(clock.now(), start_plus(elapsed_seconds));

    session
        .run(RunTarget::Ticks(1_000_000), 1_000_000, 100_000)
        .expect("tick run");
    assert_eq!(
        u128::from(clock.elapsed_nanoseconds()),
        session.emulated_time_ns()
    );
    assert!(u128::from(clock.elapsed_nanoseconds()) > elapsed);
}

#[test]
fn guest_clock_keeps_running_across_a_hard_reset() {
    let clock = Arc::new(GuestClock::new(fixed_date_time()));
    let (mut session, _receiver) =
        make_session_with_guest_clock(Path::new("."), Path::new("."), clock.clone());
    let (handle, _) = session
        .open_machine(pc98_config(), Vec::new())
        .expect("open machine");
    session
        .run(RunTarget::Frames(120), u64::MAX / 4, 1_000_000)
        .expect("frame run");
    let before_reset = clock.elapsed_nanoseconds();
    assert!(before_reset > 0);

    session.reset(true).expect("hard reset");
    assert_eq!(clock.elapsed_nanoseconds(), before_reset);
    session
        .run(RunTarget::Frames(10), u64::MAX / 4, 1_000_000)
        .expect("frame run");
    assert!(clock.elapsed_nanoseconds() > before_reset);

    session.close_machine(handle).expect("close machine");
    session
        .open_machine(pc98_config(), Vec::new())
        .expect("reopen machine");
    assert_eq!(clock.elapsed_nanoseconds(), 0);
}

#[test]
fn guest_clock_is_identical_across_two_runs() {
    let first = drive(8, 25);
    let second = drive(8, 25);
    assert_eq!(first, second);
    assert!(first.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert!(first.last().unwrap().1 != fixed_date_time());
}

#[test]
fn restore_state_rewinds_the_guest_clock() {
    let clock = Arc::new(GuestClock::new(fixed_date_time()));
    let (mut session, _receiver) =
        make_session_with_guest_clock(Path::new("."), Path::new("."), clock.clone());
    let (handle, _) = session
        .open_machine(pc98_config(), Vec::new())
        .expect("open machine");
    session
        .run(RunTarget::Frames(120), u64::MAX / 4, 1_000_000)
        .expect("frame run");
    let saved_elapsed = clock.elapsed_nanoseconds();
    let state = session.save_state(handle).expect("save state");
    session
        .run(RunTarget::Frames(120), u64::MAX / 4, 1_000_000)
        .expect("frame run");
    assert!(clock.elapsed_nanoseconds() > saved_elapsed);

    session.restore_state(handle, state).expect("restore state");
    assert_eq!(clock.elapsed_nanoseconds(), saved_elapsed);
}

#[test]
fn restore_startup_keeps_the_guest_clock_running() {
    let clock = Arc::new(GuestClock::new(fixed_date_time()));
    let (mut session, _receiver) =
        make_session_with_guest_clock(Path::new("."), Path::new("."), clock.clone());
    session
        .open_machine(pc98_config(), Vec::new())
        .expect("open machine");
    session
        .run(RunTarget::Frames(120), u64::MAX / 4, 1_000_000)
        .expect("frame run");
    let before_restore = clock.elapsed_nanoseconds();
    assert!(before_restore > 0);

    session.restore_startup().expect("restore startup");
    assert_eq!(clock.elapsed_nanoseconds(), before_restore);
    session
        .run(RunTarget::Frames(10), u64::MAX / 4, 1_000_000)
        .expect("frame run");
    assert!(clock.elapsed_nanoseconds() > before_restore);
}

/// The HLE DOS TIME command of the PC-98 reads the guest clock. Its output shows the
/// seconds of emulated time since the configured start.
#[test]
fn guest_reads_the_advancing_clock() {
    let clock = Arc::new(GuestClock::new(fixed_date_time()));
    let (mut session, _receiver) =
        make_session_with_guest_clock(Path::new("."), Path::new("."), clock.clone());
    let mut config = pc98_config();
    config.boot_device = common::BootDevice::Dos;
    session
        .open_machine(config, Vec::new())
        .expect("open machine");
    session
        .run(RunTarget::Frames(400), u64::MAX / 4, 1_000_000)
        .expect("frame run");
    let first_second = clock.elapsed_nanoseconds() / NANOSECONDS_PER_SECOND;
    session
        .type_text("time\r", 2, u64::MAX / 4)
        .expect("type text");
    session
        .run(RunTarget::Frames(30), u64::MAX / 4, 1_000_000)
        .expect("frame run");
    let last_second = clock.elapsed_nanoseconds() / NANOSECONDS_PER_SECOND;
    assert!(first_second >= 5);

    let lines = session.text_screen_lines("display.main").expect("text");
    let shown = lines
        .iter()
        .find_map(|line| line.trim().strip_prefix("Current time is 00:00:"))
        .expect("TIME output");
    let shown_second: u64 = shown[..2].parse().expect("seconds");
    assert!(
        (first_second..=last_second).contains(&shown_second),
        "{shown_second} not in {first_second}..={last_second}"
    );
}
