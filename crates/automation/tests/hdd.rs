//! In-memory hard disk synthesis and formatting through the Scheme API.
//!
//! `create-hdd!` builds a zeroed image in memory and mounts it RAM-backed, and
//! `format-hdd!` lays down a partition table and empty FAT volume in place. The
//! committed script drives the full Scheme wrapper, native, and session path,
//! and confirms the formatted disk survives a hard reset. Byte-level layout
//! correctness is covered by the `device` crate unit tests.

#[path = "common/harness.rs"]
mod harness;

use harness::{assert_completed_ok, run_committed_script};

#[test]
fn create_format_and_reset_in_memory_hdd() {
    let run = run_committed_script("hdd-create.scm", 60);
    assert_completed_ok(&run.termination);
}
