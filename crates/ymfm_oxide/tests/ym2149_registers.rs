mod common;

use common::harness::*;

const CLOCK: u32 = 2_000_000;

#[test]
fn sample_rate() {
    let chip = setup_ym2149();
    assert_eq!(chip.sample_rate(CLOCK), CLOCK / 64);
}

#[test]
fn registers_read_back_raw_values() {
    let mut chip = setup_ym2149();
    for register in 0x00..0x0Eu8 {
        write_reg_ym2149(&mut chip, register, 0xA5 ^ register);
    }
    for register in 0x00..0x0Eu8 {
        chip.write_address(register);
        assert_eq!(chip.read_data(), 0xA5 ^ register);
    }
}

#[test]
fn address_uses_the_low_four_bits() {
    let mut chip = setup_ym2149();
    write_reg_ym2149(&mut chip, 0x32, 0x5A);
    chip.write_address(0x02);
    assert_eq!(chip.read_data(), 0x5A);
    chip.write_address(0xF2);
    assert_eq!(chip.read_data(), 0x5A);
}

#[test]
fn input_ports_read_the_external_value() {
    let mut chip = setup_ym2149();
    chip.set_io_input(0, 0x3C);
    chip.set_io_input(1, 0xC3);
    write_reg_ym2149(&mut chip, 0x0E, 0x11);
    write_reg_ym2149(&mut chip, 0x0F, 0x22);
    chip.write_address(0x0E);
    assert_eq!(chip.read_data(), 0x3C);
    chip.write_address(0x0F);
    assert_eq!(chip.read_data(), 0xC3);
    assert_eq!(chip.take_io_output(0), None);
    assert_eq!(chip.take_io_output(1), None);
}

#[test]
fn output_ports_read_back_and_report_writes() {
    let mut chip = setup_ym2149();
    chip.set_io_input(0, 0x3C);
    chip.set_io_input(1, 0xC3);
    write_reg_ym2149(&mut chip, 0x07, 0xC0);
    write_reg_ym2149(&mut chip, 0x0E, 0x11);
    write_reg_ym2149(&mut chip, 0x0F, 0x22);
    assert_eq!(chip.take_io_output(0), Some(0x11));
    assert_eq!(chip.take_io_output(1), Some(0x22));
    assert_eq!(chip.take_io_output(0), None);
    chip.write_address(0x0E);
    assert_eq!(chip.read_data(), 0x11);
    chip.write_address(0x0F);
    assert_eq!(chip.read_data(), 0x22);
}

#[test]
fn bus_read_returns_data_only_in_read_mode() {
    let mut chip = setup_ym2149();
    write_reg_ym2149(&mut chip, 0x05, 0x0B);
    chip.write_address(0x05);
    assert_eq!(chip.read(0), 0xFF);
    assert_eq!(chip.read(1), 0xFF);
    assert_eq!(chip.read(2), 0xFF);
    assert_eq!(chip.read(3), 0x0B);
    assert_eq!(chip.read(7), 0x0B);
}

#[test]
fn bus_write_decodes_address_and_data_modes() {
    let mut chip = setup_ym2149();
    chip.write(0, 0x04);
    chip.write(2, 0x44);
    chip.write(3, 0x06);
    chip.write(2, 0x16);
    chip.write(1, 0x04);
    chip.write(1, 0x99);
    chip.write(6, 0x1B);
    chip.write_address(0x04);
    assert_eq!(chip.read_data(), 0x44);
    chip.write_address(0x06);
    assert_eq!(chip.read_data(), 0x1B);
}

#[test]
fn reset_clears_registers_and_keeps_the_address_latch() {
    let mut chip = setup_ym2149();
    write_reg_ym2149(&mut chip, 0x08, 0x0F);
    chip.write_address(0x08);
    chip.reset();
    assert_eq!(chip.read_data(), 0x00);
    chip.write_data(0x0C);
    chip.write_address(0x08);
    assert_eq!(chip.read_data(), 0x0C);
}
