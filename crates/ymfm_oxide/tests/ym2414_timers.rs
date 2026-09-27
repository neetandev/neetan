mod common;

use common::harness::*;
use ymfm_oxide::YmfmTimerUpdate;

// OPZ mode register 0x14: bit0 load A, bit1 load B, bit2 enable A, bit3 enable
// B, bit4 reset A, bit5 reset B, bit7 CSM. Status bit0 = Timer A, bit1 = Timer B.

#[test]
fn every_data_write_reports_busy_duration() {
    let mut chip = setup_ym2414();
    assert_eq!(chip.write_address(0x20), 0);
    assert_eq!(chip.write_data(0x00), 32 * 2);
}

#[test]
fn timer_a_schedules_period_in_input_clocks() {
    let mut chip = setup_ym2414();
    write_reg_ym2414(&mut chip, 0x10, 0xFF);
    write_reg_ym2414(&mut chip, 0x11, 0x02);
    write_reg_ym2414(&mut chip, 0x14, 0x05);
    assert_eq!(
        chip.take_timer_update(0),
        Some(YmfmTimerUpdate::Schedule(2 * 32 * 2))
    );
    assert_eq!(chip.take_timer_update(1), Some(YmfmTimerUpdate::Cancel));
}

#[test]
fn timer_b_schedules_period_in_input_clocks() {
    let mut chip = setup_ym2414();
    write_reg_ym2414(&mut chip, 0x12, 0xFE);
    write_reg_ym2414(&mut chip, 0x14, 0x0A);
    assert_eq!(
        chip.take_timer_update(1),
        Some(YmfmTimerUpdate::Schedule(16 * 2 * 32 * 2))
    );
}

#[test]
fn timer_expiry_raises_irq_and_reset_clears_it() {
    let mut chip = setup_ym2414();
    write_reg_ym2414(&mut chip, 0x12, 0xF0);
    write_reg_ym2414(&mut chip, 0x14, 0x0A);
    chip.timer_expired(1);
    assert_eq!(chip.take_irq_update(), Some(true));
    assert!(chip.irq_asserted());
    assert_eq!(chip.read_status(false), 0x02);

    write_reg_ym2414(&mut chip, 0x14, 0x2A);
    assert_eq!(chip.take_irq_update(), Some(false));
    assert_eq!(chip.read_status(false), 0x00);
}

#[test]
fn both_timers_set_their_status_bits() {
    let mut chip = setup_ym2414();
    write_reg_ym2414(&mut chip, 0x14, 0x0F);
    chip.timer_expired(0);
    chip.timer_expired(1);
    assert_eq!(chip.read_status(true), 0x83);
    write_reg_ym2414(&mut chip, 0x14, 0x1F);
    assert_eq!(chip.read_status(false), 0x02);
}
