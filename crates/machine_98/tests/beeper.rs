//! Integration tests for the PC-98 beeper.
//!
//! Covers the architectural split between the fixed-frequency hardware beeper
//! used by the PC-9801F (per undoc98 `io_tcu.txt` and the PIT-driven beeper used by
//! PC-9801VM and later.

use common::{BeeperKind, Bus, CpuMode, MachineModel, NoTrace};
use machine_98::Pc9801Bus;
use resampler::{Complex32, Forward, Radix, RadixFFT};

const OUTPUT_SAMPLE_RATE: u32 = 48_000;

/// PIT control word for channel 1, mode 3 (square wave), 16-bit access (LSB then MSB).
const PIT_CTRL_CH1_MODE3_LH: u8 = 0x76;
/// Port C BSR command: clear bit 3 (BUZ -> sound on).
const PORT_C_BSR_BUZ_ON: u8 = 0x06;
/// Port C BSR command: set bit 3 (BUZ -> sound off).
const PORT_C_BSR_BUZ_OFF: u8 = 0x07;

const PIT_CONTROL_PORT: u16 = 0x0077;
const PIT_CH1_PORT: u16 = 0x0073;
const PPI_CONTROL_PORT: u16 = 0x0037;

fn setup_bus(model: MachineModel, mode: CpuMode) -> Pc9801Bus<NoTrace> {
    Pc9801Bus::<NoTrace>::new(model, mode, OUTPUT_SAMPLE_RATE)
}

fn write_pit_ch1_reload(bus: &mut Pc9801Bus<NoTrace>, reload: u16) {
    bus.io_write_byte(PIT_CONTROL_PORT, PIT_CTRL_CH1_MODE3_LH);
    bus.io_write_byte(PIT_CH1_PORT, reload as u8);
    bus.io_write_byte(PIT_CH1_PORT, (reload >> 8) as u8);
}

#[test]
fn pc9801f_beeper_kind_is_fixed_2400hz() {
    let bus = setup_bus(MachineModel::PC9801F, CpuMode::High);
    assert_eq!(bus.beeper_kind(), BeeperKind::Fixed { hz: 2400 });
}

#[test]
fn pc9801vm_beeper_kind_is_pit_driven() {
    let bus = setup_bus(MachineModel::PC9801VM, CpuMode::High);
    assert_eq!(bus.beeper_kind(), BeeperKind::PitDriven);
}

#[test]
fn pc9801f_beeper_ignores_pit_ch1_writes() {
    let mut bus = setup_bus(MachineModel::PC9801F, CpuMode::High);
    let initial_reload = bus.beeper_state().pit_reload;
    let expected_reload = (bus.pit_clock_hz() / 2400) as u16;
    assert_eq!(initial_reload, expected_reload);

    // Write a sequence of distinct reloads that would change the audible pitch
    // on a PIT-driven machine. PC-9801F must ignore them because PIT ch1 is
    // its memory-refresh generator.
    for reload in [0x0040u16, 0x0200, 0x1000, 0x4000] {
        write_pit_ch1_reload(&mut bus, reload);
        bus.set_current_cycle(bus.current_cycle() + 256);
        assert_eq!(
            bus.beeper_state().pit_reload,
            expected_reload,
            "PC-9801F beeper pit_reload changed after PIT ch1 write of {reload:#06X}",
        );
    }
}

#[test]
fn pc9801vm_beeper_follows_pit_ch1() {
    let mut bus = setup_bus(MachineModel::PC9801VM, CpuMode::High);

    for reload in [0x0040u16, 0x0200, 0x1000, 0x4000] {
        write_pit_ch1_reload(&mut bus, reload);
        bus.set_current_cycle(bus.current_cycle() + 256);
        assert_eq!(
            bus.beeper_state().pit_reload,
            reload,
            "PC-9801VM beeper pit_reload should follow PIT ch1 write of {reload:#06X}",
        );
    }
}

#[test]
fn pc9801f_beeper_buzzer_gating_via_port_c() {
    let mut bus = setup_bus(MachineModel::PC9801F, CpuMode::High);

    // Boot state: BUZ bit (port C bit 3) is set => buzzer disabled.
    assert!(!bus.beeper_state().buzzer_enabled);

    // Clear bit 3 via PPI BSR command -> sound on.
    bus.io_write_byte(PPI_CONTROL_PORT, PORT_C_BSR_BUZ_ON);
    assert!(bus.beeper_state().buzzer_enabled);

    // Set bit 3 again -> sound off.
    bus.io_write_byte(PPI_CONTROL_PORT, PORT_C_BSR_BUZ_OFF);
    assert!(!bus.beeper_state().buzzer_enabled);
}

/// Drives `bus` for ~one FFT window of audio at the configured sample rate
/// and returns one channel of the generated samples.
fn capture_beeper_samples(
    bus: &mut Pc9801Bus<NoTrace>,
    fft_size: usize,
    sample_rate: u32,
) -> Vec<f32> {
    let cycles_per_sample = u64::from(bus.cpu_clock_hz()) / u64::from(sample_rate);
    let mut left = Vec::with_capacity(fft_size);
    let chunk_frames = 256;
    let mut stereo = vec![0.0f32; chunk_frames * 2];

    while left.len() < fft_size {
        let target_cycle = bus.current_cycle() + cycles_per_sample * chunk_frames as u64;
        bus.set_current_cycle(target_cycle);
        for sample in stereo.iter_mut() {
            *sample = 0.0;
        }
        let written = bus.generate_audio_samples(1.0, &mut stereo);
        let frames = written / 2;
        for i in 0..frames {
            if left.len() >= fft_size {
                break;
            }
            left.push(stereo[i * 2]);
        }
    }
    left.truncate(fft_size);
    left
}

fn dominant_frequency(samples: &[f32], sample_rate: u32) -> f64 {
    let n = samples.len();
    assert!(n.is_power_of_two(), "FFT size must be a power of two");
    let factors = vec![Radix::Factor2; n.trailing_zeros() as usize];
    let fft = RadixFFT::<Forward>::new(factors);
    let mut scratchpad = vec![Complex32::default(); fft.scratchpad_size()];
    let mut spectrum = vec![Complex32::default(); n / 2 + 1];
    fft.process(samples, &mut spectrum, &mut scratchpad);

    let mut max_magnitude = 0.0f64;
    let mut max_bin = 1usize;
    for (i, c) in spectrum.iter().enumerate().skip(1) {
        let magnitude = ((c.re as f64) * (c.re as f64) + (c.im as f64) * (c.im as f64)).sqrt();
        if magnitude > max_magnitude {
            max_magnitude = magnitude;
            max_bin = i;
        }
    }
    max_bin as f64 * sample_rate as f64 / n as f64
}

fn assert_pc9801f_dominant_frequency_is_2400hz(mode: CpuMode) {
    const FFT_SIZE: usize = 4096;
    let mut bus = setup_bus(MachineModel::PC9801F, mode);

    // Skip past the initial frame containing the boot-time silence and any
    // transient at the moment the buzzer is enabled.
    let _ = capture_beeper_samples(&mut bus, FFT_SIZE, OUTPUT_SAMPLE_RATE);
    bus.io_write_byte(PPI_CONTROL_PORT, PORT_C_BSR_BUZ_ON);
    assert!(bus.beeper_state().buzzer_enabled);
    let _ = capture_beeper_samples(&mut bus, FFT_SIZE, OUTPUT_SAMPLE_RATE);

    // Capture a stable window for FFT analysis.
    let samples = capture_beeper_samples(&mut bus, FFT_SIZE, OUTPUT_SAMPLE_RATE);
    let peak_hz = dominant_frequency(&samples, OUTPUT_SAMPLE_RATE);

    let bin_width = OUTPUT_SAMPLE_RATE as f64 / FFT_SIZE as f64;
    let tolerance_hz = (bin_width * 2.0).max(50.0);
    assert!(
        (peak_hz - 2400.0).abs() <= tolerance_hz,
        "PC-9801F peak frequency at cpu-mode={mode:?} was {peak_hz:.1} Hz, \
         expected 2400 Hz +/- {tolerance_hz:.1} Hz",
    );
}

#[test]
fn pc9801f_beeper_fixed_frequency_spectrum_high() {
    assert_pc9801f_dominant_frequency_is_2400hz(CpuMode::High);
}

#[test]
fn pc9801f_beeper_fixed_frequency_spectrum_low() {
    assert_pc9801f_dominant_frequency_is_2400hz(CpuMode::Low);
}
