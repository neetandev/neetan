//! Integration tests for the FM Towns sound path: the OPN2 FM chip mixing into
//! the audio buffer, and the RF5C68 PCM banked wave-RAM window.

#[path = "common/harness.rs"]
mod harness;

use common::{Bus, Machine};
use harness::machine_mx;

/// The RF5C68 wave-RAM window base in the 32-bit memory map.
const PCM_WAVE_WINDOW: u32 = 0xC220_0000;
/// RF5C68 control register (bit 6 clear selects the wave-RAM window bank).
const PCM_CONTROL_PORT: u16 = 0x04F7;

/// Programs a maximal-attack 4-operator tone on OPN2 channel 0 through the sound
/// ports, keys it on, and confirms it mixes into the audio output. Exercises the
/// whole path: I/O dispatch (0x04D8/0x04DA) -> OpnFm -> the bus audio mixer.
#[test]
fn fm_tone_mixes_into_output() {
    let mut machine = machine_mx();

    // Enable the output path like real software does: audio-out master enable
    // (0x04EC bit 6) and the FM side of the mute latch (0x04D5 bit 1).
    machine.bus.io_write_byte(0x04EC, 0x40);
    machine.bus.io_write_byte(0x04D5, 0x03);

    let fm_write =
        |machine: &mut machinetowns::TownsMachine<{ cpu::CPU_MODEL_486 }>, reg: u8, data: u8| {
            machine.bus.io_write_byte(0x04D8, reg);
            machine.bus.io_write_byte(0x04DA, data);
        };
    fm_write(&mut machine, 0xB0, 0x07); // algorithm 7 (all operators carriers)
    for op_offset in [0x00u8, 0x04, 0x08, 0x0C] {
        fm_write(&mut machine, 0x30 + op_offset, 0x01); // DT=0, MUL=1
        fm_write(&mut machine, 0x40 + op_offset, 0x00); // TL=0 (max volume)
        fm_write(&mut machine, 0x50 + op_offset, 0x1F); // KS=0, AR=31
        fm_write(&mut machine, 0x60 + op_offset, 0x00);
        fm_write(&mut machine, 0x70 + op_offset, 0x00);
        fm_write(&mut machine, 0x80 + op_offset, 0x0F); // SL=0, RR=15
        fm_write(&mut machine, 0x90 + op_offset, 0x00);
    }
    fm_write(&mut machine, 0xA4, 0x22); // block 4, F-num high
    fm_write(&mut machine, 0xA0, 0x69); // F-num low
    fm_write(&mut machine, 0xB4, 0xC0); // pan L+R
    fm_write(&mut machine, 0x28, 0xF0); // key on all operators, channel 0

    // Advance time so the keyed tone rings, then mix an audio frame.
    machine.run_for(2_000_000);
    let mut output = vec![0.0f32; 960];
    let written = machine.generate_audio_samples(1.0, &mut output);
    assert_eq!(written, output.len());
    assert!(
        output.iter().any(|&sample| sample.abs() > 1.0e-4),
        "the keyed FM tone produced no audible samples"
    );
}

/// The RF5C68 wave RAM is reached through a 4 KB memory-mapped window whose bank
/// is selected by the control register. Writing and reading the window round-trips
/// per bank, and switching banks exposes independent storage: this covers the
/// bus's memory-mapped PCM window interception and the chip's bank latch.
#[test]
fn pcm_wave_ram_window_round_trips_per_bank() {
    let mut machine = machine_mx();

    // Bank 0: write a pattern through the window and read it back.
    machine.bus.io_write_byte(PCM_CONTROL_PORT, 0x00);
    for offset in 0..16u32 {
        machine
            .bus
            .write_byte(PCM_WAVE_WINDOW + offset, (offset as u8) ^ 0x3C);
    }
    for offset in 0..16u32 {
        assert_eq!(
            machine.bus.read_byte(PCM_WAVE_WINDOW + offset),
            (offset as u8) ^ 0x3C
        );
    }

    // Bank 1 is independent storage: writing it does not disturb bank 0.
    machine.bus.io_write_byte(PCM_CONTROL_PORT, 0x01);
    machine.bus.write_byte(PCM_WAVE_WINDOW, 0xA5);
    assert_eq!(machine.bus.read_byte(PCM_WAVE_WINDOW), 0xA5);

    // Back to bank 0: the original pattern (offset ^ 0x3C) survived the switch.
    machine.bus.io_write_byte(PCM_CONTROL_PORT, 0x00);
    for offset in 0..16u32 {
        assert_eq!(
            machine.bus.read_byte(PCM_WAVE_WINDOW + offset),
            (offset as u8) ^ 0x3C
        );
    }
}
