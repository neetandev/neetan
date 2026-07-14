//! Integration tests for the FM Towns serial subsystems: the built-in RS-232C
//! USART, the kanji CG-ROM I/O alias, the serial machine-ID EEPROM, and the
//! MSM58321 real-time clock.

#[path = "common/harness.rs"]
mod harness;

use common::{Bus, Machine as _};
use harness::{fixed_time, machine_mx, machine_with_font_serial};

/// The built-in RS-232C USART reports TxRDY/TxEMPTY/DSR after init and hands back
/// an injected received byte, clearing RxRDY once the FIFO drains.
#[test]
fn rs232c_status_and_receive_via_io() {
    let mut machine = machine_mx();
    // Mode word then command word.
    machine.bus.io_write_byte(0x0A02, 0x4E);
    machine.bus.io_write_byte(0x0A02, 0x37);
    // TxRDY | TxEMPTY | DSR are always on.
    assert_eq!(machine.bus.io_read_byte(0x0A02), 0x85);
    // No received data yet.
    assert_eq!(machine.bus.io_read_byte(0x0A00), 0xFF);
    // Inject a byte: RxRDY sets and the data port returns it.
    machine.bus.push_rs232c_received_byte(0x41);
    assert_ne!(machine.bus.io_read_byte(0x0A02) & 0x02, 0);
    assert_eq!(machine.bus.io_read_byte(0x0A00), 0x41);
    // FIFO drained: RxRDY clears again.
    assert_eq!(machine.bus.io_read_byte(0x0A02) & 0x02, 0);
}

/// The RS-232C interrupt-reason bit (0x0A06) follows the interrupt-enable latch
/// (0x0A08): it only reflects RxRDY when the RxRDY source is enabled.
#[test]
fn rs232c_interrupt_reason_gated_by_enable() {
    let mut machine = machine_mx();
    machine.bus.io_write_byte(0x0A02, 0x4E);
    machine.bus.io_write_byte(0x0A02, 0x37);
    machine.bus.push_rs232c_received_byte(0x41);
    // Enabled: reason bit 0 set.
    machine.bus.io_write_byte(0x0A08, 0x02);
    assert_eq!(machine.bus.io_read_byte(0x0A06) & 0x01, 0x01);
    // Disabled: reason bit 0 clear (upper bits still float high).
    machine.bus.io_write_byte(0x0A08, 0x00);
    assert_eq!(machine.bus.io_read_byte(0x0A06) & 0x01, 0x00);
}

/// The kanji CG-ROM font read path is reachable through its I/O alias: latch a
/// JIS code at 0xFF94/0xFF95, then read the 16 glyph rows from 0xFF96 (high, no
/// advance) and 0xFF97 (low, advances the row).
#[test]
fn kanji_cg_rom_reads_through_io_alias() {
    // FONT ROM byte at offset i equals i as u8, so glyph reads are observable.
    let font: Vec<u8> = (0..0x4_0000u32).map(|offset| offset as u8).collect();
    let mut machine = machine_with_font_serial(font, vec![0; 0x20]);
    // JIS high 0, low 0x20 -> glyph code 0 (byte offset 0).
    machine.bus.io_write_byte(0xFF94, 0x00);
    machine.bus.io_write_byte(0xFF95, 0x20);
    // The JIS-high port reads back a fixed 0x80.
    assert_eq!(machine.bus.io_read_byte(0xFF94), 0x80);
    for row in 0..16u32 {
        assert_eq!(machine.bus.io_read_byte(0xFF96), (row * 2) as u8);
        assert_eq!(machine.bus.io_read_byte(0xFF97), (row * 2 + 1) as u8);
    }
    // The row wraps back to 0 after 16 rows.
    assert_eq!(machine.bus.io_read_byte(0xFF96), 0x00);
}

/// The serial machine-ID EEPROM (0x0032) is clocked one bit at a time by toggling
/// the ID-reset line, returning the ROM bits least-significant first from the last
/// byte of the array.
#[test]
fn serial_machine_id_eeprom_reads_bits() {
    let mut serial = vec![0u8; 0x20];
    serial[0x1F] = 0xA5; // bits, LSB first: 1 0 1 0 0 1 0 1
    let mut machine = machine_with_font_serial(vec![0; 0x4_0000], serial);
    let expected = [1u8, 0, 1, 0, 0, 1, 0, 1];
    for &bit in expected.iter() {
        assert_eq!(machine.bus.io_read_byte(0x0032) & 0x01, bit);
        // Advance to the next bit: a rising ID-reset edge while selected.
        machine.bus.io_write_byte(0x0032, 0x00);
        machine.bus.io_write_byte(0x0032, 0x40);
    }
}

// MSM58321 register addresses selected through the ADDRESS-WRITE strobe.
const RTC_REG_ONE_SECOND: u8 = 0x00;
const RTC_REG_TEN_SECOND: u8 = 0x01;
const RTC_REG_ONE_MINUTE: u8 = 0x02;
const RTC_REG_TEN_MINUTE: u8 = 0x03;
const RTC_REG_ONE_HOUR: u8 = 0x04;
const RTC_REG_TEN_HOUR: u8 = 0x05;
const RTC_REG_WEEKDAY: u8 = 0x06;
const RTC_REG_ONE_DAY: u8 = 0x07;
const RTC_REG_ONE_MONTH: u8 = 0x09;

const RTC_DATA_PORT: u16 = 0x0070;
const RTC_COMMAND_PORT: u16 = 0x0080;
const RTC_ENABLE: u8 = 0x80;
const RTC_ADDRESS_WRITE: u8 = 0x81;
const RTC_READ: u8 = 0x84;
const RTC_READY_BIT: u8 = 0x80;
const RTC_DIGIT_MASK: u8 = 0x0F;

/// Selects and reads back one RTC register through the data/command ports,
/// returning the raw data byte (ready flag in bit 7, digit in the low nibble).
fn read_rtc_register(
    machine: &mut machine_towns::TownsMachine<{ cpu::CPU_MODEL_486_DX }>,
    reg: u8,
) -> u8 {
    machine.bus.io_write_byte(RTC_COMMAND_PORT, RTC_ENABLE);
    machine.bus.io_write_byte(RTC_DATA_PORT, reg);
    machine
        .bus
        .io_write_byte(RTC_COMMAND_PORT, RTC_ADDRESS_WRITE);
    machine.bus.io_write_byte(RTC_COMMAND_PORT, RTC_READ);
    machine.bus.io_read_byte(RTC_DATA_PORT)
}

/// The MSM58321 hands back the injected host time, one BCD digit per register,
/// through the data/command port protocol. The fixed time is 12:34:56 on
/// 2000-01-01 (Saturday), so each digit is deterministic.
#[test]
fn rtc_registers_return_fixed_host_time() {
    let mut machine = machine_mx();
    machine.set_host_date_time_provider(fixed_time);

    let digit = |machine: &mut _, reg| read_rtc_register(machine, reg) & RTC_DIGIT_MASK;

    assert_eq!(digit(&mut machine, RTC_REG_ONE_SECOND), 6);
    assert_eq!(digit(&mut machine, RTC_REG_TEN_SECOND), 5);
    assert_eq!(digit(&mut machine, RTC_REG_ONE_MINUTE), 4);
    assert_eq!(digit(&mut machine, RTC_REG_TEN_MINUTE), 3);
    assert_eq!(digit(&mut machine, RTC_REG_ONE_HOUR), 2);
    // Ten-hour register carries the 24-hour flag (bit 3) plus the tens digit (1).
    assert_eq!(digit(&mut machine, RTC_REG_TEN_HOUR), 0x08 | 1);
    assert_eq!(digit(&mut machine, RTC_REG_WEEKDAY), 6);
    assert_eq!(digit(&mut machine, RTC_REG_ONE_DAY), 1);
    assert_eq!(digit(&mut machine, RTC_REG_ONE_MONTH), 1);
}

/// The RTC ready flag (data-port bit 7) reads low during the first ~674 us of each
/// second and high afterward, so polling software can wait for a stable reading.
#[test]
fn rtc_ready_flag_tracks_subsecond_time() {
    let mut machine = machine_mx();
    machine.set_host_date_time_provider(fixed_time);

    // At cycle 0 the second has just started: the ready flag is low.
    machine.bus.set_current_cycle(0);
    assert_eq!(
        read_rtc_register(&mut machine, RTC_REG_ONE_SECOND) & RTC_READY_BIT,
        0
    );

    // Advance well past 674 us (at 66 MHz, ~44.5k cycles); the ready flag sets.
    machine.bus.set_current_cycle(200_000);
    assert_ne!(
        read_rtc_register(&mut machine, RTC_REG_ONE_SECOND) & RTC_READY_BIT,
        0
    );
}
