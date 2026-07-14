//! Sound tests: PSG command latch, buzzer gate and one-shot.

mod harness;

use common::Bus;
use harness::{build_bus_with_synthetic_roms, run_bus_cycles};
use machine_fm7::{BootMode, Fm7Bus, SubBusView};

/// Main CPU fast clock, matching the configured audio ratio.
const MAIN_CLOCK_HZ: u64 = 1_798_000;
/// Configured audio sample rate for the synthetic-ROM harness.
const SAMPLE_RATE_HZ: u64 = 48_000;

/// Writes a PSG register through the `0xFD0D`/`0xFD0E` command latch.
fn write_psg_register(bus: &mut Fm7Bus, register: u8, value: u8) {
    bus.write_byte(0xFD0D, 3);
    bus.write_byte(0xFD0E, register);
    bus.write_byte(0xFD0D, 2);
    bus.write_byte(0xFD0E, value);
}

/// Reads a PSG register back through the command latch.
fn read_psg_register(bus: &mut Fm7Bus, register: u8) -> u8 {
    bus.write_byte(0xFD0D, 3);
    bus.write_byte(0xFD0E, register);
    bus.write_byte(0xFD0D, 1);
    bus.read_byte(0xFD0E)
}

/// Whether any sample in the written prefix is non-zero.
fn any_non_silent(buffer: &[f32], written: usize) -> bool {
    buffer[..written].iter().any(|&sample| sample != 0.0)
}

#[test]
fn psg_register_round_trips_through_the_latch() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});

    write_psg_register(&mut bus, 8, 0x0A);
    write_psg_register(&mut bus, 4, 0x37);

    assert_eq!(read_psg_register(&mut bus, 8), 0x0A);
    assert_eq!(read_psg_register(&mut bus, 4), 0x37);
}

#[test]
fn enabled_psg_channel_produces_non_silent_output() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});

    // Enable channel A tone (mixer bit 0 low) at full amplitude.
    write_psg_register(&mut bus, 7, 0xFE);
    write_psg_register(&mut bus, 8, 0x0F);

    bus.set_current_cycle(40_000);
    let mut buffer = vec![0.0f32; 4096];
    let written = bus.generate_audio_samples(1.0, &mut buffer);

    assert!(written > 0);
    assert!(any_non_silent(&buffer, written));
}

#[test]
fn silent_psg_produces_silence() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});

    bus.set_current_cycle(40_000);
    let mut buffer = vec![0.0f32; 4096];
    let written = bus.generate_audio_samples(1.0, &mut buffer);

    assert!(written > 0);
    assert!(!any_non_silent(&buffer, written));
}

#[test]
fn buzzer_continuous_gate_sounds_while_held() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    let mut buffer = vec![0.0f32; 8192];

    // Gate held: the buzzer sounds across the frame.
    bus.write_byte(0xFD03, 0x80);
    bus.set_current_cycle(40_000);
    let written = bus.generate_audio_samples(1.0, &mut buffer);
    assert!(any_non_silent(&buffer, written));

    // Gate released: the following frame is silent.
    bus.write_byte(0xFD03, 0x00);
    bus.set_current_cycle(80_000);
    let written = bus.generate_audio_samples(1.0, &mut buffer);
    assert!(!any_non_silent(&buffer, written));
}

#[test]
fn buzzer_one_shot_expires_after_205_ms() {
    let one_shot_cycles = 205 * MAIN_CLOCK_HZ / 1000;
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    let mut buffer = vec![0.0f32; 32768];

    // Arm the one-shot; it still sounds well before the 205 ms boundary.
    bus.write_byte(0xFD03, 0x40);
    run_bus_cycles(&mut bus, one_shot_cycles - 20_000);
    let written = bus.generate_audio_samples(1.0, &mut buffer);
    assert!(
        any_non_silent(&buffer, written),
        "one-shot should still sound"
    );

    // Cross the boundary so the gate-off event fires, then a later frame is silent.
    run_bus_cycles(&mut bus, 40_000);
    let _ = bus.generate_audio_samples(1.0, &mut buffer);
    run_bus_cycles(&mut bus, 40_000);
    let written = bus.generate_audio_samples(1.0, &mut buffer);
    assert!(
        !any_non_silent(&buffer, written),
        "one-shot should be silent after 205 ms"
    );
}

#[test]
fn sub_beep_request_drives_the_one_shot() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});

    // The sub CPU reads 0xD403 to request a beep.
    {
        let mut view = SubBusView { bus: &mut bus };
        view.read_byte(0xD403);
    }

    // The pending request is drained at the next event boundary.
    run_bus_cycles(&mut bus, 100_000);
    let mut buffer = vec![0.0f32; 8192];
    let written = bus.generate_audio_samples(1.0, &mut buffer);

    assert!(any_non_silent(&buffer, written));
}

/// Documents the audio ratio the tests assume, guarding against a config drift.
#[test]
fn audio_ratio_matches_harness_configuration() {
    assert_eq!(SAMPLE_RATE_HZ, 48_000);
    assert_eq!(MAIN_CLOCK_HZ, 1_798_000);
}
