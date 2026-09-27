mod common;

use common::{harness::*, signals::*};

fn timer_duration(events: &[SignalEvent], timer: u32) -> Option<i32> {
    events.iter().find_map(|event| match event {
        SignalEvent::SetTimer {
            timer_id,
            duration_in_clocks,
        } if *timer_id == timer => Some(*duration_in_clocks),
        _ => None,
    })
}

#[test]
fn timer_a_counts_operator_clocks_with_prescale_19() {
    let mut chip = setup_ymf278b();
    chip.take_signals();
    write_reg_ymf278b(&mut chip, 0x02, 0xFE);
    write_reg_ymf278b(&mut chip, 0x04, 0x01);

    // Timer A counts (1024 - 4 * 254) steps of 36 operators times prescale 19.
    let events = chip.take_signals();
    assert_eq!(timer_duration(&events, 0), Some(8 * 36 * 19));
}

#[test]
fn timer_b_counts_operator_clocks_with_prescale_19() {
    let mut chip = setup_ymf278b();
    chip.take_signals();
    write_reg_ymf278b(&mut chip, 0x03, 0xFE);
    write_reg_ymf278b(&mut chip, 0x04, 0x02);

    // Timer B counts 16 * (256 - 254) steps of 36 operators times prescale 19.
    let events = chip.take_signals();
    assert_eq!(timer_duration(&events, 1), Some(16 * 2 * 36 * 19));
}

#[test]
fn timer_expiry_raises_irq_and_status() {
    let mut chip = setup_ymf278b();
    chip.read_status(false);
    write_reg_ymf278b(&mut chip, 0x02, 0xF0);
    write_reg_ymf278b(&mut chip, 0x04, 0x01);
    chip.take_signals();

    chip.timer_expired(0);
    let events = chip.take_signals();
    assert!(events.contains(&SignalEvent::UpdateIrq { asserted: true }));
    assert_eq!(chip.read_status(false), 0xC0);
    assert_eq!(chip.read_status(true), 0xC1);
    assert!(chip.irq_asserted());

    write_reg_ymf278b(&mut chip, 0x04, 0x80);
    let events = chip.take_signals();
    assert!(events.contains(&SignalEvent::UpdateIrq { asserted: false }));
    assert_eq!(chip.read_status(false), 0x00);
}

#[test]
fn masked_timer_leaves_the_irq_low() {
    let mut chip = setup_ymf278b();
    chip.read_status(false);
    write_reg_ymf278b(&mut chip, 0x04, 0x22);
    chip.take_signals();

    chip.timer_expired(1);
    assert!(!chip.irq_asserted());
    assert_eq!(chip.read_status(false), 0x00);
}
