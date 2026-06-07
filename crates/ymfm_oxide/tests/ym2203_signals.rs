mod common;

use common::{harness::*, signals::*};
use ymfm_oxide::Ym2203;

#[test]
fn reset_triggers_timer_signals() {
    let mut chip = Ym2203::new();
    chip.reset();

    let events = chip.take_signals();
    let timer_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, SignalEvent::SetTimer { .. }))
        .collect();
    assert!(
        !timer_events.is_empty(),
        "reset should trigger timer signals"
    );
}

#[test]
fn register_write_returns_busy_duration() {
    let mut chip = Ym2203::new();
    chip.reset();
    chip.take_signals(); // Clear reset events

    // Write a register
    chip.write_address(0x28);
    let busy_clocks = chip.write_data(0x00);

    assert!(
        busy_clocks > 0,
        "register write should return a busy duration"
    );
}

#[test]
fn is_busy_propagates_to_status() {
    let mut chip = Ym2203::new();
    chip.reset();
    chip.take_signals();

    // When not busy, bit 7 should be clear
    let status = chip.read_status(false);
    assert_eq!(status & 0x80, 0, "busy bit should be clear when not busy");

    // When busy, bit 7 should be set
    let status = chip.read_status(true);
    assert_eq!(status & 0x80, 0x80, "busy bit should be set when busy");
}

#[test]
fn multiple_register_writes_each_trigger_busy() {
    let mut chip = Ym2203::new();
    chip.reset();
    chip.take_signals();

    // Write 5 different registers
    let mut busy_count = 0;
    for addr in [0x30, 0x40, 0x50, 0x60, 0x70] {
        chip.write_address(addr);
        if chip.write_data(0x00) > 0 {
            busy_count += 1;
        }
    }

    assert!(
        busy_count >= 5,
        "each register write should return a busy duration, got {busy_count}"
    );
}

#[test]
fn signal_and_busy_reporting() {
    let mut chip = Ym2203::new();
    chip.reset();
    chip.take_signals();

    // Enable Timer A
    chip.write_address(0x24);
    let busy1 = chip.write_data(0xFF); // Timer A high
    chip.write_address(0x25);
    let busy2 = chip.write_data(0x03); // Timer A low
    chip.write_address(0x27);
    let busy3 = chip.write_data(0x05); // Enable + load Timer A

    let events = chip.take_signals();

    // Should have busy durations (from writes) and eventually a timer signal
    let has_busy = busy1 > 0 && busy2 > 0 && busy3 > 0;
    let has_timer = events
        .iter()
        .any(|e| matches!(e, SignalEvent::SetTimer { .. }));

    assert!(has_busy, "should have busy durations from register writes");
    assert!(
        has_timer,
        "should have timer signal from Timer A configuration"
    );
}

#[test]
fn generate_does_not_crash_after_signal_setup() {
    let mut chip = Ym2203::new();
    chip.reset();

    setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
    key_on_2203(&mut chip, 0);

    let samples = generate_4(&mut chip, 256);
    assert_eq!(samples.len(), 256);

    // Should have non-zero FM output
    let has_nonzero_fm = samples.iter().any(|s| s[0] != 0);
    assert!(has_nonzero_fm, "FM output should be non-zero after key-on");
}
