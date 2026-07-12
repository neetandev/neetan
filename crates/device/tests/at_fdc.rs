//! Integration tests for the AT FDC front-end.

use device::{
    at_fdc::{AtFdc, FdcDataRate},
    floppy::FloppyImage,
    upd765a_fdc::{FdcAction, FdcPhase, ST0_READY_LINE_CHANGED},
};

/// DOR value: reset released, IRQ/DMA gate on, drive 0 selected, motor 0 on.
const DOR_RUNNING: u8 = 0x1C;

fn image_1440k() -> FloppyImage {
    FloppyImage::from_img_bytes(&vec![0u8; 1_474_560]).unwrap()
}

fn image_720k() -> FloppyImage {
    FloppyImage::from_img_bytes(&vec![0u8; 737_280]).unwrap()
}

fn image_360k() -> FloppyImage {
    FloppyImage::from_img_bytes(&vec![0u8; 368_640]).unwrap()
}

fn running_fdc() -> AtFdc {
    let mut fdc = AtFdc::new();
    fdc.write_dor(DOR_RUNNING);
    fdc
}

#[test]
fn power_on_holds_reset() {
    let mut fdc = AtFdc::new();
    assert_eq!(fdc.read_main_status(), 0x00);
    assert_eq!(fdc.write_data(0x03), FdcAction::None);
    assert_eq!(fdc.state.phase, FdcPhase::Idle);
}

#[test]
fn dor_reset_edges() {
    let mut fdc = AtFdc::new();

    let effect = fdc.write_dor(DOR_RUNNING);
    assert!(effect.reset_released);
    assert!(!effect.reset_started);
    assert_ne!(fdc.read_main_status(), 0x00, "MSR live after release");

    // Start a command, then assert reset: the command is aborted.
    fdc.write_data(0x06);
    assert_eq!(fdc.state.phase, FdcPhase::Command);
    let effect = fdc.write_dor(DOR_RUNNING & !0x04);
    assert!(effect.reset_started);
    assert_eq!(fdc.state.phase, FdcPhase::Idle);
    assert_eq!(fdc.read_main_status(), 0x00, "MSR reads 0 while held");
}

#[test]
fn reset_preserves_head_positions_and_motor_bits() {
    let mut fdc = running_fdc();
    fdc.state.drive_cylinder[0] = 33;

    fdc.write_dor(DOR_RUNNING & !0x04);
    fdc.write_dor(DOR_RUNNING);
    assert_eq!(fdc.state.drive_cylinder[0], 33);
    assert!(fdc.motor_on(0));
}

#[test]
fn reset_polling_yields_four_sense_results_then_invalid() {
    let mut fdc = running_fdc();
    fdc.raise_reset_polling_status();
    assert!(fdc.state.interrupt_pending);

    for drive in 0..4u8 {
        fdc.write_data(0x08);
        let st0 = fdc.read_data();
        let pcn = fdc.read_data();
        assert_eq!(st0, ST0_READY_LINE_CHANGED | drive, "drive {drive}");
        assert_eq!(pcn, 0);
    }

    // Fifth sense: invalid command status.
    fdc.write_data(0x08);
    assert_eq!(fdc.read_data(), 0x80);
}

#[test]
fn irq_gate_follows_dor_bit3() {
    let mut fdc = running_fdc();
    assert!(fdc.irq_enabled());
    fdc.state.tc = false;
    fdc.state.interrupt_pending = true;

    let effect = fdc.write_dor(DOR_RUNNING & !0x08);
    assert!(effect.irq_gate_dropped);
    assert!(!fdc.irq_enabled());
    assert!(fdc.state.tc);
    assert!(!fdc.state.interrupt_pending);
}

#[test]
fn dir_change_latch_over_insert_step_and_motor() {
    let mut fdc = running_fdc();

    // Empty drive with motor on: change bit set.
    assert_eq!(fdc.read_dir(), 0xFF);

    // Motor off: bit gated away.
    fdc.write_dor(DOR_RUNNING & !0x10);
    assert_eq!(fdc.read_dir(), 0x7F);
    fdc.write_dor(DOR_RUNNING);

    // Insert latches the change bit until a step with media.
    fdc.insert_drive(0, image_1440k(), None);
    assert_eq!(fdc.read_dir(), 0xFF);
    fdc.clear_disk_change_on_step(0);
    assert_eq!(fdc.read_dir(), 0x7F);

    // Eject latches it again; a step without media does not clear it.
    fdc.eject_drive(0);
    fdc.clear_disk_change_on_step(0);
    assert_eq!(fdc.read_dir(), 0xFF);
}

#[test]
fn ccr_rate_matching_per_media() {
    let mut fdc = running_fdc();

    fdc.insert_drive(0, image_1440k(), None);
    fdc.write_ccr(0x00);
    assert_eq!(fdc.data_rate(), FdcDataRate::Rate500Kbps);
    assert!(fdc.data_rate_matches(0));
    fdc.write_ccr(0x02);
    assert!(!fdc.data_rate_matches(0), "1.44 MB needs 500 kbps");

    fdc.insert_drive(0, image_720k(), None);
    assert!(fdc.data_rate_matches(0), "720 KB reads at 250 kbps");
    fdc.write_ccr(0x00);
    assert!(!fdc.data_rate_matches(0));

    fdc.insert_drive(0, image_360k(), None);
    fdc.write_ccr(0x01);
    assert!(
        fdc.data_rate_matches(0),
        "360 KB in a 1.2 MB drive reads at 300 kbps"
    );
    fdc.write_ccr(0x02);
    assert!(fdc.data_rate_matches(0), "360 KB also reads at 250 kbps");
    fdc.write_ccr(0x00);
    assert!(!fdc.data_rate_matches(0));

    // No media: rate always matches (the empty-drive path reports the error).
    fdc.eject_drive(0);
    assert!(fdc.data_rate_matches(0));
}

#[test]
fn dsr_soft_reset_is_self_clearing() {
    let mut fdc = running_fdc();
    fdc.write_data(0x06);
    assert_eq!(fdc.state.phase, FdcPhase::Command);

    assert!(fdc.write_dsr(0x80));
    assert_eq!(fdc.state.phase, FdcPhase::Idle);
    assert_ne!(fdc.read_main_status(), 0x00, "reset is not held");

    // While DOR holds reset, DSR bit 7 does not release anything.
    fdc.write_dor(DOR_RUNNING & !0x04);
    assert!(!fdc.write_dsr(0x80));
    assert_eq!(fdc.read_main_status(), 0x00);
}

#[test]
fn drives_report_ready_without_media() {
    let mut fdc = running_fdc();

    // SENSE DRIVE STATUS on empty drive 0: RY set (ready line strapped).
    fdc.write_data(0x04);
    fdc.write_data(0x00);
    let st3 = fdc.read_data();
    assert_ne!(st3 & 0x20, 0, "ST3 RY strapped active on AT");
}

#[test]
fn write_protection_reported_from_image() {
    let mut fdc = running_fdc();
    let mut image = image_1440k();
    image.write_protected = true;
    fdc.insert_drive(0, image, None);
    assert!(fdc.is_write_protected(0));

    fdc.write_data(0x04);
    fdc.write_data(0x00);
    let st3 = fdc.read_data();
    assert_ne!(st3 & 0x40, 0, "ST3 WP follows the image");
}
