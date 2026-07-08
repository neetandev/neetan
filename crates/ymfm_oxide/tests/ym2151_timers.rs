mod common;

use common::{harness::*, signals::*};
use ymfm_oxide::Ym2151;

// OPM mode register 0x14: bit0 load A, bit1 load B, bit2 enable A, bit3 enable
// B, bit4 reset A, bit5 reset B. Status bit0 = Timer A, bit1 = Timer B.

#[test]
fn timer_a_configuration() {
    let mut chip = Ym2151::new();
    chip.reset();
    chip.take_signals();

    write_reg_ym2151(&mut chip, 0x10, 0xFF); // Timer A value (upper 8 bits)
    write_reg_ym2151(&mut chip, 0x11, 0x03); // Timer A value (lower 2 bits)
    write_reg_ym2151(&mut chip, 0x14, 0x05); // load + enable Timer A

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
    let mut chip = Ym2151::new();
    chip.reset();
    chip.take_signals();

    write_reg_ym2151(&mut chip, 0x12, 0x80); // Timer B value
    write_reg_ym2151(&mut chip, 0x14, 0x0A); // load + enable Timer B

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
    let mut chip = Ym2151::new();
    chip.reset();

    write_reg_ym2151(&mut chip, 0x10, 0xFF);
    write_reg_ym2151(&mut chip, 0x11, 0x03);
    write_reg_ym2151(&mut chip, 0x14, 0x05);

    assert_eq!(
        chip.read_status(false) & 0x01,
        0,
        "Timer A flag should be clear before expiry"
    );

    chip.timer_expired(0);

    assert_eq!(
        chip.read_status(false) & 0x01,
        0x01,
        "Timer A flag (bit 0) should be set after expiry"
    );
}

#[test]
fn timer_b_expiry_sets_status_flag() {
    let mut chip = Ym2151::new();
    chip.reset();

    write_reg_ym2151(&mut chip, 0x12, 0x80);
    write_reg_ym2151(&mut chip, 0x14, 0x0A);

    assert_eq!(
        chip.read_status(false) & 0x02,
        0,
        "Timer B flag should be clear"
    );

    chip.timer_expired(1);

    assert_eq!(
        chip.read_status(false) & 0x02,
        0x02,
        "Timer B flag (bit 1) should be set after expiry"
    );
}

#[test]
fn timer_flag_cleared_by_reset_bit() {
    let mut chip = Ym2151::new();
    chip.reset();

    write_reg_ym2151(&mut chip, 0x10, 0xFF);
    write_reg_ym2151(&mut chip, 0x11, 0x03);
    write_reg_ym2151(&mut chip, 0x14, 0x05);
    chip.timer_expired(0);

    assert_eq!(
        chip.read_status(false) & 0x01,
        0x01,
        "Timer A flag should be set"
    );

    write_reg_ym2151(&mut chip, 0x14, 0x10); // reset Timer A flag

    assert_eq!(
        chip.read_status(false) & 0x01,
        0x00,
        "Timer A flag should be cleared after reset bit"
    );
}

#[test]
fn timer_irq_assert_and_deassert() {
    let mut chip = Ym2151::new();
    chip.reset();
    chip.take_signals();

    write_reg_ym2151(&mut chip, 0x10, 0xFF);
    write_reg_ym2151(&mut chip, 0x11, 0x03);
    write_reg_ym2151(&mut chip, 0x14, 0x05);
    chip.take_signals();

    chip.timer_expired(0);

    let events = chip.take_signals();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SignalEvent::UpdateIrq { asserted: true })),
        "timer expiry should trigger update_irq(true)"
    );

    write_reg_ym2151(&mut chip, 0x14, 0x10); // reset Timer A flag

    let events = chip.take_signals();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SignalEvent::UpdateIrq { asserted: false })),
        "flag reset should trigger update_irq(false)"
    );
}

#[test]
fn both_timers_active() {
    let mut chip = Ym2151::new();
    chip.reset();

    write_reg_ym2151(&mut chip, 0x10, 0xFF);
    write_reg_ym2151(&mut chip, 0x11, 0x03);
    write_reg_ym2151(&mut chip, 0x12, 0x80);
    write_reg_ym2151(&mut chip, 0x14, 0x0F); // load + enable both timers

    chip.timer_expired(0);
    chip.timer_expired(1);

    assert_eq!(
        chip.read_status(false) & 0x03,
        0x03,
        "both Timer A and Timer B flags should be set"
    );
}
