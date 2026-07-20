//! Runtime save states: capture, restore, and discard round-trip, and handles
//! are invalidated when the machine is reconstructed.

#[path = "common/harness.rs"]
mod harness;

use harness::{assert_completed_ok, run_committed_script};

#[test]
fn runtime_save_state_round_trip() {
    let run = run_committed_script("save-state.scm", 60);
    assert_completed_ok(&run.termination);
}
