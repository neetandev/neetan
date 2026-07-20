//! Logical input: unsupported controls and characters raise explicit errors
//! before any input is injected.

#[path = "common/harness.rs"]
mod harness;

use harness::{assert_completed_ok, run_committed_script};

#[test]
fn input_error_contract_holds() {
    let run = run_committed_script("input-errors.scm", 60);
    assert_completed_ok(&run.termination);
}

#[test]
fn key_tap_options_contract_holds() {
    let run = run_committed_script("key-tap.scm", 60);
    assert_completed_ok(&run.termination);
}
