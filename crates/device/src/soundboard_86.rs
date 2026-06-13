//! PC-9801-86 sound board: YM2608 (OPNA) stereo FM + SSG + ADPCM + PCM86 DAC.

use common::{EventKind, MachineModel};
use resampler::{Attenuation, Latency, ResamplerFir};
use ymfm_oxide::Ym2608;

use crate::{
    opn_fm::{EVOLVED_RHYTHM_ROM, FmTimerAction, OpnFm, OpnFmTiming},
    soundboard_26k::FmSampleRemainder,
};

/// 256 KB ADPCM-B sample RAM.
const ADPCM_B_RAM_SIZE: usize = 256 * 1024;

/// PCM86 sample rates indexed by (fifo & 7).
const PCM86_RATES: [u32; 8] = [44100, 33075, 22050, 16538, 11025, 8269, 5513, 4134];

/// PCM86 step bit table indexed by (dactrl >> 4) & 7.
const PCM86_BITS: [u8; 8] = [1, 1, 1, 2, 0, 0, 0, 1];

/// PCM86 buffer size (64 KB ring buffer).
const PCM86_BUFSIZE: usize = 1 << 16;

const PCM86_BUFMASK: u32 = (PCM86_BUFSIZE as u32) - 1;

/// PCM86 logical buffer capacity.
const PCM86_LOGICAL_BUF: i32 = 0x8000;

/// PCM86 rescue buffer base size (samples).
const PCM86_RESCUE: i32 = 20;

/// Q-Vision WaveStar magic handshake sequence (port 0xA462 write).
const WAVESTAR_SEQUENCE: [u8; 5] = [0xA6, 0xD3, 0x69, 0xB4, 0x5A];

/// PCM86 rescue table indexed by (fifo & 7): base rescue size per rate.
const PCM86_RESCUE_TABLE: [i32; 8] = [
    PCM86_RESCUE * 32,
    PCM86_RESCUE * 24,
    PCM86_RESCUE * 16,
    PCM86_RESCUE * 12,
    PCM86_RESCUE * 8,
    PCM86_RESCUE * 6,
    PCM86_RESCUE * 4,
    PCM86_RESCUE * 3,
];

const RESAMPLER_ATTENUTATION: Attenuation = Attenuation::Db60;

const REAMPLER_LATENCY: Latency = Latency::Sample64;

/// Snapshot of the PC-9801-86 sound board state.
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
}

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

/// PCM86 DAC state snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm86State {
    /// Sound flags register (port 0xA460).
    pub sound_flags: u8,
    /// FIFO control register (port 0xA468).
    pub fifo: u8,
    /// DAC control register (port 0xA46A).
    pub dactrl: u8,
    /// Volume registers for all 8 electronic volume lines (port 0xA466).
    /// Index = bits 7-5 of the written value. Only index 5 (PCM direct) is
    /// used by the PCM86 DAC; others are stored for register-level accuracy.
    pub vol: [u8; 8],
    /// PCM output mute register (port 0xA66E). Bit 0 = mute.
    pub pcm_mute: u8,
    /// PCM86 IRQ flag - set when FIFO drops below threshold.
    pub irq_flag: bool,
    /// FIFO interrupt threshold.
    pub fifo_size: i32,
    /// Write position in ring buffer.
    pub write_pos: u32,
    /// Read position in ring buffer.
    pub read_pos: u32,
    /// Physical buffer count.
    pub real_buf: i32,
    /// Virtual buffer count (capped at PCM86_LOGICAL_BUF).
    pub vir_buf: i32,
    /// Step bit (bytes-per-sample shift).
    pub step_bit: u8,
    /// Step mask.
    pub step_mask: u32,
    /// Rescue buffer size (rate-dependent overflow margin).
    pub rescue: i32,
    /// IRQ request pending (set on data write, cleared when IRQ fires).
    pub reqirq: bool,
    /// Cycle of last data write (for IRQ delay).
    pub last_clock_for_wait: u64,
    /// Minimum cycles after data write before IRQ can fire.
    pub data_write_irq_wait: u64,
    /// Audio frame start cycle.
    pub audio_frame_start_cycle: u64,
    /// Fractional sample remainder for the cycle-based PCM drain.
    pub sample_remainder: FmSampleRemainder,
    /// Fractional sample remainder for `update_buffer_state` vir_buf tracking.
    pub vir_buf_remainder: FmSampleRemainder,
    /// Cycle anchor for the port A466h L/R DAC clock bit.
    pub dac_clock_cycle: u64,
    /// Fractional sample phase for the port A466h L/R DAC clock bit.
    pub dac_clock_remainder: FmSampleRemainder,
    /// WaveStar handshake sequence index (0..=5).
    pub wavestar_seq_index: u8,
    /// WaveStar port 0xA464 readback value.
    pub wavestar_value: u8,
}

impl Default for Pcm86State {
    fn default() -> Self {
        Self {
            // PC-9801-86 ID (0x4x) at 0x018x base; mask/extend bits clear at reset.
            sound_flags: 0x40,
            fifo: 0,
            dactrl: 0x32,
            vol: [0; 8],
            pcm_mute: 0x00,
            irq_flag: false,
            fifo_size: 128,
            write_pos: 0,
            read_pos: 0,
            real_buf: 0,
            vir_buf: 0,
            step_bit: 2,
            step_mask: 3,
            rescue: PCM86_RESCUE_TABLE[0] << 2,
            reqirq: false,
            last_clock_for_wait: 0,
            data_write_irq_wait: 0,
            audio_frame_start_cycle: 0,
            sample_remainder: FmSampleRemainder::default(),
            vir_buf_remainder: FmSampleRemainder::default(),
            dac_clock_cycle: 0,
            dac_clock_remainder: FmSampleRemainder::default(),
            wavestar_seq_index: 0,
            wavestar_value: 0xFF,
        }
    }
}

/// PCM86 DAC FIFO-based PCM playback engine.
struct Pcm86 {
    state: Pcm86State,
    buffer: Box<[u8; PCM86_BUFSIZE]>,
    pcm_resampler: ResamplerFir,
    /// Persistent buffer accumulating drained PCM samples across calls.
    /// The resampler consumes from this; the remainder carries over,
    /// bridging per-chunk count variations from rate ratio truncation.
    pcm_input_buffer: Vec<f32>,
    pcm_resample_output: Vec<f32>,
    pending_irq_change: Option<bool>,
    /// CPU clock multiple (cpu_clock_hz / baseclock). Used for IRQ retry delay.
    clock_multiple: u64,
    /// Set by pcm86_timer_expired or control register writes to request
    /// rescheduling of the PCM86 IRQ timer in drain_actions.
    needs_reschedule: bool,
    /// Underrun drift accumulator (milliseconds of persistent `real_buf < vir_buf`).
    /// Used by `calculate_next_irq_cycle` to weight the timer towards real_buf
    /// when drift persists.
    buf_under_flag: i32,
    /// CPU cycle of last `reconcile_buffers` call, for computing drift time steps.
    last_checkbuf_cycle: u64,
    /// CPU cycle of the last `generate_samples` call, for computing per-frame
    /// drain counts.
    last_generate_cycle: u64,
}

impl Pcm86 {
    fn new(sample_rate: u32, cpu_clock_hz: u32, machine_model: MachineModel) -> Self {
        let initial_pcm_rate = PCM86_RATES[0];
        let resampler = ResamplerFir::new_from_hz(
            2,
            initial_pcm_rate,
            sample_rate,
            REAMPLER_LATENCY,
            RESAMPLER_ATTENUTATION,
        );
        let output_size = resampler.buffer_size_output();
        let baseclock = machine_model.pit_clock_hz();
        let multiple = (cpu_clock_hz / baseclock).max(1) as u64;
        Self {
            state: Pcm86State {
                data_write_irq_wait: 20000 * multiple,
                ..Pcm86State::default()
            },
            buffer: Box::new([0u8; PCM86_BUFSIZE]),
            pcm_resampler: resampler,
            pcm_input_buffer: Vec::new(),
            pcm_resample_output: vec![0.0; output_size],
            pending_irq_change: None,
            clock_multiple: multiple,
            needs_reschedule: false,
            buf_under_flag: 0,
            last_checkbuf_cycle: 0,
            last_generate_cycle: 0,
        }
    }

    fn pcm_rate(&self) -> u32 {
        PCM86_RATES[(self.state.fifo & 7) as usize]
    }

    fn is_playing(&self) -> bool {
        self.state.fifo & 0x80 != 0
    }

    fn irq_enabled(&self) -> bool {
        self.state.fifo & 0x20 != 0
    }

    fn read_port(
        &mut self,
        port: u16,
        current_cycle: u64,
        cpu_clock_hz: u32,
        pcm86_irq_pending: bool,
    ) -> u8 {
        match port {
            0xA460 => self.state.sound_flags,
            0xA466 => {
                self.update_buffer_state(current_cycle, cpu_clock_hz);
                let mut ret = 0u8;
                if self.state.vir_buf >= PCM86_LOGICAL_BUF {
                    ret |= 0x80;
                } else if self.state.vir_buf <= self.state.step_mask as i32 {
                    ret |= 0x40;
                }
                if self.dac_clock_high(current_cycle, cpu_clock_hz) {
                    ret |= 0x01;
                }
                ret
            }
            0xA468 => {
                self.update_buffer_state(current_cycle, cpu_clock_hz);
                let mut ret = self.state.fifo & !0x10;
                if self.irq_condition_met(current_cycle, pcm86_irq_pending) {
                    ret |= 0x10;
                }
                ret
            }
            0xA46A => self.state.dactrl,
            0xA462 => 0xFF,
            0xA464 => {
                if self.state.wavestar_seq_index != WAVESTAR_SEQUENCE.len() as u8 {
                    self.state.wavestar_value = 0xFF;
                }
                let ret = self.state.wavestar_value;
                if self.state.wavestar_value == 0x00 {
                    self.state.wavestar_value = 0xFF;
                } else {
                    self.state.wavestar_value = 0x00;
                }
                ret
            }
            0xA66E => self.state.pcm_mute,
            _ => 0x00,
        }
    }

    fn irq_condition_met(&mut self, current_cycle: u64, pcm86_irq_pending: bool) -> bool {
        if !self.irq_enabled() {
            return false;
        }
        // If the timer callback already confirmed the IRQ condition, report it
        // immediately regardless of the data-write wait. The wait only gates
        // the automatic buffer-level detection path below, not the
        // timer-confirmed path. Without this, edge-triggered PICs lose the
        // interrupt: the ISR misses bit 4, pcm86_irq_asserted stays high,
        // and no new edge is generated for subsequent FM timer IRQs.
        if self.state.irq_flag {
            return true;
        }
        if current_cycle.wrapping_sub(self.state.last_clock_for_wait)
            < self.state.data_write_irq_wait
        {
            return false;
        }
        // Only set irq_flag from the buffer condition if no PCM86 timer event
        // is pending.
        if !pcm86_irq_pending
            && (self.state.vir_buf <= self.state.fifo_size
                || (self.state.real_buf > self.state.step_mask as i32
                    && self.state.real_buf <= self.state.fifo_size))
        {
            self.state.irq_flag = true;
            return true;
        }
        false
    }

    fn write_port(
        &mut self,
        port: u16,
        value: u8,
        current_cycle: u64,
        cpu_clock_hz: u32,
        sample_rate: u32,
    ) {
        match port {
            0xA460 => {
                // Bits 7-2 are board ID/read-only. Bits 1:0 are mask/extend controls.
                self.state.sound_flags = (self.state.sound_flags & 0xFC) | (value & 0x03);
            }
            0xA466 => {
                let line = ((value >> 5) & 0x07) as usize;
                if line == 5 {
                    self.update_buffer_state(current_cycle, cpu_clock_hz);
                }
                self.state.vol[line] = value & 0x0F;
            }
            0xA468 => {
                self.update_buffer_state(current_cycle, cpu_clock_hz);
                let old_fifo = self.state.fifo;
                // FIFO reset when bit 3 transitions 0->1.
                if (value & 0x08) != 0 && (old_fifo & 0x08) == 0 {
                    self.state.write_pos = 0;
                    self.state.read_pos = 0;
                    self.state.real_buf = 0;
                    self.state.vir_buf = 0;
                    self.state.audio_frame_start_cycle = current_cycle;
                    self.state.sample_remainder = FmSampleRemainder::default();
                    self.state.vir_buf_remainder = FmSampleRemainder::default();
                    self.state.dac_clock_cycle = current_cycle;
                    self.state.dac_clock_remainder = FmSampleRemainder::default();
                    self.state.last_clock_for_wait = current_cycle;
                }
                // IRQ clear when bit 4 is written as 0.
                if value & 0x10 == 0 {
                    self.state.irq_flag = false;
                    self.pending_irq_change = Some(false);
                    // If buffer is empty at IRQ clear, add long wait.
                    if self.state.vir_buf == 0 {
                        self.state.last_clock_for_wait = current_cycle;
                    }
                }
                // Force IRQ if buffer already below threshold (Police Nauts workaround).
                // Checked BEFORE storing fifo, unconditionally.
                if self.state.vir_buf <= self.state.fifo_size {
                    self.state.irq_flag = true;
                }
                self.state.fifo = value;
                // Reset audio clock when playback transitions off->on.
                if (old_fifo ^ value) & 0x80 != 0 && value & 0x80 != 0 {
                    self.state.audio_frame_start_cycle = current_cycle;
                    self.state.sample_remainder = FmSampleRemainder::default();
                    self.state.vir_buf_remainder = FmSampleRemainder::default();
                    self.state.dac_clock_cycle = current_cycle;
                    self.state.dac_clock_remainder = FmSampleRemainder::default();
                }
                // Update rescue and resampler if rate changed.
                if (old_fifo ^ value) & 7 != 0 {
                    self.state.rescue =
                        PCM86_RESCUE_TABLE[(value & 7) as usize] << self.state.step_bit;
                    self.pcm_resampler = ResamplerFir::new_from_hz(
                        2,
                        self.pcm_rate(),
                        sample_rate,
                        REAMPLER_LATENCY,
                        RESAMPLER_ATTENUTATION,
                    );
                    self.pcm_resample_output
                        .resize(self.pcm_resampler.buffer_size_output(), 0.0);
                }
                // Reschedule the PCM86 timer when an IRQ request is pending.
                if self.state.reqirq {
                    self.needs_reschedule = true;
                }
            }
            0xA46A => {
                self.update_buffer_state(current_cycle, cpu_clock_hz);
                if self.state.fifo & 0x20 != 0 {
                    if value != 0xFF {
                        self.state.fifo_size = (i32::from(value) + 1) << 7;
                    } else {
                        self.state.fifo_size = 0x7FFC;
                    }
                } else {
                    // WinNT 3.5 workaround: reject abnormal settings.
                    if value & 0x0F == 0x0F {
                        return;
                    }
                    self.state.dactrl = value;
                    self.state.step_bit = PCM86_BITS[((value >> 4) & 7) as usize];
                    self.state.step_mask = (1u32 << self.state.step_bit) - 1;
                    self.state.rescue =
                        PCM86_RESCUE_TABLE[(self.state.fifo & 7) as usize] << self.state.step_bit;
                }
                // Reschedule the PCM86 timer when an IRQ request is pending.
                if self.state.reqirq {
                    self.needs_reschedule = true;
                }
            }
            0xA46C => {
                if self.state.vir_buf < PCM86_LOGICAL_BUF {
                    self.state.vir_buf += 1;
                }
                self.buffer[self.state.write_pos as usize] = value;
                self.state.write_pos = (self.state.write_pos + 1) & PCM86_BUFMASK;
                self.state.real_buf += 1;

                // Overflow protection: discard oldest data.
                if self.state.real_buf >= PCM86_LOGICAL_BUF + self.state.rescue {
                    self.state.real_buf -= 4;
                    self.state.read_pos = (self.state.read_pos + 4) & PCM86_BUFMASK;
                }

                self.state.reqirq = true;

                let mut add_clock: u64 = 0;
                if self.state.fifo_size < 8192 {
                    add_clock = self.state.data_write_irq_wait
                        - self.state.data_write_irq_wait * self.state.fifo_size as u64 / 8192;
                }
                if self.state.vir_buf > self.state.fifo_size * 2
                    || self.state.vir_buf >= PCM86_LOGICAL_BUF
                {
                    add_clock = self.state.data_write_irq_wait;
                }
                self.state.last_clock_for_wait = current_cycle + add_clock;
            }
            0xA462 => {
                let seq_len = WAVESTAR_SEQUENCE.len() as u8;
                let idx = self.state.wavestar_seq_index;
                if idx < seq_len && value == WAVESTAR_SEQUENCE[idx as usize] {
                    self.state.wavestar_seq_index = idx + 1;
                    if self.state.wavestar_seq_index == seq_len {
                        self.state.wavestar_value = 0x0B;
                    }
                } else if value == WAVESTAR_SEQUENCE[0] {
                    self.state.wavestar_seq_index = 1;
                } else {
                    self.state.wavestar_seq_index = 0;
                }
            }
            0xA464 => {
                let seq_len = WAVESTAR_SEQUENCE.len() as u8;
                if self.state.wavestar_seq_index == seq_len {
                    if value == 0x04 {
                        self.state.wavestar_value = 0x0C;
                        // TODO: implement CS4231 WSS codec and remapping for full Q-Vision WaveStar support.
                    } else {
                        self.state.wavestar_value = 0x08;
                    }
                }
                if value == 0x09 {
                    self.state.wavestar_value = 0xFF;
                }
            }
            0xA66E => {
                self.state.pcm_mute = value;
            }
            _ => {}
        }
    }

    fn update_buffer_state(&mut self, current_cycle: u64, cpu_clock_hz: u32) {
        if !self.is_playing() {
            return;
        }
        let elapsed = current_cycle.saturating_sub(self.state.audio_frame_start_cycle);
        if elapsed == 0 {
            return;
        }
        let rate = self.pcm_rate() as u64;
        let exact_samples =
            elapsed as f64 * rate as f64 / f64::from(cpu_clock_hz) + self.state.vir_buf_remainder.0;
        let samples_elapsed = exact_samples as i64;
        self.state.vir_buf_remainder = FmSampleRemainder(exact_samples - samples_elapsed as f64);
        let dec_value = samples_elapsed << self.state.step_bit;
        if self.state.vir_buf as i64 - dec_value < self.state.vir_buf as i64 {
            self.state.vir_buf -= dec_value as i32;
        }
        if self.state.vir_buf < 0 {
            self.state.vir_buf &= self.state.step_mask as i32;
        }
        self.state.audio_frame_start_cycle = current_cycle;
    }

    fn dac_clock_high(&mut self, current_cycle: u64, cpu_clock_hz: u32) -> bool {
        let elapsed = current_cycle.saturating_sub(self.state.dac_clock_cycle);
        if elapsed > 0 {
            let exact_samples = elapsed as f64 * f64::from(self.pcm_rate())
                / f64::from(cpu_clock_hz)
                + self.state.dac_clock_remainder.0;
            let samples_elapsed = exact_samples as u64;
            self.state.dac_clock_remainder =
                FmSampleRemainder(exact_samples - samples_elapsed as f64);
            self.state.dac_clock_cycle = current_cycle;
        }
        self.state.dac_clock_remainder.0 >= 0.5
    }

    /// Reconcile vir_buf with real_buf after audio generation and accumulate
    /// drift for timer scheduling weighting.
    fn reconcile_buffers(&mut self, current_cycle: u64, cpu_clock_hz: u32) {
        // Compute elapsed time in milliseconds for drift accumulation.
        let flag_step = if self.last_checkbuf_cycle > 0 {
            let elapsed = current_cycle.saturating_sub(self.last_checkbuf_cycle);
            (elapsed as i64 * 1000 / cpu_clock_hz as i64) as i32
        } else {
            0
        };
        self.last_checkbuf_cycle = current_cycle;

        let deficit = self.state.real_buf - self.state.vir_buf;
        if deficit < 0 {
            // real_buf < vir_buf: underrun condition - accumulate drift.
            self.buf_under_flag = self.buf_under_flag.saturating_add(flag_step);

            // Critical underrun: real_buf fell significantly below vir_buf.
            if deficit <= -self.state.fifo_size && self.state.vir_buf < self.state.fifo_size {
                let adjustment = deficit & !3;
                self.state.vir_buf += adjustment;
            }
        } else {
            // Recovery: reset underrun accumulator.
            self.buf_under_flag = 0;

            // Overrun: real_buf significantly exceeds vir_buf - nudge upward.
            if self.state.vir_buf > self.state.fifo_size
                && self.state.real_buf > self.state.step_mask as i32
                && self.state.real_buf > self.state.vir_buf + self.state.fifo_size * 3
            {
                let adjustment =
                    (self.state.real_buf - (self.state.vir_buf - self.state.fifo_size * 3)) / 8;
                self.state.vir_buf += adjustment;
            }
        }
    }

    /// Calculate when vir_buf will drop to fifo_size, applying drift-weighted
    /// blending between virtual and real buffer counts.
    ///
    /// Returns `Some(fire_cycle)` if buffer is above threshold, `None` if IRQ
    /// should fire now.
    fn calculate_next_irq_cycle(&self, current_cycle: u64, cpu_clock_hz: u32) -> Option<u64> {
        if self.state.fifo & 0x80 == 0 {
            return None;
        }
        let cntv = self.state.vir_buf - self.state.fifo_size;
        if cntv <= 0 {
            return None;
        }
        let cntr = self.state.real_buf - self.state.fifo_size;

        // Apply drift weighting when real_buf has data and is falling behind
        // vir_buf. The buf_under_flag accumulator tracks how long the underrun
        // persists (in ms); higher values pull the timer closer to real_buf.
        let cnt = if self.state.real_buf > self.state.step_mask as i32 && cntr < cntv {
            if self.buf_under_flag > 32000 {
                ((cntv as i64 * 9 + cntr as i64) / 10) as i32
            } else if self.buf_under_flag > 4000 {
                ((cntv as i64 * 99 + cntr as i64) / 100) as i32
            } else {
                cntv
            }
        } else {
            cntv
        };

        if cnt <= 0 {
            return None;
        }

        let byte_count = (cnt + self.state.step_mask as i32) >> self.state.step_bit;
        if byte_count <= 0 {
            return None;
        }
        let rate = self.pcm_rate() as u64;
        let cycles = byte_count as u64 * cpu_clock_hz as u64 / rate;
        Some(current_cycle + cycles.max(1))
    }

    /// Fills `output` with interleaved stereo `[L, R, L, R, ...]` at the PCM
    /// source rate. Returns the number of floats written (always even).
    #[cfg(test)]
    fn drain_samples(&mut self, output: &mut [f32]) -> usize {
        drain_samples_core(&mut self.state, &self.buffer, output)
    }

    /// Drains PCM samples based on elapsed cycles, resamples them, and mixes
    /// into the output buffer at the given volume.
    fn generate_samples(
        &mut self,
        current_cycle: u64,
        cpu_clock_hz: u32,
        volume: f32,
        output: &mut [f32],
    ) {
        self.update_buffer_state(current_cycle, cpu_clock_hz);
        self.reconcile_buffers(current_cycle, cpu_clock_hz);

        let frame_cycles = current_cycle.saturating_sub(self.last_generate_cycle);
        if self.is_playing() && self.state.real_buf > 0 && frame_cycles > 0 {
            let pcm_rate = self.pcm_rate();
            let exact_pcm = (frame_cycles as f64 * pcm_rate as f64) / f64::from(cpu_clock_hz)
                + self.state.sample_remainder.0;
            let pcm_count = exact_pcm as usize;
            self.state.sample_remainder = FmSampleRemainder(exact_pcm - pcm_count as f64);

            if pcm_count > 0 {
                let drain_interleaved = pcm_count * 2;
                let start = self.pcm_input_buffer.len();
                self.pcm_input_buffer.resize(start + drain_interleaved, 0.0);
                drain_samples_core(
                    &mut self.state,
                    &self.buffer,
                    &mut self.pcm_input_buffer[start..start + drain_interleaved],
                );
            }
        }

        if !self.pcm_input_buffer.is_empty() {
            let total_interleaved = self.pcm_input_buffer.len();
            let mut input_offset = 0;
            let mut output_offset = 0;
            let sample_count = output.len();
            while input_offset < total_interleaved && output_offset < sample_count {
                let remaining_output =
                    (sample_count - output_offset).min(self.pcm_resample_output.len());
                let out_buf = &mut self.pcm_resample_output[..remaining_output];
                let Ok((consumed, produced)) = self.pcm_resampler.resample(
                    &self.pcm_input_buffer[input_offset..total_interleaved],
                    out_buf,
                ) else {
                    break;
                };
                for i in 0..produced {
                    output[output_offset + i] += out_buf[i] * volume;
                }
                input_offset += consumed;
                output_offset += produced;
                if consumed == 0 {
                    break;
                }
            }
            self.pcm_input_buffer.drain(..input_offset);
        }

        self.last_generate_cycle = current_cycle;
    }

    fn advance_generate_cycle(&mut self, current_cycle: u64) {
        self.last_generate_cycle = current_cycle;
    }
}

#[inline(always)]
fn read_sample_8bit(state: &Pcm86State, buffer: &[u8; PCM86_BUFSIZE]) -> f32 {
    let raw = buffer[state.read_pos as usize] as i8;
    raw as f32 / 128.0
}

#[inline(always)]
fn read_sample_16bit(state: &Pcm86State, buffer: &[u8; PCM86_BUFSIZE]) -> f32 {
    let msb = buffer[state.read_pos as usize];
    let lsb = buffer[((state.read_pos + 1) & PCM86_BUFMASK) as usize];
    let raw = (msb as i8 as i16) << 8 | lsb as i16;
    raw as f32 / 32768.0
}

/// Fills `output` with interleaved stereo `[L, R, L, R, ...]` at the PCM
/// source rate. Returns the number of floats written (always even).
fn drain_samples_core(
    state: &mut Pcm86State,
    buffer: &[u8; PCM86_BUFSIZE],
    output: &mut [f32],
) -> usize {
    let count = output.len() / 2;
    if state.fifo & 0x80 == 0 || state.real_buf <= 0 || count == 0 {
        output[..count * 2].fill(0.0);
        return count * 2;
    }

    let vol = if state.pcm_mute & 0x01 != 0 {
        0.0
    } else {
        pcm86_volume(state.vol[5])
    };
    let mode = state.dactrl & 0x70;
    let frame_bytes = bytes_per_frame(mode);

    for i in 0..count {
        if frame_bytes == 0 || state.real_buf < frame_bytes as i32 {
            output[i * 2] = 0.0;
            output[i * 2 + 1] = 0.0;
            continue;
        }

        let (left, right) = match mode {
            0x10 => {
                let s = read_sample_16bit(state, buffer);
                (0.0, s)
            }
            0x20 => {
                let s = read_sample_16bit(state, buffer);
                (s, 0.0)
            }
            0x30 => {
                let l = read_sample_16bit(state, buffer);
                let r_msb = buffer[((state.read_pos + 2) & PCM86_BUFMASK) as usize];
                let r_lsb = buffer[((state.read_pos + 3) & PCM86_BUFMASK) as usize];
                let r_raw = (r_msb as i8 as i16) << 8 | r_lsb as i16;
                (l, r_raw as f32 / 32768.0)
            }
            0x50 => {
                let s = read_sample_8bit(state, buffer);
                (0.0, s)
            }
            0x60 => {
                let s = read_sample_8bit(state, buffer);
                (s, 0.0)
            }
            0x70 => {
                let l = read_sample_8bit(state, buffer);
                let r_raw = buffer[((state.read_pos + 1) & PCM86_BUFMASK) as usize] as i8;
                (l, r_raw as f32 / 128.0)
            }
            _ => (0.0, 0.0),
        };

        state.read_pos = (state.read_pos + frame_bytes as u32) & PCM86_BUFMASK;
        state.real_buf -= frame_bytes as i32;
        if state.real_buf < 0 {
            state.real_buf = 0;
        }

        output[i * 2] = left * vol;
        output[i * 2 + 1] = right * vol;
    }

    count * 2
}

fn bytes_per_frame(mode: u8) -> usize {
    match mode {
        0x10 | 0x20 => 2,
        0x30 => 4,
        0x50 | 0x60 => 1,
        0x70 => 2,
        _ => 0,
    }
}

fn pcm86_volume(level: u8) -> f32 {
    let inverted = 15u8.saturating_sub(level & 0x0F);
    inverted as f32 / 15.0
}

/// Action the bus must process after a sound board operation.
#[derive(Clone, Copy)]
pub enum Soundboard86Action {
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
    /// Set by `timer_expired` (FM timer callback), consumed by `drain_actions`
    /// to trigger a PCM86 IRQ piggyback check.
    fm_timer_just_fired: bool,
    action_buffer: Vec<Soundboard86Action>,
}

impl Soundboard86 {
    #[inline]
    fn opna_masked(&self) -> bool {
        self.pcm86.state.sound_flags & 0x02 != 0
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
        let mut core = OpnFm::<Ym2608>::new(cpu_clock_hz, sample_rate);
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

    /// Reads a PCM86 port.
    pub fn pcm86_read(
        &mut self,
        port: u16,
        current_cycle: u64,
        cpu_clock_hz: u32,
        pcm86_irq_pending: bool,
    ) -> u8 {
        self.pcm86
            .read_port(port, current_cycle, cpu_clock_hz, pcm86_irq_pending)
    }

    /// Writes a PCM86 port.
    pub fn pcm86_write(&mut self, port: u16, value: u8, current_cycle: u64, cpu_clock_hz: u32) {
        if port == 0xA460 {
            self.state.extend_enabled = value & 1 != 0;
        }
        // Keep the bridge cycle in sync so drain_actions can schedule PCM86
        // timers relative to the correct point in time.
        self.chip_action_cycle = current_cycle;
        self.pcm86
            .write_port(port, value, current_cycle, cpu_clock_hz, self.sample_rate);
    }

    /// Called when the Pcm86Irq scheduler event fires.
    ///
    /// This method never sets `reqirq = true` - that is only done by the data
    /// write handler and (as a workaround) inside `drain_actions` when the
    /// buffer is already at or below the threshold.
    pub fn pcm86_timer_expired(&mut self, current_cycle: u64, cpu_clock_hz: u32) {
        // Keep the bridge cycle in sync so drain_actions can schedule PCM86
        // timers relative to the correct point in time.
        self.chip_action_cycle = current_cycle;

        if self.pcm86.state.reqirq {
            self.pcm86.update_buffer_state(current_cycle, cpu_clock_hz);
            self.pcm86.reconcile_buffers(current_cycle, cpu_clock_hz);

            let adjusted_buf = ((self.pcm86.state.vir_buf as i64 * 4
                + self.pcm86.state.real_buf as i64)
                / 5) as i32;
            if self.pcm86.state.vir_buf <= self.pcm86.state.fifo_size
                || (self.pcm86.state.real_buf > self.pcm86.state.step_mask as i32
                    && adjusted_buf <= self.pcm86.state.fifo_size)
            {
                self.pcm86.state.reqirq = false;
                self.pcm86.state.irq_flag = true;
                self.pcm86.pending_irq_change = Some(true);
            } else {
                self.pcm86.needs_reschedule = true;
            }
        } else {
            self.pcm86.needs_reschedule = true;
        }
    }

    /// Drains pending actions from the chip bridge and PCM86, merging IRQ sources.
    ///
    /// `pcm86_irq_pending` indicates whether a `Pcm86Irq` event is currently
    /// scheduled in the system scheduler.
    pub fn drain_actions(&mut self, pcm86_irq_pending: bool) -> &[Soundboard86Action] {
        self.action_buffer.clear();

        let opna_masked = self.opna_masked();
        let current_cycle = self.chip_action_cycle;
        let cpu_clock_hz = self.cpu_clock_hz;

        // Snapshot previous merged IRQ state before processing updates.
        let was_merged = (self.state.irq_asserted && !opna_masked) || self.state.pcm86_irq_asserted;

        self.core.set_action_cycle(current_cycle);
        let timers: [Option<FmTimerAction>; 2] = {
            let actions = self.core.drain_timers();
            let mut out = [None, None];
            for (slot, action) in out.iter_mut().zip(actions.iter()) {
                *slot = Some(*action);
            }
            out
        };
        for action in timers.into_iter().flatten() {
            let kind = match action {
                FmTimerAction::Cancel { timer_id } | FmTimerAction::Schedule { timer_id, .. } => {
                    if timer_id == 0 {
                        EventKind::FmTimerA
                    } else {
                        EventKind::FmTimerB
                    }
                }
            };
            match action {
                FmTimerAction::Cancel { .. } => self
                    .action_buffer
                    .push(Soundboard86Action::CancelTimer { kind }),
                FmTimerAction::Schedule { fire_cycle, .. } => {
                    self.action_buffer
                        .push(Soundboard86Action::ScheduleTimer { kind, fire_cycle });
                }
            }
        }

        if let Some(asserted) = self.core.take_irq_change() {
            self.state.irq_asserted = asserted;
        }

        // Process PCM86 IRQ changes.
        if let Some(asserted) = self.pcm86.pending_irq_change.take() {
            self.state.pcm86_irq_asserted = asserted;
        }

        // FM timer piggyback: when an FM timer fires, check if PCM86's IRQ
        // condition is also met and include it in the merged IRQ.
        let fm_timer_fired = self.fm_timer_just_fired;
        self.fm_timer_just_fired = false;
        if fm_timer_fired && !self.state.pcm86_irq_asserted && !pcm86_irq_pending {
            self.pcm86.update_buffer_state(current_cycle, cpu_clock_hz);
            if self.pcm86.irq_condition_met(current_cycle, false) {
                self.state.pcm86_irq_asserted = true;
            }
        }

        // Schedule PCM86 IRQ event.
        // Only triggered by the timer callback and FIFO/DAC control register
        // writes - data byte writes (0xA46C) set reqirq but do not reschedule.
        if self.pcm86.needs_reschedule && self.pcm86.state.fifo & 0x80 != 0 {
            self.pcm86.needs_reschedule = false;

            if let Some(fire_cycle) = self
                .pcm86
                .calculate_next_irq_cycle(current_cycle, cpu_clock_hz)
            {
                // Buffer above threshold - schedule for when it drains.
                self.action_buffer.push(Soundboard86Action::ScheduleTimer {
                    kind: EventKind::Pcm86Irq,
                    fire_cycle,
                });
            } else if self.pcm86.state.reqirq {
                // Buffer already at/below threshold and reqirq set - fire immediately.
                self.action_buffer.push(Soundboard86Action::ScheduleTimer {
                    kind: EventKind::Pcm86Irq,
                    fire_cycle: current_cycle + 1,
                });
            } else {
                // Buffer at/below threshold, reqirq not set, still playing.
                // Workaround: set reqirq and retry after a delay to avoid
                // infinite IRQ loops on systems like WinNT4.
                self.pcm86.state.reqirq = true;
                self.action_buffer.push(Soundboard86Action::ScheduleTimer {
                    kind: EventKind::Pcm86Irq,
                    fire_cycle: current_cycle + 100 * self.pcm86.clock_multiple,
                });
            }
        } else {
            self.pcm86.needs_reschedule = false;
        }

        // Compute merged IRQ (OR of both sources, respecting OPNA mask).
        let merged = (self.state.irq_asserted && !opna_masked) || self.state.pcm86_irq_asserted;
        if merged != was_merged {
            if merged {
                self.action_buffer.push(Soundboard86Action::AssertIrq {
                    irq: self.state.irq_line,
                });
            } else {
                self.action_buffer.push(Soundboard86Action::DeassertIrq {
                    irq: self.state.irq_line,
                });
            }
        }

        self.action_buffer.as_slice()
    }

    /// Generates resampled stereo audio and mixes it into `output`.
    ///
    /// `output` is interleaved stereo (`[L, R, L, R, …]`).
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
            self.pcm86.advance_generate_cycle(current_cycle);
        } else {
            self.pcm86
                .generate_samples(current_cycle, cpu_clock_hz, volume, output);
        }
    }

    /// Creates a snapshot of the current state for save/restore.
    pub fn save_state(&self) -> Soundboard86State {
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

    /// Restores from a saved state, recreating the ymfm chip.
    pub fn load_state(
        &mut self,
        saved: &Soundboard86State,
        cpu_clock_hz: u32,
        sample_rate: u32,
        current_cycle: u64,
        rhythm_rom: Option<&[u8]>,
    ) {
        self.state = saved.clone();
        self.cpu_clock_hz = cpu_clock_hz;
        self.sample_rate = sample_rate;
        self.chip_action_cycle = current_cycle;
        self.pcm86.state = saved.pcm86.clone();
        self.pcm86.pending_irq_change = None;
        self.pcm86.last_generate_cycle = saved.audio_frame_start_cycle;
        self.pcm86.pcm_input_buffer.clear();
        self.pcm86.pcm_resampler = ResamplerFir::new_from_hz(
            2,
            self.pcm86.pcm_rate(),
            sample_rate,
            REAMPLER_LATENCY,
            RESAMPLER_ATTENUTATION,
        );
        self.pcm86
            .pcm_resample_output
            .resize(self.pcm86.pcm_resampler.buffer_size_output(), 0.0);
        // TODO: Save/restore ymfm internal state
        let rom_data = rhythm_rom.unwrap_or(EVOLVED_RHYTHM_ROM.as_slice());
        self.core.reload(cpu_clock_hz, sample_rate, current_cycle);
        self.core.chip_mut().set_adpcm_a_rom(rom_data);
        if saved.adpcm_ram {
            self.core
                .chip_mut()
                .set_adpcm_b_ram(vec![0; ADPCM_B_RAM_SIZE]);
        }
        self.core.set_timing(OpnFmTiming {
            sample_remainder: saved.sample_remainder,
            fm_sync_cursor: saved.fm_sync_cursor,
            busy_end_cycle: saved.busy_end_cycle,
            audio_frame_start_cycle: saved.audio_frame_start_cycle,
            irq_asserted: saved.irq_asserted,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pcm86() -> Pcm86 {
        let mut pcm = Pcm86::new(48000, 8_000_000, MachineModel::PC9801RA);
        pcm.state.pcm_mute = 0x00;
        pcm
    }

    fn write_bytes(pcm: &mut Pcm86, data: &[u8]) {
        for &b in data {
            if pcm.state.vir_buf < PCM86_LOGICAL_BUF {
                pcm.state.vir_buf += 1;
            }
            pcm.buffer[pcm.state.write_pos as usize] = b;
            pcm.state.write_pos = (pcm.state.write_pos + 1) & PCM86_BUFMASK;
            pcm.state.real_buf += 1;
        }
    }

    fn enable_playback(pcm: &mut Pcm86) {
        pcm.state.fifo = 0x80;
    }

    #[test]
    fn drain_16bit_big_endian_left_only() {
        let mut pcm = make_pcm86();
        pcm.state.dactrl = 0x20; // 16-bit, L only
        pcm.state.step_bit = PCM86_BITS[(0x20 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.vol[5] = 0; // max volume
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
        pcm.state.dactrl = 0x10; // 16-bit, R only
        pcm.state.step_bit = PCM86_BITS[(0x10 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.vol[5] = 0;
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
        pcm.state.dactrl = 0x30; // 16-bit stereo
        pcm.state.step_bit = PCM86_BITS[(0x30 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.vol[5] = 0;
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
        pcm.state.dactrl = 0x30; // 16-bit stereo
        pcm.state.step_bit = PCM86_BITS[(0x30 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.vol[5] = 0;
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
        pcm.state.dactrl = 0x60; // 8-bit, L only
        pcm.state.step_bit = PCM86_BITS[(0x60 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.vol[5] = 0;
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
        pcm.state.dactrl = 0x50; // 8-bit, R only
        pcm.state.step_bit = PCM86_BITS[(0x50 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.vol[5] = 0;
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
        pcm.state.dactrl = 0x70; // 8-bit stereo
        pcm.state.step_bit = PCM86_BITS[(0x70 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.vol[5] = 0;
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
            pcm.state.dactrl = mode;
            pcm.state.step_bit = PCM86_BITS[((mode >> 4) & 7) as usize];
            pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
            pcm.state.vol[5] = 0;
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
        pcm.state.dactrl = 0x20; // 16-bit L only
        pcm.state.step_bit = PCM86_BITS[(0x20 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.vol[5] = 0;
        pcm.state.pcm_mute = 0x01;
        enable_playback(&mut pcm);

        write_bytes(&mut pcm, &[0x7F, 0x00, 0x7F, 0x00]);
        assert_eq!(pcm.state.real_buf, 4);

        let mut out = [0.0f32; 4];
        pcm.drain_samples(&mut out);
        assert_eq!(out.len(), 4);
        for &s in &out {
            assert_eq!(s, 0.0, "muted output should be silent");
        }
        // FIFO should have been consumed even though muted.
        assert_eq!(pcm.state.real_buf, 0);
    }

    #[test]
    fn drain_with_volume_attenuation() {
        let mut pcm = make_pcm86();
        pcm.state.dactrl = 0x20; // 16-bit L only
        pcm.state.step_bit = PCM86_BITS[(0x20 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        // Volume 15 = minimum (inverted: 15 - 15 = 0 -> 0.0)
        pcm.state.vol[5] = 15;
        enable_playback(&mut pcm);

        write_bytes(&mut pcm, &[0x7F, 0x00]);
        let mut out = [0.0f32; 2];
        pcm.drain_samples(&mut out);
        assert_eq!(out[0], 0.0, "volume 15 should silence output");

        // Volume 0 = maximum (inverted: 15 - 0 = 15 -> 1.0)
        pcm.state.vol[5] = 0;
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
        pcm.state.dactrl = 0x30; // 16-bit stereo (4 bytes per frame)
        pcm.state.step_bit = PCM86_BITS[(0x30 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.vol[5] = 0;
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
        pcm.state.dactrl = 0x20;
        pcm.state.step_bit = PCM86_BITS[(0x20 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.vol[5] = 0;
        // Do NOT enable playback (fifo bit 7 = 0).

        write_bytes(&mut pcm, &[0x7F, 0x00]);
        let mut out = [0.0f32; 2];
        pcm.drain_samples(&mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 0.0);
    }

    #[test]
    fn irq_condition_met_when_vir_buf_below_threshold() {
        let mut pcm = make_pcm86();
        pcm.state.fifo = 0xA0; // play + IRQ enable
        pcm.state.fifo_size = 3;
        pcm.state.data_write_irq_wait = 0; // disable wait for test

        // vir_buf = 5, above threshold.
        pcm.state.vir_buf = 5;
        assert!(!pcm.irq_condition_met(100, false));

        // vir_buf = 3, at threshold.
        pcm.state.vir_buf = 3;
        assert!(pcm.irq_condition_met(100, false));

        // vir_buf = 0, below threshold.
        pcm.state.vir_buf = 0;
        assert!(pcm.irq_condition_met(100, false));
    }

    #[test]
    fn irq_condition_not_met_when_irq_disabled() {
        let mut pcm = make_pcm86();
        pcm.state.fifo = 0x80; // play, but IRQ NOT enabled (bit 5 = 0)
        pcm.state.fifo_size = 3;
        pcm.state.vir_buf = 0;
        pcm.state.data_write_irq_wait = 0;
        assert!(!pcm.irq_condition_met(100, false));
    }

    #[test]
    fn port_a466_dac_clock_toggles_during_tight_polling() {
        let mut pcm = make_pcm86();
        pcm.state.fifo = 0x80;

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
        pcm.state.fifo = 0xA0;
        pcm.state.fifo_size = 3;
        pcm.state.vir_buf = 0;
        pcm.state.data_write_irq_wait = 1000;
        pcm.state.last_clock_for_wait = 50;

        // Only 50 cycles elapsed, need 1000 -> not met.
        assert!(!pcm.irq_condition_met(100, false));

        // 1050 cycles elapsed -> met.
        assert!(pcm.irq_condition_met(1050, false));
    }

    #[test]
    fn buffer_overflow_protection() {
        let mut pcm = make_pcm86();

        // Fill past physical buffer size.
        for i in 0..PCM86_BUFSIZE + 100 {
            if pcm.state.vir_buf < PCM86_LOGICAL_BUF {
                pcm.state.vir_buf += 1;
            }
            pcm.buffer[pcm.state.write_pos as usize] = (i & 0xFF) as u8;
            pcm.state.write_pos = (pcm.state.write_pos + 1) & PCM86_BUFMASK;
            pcm.state.real_buf += 1;
            if pcm.state.real_buf >= PCM86_BUFSIZE as i32 {
                pcm.state.real_buf -= 4;
                pcm.state.read_pos = (pcm.state.read_pos + 4) & PCM86_BUFMASK;
            }
        }
        assert!(
            pcm.state.real_buf < PCM86_BUFSIZE as i32,
            "real_buf should be bounded: {}",
            pcm.state.real_buf
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
        pcm.state.dactrl = 0x20; // 16-bit L only
        pcm.state.step_bit = PCM86_BITS[(0x20 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.vol[5] = 0;
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
        pcm.state.dactrl = 0x60; // 8-bit L only
        pcm.state.step_bit = PCM86_BITS[(0x60 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.vol[5] = 0;
        enable_playback(&mut pcm);

        // Position write and read near the end of the buffer.
        pcm.state.write_pos = PCM86_BUFMASK;
        pcm.state.read_pos = PCM86_BUFMASK;

        // Write 3 bytes across the wrap boundary.
        let data = [0x20u8, 0x40, 0x60]; // +32, +64, +96 as i8
        for &b in &data {
            pcm.buffer[pcm.state.write_pos as usize] = b;
            pcm.state.write_pos = (pcm.state.write_pos + 1) & PCM86_BUFMASK;
            pcm.state.real_buf += 1;
            pcm.state.vir_buf += 1;
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
        pcm.state.dactrl = 0x60; // 8-bit L only
        pcm.state.step_bit = PCM86_BITS[(0x60 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        enable_playback(&mut pcm);
        pcm.state.audio_frame_start_cycle = 0;

        write_bytes(&mut pcm, &[0x10, 0x20, 0x30, 0x40, 0x50]);
        let read_pos_before = pcm.state.read_pos;
        let real_buf_before = pcm.state.real_buf;
        let vir_buf_before = pcm.state.vir_buf;

        // Simulate enough time for ~2 samples at 44100 Hz with 8 MHz CPU.
        pcm.update_buffer_state(200, 8_000_000);

        assert_eq!(
            pcm.state.read_pos, read_pos_before,
            "read_pos must not change"
        );
        assert_eq!(
            pcm.state.real_buf, real_buf_before,
            "real_buf must not change"
        );
        assert!(
            pcm.state.vir_buf < vir_buf_before,
            "vir_buf should have decreased"
        );
    }

    #[test]
    fn update_buffer_state_clamps_vir_buf_with_stepmask() {
        let mut pcm = make_pcm86();
        pcm.state.dactrl = 0x30; // 16-bit stereo, step_bit=2
        pcm.state.step_bit = PCM86_BITS[(0x30 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1; // 3
        enable_playback(&mut pcm);
        pcm.state.audio_frame_start_cycle = 0;

        // Set vir_buf to a small value that will underflow.
        pcm.state.vir_buf = 2;

        // Simulate enough time to consume more than 2 bytes.
        pcm.update_buffer_state(10_000_000, 8_000_000);

        // After underflow, vir_buf should be masked with step_mask (3), not clamped to 0.
        assert_eq!(
            pcm.state.vir_buf,
            pcm.state.vir_buf & pcm.state.step_mask as i32,
            "vir_buf should be masked with step_mask on underflow"
        );
    }

    #[test]
    fn update_buffer_state_no_change_when_not_playing() {
        let mut pcm = make_pcm86();
        pcm.state.dactrl = 0x60;
        pcm.state.step_bit = PCM86_BITS[(0x60 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        // Do NOT enable playback.
        pcm.state.audio_frame_start_cycle = 0;

        write_bytes(&mut pcm, &[0x10, 0x20, 0x30]);
        let vir_buf_before = pcm.state.vir_buf;

        pcm.update_buffer_state(10_000_000, 8_000_000);

        assert_eq!(
            pcm.state.vir_buf, vir_buf_before,
            "vir_buf should not change when not playing"
        );
    }

    #[test]
    fn update_buffer_state_split_call_equivalence() {
        // A single update_buffer_state(T) and two calls update_buffer_state(T/2)
        // should produce the same vir_buf (within 1 byte) thanks to fractional tracking.
        let cpu_hz = 20_000_000u32;

        let mut single = make_pcm86();
        single.state.dactrl = 0x70; // 8-bit stereo
        single.state.step_bit = PCM86_BITS[(0x70 >> 4) & 7];
        single.state.step_mask = (1u32 << single.state.step_bit) - 1;
        enable_playback(&mut single);
        single.state.audio_frame_start_cycle = 0;
        write_bytes(&mut single, &[0u8; 1000]);
        let vir_before = single.state.vir_buf;

        let elapsed = 500_000u64; // 25ms at 20MHz
        single.update_buffer_state(elapsed, cpu_hz);
        let single_result = single.state.vir_buf;

        let mut split = make_pcm86();
        split.state.dactrl = 0x70;
        split.state.step_bit = PCM86_BITS[(0x70 >> 4) & 7];
        split.state.step_mask = (1u32 << split.state.step_bit) - 1;
        enable_playback(&mut split);
        split.state.audio_frame_start_cycle = 0;
        write_bytes(&mut split, &[0u8; 1000]);
        assert_eq!(split.state.vir_buf, vir_before);

        split.update_buffer_state(elapsed / 2, cpu_hz);
        split.update_buffer_state(elapsed, cpu_hz);
        let split_result = split.state.vir_buf;

        let diff = (single_result - split_result).unsigned_abs();
        assert!(
            diff <= 1,
            "split calls should match single call within 1 byte: single={single_result} split={split_result} (started at {vir_before})"
        );
    }

    #[test]
    fn update_buffer_state_accumulates_fractional_samples() {
        // Calling update_buffer_state with very small increments (each < 1 sample)
        // should still eventually decrement vir_buf via accumulated fractions.
        let cpu_hz = 20_000_000u32;

        let mut pcm = make_pcm86();
        pcm.state.dactrl = 0x70; // 8-bit stereo, rate index from fifo
        pcm.state.step_bit = PCM86_BITS[(0x70 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        enable_playback(&mut pcm);
        pcm.state.audio_frame_start_cycle = 0;
        write_bytes(&mut pcm, &[0u8; 100]);
        let vir_before = pcm.state.vir_buf;

        // At 44100 Hz (rate index 0, default), 1 sample = 20M/44100 = ~453 cycles.
        // Use increments of 100 cycles (< 1 sample each).
        let increment = 100u64;
        let mut cycle = 0u64;
        for _ in 0..1000 {
            cycle += increment;
            pcm.update_buffer_state(cycle, cpu_hz);
        }

        assert!(
            pcm.state.vir_buf < vir_before,
            "vir_buf should have decreased after many small increments: before={vir_before} after={}",
            pcm.state.vir_buf
        );
    }

    #[test]
    fn default_reset_values() {
        let state = Pcm86State::default();
        assert_eq!(state.fifo_size, 128, "fifo_size should default to 0x80");
        assert_eq!(state.dactrl, 0x32, "dactrl should default to 0x32");
        assert_eq!(state.step_bit, 2, "step_bit should default to 2");
        assert_eq!(state.step_mask, 3, "step_mask should default to 3");
        assert_eq!(
            state.pcm_mute, 0x00,
            "pcm_mute should default to 0x00 (unmuted)"
        );
    }

    #[test]
    fn port_a66e_readback_preserves_full_byte() {
        let mut pcm = make_pcm86();
        pcm.state.pcm_mute = 0xFE;

        let readback = pcm.read_port(0xA66E, 0, 8_000_000, false);
        assert_eq!(readback, 0xFE, "full byte should be returned on read");

        // Bit 0 is 0, so mute should be inactive.
        let vol = if pcm.state.pcm_mute & 0x01 != 0 {
            0.0
        } else {
            1.0
        };
        assert_eq!(vol, 1.0, "mute should be inactive when bit 0 is 0");
    }

    #[test]
    fn reconcile_buffers_adjusts_virbuf_upward_when_realbuf_leads() {
        let mut pcm = make_pcm86();
        pcm.state.fifo_size = 128;
        pcm.state.step_mask = 1;
        pcm.state.vir_buf = 500;
        // realbuf must exceed virbuf + fifosize*3 = 500 + 384 = 884
        pcm.state.real_buf = 1000;
        let old_virbuf = pcm.state.vir_buf;
        pcm.reconcile_buffers(1000, 8_000_000);
        assert!(
            pcm.state.vir_buf > old_virbuf,
            "virbuf should increase: was {old_virbuf}, now {}",
            pcm.state.vir_buf
        );
        // Expected: adjustment = (1000 - (500 - 384)) / 8 = (1000 - 116) / 8 = 110
        assert_eq!(pcm.state.vir_buf, 610);
    }

    #[test]
    fn reconcile_buffers_no_upward_adjustment_when_realbuf_close() {
        let mut pcm = make_pcm86();
        pcm.state.fifo_size = 128;
        pcm.state.step_mask = 1;
        pcm.state.vir_buf = 500;
        // realbuf NOT exceeding virbuf + fifosize*3 = 884
        pcm.state.real_buf = 800;
        pcm.reconcile_buffers(1000, 8_000_000);
        assert_eq!(pcm.state.vir_buf, 500, "virbuf should not change");
    }

    #[test]
    fn pcm86_timer_expired_uses_reschedule_when_condition_not_met() {
        let mut sb = Soundboard86::new(8_000_000, 48000, None, false, MachineModel::PC9801RA);
        sb.pcm86.state.reqirq = true;
        sb.pcm86.state.fifo_size = 128;
        sb.pcm86.state.step_mask = 3;
        sb.pcm86.state.step_bit = 2;
        // vir_buf > fifo_size so first branch of condition fails.
        sb.pcm86.state.vir_buf = 200;
        // adjusted = (200*4 + 800)/5 = 320 > 128, so second branch also fails.
        sb.pcm86.state.real_buf = 800;
        sb.pcm86.state.fifo = 0x80; // playing
        sb.pcm86.state.audio_frame_start_cycle = 1000;

        sb.pcm86_timer_expired(1000, 8_000_000);

        // The condition was not met, so needs_reschedule should be set
        // (drain_actions will call calculate_next_irq_cycle).
        assert!(
            sb.pcm86.needs_reschedule,
            "needs_reschedule should be set when condition fails"
        );
        // reqirq should still be true (not cleared).
        assert!(sb.pcm86.state.reqirq);
        // IRQ should NOT have been asserted.
        assert!(!sb.pcm86.state.irq_flag);
    }

    #[test]
    fn drain_actions_only_schedules_when_needs_reschedule() {
        let mut sb = Soundboard86::new(8_000_000, 48000, None, false, MachineModel::PC9801RA);
        sb.pcm86.state.reqirq = true;
        sb.pcm86.state.fifo = 0x80; // playback active
        sb.pcm86.needs_reschedule = false;

        // reqirq alone should NOT trigger scheduling.
        let actions = sb.drain_actions(false);
        let timer_action = actions.iter().any(|a| {
            matches!(
                a,
                Soundboard86Action::ScheduleTimer {
                    kind: EventKind::Pcm86Irq,
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
                    kind: EventKind::Pcm86Irq,
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
    fn irq_condition_met_with_realbuf_below_threshold() {
        let mut pcm = make_pcm86();
        pcm.state.fifo = 0xA0; // IRQ enabled (bit 5) + playing (bit 7)
        pcm.state.fifo_size = 128;
        pcm.state.step_mask = 3;
        pcm.state.data_write_irq_wait = 0;
        // vir_buf above threshold - primary condition false.
        pcm.state.vir_buf = 200;
        // real_buf below threshold but above step_mask - secondary condition true.
        pcm.state.real_buf = 100;
        pcm.state.irq_flag = false;

        assert!(
            pcm.irq_condition_met(1000, false),
            "should trigger via secondary realbuf condition"
        );
    }

    #[test]
    fn irq_condition_met_realbuf_at_stepmask_does_not_trigger() {
        let mut pcm = make_pcm86();
        pcm.state.fifo = 0xA0;
        pcm.state.fifo_size = 128;
        pcm.state.step_mask = 3;
        pcm.state.data_write_irq_wait = 0;
        pcm.state.vir_buf = 200;
        // real_buf <= step_mask - secondary condition should NOT trigger.
        pcm.state.real_buf = 3;
        pcm.state.irq_flag = false;

        assert!(
            !pcm.irq_condition_met(1000, false),
            "should not trigger when real_buf <= step_mask"
        );
    }

    #[test]
    fn wavestar_sequence_detection_sets_value() {
        let mut pcm = make_pcm86();
        assert_eq!(pcm.state.wavestar_seq_index, 0);
        assert_eq!(pcm.state.wavestar_value, 0xFF);

        // Feed the full magic sequence via port 0xA462.
        for &byte in &WAVESTAR_SEQUENCE {
            pcm.write_port(0xA462, byte, 0, 8_000_000, 48000);
        }
        assert_eq!(pcm.state.wavestar_seq_index, 5);
        assert_eq!(pcm.state.wavestar_value, 0x0B);
    }

    #[test]
    fn wavestar_partial_sequence_resets() {
        let mut pcm = make_pcm86();
        // Start the sequence.
        pcm.write_port(0xA462, 0xA6, 0, 8_000_000, 48000);
        pcm.write_port(0xA462, 0xD3, 0, 8_000_000, 48000);
        assert_eq!(pcm.state.wavestar_seq_index, 2);

        // Wrong byte resets.
        pcm.write_port(0xA462, 0x00, 0, 8_000_000, 48000);
        assert_eq!(pcm.state.wavestar_seq_index, 0);
    }

    #[test]
    fn wavestar_partial_sequence_restarts_on_first_byte() {
        let mut pcm = make_pcm86();
        pcm.write_port(0xA462, 0xA6, 0, 8_000_000, 48000);
        pcm.write_port(0xA462, 0xD3, 0, 8_000_000, 48000);
        // Writing the first byte again restarts from index 1.
        pcm.write_port(0xA462, 0xA6, 0, 8_000_000, 48000);
        assert_eq!(pcm.state.wavestar_seq_index, 1);
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
    fn pcm86_timer_expired_reschedules_when_reqirq_false() {
        let mut sb = Soundboard86::new(8_000_000, 48000, None, false, MachineModel::PC9801RA);
        sb.pcm86.state.reqirq = false;
        sb.pcm86.state.fifo = 0x80; // playing

        sb.pcm86_timer_expired(1000, 8_000_000);

        assert!(
            !sb.pcm86.state.reqirq,
            "reqirq must not be set by timer callback"
        );
        assert!(
            sb.pcm86.needs_reschedule,
            "needs_reschedule should be set for setnextintr"
        );
    }

    #[test]
    fn pcm86_timer_expired_does_not_reschedule_when_not_playing() {
        let mut sb = Soundboard86::new(8_000_000, 48000, None, false, MachineModel::PC9801RA);
        sb.pcm86.state.reqirq = false;
        sb.pcm86.state.fifo = 0x00; // NOT playing

        sb.pcm86_timer_expired(1000, 8_000_000);

        assert!(!sb.pcm86.state.reqirq, "reqirq should remain false");
        assert!(
            sb.pcm86.needs_reschedule,
            "needs_reschedule set even when not playing - drain_actions gates on playback"
        );
    }

    #[test]
    fn port_a468_write_updates_buffer_before_force_irq() {
        let mut pcm = make_pcm86();
        pcm.state.dactrl = 0x60; // 8-bit L only, step_bit=0
        pcm.state.step_bit = PCM86_BITS[(0x60 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.fifo_size = 128;
        pcm.state.fifo = 0x80; // playing
        pcm.state.audio_frame_start_cycle = 0;

        // Set vir_buf above threshold - stale without update.
        pcm.state.vir_buf = 200;
        pcm.state.real_buf = 200;

        // Advance enough time at 44100 Hz with 8 MHz clock for >72 samples consumed.
        // 200 - 72 = 128, which is <= fifo_size. Force-IRQ should trigger.
        let cycles_for_73_samples = (73u64 * 8_000_000) / 44100 + 1;

        // Write 0xA468 with bit 4=0 to clear IRQ, triggering the force-IRQ check.
        pcm.write_port(0xA468, 0x80, cycles_for_73_samples, 8_000_000, 48000);

        assert!(
            pcm.state.irq_flag,
            "force-IRQ should trigger after buffer state update reduces vir_buf"
        );
    }

    #[test]
    fn irq_condition_met_latches_irq_flag() {
        let mut pcm = make_pcm86();
        pcm.state.fifo = 0xA0; // play + IRQ enable
        pcm.state.fifo_size = 128;
        pcm.state.data_write_irq_wait = 0;
        pcm.state.vir_buf = 0;
        pcm.state.irq_flag = false;

        let result = pcm.irq_condition_met(100, false);
        assert!(result, "condition should be met");
        assert!(
            pcm.state.irq_flag,
            "irq_flag should be latched after condition met"
        );
    }

    #[test]
    fn irq_condition_met_does_not_relatch_when_already_set() {
        let mut pcm = make_pcm86();
        pcm.state.fifo = 0xA0;
        pcm.state.fifo_size = 128;
        pcm.state.data_write_irq_wait = 0;
        pcm.state.vir_buf = 500; // above threshold
        pcm.state.real_buf = 500;
        pcm.state.irq_flag = true; // already latched

        let result = pcm.irq_condition_met(100, false);
        assert!(result, "should return true via irq_flag path");
    }

    #[test]
    fn port_a46a_write_updates_buffer_before_dactrl_change() {
        let mut pcm = make_pcm86();
        pcm.state.dactrl = 0x60; // 8-bit L only
        pcm.state.step_bit = PCM86_BITS[(0x60 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.fifo = 0x80; // playing
        pcm.state.audio_frame_start_cycle = 0;

        write_bytes(&mut pcm, &[0x10; 100]);
        let vir_buf_before = pcm.state.vir_buf;

        // Advance time so update_buffer_state would decrement vir_buf.
        let cycles = (10u64 * 8_000_000) / 44100 + 1;
        pcm.write_port(0xA46A, 0x30, cycles, 8_000_000, 48000); // change to 16-bit stereo

        assert!(
            pcm.state.vir_buf < vir_buf_before,
            "vir_buf should have been decremented before dactrl change: was {vir_buf_before}, now {}",
            pcm.state.vir_buf
        );
        assert_eq!(pcm.state.dactrl, 0x30, "dactrl should be updated");
    }

    #[test]
    fn port_a466_write_updates_buffer_for_line5() {
        let mut pcm = make_pcm86();
        pcm.state.dactrl = 0x60; // 8-bit L only
        pcm.state.step_bit = PCM86_BITS[(0x60 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.fifo = 0x80; // playing
        pcm.state.audio_frame_start_cycle = 0;

        write_bytes(&mut pcm, &[0x10; 100]);
        let vir_buf_before = pcm.state.vir_buf;

        let cycles = (10u64 * 8_000_000) / 44100 + 1;
        // Write line 5 volume (bits 7:5 = 0b101 = 5, value 0xA5 -> line 5, vol 5).
        pcm.write_port(0xA466, 0xA5, cycles, 8_000_000, 48000);

        assert!(
            pcm.state.vir_buf < vir_buf_before,
            "vir_buf should have been decremented for line 5 write: was {vir_buf_before}, now {}",
            pcm.state.vir_buf
        );
        assert_eq!(pcm.state.vol[5], 5, "volume should be stored");
    }

    #[test]
    fn port_a466_write_does_not_update_buffer_for_other_lines() {
        let mut pcm = make_pcm86();
        pcm.state.dactrl = 0x60;
        pcm.state.step_bit = PCM86_BITS[(0x60 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.fifo = 0x80;
        pcm.state.audio_frame_start_cycle = 0;

        write_bytes(&mut pcm, &[0x10; 100]);
        let vir_buf_before = pcm.state.vir_buf;

        let cycles = (10u64 * 8_000_000) / 44100 + 1;
        // Write line 0 volume (bits 7:5 = 0b000 = 0).
        pcm.write_port(0xA466, 0x03, cycles, 8_000_000, 48000);

        assert_eq!(
            pcm.state.vir_buf, vir_buf_before,
            "vir_buf should NOT change for non-line-5 writes"
        );
    }

    #[test]
    fn drain_suppresses_output_in_recording_mode() {
        let mut pcm = make_pcm86();
        pcm.state.dactrl = 0x60; // 8-bit L only
        pcm.state.step_bit = PCM86_BITS[(0x60 >> 4) & 7];
        pcm.state.step_mask = (1u32 << pcm.state.step_bit) - 1;
        pcm.state.vol[5] = 0; // max volume
        pcm.state.fifo = 0xC0; // bit 7 (play) + bit 6 (recording direction)
        write_bytes(&mut pcm, &[0x7F; 16]);

        let mut out = [0.0f32; 8];
        pcm.drain_samples(&mut out);
        // NP21W does not check bit 6 (recording direction) for audio output.
        // PCM output plays regardless of the FIFO direction flag.
        assert!(
            out.iter().any(|&s| s != 0.0),
            "bit 6 should not suppress DAC output (matches NP21W)"
        );
    }

    #[test]
    fn fm_timer_piggyback_asserts_pcm86_irq() {
        let mut sb = Soundboard86::new(8_000_000, 48000, None, false, MachineModel::PC9801RA);
        sb.pcm86.state.fifo = 0xA0; // playing + IRQ enabled
        sb.pcm86.state.fifo_size = 128;
        sb.pcm86.state.data_write_irq_wait = 0;
        sb.pcm86.state.vir_buf = 0; // below threshold
        sb.pcm86.state.irq_flag = false;
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
        assert!(sb.pcm86.state.irq_flag, "irq_flag should be set");
    }

    #[test]
    fn fm_timer_piggyback_skipped_when_pcm86_timer_pending() {
        let mut sb = Soundboard86::new(8_000_000, 48000, None, false, MachineModel::PC9801RA);
        sb.pcm86.state.fifo = 0xA0;
        sb.pcm86.state.fifo_size = 128;
        sb.pcm86.state.data_write_irq_wait = 0;
        sb.pcm86.state.vir_buf = 0;
        sb.pcm86.state.irq_flag = false;
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
            !sb.pcm86.state.irq_flag,
            "irq_flag should not be set when piggyback skipped"
        );
    }

    #[test]
    fn irq_condition_met_guard_skips_buffer_check_when_timer_pending() {
        let mut pcm = make_pcm86();
        pcm.state.fifo = 0xA0; // IRQ enabled + playing
        pcm.state.fifo_size = 128;
        pcm.state.data_write_irq_wait = 0;
        pcm.state.vir_buf = 0; // below threshold
        pcm.state.irq_flag = false;

        // With timer pending, buffer condition should not set irq_flag.
        let result = pcm.irq_condition_met(100, true);
        assert!(!result, "should not trigger when timer is pending");
        assert!(!pcm.state.irq_flag, "irq_flag should not be set");

        // Without timer pending, same condition should trigger.
        let result = pcm.irq_condition_met(100, false);
        assert!(result, "should trigger when timer is not pending");
        assert!(pcm.state.irq_flag, "irq_flag should be set");
    }

    #[test]
    fn irq_condition_met_returns_true_via_flag_even_when_timer_pending() {
        let mut pcm = make_pcm86();
        pcm.state.fifo = 0xA0;
        pcm.state.fifo_size = 128;
        pcm.state.data_write_irq_wait = 0;
        pcm.state.vir_buf = 500; // above threshold
        pcm.state.irq_flag = true; // already latched

        // Even with timer pending, the already-set irq_flag is returned.
        let result = pcm.irq_condition_met(100, true);
        assert!(
            result,
            "should return true via irq_flag path regardless of pending"
        );
    }

    #[test]
    fn drift_weighting_pulls_timer_towards_realbuf() {
        let mut pcm = make_pcm86();
        pcm.state.fifo = 0x80; // playing
        pcm.state.fifo_size = 128;
        pcm.state.step_bit = 0;
        pcm.state.step_mask = 0;
        pcm.state.vir_buf = 1128; // 1000 above threshold
        pcm.state.real_buf = 628; // 500 above threshold (cntr < cntv)

        // No drift - should use pure vir_buf.
        pcm.buf_under_flag = 0;
        let no_drift = pcm.calculate_next_irq_cycle(0, 8_000_000);

        // Moderate drift (>4s) - should blend slightly towards real_buf.
        pcm.buf_under_flag = 5000;
        let moderate_drift = pcm.calculate_next_irq_cycle(0, 8_000_000);

        // Severe drift (>32s) - should blend more towards real_buf.
        pcm.buf_under_flag = 33000;
        let severe_drift = pcm.calculate_next_irq_cycle(0, 8_000_000);

        // All should return Some since buffer is above threshold.
        assert!(no_drift.is_some());
        assert!(moderate_drift.is_some());
        assert!(severe_drift.is_some());

        // Drift should pull the timer earlier (lower fire_cycle) since
        // real_buf is closer to threshold than vir_buf.
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
        pcm.state.fifo_size = 128;
        pcm.state.vir_buf = 500;
        pcm.state.real_buf = 400; // deficit < 0 -> underrun

        // First call establishes baseline (last_checkbuf_cycle starts at 0).
        pcm.reconcile_buffers(1000, 8_000_000);
        assert_eq!(pcm.buf_under_flag, 0, "first call should not accumulate");

        // Second call 1 second later.
        pcm.reconcile_buffers(8_001_000, 8_000_000);
        assert!(
            pcm.buf_under_flag > 0,
            "should accumulate drift: {}",
            pcm.buf_under_flag
        );

        // Recovery: real_buf >= vir_buf resets the flag.
        pcm.state.real_buf = 600;
        pcm.reconcile_buffers(16_001_000, 8_000_000);
        assert_eq!(pcm.buf_under_flag, 0, "recovery should reset drift");
    }
}
