use std::f64::consts::PI;

use common::{Bus, CpuMode, MachineModel};
use machine_98::{NoTracing, Pc9801Bus};
use resampler::{Complex32, Forward, Radix, RadixFFT};

const OUTPUT_SAMPLE_RATE: u32 = 48_000;
const PCM_RATE: u32 = 44_100;
const CPU_CLOCK_HZ: u32 = 20_000_000;
const TONE_FREQ: f64 = 1000.0;

fn setup_pcm86_bus() -> Pc9801Bus<NoTracing> {
    let mut bus =
        Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::High, OUTPUT_SAMPLE_RATE);
    bus.install_soundboard_86(None, true);
    bus
}

fn generate_sine_wave_16bit_stereo(num_frames: usize, freq: f64, sample_rate: f64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(num_frames * 4);
    for i in 0..num_frames {
        let t = i as f64 / sample_rate;
        let sample = (2.0 * PI * freq * t).sin();
        let value = (sample * 32767.0) as i16;
        let msb = (value >> 8) as u8;
        let lsb = (value & 0xFF) as u8;
        bytes.push(msb);
        bytes.push(lsb);
        bytes.push(msb);
        bytes.push(lsb);
    }
    bytes
}

fn generate_sine_wave_16bit_mono(num_frames: usize, freq: f64, sample_rate: f64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(num_frames * 2);
    for i in 0..num_frames {
        let t = i as f64 / sample_rate;
        let sample = (2.0 * PI * freq * t).sin();
        let value = (sample * 32767.0) as i16;
        let msb = (value >> 8) as u8;
        let lsb = (value & 0xFF) as u8;
        bytes.push(msb);
        bytes.push(lsb);
    }
    bytes
}

fn generate_sine_wave_8bit_stereo(num_frames: usize, freq: f64, sample_rate: f64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(num_frames * 2);
    for i in 0..num_frames {
        let t = i as f64 / sample_rate;
        let sample = (2.0 * PI * freq * t).sin();
        let value = (sample * 127.0) as i8 as u8;
        bytes.push(value);
        bytes.push(value);
    }
    bytes
}

fn generate_sine_wave_8bit_mono(num_frames: usize, freq: f64, sample_rate: f64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(num_frames);
    for i in 0..num_frames {
        let t = i as f64 / sample_rate;
        let sample = (2.0 * PI * freq * t).sin();
        let value = (sample * 127.0) as i8 as u8;
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

#[test]
fn pcm86_sine_wave_1khz_16bit_stereo() {
    let mut bus = setup_pcm86_bus();

    // Set PCM volume line 5 to max (level 0).
    bus.io_write_byte(0xA466, 0xA0);

    // Reset FIFO.
    bus.io_write_byte(0xA468, 0x08);
    bus.io_write_byte(0xA468, 0x00);

    // Set 16-bit stereo mode (while IRQ disabled).
    bus.io_write_byte(0xA46A, 0x30);

    // Unmute.
    bus.io_write_byte(0xA66E, 0x00);

    // Generate and write sine wave data.
    let num_frames = 8192;
    let pcm_data = generate_sine_wave_16bit_stereo(num_frames, TONE_FREQ, PCM_RATE as f64);
    for &byte in &pcm_data {
        bus.io_write_byte(0xA46C, byte);
    }

    // Start playback at 44100 Hz (rate index 0).
    bus.io_write_byte(0xA468, 0x80);

    // Advance clock to cover all PCM data.
    let cycles_needed = (num_frames as u64) * CPU_CLOCK_HZ as u64 / PCM_RATE as u64;
    bus.set_current_cycle(cycles_needed);

    // Generate audio output.
    let output_frames = 10_000;
    let mut output = vec![0.0f32; output_frames * 2];
    bus.generate_audio_samples(1.0, &mut output);

    let left: Vec<f32> = output.iter().step_by(2).copied().collect();

    // Skip resampler transient, take 4096 samples for FFT.
    let skip = 256;
    let fft_size = 4096;
    assert!(
        left.len() >= skip + fft_size,
        "Not enough output samples: got {}, need {}",
        left.len(),
        skip + fft_size
    );
    let analysis_window = &left[skip..skip + fft_size];

    let (peak_freq, peak_mag) = find_peak_frequency(analysis_window, OUTPUT_SAMPLE_RATE);

    eprintln!("Left channel: peak_freq={peak_freq:.1} Hz, peak_mag={peak_mag:.6}");

    assert!(
        (peak_freq - TONE_FREQ).abs() < 15.0,
        "Expected peak near {TONE_FREQ} Hz, got {peak_freq:.1} Hz"
    );
    assert!(
        peak_mag > 0.01,
        "Peak magnitude too low: {peak_mag:.6}, expected audible signal"
    );

    let right: Vec<f32> = output.iter().skip(1).step_by(2).copied().collect();
    let analysis_right = &right[skip..skip + fft_size];
    let (peak_freq_r, peak_mag_r) = find_peak_frequency(analysis_right, OUTPUT_SAMPLE_RATE);

    eprintln!("Right channel: peak_freq={peak_freq_r:.1} Hz, peak_mag={peak_mag_r:.6}");

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
fn pcm86_8bit_stereo_sine_wave() {
    let mut bus = setup_pcm86_bus();

    bus.io_write_byte(0xA466, 0xA0);
    bus.io_write_byte(0xA468, 0x08);
    bus.io_write_byte(0xA468, 0x00);
    bus.io_write_byte(0xA46A, 0x70); // 8-bit stereo
    bus.io_write_byte(0xA66E, 0x00);

    let num_frames = 8192;
    let pcm_data = generate_sine_wave_8bit_stereo(num_frames, TONE_FREQ, PCM_RATE as f64);
    for &byte in &pcm_data {
        bus.io_write_byte(0xA46C, byte);
    }

    bus.io_write_byte(0xA468, 0x80);
    let cycles_needed = (num_frames as u64) * CPU_CLOCK_HZ as u64 / PCM_RATE as u64;
    bus.set_current_cycle(cycles_needed);

    let output_frames = 10_000;
    let mut output = vec![0.0f32; output_frames * 2];
    bus.generate_audio_samples(1.0, &mut output);

    let left: Vec<f32> = output.iter().step_by(2).copied().collect();
    let skip = 256;
    let fft_size = 4096;
    assert!(
        left.len() >= skip + fft_size,
        "Not enough output samples: got {}, need {}",
        left.len(),
        skip + fft_size
    );
    let analysis_window = &left[skip..skip + fft_size];
    let (peak_freq, peak_mag) = find_peak_frequency(analysis_window, OUTPUT_SAMPLE_RATE);

    eprintln!("8-bit stereo left: peak_freq={peak_freq:.1} Hz, peak_mag={peak_mag:.6}");

    assert!(
        (peak_freq - TONE_FREQ).abs() < 15.0,
        "8-bit stereo: expected peak near {TONE_FREQ} Hz, got {peak_freq:.1} Hz"
    );
    assert!(
        peak_mag > 0.005,
        "8-bit stereo: peak magnitude too low: {peak_mag:.6}"
    );
}

#[test]
fn pcm86_muted_produces_silence() {
    let mut bus = setup_pcm86_bus();

    bus.io_write_byte(0xA466, 0xA0);
    bus.io_write_byte(0xA468, 0x08);
    bus.io_write_byte(0xA468, 0x00);
    bus.io_write_byte(0xA46A, 0x30);
    // Explicitly mute via 0xA66E.
    bus.io_write_byte(0xA66E, 0x01);

    let num_frames = 4096;
    let pcm_data = generate_sine_wave_16bit_stereo(num_frames, TONE_FREQ, PCM_RATE as f64);
    for &byte in &pcm_data {
        bus.io_write_byte(0xA46C, byte);
    }

    bus.io_write_byte(0xA468, 0x80);
    let cycles_needed = (num_frames as u64) * CPU_CLOCK_HZ as u64 / PCM_RATE as u64;
    bus.set_current_cycle(cycles_needed);

    let mut output = vec![0.0f32; 5000 * 2];
    bus.generate_audio_samples(1.0, &mut output);

    let max_abs = output.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    eprintln!("Muted max amplitude: {max_abs:.6}");
    assert!(
        max_abs < 0.01,
        "Expected near-silence when muted, got max amplitude {max_abs:.6}"
    );
}

#[test]
fn pcm86_volume_attenuates_signal() {
    fn run_with_volume(vol_level: u8) -> f64 {
        let mut bus = setup_pcm86_bus();

        bus.io_write_byte(0xA466, 0xA0 | (vol_level & 0x0F));
        bus.io_write_byte(0xA468, 0x08);
        bus.io_write_byte(0xA468, 0x00);
        bus.io_write_byte(0xA46A, 0x30);
        bus.io_write_byte(0xA66E, 0x00);

        let num_frames = 8192;
        let pcm_data = generate_sine_wave_16bit_stereo(num_frames, TONE_FREQ, PCM_RATE as f64);
        for &byte in &pcm_data {
            bus.io_write_byte(0xA46C, byte);
        }

        bus.io_write_byte(0xA468, 0x80);
        let cycles_needed = (num_frames as u64) * CPU_CLOCK_HZ as u64 / PCM_RATE as u64;
        bus.set_current_cycle(cycles_needed);

        let mut output = vec![0.0f32; 10_000 * 2];
        bus.generate_audio_samples(1.0, &mut output);

        let left: Vec<f32> = output.iter().step_by(2).copied().collect();
        let skip = 256;
        let fft_size = 4096;
        let analysis_window = &left[skip..skip + fft_size];
        let (_freq, mag) = find_peak_frequency(analysis_window, OUTPUT_SAMPLE_RATE);
        mag
    }

    let mag_max = run_with_volume(0); // (15-0)/15 = 1.0
    let mag_half = run_with_volume(8); // (15-8)/15 ≈ 0.467
    let mag_low = run_with_volume(14); // (15-14)/15 ≈ 0.067

    eprintln!("Volume 0: {mag_max:.6}, Volume 8: {mag_half:.6}, Volume 14: {mag_low:.6}");

    assert!(
        mag_max > mag_half * 1.5,
        "Volume 0 ({mag_max:.6}) should be significantly louder than volume 8 ({mag_half:.6})"
    );
    assert!(
        mag_half > mag_low * 2.0,
        "Volume 8 ({mag_half:.6}) should be significantly louder than volume 14 ({mag_low:.6})"
    );
}

#[test]
fn pcm86_no_playback_without_play_bit() {
    let mut bus = setup_pcm86_bus();

    bus.io_write_byte(0xA466, 0xA0);
    bus.io_write_byte(0xA468, 0x08);
    bus.io_write_byte(0xA468, 0x00);
    bus.io_write_byte(0xA46A, 0x30);
    bus.io_write_byte(0xA66E, 0x00);

    let num_frames = 4096;
    let pcm_data = generate_sine_wave_16bit_stereo(num_frames, TONE_FREQ, PCM_RATE as f64);
    for &byte in &pcm_data {
        bus.io_write_byte(0xA46C, byte);
    }

    // Do NOT set play bit - leave 0xA468 at 0x00.
    let cycles_needed = (num_frames as u64) * CPU_CLOCK_HZ as u64 / PCM_RATE as u64;
    bus.set_current_cycle(cycles_needed);

    let mut output = vec![0.0f32; 5000 * 2];
    bus.generate_audio_samples(1.0, &mut output);

    let max_abs = output.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    eprintln!("No play bit max amplitude: {max_abs:.6}");
    assert!(
        max_abs < 0.01,
        "Expected near-silence without play bit, got max amplitude {max_abs:.6}"
    );
}

const SKIP: usize = 256;
const FFT_SIZE: usize = 4096;

fn run_pcm86_and_get_channels(
    mode_byte: u8,
    rate_index: u8,
    pcm_data: &[u8],
    num_frames: usize,
    sample_rate: u32,
) -> (Vec<f32>, Vec<f32>) {
    let mut bus = setup_pcm86_bus();

    bus.io_write_byte(0xA466, 0xA0);
    bus.io_write_byte(0xA468, 0x08);
    bus.io_write_byte(0xA468, 0x00);
    bus.io_write_byte(0xA46A, mode_byte);
    bus.io_write_byte(0xA66E, 0x00);

    for &byte in pcm_data {
        bus.io_write_byte(0xA46C, byte);
    }

    bus.io_write_byte(0xA468, 0x80 | (rate_index & 0x07));
    let cycles_needed = (num_frames as u64) * CPU_CLOCK_HZ as u64 / sample_rate as u64;
    bus.set_current_cycle(cycles_needed);

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

#[test]
fn pcm86_16bit_stereo_440hz() {
    let freq = 440.0;
    let num_frames = 8192;
    let pcm_data = generate_sine_wave_16bit_stereo(num_frames, freq, PCM_RATE as f64);
    let (left, right) = run_pcm86_and_get_channels(0x30, 0, &pcm_data, num_frames, PCM_RATE);

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
fn pcm86_16bit_stereo_5khz() {
    let freq = 5000.0;
    let num_frames = 8192;
    let pcm_data = generate_sine_wave_16bit_stereo(num_frames, freq, PCM_RATE as f64);
    let (left, right) = run_pcm86_and_get_channels(0x30, 0, &pcm_data, num_frames, PCM_RATE);

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
    assert!(
        peak_mag_r > 0.01,
        "Right channel: peak magnitude too low: {peak_mag_r:.6}"
    );
}

#[test]
fn pcm86_16bit_left_only() {
    let num_frames = 8192;
    let pcm_data = generate_sine_wave_16bit_mono(num_frames, TONE_FREQ, PCM_RATE as f64);
    let (left, right) = run_pcm86_and_get_channels(0x20, 0, &pcm_data, num_frames, PCM_RATE);

    let (peak_freq, peak_mag) =
        find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("16-bit left-only: peak_freq={peak_freq:.1} Hz, peak_mag={peak_mag:.6}");
    assert!(
        (peak_freq - TONE_FREQ).abs() < 15.0,
        "Left channel: expected peak near {TONE_FREQ} Hz, got {peak_freq:.1} Hz"
    );
    assert!(
        peak_mag > 0.01,
        "Left channel: peak magnitude too low: {peak_mag:.6}"
    );

    let right_max = right[SKIP..SKIP + FFT_SIZE]
        .iter()
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);
    eprintln!("16-bit left-only right channel max: {right_max:.6}");
    assert!(
        right_max < 0.01,
        "Right channel should be silent in left-only mode, got max {right_max:.6}"
    );
}

#[test]
fn pcm86_16bit_right_only() {
    let num_frames = 8192;
    let pcm_data = generate_sine_wave_16bit_mono(num_frames, TONE_FREQ, PCM_RATE as f64);
    let (left, right) = run_pcm86_and_get_channels(0x10, 0, &pcm_data, num_frames, PCM_RATE);

    let (peak_freq_r, peak_mag_r) =
        find_peak_frequency(&right[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("16-bit right-only: peak_freq={peak_freq_r:.1} Hz, peak_mag={peak_mag_r:.6}");
    assert!(
        (peak_freq_r - TONE_FREQ).abs() < 15.0,
        "Right channel: expected peak near {TONE_FREQ} Hz, got {peak_freq_r:.1} Hz"
    );
    assert!(
        peak_mag_r > 0.01,
        "Right channel: peak magnitude too low: {peak_mag_r:.6}"
    );

    let left_max = left[SKIP..SKIP + FFT_SIZE]
        .iter()
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);
    eprintln!("16-bit right-only left channel max: {left_max:.6}");
    assert!(
        left_max < 0.01,
        "Left channel should be silent in right-only mode, got max {left_max:.6}"
    );
}

#[test]
fn pcm86_8bit_left_only() {
    let num_frames = 8192;
    let pcm_data = generate_sine_wave_8bit_mono(num_frames, TONE_FREQ, PCM_RATE as f64);
    let (left, right) = run_pcm86_and_get_channels(0x60, 0, &pcm_data, num_frames, PCM_RATE);

    let (peak_freq, peak_mag) =
        find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("8-bit left-only: peak_freq={peak_freq:.1} Hz, peak_mag={peak_mag:.6}");
    assert!(
        (peak_freq - TONE_FREQ).abs() < 15.0,
        "Left channel: expected peak near {TONE_FREQ} Hz, got {peak_freq:.1} Hz"
    );
    assert!(
        peak_mag > 0.005,
        "Left channel: peak magnitude too low: {peak_mag:.6}"
    );

    let right_max = right[SKIP..SKIP + FFT_SIZE]
        .iter()
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);
    eprintln!("8-bit left-only right channel max: {right_max:.6}");
    assert!(
        right_max < 0.01,
        "Right channel should be silent in left-only mode, got max {right_max:.6}"
    );
}

#[test]
fn pcm86_sample_rate_22050hz() {
    let sample_rate = 22_050u32;
    let num_frames = 8192;
    let pcm_data = generate_sine_wave_16bit_stereo(num_frames, TONE_FREQ, sample_rate as f64);
    let (left, _right) = run_pcm86_and_get_channels(0x30, 2, &pcm_data, num_frames, sample_rate);

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
fn pcm86_sample_rate_16538hz() {
    let sample_rate = 16_538u32;
    let num_frames = 8192;
    let pcm_data = generate_sine_wave_16bit_stereo(num_frames, TONE_FREQ, sample_rate as f64);
    let (left, _right) = run_pcm86_and_get_channels(0x30, 3, &pcm_data, num_frames, sample_rate);

    let (peak_freq, peak_mag) =
        find_peak_frequency(&left[SKIP..SKIP + FFT_SIZE], OUTPUT_SAMPLE_RATE);
    eprintln!("16538 Hz rate: peak_freq={peak_freq:.1} Hz, peak_mag={peak_mag:.6}");
    assert!(
        (peak_freq - TONE_FREQ).abs() < 15.0,
        "Expected peak near {TONE_FREQ} Hz at 16538 Hz rate, got {peak_freq:.1} Hz"
    );
    assert!(
        peak_mag > 0.01,
        "Peak magnitude too low at 16538 Hz rate: {peak_mag:.6}"
    );
}
