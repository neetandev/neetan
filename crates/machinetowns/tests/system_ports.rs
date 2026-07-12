//! Integration tests for the FM Towns system control ports: memory waits, the
//! reset/power latches, and the memory-mapped stub devices.

#[path = "common/harness.rs"]
mod harness;

use common::{Bus, Machine};
use harness::{machine_base, machine_mx};

#[test]
fn base_model_reports_machine_id() {
    let mut machine = machine_base();
    // I/O 0x0030 is the CPU-class byte, 0x0031 the model byte.
    assert_eq!(machine.bus.io_read_byte(0x0030), 0x01);
    assert_eq!(machine.bus.io_read_byte(0x0031), 0x01);
}

#[test]
fn memory_wait_latches_read_back() {
    let mut machine = machine_mx();

    // Power-on: no waits, FASTMODE lamp lit.
    assert_eq!(machine.bus.io_read_byte(0x05E0), 0);
    assert_eq!(machine.bus.io_read_byte(0x05E2), 0);
    assert_eq!(machine.bus.io_read_byte(0x05E6), 0);
    assert_eq!(machine.bus.io_read_byte(0x05EC), 1);

    // 0x05E0 and 0x05E2 address the same main-RAM wait latch.
    machine.bus.io_write_byte(0x05E0, 3);
    assert_eq!(machine.bus.io_read_byte(0x05E2), 3);
    machine.bus.io_write_byte(0x05E2, 5);
    assert_eq!(machine.bus.io_read_byte(0x05E0), 5);

    machine.bus.io_write_byte(0x05E6, 4);
    assert_eq!(machine.bus.io_read_byte(0x05E6), 4);
}

#[test]
fn fastmode_write_drives_waits_and_lamp() {
    let mut machine = machine_mx();

    // Clearing bit 0 selects the FMR-compatible slow mode.
    machine.bus.io_write_byte(0x05EC, 0x00);
    assert_eq!(machine.bus.io_read_byte(0x05E2), 6);
    assert_eq!(machine.bus.io_read_byte(0x05E6), 6);
    assert_eq!(machine.bus.io_read_byte(0x05EC), 0);

    // Setting bit 0 removes all waits and lights the lamp again.
    machine.bus.io_write_byte(0x05EC, 0x01);
    assert_eq!(machine.bus.io_read_byte(0x05E2), 0);
    assert_eq!(machine.bus.io_read_byte(0x05E6), 0);
    assert_eq!(machine.bus.io_read_byte(0x05EC), 1);

    // The lamp goes out once the VRAM wait reaches the slow threshold.
    machine.bus.io_write_byte(0x05E6, 3);
    assert_eq!(machine.bus.io_read_byte(0x05EC), 0);
    machine.bus.io_write_byte(0x05E6, 2);
    assert_eq!(machine.bus.io_read_byte(0x05EC), 1);
    machine.bus.io_write_byte(0x05E0, 1);
    assert_eq!(machine.bus.io_read_byte(0x05EC), 0);
}

/// The programmed wait latches are charged as real wait-state cycles: video
/// memory takes the VRAM wait, everything else the main-RAM wait. A wide access
/// costs one wait, not one per byte, and the counter drains to zero.
#[test]
fn memory_wait_latches_charge_access_cycles() {
    let mut machine = machine_mx();
    const MAIN_RAM: u32 = 0x0000_1000;
    const VRAM_LINEAR: u32 = 0x8000_0000;

    // Distinct RAM and VRAM waits so a mis-classified access is visible.
    machine.bus.io_write_byte(0x05E0, 2);
    machine.bus.io_write_byte(0x05E6, 5);

    let _ = machine.bus.read_byte(MAIN_RAM);
    assert_eq!(machine.bus.drain_wait_cycles(), 2);

    machine.bus.write_byte(MAIN_RAM, 0);
    assert_eq!(machine.bus.drain_wait_cycles(), 2);

    // Video memory also carries the always-on baseline (2) on top of the latch.
    let _ = machine.bus.read_byte(VRAM_LINEAR);
    assert_eq!(machine.bus.drain_wait_cycles(), 2 + 5);

    // A dword access is a single bus cycle: one wait, not four.
    let _ = machine.bus.read_dword(MAIN_RAM);
    assert_eq!(machine.bus.drain_wait_cycles(), 2);
    machine.bus.write_dword(VRAM_LINEAR, 0);
    assert_eq!(machine.bus.drain_wait_cycles(), 2 + 5);

    // Draining leaves the counter empty.
    assert_eq!(machine.bus.drain_wait_cycles(), 0);
}

/// Fast mode (both wait latches zero) charges no wait-state cycles for RAM, but
/// video memory still carries the always-on VRAM baseline penalty.
#[test]
fn fast_mode_charges_only_vram_baseline() {
    let mut machine = machine_mx();
    machine.bus.io_write_byte(0x05EC, 0x01);

    let _ = machine.bus.read_byte(0x0000_1000);
    machine.bus.write_word(0x0000_1000, 0);
    assert_eq!(machine.bus.drain_wait_cycles(), 0);

    let _ = machine.bus.read_dword(0x8000_0000);
    assert_eq!(machine.bus.drain_wait_cycles(), 2);
}

/// FMR-compatible slow mode (FASTMODE bit 0 clear) programs both latches to the
/// slow value, so subsequent accesses are penalized.
#[test]
fn slow_mode_penalizes_accesses() {
    let mut machine = machine_mx();
    machine.bus.io_write_byte(0x05EC, 0x00);

    let _ = machine.bus.read_byte(0x0000_1000);
    let _ = machine.bus.read_byte(0x8000_0000);
    // RAM charges the slow latch (6); VRAM charges the baseline (2) plus it (6).
    assert_eq!(machine.bus.drain_wait_cycles(), 6 + (2 + 6));
}

/// The reset-reason port (0x0020) latches a software reset, reads it back, and
/// self-clears; the pending reset is reflected by `reset_pending`.
#[test]
fn reset_reason_reports_and_clears_software_reset() {
    let mut machine = machine_mx();
    assert_eq!(machine.bus.io_read_byte(0x0020), 0x00);
    machine.bus.io_write_byte(0x0020, 0x01);
    assert!(machine.bus.reset_pending());
    assert_eq!(machine.bus.io_read_byte(0x0020) & 0x01, 0x01);
    // Read-to-clear.
    assert_eq!(machine.bus.io_read_byte(0x0020) & 0x03, 0x00);
}

/// A power-off request (0x0022 bit 6) raises the machine shutdown signal.
#[test]
fn power_off_request_sets_shutdown() {
    let mut machine = machine_mx();
    assert!(!machine.shutdown_requested());
    machine.bus.io_write_byte(0x0022, 0x40);
    assert!(machine.shutdown_requested());
    assert!(machine.bus.reset_pending());
}

/// The run loop consumes a pending soft reset instead of leaving it latched: the
/// reset request clears once the loop has acted on it.
#[test]
fn soft_reset_request_is_consumed_by_run_loop() {
    let mut machine = machine_mx();
    machine.bus.io_write_byte(0x0020, 0x01);
    assert!(machine.bus.reset_pending());
    machine.run_for(10_000);
    assert!(!machine.bus.reset_pending());
}

/// The CD-ROM cache/2x-speed and subcode ports return benign, non-decoded values
/// rather than open-bus reads; their writes are dropped.
#[test]
fn cdrom_stub_ports_return_benign_values() {
    let mut machine = machine_mx();
    assert_eq!(machine.bus.io_read_byte(0x04C8), 0xFF);
    assert_eq!(machine.bus.io_read_byte(0x04CC), 0x00);
    assert_eq!(machine.bus.io_read_byte(0x04CD), 0x00);
    machine.bus.io_write_byte(0x04CC, 0xFF);
    machine.bus.io_write_byte(0x04CD, 0xFF);
    assert_eq!(machine.bus.io_read_byte(0x04CC), 0x00);
}

/// The sound sampling (ADC) stub reports a ready sample of silence.
#[test]
fn sound_sampling_stub_ports() {
    let mut machine = machine_mx();
    assert_eq!(machine.bus.io_read_byte(0x04E7), 0x80);
    assert_eq!(machine.bus.io_read_byte(0x04E8), 0x01);
    machine.bus.io_write_byte(0x04E7, 0x00);
    machine.bus.io_write_byte(0x04E8, 0x00);
    assert_eq!(machine.bus.io_read_byte(0x04E8), 0x01);
}

/// With no memory card inserted, the status reports "no card present", the bank
/// latch round-trips through its bit 4-5 shift, and the attribute reports the
/// absent-card bit 7 plus the register-select latch.
#[test]
fn memory_card_ports_report_no_card() {
    let mut machine = machine_mx();
    assert_eq!(machine.bus.io_read_byte(0x048A), 0x06);
    machine.bus.io_write_byte(0x0490, 0x30);
    assert_eq!(machine.bus.io_read_byte(0x0490), 0x30);
    machine.bus.io_write_byte(0x0490, 0x10);
    assert_eq!(machine.bus.io_read_byte(0x0490), 0x10);
    assert_eq!(machine.bus.io_read_byte(0x0491), 0x80);
    machine.bus.io_write_byte(0x0491, 0x01);
    assert_eq!(machine.bus.io_read_byte(0x0491), 0x81);
}
