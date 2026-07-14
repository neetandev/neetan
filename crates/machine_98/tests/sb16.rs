use std::f64::consts::PI;

use common::{Bus, CpuMode, MachineModel, NoTrace};
use machine_98::Pc9801Bus;
use resampler::{Complex32, Forward, Radix, RadixFFT};

const OUTPUT_SAMPLE_RATE: u32 = 48_000;
const CPU_CLOCK_HZ: u32 = 20_000_000;
const TONE_FREQ: f64 = 1000.0;

// SB16 default base port = 0xD2
const SB16_DSP_RESET: u16 = 0x26D2; // base + 0x2600
const SB16_DSP_READ: u16 = 0x2AD2; // base + 0x2A00
const SB16_DSP_WRITE: u16 = 0x2CD2; // base + 0x2C00
const SB16_MIXER_ADDR: u16 = 0x24D2; // base + 0x2400
const SB16_MIXER_DATA: u16 = 0x25D2; // base + 0x2500
const SB16_OPL3_BANK0_STATUS: u16 = 0x20D2; // base + 0x2000
const SB16_OPL3_BANK1_STATUS: u16 = 0x22D2; // base + 0x2200
const SB16_OPL3_BANK1_DATA: u16 = 0x23D2; // base + 0x2300 (write-only)
const SB16_OPL3_BANK0_PROBE_PARTNER: u16 = 0x21D1; // misaligned probe partner of bank-0 data
const SB16_OPL3_BANK1_PROBE_PARTNER: u16 = 0x23D1; // misaligned probe partner of bank-1 data

// DMA channel 3 I/O ports (PC-98)
const DMA_CH3_ADDR: u16 = 0x0D;
const DMA_CH3_COUNT: u16 = 0x0F;
const DMA_CH3_PAGE: u16 = 0x25;
const DMA_MODE: u16 = 0x17;
const DMA_FLIP_FLOP_CLEAR: u16 = 0x19;
const DMA_SINGLE_MASK: u16 = 0x15;

const RAM_BASE: u32 = 0x10000;

const SKIP: usize = 256;
const FFT_SIZE: usize = 4096;

fn setup_sb16_bus() -> Pc9801Bus<NoTrace> {
    let mut bus =
        Pc9801Bus::<NoTrace>::new(MachineModel::PC9801RA, CpuMode::High, OUTPUT_SAMPLE_RATE);
    bus.install_sound_blaster_16();
    bus
}

fn dsp_reset(bus: &mut Pc9801Bus<NoTrace>) {
    bus.io_write_byte(SB16_DSP_RESET, 0x01);
    bus.io_write_byte(SB16_DSP_RESET, 0x00);
    let ready = bus.io_read_byte(SB16_DSP_READ);
    assert_eq!(ready, 0xAA, "DSP did not return ready byte after reset");
}

fn dsp_write(bus: &mut Pc9801Bus<NoTrace>, value: u8) {
    bus.io_write_byte(SB16_DSP_WRITE, value);
}

fn dsp_set_sample_rate(bus: &mut Pc9801Bus<NoTrace>, rate: u16) {
    dsp_write(bus, 0x41); // Set output sample rate
    dsp_write(bus, (rate >> 8) as u8);
    dsp_write(bus, (rate & 0xFF) as u8);
}

fn dsp_speaker_on(bus: &mut Pc9801Bus<NoTrace>) {
    dsp_write(bus, 0xD1);
}

fn mixer_write(bus: &mut Pc9801Bus<NoTrace>, register: u8, value: u8) {
    bus.io_write_byte(SB16_MIXER_ADDR, register);
    bus.io_write_byte(SB16_MIXER_DATA, value);
}

fn mixer_read(bus: &mut Pc9801Bus<NoTrace>, register: u8) -> u8 {
    bus.io_write_byte(SB16_MIXER_ADDR, register);
    bus.io_read_byte(SB16_MIXER_DATA)
}

/// Writes PCM data into bus RAM starting at `address`.
fn write_pcm_to_ram(bus: &mut Pc9801Bus<NoTrace>, address: u32, data: &[u8]) {
    for (i, &byte) in data.iter().enumerate() {
        bus.write_byte(address + i as u32, byte);
    }
}

/// Sets up DMA channel 3 to read `count` bytes from the given physical address.
fn setup_dma_channel_3(bus: &mut Pc9801Bus<NoTrace>, address: u32, count: u16) {
    let page = ((address >> 16) & 0xFF) as u8;
    let addr_lo = (address & 0xFF) as u8;
    let addr_hi = ((address >> 8) & 0xFF) as u8;
    let count_lo = (count & 0xFF) as u8;
    let count_hi = ((count >> 8) & 0xFF) as u8;

    // Clear flip-flop
    bus.io_write_byte(DMA_FLIP_FLOP_CLEAR, 0x00);
    // Set address (lo, hi)
    bus.io_write_byte(DMA_CH3_ADDR, addr_lo);
    bus.io_write_byte(DMA_CH3_ADDR, addr_hi);
    // Clear flip-flop again for count
    bus.io_write_byte(DMA_FLIP_FLOP_CLEAR, 0x00);
    // Set count (lo, hi) - count is N-1 for DMA
    bus.io_write_byte(DMA_CH3_COUNT, count_lo);
    bus.io_write_byte(DMA_CH3_COUNT, count_hi);
    // Set page
    bus.io_write_byte(DMA_CH3_PAGE, page);
    // Set mode: single transfer, read (memory->device), channel 3, auto-init off, increment
    bus.io_write_byte(DMA_MODE, 0x43); // bits: 01 00 0 0 11 (single, read, no-autoinit, incr, ch3)
    // Unmask channel 3
    bus.io_write_byte(DMA_SINGLE_MASK, 0x03); // bit 2 = 0 (unmask), channel = 3
}

/// Advances the bus clock in steps, processing DMA events along the way.
fn advance_clock_with_events(bus: &mut Pc9801Bus<NoTrace>, target_cycle: u64) {
    let step = CPU_CLOCK_HZ as u64 / 1000; // ~1ms steps
    let mut cycle = bus.current_cycle();
    while cycle < target_cycle {
        cycle = (cycle + step).min(target_cycle);
        bus.set_current_cycle(cycle);
    }
}

fn generate_sine_16bit_signed_stereo(num_frames: usize, freq: f64, sample_rate: f64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(num_frames * 4);
    for i in 0..num_frames {
        let t = i as f64 / sample_rate;
        let sample = (2.0 * PI * freq * t).sin();
        let value = (sample * 32767.0) as i16;
        let [lo, hi] = value.to_le_bytes();
        // Left
        bytes.push(lo);
        bytes.push(hi);
        // Right
        bytes.push(lo);
        bytes.push(hi);
    }
    bytes
}

fn generate_sine_16bit_signed_mono(num_frames: usize, freq: f64, sample_rate: f64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(num_frames * 2);
    for i in 0..num_frames {
        let t = i as f64 / sample_rate;
        let sample = (2.0 * PI * freq * t).sin();
        let value = (sample * 32767.0) as i16;
        let [lo, hi] = value.to_le_bytes();
        bytes.push(lo);
        bytes.push(hi);
    }
    bytes
}

fn generate_sine_8bit_unsigned_stereo(num_frames: usize, freq: f64, sample_rate: f64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(num_frames * 2);
    for i in 0..num_frames {
        let t = i as f64 / sample_rate;
        let sample = (2.0 * PI * freq * t).sin();
        let value = ((sample * 127.0) + 128.0) as u8;
        bytes.push(value);
        bytes.push(value);
    }
    bytes
}

fn generate_sine_8bit_unsigned_mono(num_frames: usize, freq: f64, sample_rate: f64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(num_frames);
    for i in 0..num_frames {
        let t = i as f64 / sample_rate;
        let sample = (2.0 * PI * freq * t).sin();
        let value = ((sample * 127.0) + 128.0) as u8;
        bytes.push(value);
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
    for (i, c) in spectrum.iter().enumerate().skip(1) {
        let mag = ((c.re as f64) * (c.re as f64) + (c.im as f64) * (c.im as f64)).sqrt();
        if mag > max_magnitude {
            max_magnitude = mag;
            max_bin = i;
        }
    }

    let freq = max_bin as f64 * sample_rate as f64 / n as f64;
    let normalized_magnitude = max_magnitude * 2.0 / n as f64;
    (freq, normalized_magnitude)
}

/// Runs a full SB16 DMA playback cycle and returns (left_channel, right_channel) output.
fn run_sb16_dma_playback(
    pcm_data: &[u8],
    pcm_rate: u32,
    dsp_cmd: u8,
    mode_byte: u8,
    num_frames: usize,
) -> (Vec<f32>, Vec<f32>) {
    let mut bus = setup_sb16_bus();

    // Set mixer volumes to max
    mixer_write(&mut bus, 0x30, 0xF8); // Master left
    mixer_write(&mut bus, 0x31, 0xF8); // Master right
    mixer_write(&mut bus, 0x32, 0xF8); // Voice left
    mixer_write(&mut bus, 0x33, 0xF8); // Voice right

    // Reset DSP
    dsp_reset(&mut bus);

    // Set sample rate
    dsp_set_sample_rate(&mut bus, pcm_rate as u16);

    // Turn speaker on
    dsp_speaker_on(&mut bus);

    // Write PCM data to RAM
    write_pcm_to_ram(&mut bus, RAM_BASE, pcm_data);

    // Set up DMA channel 3
    let dma_count = (pcm_data.len() - 1) as u16;
    setup_dma_channel_3(&mut bus, RAM_BASE, dma_count);

    // Calculate transfer length for the DSP command.
    // For SB16-style commands (0xBx/0xCx), the length counts individual samples
    // (both channels interleaved for stereo), not frames or bytes.
    let is_stereo = mode_byte & 0x20 != 0;
    let channels: usize = if is_stereo { 2 } else { 1 };
    let transfer_length = (num_frames * channels - 1) as u16;

    // Start DMA playback via DSP command
    dsp_write(&mut bus, dsp_cmd);
    dsp_write(&mut bus, mode_byte);
    dsp_write(&mut bus, (transfer_length & 0xFF) as u8);
    dsp_write(&mut bus, (transfer_length >> 8) as u8);

    // Advance clock to cover all PCM data, processing DMA events along the way
    let cycles_needed = (num_frames as u64) * CPU_CLOCK_HZ as u64 / pcm_rate as u64;
    advance_clock_with_events(&mut bus, cycles_needed);

    // Generate audio output
    let output_frames = 10_000;
    let mut output = vec![0.0f32; output_frames * 2];
    bus.generate_audio_samples(1.0, &mut output);

    let left: Vec<f32> = output.iter().step_by(2).copied().collect();
    let right: Vec<f32> = output.iter().skip(1).step_by(2).copied().collect();

    assert!(
        left.len() >= SKIP + FFT_SIZE,
        "Not enough output samples: got {}, need {}",
        left.len(),
        SKIP + FFT_SIZE
    );

    (left, right)
}

// ---- Tests ----

#[test]
fn sb16_dsp_reset_and_version() {
    let mut bus = setup_sb16_bus();
    dsp_reset(&mut bus);

    dsp_write(&mut bus, 0xE1);
    assert_eq!(bus.io_read_byte(SB16_DSP_READ), 4);
    assert_eq!(bus.io_read_byte(SB16_DSP_READ), 12);
}

#[test]
fn sb16_mixer_irq_and_dma_config() {
    let mut bus = setup_sb16_bus();

    // Default IRQ is PC-98 IRQ 5, which reads back as 0x08 (ISA IRQ 10 mapping).
    assert_eq!(mixer_read(&mut bus, 0x80), 0x08);
    // Default DMA should be 3 (code 0x08)
    assert_eq!(mixer_read(&mut bus, 0x81), 0x08);

    // Write 0x02 (ISA IRQ 5 -> PC-98 IRQ 10), reads back as 0x02.
    mixer_write(&mut bus, 0x80, 0x02);
    assert_eq!(mixer_read(&mut bus, 0x80), 0x02);

    // Change DMA to 0 (code 0x01)
    mixer_write(&mut bus, 0x81, 0x01);
    assert_eq!(mixer_read(&mut bus, 0x81), 0x01);
}

#[test]
fn sb16_16bit_stereo_signed_1khz() {
    let pcm_rate = 44100u32;
    let num_frames = 8192;
    let pcm_data = generate_sine_16bit_signed_stereo(num_frames, TONE_FREQ, pcm_rate as f64);

    // 0xB0 = 16-bit DMA output, single transfer
    // mode_byte: bit 4 = signed (0x10), bit 5 = stereo (0x20) -> 0x30
    let (left, right) = run_sb16_dma_playback(&pcm_data, pcm_rate, 0xB0, 0x30, num_frames);

    let (peak_freq, peak_mag) =
        find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("16-bit stereo left: peak_freq={peak_freq:.1} Hz, peak_mag={peak_mag:.6}");
    assert!(
        (peak_freq - TONE_FREQ).abs() < 15.0,
        "Expected peak near {TONE_FREQ} Hz, got {peak_freq:.1} Hz"
    );
    assert!(peak_mag > 0.01, "Peak magnitude too low: {peak_mag:.6}");

    let (peak_freq_r, peak_mag_r) =
        find_peak_frequency(&right[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("16-bit stereo right: peak_freq={peak_freq_r:.1} Hz, peak_mag={peak_mag_r:.6}");
    assert!(
        (peak_freq_r - TONE_FREQ).abs() < 15.0,
        "Right channel: expected peak near {TONE_FREQ} Hz, got {peak_freq_r:.1} Hz"
    );
    assert!(
        peak_mag_r > 0.01,
        "Right channel: peak magnitude too low: {peak_mag_r:.6}"
    );
}

#[test]
fn sb16_16bit_mono_signed_1khz() {
    let pcm_rate = 44100u32;
    let num_frames = 8192;
    let pcm_data = generate_sine_16bit_signed_mono(num_frames, TONE_FREQ, pcm_rate as f64);

    // 0xB0 = 16-bit DMA output, single transfer
    // mode_byte: bit 4 = signed (0x10), bit 5 = 0 (mono) -> 0x10
    let (left, right) = run_sb16_dma_playback(&pcm_data, pcm_rate, 0xB0, 0x10, num_frames);

    let (peak_freq, peak_mag) =
        find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("16-bit mono left: peak_freq={peak_freq:.1} Hz, peak_mag={peak_mag:.6}");
    assert!(
        (peak_freq - TONE_FREQ).abs() < 15.0,
        "Expected peak near {TONE_FREQ} Hz, got {peak_freq:.1} Hz"
    );
    assert!(peak_mag > 0.01, "Peak magnitude too low: {peak_mag:.6}");

    // Mono should duplicate to both channels
    let (peak_freq_r, peak_mag_r) =
        find_peak_frequency(&right[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("16-bit mono right: peak_freq={peak_freq_r:.1} Hz, peak_mag={peak_mag_r:.6}");
    assert!(
        (peak_freq_r - TONE_FREQ).abs() < 15.0,
        "Right channel: expected peak near {TONE_FREQ} Hz, got {peak_freq_r:.1} Hz"
    );
}

#[test]
fn sb16_8bit_unsigned_stereo_1khz() {
    let pcm_rate = 22050u32;
    let num_frames = 8192;
    let pcm_data = generate_sine_8bit_unsigned_stereo(num_frames, TONE_FREQ, pcm_rate as f64);

    // 0xC0 = 8-bit DMA output, single transfer
    // mode_byte: bit 5 = stereo (0x20), unsigned -> 0x20
    let (left, right) = run_sb16_dma_playback(&pcm_data, pcm_rate, 0xC0, 0x20, num_frames);

    let (peak_freq, peak_mag) =
        find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("8-bit stereo left: peak_freq={peak_freq:.1} Hz, peak_mag={peak_mag:.6}");
    assert!(
        (peak_freq - TONE_FREQ).abs() < 15.0,
        "Expected peak near {TONE_FREQ} Hz, got {peak_freq:.1} Hz"
    );
    assert!(peak_mag > 0.005, "Peak magnitude too low: {peak_mag:.6}");

    let (peak_freq_r, peak_mag_r) =
        find_peak_frequency(&right[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("8-bit stereo right: peak_freq={peak_freq_r:.1} Hz, peak_mag={peak_mag_r:.6}");
    assert!(
        (peak_freq_r - TONE_FREQ).abs() < 15.0,
        "Right channel: expected peak near {TONE_FREQ} Hz, got {peak_freq_r:.1} Hz"
    );
}

#[test]
fn sb16_8bit_unsigned_mono_1khz() {
    let pcm_rate = 22050u32;
    let num_frames = 8192;
    let pcm_data = generate_sine_8bit_unsigned_mono(num_frames, TONE_FREQ, pcm_rate as f64);

    // 0xC0 = 8-bit DMA output, single transfer
    // mode_byte: unsigned mono -> 0x00
    let (left, right) = run_sb16_dma_playback(&pcm_data, pcm_rate, 0xC0, 0x00, num_frames);

    let (peak_freq, peak_mag) =
        find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("8-bit mono left: peak_freq={peak_freq:.1} Hz, peak_mag={peak_mag:.6}");
    assert!(
        (peak_freq - TONE_FREQ).abs() < 15.0,
        "Expected peak near {TONE_FREQ} Hz, got {peak_freq:.1} Hz"
    );
    assert!(peak_mag > 0.005, "Peak magnitude too low: {peak_mag:.6}");

    // Mono duplicated to right
    let (peak_freq_r, _) = find_peak_frequency(&right[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    assert!(
        (peak_freq_r - TONE_FREQ).abs() < 15.0,
        "Right channel: expected peak near {TONE_FREQ} Hz, got {peak_freq_r:.1} Hz"
    );
}

#[test]
fn sb16_16bit_stereo_440hz() {
    let freq = 440.0;
    let pcm_rate = 44100u32;
    let num_frames = 8192;
    let pcm_data = generate_sine_16bit_signed_stereo(num_frames, freq, pcm_rate as f64);

    let (left, right) = run_sb16_dma_playback(&pcm_data, pcm_rate, 0xB0, 0x30, num_frames);

    let (peak_freq, peak_mag) =
        find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("440 Hz left: peak_freq={peak_freq:.1} Hz, peak_mag={peak_mag:.6}");
    assert!(
        (peak_freq - freq).abs() < 15.0,
        "Expected peak near {freq} Hz, got {peak_freq:.1} Hz"
    );
    assert!(peak_mag > 0.01, "Peak magnitude too low: {peak_mag:.6}");

    let (peak_freq_r, peak_mag_r) =
        find_peak_frequency(&right[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("440 Hz right: peak_freq={peak_freq_r:.1} Hz, peak_mag={peak_mag_r:.6}");
    assert!(
        (peak_freq_r - freq).abs() < 15.0,
        "Right channel: expected peak near {freq} Hz, got {peak_freq_r:.1} Hz"
    );
    assert!(
        peak_mag_r > 0.01,
        "Right channel: peak magnitude too low: {peak_mag_r:.6}"
    );
}

#[test]
fn sb16_16bit_stereo_5khz() {
    let freq = 5000.0;
    let pcm_rate = 44100u32;
    let num_frames = 8192;
    let pcm_data = generate_sine_16bit_signed_stereo(num_frames, freq, pcm_rate as f64);

    let (left, right) = run_sb16_dma_playback(&pcm_data, pcm_rate, 0xB0, 0x30, num_frames);

    let (peak_freq, peak_mag) =
        find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("5 kHz left: peak_freq={peak_freq:.1} Hz, peak_mag={peak_mag:.6}");
    assert!(
        (peak_freq - freq).abs() < 15.0,
        "Expected peak near {freq} Hz, got {peak_freq:.1} Hz"
    );
    assert!(peak_mag > 0.01, "Peak magnitude too low: {peak_mag:.6}");

    let (peak_freq_r, peak_mag_r) =
        find_peak_frequency(&right[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("5 kHz right: peak_freq={peak_freq_r:.1} Hz, peak_mag={peak_mag_r:.6}");
    assert!(
        (peak_freq_r - freq).abs() < 15.0,
        "Right channel: expected peak near {freq} Hz, got {peak_freq_r:.1} Hz"
    );
}

#[test]
fn sb16_speaker_off_produces_silence() {
    let pcm_rate = 44100u32;
    let num_frames = 4096;
    let pcm_data = generate_sine_16bit_signed_stereo(num_frames, TONE_FREQ, pcm_rate as f64);

    let mut bus = setup_sb16_bus();
    mixer_write(&mut bus, 0x30, 0xF8);
    mixer_write(&mut bus, 0x31, 0xF8);
    mixer_write(&mut bus, 0x32, 0xF8);
    mixer_write(&mut bus, 0x33, 0xF8);

    dsp_reset(&mut bus);
    dsp_set_sample_rate(&mut bus, pcm_rate as u16);
    // Deliberately do NOT turn speaker on

    write_pcm_to_ram(&mut bus, RAM_BASE, &pcm_data);
    setup_dma_channel_3(&mut bus, RAM_BASE, (pcm_data.len() - 1) as u16);

    let transfer_length = (num_frames - 1) as u16;
    dsp_write(&mut bus, 0xB0);
    dsp_write(&mut bus, 0x30);
    dsp_write(&mut bus, (transfer_length & 0xFF) as u8);
    dsp_write(&mut bus, (transfer_length >> 8) as u8);

    let cycles_needed = (num_frames as u64) * CPU_CLOCK_HZ as u64 / pcm_rate as u64;
    advance_clock_with_events(&mut bus, cycles_needed);

    let mut output = vec![0.0f32; 5000 * 2];
    bus.generate_audio_samples(1.0, &mut output);

    let max_abs = output.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    eprintln!("Speaker off max amplitude: {max_abs:.6}");
    assert!(
        max_abs < 0.01,
        "Expected near-silence with speaker off, got max amplitude {max_abs:.6}"
    );
}

#[test]
fn sb16_mixer_volume_attenuates_signal() {
    fn run_with_voice_volume(vol: u8) -> f64 {
        let pcm_rate = 44100u32;
        let num_frames = 8192;
        let pcm_data = generate_sine_16bit_signed_stereo(num_frames, TONE_FREQ, pcm_rate as f64);

        let mut bus = setup_sb16_bus();
        mixer_write(&mut bus, 0x30, 0xF8); // Master max
        mixer_write(&mut bus, 0x31, 0xF8);
        mixer_write(&mut bus, 0x32, vol); // Voice left
        mixer_write(&mut bus, 0x33, vol); // Voice right

        dsp_reset(&mut bus);
        dsp_set_sample_rate(&mut bus, pcm_rate as u16);
        dsp_speaker_on(&mut bus);

        write_pcm_to_ram(&mut bus, RAM_BASE, &pcm_data);
        setup_dma_channel_3(&mut bus, RAM_BASE, (pcm_data.len() - 1) as u16);

        let transfer_length = (num_frames - 1) as u16;
        dsp_write(&mut bus, 0xB0);
        dsp_write(&mut bus, 0x30);
        dsp_write(&mut bus, (transfer_length & 0xFF) as u8);
        dsp_write(&mut bus, (transfer_length >> 8) as u8);

        let cycles_needed = (num_frames as u64) * CPU_CLOCK_HZ as u64 / pcm_rate as u64;
        advance_clock_with_events(&mut bus, cycles_needed);

        let mut output = vec![0.0f32; 10_000 * 2];
        bus.generate_audio_samples(1.0, &mut output);

        let left: Vec<f32> = output.iter().step_by(2).copied().collect();
        let (_freq, mag) = find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
        mag
    }

    let mag_max = run_with_voice_volume(0xF8); // ~max
    let mag_mid = run_with_voice_volume(0x80); // mid
    let mag_low = run_with_voice_volume(0x20); // low

    eprintln!("Volume F8: {mag_max:.6}, Volume 80: {mag_mid:.6}, Volume 20: {mag_low:.6}");

    assert!(
        mag_max > mag_mid * 1.3,
        "Max volume ({mag_max:.6}) should be louder than mid ({mag_mid:.6})"
    );
    assert!(
        mag_mid > mag_low * 1.3,
        "Mid volume ({mag_mid:.6}) should be louder than low ({mag_low:.6})"
    );
}

#[test]
fn sb16_dma_irq_fires_on_block_complete() {
    let pcm_rate = 22050u32;
    let num_frames = 1024;
    let pcm_data = generate_sine_8bit_unsigned_mono(num_frames, TONE_FREQ, pcm_rate as f64);

    let mut bus = setup_sb16_bus();
    dsp_reset(&mut bus);
    dsp_set_sample_rate(&mut bus, pcm_rate as u16);
    dsp_speaker_on(&mut bus);

    write_pcm_to_ram(&mut bus, RAM_BASE, &pcm_data);
    setup_dma_channel_3(&mut bus, RAM_BASE, (pcm_data.len() - 1) as u16);

    // 8-bit DMA single transfer
    let transfer_length = (num_frames - 1) as u16;
    dsp_write(&mut bus, 0xC0);
    dsp_write(&mut bus, 0x00);
    dsp_write(&mut bus, (transfer_length & 0xFF) as u8);
    dsp_write(&mut bus, (transfer_length >> 8) as u8);

    // Advance enough for all data to be transferred
    let cycles_needed = (num_frames as u64 + 256) * CPU_CLOCK_HZ as u64 / pcm_rate as u64;
    advance_clock_with_events(&mut bus, cycles_needed);

    // Check that 8-bit IRQ is pending
    let irq_status = mixer_read(&mut bus, 0x82);
    eprintln!("IRQ status after block complete: {irq_status:#04X}");
    assert!(
        irq_status & 0x01 != 0,
        "Expected 8-bit IRQ pending after DMA block complete, got status {irq_status:#04X}"
    );
}

#[test]
fn sb16_16bit_dma_irq_fires_on_block_complete() {
    let pcm_rate = 22050u32;
    let num_frames = 1024;
    let pcm_data = generate_sine_16bit_signed_mono(num_frames, TONE_FREQ, pcm_rate as f64);

    let mut bus = setup_sb16_bus();
    dsp_reset(&mut bus);
    dsp_set_sample_rate(&mut bus, pcm_rate as u16);
    dsp_speaker_on(&mut bus);

    write_pcm_to_ram(&mut bus, RAM_BASE, &pcm_data);
    setup_dma_channel_3(&mut bus, RAM_BASE, (pcm_data.len() - 1) as u16);

    let transfer_length = (num_frames - 1) as u16;
    dsp_write(&mut bus, 0xB0);
    dsp_write(&mut bus, 0x10); // signed mono
    dsp_write(&mut bus, (transfer_length & 0xFF) as u8);
    dsp_write(&mut bus, (transfer_length >> 8) as u8);

    let cycles_needed = (num_frames as u64 + 256) * CPU_CLOCK_HZ as u64 / pcm_rate as u64;
    advance_clock_with_events(&mut bus, cycles_needed);

    let irq_status = mixer_read(&mut bus, 0x82);
    eprintln!("IRQ status after 16-bit block complete: {irq_status:#04X}");
    assert_ne!(
        irq_status & 0x02,
        0,
        "Expected 16-bit IRQ pending after DMA block complete, got status {irq_status:#04X}"
    );
}

#[test]
fn sb16_sample_rate_22050hz() {
    let pcm_rate = 22050u32;
    let num_frames = 8192;
    let pcm_data = generate_sine_16bit_signed_stereo(num_frames, TONE_FREQ, pcm_rate as f64);

    let (left, _right) = run_sb16_dma_playback(&pcm_data, pcm_rate, 0xB0, 0x30, num_frames);

    let (peak_freq, peak_mag) =
        find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("22050 Hz rate: peak_freq={peak_freq:.1} Hz, peak_mag={peak_mag:.6}");
    assert!(
        (peak_freq - TONE_FREQ).abs() < 15.0,
        "Expected peak near {TONE_FREQ} Hz at 22050 Hz rate, got {peak_freq:.1} Hz"
    );
    assert!(
        peak_mag > 0.01,
        "Peak magnitude too low at 22050 Hz rate: {peak_mag:.6}"
    );
}

#[test]
fn sb16_sample_rate_11025hz() {
    let pcm_rate = 11025u32;
    let num_frames = 8192;
    let pcm_data = generate_sine_16bit_signed_stereo(num_frames, TONE_FREQ, pcm_rate as f64);

    let (left, _right) = run_sb16_dma_playback(&pcm_data, pcm_rate, 0xB0, 0x30, num_frames);

    let (peak_freq, peak_mag) =
        find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("11025 Hz rate: peak_freq={peak_freq:.1} Hz, peak_mag={peak_mag:.6}");
    assert!(
        (peak_freq - TONE_FREQ).abs() < 15.0,
        "Expected peak near {TONE_FREQ} Hz at 11025 Hz rate, got {peak_freq:.1} Hz"
    );
    assert!(
        peak_mag > 0.01,
        "Peak magnitude too low at 11025 Hz rate: {peak_mag:.6}"
    );
}

#[test]
fn sb16_16bit_stereo_incremental_generate() {
    let pcm_rate = 44100u32;
    let num_frames = 8192;
    let pcm_data = generate_sine_16bit_signed_stereo(num_frames, TONE_FREQ, pcm_rate as f64);

    let mut bus = setup_sb16_bus();
    mixer_write(&mut bus, 0x30, 0xF8);
    mixer_write(&mut bus, 0x31, 0xF8);
    mixer_write(&mut bus, 0x32, 0xF8);
    mixer_write(&mut bus, 0x33, 0xF8);

    dsp_reset(&mut bus);
    dsp_set_sample_rate(&mut bus, pcm_rate as u16);
    dsp_speaker_on(&mut bus);
    write_pcm_to_ram(&mut bus, RAM_BASE, &pcm_data);

    let dma_count = (pcm_data.len() - 1) as u16;
    setup_dma_channel_3(&mut bus, RAM_BASE, dma_count);

    let transfer_length = (num_frames * 2 - 1) as u16; // stereo: 2 channels
    dsp_write(&mut bus, 0xB0); // 16-bit DMA single
    dsp_write(&mut bus, 0x30); // signed stereo
    dsp_write(&mut bus, (transfer_length & 0xFF) as u8);
    dsp_write(&mut bus, (transfer_length >> 8) as u8);

    // Simulate the real app pattern: advance clock in ~5ms steps,
    // calling generate_audio_samples between each step (like SDL audio callback).
    // Use the same step size as the real audio engine (240 frames at 48kHz ≈ 5ms).
    let output_step_frames = 240usize;
    let step_cycles = (output_step_frames as u64) * CPU_CLOCK_HZ as u64 / OUTPUT_SAMPLE_RATE as u64;
    let total_cycles = (num_frames as u64) * CPU_CLOCK_HZ as u64 / pcm_rate as u64;
    let mut all_output: Vec<f32> = Vec::new();
    let mut cycle = bus.current_cycle();
    let mut step_num = 0u32;

    // The DMA event handler reschedules the next DMA event, but
    // set_current_cycle/process_events only pops due events once per call.
    // Advance the clock in small sub-steps to ensure all DMA events fire.
    // Sub-step must be smaller than the DMA batch interval (~14500 cycles at 20MHz)
    // to ensure every DMA event fires.
    let sub_step = 10_000u64;
    while cycle < total_cycles {
        let target = (cycle + step_cycles).min(total_cycles);

        // Advance in sub-steps to process all DMA events.
        while cycle < target {
            cycle = (cycle + sub_step).min(target);
            bus.set_current_cycle(cycle);
        }

        let mut output = vec![0.0f32; output_step_frames * 2];
        bus.generate_audio_samples(1.0, &mut output);

        let nonzero = output.iter().filter(|&&v| v.abs() > 1e-6).count();
        let max_amp = output.iter().map(|v| v.abs()).fold(0.0f32, f32::max);

        if step_num < 10 || step_num.is_multiple_of(10) {
            eprintln!(
                "  step {step_num}: cycle={cycle} nonzero={nonzero}/{} max_amp={max_amp:.6}",
                output.len()
            );
        }

        all_output.extend_from_slice(&output);
        step_num += 1;
    }

    // De-interleave left channel.
    let left: Vec<f32> = all_output.iter().step_by(2).copied().collect();

    eprintln!(
        "Incremental generate: {} total left samples, {} output frames",
        left.len(),
        all_output.len() / 2
    );

    assert!(
        left.len() >= SKIP + FFT_SIZE,
        "Not enough output samples: got {}, need {}",
        left.len(),
        SKIP + FFT_SIZE
    );

    let (peak_freq, peak_mag) =
        find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("Incremental 16-bit stereo: peak_freq={peak_freq:.1} Hz, peak_mag={peak_mag:.6}");
    assert!(
        (peak_freq - TONE_FREQ).abs() < 25.0,
        "Expected peak near {TONE_FREQ} Hz, got {peak_freq:.1} Hz"
    );
    assert!(peak_mag > 0.1, "Peak magnitude too low: {peak_mag:.6}");
}

const SB16_OPL3_ADDR_LO: u16 = 0x20D2;
const SB16_OPL3_DATA_LO: u16 = 0x21D2;

/// Runs the standard OPL3 timer-based detection sequence that drivers use
/// to confirm the chip is present. The test emulates a real CPU polling
/// loop: after starting Timer A, it reads the status port in a tight loop
/// and checks that the timer overflow flag appears within 300 iterations.
///
/// This validates that C-bus I/O wait cycles are applied to the SB16
/// ports. Without them the polling loop on fast machines (66 MHz)
/// outruns the timer and detection fails.
fn opl3_timer_detection(bus: &mut Pc9801Bus<NoTrace>) {
    // Reset timers and clear status.
    bus.io_write_byte(SB16_OPL3_ADDR_LO, 0x04);
    bus.io_write_byte(SB16_OPL3_DATA_LO, 0x60);
    bus.io_write_byte(SB16_OPL3_ADDR_LO, 0x04);
    bus.io_write_byte(SB16_OPL3_DATA_LO, 0x80);

    let status_before = bus.io_read_byte(SB16_OPL3_ADDR_LO);
    assert_eq!(status_before, 0x00, "status must be clear after reset");

    // Set Timer 1 (register 0x02) to 0xFF so it overflows after just
    // 4 OPL periods (1152 input clocks ≈ 80 µs of wall time).
    bus.io_write_byte(SB16_OPL3_ADDR_LO, 0x02);
    bus.io_write_byte(SB16_OPL3_DATA_LO, 0xFF);

    // Start Timer A (bit 0) and unmask its status flag.
    bus.io_write_byte(SB16_OPL3_ADDR_LO, 0x04);
    bus.io_write_byte(SB16_OPL3_DATA_LO, 0x21);

    // Poll the status register, advancing the bus clock after each read
    // to simulate a real IN AL,DX / TEST / JZ loop.  Each iteration the
    // CPU clock advances by the I/O wait penalty plus a few cycles for
    // the surrounding instructions.
    const MAX_POLLS: u32 = 300;
    let mut detected = false;
    for _ in 0..MAX_POLLS {
        let status = bus.io_read_byte(SB16_OPL3_ADDR_LO);
        let wait = bus.drain_wait_cycles().max(0) as u64;
        bus.set_current_cycle(bus.current_cycle() + wait + 8);

        if status & 0xC0 == 0xC0 {
            detected = true;
            break;
        }
    }

    assert!(
        detected,
        "OPL3 Timer A must be detected within {MAX_POLLS} polls"
    );

    // Clean up: mask and reset timers.
    bus.io_write_byte(SB16_OPL3_ADDR_LO, 0x04);
    bus.io_write_byte(SB16_OPL3_DATA_LO, 0x60);
    bus.io_write_byte(SB16_OPL3_ADDR_LO, 0x04);
    bus.io_write_byte(SB16_OPL3_DATA_LO, 0x80);
}

#[test]
fn sb16_opl3_timer_detection_20mhz() {
    let mut bus = setup_sb16_bus(); // PC9801RA, 20 MHz
    opl3_timer_detection(&mut bus);
}

#[test]
fn sb16_opl3_timer_detection_66mhz() {
    let mut bus =
        Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AP, CpuMode::Low, OUTPUT_SAMPLE_RATE);
    bus.install_sound_blaster_16();
    opl3_timer_detection(&mut bus);
}

fn setup_dma_channel_3_write(bus: &mut Pc9801Bus<NoTrace>, address: u32, count: u16) {
    let page = ((address >> 16) & 0xFF) as u8;
    let addr_lo = (address & 0xFF) as u8;
    let addr_hi = ((address >> 8) & 0xFF) as u8;
    let count_lo = (count & 0xFF) as u8;
    let count_hi = ((count >> 8) & 0xFF) as u8;

    bus.io_write_byte(DMA_FLIP_FLOP_CLEAR, 0x00);
    bus.io_write_byte(DMA_CH3_ADDR, addr_lo);
    bus.io_write_byte(DMA_CH3_ADDR, addr_hi);
    bus.io_write_byte(DMA_FLIP_FLOP_CLEAR, 0x00);
    bus.io_write_byte(DMA_CH3_COUNT, count_lo);
    bus.io_write_byte(DMA_CH3_COUNT, count_hi);
    bus.io_write_byte(DMA_CH3_PAGE, page);
    // Mode: single transfer, write (device->memory), no-autoinit, increment, channel 3
    bus.io_write_byte(DMA_MODE, 0x47);
    bus.io_write_byte(DMA_SINGLE_MASK, 0x03);
}

fn dsp_set_input_sample_rate(bus: &mut Pc9801Bus<NoTrace>, rate: u16) {
    dsp_write(bus, 0x42);
    dsp_write(bus, (rate >> 8) as u8);
    dsp_write(bus, (rate & 0xFF) as u8);
}

fn read_ram_byte(bus: &mut Pc9801Bus<NoTrace>, address: u32) -> u8 {
    bus.read_byte(address)
}

#[test]
fn sb16_8bit_recording_dma_writes_silence_to_ram() {
    let mut bus = setup_sb16_bus();
    dsp_reset(&mut bus);

    let block_size: u32 = 512;

    // Fill RAM with non-silence marker.
    for i in 0..block_size {
        bus.write_byte(RAM_BASE + i, 0xAA);
    }

    // Set input sample rate.
    dsp_set_input_sample_rate(&mut bus, 11025);

    // Program DMA ch3 for write mode (device->memory).
    setup_dma_channel_3_write(&mut bus, RAM_BASE, (block_size - 1) as u16);

    // Start 8-bit single-transfer recording: 0xC8 = 8-bit DMA input, no auto-init.
    // mode=0x00 (unsigned mono), length = block_size - 1.
    dsp_write(&mut bus, 0xC8);
    dsp_write(&mut bus, 0x00);
    dsp_write(&mut bus, ((block_size - 1) & 0xFF) as u8);
    dsp_write(&mut bus, (((block_size - 1) >> 8) & 0xFF) as u8);

    // Advance clock past the transfer time.
    let cycles_needed = block_size as u64 * CPU_CLOCK_HZ as u64 / 11025;
    advance_clock_with_events(&mut bus, cycles_needed * 2);

    // Verify RAM was overwritten with 8-bit unsigned silence (0x80).
    let mut silence_count = 0;
    for i in 0..block_size {
        if read_ram_byte(&mut bus, RAM_BASE + i) == 0x80 {
            silence_count += 1;
        }
    }
    assert_eq!(
        silence_count, block_size,
        "Expected all {block_size} bytes to be 0x80 silence, got {silence_count}"
    );
}

#[test]
fn sb16_8bit_recording_dma_fires_irq_after_block() {
    let mut bus = setup_sb16_bus();
    dsp_reset(&mut bus);

    let block_size: u32 = 512;
    let sample_rate: u32 = 11025;

    // Set input sample rate.
    dsp_set_input_sample_rate(&mut bus, sample_rate as u16);

    // Program DMA ch3 for write mode.
    setup_dma_channel_3_write(&mut bus, RAM_BASE, (block_size - 1) as u16);

    // Start 8-bit single-transfer recording.
    dsp_write(&mut bus, 0xC8);
    dsp_write(&mut bus, 0x00);
    dsp_write(&mut bus, ((block_size - 1) & 0xFF) as u8);
    dsp_write(&mut bus, (((block_size - 1) >> 8) & 0xFF) as u8);

    // IRQ should NOT be pending immediately.
    let irq_status_before = mixer_read(&mut bus, 0x82);
    assert_eq!(
        irq_status_before & 0x01,
        0,
        "8-bit IRQ should not be pending immediately after starting recording DMA"
    );

    // Advance clock halfway - still no IRQ.
    let half_cycles = block_size as u64 * CPU_CLOCK_HZ as u64 / sample_rate as u64 / 2;
    advance_clock_with_events(&mut bus, half_cycles);
    let irq_status_half = mixer_read(&mut bus, 0x82);
    assert_eq!(
        irq_status_half & 0x01,
        0,
        "8-bit IRQ should not be pending halfway through the transfer"
    );

    // Advance clock well past the full transfer time.
    let full_cycles = block_size as u64 * CPU_CLOCK_HZ as u64 / sample_rate as u64 * 2;
    advance_clock_with_events(&mut bus, full_cycles);

    // IRQ should now be pending (mixer register 0x82 bit 0 = 8-bit IRQ).
    let irq_status_after = mixer_read(&mut bus, 0x82);
    assert_ne!(
        irq_status_after & 0x01,
        0,
        "8-bit IRQ should be pending after recording DMA block completes"
    );
}

#[test]
fn sb16_16bit_recording_dma_writes_silence_to_ram() {
    let mut bus = setup_sb16_bus();
    dsp_reset(&mut bus);

    // 16-bit signed mono: 256 samples = 512 bytes.
    let sample_count: u32 = 256;
    let byte_count = sample_count * 2;

    // Fill RAM with non-silence marker.
    for i in 0..byte_count {
        bus.write_byte(RAM_BASE + i, 0xAA);
    }

    // Set input sample rate.
    dsp_set_input_sample_rate(&mut bus, 11025);

    // Program DMA ch3 for write mode.
    setup_dma_channel_3_write(&mut bus, RAM_BASE, (byte_count - 1) as u16);

    // Start 16-bit single-transfer recording: 0xB8 = 16-bit DMA input, no auto-init.
    // mode=0x10 (signed mono), length = sample_count - 1.
    dsp_write(&mut bus, 0xB8);
    dsp_write(&mut bus, 0x10);
    dsp_write(&mut bus, ((sample_count - 1) & 0xFF) as u8);
    dsp_write(&mut bus, (((sample_count - 1) >> 8) & 0xFF) as u8);

    // Advance clock past the transfer time.
    let cycles_needed = byte_count as u64 * CPU_CLOCK_HZ as u64 / 11025;
    advance_clock_with_events(&mut bus, cycles_needed * 2);

    // Verify RAM was overwritten with 16-bit signed silence (0x00).
    let mut zero_count = 0;
    for i in 0..byte_count {
        if read_ram_byte(&mut bus, RAM_BASE + i) == 0x00 {
            zero_count += 1;
        }
    }
    assert_eq!(
        zero_count, byte_count,
        "Expected all {byte_count} bytes to be 0x00 (16-bit signed silence), got {zero_count}"
    );
}

#[test]
fn sb16_opl3_bank1_status_read_mirrors_bank0() {
    let mut bus = setup_sb16_bus();
    let bank0 = bus.io_read_byte(SB16_OPL3_BANK0_STATUS);
    let bank1 = bus.io_read_byte(SB16_OPL3_BANK1_STATUS);
    assert_eq!(bank0, bank1);
}

#[test]
fn sb16_opl3_bank1_data_read_returns_open_bus() {
    let mut bus = setup_sb16_bus();
    assert_eq!(bus.io_read_byte(SB16_OPL3_BANK1_DATA), 0xFF);
}

#[test]
fn sb16_opl3_bank1_status_read_without_sb16_returns_open_bus() {
    let mut bus =
        Pc9801Bus::<NoTrace>::new(MachineModel::PC9801RA, CpuMode::High, OUTPUT_SAMPLE_RATE);
    assert_eq!(bus.io_read_byte(SB16_OPL3_BANK1_STATUS), 0xFF);
}

#[test]
fn sb16_opl3_bank0_misaligned_word_probe_reads_open_bus() {
    let mut bus = setup_sb16_bus();
    assert_eq!(bus.io_read_word(SB16_OPL3_BANK0_PROBE_PARTNER), 0xFFFF);
}

#[test]
fn sb16_opl3_bank1_misaligned_word_probe_reads_open_bus() {
    let mut bus = setup_sb16_bus();
    assert_eq!(bus.io_read_word(SB16_OPL3_BANK1_PROBE_PARTNER), 0xFFFF);
}
