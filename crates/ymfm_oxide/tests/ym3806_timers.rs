mod common;

use common::harness::*;
use ymfm_oxide::YmfmTimerUpdate;

#[test]
fn timer_b_schedules_from_the_upper_bits() {
    let mut chip = setup_ym3806();
    write_reg_ym3806(&mut chip, 0x03, 0x71);
    // The value 0x71 selects timer B value 0xDC: 16 * (256 - 0xDC) steps of
    // 32 operators times prescale 2.
    assert_eq!(
        chip.take_timer_update(1),
        Some(YmfmTimerUpdate::Schedule(16 * 36 * 32 * 2))
    );
    assert_eq!(chip.take_timer_update(0), Some(YmfmTimerUpdate::Cancel));
}

#[test]
fn timer_b_expiry_sets_status_and_irq() {
    let mut chip = setup_ym3806();
    write_reg_ym3806(&mut chip, 0x03, 0xFD);
    chip.take_timer_update(1);
    chip.timer_expired(1);
    assert_eq!(chip.take_irq_update(), Some(true));
    assert!(chip.irq_asserted());
    assert_eq!(chip.read_status(false), 0x04);
    assert_eq!(
        chip.take_timer_update(1),
        Some(YmfmTimerUpdate::Schedule(16 * 32 * 2))
    );
}

#[test]
fn timer_b_write_with_bit_0_clears_the_status() {
    let mut chip = setup_ym3806();
    write_reg_ym3806(&mut chip, 0x03, 0xFD);
    chip.timer_expired(1);
    assert_eq!(chip.read_status(false), 0x04);
    write_reg_ym3806(&mut chip, 0x03, 0xFD);
    assert_eq!(chip.read_status(false), 0x00);
    assert_eq!(chip.take_irq_update(), Some(false));
}

#[test]
fn timer_b_disable_cancels_and_keeps_the_status() {
    let mut chip = setup_ym3806();
    write_reg_ym3806(&mut chip, 0x03, 0xFD);
    chip.timer_expired(1);
    write_reg_ym3806(&mut chip, 0x03, 0xFC);
    assert_eq!(chip.take_timer_update(1), Some(YmfmTimerUpdate::Cancel));
    assert_eq!(chip.read_status(false), 0x04);
}

#[test]
fn timer_b_expiry_while_disabled_keeps_status_clear() {
    let mut chip = setup_ym3806();
    chip.timer_expired(1);
    assert_eq!(chip.read_status(false), 0x00);
    assert!(!chip.irq_asserted());
}

#[test]
fn timer_b_first_period_subtracts_the_free_running_counter() {
    let mut chip = setup_ym3806();
    generate_2_ym3806(&mut chip, 5);
    write_reg_ym3806(&mut chip, 0x03, 0x71);
    assert_eq!(
        chip.take_timer_update(1),
        Some(YmfmTimerUpdate::Schedule((16 * 36 - 5) * 32 * 2))
    );
}
