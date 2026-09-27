mod common;

use common::{harness::*, signals::*};

#[test]
fn every_access_reports_busy_duration() {
    let mut chip = setup_ymf289b();
    assert_eq!(chip.write_address(0x20), 56);
    assert_eq!(chip.write_data(0x21), 56);
    assert_eq!(chip.write_address_hi(0x20), 56);
}

#[test]
fn registers_read_back_only_in_ymf289b_mode() {
    let mut chip = setup_ymf289b();
    write_reg_ymf289b(&mut chip, 0x40, 0x2A);
    chip.write_address(0x40);
    assert_eq!(chip.read_data(), 0xFF);

    write_reg_ymf289b_hi(&mut chip, 0x05, 0x05);
    chip.write_address(0x40);
    assert_eq!(chip.read_data(), 0x2A);
    chip.write_address_hi(0x05);
    assert_eq!(chip.read_data(), 0x05);
}

#[test]
fn busy_flags_appear_only_in_ymf289b_mode() {
    let mut chip = setup_ymf289b();
    assert_eq!(chip.read_status(true), 0x00);
    write_reg_ymf289b_hi(&mut chip, 0x05, 0x05);
    assert_eq!(chip.read_status(true), 0x05);
    assert_eq!(chip.read_status(false), 0x00);
}

#[test]
fn register_clear_resets_the_mode_registers() {
    let mut chip = setup_ymf289b();
    write_reg_ymf289b_hi(&mut chip, 0x05, 0x05);
    write_reg_ymf289b(&mut chip, 0x40, 0x2A);
    write_reg_ymf289b_hi(&mut chip, 0x08, 0x04);
    chip.write_address(0x40);
    assert_eq!(chip.read_data(), 0xFF);
}

#[test]
fn compatibility_mode_masks_the_high_bank() {
    let mut chip = Ymf289bFixture::new();
    write_reg_ymf289b_hi(&mut chip.0, 0x40, 0x3F);
    write_reg_ymf289b_hi(&mut chip.0, 0x05, 0x05);
    chip.0.write_address(0x40);
    assert_eq!(chip.0.read_data(), 0x3F);
}

#[test]
fn timer_expiry_raises_irq() {
    let mut chip = setup_ymf289b();
    write_reg_ymf289b(&mut chip, 0x02, 0xFE);
    write_reg_ymf289b(&mut chip, 0x04, 0x01);
    let events = chip.take_signals();
    assert!(events.contains(&SignalEvent::SetTimer {
        timer_id: 0,
        duration_in_clocks: 2 * 4 * 36 * 8,
    }));

    chip.timer_expired(0);
    let events = chip.take_signals();
    assert!(events.contains(&SignalEvent::UpdateIrq { asserted: true }));
    assert_eq!(chip.read_status(false), 0xC0);
}

/// A YMF289B that stays in OPL2 compatibility mode after reset.
struct Ymf289bFixture(ymfm_oxide::Ymf289b);

impl Ymf289bFixture {
    fn new() -> Self {
        let mut chip = ymfm_oxide::Ymf289b::new();
        chip.reset();
        Self(chip)
    }
}
