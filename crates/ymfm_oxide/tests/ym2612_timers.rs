mod common;

use common::{harness::*, signals::*};
use ymfm_oxide::{Ym2612, Ym3438, Ymf276};

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
fn timer_a_schedules_period_in_input_clocks() {
    let mut chip: Ym2612 = setup_opn2();
    chip.take_signals();

    write_reg_opn2(&mut chip, 0x24, 0xFF);
    write_reg_opn2(&mut chip, 0x25, 0x02);
    write_reg_opn2(&mut chip, 0x27, 0x05);

    // Timer A counts (1024 - 1022) steps of 24 operators times prescale 6.
    let events = chip.take_signals();
    assert_eq!(timer_duration(&events, 0), Some(2 * 24 * 6));
}

#[test]
fn timer_b_schedules_period_in_input_clocks() {
    let mut chip: Ym3438 = setup_opn2();
    chip.take_signals();

    write_reg_opn2(&mut chip, 0x26, 0xFE);
    write_reg_opn2(&mut chip, 0x27, 0x0A);

    // Timer B counts 16 * (256 - 254) steps of 24 operators times prescale 6.
    let events = chip.take_signals();
    assert_eq!(timer_duration(&events, 1), Some(16 * 2 * 24 * 6));
}

#[test]
fn timer_expiry_raises_and_reset_clears_irq() {
    let mut chip: Ymf276 = setup_opn2();
    write_reg_opn2(&mut chip, 0x24, 0xFF);
    write_reg_opn2(&mut chip, 0x25, 0x03);
    write_reg_opn2(&mut chip, 0x27, 0x05);
    chip.take_signals();

    chip.timer_expired(0);
    let events = chip.take_signals();
    assert!(events.contains(&SignalEvent::UpdateIrq { asserted: true }));
    assert_eq!(chip.read_status(false) & 0x01, 0x01);

    write_reg_opn2(&mut chip, 0x27, 0x15);
    let events = chip.take_signals();
    assert!(events.contains(&SignalEvent::UpdateIrq { asserted: false }));
    assert_eq!(chip.read_status(false) & 0x01, 0x00);
}

#[test]
fn disabled_timer_is_cancelled() {
    let mut chip: Ym2612 = setup_opn2();
    write_reg_opn2(&mut chip, 0x27, 0x05);
    chip.take_signals();

    write_reg_opn2(&mut chip, 0x27, 0x00);
    let events = chip.take_signals();
    assert_eq!(timer_duration(&events, 0), Some(-1));
}
