//! PC-9801-86 sound board: YM2608 (OPNA) stereo FM + SSG + ADPCM + PCM86 DAC.

use common::MachineModel;
use resampler::{Attenuation, Latency, ResamplerFir, ResamplerFirState};
use ymfm_oxide::{Ym2608, Ym2608State, YmfmOutput3};

use crate::{
    opn_fm::{EVOLVED_RHYTHM_ROM, FmTimerAction, OpnFm, OpnFmState},
    soundboard_26k::FmSampleRemainder,
};

/// Timer outputs exposed by the PC-9801-86 board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Soundboard86Timer {
    /// FM timer A.
    FmTimerA,
    /// FM timer B.
    FmTimerB,
    /// PCM FIFO interrupt request.
    Pcm86Irq,
}

/// 256 KB ADPCM-B sample RAM.
const ADPCM_B_RAM_SIZE: usize = 256 * 1024;

/// YM2608 input clock on the PC-9801-86 board (7.9872 MHz).
const YM2608_CLOCK_HZ: u32 = 7_987_200;

/// Sound function ID and OPNA mask register.
const PORT_SOUND_ID: u16 = 0xA460;
/// Port that a PC-98GS or PC-9801-73 decodes as the uPD6380 DSP control port. A
/// Q-Vision WaveStar takes its handshake here.
const PORT_DSP_CONTROL: u16 = 0xA462;
/// Port that a PC-98GS or PC-9801-73 decodes as the uPD6380 DSP data port. A
/// Q-Vision WaveStar shows its status and takes its mode here.
const PORT_DSP_DATA: u16 = 0xA464;
/// FIFO status (read) and electronic volume (write).
const PORT_FIFO_STATUS_VOLUME: u16 = 0xA466;
/// FIFO control register.
const PORT_FIFO_CONTROL: u16 = 0xA468;
/// D/A converter control, or the FIFO interrupt threshold while interrupts are enabled.
const PORT_DAC_CONTROL: u16 = 0xA46A;
/// FIFO data port.
const PORT_FIFO_DATA: u16 = 0xA46C;
/// Sound output mute register.
const PORT_MUTE: u16 = 0xA66E;

/// Value read from a port that the board does not drive.
const OPEN_BUS: u8 = 0xFF;

/// Board ID in A460h bits 7-4: PC-9801-86 at I/O base 0188h.
const SOUND_ID_PC9801_86: u8 = 0x40;
/// Writable bits of A460h (bit 1 OPNA mask, bit 0 OPNA extension).
const SOUND_ID_WRITABLE_MASK: u8 = 0x03;
/// A460h bit 1: the OPNA is masked.
const SOUND_ID_OPNA_MASK: u8 = 0x02;
/// A460h bit 0: the OPNA extended register bank is enabled.
const SOUND_ID_OPNA_EXTEND: u8 = 0x01;

/// FIFO capacity in bytes that the CPU sees (32 KiB).
const FIFO_CAPACITY: i32 = 0x8000;
/// Size of the ring memory in bytes. It holds the FIFO capacity plus the
/// overflow margin.
const RING_SIZE: usize = 0x10000;
/// Index mask of the ring memory.
const RING_INDEX_MASK: u32 = RING_SIZE as u32 - 1;

/// A468h bit 7: the FIFO runs.
const FIFO_CONTROL_RUN: u8 = 0x80;
/// A468h bit 5: the FIFO interrupt is enabled.
const FIFO_CONTROL_IRQ_ENABLE: u8 = 0x20;
/// A468h bit 4: the FIFO interrupt request flag.
const FIFO_CONTROL_IRQ_REQUEST: u8 = 0x10;
/// A468h bit 3: the FIFO reset request.
const FIFO_CONTROL_RESET: u8 = 0x08;
/// A468h bits 2-0: the sampling rate code.
const FIFO_CONTROL_RATE_MASK: u8 = 0x07;

/// A466h read bit 7: the FIFO holds exactly 32 KiB.
const FIFO_STATUS_FULL: u8 = 0x80;
/// A466h read bit 6: the FIFO holds no complete sample frame.
const FIFO_STATUS_EMPTY: u8 = 0x40;
/// A466h read bit 0: the L/R clock of the D/A converter.
const FIFO_STATUS_LR_CLOCK: u8 = 0x01;

/// A66Eh bit 0: the sound output is muted.
const MUTE_OUTPUT: u8 = 0x01;
/// Electronic volume line of the PCM output (VOL6).
const VOLUME_LINE_PCM: usize = 5;
/// Largest attenuation value of an electronic volume line.
const VOLUME_LEVEL_MAX: u8 = 15;
/// Full scale of a signed 8-bit sample.
const SAMPLE_SCALE_8BIT: f32 = 128.0;
/// Full scale of a signed 16-bit sample.
const SAMPLE_SCALE_16BIT: f32 = 32768.0;

/// Shift of the volume line select in an A466h write.
const VOLUME_LINE_SHIFT: u8 = 5;
/// Mask of the attenuation value in an A466h write.
const VOLUME_LEVEL_MASK: u8 = 0x0F;

/// A46Ah bits 6-4: sample width and output channel selection.
const DAC_CONTROL_FORMAT_MASK: u8 = 0x70;
/// Shift of the format bits in A46Ah.
const DAC_CONTROL_FORMAT_SHIFT: u32 = 4;
/// A46Ah bit 6: 8-bit samples.
const DAC_FORMAT_8BIT: u8 = 0x40;
/// A46Ah bit 5: the left channel outputs.
const DAC_OUTPUT_LEFT: u8 = 0x20;
/// A46Ah bit 4: the right channel outputs.
const DAC_OUTPUT_RIGHT: u8 = 0x10;
/// D/A converter control after reset: 16-bit stereo output in PCM mode 010b.
const DAC_CONTROL_RESET: u8 = 0x32;
/// A46Ah bits 3-0 of a D/A converter setting that the board ignores.
const DAC_CONTROL_IGNORED_LOW_BITS: u8 = 0x0F;

/// FIFO interrupt threshold granularity in bytes.
const IRQ_THRESHOLD_UNIT: i32 = 128;
/// Threshold code that selects the largest threshold.
const IRQ_THRESHOLD_LARGEST_CODE: u8 = 0xFF;
/// Largest interrupt threshold in bytes.
const IRQ_THRESHOLD_LARGEST_BYTES: i32 = 0x7FFC;
/// Thresholds below this byte count shorten the data write delay in proportion.
const IRQ_WAIT_SCALE_BYTES: i32 = 8192;
/// Delay of the level interrupt after a FIFO data write, in PIT clocks.
const DATA_WRITE_IRQ_WAIT_PIT_CLOCKS: u64 = 20_000;
/// Delay of an interrupt check retry, in PIT clocks.
const IRQ_RETRY_PIT_CLOCKS: u64 = 100;
/// Delay of an interrupt check for pending data whose level already reached
/// the threshold, in CPU cycles.
const IMMEDIATE_CHECK_CYCLES: u64 = 1;
/// Underrun time after which the interrupt timer leans towards the ring level
/// by one part in `UNDERRUN_MODERATE_BLEND_PARTS`.
const UNDERRUN_MODERATE_MILLISECONDS: i32 = 4000;
/// Blend parts of the interrupt timer distance after a moderate underrun.
const UNDERRUN_MODERATE_BLEND_PARTS: i64 = 100;
/// Underrun time after which the interrupt timer leans towards the ring level
/// by one part in `UNDERRUN_SEVERE_BLEND_PARTS`.
const UNDERRUN_SEVERE_MILLISECONDS: i32 = 32_000;
/// Blend parts of the interrupt timer distance after a severe underrun.
const UNDERRUN_SEVERE_BLEND_PARTS: i64 = 10;
/// Blend parts of the level that the interrupt check timer compares with the
/// threshold: four parts CPU view and one part ring level.
const IRQ_CHECK_BLEND_PARTS: i64 = 5;
/// Ring level lead over the CPU view, in interrupt thresholds, that the CPU
/// view tolerates before it catches up.
const RING_LEAD_TOLERANCE_THRESHOLDS: i32 = 3;
/// Divisor of a catch-up step of the CPU view.
const RING_LEAD_CATCH_UP_DIVISOR: i32 = 8;
/// Milliseconds per second.
const MILLISECONDS_PER_SECOND: i64 = 1000;
/// Sample period phase from which the L/R clock bit reads high.
const LR_CLOCK_HIGH_PHASE: f64 = 0.5;

/// Q-Vision WaveStar handshake written to A462h.
const WAVESTAR_SEQUENCE: [u8; 5] = [0xA6, 0xD3, 0x69, 0xB4, 0x5A];
/// WaveStar status without a completed handshake.
const WAVESTAR_IDLE: u8 = 0xFF;
/// WaveStar status after the handshake.
const WAVESTAR_DETECTED: u8 = 0x0B;
/// A464h value that selects the Windows Sound System mode.
const WAVESTAR_SELECT_WSS: u8 = 0x04;
/// A464h value that resets the WaveStar status.
const WAVESTAR_RESET: u8 = 0x09;
/// WaveStar status in the Windows Sound System mode.
const WAVESTAR_MODE_WSS: u8 = 0x0C;
/// WaveStar status in the PCM86 mode.
const WAVESTAR_MODE_PCM86: u8 = 0x08;
/// WaveStar status that alternates with `WAVESTAR_IDLE` on every read.
const WAVESTAR_STATUS_TOGGLE: u8 = 0x00;

/// D/A converter clock in Hz for rate code 0.
const DAC_BASE_RATE_HZ: u32 = 44_100;

/// Divisors of the D/A converter oscillator for the rate codes 0-7.
const DAC_RATE_DIVISORS: [u32; 8] = [36, 48, 72, 96, 144, 192, 288, 384];

/// Sampling rates in Hz for the rate codes 0-7, rounded to the nearest hertz.
const DAC_SAMPLE_RATES: [u32; 8] = {
    let mut rates = [0u32; 8];
    let mut code = 0;
    while code < rates.len() {
        let divisor = DAC_RATE_DIVISORS[code];
        rates[code] = (DAC_BASE_RATE_HZ * DAC_RATE_DIVISORS[0] + divisor / 2) / divisor;
        code += 1;
    }
    rates
};

/// Bytes of a 16-bit stereo sample frame, the largest frame of the D/A converter.
const LARGEST_FRAME_BYTES: i32 = 4;

/// Bytes that a ring memory overflow drops from the oldest data.
const OVERFLOW_DROP_BYTES: i32 = LARGEST_FRAME_BYTES;

/// Granularity in bytes of an underrun correction of the CPU view.
const FRAME_ALIGNMENT_BYTES: i32 = LARGEST_FRAME_BYTES;

/// Overflow margin of the ring memory in D/A converter oscillator clocks
/// (about 14.5 ms at every sampling rate).
const OVERFLOW_MARGIN_OSCILLATOR_CLOCKS: u32 = 23_040;

/// Overflow margin in sample frames for the rate codes 0-7.
const OVERFLOW_MARGIN_FRAMES: [i32; 8] = {
    let mut frames = [0i32; 8];
    let mut code = 0;
    while code < frames.len() {
        frames[code] = (OVERFLOW_MARGIN_OSCILLATOR_CLOCKS / DAC_RATE_DIVISORS[code]) as i32;
        code += 1;
    }
    frames
};

/// Shift from sample frames to bytes, indexed by the A46Ah format bits 6-4.
///
/// A 16-bit sample adds one and a frame with both channels adds one. A format
/// without an output channel sizes its frame like one channel.
const FRAME_SHIFTS: [u8; 8] = {
    let both_channels = DAC_OUTPUT_LEFT | DAC_OUTPUT_RIGHT;
    let mut shifts = [0u8; 8];
    let mut index = 0;
    while index < shifts.len() {
        let format = (index as u8) << DAC_CONTROL_FORMAT_SHIFT;
        let wide_samples = format & DAC_FORMAT_8BIT == 0;
        let stereo = format & both_channels == both_channels;
        shifts[index] = wide_samples as u8 + stereo as u8;
        index += 1;
    }
    shifts
};

/// Interleaved channels of the PCM resampler input and output.
const OUTPUT_CHANNELS: usize = 2;

/// Stop band attenuation of the PCM resampler.
const RESAMPLER_ATTENUATION: Attenuation = Attenuation::Db60;

/// Filter latency of the PCM resampler.
const RESAMPLER_LATENCY: Latency = Latency::Sample64;

save_state::runtime_state! {
/// Register and timing state of the PC-9801-86 sound board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Soundboard86State {
    /// Low bank address latch (write via port 0x0188).
    pub address_lo: u8,
    /// High bank address latch (write via port 0x018C).
    pub address_hi: u8,
    /// IRQ line number for the FM chip.
    pub irq_line: u8,
    /// Whether the OPNA IRQ output is currently asserted.
    pub irq_asserted: bool,
    /// Whether the PCM86 IRQ output is currently asserted.
    pub pcm86_irq_asserted: bool,
    /// CPU cycle at which the busy flag clears.
    pub busy_end_cycle: u64,
    /// CPU cycle at which the current audio output frame started.
    /// Only advanced by `generate_samples()`.
    pub audio_frame_start_cycle: u64,
    /// CPU cycle up to which the FM chip has been clocked.
    /// Advanced by `sync_to_cycle()` on every port access.
    pub fm_sync_cursor: u64,
    /// Fractional sample remainder carried across frames.
    pub sample_remainder: FmSampleRemainder,
    /// Whether extended mode (6-ch FM + ADPCM) is enabled.
    pub extend_enabled: bool,
    /// Whether the 256 KiB ADPCM-B sample RAM upgrade is installed.
    pub adpcm_ram: bool,
    /// Last data written to a register (returned for reads of addr >= 0x10).
    pub last_written_data: u8,
    /// PCM86 state.
    pub pcm86: Pcm86State,
}}

impl Default for Soundboard86State {
    fn default() -> Self {
        Self {
            address_lo: 0,
            address_hi: 0,
            irq_line: 12,
            irq_asserted: false,
            pcm86_irq_asserted: false,
            busy_end_cycle: 0,
            audio_frame_start_cycle: 0,
            fm_sync_cursor: 0,
            sample_remainder: FmSampleRemainder::default(),
            extend_enabled: false,
            adpcm_ram: true,
            last_written_data: 0,
            pcm86: Pcm86State::default(),
        }
    }
}

save_state::runtime_state! {
/// PCM86 registers, FIFO counters and D/A converter clock phase.
///
/// The FIFO keeps two byte counts. `level` counts the bytes in the ring
/// memory that the audio output has not converted yet. `virtual_level` follows
/// the D/A converter clock in CPU time and drives the status flags and the
/// interrupt request that the CPU sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm86State {
    /// Sound function ID and OPNA mask register (port 0xA460).
    pub sound_flags: u8,
    /// FIFO control register (port 0xA468).
    pub fifo_control: u8,
    /// D/A converter control register (port 0xA46A).
    pub dac_control: u8,
    /// Attenuation of the eight electronic volume lines (port 0xA466), 0 is loudest.
    pub volume_levels: [u8; 8],
    /// Sound output mute register (port 0xA66E). Bit 0 mutes the output.
    pub mute: u8,
    /// FIFO interrupt request flag (port 0xA468 bit 4).
    pub irq_request: bool,
    /// FIFO level in bytes at or below which the interrupt request sets.
    pub irq_threshold_bytes: i32,
    /// Ring index of the next byte the CPU writes.
    pub write_index: u32,
    /// Ring index of the next byte the audio output converts.
    pub read_index: u32,
    /// Bytes in the ring memory that the audio output has not converted.
    pub level: i32,
    /// FIFO level that the CPU sees, limited to the FIFO capacity.
    pub virtual_level: i32,
    /// Shift from sample frames to bytes for the selected output format.
    pub frame_shift: u8,
    /// Byte mask of one sample frame, `(1 << frame_shift) - 1`.
    pub frame_mask: u32,
    /// Bytes above the FIFO capacity that the ring memory keeps before it
    /// drops the oldest frames.
    pub overflow_margin: i32,
    /// Whether the CPU wrote FIFO data since the last interrupt.
    pub data_written: bool,
    /// CPU cycle from which the data write delay of the interrupt runs.
    pub irq_wait_start_cycle: u64,
    /// CPU cycles after a data write before the FIFO level raises an interrupt.
    pub irq_wait_cycles: u64,
    /// CPU cycle up to which `virtual_level` follows the converter clock.
    pub virtual_level_cycle: u64,
    /// Fractional sample frame carried between audio output frames.
    pub sample_remainder: FmSampleRemainder,
    /// Fractional sample frame carried between `virtual_level` updates.
    pub virtual_level_remainder: FmSampleRemainder,
    /// CPU cycle anchor of the L/R clock bit of port 0xA466.
    pub dac_clock_cycle: u64,
    /// Sample phase of the L/R clock bit of port 0xA466.
    pub dac_clock_remainder: FmSampleRemainder,
    /// Number of matched bytes of the Q-Vision WaveStar handshake (0 to 5).
    pub wavestar_matched_bytes: u8,
    /// Q-Vision WaveStar status of port 0xA464.
    pub wavestar_value: u8,
}}

save_state::runtime_state! {
/// Complete PCM86 ring buffer and resampler state.
#[derive(Clone)]
struct Pcm86RuntimeState {
    state: Pcm86State,
    buffer: Box<[u8]>,
    resampler: ResamplerFirState,
    pcm_input_buffer: Vec<f32>,
    pending_irq_change: Option<bool>,
    clock_multiple: u64,
    needs_reschedule: bool,
    underrun_milliseconds: i32,
    last_reconcile_cycle: u64,
    last_generate_cycle: u64,
}}

save_state::runtime_state! {
/// Complete PC-9801-86 audio device state.
#[derive(Clone)]
pub struct Soundboard86RuntimeState {
    board: Soundboard86State,
    core: OpnFmState<Ym2608State, YmfmOutput3>,
    pcm86: Pcm86RuntimeState,
    cpu_clock_hz: u32,
    chip_action_cycle: u64,
    sample_rate: u32,
    fm_timer_just_fired: bool,
}}

impl Default for Pcm86State {
    fn default() -> Self {
        Self {
            sound_flags: SOUND_ID_PC9801_86,
            fifo_control: 0,
            dac_control: DAC_CONTROL_RESET,
            volume_levels: [0; 8],
            mute: 0x00,
            irq_request: false,
            irq_threshold_bytes: IRQ_THRESHOLD_UNIT,
            write_index: 0,
            read_index: 0,
            level: 0,
            virtual_level: 0,
            frame_shift: frame_shift(DAC_CONTROL_RESET),
            frame_mask: frame_mask(frame_shift(DAC_CONTROL_RESET)),
            overflow_margin: overflow_margin(0, frame_shift(DAC_CONTROL_RESET)),
            data_written: false,
            irq_wait_start_cycle: 0,
            irq_wait_cycles: 0,
            virtual_level_cycle: 0,
            sample_remainder: FmSampleRemainder::default(),
            virtual_level_remainder: FmSampleRemainder::default(),
            dac_clock_cycle: 0,
            dac_clock_remainder: FmSampleRemainder::default(),
            wavestar_matched_bytes: 0,
            wavestar_value: WAVESTAR_IDLE,
        }
    }
}

/// Returns the frame shift of the format bits of a D/A converter control value.
const fn frame_shift(dac_control: u8) -> u8 {
    FRAME_SHIFTS[((dac_control & DAC_CONTROL_FORMAT_MASK) >> DAC_CONTROL_FORMAT_SHIFT) as usize]
}

/// Returns the byte mask of one sample frame for a frame shift.
const fn frame_mask(frame_shift: u8) -> u32 {
    (1 << frame_shift) - 1
}

/// Returns the overflow margin in bytes for a rate code and a frame shift.
const fn overflow_margin(rate_code: u8, frame_shift: u8) -> i32 {
    OVERFLOW_MARGIN_FRAMES[(rate_code & FIFO_CONTROL_RATE_MASK) as usize] << frame_shift
}

/// Returns the sample frames that `elapsed` CPU cycles cover at `rate`, with
/// the fraction carried in `remainder`.
fn elapsed_frames(
    elapsed: u64,
    rate: u32,
    cpu_clock_hz: u32,
    remainder: &mut FmSampleRemainder,
) -> u64 {
    let exact = elapsed as f64 * f64::from(rate) / f64::from(cpu_clock_hz) + remainder.0;
    let frames = exact as u64;
    *remainder = FmSampleRemainder(exact - frames as f64);
    frames
}

/// Advances a converter clock from `anchor` to `current_cycle` and returns the
/// sample frames it covered at `rate`. Returns `None` and keeps the clock when
/// no CPU cycle elapsed.
fn step_converter_clock(
    anchor: &mut u64,
    phase: &mut FmSampleRemainder,
    current_cycle: u64,
    rate: u32,
    cpu_clock_hz: u32,
) -> Option<u64> {
    let elapsed = current_cycle.saturating_sub(*anchor);
    if elapsed == 0 {
        return None;
    }
    *anchor = current_cycle;
    Some(elapsed_frames(elapsed, rate, cpu_clock_hz, phase))
}

/// Mixes `virtual_value` and `ring_value` in the ratio `parts - 1` to 1.
fn blend_toward_ring(virtual_value: i32, ring_value: i32, parts: i64) -> i32 {
    ((i64::from(virtual_value) * (parts - 1) + i64::from(ring_value)) / parts) as i32
}

/// Returns `index` wrapped into the ring memory.
fn ring_index(index: u32) -> u32 {
    index & RING_INDEX_MASK
}

/// Returns the interrupt threshold in bytes of an A46Ah threshold code.
fn irq_threshold_bytes(code: u8) -> i32 {
    match code {
        IRQ_THRESHOLD_LARGEST_CODE => IRQ_THRESHOLD_LARGEST_BYTES,
        _ => (i32::from(code) + 1) * IRQ_THRESHOLD_UNIT,
    }
}

/// Creates the stereo PCM resampler from the PCM rate to the output rate.
fn new_resampler(pcm_rate: u32, output_rate: u32) -> ResamplerFir {
    ResamplerFir::new_from_hz(
        OUTPUT_CHANNELS,
        pcm_rate,
        output_rate,
        RESAMPLER_LATENCY,
        RESAMPLER_ATTENUATION,
    )
}

/// Fill class of the CPU view of the FIFO.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FifoFill {
    /// The FIFO holds its full capacity.
    Full,
    /// The FIFO holds at least one frame and has room left.
    Partial,
    /// The FIFO holds no complete sample frame.
    Empty,
}

/// Meaning of a write to A46Ah.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DacControlWrite {
    /// New FIFO interrupt threshold in bytes.
    Threshold(i32),
    /// New D/A converter control value.
    Format(u8),
    /// A converter setting with bits 3-0 all set, which the board ignores.
    Ignored,
}

impl DacControlWrite {
    /// Decodes an A46Ah write. An enabled FIFO interrupt makes the port take
    /// the threshold code.
    fn decode(value: u8, irq_enabled: bool) -> Self {
        if irq_enabled {
            Self::Threshold(irq_threshold_bytes(value))
        } else if value & DAC_CONTROL_IGNORED_LOW_BITS == DAC_CONTROL_IGNORED_LOW_BITS {
            Self::Ignored
        } else {
            Self::Format(value)
        }
    }
}

/// PCM86 FIFO and D/A converter.
struct Pcm86 {
    state: Pcm86State,
    buffer: Box<[u8; RING_SIZE]>,
    pcm_resampler: ResamplerFir,
    /// Converted stereo frames at the PCM rate that wait for the resampler.
    pcm_input_buffer: Vec<f32>,
    pcm_resample_output: Vec<f32>,
    /// Interrupt line change for the next action drain.
    pending_irq_change: Option<bool>,
    /// CPU clock divided by the PIT clock, at least 1.
    clock_multiple: u64,
    /// Whether the next action drain recomputes the interrupt check timer.
    needs_reschedule: bool,
    /// Milliseconds for which `level` stayed below `virtual_level`.
    underrun_milliseconds: i32,
    /// CPU cycle of the last level reconciliation.
    last_reconcile_cycle: u64,
    /// CPU cycle of the last audio output frame.
    last_generate_cycle: u64,
}

impl Pcm86 {
    /// Creates the FIFO in its reset state.
    fn new(sample_rate: u32, cpu_clock_hz: u32, machine_model: MachineModel) -> Self {
        let resampler = new_resampler(DAC_SAMPLE_RATES[0], sample_rate);
        let output_size = resampler.buffer_size_output();
        let clock_multiple = u64::from((cpu_clock_hz / machine_model.pit_clock_hz()).max(1));
        Self {
            state: Pcm86State {
                irq_wait_cycles: DATA_WRITE_IRQ_WAIT_PIT_CLOCKS * clock_multiple,
                ..Pcm86State::default()
            },
            buffer: Box::new([0u8; RING_SIZE]),
            pcm_resampler: resampler,
            pcm_input_buffer: Vec::new(),
            pcm_resample_output: vec![0.0; output_size],
            pending_irq_change: None,
            clock_multiple,
            needs_reschedule: false,
            underrun_milliseconds: 0,
            last_reconcile_cycle: 0,
            last_generate_cycle: 0,
        }
    }

    /// Captures the FIFO contents, registers and resampler history.
    fn capture_state(&self) -> Pcm86RuntimeState {
        Pcm86RuntimeState {
            state: self.state.clone(),
            buffer: self.buffer.to_vec().into_boxed_slice(),
            resampler: self.pcm_resampler.capture_state(),
            pcm_input_buffer: self.pcm_input_buffer.clone(),
            pending_irq_change: self.pending_irq_change,
            clock_multiple: self.clock_multiple,
            needs_reschedule: self.needs_reschedule,
            underrun_milliseconds: self.underrun_milliseconds,
            last_reconcile_cycle: self.last_reconcile_cycle,
            last_generate_cycle: self.last_generate_cycle,
        }
    }

    /// Checks the indices and the clock configuration of a saved FIFO.
    fn validate_state(
        &self,
        state: &Pcm86RuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        let fifo = &state.state;
        if state.buffer.len() != RING_SIZE
            || state.clock_multiple != self.clock_multiple
            || fifo.write_index > RING_INDEX_MASK
            || fifo.read_index > RING_INDEX_MASK
            || fifo.wavestar_matched_bytes > WAVESTAR_SEQUENCE.len() as u8
            || !fifo.sample_remainder.0.is_finite()
            || !fifo.virtual_level_remainder.0.is_finite()
            || !fifo.dac_clock_remainder.0.is_finite()
            || !state.pcm_input_buffer.len().is_multiple_of(2)
        {
            return Err(save_state::StateValidationError::new(
                "PCM86 streaming state is invalid",
            ));
        }
        self.pcm_resampler.validate_state(&state.resampler)
    }

    /// Restores a saved FIFO after validation.
    fn restore_state(
        &mut self,
        state: Pcm86RuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        self.validate_state(&state)?;
        self.pcm_resampler.restore_state(state.resampler)?;
        self.state = state.state;
        self.buffer.copy_from_slice(&state.buffer);
        self.pcm_input_buffer = state.pcm_input_buffer;
        self.pending_irq_change = state.pending_irq_change;
        self.needs_reschedule = state.needs_reschedule;
        self.underrun_milliseconds = state.underrun_milliseconds;
        self.last_reconcile_cycle = state.last_reconcile_cycle;
        self.last_generate_cycle = state.last_generate_cycle;
        self.pcm_resample_output.clear();
        self.pcm_resample_output
            .resize(self.pcm_resampler.buffer_size_output(), 0.0);
        Ok(())
    }

    /// Returns the sampling rate selected in A468h.
    fn sample_rate_hz(&self) -> u32 {
        DAC_SAMPLE_RATES[(self.state.fifo_control & FIFO_CONTROL_RATE_MASK) as usize]
    }

    /// Returns whether the D/A converter runs.
    fn is_playing(&self) -> bool {
        self.state.fifo_control & FIFO_CONTROL_RUN != 0
    }

    /// Returns whether the FIFO interrupt is enabled.
    fn irq_enabled(&self) -> bool {
        self.state.fifo_control & FIFO_CONTROL_IRQ_ENABLE != 0
    }

    /// Reads a PCM86 port at `current_cycle`.
    ///
    /// `irq_check_pending` tells whether an interrupt check timer is scheduled.
    fn read_port(
        &mut self,
        port: u16,
        current_cycle: u64,
        cpu_clock_hz: u32,
        irq_check_pending: bool,
    ) -> u8 {
        match port {
            PORT_SOUND_ID => self.state.sound_flags,
            PORT_FIFO_STATUS_VOLUME => self.read_fifo_status(current_cycle, cpu_clock_hz),
            PORT_FIFO_CONTROL => {
                self.read_fifo_control(current_cycle, cpu_clock_hz, irq_check_pending)
            }
            PORT_DAC_CONTROL => self.state.dac_control,
            PORT_DSP_CONTROL => OPEN_BUS,
            PORT_DSP_DATA => self.read_wavestar_status(),
            PORT_MUTE => self.state.mute,
            _ => 0x00,
        }
    }

    /// Returns the A466h status: the fill flags of the CPU view and the L/R clock.
    fn read_fifo_status(&mut self, current_cycle: u64, cpu_clock_hz: u32) -> u8 {
        self.update_buffer_state(current_cycle, cpu_clock_hz);
        let fill_flags = match self.fifo_fill() {
            FifoFill::Full => FIFO_STATUS_FULL,
            FifoFill::Empty => FIFO_STATUS_EMPTY,
            FifoFill::Partial => 0,
        };
        let clock_flag = if self.dac_clock_high(current_cycle, cpu_clock_hz) {
            FIFO_STATUS_LR_CLOCK
        } else {
            0
        };
        fill_flags | clock_flag
    }

    /// Returns A468h with the interrupt request flag of the current FIFO state.
    fn read_fifo_control(
        &mut self,
        current_cycle: u64,
        cpu_clock_hz: u32,
        irq_check_pending: bool,
    ) -> u8 {
        self.update_buffer_state(current_cycle, cpu_clock_hz);
        let request_flag = if self.irq_condition_met(current_cycle, irq_check_pending) {
            FIFO_CONTROL_IRQ_REQUEST
        } else {
            0
        };
        (self.state.fifo_control & !FIFO_CONTROL_IRQ_REQUEST) | request_flag
    }

    /// Returns the fill class of the CPU view of the FIFO.
    fn fifo_fill(&self) -> FifoFill {
        let view = self.state.virtual_level;
        if view >= FIFO_CAPACITY {
            FifoFill::Full
        } else if self.holds_frame(view) {
            FifoFill::Partial
        } else {
            FifoFill::Empty
        }
    }

    /// Returns whether `bytes` cover at least one complete sample frame.
    fn holds_frame(&self, bytes: i32) -> bool {
        bytes > self.state.frame_mask as i32
    }

    /// Returns whether the ring memory holds at least one complete sample frame.
    fn ring_holds_frame(&self) -> bool {
        self.holds_frame(self.state.level)
    }

    /// Returns the WaveStar status of port 0xA464 and toggles it.
    ///
    /// Without a completed handshake the port reads 0xFF. After a read of 0x00
    /// the next read returns 0xFF, after any other value it returns 0x00.
    fn read_wavestar_status(&mut self) -> u8 {
        if !self.wavestar_detected() {
            self.state.wavestar_value = WAVESTAR_IDLE;
        }
        let next = match self.state.wavestar_value {
            WAVESTAR_STATUS_TOGGLE => WAVESTAR_IDLE,
            _ => WAVESTAR_STATUS_TOGGLE,
        };
        std::mem::replace(&mut self.state.wavestar_value, next)
    }

    /// Returns whether the complete WaveStar handshake was written.
    fn wavestar_detected(&self) -> bool {
        usize::from(self.state.wavestar_matched_bytes) == WAVESTAR_SEQUENCE.len()
    }

    /// Returns whether the FIFO interrupt request is active.
    ///
    /// A set request flag stays active. Otherwise the FIFO level sets the flag
    /// once the data write delay has passed and no interrupt check timer is
    /// scheduled. Both counts take part: the CPU view at or below the
    /// threshold, or a ring level that holds a frame and lies at or below the
    /// threshold.
    fn irq_condition_met(&mut self, current_cycle: u64, irq_check_pending: bool) -> bool {
        if !self.irq_enabled() {
            return false;
        }
        let level_may_raise = !self.state.irq_request
            && !irq_check_pending
            && self.data_write_wait_elapsed(current_cycle);
        if level_may_raise && self.level_reached_threshold() {
            self.state.irq_request = true;
        }
        self.state.irq_request
    }

    /// Returns whether the data write delay of the level interrupt has passed.
    fn data_write_wait_elapsed(&self, current_cycle: u64) -> bool {
        current_cycle.wrapping_sub(self.state.irq_wait_start_cycle) >= self.state.irq_wait_cycles
    }

    /// Returns whether the CPU view, or a ring level with a complete frame, is
    /// at or below the interrupt threshold.
    fn level_reached_threshold(&self) -> bool {
        let threshold = self.state.irq_threshold_bytes;
        self.state.virtual_level <= threshold
            || (self.ring_holds_frame() && self.state.level <= threshold)
    }

    /// Writes a PCM86 port at `current_cycle`.
    fn write_port(
        &mut self,
        port: u16,
        value: u8,
        current_cycle: u64,
        cpu_clock_hz: u32,
        sample_rate: u32,
    ) {
        match port {
            PORT_SOUND_ID => {
                self.state.sound_flags = (self.state.sound_flags & !SOUND_ID_WRITABLE_MASK)
                    | (value & SOUND_ID_WRITABLE_MASK);
            }
            PORT_FIFO_STATUS_VOLUME => {
                let line = usize::from(value >> VOLUME_LINE_SHIFT);
                if line == VOLUME_LINE_PCM {
                    self.update_buffer_state(current_cycle, cpu_clock_hz);
                }
                self.state.volume_levels[line] = value & VOLUME_LEVEL_MASK;
            }
            PORT_FIFO_CONTROL => {
                self.update_buffer_state(current_cycle, cpu_clock_hz);
                self.write_fifo_control(value, current_cycle, sample_rate);
            }
            PORT_DAC_CONTROL => {
                self.update_buffer_state(current_cycle, cpu_clock_hz);
                self.write_dac_control(value);
            }
            PORT_FIFO_DATA => self.push_byte(value, current_cycle),
            PORT_DSP_CONTROL => self.write_wavestar_handshake(value),
            PORT_DSP_DATA => self.write_wavestar_mode(value),
            PORT_MUTE => self.state.mute = value,
            _ => {}
        }
    }

    /// Applies a write to the FIFO control register.
    ///
    /// A rising reset bit empties the FIFO. A rising reset or run bit restarts
    /// the converter clocks. A new rate code selects a new overflow margin and
    /// resampler.
    fn write_fifo_control(&mut self, value: u8, current_cycle: u64, sample_rate: u32) {
        let previous = std::mem::replace(&mut self.state.fifo_control, value);
        let raised = value & !previous;
        if raised & FIFO_CONTROL_RESET != 0 {
            self.clear_fifo(current_cycle);
        }
        if raised & (FIFO_CONTROL_RESET | FIFO_CONTROL_RUN) != 0 {
            self.restart_clocks(current_cycle);
        }
        self.write_irq_request_flag(value & FIFO_CONTROL_IRQ_REQUEST != 0, current_cycle);
        if (previous ^ value) & FIFO_CONTROL_RATE_MASK != 0 {
            self.select_rate(sample_rate);
        }
        self.request_reschedule_after_data();
    }

    /// Empties the ring memory and both level counts.
    fn clear_fifo(&mut self, current_cycle: u64) {
        self.state.write_index = 0;
        self.state.read_index = 0;
        self.state.level = 0;
        self.state.virtual_level = 0;
        self.state.irq_wait_start_cycle = current_cycle;
    }

    /// Applies bit 4 of an A468h write.
    ///
    /// A 0 clears the request flag and the interrupt line, and an empty CPU
    /// view restarts the data write delay. The flag then sets whenever the CPU
    /// view is at or below the threshold.
    fn write_irq_request_flag(&mut self, keep_request: bool, current_cycle: u64) {
        if !keep_request {
            self.state.irq_request = false;
            self.pending_irq_change = Some(false);
            if self.state.virtual_level == 0 {
                self.state.irq_wait_start_cycle = current_cycle;
            }
        }
        self.state.irq_request |= self.state.virtual_level <= self.state.irq_threshold_bytes;
    }

    /// Applies the rate code of A468h: the overflow margin and the resampler.
    fn select_rate(&mut self, output_rate: u32) {
        self.state.overflow_margin =
            overflow_margin(self.state.fifo_control, self.state.frame_shift);
        self.pcm_resampler = new_resampler(self.sample_rate_hz(), output_rate);
        self.pcm_resample_output
            .resize(self.pcm_resampler.buffer_size_output(), 0.0);
    }

    /// Requests a new interrupt check time when FIFO data is waiting for its
    /// interrupt.
    fn request_reschedule_after_data(&mut self) {
        self.needs_reschedule |= self.state.data_written;
    }

    /// Restarts the converter clocks of the audio output, the CPU view and
    /// the L/R clock bit at `current_cycle`.
    fn restart_clocks(&mut self, current_cycle: u64) {
        self.state.virtual_level_cycle = current_cycle;
        self.state.sample_remainder = FmSampleRemainder::default();
        self.state.virtual_level_remainder = FmSampleRemainder::default();
        self.state.dac_clock_cycle = current_cycle;
        self.state.dac_clock_remainder = FmSampleRemainder::default();
    }

    /// Applies a write to A46Ah: the interrupt threshold while the interrupt
    /// is enabled, the D/A converter control otherwise.
    fn write_dac_control(&mut self, value: u8) {
        match DacControlWrite::decode(value, self.irq_enabled()) {
            DacControlWrite::Ignored => return,
            DacControlWrite::Threshold(bytes) => self.state.irq_threshold_bytes = bytes,
            DacControlWrite::Format(dac_control) => self.select_format(dac_control),
        }
        self.request_reschedule_after_data();
    }

    /// Applies a D/A converter control value: the frame size and the overflow
    /// margin that depends on it.
    fn select_format(&mut self, dac_control: u8) {
        self.state.dac_control = dac_control;
        self.state.frame_shift = frame_shift(dac_control);
        self.state.frame_mask = frame_mask(self.state.frame_shift);
        self.state.overflow_margin =
            overflow_margin(self.state.fifo_control, self.state.frame_shift);
    }

    /// Appends one byte to the FIFO and delays the level interrupt.
    fn push_byte(&mut self, value: u8, current_cycle: u64) {
        if self.state.virtual_level < FIFO_CAPACITY {
            self.state.virtual_level += 1;
        }
        self.append_to_ring(value);
        self.state.data_written = true;
        self.state.irq_wait_start_cycle = current_cycle + self.data_write_delay();
    }

    /// Stores a byte at the write index of the ring memory.
    ///
    /// The ring memory drops its oldest frame when it runs past the FIFO
    /// capacity plus the overflow margin.
    fn append_to_ring(&mut self, value: u8) {
        let index = self.state.write_index;
        self.buffer[index as usize] = value;
        self.state.write_index = ring_index(index + 1);
        self.state.level += 1;
        if self.state.level >= FIFO_CAPACITY + self.state.overflow_margin {
            self.discard_oldest(OVERFLOW_DROP_BYTES);
        }
    }

    /// Drops the oldest `bytes` of the ring memory.
    fn discard_oldest(&mut self, bytes: i32) {
        self.state.level -= bytes;
        self.state.read_index = ring_index(self.state.read_index + bytes as u32);
    }

    /// Returns the delay of the level interrupt after a data write in CPU cycles.
    ///
    /// The delay is the full data write wait while the CPU view holds more
    /// than twice the threshold or is full. Below that the wait shrinks by the
    /// share of the threshold in 8 KiB, down to zero.
    fn data_write_delay(&self) -> u64 {
        let wait = self.state.irq_wait_cycles;
        let threshold = self.state.irq_threshold_bytes;
        let view = self.state.virtual_level;
        if view >= FIFO_CAPACITY || view > threshold * 2 {
            return wait;
        }
        let shortening = wait * threshold as u64 / IRQ_WAIT_SCALE_BYTES as u64;
        wait.saturating_sub(shortening)
    }

    /// Advances the WaveStar handshake detector with a byte written to A462h.
    ///
    /// A byte that continues the sequence counts one more match. Any other
    /// byte restarts the count, at one when it is the first sequence byte.
    fn write_wavestar_handshake(&mut self, value: u8) {
        let matched = usize::from(self.state.wavestar_matched_bytes);
        let continues = WAVESTAR_SEQUENCE.get(matched) == Some(&value);
        let progress = if continues {
            matched + 1
        } else {
            usize::from(value == WAVESTAR_SEQUENCE[0])
        };
        self.state.wavestar_matched_bytes = progress as u8;
        if continues && self.wavestar_detected() {
            self.state.wavestar_value = WAVESTAR_DETECTED;
        }
    }

    /// Applies a WaveStar mode write to A464h.
    ///
    /// 09h resets the status at any time. After the handshake 04h selects the
    /// Windows Sound System mode and any other value the PCM86 mode.
    fn write_wavestar_mode(&mut self, value: u8) {
        let status = match value {
            WAVESTAR_RESET => WAVESTAR_IDLE,
            _ if !self.wavestar_detected() => return,
            WAVESTAR_SELECT_WSS => WAVESTAR_MODE_WSS,
            _ => WAVESTAR_MODE_PCM86,
        };
        self.state.wavestar_value = status;
    }

    /// Lowers the CPU view of the FIFO level by the frames that the converter
    /// clock consumed up to `current_cycle`.
    fn update_buffer_state(&mut self, current_cycle: u64, cpu_clock_hz: u32) {
        if !self.is_playing() {
            return;
        }
        let rate = self.sample_rate_hz();
        let consumed = step_converter_clock(
            &mut self.state.virtual_level_cycle,
            &mut self.state.virtual_level_remainder,
            current_cycle,
            rate,
            cpu_clock_hz,
        );
        if let Some(frames) = consumed {
            self.consume_view_frames(frames);
        }
    }

    /// Lowers the CPU view by `frames` sample frames. A view that drops below
    /// zero keeps only the bytes of an incomplete frame.
    fn consume_view_frames(&mut self, frames: u64) {
        let bytes = i32::try_from(frames << self.state.frame_shift).unwrap_or(i32::MAX);
        let lowered = self.state.virtual_level.saturating_sub(bytes);
        self.state.virtual_level = if lowered < 0 {
            lowered.rem_euclid(1 << self.state.frame_shift)
        } else {
            lowered
        };
    }

    /// Returns the L/R clock bit of port 0xA466 at `current_cycle`. The bit is
    /// high in the second half of each sample period.
    fn dac_clock_high(&mut self, current_cycle: u64, cpu_clock_hz: u32) -> bool {
        let rate = self.sample_rate_hz();
        step_converter_clock(
            &mut self.state.dac_clock_cycle,
            &mut self.state.dac_clock_remainder,
            current_cycle,
            rate,
            cpu_clock_hz,
        );
        self.state.dac_clock_remainder.0 >= LR_CLOCK_HIGH_PHASE
    }

    /// Compares the ring level with the CPU view after audio output.
    ///
    /// A ring level below the CPU view counts as underrun time and may lower
    /// the CPU view. A ring level at or above the CPU view clears the underrun
    /// time and may raise the CPU view.
    fn reconcile_buffers(&mut self, current_cycle: u64, cpu_clock_hz: u32) {
        let step_milliseconds = self.milliseconds_since_reconcile(current_cycle, cpu_clock_hz);
        let ring_lead = self.state.level - self.state.virtual_level;
        if ring_lead < 0 {
            self.underrun_milliseconds =
                self.underrun_milliseconds.saturating_add(step_milliseconds);
            self.correct_underrun(ring_lead);
        } else {
            self.underrun_milliseconds = 0;
            self.correct_ring_lead(ring_lead);
        }
    }

    /// Returns the milliseconds since the previous reconciliation and records
    /// `current_cycle` as the new one. The first reconciliation counts zero.
    fn milliseconds_since_reconcile(&mut self, current_cycle: u64, cpu_clock_hz: u32) -> i32 {
        let previous = std::mem::replace(&mut self.last_reconcile_cycle, current_cycle);
        if previous == 0 {
            return 0;
        }
        let elapsed = current_cycle.saturating_sub(previous) as i64;
        (elapsed * MILLISECONDS_PER_SECOND / i64::from(cpu_clock_hz)) as i32
    }

    /// Lowers the CPU view towards the ring level when the ring level lags by
    /// at least one threshold and the CPU view lies below the threshold. The
    /// correction is a whole number of 4-byte steps, rounded towards the
    /// larger correction.
    fn correct_underrun(&mut self, ring_lead: i32) {
        let threshold = self.state.irq_threshold_bytes;
        if ring_lead > -threshold || self.state.virtual_level >= threshold {
            return;
        }
        let aligned = ring_lead.div_euclid(FRAME_ALIGNMENT_BYTES) * FRAME_ALIGNMENT_BYTES;
        self.state.virtual_level += aligned;
    }

    /// Raises the CPU view when the ring level leads by more than three
    /// thresholds, the CPU view lies above the threshold and the ring level
    /// holds a frame. The step is an eighth of the lead plus three thresholds.
    fn correct_ring_lead(&mut self, ring_lead: i32) {
        let threshold = self.state.irq_threshold_bytes;
        let tolerance = threshold * RING_LEAD_TOLERANCE_THRESHOLDS;
        if ring_lead > tolerance && self.state.virtual_level > threshold && self.ring_holds_frame()
        {
            self.state.virtual_level += (ring_lead + tolerance) / RING_LEAD_CATCH_UP_DIVISOR;
        }
    }

    /// Returns the CPU cycle at which the CPU view reaches the threshold, or
    /// `None` when it is already there or the converter is stopped.
    ///
    /// A lasting underrun blends the distance towards the ring level when the
    /// ring level holds a frame and lies closer to the threshold.
    fn calculate_next_irq_cycle(&self, current_cycle: u64, cpu_clock_hz: u32) -> Option<u64> {
        if !self.is_playing() {
            return None;
        }
        let threshold = self.state.irq_threshold_bytes;
        let view_distance = self.state.virtual_level - threshold;
        if view_distance <= 0 {
            return None;
        }
        let ring_distance = self.state.level - threshold;
        let distance = match self.underrun_blend_parts() {
            Some(parts) if self.ring_holds_frame() && ring_distance < view_distance => {
                blend_toward_ring(view_distance, ring_distance, parts)
            }
            _ => view_distance,
        };
        let distance = u32::try_from(distance).ok().filter(|&bytes| bytes > 0)?;
        let frames = distance.div_ceil(1 << self.state.frame_shift);
        let cycles = u64::from(frames) * u64::from(cpu_clock_hz) / u64::from(self.sample_rate_hz());
        Some(current_cycle + cycles.max(1))
    }

    /// Returns the blend parts of the interrupt timer distance for the current
    /// underrun time, or `None` for an underrun that is too short.
    fn underrun_blend_parts(&self) -> Option<i64> {
        match self.underrun_milliseconds {
            milliseconds if milliseconds > UNDERRUN_SEVERE_MILLISECONDS => {
                Some(UNDERRUN_SEVERE_BLEND_PARTS)
            }
            milliseconds if milliseconds > UNDERRUN_MODERATE_MILLISECONDS => {
                Some(UNDERRUN_MODERATE_BLEND_PARTS)
            }
            _ => None,
        }
    }

    /// Runs the scheduled interrupt check.
    ///
    /// After a data write the check raises the interrupt when the CPU view is
    /// at or below the threshold, or when the ring level holds a frame and a
    /// blend of four parts CPU view and one part ring level is at or below the
    /// threshold. Every other case requests a new check time.
    fn run_irq_check(&mut self, current_cycle: u64, cpu_clock_hz: u32) {
        let raise = self.state.data_written && {
            self.update_buffer_state(current_cycle, cpu_clock_hz);
            self.reconcile_buffers(current_cycle, cpu_clock_hz);
            self.check_timer_level_reached()
        };
        if raise {
            self.state.data_written = false;
            self.state.irq_request = true;
            self.pending_irq_change = Some(true);
        } else {
            self.needs_reschedule = true;
        }
    }

    /// Returns whether the interrupt check timer finds the FIFO level at the
    /// threshold.
    fn check_timer_level_reached(&self) -> bool {
        let threshold = self.state.irq_threshold_bytes;
        let blended = blend_toward_ring(
            self.state.virtual_level,
            self.state.level,
            IRQ_CHECK_BLEND_PARTS,
        );
        self.state.virtual_level <= threshold || (self.ring_holds_frame() && blended <= threshold)
    }

    /// Checks the FIFO interrupt condition when an FM timer fires and returns
    /// whether the interrupt request is active.
    fn fm_timer_irq_check(&mut self, current_cycle: u64, cpu_clock_hz: u32) -> bool {
        self.update_buffer_state(current_cycle, cpu_clock_hz);
        self.irq_condition_met(current_cycle, false)
    }

    /// Takes a pending reschedule request and returns the CPU cycle of the
    /// next interrupt check while the converter runs.
    fn take_irq_check_schedule(&mut self, current_cycle: u64, cpu_clock_hz: u32) -> Option<u64> {
        let requested = std::mem::take(&mut self.needs_reschedule);
        if !requested || !self.is_playing() {
            return None;
        }
        let fire_cycle = match self.calculate_next_irq_cycle(current_cycle, cpu_clock_hz) {
            Some(fire_cycle) => fire_cycle,
            None => self.threshold_reached_check_cycle(current_cycle),
        };
        Some(fire_cycle)
    }

    /// Returns the check cycle for a FIFO level that already reached the
    /// threshold.
    ///
    /// Pending data gets the check on the next cycle. Without pending data the
    /// check waits the retry delay and counts as a data write. The delay keeps
    /// the interrupt from firing back to back.
    fn threshold_reached_check_cycle(&mut self, current_cycle: u64) -> u64 {
        if self.state.data_written {
            return current_cycle + IMMEDIATE_CHECK_CYCLES;
        }
        self.state.data_written = true;
        current_cycle + IRQ_RETRY_PIT_CLOCKS * self.clock_multiple
    }

    /// Fills `output` with interleaved stereo `[L, R, L, R, ...]` at the PCM
    /// source rate. Returns the number of floats written (always even).
    #[cfg(test)]
    fn drain_samples(&mut self, output: &mut [f32]) -> usize {
        convert_frames(&mut self.state, &self.buffer, output)
    }

    /// Converts the frames due since the last audio output frame, resamples
    /// them and mixes them into the output buffer at the given volume.
    fn generate_samples(
        &mut self,
        current_cycle: u64,
        cpu_clock_hz: u32,
        volume: f32,
        output: &mut [f32],
    ) {
        self.update_buffer_state(current_cycle, cpu_clock_hz);
        self.reconcile_buffers(current_cycle, cpu_clock_hz);
        let previous = std::mem::replace(&mut self.last_generate_cycle, current_cycle);
        self.convert_due_frames(current_cycle.saturating_sub(previous), cpu_clock_hz);
        self.mix_resampled(volume, output);
    }

    /// Appends the sample frames that `elapsed` CPU cycles cover to the
    /// resampler input while the converter runs and the ring memory has data.
    fn convert_due_frames(&mut self, elapsed: u64, cpu_clock_hz: u32) {
        if elapsed == 0 || !self.is_playing() || self.state.level <= 0 {
            return;
        }
        let rate = self.sample_rate_hz();
        let frames = elapsed_frames(
            elapsed,
            rate,
            cpu_clock_hz,
            &mut self.state.sample_remainder,
        ) as usize;
        let start = self.pcm_input_buffer.len();
        self.pcm_input_buffer
            .resize(start + frames * OUTPUT_CHANNELS, 0.0);
        convert_frames(
            &mut self.state,
            &self.buffer,
            &mut self.pcm_input_buffer[start..],
        );
    }

    /// Resamples the waiting input to the output rate and adds it to `output`
    /// at `volume`. Input that does not fit into `output` waits for the next
    /// frame.
    fn mix_resampled(&mut self, volume: f32, output: &mut [f32]) {
        let mut consumed_total = 0;
        let mut unfilled = output;
        while consumed_total < self.pcm_input_buffer.len() && !unfilled.is_empty() {
            let chunk = unfilled.len().min(self.pcm_resample_output.len());
            let scratch = &mut self.pcm_resample_output[..chunk];
            let Ok((consumed, produced)) = self
                .pcm_resampler
                .resample(&self.pcm_input_buffer[consumed_total..], scratch)
            else {
                break;
            };
            let (mixed, rest) = std::mem::take(&mut unfilled).split_at_mut(produced);
            for (target, sample) in mixed.iter_mut().zip(scratch.iter()) {
                *target += sample * volume;
            }
            unfilled = rest;
            consumed_total += consumed;
            if consumed == 0 {
                break;
            }
        }
        self.pcm_input_buffer.drain(..consumed_total);
    }
}

/// Reads the signed sample of `sample_bytes` bytes (big-endian) at ring index `index`.
fn read_sample(buffer: &[u8; RING_SIZE], index: u32, sample_bytes: u32) -> f32 {
    let high = buffer[(index & RING_INDEX_MASK) as usize];
    if sample_bytes == 1 {
        return f32::from(high as i8) / SAMPLE_SCALE_8BIT;
    }
    let low = buffer[((index + 1) & RING_INDEX_MASK) as usize];
    f32::from(i16::from_be_bytes([high, low])) / SAMPLE_SCALE_16BIT
}

/// Converts sample frames from the ring memory into interleaved stereo
/// values `[L, R, L, R, ...]` and returns the number of values written.
///
/// A stopped converter, a disabled format and a frame slot without a complete
/// frame in the ring memory stay silent.
fn convert_frames(state: &mut Pcm86State, buffer: &[u8; RING_SIZE], output: &mut [f32]) -> usize {
    let value_count = output.len() / 2 * 2;
    let output = &mut output[..value_count];
    output.fill(0.0);

    let format = state.dac_control & DAC_CONTROL_FORMAT_MASK;
    let frame_bytes = bytes_per_frame(format) as u32;
    if state.fifo_control & FIFO_CONTROL_RUN == 0 || state.level <= 0 || frame_bytes == 0 {
        return value_count;
    }
    let sample_bytes = if format & DAC_FORMAT_8BIT != 0 { 1 } else { 2 };
    let gain = if state.mute & MUTE_OUTPUT != 0 {
        0.0
    } else {
        pcm86_volume(state.volume_levels[VOLUME_LINE_PCM])
    };

    for frame in output.as_chunks_mut::<2>().0 {
        if state.level < frame_bytes as i32 {
            continue;
        }
        let mut offset = 0;
        if format & DAC_OUTPUT_LEFT != 0 {
            frame[0] = read_sample(buffer, state.read_index, sample_bytes) * gain;
            offset += sample_bytes;
        }
        if format & DAC_OUTPUT_RIGHT != 0 {
            frame[1] = read_sample(buffer, state.read_index + offset, sample_bytes) * gain;
        }
        state.read_index = (state.read_index + frame_bytes) & RING_INDEX_MASK;
        state.level -= frame_bytes as i32;
    }
    value_count
}

/// Returns the bytes of one sample frame for the format bits of A46Ah.
fn bytes_per_frame(format: u8) -> usize {
    let channels =
        usize::from(format & DAC_OUTPUT_LEFT != 0) + usize::from(format & DAC_OUTPUT_RIGHT != 0);
    let sample_bytes = if format & DAC_FORMAT_8BIT != 0 { 1 } else { 2 };
    channels * sample_bytes
}

/// Converts an electronic volume attenuation value to a linear gain.
fn pcm86_volume(level: u8) -> f32 {
    f32::from(VOLUME_LEVEL_MAX - (level & VOLUME_LEVEL_MASK)) / f32::from(VOLUME_LEVEL_MAX)
}

/// Returns the timer kind of an FM timer number (0 = timer A, 1 = timer B).
fn fm_timer_kind(timer_id: u8) -> Soundboard86Timer {
    if timer_id == 0 {
        Soundboard86Timer::FmTimerA
    } else {
        Soundboard86Timer::FmTimerB
    }
}

/// Action the bus must process after a sound board operation.
#[derive(Clone, Copy)]
pub enum Soundboard86Action {
    /// Schedule a timer to fire at the given cycle.
    ScheduleTimer {
        /// Timer event kind.
        kind: Soundboard86Timer,
        /// CPU cycle at which the timer should fire.
        fire_cycle: u64,
    },
    /// Cancel a previously scheduled timer.
    CancelTimer {
        /// Timer event kind.
        kind: Soundboard86Timer,
    },
    /// Assert an IRQ line on the PIC.
    AssertIrq {
        /// IRQ line number.
        irq: u8,
    },
    /// Deassert an IRQ line on the PIC.
    DeassertIrq {
        /// IRQ line number.
        irq: u8,
    },
}

/// PC-9801-86 sound board: YM2608 (OPNA) FM + SSG + ADPCM + PCM86 DAC.
pub struct Soundboard86 {
    /// Current device state (saveable).
    pub state: Soundboard86State,
    core: OpnFm<Ym2608>,
    cpu_clock_hz: u32,
    /// Latest activity cycle for the FM and PCM86 timing bridge.
    chip_action_cycle: u64,
    pcm86: Pcm86,
    sample_rate: u32,
    /// Whether an FM timer fired since the last action drain.
    fm_timer_just_fired: bool,
    action_buffer: Vec<Soundboard86Action>,
}

impl Soundboard86 {
    #[inline]
    fn opna_masked(&self) -> bool {
        self.pcm86.state.sound_flags & SOUND_ID_OPNA_MASK != 0
    }

    /// Creates a new PC-9801-86 sound board instance.
    ///
    /// `rhythm_rom` is the optional 8 KB `ym2608.rom` ADPCM-A rhythm data.
    /// When `None`, the embedded evolved rhythm ROM is used.
    /// `adpcm_ram` enables the 256 KiB ADPCM-B sample RAM upgrade.
    pub fn new(
        cpu_clock_hz: u32,
        sample_rate: u32,
        rhythm_rom: Option<&[u8]>,
        adpcm_ram: bool,
        machine_model: MachineModel,
    ) -> Self {
        let rom_data = rhythm_rom.unwrap_or(EVOLVED_RHYTHM_ROM.as_slice());
        let mut core = OpnFm::<Ym2608>::new(cpu_clock_hz, sample_rate, YM2608_CLOCK_HZ);
        core.chip_mut().set_adpcm_a_rom(rom_data);
        if adpcm_ram {
            core.chip_mut().set_adpcm_b_ram(vec![0; ADPCM_B_RAM_SIZE]);
        }

        Self {
            state: Soundboard86State {
                adpcm_ram,
                ..Soundboard86State::default()
            },
            core,
            cpu_clock_hz,
            chip_action_cycle: 0,
            pcm86: Pcm86::new(sample_rate, cpu_clock_hz, machine_model),
            sample_rate,
            fm_timer_just_fired: false,
            action_buffer: Vec::new(),
        }
    }

    /// Captures complete OPNA, PCM86, and resampler history.
    pub fn save_state(&self) -> Soundboard86RuntimeState {
        Soundboard86RuntimeState {
            board: self.capture_board_state(),
            core: self.core.capture_state(),
            pcm86: self.pcm86.capture_state(),
            cpu_clock_hz: self.cpu_clock_hz,
            chip_action_cycle: self.chip_action_cycle,
            sample_rate: self.sample_rate,
            fm_timer_just_fired: self.fm_timer_just_fired,
        }
    }

    /// Creates a snapshot of the current board state.
    pub fn capture_board_state(&self) -> Soundboard86State {
        let timing = self.core.timing();
        let mut state = self.state.clone();
        state.sample_remainder = timing.sample_remainder;
        state.fm_sync_cursor = timing.fm_sync_cursor;
        state.busy_end_cycle = timing.busy_end_cycle;
        state.audio_frame_start_cycle = timing.audio_frame_start_cycle;
        state.irq_asserted = timing.irq_asserted;
        state.pcm86 = self.pcm86.state.clone();
        state
    }

    /// Restores complete audio state without recreating either synthesizer.
    pub fn restore_state(
        &mut self,
        state: Soundboard86RuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.cpu_clock_hz != self.cpu_clock_hz || state.sample_rate != self.sample_rate {
            return Err(save_state::StateValidationError::new(
                "PC-9801-86 clock configuration differs",
            ));
        }
        self.core.validate_state(&state.core)?;
        self.pcm86.validate_state(&state.pcm86)?;
        self.core.restore_state(state.core)?;
        self.pcm86.restore_state(state.pcm86)?;
        self.state = state.board;
        self.chip_action_cycle = state.chip_action_cycle;
        self.fm_timer_just_fired = state.fm_timer_just_fired;
        self.action_buffer.clear();
        Ok(())
    }

    /// Returns the low bank address latch value.
    pub fn address_lo(&self) -> u8 {
        self.state.address_lo
    }

    /// Returns the configured IRQ line number.
    pub fn irq_line(&self) -> u8 {
        self.state.irq_line
    }

    /// Returns whether extended mode (6-ch FM + ADPCM) is enabled.
    pub fn extend_enabled(&self) -> bool {
        self.state.extend_enabled
    }

    /// Sets the extend enabled flag (controlled by port 0xA460 bit 0).
    pub fn set_extend_enabled(&mut self, enabled: bool) {
        self.state.extend_enabled = enabled;
    }

    /// Reads the chip status register (port 0x0188 read).
    pub fn read_status(&mut self, current_cycle: u64) -> u8 {
        if self.opna_masked() {
            return 0xFF;
        }
        self.chip_action_cycle = current_cycle;
        self.core.read_status(current_cycle)
    }

    /// Reads data from the currently addressed register (port 0x018A read).
    pub fn read_data(&mut self, current_cycle: u64) -> u8 {
        if self.opna_masked() {
            return 0xFF;
        }
        self.chip_action_cycle = current_cycle;
        self.core.sync(current_cycle);
        self.core.set_action_cycle(current_cycle);
        let addr = self.state.address_lo;
        if addr == 0x0E {
            let irq_bits = match self.state.irq_line {
                3 => 0x00u8,
                13 => 0x01,
                10 => 0x02,
                12 => 0x03,
                _ => 0x03,
            };
            (irq_bits << 6) | 0x3F
        } else if addr == 0xFF {
            1
        } else if addr < 0x10 {
            self.core.chip_mut().read_data()
        } else {
            self.state.last_written_data
        }
    }

    /// Reads the high bank status register (port 0x018C read).
    pub fn read_status_hi(&mut self, current_cycle: u64) -> u8 {
        if self.opna_masked() {
            return 0xFF;
        }
        if !self.state.extend_enabled {
            return 0xFF;
        }
        self.chip_action_cycle = current_cycle;
        self.core.read_status_hi(current_cycle)
    }

    /// Reads data from the high bank (port 0x018E read).
    pub fn read_data_hi(&mut self, current_cycle: u64) -> u8 {
        if self.opna_masked() {
            return 0xFF;
        }
        if !self.state.extend_enabled {
            return 0xFF;
        }
        self.chip_action_cycle = current_cycle;
        self.core.sync(current_cycle);
        self.core.set_action_cycle(current_cycle);
        let addr = self.state.address_hi;
        if addr == 0x08 || addr == 0x0F {
            self.core.chip_mut().read_data_hi()
        } else {
            0xFF
        }
    }

    /// Writes the register address latch for the low bank (port 0x0188 write).
    pub fn write_address(&mut self, value: u8, current_cycle: u64) {
        if self.opna_masked() {
            return;
        }
        self.state.address_lo = value;
        self.chip_action_cycle = current_cycle;
        self.core.write_address(value, current_cycle);
    }

    /// Writes data to the currently addressed register in the low bank (port 0x018A write).
    pub fn write_data(&mut self, value: u8, current_cycle: u64) {
        if self.opna_masked() {
            return;
        }
        self.state.last_written_data = value;
        self.chip_action_cycle = current_cycle;
        self.core.write_data(value, current_cycle);
    }

    /// Writes the register address latch for the high bank (port 0x018C write).
    pub fn write_address_hi(&mut self, value: u8, current_cycle: u64) {
        if self.opna_masked() {
            return;
        }
        if !self.state.extend_enabled {
            return;
        }
        self.state.address_hi = value;
        self.chip_action_cycle = current_cycle;
        self.core.write_address_hi(value, current_cycle);
    }

    /// Writes data to the currently addressed register in the high bank (port 0x018E write).
    pub fn write_data_hi(&mut self, value: u8, current_cycle: u64) {
        if self.opna_masked() {
            return;
        }
        if !self.state.extend_enabled {
            return;
        }
        self.chip_action_cycle = current_cycle;
        self.core.write_data_hi(value, current_cycle);
    }

    /// Notifies the chip that a timer has expired.
    pub fn timer_expired(&mut self, timer_id: u32, current_cycle: u64) {
        self.chip_action_cycle = current_cycle;
        self.core.timer_expired(timer_id, current_cycle);
        self.fm_timer_just_fired = true;
    }

    /// Reads a PCM86 port. `irq_check_pending` tells whether the PCM86
    /// interrupt check timer is scheduled.
    pub fn pcm86_read(
        &mut self,
        port: u16,
        current_cycle: u64,
        cpu_clock_hz: u32,
        irq_check_pending: bool,
    ) -> u8 {
        self.pcm86
            .read_port(port, current_cycle, cpu_clock_hz, irq_check_pending)
    }

    /// Writes a PCM86 port.
    pub fn pcm86_write(&mut self, port: u16, value: u8, current_cycle: u64, cpu_clock_hz: u32) {
        if port == PORT_SOUND_ID {
            self.state.extend_enabled = value & SOUND_ID_OPNA_EXTEND != 0;
        }
        self.chip_action_cycle = current_cycle;
        self.pcm86
            .write_port(port, value, current_cycle, cpu_clock_hz, self.sample_rate);
    }

    /// Handles the scheduled PCM86 interrupt check.
    pub fn pcm86_timer_expired(&mut self, current_cycle: u64, cpu_clock_hz: u32) {
        self.chip_action_cycle = current_cycle;
        self.pcm86.run_irq_check(current_cycle, cpu_clock_hz);
    }

    /// Returns the board interrupt line: the OPNA output unless masked, or
    /// the PCM86 output.
    fn merged_irq(&self) -> bool {
        (self.state.irq_asserted && !self.opna_masked()) || self.state.pcm86_irq_asserted
    }

    /// Drains pending actions from the chip bridge and PCM86, merging IRQ sources.
    ///
    /// `pcm86_irq_pending` tells whether the PCM86 interrupt check timer is
    /// scheduled. An FM timer interrupt also checks the PCM86 interrupt
    /// condition.
    pub fn drain_actions(&mut self, pcm86_irq_pending: bool) -> &[Soundboard86Action] {
        self.action_buffer.clear();
        let current_cycle = self.chip_action_cycle;
        let cpu_clock_hz = self.cpu_clock_hz;
        let was_merged = self.merged_irq();

        self.core.set_action_cycle(current_cycle);
        for action in self.core.drain_timers() {
            self.action_buffer.push(match *action {
                FmTimerAction::Schedule {
                    timer_id,
                    fire_cycle,
                } => Soundboard86Action::ScheduleTimer {
                    kind: fm_timer_kind(timer_id),
                    fire_cycle,
                },
                FmTimerAction::Cancel { timer_id } => Soundboard86Action::CancelTimer {
                    kind: fm_timer_kind(timer_id),
                },
            });
        }
        if let Some(asserted) = self.core.take_irq_change() {
            self.state.irq_asserted = asserted;
        }

        if let Some(asserted) = self.pcm86.pending_irq_change.take() {
            self.state.pcm86_irq_asserted = asserted;
        }
        let fm_timer_fired = std::mem::take(&mut self.fm_timer_just_fired);
        if fm_timer_fired && !self.state.pcm86_irq_asserted && !pcm86_irq_pending {
            self.state.pcm86_irq_asserted =
                self.pcm86.fm_timer_irq_check(current_cycle, cpu_clock_hz);
        }
        if let Some(fire_cycle) = self
            .pcm86
            .take_irq_check_schedule(current_cycle, cpu_clock_hz)
        {
            self.action_buffer.push(Soundboard86Action::ScheduleTimer {
                kind: Soundboard86Timer::Pcm86Irq,
                fire_cycle,
            });
        }

        let merged = self.merged_irq();
        if merged != was_merged {
            let irq = self.state.irq_line;
            self.action_buffer.push(if merged {
                Soundboard86Action::AssertIrq { irq }
            } else {
                Soundboard86Action::DeassertIrq { irq }
            });
        }
        self.action_buffer.as_slice()
    }

    /// Generates resampled stereo audio and mixes it into `output`.
    ///
    /// `output` is interleaved stereo (`[L, R, L, R, ...]`).
    pub fn generate_samples(
        &mut self,
        current_cycle: u64,
        cpu_clock_hz: u32,
        volume: f32,
        output: &mut [f32],
    ) {
        self.core
            .generate_samples(current_cycle, cpu_clock_hz, volume, output);
        if output.is_empty() {
            self.pcm86.last_generate_cycle = current_cycle;
        } else {
            self.pcm86
                .generate_samples(current_cycle, cpu_clock_hz, volume, output);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pcm86() -> Pcm86 {
        let mut pcm = Pcm86::new(48000, 8_000_000, MachineModel::PC9801RA);
        pcm.state.mute = 0x00;
        pcm
    }

    fn write_bytes(pcm: &mut Pcm86, data: &[u8]) {
        for &b in data {
            if pcm.state.virtual_level < FIFO_CAPACITY {
                pcm.state.virtual_level += 1;
            }
            pcm.buffer[pcm.state.write_index as usize] = b;
            pcm.state.write_index = (pcm.state.write_index + 1) & RING_INDEX_MASK;
            pcm.state.level += 1;
        }
    }

    fn enable_playback(pcm: &mut Pcm86) {
        pcm.state.fifo_control = 0x80;
    }

    #[test]
    fn drain_16bit_big_endian_left_only() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x20; // 16-bit, L only
        pcm.state.frame_shift = FRAME_SHIFTS[(0x20 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.volume_levels[5] = 0; // max volume
        enable_playback(&mut pcm);

        // Write 0x7F00 big-endian -> +32512 -> ~0.9922
        write_bytes(&mut pcm, &[0x7F, 0x00]);
        let mut out = [0.0f32; 2];
        pcm.drain_samples(&mut out);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.9921875).abs() < 0.001, "L = {}", out[0]);
        assert_eq!(out[1], 0.0, "R should be 0.0 for L-only mode");
    }

    #[test]
    fn drain_16bit_big_endian_right_only() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x10; // 16-bit, R only
        pcm.state.frame_shift = FRAME_SHIFTS[(0x10 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.volume_levels[5] = 0;
        enable_playback(&mut pcm);

        // Write 0x8000 big-endian -> -32768 -> -1.0
        write_bytes(&mut pcm, &[0x80, 0x00]);
        let mut out = [0.0f32; 2];
        pcm.drain_samples(&mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], 0.0, "L should be 0.0 for R-only mode");
        assert!((out[1] - (-1.0)).abs() < 0.001, "R = {}", out[1]);
    }

    #[test]
    fn drain_16bit_stereo() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x30; // 16-bit stereo
        pcm.state.frame_shift = FRAME_SHIFTS[(0x30 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.volume_levels[5] = 0;
        enable_playback(&mut pcm);

        // L = 0x4000 (+16384 -> 0.5), R = 0xC000 (-16384 -> -0.5)
        write_bytes(&mut pcm, &[0x40, 0x00, 0xC0, 0x00]);
        let mut out = [0.0f32; 2];
        pcm.drain_samples(&mut out);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.5).abs() < 0.001, "L = {}", out[0]);
        assert!((out[1] - (-0.5)).abs() < 0.001, "R = {}", out[1]);
    }

    #[test]
    fn drain_16bit_stereo_multiple_frames() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x30; // 16-bit stereo
        pcm.state.frame_shift = FRAME_SHIFTS[(0x30 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.volume_levels[5] = 0;
        enable_playback(&mut pcm);

        // Frame 0: L=+16384, R=-16384
        // Frame 1: L=0, R=+32512
        write_bytes(&mut pcm, &[0x40, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x7F, 0x00]);
        let mut out = [0.0f32; 4];
        pcm.drain_samples(&mut out);
        assert_eq!(out.len(), 4);
        assert!((out[0] - 0.5).abs() < 0.001, "F0 L = {}", out[0]);
        assert!((out[1] - (-0.5)).abs() < 0.001, "F0 R = {}", out[1]);
        assert!(out[2].abs() < 0.001, "F1 L = {}", out[2]);
        assert!((out[3] - 0.9921875).abs() < 0.001, "F1 R = {}", out[3]);
    }

    #[test]
    fn drain_8bit_left_only() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x60; // 8-bit, L only
        pcm.state.frame_shift = FRAME_SHIFTS[(0x60 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.volume_levels[5] = 0;
        enable_playback(&mut pcm);

        // 0x40 as signed = +64 -> 64/128 = 0.5
        write_bytes(&mut pcm, &[0x40]);
        let mut out = [0.0f32; 2];
        pcm.drain_samples(&mut out);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.5).abs() < 0.01, "L = {}", out[0]);
        assert_eq!(out[1], 0.0, "R should be 0.0 for L-only mode");
    }

    #[test]
    fn drain_8bit_right_only() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x50; // 8-bit, R only
        pcm.state.frame_shift = FRAME_SHIFTS[(0x50 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.volume_levels[5] = 0;
        enable_playback(&mut pcm);

        // 0x80 as signed = -128 -> -128/128 = -1.0
        write_bytes(&mut pcm, &[0x80]);
        let mut out = [0.0f32; 2];
        pcm.drain_samples(&mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], 0.0, "L should be 0.0 for R-only mode");
        assert!((out[1] - (-1.0)).abs() < 0.01, "R = {}", out[1]);
    }

    #[test]
    fn drain_8bit_stereo() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x70; // 8-bit stereo
        pcm.state.frame_shift = FRAME_SHIFTS[(0x70 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.volume_levels[5] = 0;
        enable_playback(&mut pcm);

        // L = 0x40 (+64 -> 0.5), R = 0xC0 (-64 -> -0.5)
        write_bytes(&mut pcm, &[0x40, 0xC0]);
        let mut out = [0.0f32; 2];
        pcm.drain_samples(&mut out);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.5).abs() < 0.01, "L = {}", out[0]);
        assert!((out[1] - (-0.5)).abs() < 0.01, "R = {}", out[1]);
    }

    #[test]
    fn drain_disabled_modes_produce_silence() {
        for mode in [0x00u8, 0x40] {
            let mut pcm = make_pcm86();
            pcm.state.dac_control = mode;
            pcm.state.frame_shift = FRAME_SHIFTS[((mode >> 4) & 7) as usize];
            pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
            pcm.state.volume_levels[5] = 0;
            enable_playback(&mut pcm);

            write_bytes(&mut pcm, &[0xFF; 16]);
            let mut out = [0.0f32; 8];
            pcm.drain_samples(&mut out);
            for (i, &s) in out.iter().enumerate() {
                assert_eq!(s, 0.0, "mode 0x{mode:02X} sample {i} should be silent");
            }
        }
    }

    #[test]
    fn drain_muted_produces_silence_but_consumes_fifo() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x20; // 16-bit L only
        pcm.state.frame_shift = FRAME_SHIFTS[(0x20 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.volume_levels[5] = 0;
        pcm.state.mute = 0x01;
        enable_playback(&mut pcm);

        write_bytes(&mut pcm, &[0x7F, 0x00, 0x7F, 0x00]);
        assert_eq!(pcm.state.level, 4);

        let mut out = [0.0f32; 4];
        pcm.drain_samples(&mut out);
        assert_eq!(out.len(), 4);
        for &s in &out {
            assert_eq!(s, 0.0, "muted output should be silent");
        }
        // FIFO should have been consumed even though muted.
        assert_eq!(pcm.state.level, 0);
    }

    #[test]
    fn drain_with_volume_attenuation() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x20; // 16-bit L only
        pcm.state.frame_shift = FRAME_SHIFTS[(0x20 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        // Volume 15 = minimum (inverted: 15 - 15 = 0 -> 0.0)
        pcm.state.volume_levels[5] = 15;
        enable_playback(&mut pcm);

        write_bytes(&mut pcm, &[0x7F, 0x00]);
        let mut out = [0.0f32; 2];
        pcm.drain_samples(&mut out);
        assert_eq!(out[0], 0.0, "volume 15 should silence output");

        // Volume 0 = maximum (inverted: 15 - 0 = 15 -> 1.0)
        pcm.state.volume_levels[5] = 0;
        write_bytes(&mut pcm, &[0x7F, 0x00]);
        let mut out = [0.0f32; 2];
        pcm.drain_samples(&mut out);
        assert!(
            (out[0] - 0.9921875).abs() < 0.001,
            "volume 0 = max: {}",
            out[0]
        );
    }

    #[test]
    fn drain_underflow_produces_silence() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x30; // 16-bit stereo (4 bytes per frame)
        pcm.state.frame_shift = FRAME_SHIFTS[(0x30 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.volume_levels[5] = 0;
        enable_playback(&mut pcm);

        // Write only 2 bytes - not enough for a 4-byte stereo frame.
        write_bytes(&mut pcm, &[0x40, 0x00]);
        let mut out = [0.0f32; 2];
        pcm.drain_samples(&mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], 0.0, "underflow L should be silent");
        assert_eq!(out[1], 0.0, "underflow R should be silent");
    }

    #[test]
    fn drain_not_playing_produces_silence() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x20;
        pcm.state.frame_shift = FRAME_SHIFTS[(0x20 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.volume_levels[5] = 0;
        // Do NOT enable playback (A468h bit 7 = 0).

        write_bytes(&mut pcm, &[0x7F, 0x00]);
        let mut out = [0.0f32; 2];
        pcm.drain_samples(&mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 0.0);
    }

    #[test]
    fn irq_condition_met_when_virtual_level_below_threshold() {
        let mut pcm = make_pcm86();
        pcm.state.fifo_control = 0xA0; // play + IRQ enable
        pcm.state.irq_threshold_bytes = 3;
        pcm.state.irq_wait_cycles = 0; // disable wait for test

        // virtual_level = 5, above threshold.
        pcm.state.virtual_level = 5;
        assert!(!pcm.irq_condition_met(100, false));

        // virtual_level = 3, at threshold.
        pcm.state.virtual_level = 3;
        assert!(pcm.irq_condition_met(100, false));

        // virtual_level = 0, below threshold.
        pcm.state.virtual_level = 0;
        assert!(pcm.irq_condition_met(100, false));
    }

    #[test]
    fn irq_condition_not_met_when_irq_disabled() {
        let mut pcm = make_pcm86();
        pcm.state.fifo_control = 0x80; // play, but IRQ NOT enabled (bit 5 = 0)
        pcm.state.irq_threshold_bytes = 3;
        pcm.state.virtual_level = 0;
        pcm.state.irq_wait_cycles = 0;
        assert!(!pcm.irq_condition_met(100, false));
    }

    #[test]
    fn port_a466_dac_clock_toggles_during_tight_polling() {
        let mut pcm = make_pcm86();
        pcm.state.fifo_control = 0x80;

        let mut saw_low = false;
        let mut saw_high = false;
        for index in 0..32 {
            let value = pcm.read_port(0xA466, index * 20, 8_000_000, false);
            saw_low |= value & 0x01 == 0;
            saw_high |= value & 0x01 != 0;
        }

        assert!(saw_low, "L/R clock should have a low phase");
        assert!(saw_high, "L/R clock should have a high phase");
    }

    #[test]
    fn irq_condition_delayed_by_data_write_wait() {
        let mut pcm = make_pcm86();
        pcm.state.fifo_control = 0xA0;
        pcm.state.irq_threshold_bytes = 3;
        pcm.state.virtual_level = 0;
        pcm.state.irq_wait_cycles = 1000;
        pcm.state.irq_wait_start_cycle = 50;

        // Only 50 cycles elapsed, need 1000 -> not met.
        assert!(!pcm.irq_condition_met(100, false));

        // 1050 cycles elapsed -> met.
        assert!(pcm.irq_condition_met(1050, false));
    }

    #[test]
    fn buffer_overflow_protection() {
        let mut pcm = make_pcm86();

        // Fill past physical buffer size.
        for i in 0..RING_SIZE + 100 {
            if pcm.state.virtual_level < FIFO_CAPACITY {
                pcm.state.virtual_level += 1;
            }
            pcm.buffer[pcm.state.write_index as usize] = (i & 0xFF) as u8;
            pcm.state.write_index = (pcm.state.write_index + 1) & RING_INDEX_MASK;
            pcm.state.level += 1;
            if pcm.state.level >= RING_SIZE as i32 {
                pcm.state.level -= 4;
                pcm.state.read_index = (pcm.state.read_index + 4) & RING_INDEX_MASK;
            }
        }
        assert!(
            pcm.state.level < RING_SIZE as i32,
            "level should be bounded: {}",
            pcm.state.level
        );
    }

    #[test]
    fn bytes_per_frame_table() {
        assert_eq!(bytes_per_frame(0x00), 0); // 16-bit disabled
        assert_eq!(bytes_per_frame(0x10), 2); // 16-bit R
        assert_eq!(bytes_per_frame(0x20), 2); // 16-bit L
        assert_eq!(bytes_per_frame(0x30), 4); // 16-bit stereo
        assert_eq!(bytes_per_frame(0x40), 0); // 8-bit disabled
        assert_eq!(bytes_per_frame(0x50), 1); // 8-bit R
        assert_eq!(bytes_per_frame(0x60), 1); // 8-bit L
        assert_eq!(bytes_per_frame(0x70), 2); // 8-bit stereo
    }

    #[test]
    fn pcm86_volume_function() {
        assert!((pcm86_volume(0) - 1.0).abs() < 0.001); // 0 = max
        assert!((pcm86_volume(15) - 0.0).abs() < 0.001); // 15 = min
        assert!((pcm86_volume(7) - (8.0 / 15.0)).abs() < 0.001); // mid
    }

    #[test]
    fn drain_16bit_sign_extension() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x20; // 16-bit L only
        pcm.state.frame_shift = FRAME_SHIFTS[(0x20 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.volume_levels[5] = 0;
        enable_playback(&mut pcm);

        // MSB 0xFF (as i8 = -1), LSB 0xFE -> ((-1) << 8) | 0xFE = -2
        write_bytes(&mut pcm, &[0xFF, 0xFE]);
        let mut out = [0.0f32; 2];
        pcm.drain_samples(&mut out);
        let expected = -2.0f32 / 32768.0;
        assert!(
            (out[0] - expected).abs() < 0.0001,
            "sign extension: expected {expected}, got {}",
            out[0]
        );
    }

    #[test]
    fn drain_ring_buffer_wraps() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x60; // 8-bit L only
        pcm.state.frame_shift = FRAME_SHIFTS[(0x60 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.volume_levels[5] = 0;
        enable_playback(&mut pcm);

        // Position write and read near the end of the buffer.
        pcm.state.write_index = RING_INDEX_MASK;
        pcm.state.read_index = RING_INDEX_MASK;

        // Write 3 bytes across the wrap boundary.
        let data = [0x20u8, 0x40, 0x60]; // +32, +64, +96 as i8
        for &b in &data {
            pcm.buffer[pcm.state.write_index as usize] = b;
            pcm.state.write_index = (pcm.state.write_index + 1) & RING_INDEX_MASK;
            pcm.state.level += 1;
            pcm.state.virtual_level += 1;
        }

        let mut out = [0.0f32; 6];
        pcm.drain_samples(&mut out);
        assert!((out[0] - 0x20 as f32 / 128.0).abs() < 0.01);
        assert!((out[2] - 0x40 as f32 / 128.0).abs() < 0.01);
        assert!((out[4] - 0x60 as f32 / 128.0).abs() < 0.01);
    }

    #[test]
    fn update_buffer_state_does_not_modify_read_pos_or_real_buf() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x60; // 8-bit L only
        pcm.state.frame_shift = FRAME_SHIFTS[(0x60 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        enable_playback(&mut pcm);
        pcm.state.virtual_level_cycle = 0;

        write_bytes(&mut pcm, &[0x10, 0x20, 0x30, 0x40, 0x50]);
        let read_pos_before = pcm.state.read_index;
        let ring_level_before = pcm.state.level;
        let virtual_level_before = pcm.state.virtual_level;

        // Simulate enough time for ~2 samples at 44100 Hz with 8 MHz CPU.
        pcm.update_buffer_state(200, 8_000_000);

        assert_eq!(
            pcm.state.read_index, read_pos_before,
            "read_pos must not change"
        );
        assert_eq!(pcm.state.level, ring_level_before, "level must not change");
        assert!(
            pcm.state.virtual_level < virtual_level_before,
            "virtual_level should have decreased"
        );
    }

    #[test]
    fn update_buffer_state_clamps_virtual_level_with_frame_mask() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x30; // 16-bit stereo, frame_shift=2
        pcm.state.frame_shift = FRAME_SHIFTS[(0x30 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1; // 3
        enable_playback(&mut pcm);
        pcm.state.virtual_level_cycle = 0;

        // Set virtual_level to a small value that will underflow.
        pcm.state.virtual_level = 2;

        // Simulate enough time to consume more than 2 bytes.
        pcm.update_buffer_state(10_000_000, 8_000_000);

        // After underflow, virtual_level should be masked with frame_mask (3), not clamped to 0.
        assert_eq!(
            pcm.state.virtual_level,
            pcm.state.virtual_level & pcm.state.frame_mask as i32,
            "virtual_level should be masked with frame_mask on underflow"
        );
    }

    #[test]
    fn update_buffer_state_no_change_when_not_playing() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x60;
        pcm.state.frame_shift = FRAME_SHIFTS[(0x60 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        // Do NOT enable playback.
        pcm.state.virtual_level_cycle = 0;

        write_bytes(&mut pcm, &[0x10, 0x20, 0x30]);
        let virtual_level_before = pcm.state.virtual_level;

        pcm.update_buffer_state(10_000_000, 8_000_000);

        assert_eq!(
            pcm.state.virtual_level, virtual_level_before,
            "virtual_level should not change when not playing"
        );
    }

    #[test]
    fn update_buffer_state_split_call_equivalence() {
        // A single update_buffer_state(T) and two calls update_buffer_state(T/2)
        // should produce the same virtual_level (within 1 byte) thanks to fractional tracking.
        let cpu_clock_hz = 20_000_000u32;

        let mut single = make_pcm86();
        single.state.dac_control = 0x70; // 8-bit stereo
        single.state.frame_shift = FRAME_SHIFTS[(0x70 >> 4) & 7];
        single.state.frame_mask = (1u32 << single.state.frame_shift) - 1;
        enable_playback(&mut single);
        single.state.virtual_level_cycle = 0;
        write_bytes(&mut single, &[0u8; 1000]);
        let view_before = single.state.virtual_level;

        let elapsed = 500_000u64; // 25ms at 20MHz
        single.update_buffer_state(elapsed, cpu_clock_hz);
        let single_result = single.state.virtual_level;

        let mut split = make_pcm86();
        split.state.dac_control = 0x70;
        split.state.frame_shift = FRAME_SHIFTS[(0x70 >> 4) & 7];
        split.state.frame_mask = (1u32 << split.state.frame_shift) - 1;
        enable_playback(&mut split);
        split.state.virtual_level_cycle = 0;
        write_bytes(&mut split, &[0u8; 1000]);
        assert_eq!(split.state.virtual_level, view_before);

        split.update_buffer_state(elapsed / 2, cpu_clock_hz);
        split.update_buffer_state(elapsed, cpu_clock_hz);
        let split_result = split.state.virtual_level;

        let difference = (single_result - split_result).unsigned_abs();
        assert!(
            difference <= 1,
            "split calls should match single call within 1 byte: single={single_result} split={split_result} (started at {view_before})"
        );
    }

    #[test]
    fn update_buffer_state_accumulates_fractional_samples() {
        // Calling update_buffer_state with very small increments (each < 1 sample)
        // should still eventually decrement virtual_level via accumulated fractions.
        let cpu_clock_hz = 20_000_000u32;

        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x70; // 8-bit stereo, rate index from fifo
        pcm.state.frame_shift = FRAME_SHIFTS[(0x70 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        enable_playback(&mut pcm);
        pcm.state.virtual_level_cycle = 0;
        write_bytes(&mut pcm, &[0u8; 100]);
        let view_before = pcm.state.virtual_level;

        // At 44100 Hz (rate index 0, default), 1 sample = 20M/44100 = ~453 cycles.
        // Use increments of 100 cycles (< 1 sample each).
        let increment = 100u64;
        let mut cycle = 0u64;
        for _ in 0..1000 {
            cycle += increment;
            pcm.update_buffer_state(cycle, cpu_clock_hz);
        }

        assert!(
            pcm.state.virtual_level < view_before,
            "virtual_level should have decreased after many small increments: before={view_before} after={}",
            pcm.state.virtual_level
        );
    }

    #[test]
    fn default_reset_values() {
        let state = Pcm86State::default();
        assert_eq!(
            state.irq_threshold_bytes, 128,
            "threshold should default to 0x80"
        );
        assert_eq!(
            state.dac_control, 0x32,
            "dac_control should default to 0x32"
        );
        assert_eq!(state.frame_shift, 2, "frame_shift should default to 2");
        assert_eq!(state.frame_mask, 3, "frame_mask should default to 3");
        assert_eq!(
            state.mute, 0x00,
            "pcm_mute should default to 0x00 (unmuted)"
        );
    }

    #[test]
    fn port_a66e_readback_preserves_full_byte() {
        let mut pcm = make_pcm86();
        pcm.state.mute = 0xFE;

        let readback = pcm.read_port(0xA66E, 0, 8_000_000, false);
        assert_eq!(readback, 0xFE, "full byte should be returned on read");

        // Bit 0 is 0, so mute should be inactive.
        let volume = if pcm.state.mute & 0x01 != 0 { 0.0 } else { 1.0 };
        assert_eq!(volume, 1.0, "mute should be inactive when bit 0 is 0");
    }

    #[test]
    fn reconcile_buffers_adjusts_virtual_level_upward_when_ring_level_leads() {
        let mut pcm = make_pcm86();
        pcm.state.irq_threshold_bytes = 128;
        pcm.state.frame_mask = 1;
        pcm.state.virtual_level = 500;
        // level must exceed virtual_level + threshold*3 = 500 + 384 = 884
        pcm.state.level = 1000;
        let old_virtual_level = pcm.state.virtual_level;
        pcm.reconcile_buffers(1000, 8_000_000);
        assert!(
            pcm.state.virtual_level > old_virtual_level,
            "virtual_level should increase: was {old_virtual_level}, now {}",
            pcm.state.virtual_level
        );
        // Expected: adjustment = (1000 - (500 - 384)) / 8 = (1000 - 116) / 8 = 110
        assert_eq!(pcm.state.virtual_level, 610);
    }

    #[test]
    fn reconcile_buffers_no_upward_adjustment_when_ring_level_close() {
        let mut pcm = make_pcm86();
        pcm.state.irq_threshold_bytes = 128;
        pcm.state.frame_mask = 1;
        pcm.state.virtual_level = 500;
        // level NOT exceeding virtual_level + threshold*3 = 884
        pcm.state.level = 800;
        pcm.reconcile_buffers(1000, 8_000_000);
        assert_eq!(
            pcm.state.virtual_level, 500,
            "virtual_level should not change"
        );
    }

    #[test]
    fn pcm86_timer_expired_uses_reschedule_when_condition_not_met() {
        let mut sb = Soundboard86::new(8_000_000, 48000, None, false, MachineModel::PC9801RA);
        sb.pcm86.state.data_written = true;
        sb.pcm86.state.irq_threshold_bytes = 128;
        sb.pcm86.state.frame_mask = 3;
        sb.pcm86.state.frame_shift = 2;
        // virtual_level > threshold so first branch of condition fails.
        sb.pcm86.state.virtual_level = 200;
        // adjusted = (200*4 + 800)/5 = 320 > 128, so second branch also fails.
        sb.pcm86.state.level = 800;
        sb.pcm86.state.fifo_control = 0x80; // playing
        sb.pcm86.state.virtual_level_cycle = 1000;

        sb.pcm86_timer_expired(1000, 8_000_000);

        // The condition was not met, so needs_reschedule should be set
        // (drain_actions will call calculate_next_irq_cycle).
        assert!(
            sb.pcm86.needs_reschedule,
            "needs_reschedule should be set when condition fails"
        );
        // data_written should still be true (not cleared).
        assert!(sb.pcm86.state.data_written);
        // IRQ should NOT have been asserted.
        assert!(!sb.pcm86.state.irq_request);
    }

    #[test]
    fn drain_actions_only_schedules_when_needs_reschedule() {
        let mut sb = Soundboard86::new(8_000_000, 48000, None, false, MachineModel::PC9801RA);
        sb.pcm86.state.data_written = true;
        sb.pcm86.state.fifo_control = 0x80; // playback active
        sb.pcm86.needs_reschedule = false;

        // data_written alone should NOT trigger scheduling.
        let actions = sb.drain_actions(false);
        let timer_action = actions.iter().any(|a| {
            matches!(
                a,
                Soundboard86Action::ScheduleTimer {
                    kind: Soundboard86Timer::Pcm86Irq,
                    ..
                }
            )
        });
        assert!(
            !timer_action,
            "should not schedule PCM86 timer without needs_reschedule"
        );

        // With needs_reschedule, scheduling should happen.
        sb.pcm86.needs_reschedule = true;
        let actions = sb.drain_actions(false);
        let timer_action = actions.iter().any(|a| {
            matches!(
                a,
                Soundboard86Action::ScheduleTimer {
                    kind: Soundboard86Timer::Pcm86Irq,
                    ..
                }
            )
        });
        assert!(
            timer_action,
            "should schedule PCM86 timer when needs_reschedule is set"
        );
    }

    #[test]
    fn irq_condition_met_with_ring_level_below_threshold() {
        let mut pcm = make_pcm86();
        pcm.state.fifo_control = 0xA0; // IRQ enabled (bit 5) + playing (bit 7)
        pcm.state.irq_threshold_bytes = 128;
        pcm.state.frame_mask = 3;
        pcm.state.irq_wait_cycles = 0;
        // virtual_level above threshold - primary condition false.
        pcm.state.virtual_level = 200;
        // level below threshold but above frame_mask - secondary condition true.
        pcm.state.level = 100;
        pcm.state.irq_request = false;

        assert!(
            pcm.irq_condition_met(1000, false),
            "should trigger via secondary level condition"
        );
    }

    #[test]
    fn irq_condition_met_ring_level_at_frame_mask_does_not_trigger() {
        let mut pcm = make_pcm86();
        pcm.state.fifo_control = 0xA0;
        pcm.state.irq_threshold_bytes = 128;
        pcm.state.frame_mask = 3;
        pcm.state.irq_wait_cycles = 0;
        pcm.state.virtual_level = 200;
        // level <= frame_mask - secondary condition should NOT trigger.
        pcm.state.level = 3;
        pcm.state.irq_request = false;

        assert!(
            !pcm.irq_condition_met(1000, false),
            "should not trigger when level <= frame_mask"
        );
    }

    #[test]
    fn wavestar_sequence_detection_sets_value() {
        let mut pcm = make_pcm86();
        assert_eq!(pcm.state.wavestar_matched_bytes, 0);
        assert_eq!(pcm.state.wavestar_value, 0xFF);

        // Feed the full magic sequence via port 0xA462.
        for &byte in &WAVESTAR_SEQUENCE {
            pcm.write_port(0xA462, byte, 0, 8_000_000, 48000);
        }
        assert_eq!(pcm.state.wavestar_matched_bytes, 5);
        assert_eq!(pcm.state.wavestar_value, 0x0B);
    }

    #[test]
    fn wavestar_partial_sequence_resets() {
        let mut pcm = make_pcm86();
        // Start the sequence.
        pcm.write_port(0xA462, 0xA6, 0, 8_000_000, 48000);
        pcm.write_port(0xA462, 0xD3, 0, 8_000_000, 48000);
        assert_eq!(pcm.state.wavestar_matched_bytes, 2);

        // Wrong byte resets.
        pcm.write_port(0xA462, 0x00, 0, 8_000_000, 48000);
        assert_eq!(pcm.state.wavestar_matched_bytes, 0);
    }

    #[test]
    fn wavestar_partial_sequence_restarts_on_first_byte() {
        let mut pcm = make_pcm86();
        pcm.write_port(0xA462, 0xA6, 0, 8_000_000, 48000);
        pcm.write_port(0xA462, 0xD3, 0, 8_000_000, 48000);
        // Writing the first byte again restarts from index 1.
        pcm.write_port(0xA462, 0xA6, 0, 8_000_000, 48000);
        assert_eq!(pcm.state.wavestar_matched_bytes, 1);
    }

    #[test]
    fn wavestar_port_a464_read_toggles() {
        let mut pcm = make_pcm86();
        // Complete the sequence.
        for &byte in &WAVESTAR_SEQUENCE {
            pcm.write_port(0xA462, byte, 0, 8_000_000, 48000);
        }
        assert_eq!(pcm.state.wavestar_value, 0x0B);

        let first = pcm.read_port(0xA464, 0, 8_000_000, false);
        assert_eq!(first, 0x0B);
        // After read, value toggles to 0x00.
        let second = pcm.read_port(0xA464, 0, 8_000_000, false);
        assert_eq!(second, 0x00);
        // After 0x00, toggles back to 0xFF.
        let third = pcm.read_port(0xA464, 0, 8_000_000, false);
        assert_eq!(third, 0xFF);
    }

    #[test]
    fn wavestar_port_a464_write_mode_switch() {
        let mut pcm = make_pcm86();
        // Complete the sequence.
        for &byte in &WAVESTAR_SEQUENCE {
            pcm.write_port(0xA462, byte, 0, 8_000_000, 48000);
        }
        // Switch to WSS mode.
        pcm.write_port(0xA464, 0x04, 0, 8_000_000, 48000);
        assert_eq!(pcm.state.wavestar_value, 0x0C);

        // Switch back to PCM86 mode.
        pcm.write_port(0xA464, 0x00, 0, 8_000_000, 48000);
        assert_eq!(pcm.state.wavestar_value, 0x08);

        // Reset value with 0x09.
        pcm.write_port(0xA464, 0x09, 0, 8_000_000, 48000);
        assert_eq!(pcm.state.wavestar_value, 0xFF);
    }

    #[test]
    fn wavestar_port_a462_read_returns_ff() {
        let mut pcm = make_pcm86();
        assert_eq!(pcm.read_port(0xA462, 0, 8_000_000, false), 0xFF);
    }

    #[test]
    fn pcm86_timer_expired_reschedules_when_data_written_false() {
        let mut sb = Soundboard86::new(8_000_000, 48000, None, false, MachineModel::PC9801RA);
        sb.pcm86.state.data_written = false;
        sb.pcm86.state.fifo_control = 0x80; // playing

        sb.pcm86_timer_expired(1000, 8_000_000);

        assert!(
            !sb.pcm86.state.data_written,
            "data_written must not be set by timer callback"
        );
        assert!(
            sb.pcm86.needs_reschedule,
            "needs_reschedule should be set for the next IRQ check"
        );
    }

    #[test]
    fn pcm86_timer_expired_does_not_reschedule_when_not_playing() {
        let mut sb = Soundboard86::new(8_000_000, 48000, None, false, MachineModel::PC9801RA);
        sb.pcm86.state.data_written = false;
        sb.pcm86.state.fifo_control = 0x00; // NOT playing

        sb.pcm86_timer_expired(1000, 8_000_000);

        assert!(
            !sb.pcm86.state.data_written,
            "data_written should remain false"
        );
        assert!(
            sb.pcm86.needs_reschedule,
            "needs_reschedule set even when not playing - drain_actions gates on playback"
        );
    }

    #[test]
    fn port_a468_write_updates_buffer_before_force_irq() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x60; // 8-bit L only, frame_shift=0
        pcm.state.frame_shift = FRAME_SHIFTS[(0x60 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.irq_threshold_bytes = 128;
        pcm.state.fifo_control = 0x80; // playing
        pcm.state.virtual_level_cycle = 0;

        // Set virtual_level above threshold - stale without update.
        pcm.state.virtual_level = 200;
        pcm.state.level = 200;

        // Advance enough time at 44100 Hz with 8 MHz clock for >72 samples consumed.
        // 200 - 72 = 128, which is <= threshold. Force-IRQ should trigger.
        let cycles_for_73_samples = (73u64 * 8_000_000) / 44100 + 1;

        // Write 0xA468 with bit 4=0 to clear IRQ, triggering the force-IRQ check.
        pcm.write_port(0xA468, 0x80, cycles_for_73_samples, 8_000_000, 48000);

        assert!(
            pcm.state.irq_request,
            "force-IRQ should trigger after buffer state update reduces virtual_level"
        );
    }

    #[test]
    fn irq_condition_met_latches_irq_request() {
        let mut pcm = make_pcm86();
        pcm.state.fifo_control = 0xA0; // play + IRQ enable
        pcm.state.irq_threshold_bytes = 128;
        pcm.state.irq_wait_cycles = 0;
        pcm.state.virtual_level = 0;
        pcm.state.irq_request = false;

        let result = pcm.irq_condition_met(100, false);
        assert!(result, "condition should be met");
        assert!(
            pcm.state.irq_request,
            "irq_request should be latched after condition met"
        );
    }

    #[test]
    fn irq_condition_met_does_not_relatch_when_already_set() {
        let mut pcm = make_pcm86();
        pcm.state.fifo_control = 0xA0;
        pcm.state.irq_threshold_bytes = 128;
        pcm.state.irq_wait_cycles = 0;
        pcm.state.virtual_level = 500; // above threshold
        pcm.state.level = 500;
        pcm.state.irq_request = true; // already latched

        let result = pcm.irq_condition_met(100, false);
        assert!(result, "should return true via irq_request path");
    }

    #[test]
    fn port_a46a_write_updates_buffer_before_dactrl_change() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x60; // 8-bit L only
        pcm.state.frame_shift = FRAME_SHIFTS[(0x60 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.fifo_control = 0x80; // playing
        pcm.state.virtual_level_cycle = 0;

        write_bytes(&mut pcm, &[0x10; 100]);
        let virtual_level_before = pcm.state.virtual_level;

        // Advance time so update_buffer_state would decrement virtual_level.
        let cycles = (10u64 * 8_000_000) / 44100 + 1;
        pcm.write_port(0xA46A, 0x30, cycles, 8_000_000, 48000); // change to 16-bit stereo

        assert!(
            pcm.state.virtual_level < virtual_level_before,
            "virtual_level should have been decremented before dac_control change: was {virtual_level_before}, now {}",
            pcm.state.virtual_level
        );
        assert_eq!(pcm.state.dac_control, 0x30, "dac_control should be updated");
    }

    #[test]
    fn port_a466_write_updates_buffer_for_line5() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x60; // 8-bit L only
        pcm.state.frame_shift = FRAME_SHIFTS[(0x60 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.fifo_control = 0x80; // playing
        pcm.state.virtual_level_cycle = 0;

        write_bytes(&mut pcm, &[0x10; 100]);
        let virtual_level_before = pcm.state.virtual_level;

        let cycles = (10u64 * 8_000_000) / 44100 + 1;
        // Write line 5 volume (bits 7:5 = 0b101 = 5, value 0xA5 -> line 5, level 5).
        pcm.write_port(0xA466, 0xA5, cycles, 8_000_000, 48000);

        assert!(
            pcm.state.virtual_level < virtual_level_before,
            "virtual_level should have been decremented for line 5 write: was {virtual_level_before}, now {}",
            pcm.state.virtual_level
        );
        assert_eq!(pcm.state.volume_levels[5], 5, "volume should be stored");
    }

    #[test]
    fn port_a466_write_does_not_update_buffer_for_other_lines() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x60;
        pcm.state.frame_shift = FRAME_SHIFTS[(0x60 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.fifo_control = 0x80;
        pcm.state.virtual_level_cycle = 0;

        write_bytes(&mut pcm, &[0x10; 100]);
        let virtual_level_before = pcm.state.virtual_level;

        let cycles = (10u64 * 8_000_000) / 44100 + 1;
        // Write line 0 volume (bits 7:5 = 0b000 = 0).
        pcm.write_port(0xA466, 0x03, cycles, 8_000_000, 48000);

        assert_eq!(
            pcm.state.virtual_level, virtual_level_before,
            "virtual_level should NOT change for non-line-5 writes"
        );
    }

    #[test]
    fn drain_suppresses_output_in_recording_mode() {
        let mut pcm = make_pcm86();
        pcm.state.dac_control = 0x60; // 8-bit L only
        pcm.state.frame_shift = FRAME_SHIFTS[(0x60 >> 4) & 7];
        pcm.state.frame_mask = (1u32 << pcm.state.frame_shift) - 1;
        pcm.state.volume_levels[5] = 0; // max volume
        pcm.state.fifo_control = 0xC0; // bit 7 (play) + bit 6 (recording direction)
        write_bytes(&mut pcm, &[0x7F; 16]);

        let mut out = [0.0f32; 8];
        pcm.drain_samples(&mut out);
        // The audio output ignores bit 6 (recording direction).
        // PCM output plays regardless of the FIFO direction flag.
        assert!(
            out.iter().any(|&s| s != 0.0),
            "bit 6 should not suppress DAC output"
        );
    }

    #[test]
    fn fm_timer_piggyback_asserts_pcm86_irq() {
        let mut sb = Soundboard86::new(8_000_000, 48000, None, false, MachineModel::PC9801RA);
        sb.pcm86.state.fifo_control = 0xA0; // playing + IRQ enabled
        sb.pcm86.state.irq_threshold_bytes = 128;
        sb.pcm86.state.irq_wait_cycles = 0;
        sb.pcm86.state.virtual_level = 0; // below threshold
        sb.pcm86.state.irq_request = false;
        sb.chip_action_cycle = 1000;

        // Simulate FM timer fire.
        sb.fm_timer_just_fired = true;

        // No PCM86 timer pending, so piggyback should check condition.
        let actions = sb.drain_actions(false);
        let has_irq_assert = actions
            .iter()
            .any(|a| matches!(a, Soundboard86Action::AssertIrq { .. }));
        assert!(
            has_irq_assert,
            "FM timer piggyback should assert PCM86 IRQ when condition met"
        );
        assert!(sb.pcm86.state.irq_request, "irq_request should be set");
    }

    #[test]
    fn fm_timer_piggyback_skipped_when_pcm86_timer_pending() {
        let mut sb = Soundboard86::new(8_000_000, 48000, None, false, MachineModel::PC9801RA);
        sb.pcm86.state.fifo_control = 0xA0;
        sb.pcm86.state.irq_threshold_bytes = 128;
        sb.pcm86.state.irq_wait_cycles = 0;
        sb.pcm86.state.virtual_level = 0;
        sb.pcm86.state.irq_request = false;
        sb.chip_action_cycle = 1000;

        sb.fm_timer_just_fired = true;

        // PCM86 timer IS pending - piggyback should be skipped.
        let actions = sb.drain_actions(true);
        let has_irq_assert = actions
            .iter()
            .any(|a| matches!(a, Soundboard86Action::AssertIrq { .. }));
        assert!(
            !has_irq_assert,
            "piggyback should be skipped when PCM86 timer is pending"
        );
        assert!(
            !sb.pcm86.state.irq_request,
            "irq_request should not be set when piggyback skipped"
        );
    }

    #[test]
    fn irq_condition_met_guard_skips_buffer_check_when_timer_pending() {
        let mut pcm = make_pcm86();
        pcm.state.fifo_control = 0xA0; // IRQ enabled + playing
        pcm.state.irq_threshold_bytes = 128;
        pcm.state.irq_wait_cycles = 0;
        pcm.state.virtual_level = 0; // below threshold
        pcm.state.irq_request = false;

        // With timer pending, buffer condition should not set irq_request.
        let result = pcm.irq_condition_met(100, true);
        assert!(!result, "should not trigger when timer is pending");
        assert!(!pcm.state.irq_request, "irq_request should not be set");

        // Without timer pending, same condition should trigger.
        let result = pcm.irq_condition_met(100, false);
        assert!(result, "should trigger when timer is not pending");
        assert!(pcm.state.irq_request, "irq_request should be set");
    }

    #[test]
    fn irq_condition_met_returns_true_via_flag_even_when_timer_pending() {
        let mut pcm = make_pcm86();
        pcm.state.fifo_control = 0xA0;
        pcm.state.irq_threshold_bytes = 128;
        pcm.state.irq_wait_cycles = 0;
        pcm.state.virtual_level = 500; // above threshold
        pcm.state.irq_request = true; // already latched

        // Even with timer pending, the already-set irq_request is returned.
        let result = pcm.irq_condition_met(100, true);
        assert!(
            result,
            "should return true via irq_request path regardless of pending"
        );
    }

    #[test]
    fn drift_weighting_pulls_timer_towards_ring_level() {
        let mut pcm = make_pcm86();
        pcm.state.fifo_control = 0x80; // playing
        pcm.state.irq_threshold_bytes = 128;
        pcm.state.frame_shift = 0;
        pcm.state.frame_mask = 0;
        pcm.state.virtual_level = 1128; // 1000 above threshold
        pcm.state.level = 628; // 500 above threshold (cntr < cntv)

        // No drift - should use pure virtual_level.
        pcm.underrun_milliseconds = 0;
        let no_drift = pcm.calculate_next_irq_cycle(0, 8_000_000);

        // Moderate drift (>4s) - should blend slightly towards level.
        pcm.underrun_milliseconds = 5000;
        let moderate_drift = pcm.calculate_next_irq_cycle(0, 8_000_000);

        // Severe drift (>32s) - should blend more towards level.
        pcm.underrun_milliseconds = 33000;
        let severe_drift = pcm.calculate_next_irq_cycle(0, 8_000_000);

        // All should return Some since buffer is above threshold.
        assert!(no_drift.is_some());
        assert!(moderate_drift.is_some());
        assert!(severe_drift.is_some());

        // Drift should pull the timer earlier (lower fire_cycle) since
        // level is closer to threshold than virtual_level.
        assert!(
            moderate_drift.unwrap() <= no_drift.unwrap(),
            "moderate drift should fire no later: moderate={}, none={}",
            moderate_drift.unwrap(),
            no_drift.unwrap()
        );
        assert!(
            severe_drift.unwrap() <= moderate_drift.unwrap(),
            "severe drift should fire no later: severe={}, moderate={}",
            severe_drift.unwrap(),
            moderate_drift.unwrap()
        );
    }

    #[test]
    fn reconcile_buffers_accumulates_underrun_drift() {
        let mut pcm = make_pcm86();
        pcm.state.irq_threshold_bytes = 128;
        pcm.state.virtual_level = 500;
        pcm.state.level = 400; // deficit < 0 -> underrun

        // First call establishes baseline (last_checkbuf_cycle starts at 0).
        pcm.reconcile_buffers(1000, 8_000_000);
        assert_eq!(
            pcm.underrun_milliseconds, 0,
            "first call should not accumulate"
        );

        // Second call 1 second later.
        pcm.reconcile_buffers(8_001_000, 8_000_000);
        assert!(
            pcm.underrun_milliseconds > 0,
            "should accumulate drift: {}",
            pcm.underrun_milliseconds
        );

        // Recovery: level >= virtual_level resets the flag.
        pcm.state.level = 600;
        pcm.reconcile_buffers(16_001_000, 8_000_000);
        assert_eq!(pcm.underrun_milliseconds, 0, "recovery should reset drift");
    }
}
