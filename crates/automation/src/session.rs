//! The per-thread automation session shared with native callbacks.
//!
//! Native callbacks capture an `Rc<RefCell<AutomationSession>>`. The session
//! owns the `MessageProtocol` sender and the cancel handle, so a native can emit
//! output, record the authoritative result, and observe cancellation without
//! re-entering Scheme. It also owns the constructed machine, the reconstruction
//! epoch, the session-total tick and frame bases, and the total-session budgets.
//!
//! This module defines the session state, its shared error and budget types, and
//! the core lifecycle accessors. The larger method groups live in thematic
//! submodules, each holding its own `impl AutomationSession` block:
//! [`run`] (bounded execution), [`lifecycle`] (construction and save states),
//! [`media`] (mounting), [`input`] (keys, joystick, mouse), [`screen`] (display
//! reads), and [`inspect`] (register and memory inspection and mutation).

mod input;
mod inspect;
mod lifecycle;
mod media;
mod run;
mod screen;
pub(crate) mod trace;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::mpsc::Sender,
};

use common::{
    AutomatedMachine, AutomationTimeline, HostKey, InputCapabilities, JoystickState,
    MachineStateBlob, SharedHostDateTimeSource, StartupCapabilities,
    tracing::{TraceFailure, TraceHandle},
};
use machine_factory::{InitError, InitErrorKind, config::EmulatorConfig};

use crate::{
    config::CommonConfig,
    media::{MediaKind, MediaMount, MediaRequest},
    protocol::{ExecutionResult, MessageProtocol, TestCaseOutcome},
    watchdog::CancelHandle,
};

/// Audio drain interval used for input-driven runs, in ticks.
pub(crate) const INPUT_DRAIN_INTERVAL_TICKS: u64 = 1_000_000;

/// Largest public exact integer, matching the signed 128-bit R7RS integers.
pub(crate) const PUBLIC_INTEGER_MAX: u128 = i128::MAX as u128;

/// Total-session budgets enforced between bounded machine chunks.
///
/// `None` means unbounded. Ticks and frames are checked by the run wrapper;
/// native-call and artifact-byte counters are decremented by their callbacks.
#[derive(Clone, Copy, Debug, Default)]
pub struct SessionBudgets {
    /// Remaining total ticks, or `None` for unbounded.
    pub ticks: Option<u128>,
    /// Remaining total frames, or `None` for unbounded.
    pub frames: Option<u128>,
    /// Remaining native calls, or `None` for unbounded.
    pub native_calls: Option<u64>,
    /// Remaining artifact bytes, or `None` for unbounded.
    pub artifact_bytes: Option<u128>,
}

/// Why a session run could not proceed or complete cleanly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunError {
    /// No machine has been constructed yet.
    NoMachine,
    /// A public tick, frame, or value would exceed the signed 128-bit range.
    Range,
    /// An active trace collector exhausted its bounded queue during the run.
    TraceOverflow,
}

/// Why a Scheme-facing session operation failed.
///
/// Each variant maps to a stable `neetan/*` error symbol the Scheme wrapper
/// raises.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpError {
    /// No machine has been constructed yet.
    NoMachine,
    /// A machine operation conflicts with the active machine lifecycle.
    MachineState(String),
    /// An opaque machine or save-state handle is no longer active.
    StaleHandle(String),
    /// The operation or control is not supported by this machine.
    Unsupported(String),
    /// An argument was out of the accepted set.
    Argument(String),
    /// A value exceeded the accepted range.
    Range,
    /// A path resolved outside its allowed root.
    PathEscape(String),
    /// A host filesystem operation failed.
    Io(String),
    /// A trace collector exhausted its bounded queue.
    TraceOverflow(String),
    /// A trace operation was invalid for the current collection state.
    TraceState(String),
    /// The guest requested a system shutdown during the operation.
    GuestShutdown,
    /// Machine construction failed.
    Construction(InitError),
}

impl OpError {
    /// Returns the stable `neetan/*` error symbol for this failure.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            OpError::NoMachine => "neetan/no-machine",
            OpError::MachineState(_) => "neetan/machine-state",
            OpError::StaleHandle(_) => "neetan/stale-handle",
            OpError::Unsupported(_) => "neetan/unsupported",
            OpError::Argument(_) => "neetan/argument",
            OpError::Range => "neetan/range",
            OpError::PathEscape(_) => "neetan/path-escape",
            OpError::Io(_) => "neetan/io",
            OpError::TraceOverflow(_) => "neetan/trace-overflow",
            OpError::TraceState(_) => "neetan/trace-state",
            OpError::GuestShutdown => "neetan/guest-shutdown",
            OpError::Construction(error) => match error.kind {
                InitErrorKind::BadSpec => "neetan/argument",
                InitErrorKind::RomMissing | InitErrorKind::Io => "neetan/io",
                InitErrorKind::Unsupported => "neetan/unsupported",
            },
        }
    }

    /// Returns the human-readable message for this failure.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            OpError::NoMachine => "no machine has been constructed".to_owned(),
            OpError::Unsupported(message)
            | OpError::MachineState(message)
            | OpError::StaleHandle(message)
            | OpError::Argument(message)
            | OpError::PathEscape(message)
            | OpError::Io(message)
            | OpError::TraceOverflow(message)
            | OpError::TraceState(message) => message.clone(),
            OpError::GuestShutdown => "guest requested shutdown".to_owned(),
            OpError::Range => "value is out of range".to_owned(),
            OpError::Construction(error) => error.message.clone(),
        }
    }

    /// Maps a bounded-run error to an operation error.
    fn from_run(error: RunError) -> Self {
        match error {
            RunError::NoMachine => OpError::NoMachine,
            RunError::Range => OpError::Range,
            RunError::TraceOverflow => {
                OpError::TraceOverflow("trace collector exhausted its bounded queue".to_owned())
            }
        }
    }
}

/// The set of controls automation is currently holding down.
#[derive(Default)]
struct TrackedControls {
    keys_down: BTreeSet<HostKey>,
    joysticks: BTreeMap<usize, JoystickState>,
    mouse_buttons: (bool, bool, bool),
}

/// Monotonic identity of one logical machine scope.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct MachineId(u64);

impl MachineId {
    /// Returns the private integer token passed through Scheme.
    #[must_use]
    pub(crate) fn token(self) -> u64 {
        self.0
    }
}

/// Monotonic identity of one runtime save state.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct StateId(u64);

impl StateId {
    /// Returns the private integer token passed through Scheme.
    #[must_use]
    pub(crate) fn token(self) -> u64 {
        self.0
    }
}

/// A captured runtime state together with its owning logical machine.
struct RuntimeState {
    owner: MachineId,
    blob: MachineStateBlob,
}

/// Checked monotonic allocator for private resource identities.
#[derive(Debug)]
struct IdAllocator {
    next: Option<u64>,
}

impl IdAllocator {
    /// Creates an allocator whose first identity is one.
    fn new() -> Self {
        Self { next: Some(1) }
    }

    /// Allocates one identity and permanently exhausts after `u64::MAX`.
    fn allocate(&mut self) -> Result<u64, OpError> {
        let id = self.next.ok_or(OpError::Range)?;
        self.next = id.checked_add(1);
        Ok(id)
    }
}

/// Every resource owned by one active logical machine scope.
struct ActiveMachine {
    id: MachineId,
    machine: Box<dyn AutomatedMachine>,
    epoch: u64,
    session_ticks_base: u128,
    session_frames_base: u128,
    startup_spec: EmulatorConfig,
    mounts: BTreeMap<(MediaKind, usize), MediaMount>,
    startup_media: Vec<MediaRequest>,
    tracked: TrackedControls,
    runtime_states: BTreeMap<StateId, RuntimeState>,
    trace: TraceHandle,
    trace_failure: Option<TraceFailure>,
}

/// A terminal exit request produced by Scheme `exit` or `emergency-exit`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExitRequest {
    /// The requested exit code, when representable.
    pub code: Option<i64>,
    /// Whether `dynamic-wind` cleanup was bypassed.
    pub emergency: bool,
}

/// State shared between the executor thread and its native callbacks.
pub struct AutomationSession {
    events: Sender<MessageProtocol>,
    cancel: CancelHandle,
    result: Option<ExecutionResult>,
    exit: Option<ExitRequest>,
    active: Option<ActiveMachine>,
    /// Next logical machine identity. Identities are never reused in one session.
    machine_ids: IdAllocator,
    budgets: SessionBudgets,
    common: CommonConfig,
    factory_rtc: SharedHostDateTimeSource,
    sample_rate: u32,
    /// Root beneath which script-relative media sources resolve.
    read_root: PathBuf,
    /// Root beneath which writable artifacts (including printer output) resolve.
    artifact_root: PathBuf,
    /// Next save-state identity to allocate. Monotonic, never reused; the base is
    /// 1 so 0 is never a valid handle.
    state_ids: IdAllocator,
}

impl AutomationSession {
    /// Creates a session with no recorded result and no machine.
    #[must_use]
    pub fn new(
        events: Sender<MessageProtocol>,
        cancel: CancelHandle,
        common: CommonConfig,
        factory_rtc: SharedHostDateTimeSource,
        sample_rate: u32,
        read_root: PathBuf,
        artifact_root: PathBuf,
    ) -> Self {
        Self {
            events,
            cancel,
            result: None,
            exit: None,
            active: None,
            machine_ids: IdAllocator::new(),
            budgets: SessionBudgets::default(),
            common,
            factory_rtc,
            sample_rate,
            read_root,
            artifact_root,
            state_ids: IdAllocator::new(),
        }
    }

    /// Returns whether a machine has been constructed.
    #[must_use]
    pub fn has_machine(&self) -> bool {
        self.active.is_some()
    }

    /// Returns whether tracing is available.
    ///
    /// The automated trace sink is installed for every constructed machine, so
    /// tracing is supported whenever a machine is present.
    #[must_use]
    pub fn supports_tracing(&self) -> bool {
        self.active.is_some()
    }

    /// Returns the current reconstruction epoch.
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.active.as_ref().map_or(0, |active| active.epoch)
    }

    /// Sets the total-session budgets.
    pub fn set_budgets(&mut self, budgets: SessionBudgets) {
        self.budgets = budgets;
    }

    /// Returns the full timeline overlaying the epoch and session-total bases on
    /// the machine's epoch-relative counters.
    #[must_use]
    pub fn timeline(&self) -> AutomationTimeline {
        let Some(active) = self.active.as_ref() else {
            return AutomationTimeline::default();
        };
        let (epoch_ticks, epoch_frames) = {
            let timeline = active.machine.automation_timeline();
            (timeline.epoch_ticks, timeline.epoch_frames)
        };
        AutomationTimeline {
            epoch: active.epoch,
            session_ticks: active.session_ticks_base.saturating_add(epoch_ticks),
            session_frames: active.session_frames_base.saturating_add(epoch_frames),
            epoch_ticks,
            epoch_frames,
        }
    }

    /// Returns the constructed machine's descriptor, when present.
    #[must_use]
    pub fn descriptor(&self) -> Option<common::AutomationDescriptor> {
        self.active
            .as_ref()
            .map(|active| active.machine.automation_descriptor())
    }

    /// Returns the emulated time in nanoseconds from the session-total ticks and
    /// the machine's rational timebase.
    #[must_use]
    pub fn emulated_time_ns(&self) -> u128 {
        let Some(active) = self.active.as_ref() else {
            return 0;
        };
        let timebase = active.machine.automation_descriptor().timebase;
        let numerator = timebase.ticks_per_second_numerator.max(1) as u128;
        let denominator = timebase.ticks_per_second_denominator as u128;
        self.timeline()
            .session_ticks
            .saturating_mul(denominator)
            .saturating_mul(1_000_000_000)
            / numerator
    }

    /// Records the authoritative result. The first call wins and is terminal.
    ///
    /// Returns `true` when the result was accepted and a `Result` message was
    /// emitted, or `false` when a result was already set. The caller's Scheme
    /// wrapper raises `neetan/result-state` on `false`.
    pub fn record_result(&mut self, result: ExecutionResult) -> bool {
        if self.result.is_some() {
            return false;
        }
        self.result = Some(result.clone());
        let _ = self.events.send(MessageProtocol::Result(result));
        true
    }

    /// Returns the recorded result, when one is set.
    #[must_use]
    pub fn result(&self) -> Option<&ExecutionResult> {
        self.result.as_ref()
    }

    /// Emits captured Scheme output.
    pub fn emit_output(&self, text: String) {
        let _ = self.events.send(MessageProtocol::Output(text));
    }

    /// Emits the structured outcome of one Scheme test case.
    pub fn emit_test_case_result(
        &self,
        suite: String,
        test_case: String,
        outcome: TestCaseOutcome,
    ) {
        let _ = self.events.send(MessageProtocol::TestCaseFinished {
            suite,
            test_case,
            outcome,
        });
    }

    /// Records a controlled exit request from the process context.
    pub fn record_exit(&mut self, code: Option<i64>, emergency: bool) {
        if self.exit.is_none() {
            self.exit = Some(ExitRequest { code, emergency });
        }
    }

    /// Returns the recorded exit request, when one is set.
    #[must_use]
    pub fn exit_request(&self) -> Option<ExitRequest> {
        self.exit
    }

    /// Returns whether the run was cancelled or its deadline tripped.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.cancel.deadline_tripped() || self.cancel.cancel_requested()
    }

    /// Returns the exposed common host settings.
    #[must_use]
    pub fn common_config(&self) -> &CommonConfig {
        &self.common
    }

    /// Returns whether the guest has requested shutdown.
    #[must_use]
    pub fn shutdown_requested(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.machine.shutdown_requested())
    }

    /// Returns the machine's startup media capabilities, when present.
    #[must_use]
    pub fn media_capabilities(&self) -> Option<StartupCapabilities> {
        self.active
            .as_ref()
            .map(|active| active.machine.startup_capabilities())
    }

    /// Returns the machine's logical input capabilities, when present.
    #[must_use]
    pub fn input_capabilities(&self) -> Option<InputCapabilities> {
        self.descriptor().map(|descriptor| descriptor.input)
    }

    /// Charges one native call against the budget, returning whether it was
    /// within the remaining allowance.
    pub fn charge_native_call(&mut self) -> bool {
        if let Some(remaining) = self.budgets.native_calls.as_mut() {
            if *remaining == 0 {
                return false;
            }
            *remaining -= 1;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{IdAllocator, OpError};

    #[test]
    fn identity_allocator_uses_last_token_then_exhausts() {
        let mut allocator = IdAllocator {
            next: Some(u64::MAX),
        };
        assert_eq!(allocator.allocate(), Ok(u64::MAX));
        assert_eq!(allocator.allocate(), Err(OpError::Range));
        assert_eq!(allocator.allocate(), Err(OpError::Range));
        assert_eq!(OpError::Range.symbol(), "neetan/range");
    }

    #[test]
    fn machine_and_state_identity_domains_exhaust_independently() {
        let mut machine_ids = IdAllocator {
            next: Some(u64::MAX),
        };
        let mut state_ids = IdAllocator {
            next: Some(u64::MAX),
        };
        assert_eq!(machine_ids.allocate(), Ok(u64::MAX));
        assert_eq!(state_ids.allocate(), Ok(u64::MAX));
        assert_eq!(machine_ids.allocate(), Err(OpError::Range));
        assert_eq!(state_ids.allocate(), Err(OpError::Range));
    }

    #[test]
    fn identity_allocator_never_reuses_tokens() {
        let mut allocator = IdAllocator::new();
        assert_eq!(allocator.allocate(), Ok(1));
        assert_eq!(allocator.allocate(), Ok(2));
        assert_eq!(allocator.allocate(), Ok(3));
    }
}
