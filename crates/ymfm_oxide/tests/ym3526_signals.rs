mod common;

use common::{harness::*, signals::*};
use ymfm_oxide::Ym3526;

#[test]
fn reset_triggers_timer_signals() {
    let mut chip = Ym3526::new();
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
    let mut chip = Ym3526::new();
    chip.reset();
    chip.take_signals();

    chip.write_address(0x20);
    let busy_clocks = chip.write_data(0x00);

    assert!(
        busy_clocks > 0,
        "register write should return a busy duration"
    );
}

#[test]
fn status_register_reflects_irq_flag() {
    let mut chip = Ym3526::new();
    chip.reset();

    // OPL status bit 7 = IRQ flag (OR of unmasked timer flags)
    let status_before = chip.read_status();
    assert_eq!(
        status_before & 0x80,
        0,
        "IRQ flag should be clear initially"
    );

    // Trigger Timer A to set IRQ
    write_reg_opl(&mut chip, 0x02, 0xFF);
    write_reg_opl(&mut chip, 0x04, 0x01);
    chip.timer_expired(0);

    let status_after = chip.read_status();
    assert_eq!(
        status_after & 0x80,
        0x80,
        "IRQ flag (bit 7) should be set when timer flag is active"
    );
}

#[test]
fn multiple_register_writes_each_trigger_busy() {
    let mut chip = Ym3526::new();
    chip.reset();
    chip.take_signals();

    let mut busy_count = 0;
    for addr in [0x20, 0x40, 0x60, 0x80, 0xE0] {
        if chip.write_address(addr) > 0 {
            busy_count += 1;
        }
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
    let mut chip = Ym3526::new();
    chip.reset();
    chip.take_signals();

    let busy1 = chip.write_address(0x02);
    let busy2 = chip.write_data(0xFF);
    let busy3 = chip.write_address(0x04);
    let busy4 = chip.write_data(0x01);

    let events = chip.take_signals();

    let has_busy = busy1 > 0 && busy2 > 0 && busy3 > 0 && busy4 > 0;
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
    let mut chip = Ym3526::new();
    chip.reset();

    setup_opl_simple_tone(&mut chip, 0, 1, 0);
    key_on_opl(&mut chip, 0);

    let samples = generate_1_opl(&mut chip, 256);
    assert_eq!(samples.len(), 256);

    let has_nonzero = samples.iter().any(|s| s[0] != 0);
    assert!(has_nonzero, "FM output should be non-zero after key-on");
}
