//! Reset and restore: a hard reset advances the automation epoch and rewinds the
//! epoch-relative counters, and `restore-startup!` reconstructs from the startup
//! specification.

#[path = "common/harness.rs"]
mod harness;

use harness::{assert_completed_ok, run_committed_script};

#[test]
fn reset_and_restore_manage_the_epoch() {
    let run = run_committed_script("reset-restore.scm", 60);
    assert_completed_ok(&run.termination);
}
