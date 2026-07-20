//! The typed message protocol streamed from an executor thread.
//!
//! `execute_script` sends `MessageProtocol` values in order over a channel. The
//! `run` command renders them to the console. The protocol is machine-neutral:
//! machine identity and tick-based progress ride the same envelope regardless of
//! the target.

use std::path::PathBuf;

use r7rs::Diagnostic;

/// Stable identity of a constructed machine, filled from the automation
/// descriptor once the script builds a machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineIdentity {
    /// The stable target family identifier, for example `pc98`.
    pub target: String,
    /// The stable model identifier, for example `pc9801vm`.
    pub model: String,
}

/// A heartbeat describing how far a run has advanced.
///
/// Before a machine is constructed, `tick`, `frame`, and `emulated_ns` are zero
/// and only `wall_elapsed_ms` advances.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RunProgress {
    /// The current automation epoch.
    pub epoch: u64,
    /// The session-total primary scheduling tick.
    pub tick: i128,
    /// The session-total presented frame count.
    pub frame: i128,
    /// The emulated time in nanoseconds.
    pub emulated_ns: i128,
    /// Wall-clock time elapsed since the run started, in milliseconds.
    pub wall_elapsed_ms: u64,
}

/// The authoritative result a script sets through `execution-result`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionResult {
    /// The script passed.
    Ok,
    /// The script failed with an explanatory message.
    Error {
        /// The failure message.
        message: String,
    },
}

/// The outcome of one Scheme `test-case` form.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TestCaseOutcome {
    /// The test case completed successfully.
    Success,
    /// The test case raised an assertion or another catchable condition.
    Failure {
        /// The stable failure kind reported by the test library.
        kind: String,
        /// The condition message reported by the test library.
        message: String,
    },
}

/// One value of the normative message protocol.
#[derive(Clone, Debug)]
pub enum MessageProtocol {
    /// The executor started running the named script.
    Started {
        /// The script path.
        script: PathBuf,
    },
    /// A machine scope became ready after complete construction and media setup.
    MachineReady {
        /// The identity of the constructed machine.
        identity: MachineIdentity,
    },
    /// Captured Scheme output or a host note.
    Output(String),
    /// A periodic progress heartbeat.
    Progress(RunProgress),
    /// The result set by `execution-result`.
    Result(ExecutionResult),
    /// One Scheme test case completed.
    TestCaseFinished {
        /// The containing test-suite name.
        suite: String,
        /// The test-case name.
        test_case: String,
        /// The case outcome.
        outcome: TestCaseOutcome,
    },
    /// The executor finished. Always the last message.
    Finished(RunTermination),
}

/// The terminal outcome of one executor run.
#[derive(Clone, Debug)]
pub enum RunTermination {
    /// The script set a result and completed.
    Completed(ExecutionResult),
    /// The script ended without setting a result.
    NoResult,
    /// The cooperative deadline tripped.
    Timeout,
    /// External cancellation stopped the run.
    Cancelled,
    /// Command-line, configuration, or machine construction failed.
    ConfigError(String),
    /// Compilation failed with a source location.
    CompileError(Diagnostic),
    /// A runtime Scheme error escaped the script.
    RuntimeError(Diagnostic),
    /// An internal emulator, automation, or channel error occurred.
    Internal(String),
}

impl RunTermination {
    /// Returns the process exit code for this terminal outcome.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            RunTermination::Completed(ExecutionResult::Ok) => 0,
            RunTermination::Completed(ExecutionResult::Error { .. }) => 1,
            RunTermination::Timeout | RunTermination::Cancelled => 124,
            RunTermination::ConfigError(_) => 2,
            RunTermination::CompileError(_) | RunTermination::RuntimeError(_) => 3,
            RunTermination::NoResult | RunTermination::Internal(_) => 4,
        }
    }
}
