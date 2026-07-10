//! RTC and system-port integration tests: the uPD4990A real-time clock readout
//! and the SYSPORTVA mode switch, speed/DIP-switch and CRT-mode ports, all driven
//! through the public bus surface.

use common::{Bus, HostDateTime, Machine as _};
use machine88va::Pc88VaMachine;

#[path = "common/harness.rs"]
mod harness;
use harness::*;

fn test_time() -> HostDateTime {
    HostDateTime {
        year: 2026,
        month: 3,
        day: 3,
        day_of_week: 1,
        hour: 14,
        minute: 30,
        second: 45,
    }
}

fn rtc_stb_command(machine: &mut Pc88VaMachine, command: u8) {
    // DATA phase latches the command on port 0x010.
    machine.bus.io_write_byte(0x010, command & 0x07);
    machine.bus.io_write_byte(0x040, 0x00);
    // STB rising edge (port 0x040 bit 1).
    machine.bus.io_write_byte(0x040, 0x02);
    // Release STB.
    machine.bus.io_write_byte(0x040, 0x00);
}

fn rtc_clock_pulse(machine: &mut Pc88VaMachine) {
    // CLK rising edge (port 0x040 bit 2), then release.
    machine.bus.io_write_byte(0x040, 0x04);
    machine.bus.io_write_byte(0x040, 0x00);
}

fn rtc_cdat(machine: &mut Pc88VaMachine) -> u8 {
    (machine.bus.io_read_byte(0x040) >> 4) & 0x01
}

#[test]
fn rtc_time_read_shifts_out_calendar() {
    let mut machine = machine();
    machine.set_host_date_time_provider(test_time);

    // Time Read loads the calendar; Register Shift starts the readout.
    rtc_stb_command(&mut machine, 0x03);
    rtc_stb_command(&mut machine, 0x01);

    // reg[7] = seconds = 0x45 = 0100_0101, shifted out LSB-first per position.
    assert_eq!(rtc_cdat(&mut machine), 1);
    rtc_clock_pulse(&mut machine);
    assert_eq!(rtc_cdat(&mut machine), 0);
    rtc_clock_pulse(&mut machine);
    assert_eq!(rtc_cdat(&mut machine), 1);
}

#[test]
fn mode_switch_round_trips_through_0x150_0x151() {
    let mut machine = machine();
    machine.bus.io_write_byte(0x1C6, 0x01);
    assert_eq!(machine.bus.io_read_byte(0x150), 0xFE);
    assert_eq!(machine.bus.io_read_byte(0x151), 0xFF);

    machine.bus.io_write_byte(0x1C6, 0x00);
    assert_eq!(machine.bus.io_read_byte(0x150), 0xFD);
    assert_eq!(machine.bus.io_read_byte(0x151), 0xFF);
}

#[test]
fn speed_bit_and_dip_switches_read_back() {
    let mut machine = machine();
    // bit1 sets the SPEED bit (bit5) in the DIP-switch read at 0x1C9.
    machine.bus.io_write_byte(0x1C6, 0x02);
    assert_eq!(machine.bus.io_read_byte(0x1C9) & 0x20, 0x20);
    // Fixed bits 7,6,0 are always set.
    assert_eq!(machine.bus.io_read_byte(0x1C9) & 0xC1, 0xC1);
}

#[test]
fn crt_mode_bit_reflects_configured_dip() {
    let mut machine = machine();
    // Default DIP selects 24.8 kHz: 0x040 bit1 clear, 0x1CB bit3 set.
    assert_eq!(machine.bus.io_read_byte(0x040) & 0x02, 0x00);
    assert_eq!(machine.bus.io_read_byte(0x1CB) & 0x08, 0x08);
    // System port 4 fixed bits: 7,6,0.
    assert_eq!(machine.bus.io_read_byte(0x040) & 0xC1, 0xC1);
}

#[test]
fn system_port_5_round_trips_masked() {
    let mut machine = machine();
    machine.bus.io_write_byte(0x190, 0xFF);
    assert_eq!(machine.bus.io_read_byte(0x190), 0x1D);
}
