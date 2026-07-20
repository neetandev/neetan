//! Generic tracing end to end.
//!
//! Drives the `pc98-trace.scm` fixture through `execute_script`, exercising the
//! full stack: the `(neetan trace 1)` natives, the declarative-filter compiler,
//! continuous collection and drain, exclusive `wait-for-event`, the
//! `neetan/trace-state` guard, up-front filter validation, and bounded-queue
//! overflow reported as `neetan/trace-overflow`. Running the fixture twice under
//! the fixed guest clock also checks that tracing stays deterministic.

#[path = "common/harness.rs"]
mod harness;

use automation::{ExecutionResult, RunTermination};
use harness::run_committed_script;

#[test]
fn pc98_trace_script_passes() {
    let run = run_committed_script("pc98-trace.scm", 120);
    assert!(
        matches!(
            run.termination,
            RunTermination::Completed(ExecutionResult::Ok)
        ),
        "trace script did not pass: {:?}",
        run.termination
    );
}

#[test]
fn pc98_trace_script_is_deterministic() {
    let first = run_committed_script("pc98-trace.scm", 120);
    let second = run_committed_script("pc98-trace.scm", 120);
    assert!(matches!(
        first.termination,
        RunTermination::Completed(ExecutionResult::Ok)
    ));
    assert!(matches!(
        second.termination,
        RunTermination::Completed(ExecutionResult::Ok)
    ));
    assert_eq!(first.exit_code, second.exit_code);
}
