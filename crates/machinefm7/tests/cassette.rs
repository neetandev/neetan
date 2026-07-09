//! FM-7 cassette I/O tests.
//!
//! Drives the `0xFD00` motor gate and the `0xFD02` EAR read against a synthetic
//! T77 waveform and checks the printer-stub status bits.

mod harness;

use harness::{build_bus_with_synthetic_roms, run_bus_cycles};
use machinefm7::BootMode;

/// `0xFD00` write bit gating the cassette motor.
const CASSETTE_MOTOR_BIT: u8 = 0x02;
/// `0xFD02` read bit carrying the cassette EAR level.
const EAR_BIT: u8 = 0x80;
/// `0xFD02` low seven bits reporting the idle printer status (all ready).
const PRINTER_IDLE_BITS: u8 = 0x7F;
/// T77 playback sample rate (one tick every nine microseconds).
const T77_SAMPLE_RATE_HZ: u64 = 111_111;

/// Builds a T77 image from `(level, count)` pulse-width records.
fn t77_image(records: &[(bool, u16)]) -> Vec<u8> {
    let mut image = b"XM7 TAPE IMAGE 0".to_vec();
    for &(level, count) in records {
        let word = if level { 0x8000 } else { 0 } | (count & 0x7FFF);
        image.extend_from_slice(&word.to_be_bytes());
    }
    image
}

#[test]
fn ear_bit_tracks_the_waveform_while_the_motor_runs() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    // A long high run followed by a long low run.
    let image = t77_image(&[(true, 0x7FFF), (false, 0x7FFF)]);
    bus.insert_cassette("t77", &image).expect("t77 loads");

    // Motor on positions the head at the start of the high run.
    bus.write_byte(0xFD00, CASSETTE_MOTOR_BIT);
    let value = bus.read_byte(0xFD02);
    assert_eq!(value & EAR_BIT, EAR_BIT);
    assert_eq!(value & PRINTER_IDLE_BITS, PRINTER_IDLE_BITS);

    // Advance well past the high run into the low run.
    let samples_into_low = 0x7FFF + 0x2000;
    let cycles = samples_into_low * u64::from(bus.cpu_clock_hz()) / T77_SAMPLE_RATE_HZ;
    run_bus_cycles(&mut bus, cycles);
    assert_eq!(bus.read_byte(0xFD02) & EAR_BIT, 0);
}

#[test]
fn stopping_the_motor_holds_the_head_position() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    let image = t77_image(&[(true, 0x7FFF), (false, 0x7FFF)]);
    bus.insert_cassette("t77", &image).expect("t77 loads");

    bus.write_byte(0xFD00, CASSETTE_MOTOR_BIT);
    let samples_into_low = 0x7FFF + 0x2000;
    let cycles = samples_into_low * u64::from(bus.cpu_clock_hz()) / T77_SAMPLE_RATE_HZ;
    run_bus_cycles(&mut bus, cycles);
    assert_eq!(bus.read_byte(0xFD02) & EAR_BIT, 0);

    // Motor off holds the head; the EAR level stays in the low run.
    bus.write_byte(0xFD00, 0x00);
    run_bus_cycles(&mut bus, cycles);
    assert_eq!(bus.read_byte(0xFD02) & EAR_BIT, 0);
}

#[test]
fn mic_and_printer_writes_are_accepted_without_side_effects() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    // MIC (bit 0), motor (bit 1), printer strobe (bit 6) and select (bit 7).
    bus.write_byte(0xFD00, 0xC3);
    // With no tape the EAR line idles low and the printer status reads ready.
    assert_eq!(bus.read_byte(0xFD02), PRINTER_IDLE_BITS);
}
