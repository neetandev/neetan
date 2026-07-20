//! Machine-neutral automation boundary.
//!
//! Defines the [`AutomatedMachine`] supertrait and its descriptor, timeline,
//! run-request, and run-outcome types. These carry only stable identifiers and
//! plain scalars, never concrete device or CPU types, so the automation
//! frontend can drive any machine family without machine-specific knowledge.

use crate::{
    Machine,
    inspect::MachineInspector,
    trace::{TraceEventClass, TraceInterest},
};

/// A machine that exposes the automation execution boundary on top of the
/// interactive [`Machine`] interface.
pub trait AutomatedMachine: Machine {
    /// Returns the stable, device-free descriptor of this machine.
    fn automation_descriptor(&self) -> AutomationDescriptor;

    /// Returns the current epoch-relative timeline counters.
    ///
    /// Only the epoch-relative fields are filled here. The automation session
    /// overlays the epoch number and the session-total bases, because the epoch
    /// is host lifecycle state.
    fn automation_timeline(&self) -> AutomationTimeline;

    /// Advances the machine according to `request` and reports the outcome.
    fn run_automation(&mut self, request: RunRequest) -> RunOutcome;

    /// Returns the read-only and mutation inspector, when supported.
    ///
    /// The inspector is populated in a later phase; it is `None` for now.
    fn inspector(&mut self) -> Option<&mut dyn MachineInspector> {
        None
    }

    /// Asserts the documented machine reset mechanism, returning whether a real
    /// reset was performed.
    ///
    /// The default is `false`, meaning soft reset is not implemented; a family
    /// overrides it only when the mechanism is correctly emulated.
    fn soft_reset(&mut self) -> bool {
        false
    }

    /// Returns the stable trace identifiers this machine emits, for schema
    /// discovery. It must stay in sync with the machine's actual emitters.
    fn trace_catalog(&self) -> TraceCatalog;
}

/// A device identifier and the action identifiers a machine emits for it.
#[derive(Debug, Clone, Copy)]
pub struct TraceDeviceCatalog {
    /// Stable namespaced device identifier.
    pub device: &'static str,
    /// Stable action identifiers emitted for this device.
    pub actions: &'static [&'static str],
}

/// A call provider identifier and the named interface identifiers a machine emits.
#[derive(Debug, Clone, Copy)]
pub struct TraceProviderCatalog {
    /// Stable namespaced provider identifier.
    pub provider: &'static str,
    /// Stable named-interface identifiers, empty when only numeric interfaces are
    /// used.
    pub named_interfaces: &'static [&'static str],
}

/// The stable trace identifiers a machine actually emits.
///
/// This feeds `trace-schema` discovery. Every field is a borrowed static slice,
/// so a catalog allocates nothing. Address spaces are reported separately from
/// the inspector, so they are not duplicated here.
#[derive(Debug, Clone, Copy, Default)]
pub struct TraceCatalog {
    /// Interrupt controller and source identifiers.
    pub controllers: &'static [&'static str],
    /// Scheduled-event identifiers in scheduler-slot order.
    pub scheduled: &'static [&'static str],
    /// Device identifiers with their action identifiers.
    pub devices: &'static [TraceDeviceCatalog],
    /// Call providers with their named interface identifiers.
    pub providers: &'static [TraceProviderCatalog],
}

impl TraceCatalog {
    /// Returns the event classes this machine can emit.
    ///
    /// The four baseline classes are always emitted. Device and call classes are
    /// reported only when the catalog declares device or provider identifiers.
    #[must_use]
    pub fn classes(&self) -> TraceInterest {
        let mut interest = TraceInterest::MACHINE_BASELINE;
        if !self.devices.is_empty() {
            interest = interest.union(TraceInterest::only(TraceEventClass::Device));
        }
        if !self.providers.is_empty() {
            interest = interest.union(TraceInterest::only(TraceEventClass::Call));
        }
        interest
    }
}

/// Stable, device-free identity and structured descriptors for a machine.
#[derive(Debug, Clone, Copy)]
pub struct AutomationDescriptor {
    /// Stable target identifier, for example `"pc98"`.
    pub target: &'static str,
    /// Stable model identifier, for example `"pc9801vm"`.
    pub model: &'static str,
    /// Primary scheduling-tick frequency.
    pub timebase: AutomationTimebase,
    /// Fixed audio output rate in Hz.
    pub audio_sample_rate: u32,
    /// Logical input classes this machine supports.
    pub input: InputCapabilities,
}

/// Logical input classes a machine exposes to automation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputCapabilities {
    /// Whether the machine accepts host-key input.
    pub keyboard: bool,
    /// Number of mouse buttons, or `0` when the machine has no mouse.
    pub mouse_buttons: u8,
    /// Number of joystick ports, or `0` when the machine has no joystick.
    pub joystick_ports: u8,
}

/// Primary scheduling-tick frequency expressed as an exact rational.
#[derive(Debug, Clone, Copy)]
pub struct AutomationTimebase {
    /// Numerator of the ticks-per-second rate.
    pub ticks_per_second_numerator: u64,
    /// Denominator of the ticks-per-second rate. Always nonzero.
    pub ticks_per_second_denominator: u64,
}

/// Session-total and epoch-relative tick and frame counters.
///
/// Session totals remain monotonic across resets. Reconstructing the machine
/// increments the epoch and resets only the epoch-relative values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AutomationTimeline {
    /// Host lifecycle epoch, incremented on every reconstruction.
    pub epoch: u64,
    /// Total ticks advanced across all epochs.
    pub session_ticks: u128,
    /// Total frames presented across all epochs.
    pub session_frames: u128,
    /// Ticks advanced within the current epoch.
    pub epoch_ticks: u128,
    /// Frames presented within the current epoch.
    pub epoch_frames: u128,
}

/// Selects what a single automation run advances toward.
#[derive(Debug, Clone, Copy)]
pub enum RunTarget {
    /// Advance this many epoch-relative ticks.
    Ticks(u64),
    /// Present this many further frames.
    Frames(u64),
    /// Advance until this absolute session frame is reached.
    UntilSessionFrame(u128),
}

/// A single bounded automation run request.
#[derive(Debug, Clone, Copy)]
pub struct RunRequest {
    /// What the run advances toward.
    pub target: RunTarget,
    /// Maximum epoch-relative ticks. This is the fallback bound for a
    /// frame-based target that never reaches a presentation boundary.
    pub max_ticks: u64,
    /// Bounded intermediate audio-drain interval for tick-only execution.
    pub audio_drain_interval_ticks: u64,
}

/// The result of a single automation run.
#[derive(Debug, Clone, Copy)]
pub struct RunOutcome {
    /// Why the run stopped.
    pub stop_reason: StopReason,
    /// Ticks advanced during this run.
    pub ticks: u64,
    /// Frames presented during this run.
    pub frames: u64,
    /// Indivisible-operation tick overshoot for a tick target.
    pub overshoot_ticks: u64,
    /// The timeline after this run, with epoch-relative fields filled.
    pub timeline: AutomationTimeline,
}

/// Why an automation run stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The requested target was reached.
    TargetReached,
    /// The maximum tick fallback was exhausted before the target.
    TickLimit,
    /// The guest requested a system shutdown.
    GuestShutdown,
    /// The run was cancelled by the session between chunks.
    Cancelled,
    /// A total-session budget was exhausted.
    CounterExhausted,
    /// The machine reported an unrecoverable error.
    MachineError,
}

/// Low-level primitives a machine family provides so the shared automation run
/// loop can drive it uniformly.
///
/// This is an internal helper contract, not part of the machine-neutral
/// [`AutomatedMachine`] boundary. Each family implements it for the trace-sink
/// monomorph and defers `run_automation` to [`drive_automation`].
pub trait AutomationDriver {
    /// Arms an exact stop at absolute epoch frame `target_frame`.
    fn arm_presentation_yield(&mut self, target_frame: u64);

    /// Disarms any pending presentation-boundary stop.
    fn disarm_presentation_yield(&mut self);

    /// Returns the ticks advanced within the current epoch.
    fn epoch_ticks(&self) -> u64;

    /// Returns the frames presented within the current epoch.
    fn epoch_frames(&self) -> u64;

    /// Advances the machine by up to `budget` ticks, returning ticks consumed.
    fn run_for(&mut self, budget: u64) -> u64;

    /// Returns whether the guest requested a system shutdown.
    fn shutdown_requested(&self) -> bool;

    /// Generates and discards audio covering `elapsed_ticks` for determinism.
    fn drain_audio(&mut self, elapsed_ticks: u64);
}

/// Drives an [`AutomationDriver`] through one bounded run request.
///
/// Frame targets arm an exact presentation stop and drain audio once per
/// presentation. Tick targets run in bounded chunks, draining each, and may
/// overshoot the final target by one indivisible operation.
pub fn drive_automation<D>(driver: &mut D, request: RunRequest) -> RunOutcome
where
    D: AutomationDriver + ?Sized,
{
    let start_ticks = driver.epoch_ticks();
    let start_frames = driver.epoch_frames();

    let (stop_reason, tick_target) = match request.target {
        RunTarget::Ticks(count) => {
            let target = start_ticks.saturating_add(count);
            (
                drive_ticks(driver, target, request.audio_drain_interval_ticks),
                Some(target),
            )
        }
        RunTarget::Frames(count) => {
            let target = start_frames.saturating_add(count);
            (
                drive_frames(driver, target, start_ticks, request.max_ticks),
                None,
            )
        }
        RunTarget::UntilSessionFrame(target) => {
            // The session lowers absolute session frames to an epoch frame
            // target before dispatch; treat the value as that epoch target.
            let target = u64::try_from(target).unwrap_or(u64::MAX);
            (
                drive_frames(driver, target, start_ticks, request.max_ticks),
                None,
            )
        }
    };

    driver.disarm_presentation_yield();

    let end_ticks = driver.epoch_ticks();
    let end_frames = driver.epoch_frames();
    let overshoot_ticks = match tick_target {
        Some(target) => end_ticks.saturating_sub(target),
        None => 0,
    };

    RunOutcome {
        stop_reason,
        ticks: end_ticks.saturating_sub(start_ticks),
        frames: end_frames.saturating_sub(start_frames),
        overshoot_ticks,
        timeline: AutomationTimeline {
            epoch_ticks: end_ticks as u128,
            epoch_frames: end_frames as u128,
            ..AutomationTimeline::default()
        },
    }
}

/// Runs bounded tick chunks up to `tick_target`, draining audio per chunk.
fn drive_ticks<D>(driver: &mut D, tick_target: u64, drain_interval: u64) -> StopReason
where
    D: AutomationDriver + ?Sized,
{
    let interval = drain_interval.max(1);
    loop {
        let current = driver.epoch_ticks();
        if current >= tick_target {
            return StopReason::TargetReached;
        }
        if driver.shutdown_requested() {
            return StopReason::GuestShutdown;
        }
        let budget = (tick_target - current).min(interval);
        let consumed = driver.run_for(budget);
        driver.drain_audio(consumed);
        if consumed == 0 && driver.epoch_ticks() == current {
            return StopReason::TargetReached;
        }
    }
}

/// Runs until `target_frame` epoch frames are presented or `max_ticks` elapse.
fn drive_frames<D>(
    driver: &mut D,
    target_frame: u64,
    start_ticks: u64,
    max_ticks: u64,
) -> StopReason
where
    D: AutomationDriver + ?Sized,
{
    let tick_limit = start_ticks.saturating_add(max_ticks);
    loop {
        let frames_now = driver.epoch_frames();
        if frames_now >= target_frame {
            return StopReason::TargetReached;
        }
        if driver.shutdown_requested() {
            return StopReason::GuestShutdown;
        }
        let current_ticks = driver.epoch_ticks();
        if current_ticks >= tick_limit {
            return StopReason::TickLimit;
        }
        driver.arm_presentation_yield(frames_now + 1);
        let consumed = driver.run_for(tick_limit - current_ticks);
        driver.drain_audio(consumed);
        if consumed == 0 && driver.epoch_ticks() == current_ticks {
            return StopReason::TickLimit;
        }
    }
}
