//! Shared OPN/OPNA FM audio driver.
//!
//! Encapsulates the machinery common to every YMFM OPN-family sound device:
//! clocking the chip up to the current cycle on each port access
//! ([`OpnFm::sync`]), busy-flag timing, FM-timer scheduling, coalesced IRQ-edge
//! reporting, and FIR resampling of the native output to the device rate. The
//! board-specific port maps, IRQ routing, and any extra subsystems (PCM86,
//! ADPCM control, mailboxes) stay in the individual board wrappers.
//!
//! The driver is generic over [`OpnChip`], implemented here for the YM2203
//! (mono OPN) and YM2608 (stereo OPNA). It is decoupled from any scheduler
//! enum or interrupt controller: [`OpnFm::drain_timers`] returns
//! [`FmTimerAction`]s keyed by `timer_id`, and [`OpnFm::take_irq_change`]
//! reports the chip IRQ edge, leaving each consumer to map those onto its own
//! scheduler events and IRQ wiring.

use resampler::{Attenuation, Latency, ResamplerFir};
pub use ymfm_oxide::Ym2203;
use ymfm_oxide::{Ym2608, YmfmOpnFidelity, YmfmOutput3, YmfmOutput4, YmfmTimerUpdate};

const FIDELITY: YmfmOpnFidelity = YmfmOpnFidelity::Max;
const RESAMPLER_LATENCY: Latency = Latency::Sample64;
const RESAMPLER_ATTENUATION: Attenuation = Attenuation::Db60;

/// Initial scratch capacity (native samples) for the generation buffers.
const INITIAL_NATIVE_CAPACITY: usize = 4096;

/// YM2608 ADPCM-A rhythm ROM size (`ym2608.rom`).
pub const RHYTHM_ROM_SIZE: usize = 8192;

/// Algorithmically generated YM2608 ADPCM-A rhythm ROM (8 KB).
///
/// Functional equivalent of the original chip samples with completely different
/// binary content, produced by an evolutionary algorithm. Shared by every OPNA
/// consumer that does not supply its own rhythm data.
pub static EVOLVED_RHYTHM_ROM: &[u8; RHYTHM_ROM_SIZE] =
    include_bytes!("../../../utils/rhythm/rhythm.bin");

/// Fractional sample remainder for drift-free FM sample-count accumulation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FmSampleRemainder(pub f64);

impl Eq for FmSampleRemainder {}

impl Default for FmSampleRemainder {
    fn default() -> Self {
        Self(0.0)
    }
}

/// FM timer scheduling request emitted by [`OpnFm::drain_timers`].
///
/// `timer_id` is 0 for timer A and 1 for timer B; each consumer maps it to its
/// own scheduler event kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FmTimerAction {
    /// Schedule the timer to fire at `fire_cycle` (CPU-clock units).
    Schedule {
        /// 0 = timer A, 1 = timer B.
        timer_id: u8,
        /// CPU cycle at which the timer should fire.
        fire_cycle: u64,
    },
    /// Cancel a previously scheduled timer.
    Cancel {
        /// 0 = timer A, 1 = timer B.
        timer_id: u8,
    },
}

/// A YMFM OPN-family chip driven by [`OpnFm`].
///
/// The hi-bank methods default to no-op / open-bus and are only meaningful on
/// the YM2608 (OPNA). `mix_sample` writes [`OpnChip::CHANNELS`] float samples,
/// encoding the per-chip output scaling.
pub trait OpnChip {
    /// Native output sample type produced by `generate`.
    type Native: Copy;

    /// Chip input clock in Hz.
    const CLOCK: u32;

    /// Number of output channels written by `mix_sample` (1 mono, 2 stereo).
    const CHANNELS: usize;

    /// Creates a reset chip at the driver fidelity.
    fn create() -> Self;

    /// Returns a zeroed native sample (used to grow scratch buffers).
    fn native_zero() -> Self::Native;

    /// Returns the native output sample rate for the given input clock.
    fn sample_rate(&mut self, clock: u32) -> u32;

    /// Generates native samples into `out`.
    fn generate(&mut self, out: &mut [Self::Native]);

    /// Mixes one native sample into `out` (`CHANNELS` floats), scaled to f32.
    fn mix_sample(sample: &Self::Native, out: &mut [f32]);

    /// Reads the low-bank status register.
    fn read_status(&mut self, busy: bool) -> u8;

    /// Reads data from the currently addressed low-bank register.
    fn read_data(&mut self) -> u8;

    /// Latches the low-bank register address, returning busy clocks.
    fn write_address(&mut self, value: u8) -> u32;

    /// Writes the addressed low-bank register, returning busy clocks.
    fn write_data(&mut self, value: u8) -> u32;

    /// Latches the high-bank register address, returning busy clocks.
    fn write_address_hi(&mut self, _value: u8) -> u32 {
        0
    }

    /// Writes the addressed high-bank register, returning busy clocks.
    fn write_data_hi(&mut self, _value: u8) -> u32 {
        0
    }

    /// Reads the high-bank status register.
    fn read_status_hi(&mut self, _busy: bool) -> u8 {
        0xFF
    }

    /// Reads data from the currently addressed high-bank register.
    fn read_data_hi(&mut self) -> u8 {
        0xFF
    }

    /// Notifies the chip that timer `timer_id` has expired.
    fn timer_expired(&mut self, timer_id: u32);

    /// Sets the value read back from an SSG parallel I/O input port (port 0 is
    /// A, port 1 is B). No-op on chips without an SSG I/O port.
    fn set_io_input(&mut self, _port: u8, _value: u8) {}

    /// Returns and clears the pending timer update for `timer_id`.
    fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate>;

    /// Returns and clears the pending IRQ-output edge.
    fn take_irq_update(&mut self) -> Option<bool>;
}

impl OpnChip for Ym2203 {
    type Native = YmfmOutput4;
    const CLOCK: u32 = 3_993_600;
    const CHANNELS: usize = 1;

    fn create() -> Self {
        let mut chip = Ym2203::new();
        chip.reset();
        chip.set_fidelity(FIDELITY);
        chip
    }

    fn native_zero() -> Self::Native {
        YmfmOutput4 { data: [0; 4] }
    }

    fn sample_rate(&mut self, clock: u32) -> u32 {
        Ym2203::sample_rate(self, clock)
    }

    fn generate(&mut self, out: &mut [Self::Native]) {
        Ym2203::generate(self, out);
    }

    fn mix_sample(sample: &Self::Native, out: &mut [f32]) {
        const FM_SCALE: f32 = 1.0 / 32768.0;
        const SSG_SCALE: f32 = (1.0 / 3.0) / 32768.0;
        out[0] = sample.data[0] as f32 * FM_SCALE
            + sample.data[1] as f32 * SSG_SCALE
            + sample.data[2] as f32 * SSG_SCALE
            + sample.data[3] as f32 * SSG_SCALE;
    }

    fn read_status(&mut self, busy: bool) -> u8 {
        Ym2203::read_status(self, busy)
    }

    fn read_data(&mut self) -> u8 {
        Ym2203::read_data(self)
    }

    fn write_address(&mut self, value: u8) -> u32 {
        Ym2203::write_address(self, value)
    }

    fn write_data(&mut self, value: u8) -> u32 {
        Ym2203::write_data(self, value)
    }

    fn timer_expired(&mut self, timer_id: u32) {
        Ym2203::timer_expired(self, timer_id);
    }

    fn set_io_input(&mut self, port: u8, value: u8) {
        Ym2203::set_io_input(self, port, value);
    }

    fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        Ym2203::take_timer_update(self, timer_id)
    }

    fn take_irq_update(&mut self) -> Option<bool> {
        Ym2203::take_irq_update(self)
    }
}

impl OpnChip for Ym2608 {
    type Native = YmfmOutput3;
    const CLOCK: u32 = 7_987_200;
    const CHANNELS: usize = 2;

    fn create() -> Self {
        let mut chip = Ym2608::new();
        chip.reset();
        chip.set_fidelity(FIDELITY);
        chip
    }

    fn native_zero() -> Self::Native {
        YmfmOutput3 { data: [0; 3] }
    }

    fn sample_rate(&mut self, clock: u32) -> u32 {
        Ym2608::sample_rate(self, clock)
    }

    fn generate(&mut self, out: &mut [Self::Native]) {
        Ym2608::generate(self, out);
    }

    fn mix_sample(sample: &Self::Native, out: &mut [f32]) {
        const FM_SCALE: f32 = 2.0 / 32768.0;
        const SSG_SCALE: f32 = 0.5 / 32768.0;
        let ssg = sample.data[2] as f32 * SSG_SCALE;
        out[0] = sample.data[0] as f32 * FM_SCALE + ssg;
        out[1] = sample.data[1] as f32 * FM_SCALE + ssg;
    }

    fn read_status(&mut self, busy: bool) -> u8 {
        Ym2608::read_status(self, busy)
    }

    fn read_data(&mut self) -> u8 {
        Ym2608::read_data(self)
    }

    fn write_address(&mut self, value: u8) -> u32 {
        Ym2608::write_address(self, value)
    }

    fn write_data(&mut self, value: u8) -> u32 {
        Ym2608::write_data(self, value)
    }

    fn write_address_hi(&mut self, value: u8) -> u32 {
        Ym2608::write_address_hi(self, value)
    }

    fn write_data_hi(&mut self, value: u8) -> u32 {
        Ym2608::write_data_hi(self, value)
    }

    fn read_status_hi(&mut self, busy: bool) -> u8 {
        Ym2608::read_status_hi(self, busy)
    }

    fn read_data_hi(&mut self) -> u8 {
        Ym2608::read_data_hi(self)
    }

    fn timer_expired(&mut self, timer_id: u32) {
        Ym2608::timer_expired(self, timer_id);
    }

    fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        Ym2608::take_timer_update(self, timer_id)
    }

    fn take_irq_update(&mut self) -> Option<bool> {
        Ym2608::take_irq_update(self)
    }
}

/// Generic OPN/OPNA FM audio driver: chip clocking, busy timing, timer/IRQ
/// coalescing, and resampling.
pub struct OpnFm<C: OpnChip> {
    chip: C,
    cpu_clock_hz: u32,
    sample_rate: u32,
    native_rate: u32,
    chip_action_cycle: u64,
    native_buffer: Vec<C::Native>,
    pending_native: Vec<C::Native>,
    resampler: ResamplerFir,
    resample_input: Vec<f32>,
    resample_output: Vec<f32>,
    sample_remainder: FmSampleRemainder,
    fm_sync_cursor: u64,
    busy_end_cycle: u64,
    audio_frame_start_cycle: u64,
    irq_asserted: bool,
    timer_actions: Vec<FmTimerAction>,
}

impl<C: OpnChip> OpnFm<C> {
    /// Creates a driver around a reset chip and a configured resampler.
    pub fn new(cpu_clock_hz: u32, sample_rate: u32) -> Self {
        let mut chip = C::create();
        let native_rate = chip.sample_rate(C::CLOCK);
        let resampler = ResamplerFir::new_from_hz(
            C::CHANNELS,
            native_rate,
            sample_rate,
            RESAMPLER_LATENCY,
            RESAMPLER_ATTENUATION,
        );
        let resample_output_size = resampler.buffer_size_output();
        Self {
            chip,
            cpu_clock_hz,
            sample_rate,
            native_rate,
            chip_action_cycle: 0,
            native_buffer: vec![C::native_zero(); INITIAL_NATIVE_CAPACITY],
            pending_native: Vec::new(),
            resampler,
            resample_input: vec![0.0; INITIAL_NATIVE_CAPACITY * C::CHANNELS],
            resample_output: vec![0.0; resample_output_size],
            sample_remainder: FmSampleRemainder::default(),
            fm_sync_cursor: 0,
            busy_end_cycle: 0,
            audio_frame_start_cycle: 0,
            irq_asserted: false,
            timer_actions: Vec::new(),
        }
    }

    /// Borrows the chip for board-specific configuration (e.g. ADPCM setup).
    pub fn chip_mut(&mut self) -> &mut C {
        &mut self.chip
    }

    /// Sets the SSG parallel I/O input value (port 0 is A, port 1 is B).
    pub fn set_io_input(&mut self, port: u8, value: u8) {
        self.chip.set_io_input(port, value);
    }

    /// Returns the native output sample rate.
    pub fn native_rate(&self) -> u32 {
        self.native_rate
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.irq_asserted
    }

    fn apply_busy(&mut self, busy_clocks: u32, current_cycle: u64) {
        if busy_clocks != 0 {
            let cpu_clocks =
                u64::from(busy_clocks) * u64::from(self.cpu_clock_hz) / u64::from(C::CLOCK);
            self.busy_end_cycle = current_cycle + cpu_clocks;
        }
    }

    fn busy_at(&self, current_cycle: u64) -> bool {
        current_cycle < self.busy_end_cycle
    }

    /// Advances the chip clock to `current_cycle`, buffering native samples for
    /// the next [`OpnFm::generate_samples`] so the chip's internal clock is
    /// always up to date when registers are accessed.
    pub fn sync(&mut self, current_cycle: u64) {
        let elapsed_cycles = current_cycle.saturating_sub(self.fm_sync_cursor);
        if elapsed_cycles == 0 {
            return;
        }

        let native_rate = u64::from(self.native_rate);
        let exact_native = (elapsed_cycles as f64 * native_rate as f64)
            / f64::from(self.cpu_clock_hz)
            + self.sample_remainder.0;
        let native_count = exact_native as usize;
        if native_count == 0 {
            return;
        }

        self.sample_remainder = FmSampleRemainder(exact_native - native_count as f64);
        self.fm_sync_cursor = current_cycle;

        if self.native_buffer.len() < native_count {
            self.native_buffer.resize(native_count, C::native_zero());
        }
        self.chip.generate(&mut self.native_buffer[..native_count]);
        self.pending_native
            .extend_from_slice(&self.native_buffer[..native_count]);
    }

    /// Records the cycle at which the current chip access happens (used as the
    /// base for timer scheduling in [`OpnFm::drain_timers`]).
    pub fn set_action_cycle(&mut self, current_cycle: u64) {
        self.chip_action_cycle = current_cycle;
    }

    /// Reads the low-bank status register (with sync + busy reconciliation).
    pub fn read_status(&mut self, current_cycle: u64) -> u8 {
        self.sync(current_cycle);
        self.chip_action_cycle = current_cycle;
        let busy = self.busy_at(current_cycle);
        self.chip.read_status(busy)
    }

    /// Reads the currently addressed low-bank register.
    pub fn read_data(&mut self, current_cycle: u64) -> u8 {
        self.sync(current_cycle);
        self.chip_action_cycle = current_cycle;
        self.chip.read_data()
    }

    /// Reads the high-bank status register (OPNA only).
    pub fn read_status_hi(&mut self, current_cycle: u64) -> u8 {
        self.sync(current_cycle);
        self.chip_action_cycle = current_cycle;
        let busy = self.busy_at(current_cycle);
        self.chip.read_status_hi(busy)
    }

    /// Reads the currently addressed high-bank register (OPNA only).
    pub fn read_data_hi(&mut self, current_cycle: u64) -> u8 {
        self.sync(current_cycle);
        self.chip_action_cycle = current_cycle;
        self.chip.read_data_hi()
    }

    /// Latches the low-bank register address.
    pub fn write_address(&mut self, value: u8, current_cycle: u64) {
        self.sync(current_cycle);
        self.chip_action_cycle = current_cycle;
        let busy_clocks = self.chip.write_address(value);
        self.apply_busy(busy_clocks, current_cycle);
    }

    /// Writes the addressed low-bank register.
    pub fn write_data(&mut self, value: u8, current_cycle: u64) {
        self.sync(current_cycle);
        self.chip_action_cycle = current_cycle;
        let busy_clocks = self.chip.write_data(value);
        self.apply_busy(busy_clocks, current_cycle);
    }

    /// Latches the high-bank register address (OPNA only).
    pub fn write_address_hi(&mut self, value: u8, current_cycle: u64) {
        self.sync(current_cycle);
        self.chip_action_cycle = current_cycle;
        let busy_clocks = self.chip.write_address_hi(value);
        self.apply_busy(busy_clocks, current_cycle);
    }

    /// Writes the addressed high-bank register (OPNA only).
    pub fn write_data_hi(&mut self, value: u8, current_cycle: u64) {
        self.sync(current_cycle);
        self.chip_action_cycle = current_cycle;
        let busy_clocks = self.chip.write_data_hi(value);
        self.apply_busy(busy_clocks, current_cycle);
    }

    /// Notifies the chip that timer `timer_id` has expired.
    pub fn timer_expired(&mut self, timer_id: u32, current_cycle: u64) {
        self.sync(current_cycle);
        self.chip_action_cycle = current_cycle;
        self.chip.timer_expired(timer_id);
    }

    /// Drains pending FM timer schedule/cancel requests, keyed by `timer_id`.
    pub fn drain_timers(&mut self) -> &[FmTimerAction] {
        self.timer_actions.clear();
        let current_cycle = self.chip_action_cycle;
        for timer_id in [0u8, 1u8] {
            let Some(update) = self.chip.take_timer_update(timer_id) else {
                continue;
            };
            match update {
                YmfmTimerUpdate::Cancel => {
                    self.timer_actions.push(FmTimerAction::Cancel { timer_id });
                }
                YmfmTimerUpdate::Schedule(duration_in_clocks) => {
                    let cpu_cycles = u64::from(duration_in_clocks) * u64::from(self.cpu_clock_hz)
                        / u64::from(C::CLOCK);
                    self.timer_actions.push(FmTimerAction::Schedule {
                        timer_id,
                        fire_cycle: current_cycle + cpu_cycles,
                    });
                }
            }
        }
        self.timer_actions.as_slice()
    }

    /// Returns and clears the coalesced chip IRQ-output edge, updating the
    /// cached [`OpnFm::irq_asserted`] state.
    pub fn take_irq_change(&mut self) -> Option<bool> {
        let change = self.chip.take_irq_update();
        if let Some(asserted) = change {
            self.irq_asserted = asserted;
        }
        change
    }

    /// Generates resampled FM+SSG audio and additively mixes it into `output`
    /// (interleaved stereo `[L, R, L, R, ...]`) at `volume`. A mono chip is
    /// duplicated to both channels; a stereo chip is mixed per channel.
    pub fn generate_samples(
        &mut self,
        current_cycle: u64,
        cpu_clock_hz: u32,
        volume: f32,
        output: &mut [f32],
    ) {
        if output.is_empty() {
            self.sync(current_cycle);
            self.pending_native.clear();
            self.audio_frame_start_cycle = current_cycle;
            self.fm_sync_cursor = current_cycle;
            return;
        }

        let gap_cycles = current_cycle.saturating_sub(self.fm_sync_cursor);
        let remaining_from_timing = if gap_cycles > 0 {
            let native_rate = u64::from(self.native_rate);
            let exact_native = (gap_cycles as f64 * native_rate as f64) / f64::from(cpu_clock_hz)
                + self.sample_remainder.0;
            let count = exact_native as usize;
            self.sample_remainder = FmSampleRemainder(exact_native - count as f64);
            count
        } else {
            0
        };

        let pending_count = self.pending_native.len();
        let total_from_timing = pending_count + remaining_from_timing;

        let output_frames = output.len() / 2;
        let min_native = (output_frames as u64 * u64::from(self.native_rate))
            .div_ceil(u64::from(self.sample_rate))
            + 1;
        let total_native = total_from_timing.max(min_native as usize);
        let remaining_native = total_native - pending_count;

        if remaining_native > 0 {
            if self.native_buffer.len() < remaining_native {
                self.native_buffer
                    .resize(remaining_native, C::native_zero());
            }
            self.chip
                .generate(&mut self.native_buffer[..remaining_native]);
        }

        if total_native > 0 {
            let total_interleaved = total_native * C::CHANNELS;
            if self.resample_input.len() < total_interleaved {
                self.resample_input.resize(total_interleaved, 0.0);
            }

            for i in 0..pending_count {
                let base = i * C::CHANNELS;
                C::mix_sample(
                    &self.pending_native[i],
                    &mut self.resample_input[base..base + C::CHANNELS],
                );
            }
            for i in 0..remaining_native {
                let base = (pending_count + i) * C::CHANNELS;
                C::mix_sample(
                    &self.native_buffer[i],
                    &mut self.resample_input[base..base + C::CHANNELS],
                );
            }

            if C::CHANNELS == 1 {
                self.mix_mono_into(total_interleaved, volume, output);
            } else {
                self.mix_interleaved_into(total_interleaved, volume, output);
            }
        }

        self.pending_native.clear();
        self.audio_frame_start_cycle = current_cycle;
        self.fm_sync_cursor = current_cycle;
    }

    /// Mono path: resample, then duplicate each output sample to L and R.
    fn mix_mono_into(&mut self, total_native: usize, volume: f32, output: &mut [f32]) {
        let mut input_offset = 0;
        let mut output_frame_offset = 0;
        let frame_count = output.len() / 2;
        while input_offset < total_native && output_frame_offset < frame_count {
            let Ok((consumed, produced)) = self.resampler.resample(
                &self.resample_input[input_offset..total_native],
                &mut self.resample_output,
            ) else {
                break;
            };
            let usable = produced.min(frame_count - output_frame_offset);
            for i in 0..usable {
                let sample = self.resample_output[i] * volume;
                output[(output_frame_offset + i) * 2] += sample;
                output[(output_frame_offset + i) * 2 + 1] += sample;
            }
            input_offset += consumed;
            output_frame_offset += usable;
            if consumed == 0 {
                break;
            }
        }
    }

    /// Stereo path: resample interleaved L/R directly into the output.
    fn mix_interleaved_into(&mut self, total_interleaved: usize, volume: f32, output: &mut [f32]) {
        let mut input_offset = 0;
        let mut output_offset = 0;
        let sample_count = output.len();
        while input_offset < total_interleaved && output_offset < sample_count {
            let Ok((consumed, produced)) = self.resampler.resample(
                &self.resample_input[input_offset..total_interleaved],
                &mut self.resample_output,
            ) else {
                break;
            };
            let usable = produced.min(sample_count - output_offset);
            for (out, &resampled) in output[output_offset..output_offset + usable]
                .iter_mut()
                .zip(&self.resample_output[..usable])
            {
                *out += resampled * volume;
            }
            input_offset += consumed;
            output_offset += usable;
            if consumed == 0 {
                break;
            }
        }
    }

    /// Returns the timing scalars for save/restore.
    pub fn timing(&self) -> OpnFmTiming {
        OpnFmTiming {
            sample_remainder: self.sample_remainder,
            fm_sync_cursor: self.fm_sync_cursor,
            busy_end_cycle: self.busy_end_cycle,
            audio_frame_start_cycle: self.audio_frame_start_cycle,
            irq_asserted: self.irq_asserted,
        }
    }

    /// Restores the timing scalars from a saved snapshot.
    pub fn set_timing(&mut self, timing: OpnFmTiming) {
        self.sample_remainder = timing.sample_remainder;
        self.fm_sync_cursor = timing.fm_sync_cursor;
        self.busy_end_cycle = timing.busy_end_cycle;
        self.audio_frame_start_cycle = timing.audio_frame_start_cycle;
        self.irq_asserted = timing.irq_asserted;
    }

    /// Recreates the chip and resampler for the given clock/rate (save/load).
    /// The caller reapplies any board-specific chip configuration via
    /// [`OpnFm::chip_mut`] afterward.
    pub fn reload(&mut self, cpu_clock_hz: u32, sample_rate: u32, current_cycle: u64) {
        self.cpu_clock_hz = cpu_clock_hz;
        self.sample_rate = sample_rate;
        self.chip_action_cycle = current_cycle;
        self.chip = C::create();
        self.native_rate = self.chip.sample_rate(C::CLOCK);
        self.resampler = ResamplerFir::new_from_hz(
            C::CHANNELS,
            self.native_rate,
            sample_rate,
            RESAMPLER_LATENCY,
            RESAMPLER_ATTENUATION,
        );
        self.resample_output
            .resize(self.resampler.buffer_size_output(), 0.0);
        self.pending_native.clear();
    }
}

/// Saveable timing scalars of an [`OpnFm`] driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpnFmTiming {
    /// Fractional sample remainder carried across frames.
    pub sample_remainder: FmSampleRemainder,
    /// CPU cycle up to which the chip has been clocked.
    pub fm_sync_cursor: u64,
    /// CPU cycle at which the busy flag clears.
    pub busy_end_cycle: u64,
    /// CPU cycle at which the current audio frame started.
    pub audio_frame_start_cycle: u64,
    /// Whether the chip IRQ output is currently asserted.
    pub irq_asserted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_then_cancel_timer_a_round_trip() {
        let mut fm = OpnFm::<Ym2608>::new(8_000_000, 48_000);
        // Program timer A and enable/load it: set period registers then the
        // timer control (reg 0x27) load bits.
        fm.write_address(0x24, 0);
        fm.write_data(0xFF, 0);
        fm.write_address(0x25, 0);
        fm.write_data(0x03, 0);
        fm.write_address(0x27, 0);
        fm.write_data(0x01, 0); // load + enable timer A
        let scheduled = fm
            .drain_timers()
            .iter()
            .any(|a| matches!(a, FmTimerAction::Schedule { timer_id: 0, .. }));
        assert!(scheduled, "loading timer A should schedule it");

        // Clear the load bit: the timer is cancelled.
        fm.write_address(0x27, 0);
        fm.write_data(0x00, 0);
        let cancelled = fm
            .drain_timers()
            .iter()
            .any(|a| matches!(a, FmTimerAction::Cancel { timer_id: 0 }));
        assert!(cancelled, "clearing timer A load should cancel it");
    }

    #[test]
    fn mono_generation_is_non_silent_after_key_on() {
        let mut fm = OpnFm::<Ym2203>::new(4_000_000, 48_000);
        // A minimal FM note: set a total level and key it on.
        fm.write_address(0x40, 0);
        fm.write_data(0x00, 0); // operator 1 total level = max volume
        fm.write_address(0x28, 0);
        fm.write_data(0xF0, 0); // key on channel 0, all operators
        let mut out = vec![0.0f32; 256 * 2];
        fm.generate_samples(48_000, 4_000_000, 1.0, &mut out);
        // The driver must at least fill the buffer without panicking; mono is
        // duplicated to both channels.
        assert_eq!(out.len(), 256 * 2);
    }

    #[test]
    fn empty_output_advances_cursors() {
        let mut fm = OpnFm::<Ym2608>::new(8_000_000, 48_000);
        let mut empty: [f32; 0] = [];
        fm.generate_samples(1_000, 8_000_000, 1.0, &mut empty);
        assert_eq!(fm.timing().fm_sync_cursor, 1_000);
        assert_eq!(fm.timing().audio_frame_start_cycle, 1_000);
    }
}
