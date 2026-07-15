//! Bus-level tests for the AT Sound Blaster 16 wiring: DSP detection at the
//! standard 0x220 ports, the AT mixer IRQ/DMA register mapping, and DMA
//! playback on the 8-bit channel 1 and the 16-bit channel 5 verified by an FFT
//! of the mixed audio output.

use std::f64::consts::PI;

use common::Bus;
use machine_at::{AtBus, LoadedRoms};
use resampler::{Complex32, Forward, Radix, RadixFFT};

const OUTPUT_SAMPLE_RATE: u32 = 48_000;
const CPU_CLOCK_HZ: u32 = 50_000_000;
const TONE_FREQ: f64 = 1000.0;

// SB16 standard AT port map (base 0x220).
const SB16_MIXER_ADDR: u16 = 0x0224;
const SB16_MIXER_DATA: u16 = 0x0225;
const SB16_DSP_RESET: u16 = 0x0226;
const SB16_DSP_READ: u16 = 0x022A;
const SB16_DSP_WRITE: u16 = 0x022C;

const RAM_BASE: u32 = 0x2_0000;

const SKIP: usize = 256;
const FFT_SIZE: usize = 4096;

fn build_bus() -> AtBus {
    let roms = LoadedRoms {
        system_bios: vec![0u8; 0x1_0000],
        vga_bios: vec![0u8; 0x8000],
    };
    AtBus::new(CPU_CLOCK_HZ, 16 << 20, roms, OUTPUT_SAMPLE_RATE)
}

fn dsp_reset(bus: &mut AtBus) {
    bus.io_write_byte(SB16_DSP_RESET, 0x01);
    bus.io_write_byte(SB16_DSP_RESET, 0x00);
    assert_eq!(
        bus.io_read_byte(SB16_DSP_READ),
        0xAA,
        "DSP did not return the ready byte after reset"
    );
}

fn dsp_write(bus: &mut AtBus, value: u8) {
    bus.io_write_byte(SB16_DSP_WRITE, value);
}

fn dsp_set_sample_rate(bus: &mut AtBus, rate: u16) {
    dsp_write(bus, 0x41);
    dsp_write(bus, (rate >> 8) as u8);
    dsp_write(bus, (rate & 0xFF) as u8);
}

fn mixer_write(bus: &mut AtBus, register: u8, value: u8) {
    bus.io_write_byte(SB16_MIXER_ADDR, register);
    bus.io_write_byte(SB16_MIXER_DATA, value);
}

fn mixer_read(bus: &mut AtBus, register: u8) -> u8 {
    bus.io_write_byte(SB16_MIXER_ADDR, register);
    bus.io_read_byte(SB16_MIXER_DATA)
}

fn write_pcm_to_ram(bus: &mut AtBus, address: u32, data: &[u8]) {
    for (index, &byte) in data.iter().enumerate() {
        bus.write_byte(address + index as u32, byte);
    }
}

/// Programs 8-bit DMA channel 1 for a memory-to-device transfer of `count`
/// bytes at `address`.
fn program_dma_channel1(bus: &mut AtBus, address: u32, count: u16) {
    let count = count - 1;
    bus.io_write_byte(0x0B, 0x49); // single, read, increment, channel 1
    bus.io_write_byte(0x0C, 0x00); // clear flip-flop
    bus.io_write_byte(0x02, address as u8);
    bus.io_write_byte(0x02, (address >> 8) as u8);
    bus.io_write_byte(0x83, (address >> 16) as u8); // channel 1 page register
    bus.io_write_byte(0x0C, 0x00);
    bus.io_write_byte(0x03, count as u8);
    bus.io_write_byte(0x03, (count >> 8) as u8);
    bus.io_write_byte(0x0A, 0x01); // unmask channel 1
}

/// Programs 16-bit DMA channel 5 for a memory-to-device transfer of `words`
/// 16-bit words at the byte `address`.
fn program_dma_channel5(bus: &mut AtBus, address: u32, words: u16) {
    let word_address = (address >> 1) as u16;
    let count = words - 1;
    bus.io_write_byte(0xD6, 0x49); // single, read, increment, local channel 1
    bus.io_write_byte(0xD8, 0x00); // clear flip-flop
    bus.io_write_byte(0xC4, word_address as u8);
    bus.io_write_byte(0xC4, (word_address >> 8) as u8);
    bus.io_write_byte(0x8B, (address >> 16) as u8); // channel 5 page register
    bus.io_write_byte(0xD8, 0x00);
    bus.io_write_byte(0xC6, count as u8);
    bus.io_write_byte(0xC6, (count >> 8) as u8);
    bus.io_write_byte(0xD4, 0x01); // unmask local channel 1 (channel 5)
}

fn advance_clock_with_events(bus: &mut AtBus, target_cycle: u64) {
    while bus.current_cycle() < target_cycle {
        let step = bus
            .next_event_cycle()
            .unwrap_or(target_cycle)
            .clamp(bus.current_cycle() + 1, target_cycle);
        bus.set_current_cycle(step);
    }
}

fn generate_sine_8bit_unsigned_mono(num_frames: usize, freq: f64, sample_rate: f64) -> Vec<u8> {
    (0..num_frames)
        .map(|index| {
            let t = index as f64 / sample_rate;
            let sample = (2.0 * PI * freq * t).sin();
            ((sample * 127.0) + 128.0) as u8
        })
        .collect()
}

fn generate_sine_16bit_signed_mono(num_frames: usize, freq: f64, sample_rate: f64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(num_frames * 2);
    for index in 0..num_frames {
        let t = index as f64 / sample_rate;
        let sample = (2.0 * PI * freq * t).sin();
        let value = (sample * 32767.0) as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn find_peak_frequency(samples: &[f32], sample_rate: u32) -> (f64, f64) {
    let n = samples.len();
    let fft = RadixFFT::<Forward>::new(vec![Radix::Factor2; n.trailing_zeros() as usize]);
    let mut scratchpad = vec![Complex32::default(); fft.scratchpad_size()];
    let mut spectrum = vec![Complex32::default(); n / 2 + 1];

    fft.process(samples, &mut spectrum, &mut scratchpad);

    let mut max_magnitude = 0.0f64;
    let mut max_bin = 1usize;
    for (bin, c) in spectrum.iter().enumerate().skip(1) {
        let magnitude = ((c.re as f64) * (c.re as f64) + (c.im as f64) * (c.im as f64)).sqrt();
        if magnitude > max_magnitude {
            max_magnitude = magnitude;
            max_bin = bin;
        }
    }

    let freq = max_bin as f64 * sample_rate as f64 / n as f64;
    let normalized_magnitude = max_magnitude * 2.0 / n as f64;
    (freq, normalized_magnitude)
}

fn setup_voice(bus: &mut AtBus) {
    mixer_write(bus, 0x30, 0xF8); // master left
    mixer_write(bus, 0x31, 0xF8); // master right
    mixer_write(bus, 0x32, 0xF8); // voice left
    mixer_write(bus, 0x33, 0xF8); // voice right
    dsp_reset(bus);
    dsp_write(bus, 0xD1); // speaker on
}

fn left_channel(output: &[f32]) -> Vec<f32> {
    output.iter().step_by(2).copied().collect()
}

#[test]
fn at_sb16_dsp_reset_and_version() {
    let mut bus = build_bus();
    dsp_reset(&mut bus);
    dsp_write(&mut bus, 0xE1);
    assert_eq!(bus.io_read_byte(SB16_DSP_READ), 4);
    assert_eq!(bus.io_read_byte(SB16_DSP_READ), 12);
}

#[test]
fn at_sb16_mixer_irq_and_dma_defaults() {
    let mut bus = build_bus();
    // Power-on defaults: IRQ 5 (0x02), 8-bit DMA 1 + 16-bit DMA 5 (0x22).
    assert_eq!(mixer_read(&mut bus, 0x80), 0x02);
    assert_eq!(mixer_read(&mut bus, 0x81), 0x22);

    mixer_write(&mut bus, 0x80, 0x04); // IRQ 7
    assert_eq!(mixer_read(&mut bus, 0x80), 0x04);
    mixer_write(&mut bus, 0x81, 0x0A); // 8-bit DMA 3 + 16-bit DMA 7
    assert_eq!(mixer_read(&mut bus, 0x81), 0x0A);
}

#[test]
fn at_sb16_8bit_mono_1khz_on_channel_1() {
    let pcm_rate = 22_050u32;
    let num_frames = 8192;
    let pcm = generate_sine_8bit_unsigned_mono(num_frames, TONE_FREQ, pcm_rate as f64);

    let mut bus = build_bus();
    setup_voice(&mut bus);
    dsp_set_sample_rate(&mut bus, pcm_rate as u16);
    write_pcm_to_ram(&mut bus, RAM_BASE, &pcm);
    program_dma_channel1(&mut bus, RAM_BASE, pcm.len() as u16);

    // 0xC0 = 8-bit output, single transfer. Mode 0x00 = unsigned mono.
    let transfer_length = (num_frames - 1) as u16;
    dsp_write(&mut bus, 0xC0);
    dsp_write(&mut bus, 0x00);
    dsp_write(&mut bus, transfer_length as u8);
    dsp_write(&mut bus, (transfer_length >> 8) as u8);

    let cycles_needed = num_frames as u64 * CPU_CLOCK_HZ as u64 / pcm_rate as u64;
    advance_clock_with_events(&mut bus, cycles_needed);

    // Terminal count on a single transfer raises the 8-bit IRQ.
    assert_eq!(mixer_read(&mut bus, 0x82) & 0x01, 0x01, "8-bit IRQ pending");

    let mut output = vec![0.0f32; 10_000 * 2];
    bus.generate_audio_samples(1.0, &mut output);
    let left = left_channel(&output);
    let (peak_freq, peak_mag) =
        find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    assert!(
        (peak_freq - TONE_FREQ).abs() < 15.0,
        "expected a peak near {TONE_FREQ} Hz, got {peak_freq:.1} Hz"
    );
    assert!(peak_mag > 0.01, "peak magnitude too low: {peak_mag:.6}");
}

#[test]
fn at_sb16_16bit_mono_1khz_on_channel_5() {
    let pcm_rate = 44_100u32;
    let num_frames = 8192;
    let pcm = generate_sine_16bit_signed_mono(num_frames, TONE_FREQ, pcm_rate as f64);

    let mut bus = build_bus();
    setup_voice(&mut bus);
    dsp_set_sample_rate(&mut bus, pcm_rate as u16);
    write_pcm_to_ram(&mut bus, RAM_BASE, &pcm);
    program_dma_channel5(&mut bus, RAM_BASE, num_frames as u16);

    // 0xB0 = 16-bit output, single transfer. Mode 0x10 = signed mono.
    let transfer_length = (num_frames - 1) as u16;
    dsp_write(&mut bus, 0xB0);
    dsp_write(&mut bus, 0x10);
    dsp_write(&mut bus, transfer_length as u8);
    dsp_write(&mut bus, (transfer_length >> 8) as u8);

    let cycles_needed = num_frames as u64 * CPU_CLOCK_HZ as u64 / pcm_rate as u64;
    advance_clock_with_events(&mut bus, cycles_needed);

    // Terminal count on a single transfer raises the 16-bit IRQ.
    assert_eq!(
        mixer_read(&mut bus, 0x82) & 0x02,
        0x02,
        "16-bit IRQ pending"
    );

    let mut output = vec![0.0f32; 10_000 * 2];
    bus.generate_audio_samples(1.0, &mut output);
    let left = left_channel(&output);
    let (peak_freq, peak_mag) =
        find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    assert!(
        (peak_freq - TONE_FREQ).abs() < 15.0,
        "expected a peak near {TONE_FREQ} Hz, got {peak_freq:.1} Hz"
    );
    assert!(peak_mag > 0.01, "peak magnitude too low: {peak_mag:.6}");
}
