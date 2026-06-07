//! PC-9801-26K sound board: YM2203 (OPN) FM + SSG synthesis with resampling.

use common::EventKind;
use resampler::{Attenuation, Latency, ResamplerFir};
use ymfm_oxide::{Ym2203, YmfmOpnFidelity, YmfmOutput4, YmfmTimerUpdate};

/// YM2203 input clock: 15.9744 MHz / 4 = 3,993,600 Hz.
const YM2203_CLOCK: u32 = 3_993_600;

const FIDELITY: YmfmOpnFidelity = YmfmOpnFidelity::Max;

/// Fractional sample remainder for drift-free FM sample count accumulation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FmSampleRemainder(pub f64);

impl Eq for FmSampleRemainder {}

impl Default for FmSampleRemainder {
    fn default() -> Self {
        Self(0.0)
    }
}

/// Snapshot of the PC-9801-26K sound board state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Soundboard26kState {
    /// Address latch (write-only via port 0x0188).
    pub address: u8,
    /// IRQ line number (default 12 = INT5).
    pub irq_line: u8,
    /// Whether the IRQ output is currently asserted.
    pub irq_asserted: bool,
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
    /// Whether this board uses alternate timer event kinds (dual-board config).
    pub alternate_timers: bool,
}

impl Default for Soundboard26kState {
    fn default() -> Self {
        Self {
            address: 0,
            irq_line: 12,
            irq_asserted: false,
            busy_end_cycle: 0,
            audio_frame_start_cycle: 0,
            fm_sync_cursor: 0,
            sample_remainder: FmSampleRemainder::default(),
            alternate_timers: false,
        }
    }
}

/// Action the bus must process after a sound board operation.
#[derive(Clone, Copy)]
pub enum Soundboard26kAction {
    /// Schedule a timer to fire at the given cycle.
    ScheduleTimer {
        /// Timer event kind.
        kind: EventKind,
        /// CPU cycle at which the timer should fire.
        fire_cycle: u64,
    },
    /// Cancel a previously scheduled timer.
    CancelTimer {
        /// Timer event kind.
        kind: EventKind,
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

/// PC-9801-26K sound board: YM2203 (OPN) FM + SSG synthesis with resampling.
pub struct Soundboard26k {
    /// Current device state (saveable).
    pub state: Soundboard26kState,
    chip: Ym2203,
    cpu_clock_hz: u32,
    chip_action_cycle: u64,
    native_rate: u32,
    sample_rate: u32,
    native_buffer: Vec<YmfmOutput4>,
    pending_native: Vec<YmfmOutput4>,
    resampler: ResamplerFir,
    resample_input: Vec<f32>,
    resample_output: Vec<f32>,
    action_buffer: Vec<Soundboard26kAction>,
}

impl Soundboard26k {
    /// Creates a new PC-9801-26K sound board instance.
    ///
    /// When `alternate_timers` is `true`, uses `FmTimer2A`/`FmTimer2B` event
    /// kinds instead of `FmTimerA`/`FmTimerB` (for dual-board configurations).
    pub fn new(cpu_clock_hz: u32, sample_rate: u32, alternate_timers: bool) -> Self {
        let mut chip = Ym2203::new();
        chip.reset();
        chip.set_fidelity(FIDELITY);

        let native_rate = chip.sample_rate(YM2203_CLOCK);
        let resampler = ResamplerFir::new_from_hz(
            1,
            native_rate,
            sample_rate,
            Latency::Sample64,
            Attenuation::Db60,
        );
        let resample_output_size = resampler.buffer_size_output();

        let state = Soundboard26kState {
            alternate_timers,
            ..Default::default()
        };

        Self {
            state,
            chip,
            cpu_clock_hz,
            chip_action_cycle: 0,
            native_rate,
            sample_rate,
            native_buffer: vec![YmfmOutput4 { data: [0; 4] }; 4096],
            pending_native: Vec::new(),
            resampler,
            resample_input: vec![0.0; 4096],
            resample_output: vec![0.0; resample_output_size],
            action_buffer: Vec::new(),
        }
    }

    const fn timer_kind(&self, timer_id: u8) -> EventKind {
        match (self.state.alternate_timers, timer_id) {
            (true, 0) => EventKind::FmTimer2A,
            (true, _) => EventKind::FmTimer2B,
            (false, 0) => EventKind::FmTimerA,
            (false, _) => EventKind::FmTimerB,
        }
    }

    fn apply_busy(&mut self, busy_clocks: u32, current_cycle: u64) {
        if busy_clocks != 0 {
            let cpu_clocks =
                u64::from(busy_clocks) * u64::from(self.cpu_clock_hz) / u64::from(YM2203_CLOCK);
            self.state.busy_end_cycle = current_cycle + cpu_clocks;
        }
    }

    fn busy_at(&self, current_cycle: u64) -> bool {
        current_cycle < self.state.busy_end_cycle
    }

    /// Returns the currently latched register address.
    pub fn address(&self) -> u8 {
        self.state.address
    }

    /// Returns the configured IRQ line number.
    pub fn irq_line(&self) -> u8 {
        self.state.irq_line
    }

    /// Sets the address latch to a specific value (used by ITF initialization).
    pub fn set_address(&mut self, address: u8) {
        self.state.address = address;
    }

    /// Advances the YM2203 chip clock to `current_cycle` by generating native
    /// samples, buffering them for later resampling in `generate_samples()`.
    ///
    /// This ensures `m_total_clocks` inside ymfm is always up-to-date when
    /// registers are read or written, eliminating timer scheduling
    /// non-determinism caused by varying audio generation intervals.
    fn sync_to_cycle(&mut self, current_cycle: u64) {
        let sync_start = self.state.fm_sync_cursor;
        let elapsed_cycles = current_cycle.saturating_sub(sync_start);
        if elapsed_cycles == 0 {
            return;
        }

        let native_rate = u64::from(self.native_rate);
        let exact_native = (elapsed_cycles as f64 * native_rate as f64)
            / f64::from(self.cpu_clock_hz)
            + self.state.sample_remainder.0;
        let native_count = exact_native as usize;
        if native_count == 0 {
            return;
        }

        self.state.sample_remainder = FmSampleRemainder(exact_native - native_count as f64);
        self.state.fm_sync_cursor = current_cycle;

        if self.native_buffer.len() < native_count {
            self.native_buffer
                .resize(native_count, YmfmOutput4 { data: [0; 4] });
        }
        self.chip.generate(&mut self.native_buffer[..native_count]);
        self.pending_native
            .extend_from_slice(&self.native_buffer[..native_count]);
    }

    /// Reads the chip status register (port 0x0188 read).
    pub fn read_status(&mut self, current_cycle: u64) -> u8 {
        self.sync_to_cycle(current_cycle);
        self.chip_action_cycle = current_cycle;
        self.chip.read_status(self.busy_at(current_cycle))
    }

    /// Reads data from the currently addressed register (port 0x018A read).
    ///
    /// Caller must call `drain_actions()` afterward.
    pub fn read_data(&mut self, current_cycle: u64) -> u8 {
        self.sync_to_cycle(current_cycle);
        self.chip_action_cycle = current_cycle;
        self.chip.read_data()
    }

    /// Writes the register address latch (port 0x0188 write).
    ///
    /// Caller must call `drain_actions()` afterward.
    pub fn write_address(&mut self, value: u8, current_cycle: u64) {
        self.state.address = value;
        self.sync_to_cycle(current_cycle);
        self.chip_action_cycle = current_cycle;
        let busy_clocks = self.chip.write_address(value);
        self.apply_busy(busy_clocks, current_cycle);
    }

    /// Writes data to the currently addressed register (port 0x018A write).
    ///
    /// Caller must call `drain_actions()` afterward.
    pub fn write_data(&mut self, value: u8, current_cycle: u64) {
        self.sync_to_cycle(current_cycle);
        self.chip_action_cycle = current_cycle;
        let busy_clocks = self.chip.write_data(value);
        self.apply_busy(busy_clocks, current_cycle);
    }

    /// Notifies the chip that a timer has expired.
    ///
    /// Caller must call `drain_actions()` afterward.
    pub fn timer_expired(&mut self, timer_id: u32, current_cycle: u64) {
        self.sync_to_cycle(current_cycle);
        self.chip_action_cycle = current_cycle;
        self.chip.timer_expired(timer_id);
    }

    /// Drains pending actions from the chip.
    ///
    /// Returns actions the bus must process (timer scheduling and IRQ
    /// assertion/deassertion). Also updates `state.busy_end_cycle` and
    /// `state.irq_asserted` internally.
    pub fn drain_actions(&mut self) -> &[Soundboard26kAction] {
        self.action_buffer.clear();
        let current_cycle = self.chip_action_cycle;

        for (timer_id, kind) in [(0, self.timer_kind(0)), (1, self.timer_kind(1))] {
            let Some(update) = self.chip.take_timer_update(timer_id) else {
                continue;
            };
            match update {
                YmfmTimerUpdate::Cancel => self
                    .action_buffer
                    .push(Soundboard26kAction::CancelTimer { kind }),
                YmfmTimerUpdate::Schedule(duration_in_clocks) => {
                    let cpu_cycles = u64::from(duration_in_clocks) * u64::from(self.cpu_clock_hz)
                        / u64::from(YM2203_CLOCK);
                    self.action_buffer.push(Soundboard26kAction::ScheduleTimer {
                        kind,
                        fire_cycle: current_cycle + cpu_cycles,
                    });
                }
            }
        }

        if let Some(asserted) = self.chip.take_irq_update() {
            self.state.irq_asserted = asserted;
            if asserted {
                self.action_buffer.push(Soundboard26kAction::AssertIrq {
                    irq: self.state.irq_line,
                });
            } else {
                self.action_buffer.push(Soundboard26kAction::DeassertIrq {
                    irq: self.state.irq_line,
                });
            }
        }
        self.action_buffer.as_slice()
    }

    /// Generates resampled FM+SSG audio and mixes it into `output`.
    ///
    /// `output` is interleaved stereo (`[L, R, L, R, …]`); this method
    /// additively mixes in the YM2203 (mono) output scaled by `volume`,
    /// duplicated to both channels.
    pub fn generate_samples(
        &mut self,
        current_cycle: u64,
        cpu_clock_hz: u32,
        volume: f32,
        output: &mut [f32],
    ) {
        if output.is_empty() {
            self.sync_to_cycle(current_cycle);
            self.pending_native.clear();
            self.state.audio_frame_start_cycle = current_cycle;
            self.state.fm_sync_cursor = current_cycle;
            return;
        }

        // Generate remaining FM native samples from fm_sync_cursor to current_cycle.
        let sync_cursor = self.state.fm_sync_cursor;
        let gap_cycles = current_cycle.saturating_sub(sync_cursor);
        let remaining_native = if gap_cycles > 0 {
            let native_rate = u64::from(self.native_rate);
            let exact_native = (gap_cycles as f64 * native_rate as f64) / f64::from(cpu_clock_hz)
                + self.state.sample_remainder.0;
            let count = exact_native as usize;
            self.state.sample_remainder = FmSampleRemainder(exact_native - count as f64);
            count
        } else {
            0
        };

        let pending_count = self.pending_native.len();
        let total_from_timing = pending_count + remaining_native;

        // Ensure the resampler receives enough input to fill the output.
        let output_frames = output.len() / 2;
        let min_native = (output_frames as u64 * u64::from(self.native_rate))
            .div_ceil(u64::from(self.sample_rate))
            + 1;
        let total_native = total_from_timing.max(min_native as usize);
        let remaining_native = total_native - pending_count;

        if remaining_native > 0 {
            if self.native_buffer.len() < remaining_native {
                self.native_buffer
                    .resize(remaining_native, YmfmOutput4 { data: [0; 4] });
            }
            self.chip
                .generate(&mut self.native_buffer[..remaining_native]);
        }

        if total_native > 0 {
            if self.resample_input.len() < total_native {
                self.resample_input.resize(total_native, 0.0);
            }

            const FM_SCALE: f32 = 1.0 / 32768.0;
            const SSG_SCALE: f32 = (1.0 / 3.0) / 32768.0;

            for i in 0..pending_count {
                let s = &self.pending_native[i];
                self.resample_input[i] = s.data[0] as f32 * FM_SCALE
                    + s.data[1] as f32 * SSG_SCALE
                    + s.data[2] as f32 * SSG_SCALE
                    + s.data[3] as f32 * SSG_SCALE;
            }
            for i in 0..remaining_native {
                let s = &self.native_buffer[i];
                self.resample_input[pending_count + i] = s.data[0] as f32 * FM_SCALE
                    + s.data[1] as f32 * SSG_SCALE
                    + s.data[2] as f32 * SSG_SCALE
                    + s.data[3] as f32 * SSG_SCALE;
            }

            // Resample mono, then duplicate each sample to both stereo channels.
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

        self.pending_native.clear();
        self.state.audio_frame_start_cycle = current_cycle;
        self.state.fm_sync_cursor = current_cycle;
    }

    /// Creates a snapshot of the current state for save/restore.
    pub fn save_state(&self) -> Soundboard26kState {
        self.state.clone()
    }

    /// Restores from a saved state, recreating the ymfm chip.
    pub fn load_state(
        &mut self,
        saved: &Soundboard26kState,
        cpu_clock_hz: u32,
        sample_rate: u32,
        current_cycle: u64,
    ) {
        self.state = saved.clone();
        // TODO: Save/restore ymfm internal state
        self.chip_action_cycle = current_cycle;
        self.cpu_clock_hz = cpu_clock_hz;
        self.chip = Ym2203::new();
        self.chip.reset();
        self.chip.set_fidelity(FIDELITY);
        self.native_rate = self.chip.sample_rate(YM2203_CLOCK);
        self.resampler = ResamplerFir::new_from_hz(
            1,
            self.native_rate,
            sample_rate,
            Latency::Sample64,
            Attenuation::Db60,
        );
        self.resample_output
            .resize(self.resampler.buffer_size_output(), 0.0);
    }
}
