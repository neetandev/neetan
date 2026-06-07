mod common;

use common::{harness::*, signals::*};
use ymfm_oxide::Ym3812;

#[test]
fn timer_a_configuration() {
    let mut chip = Ym3812::new();
    chip.reset();
    chip.take_signals();

    write_reg_opl2(&mut chip, 0x02, 0xFF);
    write_reg_opl2(&mut chip, 0x04, 0x01);

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
    let mut chip = Ym3812::new();
    chip.reset();
    chip.take_signals();

    write_reg_opl2(&mut chip, 0x03, 0x80);
    write_reg_opl2(&mut chip, 0x04, 0x02);

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
    let mut chip = Ym3812::new();
    chip.reset();

    write_reg_opl2(&mut chip, 0x02, 0xFF);
    write_reg_opl2(&mut chip, 0x04, 0x01);

    let status_before = chip.read_status();
    assert_eq!(
        status_before & 0x40,
        0,
        "Timer A flag should be clear before expiry"
    );

    chip.timer_expired(0);

    let status_after = chip.read_status();
    assert_eq!(
        status_after & 0x40,
        0x40,
        "Timer A flag (bit 6) should be set after expiry"
    );
}

#[test]
fn timer_b_expiry_sets_status_flag() {
    let mut chip = Ym3812::new();
    chip.reset();

    write_reg_opl2(&mut chip, 0x03, 0x80);
    write_reg_opl2(&mut chip, 0x04, 0x02);

    let status_before = chip.read_status();
    assert_eq!(status_before & 0x20, 0, "Timer B flag should be clear");

    chip.timer_expired(1);

    let status_after = chip.read_status();
    assert_eq!(
        status_after & 0x20,
        0x20,
        "Timer B flag (bit 5) should be set after expiry"
    );
}

#[test]
fn timer_flag_cleared_by_reset_bit() {
    let mut chip = Ym3812::new();
    chip.reset();

    write_reg_opl2(&mut chip, 0x02, 0xFF);
    write_reg_opl2(&mut chip, 0x04, 0x01);
    chip.timer_expired(0);

    assert_eq!(
        chip.read_status() & 0x40,
        0x40,
        "Timer A flag should be set"
    );

    write_reg_opl2(&mut chip, 0x04, 0x80);

    assert_eq!(
        chip.read_status() & 0x40,
        0x00,
        "Timer A flag should be cleared after IRQ reset"
    );
}

#[test]
fn timer_irq_assert_and_deassert() {
    let mut chip = Ym3812::new();
    chip.reset();
    chip.take_signals();

    write_reg_opl2(&mut chip, 0x02, 0xFF);
    write_reg_opl2(&mut chip, 0x04, 0x01);
    chip.take_signals();

    chip.timer_expired(0);

    let events = chip.take_signals();
    let irq_assert = events
        .iter()
        .find(|e| matches!(e, SignalEvent::UpdateIrq { asserted: true }));
    assert!(
        irq_assert.is_some(),
        "timer expiry should trigger update_irq(true)"
    );

    write_reg_opl2(&mut chip, 0x04, 0x80);

    let events = chip.take_signals();
    let irq_deassert = events
        .iter()
        .find(|e| matches!(e, SignalEvent::UpdateIrq { asserted: false }));
    assert!(
        irq_deassert.is_some(),
        "IRQ reset should trigger update_irq(false)"
    );
}

#[test]
fn status_read_does_not_clear_flags() {
    let mut chip = Ym3812::new();
    chip.reset();

    write_reg_opl2(&mut chip, 0x02, 0xFF);
    write_reg_opl2(&mut chip, 0x04, 0x01);
    chip.timer_expired(0);

    let status1 = chip.read_status();
    let status2 = chip.read_status();
    let status3 = chip.read_status();

    assert_eq!(
        status1, status2,
        "repeated reads should return the same status"
    );
    assert_eq!(
        status2, status3,
        "repeated reads should return the same status"
    );
    assert_eq!(
        status1 & 0x40,
        0x40,
        "Timer A flag should persist across reads"
    );
}

#[test]
fn both_timers_active() {
    let mut chip = Ym3812::new();
    chip.reset();

    write_reg_opl2(&mut chip, 0x02, 0xFF);
    write_reg_opl2(&mut chip, 0x03, 0x80);
    write_reg_opl2(&mut chip, 0x04, 0x03);

    chip.timer_expired(0);
    chip.timer_expired(1);

    let status = chip.read_status();
    assert_eq!(
        status & 0x60,
        0x60,
        "both Timer A and Timer B flags should be set"
    );
}

#[test]
fn timer_a_period_varies_with_value() {
    let mut chip1 = Ym3812::new();
    chip1.reset();
    chip1.take_signals();

    write_reg_opl2(&mut chip1, 0x02, 0x00);
    write_reg_opl2(&mut chip1, 0x04, 0x01);

    let events1 = chip1.take_signals();
    let duration1 = events1
        .iter()
        .filter_map(|e| match e {
            SignalEvent::SetTimer {
                timer_id: 0,
                duration_in_clocks,
            } if *duration_in_clocks > 0 => Some(*duration_in_clocks),
            _ => None,
        })
        .next_back();

    let mut chip2 = Ym3812::new();
    chip2.reset();
    chip2.take_signals();

    write_reg_opl2(&mut chip2, 0x02, 0xFF);
    write_reg_opl2(&mut chip2, 0x04, 0x01);

    let events2 = chip2.take_signals();
    let duration2 = events2
        .iter()
        .filter_map(|e| match e {
            SignalEvent::SetTimer {
                timer_id: 0,
                duration_in_clocks,
            } if *duration_in_clocks > 0 => Some(*duration_in_clocks),
            _ => None,
        })
        .next_back();

    if let (Some(d1), Some(d2)) = (duration1, duration2) {
        assert!(
            d1 > d2,
            "Timer A=0 ({d1}) should have longer period than Timer A=255 ({d2})"
        );
    } else {
        assert!(
            events1
                .iter()
                .any(|e| matches!(e, SignalEvent::SetTimer { timer_id: 0, .. })),
            "should have set_timer call for Timer A=0"
        );
        assert!(
            events2
                .iter()
                .any(|e| matches!(e, SignalEvent::SetTimer { timer_id: 0, .. })),
            "should have set_timer call for Timer A=255"
        );
    }
}
