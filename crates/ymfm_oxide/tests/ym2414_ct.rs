mod common;

use common::harness::*;

// OPZ register 0x1B bits 7:6 drive the CT output pins. The exposed field
// places register bit 6 in bit 0 and register bit 7 in bit 1.

#[test]
fn ct_outputs_start_cleared() {
    let mut chip = setup_ym2414();
    assert_eq!(chip.ct_state(), 0);
    assert_eq!(chip.take_ct_update(), None);
}

#[test]
fn ct_write_latches_state_and_reports_update() {
    let mut chip = setup_ym2414();
    for (data, ct) in [(0xC0u8, 3u8), (0x40, 1), (0x80, 2), (0x00, 0)] {
        write_reg_ym2414(&mut chip, 0x1B, data);
        assert_eq!(chip.ct_state(), ct);
        assert_eq!(chip.take_ct_update(), Some(ct));
        assert_eq!(chip.take_ct_update(), None);
    }
}

#[test]
fn unchanged_ct_bits_produce_no_update() {
    let mut chip = setup_ym2414();
    write_reg_ym2414(&mut chip, 0x1B, 0x40);
    chip.take_ct_update();
    write_reg_ym2414(&mut chip, 0x1B, 0x7F);
    assert_eq!(chip.ct_state(), 1);
    assert_eq!(chip.take_ct_update(), None);
}

#[test]
fn other_registers_do_not_touch_ct_outputs() {
    let mut chip = setup_ym2414();
    write_reg_ym2414(&mut chip, 0x1A, 0xC0);
    write_reg_ym2414(&mut chip, 0x1C, 0xC0);
    assert_eq!(chip.ct_state(), 0);
    assert_eq!(chip.take_ct_update(), None);
}

#[test]
fn reset_keeps_ct_outputs() {
    let mut chip = setup_ym2414();
    write_reg_ym2414(&mut chip, 0x1B, 0xC0);
    chip.take_ct_update();
    chip.reset();
    assert_eq!(chip.ct_state(), 3);
    assert_eq!(chip.take_ct_update(), None);
}
