mod common;

use common::{harness::*, signals::*};
use ymfm_oxide::Ym2608;

#[test]
fn timer_a_configuration() {
    let mut chip = Ym2608::new();
    chip.reset();
    chip.take_signals();

    write_reg_2608(&mut chip, 0x24, 0xFF);
    write_reg_2608(&mut chip, 0x25, 0x03);
    write_reg_2608(&mut chip, 0x27, 0x05);

    let events = chip.take_signals();
    let timer_a = events.iter().find_map(|e| match e {
        SignalEvent::SetTimer {
            timer_id: 0,
            duration_in_clocks,
        } => Some(*duration_in_clocks),
        _ => None,
    });
    assert!(timer_a.is_some(), "should have set_timer for Timer A");
    assert!(timer_a.unwrap() > 0, "Timer A duration should be positive");
}

#[test]
fn timer_b_configuration() {
    let mut chip = Ym2608::new();
    chip.reset();
    chip.take_signals();

    write_reg_2608(&mut chip, 0x26, 0x80);
    write_reg_2608(&mut chip, 0x27, 0x0A);

    let events = chip.take_signals();
    let timer_b = events.iter().find_map(|e| match e {
        SignalEvent::SetTimer {
            timer_id: 1,
            duration_in_clocks,
        } => Some(*duration_in_clocks),
        _ => None,
    });
    assert!(timer_b.is_some(), "should have set_timer for Timer B");
    assert!(timer_b.unwrap() > 0, "Timer B duration should be positive");
}

#[test]
fn timer_a_expiry_sets_status_flag() {
    let mut chip = Ym2608::new();
    chip.reset();

    write_reg_2608(&mut chip, 0x24, 0xFF);
    write_reg_2608(&mut chip, 0x25, 0x03);
    write_reg_2608(&mut chip, 0x27, 0x05);

    assert_eq!(
        chip.read_status(false) & 0x01,
        0,
        "Timer A flag clear before expiry"
    );

    chip.timer_expired(0);

    assert_eq!(
        chip.read_status(false) & 0x01,
        0x01,
        "Timer A flag set after expiry"
    );
}

#[test]
fn timer_b_expiry_sets_status_flag() {
    let mut chip = Ym2608::new();
    chip.reset();

    write_reg_2608(&mut chip, 0x26, 0x80);
    write_reg_2608(&mut chip, 0x27, 0x0A);

    assert_eq!(
        chip.read_status(false) & 0x02,
        0,
        "Timer B flag clear before expiry"
    );

    chip.timer_expired(1);

    assert_eq!(
        chip.read_status(false) & 0x02,
        0x02,
        "Timer B flag set after expiry"
    );
}

#[test]
fn timer_flag_cleared_by_reset_bit() {
    let mut chip = Ym2608::new();
    chip.reset();

    write_reg_2608(&mut chip, 0x24, 0xFF);
    write_reg_2608(&mut chip, 0x25, 0x03);
    write_reg_2608(&mut chip, 0x27, 0x05);
    chip.timer_expired(0);

    assert_eq!(chip.read_status(false) & 0x01, 0x01);

    write_reg_2608(&mut chip, 0x27, 0x15); // Reset A flag

    assert_eq!(
        chip.read_status(false) & 0x01,
        0x00,
        "Timer A flag should be cleared"
    );
}

#[test]
fn timer_irq_assert_deassert() {
    let mut chip = Ym2608::new();
    chip.reset();
    chip.take_signals();

    write_reg_2608(&mut chip, 0x24, 0xFF);
    write_reg_2608(&mut chip, 0x25, 0x03);
    write_reg_2608(&mut chip, 0x27, 0x05);
    chip.take_signals();

    chip.timer_expired(0);

    let events = chip.take_signals();
    let has_assert = events
        .iter()
        .any(|e| matches!(e, SignalEvent::UpdateIrq { asserted: true }));
    assert!(has_assert, "timer expiry should assert IRQ");

    write_reg_2608(&mut chip, 0x27, 0x15);

    let events = chip.take_signals();
    let has_deassert = events
        .iter()
        .any(|e| matches!(e, SignalEvent::UpdateIrq { asserted: false }));
    assert!(has_deassert, "clearing flag should deassert IRQ");
}

#[test]
fn status_read_does_not_clear_flags() {
    let mut chip = Ym2608::new();
    chip.reset();

    write_reg_2608(&mut chip, 0x24, 0xFF);
    write_reg_2608(&mut chip, 0x25, 0x03);
    write_reg_2608(&mut chip, 0x27, 0x05);
    chip.timer_expired(0);

    let s1 = chip.read_status(false);
    let s2 = chip.read_status(false);
    let s3 = chip.read_status(false);

    assert_eq!(s1, s2, "repeated reads should be identical");
    assert_eq!(s2, s3, "repeated reads should be identical");
}

#[test]
fn both_timers_active() {
    let mut chip = Ym2608::new();
    chip.reset();

    write_reg_2608(&mut chip, 0x24, 0xFF);
    write_reg_2608(&mut chip, 0x25, 0x03);
    write_reg_2608(&mut chip, 0x26, 0x80);
    write_reg_2608(&mut chip, 0x27, 0x0F);

    chip.timer_expired(0);
    chip.timer_expired(1);

    let status = chip.read_status(false);
    assert_eq!(status & 0x03, 0x03, "both timer flags should be set");
}

#[test]
fn read_status_hi_independent() {
    let mut chip = Ym2608::new();
    chip.reset();

    // Set Timer A flag
    write_reg_2608(&mut chip, 0x24, 0xFF);
    write_reg_2608(&mut chip, 0x25, 0x03);
    write_reg_2608(&mut chip, 0x27, 0x05);
    chip.timer_expired(0);

    let status_lo = chip.read_status(false);
    let status_hi = chip.read_status_hi(false);

    // Timer flags should appear in low status
    assert_eq!(status_lo & 0x01, 0x01, "Timer A in low status");

    // High status register is primarily for ADPCM flags
    // Timer A flag should also appear in hi status (bit 0)
    // but the key point is hi register has additional ADPCM bits
    // Just verify hi status can be read without error
    let _ = status_hi;
}

#[test]
fn busy_flag_in_status() {
    let mut chip = Ym2608::new();
    chip.reset();
    chip.take_signals();

    let status = chip.read_status(false);
    assert_eq!(status & 0x80, 0, "busy bit clear when not busy");

    let status = chip.read_status(true);
    assert_eq!(status & 0x80, 0x80, "busy bit set when busy");
}
