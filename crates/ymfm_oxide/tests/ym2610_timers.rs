mod common;

use common::{harness::*, signals::*};
use ymfm_oxide::{Ym2610, Ym2610b, YmfmOpnFidelity};

#[test]
fn timer_a_configuration() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Max, false);
    chip.take_signals();

    write_reg_2610(&mut chip, 0x24, 0xFF);
    write_reg_2610(&mut chip, 0x25, 0x03);
    write_reg_2610(&mut chip, 0x27, 0x05);

    let events = chip.take_signals();
    let timer_a = events.iter().find_map(|event| match event {
        SignalEvent::SetTimer {
            timer_id: 0,
            duration_in_clocks,
        } => Some(*duration_in_clocks),
        _ => None,
    });
    assert!(
        timer_a.is_some_and(|duration| duration > 0),
        "Timer A should be scheduled"
    );
}

#[test]
fn timer_b_configuration() {
    let mut chip: Ym2610b = setup_ym2610(YmfmOpnFidelity::Max, false);
    chip.take_signals();

    write_reg_2610(&mut chip, 0x26, 0x80);
    write_reg_2610(&mut chip, 0x27, 0x0A);

    let events = chip.take_signals();
    let timer_b = events.iter().find_map(|event| match event {
        SignalEvent::SetTimer {
            timer_id: 1,
            duration_in_clocks,
        } => Some(*duration_in_clocks),
        _ => None,
    });
    assert!(
        timer_b.is_some_and(|duration| duration > 0),
        "Timer B should be scheduled"
    );
}

#[test]
fn timer_expiry_sets_status_flags() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Max, false);

    write_reg_2610(&mut chip, 0x24, 0xFF);
    write_reg_2610(&mut chip, 0x25, 0x03);
    write_reg_2610(&mut chip, 0x26, 0x80);
    write_reg_2610(&mut chip, 0x27, 0x0F);
    assert_eq!(chip.read_status(false) & 0x03, 0x00);

    chip.timer_expired(0);
    assert_eq!(chip.read_status(false) & 0x03, 0x01);

    chip.timer_expired(1);
    assert_eq!(chip.read_status(false) & 0x03, 0x03);
    assert_eq!(chip.read_status(true) & 0x80, 0x80);
}

#[test]
fn timer_irq_assert_deassert() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Max, false);

    write_reg_2610(&mut chip, 0x24, 0xFF);
    write_reg_2610(&mut chip, 0x25, 0x03);
    write_reg_2610(&mut chip, 0x27, 0x05);
    chip.take_signals();

    chip.timer_expired(0);
    let events = chip.take_signals();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, SignalEvent::UpdateIrq { asserted: true })),
        "timer expiry should assert IRQ"
    );
    assert!(chip.irq_asserted());

    write_reg_2610(&mut chip, 0x27, 0x15);
    let events = chip.take_signals();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, SignalEvent::UpdateIrq { asserted: false })),
        "flag reset should deassert IRQ"
    );
    assert!(!chip.irq_asserted());
    assert_eq!(chip.read_status(false) & 0x01, 0x00);
}
