mod common;

use common::{harness::*, signals::*};
use ymfm_oxide::YmfmOpnFidelity;

#[test]
fn read_data_returns_id_and_missing_io_ports() {
    let mut chip = setup_ymf288(YmfmOpnFidelity::Max);
    chip.write_address(0xFF);
    assert_eq!(chip.read_data(), 2);
    chip.write_address(0x0E);
    assert_eq!(chip.read_data(), 0xFF);
    chip.write_address(0x0F);
    assert_eq!(chip.read_data(), 0xFF);
}

#[test]
fn ssg_registers_read_back() {
    let mut chip = setup_ymf288(YmfmOpnFidelity::Max);
    write_reg_ymf288(&mut chip, 0x02, 0x5A);
    chip.write_address(0x02);
    assert_eq!(chip.read_data(), 0x5A);
}

#[test]
fn fm_registers_read_back_only_in_ymf288_mode() {
    let mut chip = setup_ymf288(YmfmOpnFidelity::Max);
    write_reg_ymf288(&mut chip, 0x40, 0x2A);
    chip.write_address(0x40);
    assert_eq!(chip.read_data(), 0);

    write_reg_ymf288(&mut chip, 0x20, 0x02);
    chip.write_address(0x40);
    assert_eq!(chip.read_data(), 0x2A);
}

#[test]
fn busy_times_are_shorter_in_ymf288_mode() {
    let mut chip = setup_ymf288(YmfmOpnFidelity::Max);
    assert_eq!(chip.write_address(0x40), 0);
    assert_eq!(chip.write_data(0x00), 32 * 6);
    assert_eq!(chip.write_address_hi(0x40), 0);
    assert_eq!(chip.write_data_hi(0x00), 32 * 6);

    write_reg_ymf288(&mut chip, 0x20, 0x02);
    assert_eq!(chip.write_address(0x40), 16);
    assert_eq!(chip.write_data(0x00), 16);
    assert_eq!(chip.write_address_hi(0x40), 16);
    assert_eq!(chip.write_data_hi(0x00), 16);
}

#[test]
fn data_write_to_other_bank_is_ignored() {
    let mut chip = setup_ymf288(YmfmOpnFidelity::Max);
    chip.write_address_hi(0x30);
    assert_eq!(chip.write_data(0x01), 0);
    chip.write_address(0x30);
    assert_eq!(chip.write_data_hi(0x01), 0);
}

#[test]
fn timer_a_schedules_and_raises_irq() {
    let mut chip = setup_ymf288(YmfmOpnFidelity::Max);
    chip.take_signals();

    write_reg_ymf288(&mut chip, 0x24, 0xFF);
    write_reg_ymf288(&mut chip, 0x25, 0x02);
    write_reg_ymf288(&mut chip, 0x27, 0x05);
    let events = chip.take_signals();
    assert!(events.contains(&SignalEvent::SetTimer {
        timer_id: 0,
        duration_in_clocks: 2 * 24 * 6,
    }));

    chip.timer_expired(0);
    let events = chip.take_signals();
    assert!(events.contains(&SignalEvent::UpdateIrq { asserted: true }));
    assert_eq!(chip.read_status(false), 0x01);
    assert_eq!(chip.read_status_hi(true), 0x81);
}

#[test]
fn flag_control_masks_status_and_irq() {
    let mut chip = setup_ymf288(YmfmOpnFidelity::Max);
    write_reg_ymf288(&mut chip, 0x26, 0xFF);
    write_reg_ymf288(&mut chip, 0x27, 0x0A);
    chip.timer_expired(1);
    assert_eq!(chip.read_status_hi(false), 0x02);

    write_reg_ymf288_hi(&mut chip, 0x10, 0x02);
    assert_eq!(chip.read_status_hi(false), 0x00);
    assert!(!chip.irq_asserted());
}

#[test]
fn flag_reset_clears_timer_status() {
    let mut chip = setup_ymf288(YmfmOpnFidelity::Max);
    write_reg_ymf288(&mut chip, 0x24, 0xFF);
    write_reg_ymf288(&mut chip, 0x25, 0x03);
    write_reg_ymf288(&mut chip, 0x27, 0x05);
    chip.timer_expired(0);
    assert_eq!(chip.read_status(false), 0x01);

    write_reg_ymf288_hi(&mut chip, 0x10, 0x80);
    assert_eq!(chip.read_status(false), 0x00);
    chip.take_signals();
}
