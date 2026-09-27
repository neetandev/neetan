mod common;

use common::harness::*;

#[test]
fn first_status_read_after_reset_returns_the_opl2_id() {
    let mut chip = setup_ymf278b_compatible();
    assert_eq!(chip.read_status(false), 0x06);
    assert_eq!(chip.read_status(false), 0x00);
}

#[test]
fn setting_new_keeps_the_status() {
    let mut chip = setup_ymf278b_compatible();
    chip.read_status(false);
    write_reg_ymf278b(&mut chip, 0x105, 0x01);
    assert_eq!(chip.read_status(false), 0x00);
}

#[test]
fn setting_new2_returns_the_opl4_id() {
    let mut chip = setup_ymf278b_compatible();
    chip.read_status(false);
    write_reg_ymf278b(&mut chip, 0x105, 0x03);
    assert_eq!(chip.read_status(true), 0x02);
    assert_eq!(chip.read_status(true), 0x01);
}

#[test]
fn clearing_new2_before_the_read_returns_the_opl3_id() {
    let mut chip = setup_ymf278b_compatible();
    chip.read_status(false);
    write_reg_ymf278b(&mut chip, 0x105, 0x03);
    write_reg_ymf278b(&mut chip, 0x105, 0x01);
    assert_eq!(chip.read_status(true), 0x00);
    assert_eq!(chip.read_status(true), 0x00);
}

#[test]
fn busy_flag_needs_new2() {
    let mut chip = setup_ymf278b();
    chip.read_status(false);
    assert_eq!(chip.read_status(true), 0x01);
    assert_eq!(chip.read_status(false), 0x00);
    write_reg_ymf278b(&mut chip, 0x105, 0x01);
    assert_eq!(chip.read_status(true), 0x00);
}

#[test]
fn load_flag_stays_set_for_13_samples() {
    let mut chip = setup_ymf278b();
    chip.read_status(false);
    write_pcm_ymf278b(&mut chip, 0x08, 0x00);
    assert_eq!(chip.read_status(false), 0x02);
    generate_6_ymf278b(&mut chip, 12);
    assert_eq!(chip.read_status(false), 0x02);
    generate_6_ymf278b(&mut chip, 1);
    assert_eq!(chip.read_status(false), 0x00);

    write_pcm_ymf278b(&mut chip, 0x1F, 0x01);
    generate_6_ymf278b(&mut chip, 40);
    assert_eq!(chip.read_status(false), 0x00);

    write_pcm_ymf278b(&mut chip, 0x10, 0x02);
    write_reg_ymf278b(&mut chip, 0x105, 0x01);
    assert_eq!(chip.read_status(false), 0x00);
}

#[test]
fn other_pcm_writes_leave_the_load_flag_clear() {
    let mut chip = setup_ymf278b();
    chip.read_status(false);
    write_pcm_ymf278b(&mut chip, 0x20, 0x00);
    write_pcm_ymf278b(&mut chip, 0x07, 0x00);
    assert_eq!(chip.read_status(false), 0x00);
}

#[test]
fn writes_report_busy_durations() {
    let mut chip = setup_ymf278b_compatible();
    assert_eq!(chip.write_address(0x20), 0);
    assert_eq!(chip.write_data(0x21), 56);
    assert_eq!(chip.write_address_hi(0x20), 0);
    assert_eq!(chip.write_address_pcm(0x50), 0);
    assert_eq!(chip.write_data_pcm(0x01), 0);
    assert_eq!(chip.write_data(0x01), 56);

    write_reg_ymf278b(&mut chip, 0x105, 0x03);
    assert_eq!(chip.write_address_pcm(0x50), 0);
    assert_eq!(chip.write_data_pcm(0x01), 88);
    assert_eq!(chip.write_data(0x01), 56);
}

#[test]
fn pcm_writes_need_new2() {
    let mut chip = setup_ymf278b_compatible();
    write_pcm_ymf278b(&mut chip, 0x50, 0x2A);
    chip.write_address_pcm(0x50);
    assert_eq!(chip.read_data_pcm(), 0x00);

    write_reg_ymf278b(&mut chip, 0x105, 0x03);
    write_pcm_ymf278b(&mut chip, 0x50, 0x2A);
    chip.write_address_pcm(0x50);
    assert_eq!(chip.read_data_pcm(), 0x2A);
}

#[test]
fn pcm_data_reads_zero_while_an_fm_register_is_addressed() {
    let mut chip = setup_ymf278b();
    write_pcm_ymf278b(&mut chip, 0x50, 0x2A);
    chip.write_address(0x50);
    assert_eq!(chip.read_data_pcm(), 0x00);
    chip.write_address_hi(0x50);
    assert_eq!(chip.read_data_pcm(), 0x00);
}

#[test]
fn memory_register_reads_the_device_id() {
    let mut chip = setup_ymf278b();
    chip.write_address_pcm(0x02);
    assert_eq!(chip.read_data_pcm(), 0x20);
    write_pcm_ymf278b(&mut chip, 0x02, 0x12);
    chip.write_address_pcm(0x02);
    assert_eq!(chip.read_data_pcm(), 0x32);
}

#[test]
fn mix_control_resets_to_fm_minus_9db() {
    let mut chip = setup_ymf278b();
    chip.write_address_pcm(0xF8);
    assert_eq!(chip.read_data_pcm(), 0x1B);
    chip.write_address_pcm(0xF9);
    assert_eq!(chip.read_data_pcm(), 0x00);
}

#[test]
fn high_bank_address_is_masked_without_new() {
    let mut chip = setup_ymf278b_compatible();
    write_reg_ymf278b(&mut chip, 0x1A0, 0x44);
    write_reg_ymf278b(&mut chip, 0x105, 0x03);
    write_reg_ymf278b(&mut chip, 0x1A0, 0x55);
    assert_eq!(
        generate_6_ymf278b(&mut chip, 4),
        generate_6_ymf278b(&mut setup_ymf278b(), 4)
    );
}

#[test]
fn bus_interface_decodes_the_offsets() {
    let mut chip = setup_ymf278b_compatible();
    assert_eq!(chip.read(0, false), 0x06);
    assert_eq!(chip.write(2, 0x05), 0);
    assert_eq!(chip.write(3, 0x03), 56);
    assert_eq!(chip.read(0, true), 0x02);
    assert_eq!(chip.read(0, true), 0x01);
    assert_eq!(chip.write(4, 0x50), 0);
    assert_eq!(chip.write(5, 0x7E), 88);
    assert_eq!(chip.read(5, false), 0x7E);
    assert_eq!(chip.write(0, 0x20), 0);
    assert_eq!(chip.write(1, 0x21), 56);
    assert_eq!(chip.write(6, 0x21), 0);
    assert_eq!(chip.write(7, 0x21), 0);
    for offset in [1, 2, 3, 4, 6, 7] {
        assert_eq!(chip.read(offset, false), 0xFF);
    }
    assert_eq!(chip.read(8, false), 0x00);
}
