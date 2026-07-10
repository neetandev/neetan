mod common;

use common::harness::*;
use ymfm_oxide::Ym2151;

// OPM register 0x1B bits 7:6 drive the CT output pins. The exposed field
// places register bit 6 in bit 0 and register bit 7 in bit 1.

#[test]
fn ct_outputs_start_cleared() {
    let mut chip = Ym2151::new();
    chip.reset();

    assert_eq!(chip.ct_state(), 0);
    assert_eq!(chip.take_ct_update(), None);
}

#[test]
fn ct_write_latches_state_and_reports_update() {
    let mut chip = Ym2151::new();
    chip.reset();

    write_reg_ym2151(&mut chip, 0x1B, 0xC0);
    assert_eq!(chip.ct_state(), 0x03);
    assert_eq!(chip.take_ct_update(), Some(0x03));
    assert_eq!(chip.take_ct_update(), None);

    write_reg_ym2151(&mut chip, 0x1B, 0x40);
    assert_eq!(chip.ct_state(), 0x01);
    assert_eq!(chip.take_ct_update(), Some(0x01));

    write_reg_ym2151(&mut chip, 0x1B, 0x80);
    assert_eq!(chip.ct_state(), 0x02);
    assert_eq!(chip.take_ct_update(), Some(0x02));

    write_reg_ym2151(&mut chip, 0x1B, 0x00);
    assert_eq!(chip.ct_state(), 0x00);
    assert_eq!(chip.take_ct_update(), Some(0x00));
}

#[test]
fn unchanged_ct_bits_produce_no_update() {
    let mut chip = Ym2151::new();
    chip.reset();

    write_reg_ym2151(&mut chip, 0x1B, 0x40);
    chip.take_ct_update();

    write_reg_ym2151(&mut chip, 0x1B, 0x7F);
    assert_eq!(chip.ct_state(), 0x01);
    assert_eq!(chip.take_ct_update(), None);
}

#[test]
fn other_registers_do_not_touch_ct_outputs() {
    let mut chip = Ym2151::new();
    chip.reset();

    write_reg_ym2151(&mut chip, 0x1A, 0xC0);
    write_reg_ym2151(&mut chip, 0x1C, 0xC0);
    assert_eq!(chip.ct_state(), 0);
    assert_eq!(chip.take_ct_update(), None);
}

#[test]
fn reset_clears_ct_outputs() {
    let mut chip = Ym2151::new();
    chip.reset();

    write_reg_ym2151(&mut chip, 0x1B, 0xC0);
    chip.reset();
    assert_eq!(chip.ct_state(), 0);
    assert_eq!(chip.take_ct_update(), None);
}
