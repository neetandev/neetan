//! AY-3-8910 PSG audio tests.

mod harness;

use common::Machine as _;
use harness::{build_machine, run_bus_cycles};
use machine_x1::X1Model;

fn write_psg(bus: &mut machine_x1::X1Bus, register: u8, value: u8) {
    bus.io_write(0x1C00, register); // address latch
    bus.io_write(0x1B00, value); // data
}

#[test]
fn psg_register_round_trips_through_the_latch() {
    let mut machine = build_machine(X1Model::X1);
    write_psg(&mut machine.bus, 0x08, 0x0F); // channel A amplitude
    machine.bus.io_write(0x1C00, 0x08);
    assert_eq!(machine.bus.io_read(0x1B00).0, 0x0F);
}

#[test]
fn enabled_tone_produces_non_silent_audio() {
    let mut machine = build_machine(X1Model::X1);
    // Channel A: a mid tone, full volume, tone enabled (mixer bits active low).
    write_psg(&mut machine.bus, 0x00, 0x40); // fine tune
    write_psg(&mut machine.bus, 0x01, 0x00); // coarse tune
    write_psg(&mut machine.bus, 0x08, 0x0F); // amplitude
    write_psg(&mut machine.bus, 0x07, 0x3E); // mixer: tone A on

    run_bus_cycles(&mut machine.bus, 200_000);

    let mut output = [0.0f32; 8192];
    let written = machine.generate_audio_samples(1.0, &mut output);
    assert!(written > 0);
    assert!(output[..written].iter().any(|&sample| sample != 0.0));
}

/// Writes an OPM register through the CZ-8BS1 board ports (0x0700 address,
/// 0x0701 data).
fn write_opm(bus: &mut machine_x1::X1Bus, addr: u8, data: u8) {
    bus.io_write(0x0700, addr);
    bus.io_write(0x0701, data);
}

/// Sets up a simple algorithm-7 tone on OPM channel 0 and keys it on.
fn setup_opm_tone(bus: &mut machine_x1::X1Bus) {
    write_opm(bus, 0x20, 0xC7); // pan L+R, feedback 0, algorithm 7
    for op in 0..4u8 {
        let off = op << 3; // channel 0, operator in bits 3-4
        write_opm(bus, 0x40 + off, 0x01); // DT1=0, MUL=1
        write_opm(bus, 0x60 + off, 0x00); // TL=0
        write_opm(bus, 0x80 + off, 0x1F); // KS=0, AR=31
        write_opm(bus, 0xA0 + off, 0x00); // AMS-EN=0, D1R=0
        write_opm(bus, 0xC0 + off, 0x00); // DT2=0, D2R=0
        write_opm(bus, 0xE0 + off, 0x0F); // D1L=0, RR=15
    }
    write_opm(bus, 0x28, 0x4A); // key code
    write_opm(bus, 0x30, 0x00); // key fraction
    write_opm(bus, 0x08, 0x78); // key on all four operators, channel 0
}

#[test]
fn opm_tone_produces_non_silent_audio_on_turbo() {
    let mut machine = build_machine(X1Model::X1Turbo);
    setup_opm_tone(&mut machine.bus);

    run_bus_cycles(&mut machine.bus, 200_000);

    let mut output = [0.0f32; 8192];
    let written = machine.generate_audio_samples(1.0, &mut output);
    assert!(written > 0);
    assert!(
        output[..written].iter().any(|&sample| sample != 0.0),
        "OPM tone should produce non-silent audio"
    );
}

#[test]
fn opm_timer_a_sets_pollable_status_flag() {
    let mut machine = build_machine(X1Model::X1Turbo);
    let bus = &mut machine.bus;

    // Program OPM timer A (value 0x3FF), then load + enable it (reg 0x14 = 0x05).
    write_opm(bus, 0x10, 0xFF);
    write_opm(bus, 0x11, 0x03);
    write_opm(bus, 0x14, 0x05);

    assert_eq!(
        bus.io_read(0x0701).0 & 0x01,
        0x00,
        "timer A flag should be clear before overflow"
    );

    // Advance past the (short) timer A period so the scheduled FmTimerA fires.
    run_bus_cycles(bus, 10_000);

    assert_eq!(
        bus.io_read(0x0701).0 & 0x01,
        0x01,
        "timer A overflow should set the pollable status flag"
    );
}

#[test]
fn fm_detection_reads_zero_on_turbo() {
    let mut machine = build_machine(X1Model::X1Turbo);
    // The CZ-8BS1 detection port reads back 0x00 when the board is present.
    assert_eq!(machine.bus.io_read(0x0700).0, 0x00);
}

#[test]
fn base_x1_has_no_fm_board() {
    let mut machine = build_machine(X1Model::X1);
    // The base X1 does not decode the FM board ports; the address reads open bus.
    assert_eq!(machine.bus.io_read(0x0700).0, 0xFF);

    // OPM register writes are a no-op and leave the audio path PSG-only (silent
    // with no PSG programmed).
    setup_opm_tone(&mut machine.bus);
    run_bus_cycles(&mut machine.bus, 200_000);
    let mut output = [0.0f32; 8192];
    let written = machine.generate_audio_samples(1.0, &mut output);
    assert!(written > 0);
    assert!(
        output[..written].iter().all(|&sample| sample == 0.0),
        "base X1 has no FM board, so OPM writes must not produce audio"
    );
}
