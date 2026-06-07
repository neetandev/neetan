//! Creative Sound Blaster 16 for PC-98 (CT2720): YMF262 (OPL3) + CT1741 DSP + CT1745 mixer.

use std::collections::VecDeque;

use common::EventKind;
use resampler::{Attenuation, Latency, ResamplerFir};
use ymfm_oxide::{Ymf262, YmfmOutput4, YmfmTimerUpdate};

use crate::soundboard_26k::FmSampleRemainder;

const YMF262_CLOCK: u32 = 14_318_181;

const DSP_VERSION_MAJOR: u8 = 4;
const DSP_VERSION_MINOR: u8 = 12;

const DMA_RING_BUF_SIZE: usize = 16384;
const DMA_RING_BUF_MASK: usize = DMA_RING_BUF_SIZE - 1;

/// Bytes transferred per DMA batch event.
pub const DMA_BATCH_SIZE: usize = 128;

const RESAMPLER_ATTENUATION: Attenuation = Attenuation::Db60;
const RESAMPLER_LATENCY: Latency = Latency::Sample64;

const DEFAULT_SAMPLE_RATE: u32 = 22050;

const COPYRIGHT_STRING: &[u8] = b"COPYRIGHT (C) CREATIVE TECHNOLOGY LTD, 1992.\0";

const DMA_FORMAT_UNSIGNED_8_MONO: u8 = 0;
const DMA_FORMAT_UNSIGNED_8_STEREO: u8 = 1;
const DMA_FORMAT_SIGNED_16_MONO: u8 = 2;
const DMA_FORMAT_SIGNED_16_STEREO: u8 = 3;

/// Returns the number of bytes per sample frame for the given DMA format.
pub fn dma_format_bytes_per_sample(format: u8) -> u32 {
    match format {
        DMA_FORMAT_UNSIGNED_8_MONO => 1,
        DMA_FORMAT_UNSIGNED_8_STEREO => 2,
        DMA_FORMAT_SIGNED_16_MONO => 2,
        DMA_FORMAT_SIGNED_16_STEREO => 4,
        _ => 1,
    }
}

/// Returns whether the given DMA format uses 16-bit samples.
pub fn dma_format_is_16bit(format: u8) -> bool {
    format == DMA_FORMAT_SIGNED_16_MONO || format == DMA_FORMAT_SIGNED_16_STEREO
}

fn mixer_default_registers() -> [u8; 256] {
    let mut regs = [0u8; 256];
    // Master L/R
    regs[0x30] = 0xC0;
    regs[0x31] = 0xC0;
    // Voice/DAC L/R
    regs[0x32] = 0xC0;
    regs[0x33] = 0xC0;
    // FM/MIDI L/R
    regs[0x34] = 0xC0;
    regs[0x35] = 0xC0;
    // DMA channel (DMA 3 = bit 3)
    regs[0x81] = 0x08;
    // Output switch
    regs[0x3C] = 0x1F;
    // Input switch L/R
    regs[0x3D] = 0x15;
    regs[0x3E] = 0x0B;
    // Treble/Bass
    regs[0x44] = 0x80;
    regs[0x45] = 0x80;
    regs[0x46] = 0x80;
    regs[0x47] = 0x80;
    regs
}

fn mixer_volume_scale(register_value: u8) -> f32 {
    let level = (register_value >> 3) & 0x1F;
    if level == 0 {
        return 0.0;
    }
    let normalized = level as f32 / 31.0;
    normalized * normalized
}

/// DSP state snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sb16DspState {
    /// Whether the DSP is in reset.
    pub reset_active: bool,
    /// Current command byte being assembled.
    pub command: u8,
    /// Remaining parameter bytes for the current command.
    pub params_needed: u8,
    /// Accumulated command parameter bytes.
    pub input_buffer: Vec<u8>,
    /// DSP->CPU output FIFO.
    pub output_buffer: VecDeque<u8>,
    /// Test register (written via 0xE4, read via 0xE8).
    pub test_register: u8,
    /// Speaker on/off state.
    pub speaker_enabled: bool,
    /// Current PCM sample rate.
    pub sample_rate: u32,
    /// Time constant (legacy rate setting).
    pub time_constant: u8,
    /// Whether DMA transfer is active.
    pub dma_active: bool,
    /// Whether auto-init mode is enabled.
    pub dma_auto_init: bool,
    /// DMA PCM format (DMA_FORMAT_* constants).
    pub dma_format: u8,
    /// Block transfer size in bytes (set by command 0x48 or DMA start commands).
    pub dma_block_size: u32,
    /// Bytes remaining in current block.
    pub dma_bytes_remaining: u32,
    /// PC-98 DMA channel (0 or 3).
    pub dma_channel: u8,
    /// Raw mixer register 0x81 value (low + high DMA channel bits).
    pub dma_channel_register: u8,
    /// Whether the current DMA transfer is recording (device->memory) rather than playback.
    pub dma_is_recording: bool,
    /// 8-bit IRQ pending flag.
    pub irq_pending_8bit: bool,
    /// 16-bit IRQ pending flag.
    pub irq_pending_16bit: bool,
    /// PCM data ring buffer.
    pub pcm_buffer: Box<[u8; DMA_RING_BUF_SIZE]>,
    /// Write position in ring buffer.
    pub pcm_write_pos: usize,
    /// Read position in ring buffer.
    pub pcm_read_pos: usize,
    /// Number of buffered bytes.
    pub pcm_buffered: usize,
}

impl Default for Sb16DspState {
    fn default() -> Self {
        Self {
            reset_active: false,
            command: 0,
            params_needed: 0,
            input_buffer: Vec::new(),
            output_buffer: VecDeque::new(),
            test_register: 0,
            speaker_enabled: false,
            sample_rate: DEFAULT_SAMPLE_RATE,
            time_constant: 0,
            dma_active: false,
            dma_auto_init: false,
            dma_format: DMA_FORMAT_UNSIGNED_8_MONO,
            dma_block_size: 0,
            dma_bytes_remaining: 0,
            dma_channel: 3,
            dma_channel_register: 0x08,
            dma_is_recording: false,
            irq_pending_8bit: false,
            irq_pending_16bit: false,
            pcm_buffer: Box::new([0u8; DMA_RING_BUF_SIZE]),
            pcm_write_pos: 0,
            pcm_read_pos: 0,
            pcm_buffered: 0,
        }
    }
}

/// CT1745 mixer state snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sb16MixerState {
    /// Latched mixer register address.
    pub address: u8,
    /// Raw register values.
    pub registers: Box<[u8; 256]>,
}

impl Default for Sb16MixerState {
    fn default() -> Self {
        Self {
            address: 0,
            registers: Box::new(mixer_default_registers()),
        }
    }
}

/// Top-level SB16 sound board state snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundBlaster16State {
    /// I/O base address (default 0xD2).
    pub base_port: u16,
    /// PC-98 IRQ line (default 5).
    pub irq_line: u8,
    /// Whether the merged IRQ output is currently asserted.
    pub irq_asserted: bool,
    /// Whether the OPL3 chip IRQ output is asserted.
    pub opl3_irq_asserted: bool,
    /// Latched OPL3 register address (0x000–0x1FF).
    pub opl3_address: u16,
    /// CPU cycle at which the busy flag clears.
    pub busy_end_cycle: u64,
    /// CPU cycle at which the current audio output frame started.
    /// Only advanced by `generate_samples()`.
    pub audio_frame_start_cycle: u64,
    /// CPU cycle up to which the OPL3 chip has been clocked.
    /// Advanced by `sync_to_cycle()` on every port access.
    pub fm_sync_cursor: u64,
    /// Fractional sample remainder carried across frames.
    pub sample_remainder: FmSampleRemainder,
    /// DSP state.
    pub dsp: Sb16DspState,
    /// Mixer state.
    pub mixer: Sb16MixerState,
}

impl Default for SoundBlaster16State {
    fn default() -> Self {
        Self {
            base_port: 0xD2,
            irq_line: 5,
            irq_asserted: false,
            opl3_irq_asserted: false,
            opl3_address: 0,
            busy_end_cycle: 0,
            audio_frame_start_cycle: 0,
            fm_sync_cursor: 0,
            sample_remainder: FmSampleRemainder::default(),
            dsp: Sb16DspState::default(),
            mixer: Sb16MixerState::default(),
        }
    }
}

/// Actions emitted by the SB16 device for the bus to apply.
#[derive(Clone, Copy)]
pub enum SoundboardSb16Action {
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
    /// Request a DMA transfer from system memory.
    StartDma {
        /// DMA channel number.
        channel: u8,
    },
    /// Cancel DMA transfer scheduling.
    StopDma,
}

fn dsp_params_needed(cmd: u8) -> u8 {
    match cmd {
        0x02 => 1,        // Get ASP version
        0x04 => 1,        // Set ASP mode register
        0x05 => 2,        // Set ASP codec param
        0x0E => 2,        // Write ASP register
        0x0F => 1,        // Read ASP register
        0x10 => 1,        // 8-bit direct DAC
        0x14 => 2,        // 8-bit DMA single (length_lo, length_hi)
        0x16 | 0x17 => 2, // 2-bit ADPCM
        0x1C => 0,        // 8-bit auto-init
        0x40 => 1,        // Set time constant
        0x41 | 0x42 => 2, // Set sample rate (rate_hi, rate_lo)
        0x48 => 2,        // Set block size (size_lo, size_hi)
        0x74..=0x77 => 2, // 4-bit ADPCM
        0x7D..=0x7F => 0, // ADPCM auto-init
        0x80 => 2,        // Pause DAC duration
        0xB0..=0xBF => 3, // 16-bit DMA (mode, length_lo, length_hi)
        0xC0..=0xCF => 3, // 8-bit DMA (mode, length_lo, length_hi)
        0xD0 => 0,        // Pause 8-bit DMA
        0xD1 => 0,        // Speaker on
        0xD3 => 0,        // Speaker off
        0xD4 => 0,        // Continue 8-bit DMA
        0xD5 => 0,        // Pause 16-bit DMA
        0xD6 => 0,        // Continue 16-bit DMA
        0xD8 => 0,        // Get speaker status
        0xD9 => 0,        // Exit 16-bit auto-init
        0xDA => 0,        // Exit 8-bit auto-init
        0xE0 => 1,        // DSP identification
        0xE1 => 0,        // Get version
        0xE3 => 0,        // Get copyright
        0xE4 => 1,        // Write test register
        0xE8 => 0,        // Read test register
        0xF2 => 0,        // Force 8-bit IRQ
        0xF3 => 0,        // Force 16-bit IRQ
        _ => 0,           // Unknown commands take no params
    }
}

/// Creative Sound Blaster 16 for PC-98.
pub struct SoundBlaster16 {
    /// Current device state (saveable).
    pub state: SoundBlaster16State,
    opl3: Ymf262,
    opl3_action_cycle: u64,
    opl3_native_rate: u32,
    opl3_native_buffer: Vec<YmfmOutput4>,
    opl3_pending_native: Vec<YmfmOutput4>,
    opl3_resampler: ResamplerFir,
    opl3_resample_input: Vec<f32>,
    opl3_resample_output: Vec<f32>,
    pcm_resampler: ResamplerFir,
    pcm_resample_input: Vec<f32>,
    pcm_resample_output: Vec<f32>,
    sample_rate: u32,
    cpu_clock_hz: u32,
    pending_dsp_actions: Vec<SoundboardSb16Action>,
    /// Set when DMA parameters change and a new resampler is needed.
    pcm_rate_dirty: bool,
    action_buffer: Vec<SoundboardSb16Action>,
}

impl SoundBlaster16 {
    /// Creates a new SB16 sound board instance.
    pub fn new(cpu_clock_hz: u32, sample_rate: u32) -> Self {
        let mut opl3 = Ymf262::new();
        opl3.reset();

        let opl3_native_rate = opl3.sample_rate(YMF262_CLOCK);
        let opl3_resampler = ResamplerFir::new_from_hz(
            2,
            opl3_native_rate,
            sample_rate,
            RESAMPLER_LATENCY,
            RESAMPLER_ATTENUATION,
        );
        let opl3_resample_output_size = opl3_resampler.buffer_size_output();

        let pcm_resampler = ResamplerFir::new_from_hz(
            2,
            DEFAULT_SAMPLE_RATE,
            sample_rate,
            RESAMPLER_LATENCY,
            RESAMPLER_ATTENUATION,
        );
        let pcm_resample_output_size = pcm_resampler.buffer_size_output();

        Self {
            state: SoundBlaster16State::default(),
            opl3,
            opl3_action_cycle: 0,
            opl3_native_rate,
            opl3_native_buffer: vec![YmfmOutput4 { data: [0; 4] }; 4096],
            opl3_pending_native: Vec::new(),
            opl3_resampler,
            opl3_resample_input: vec![0.0; 4096 * 2],
            opl3_resample_output: vec![0.0; opl3_resample_output_size],
            pcm_resampler,
            pcm_resample_input: vec![0.0; 4096 * 2],
            pcm_resample_output: vec![0.0; pcm_resample_output_size],
            sample_rate,
            cpu_clock_hz,
            pending_dsp_actions: Vec::new(),
            pcm_rate_dirty: false,
            action_buffer: Vec::new(),
        }
    }

    /// Returns the I/O base port.
    pub fn base_port(&self) -> u16 {
        self.state.base_port
    }

    fn sync_to_cycle(&mut self, current_cycle: u64) {
        let sync_start = self.state.fm_sync_cursor;
        let elapsed_cycles = current_cycle.saturating_sub(sync_start);
        if elapsed_cycles == 0 {
            return;
        }

        let native_rate = u64::from(self.opl3_native_rate);
        let exact_native = (elapsed_cycles as f64 * native_rate as f64)
            / f64::from(self.cpu_clock_hz)
            + self.state.sample_remainder.0;
        let native_count = exact_native as usize;
        if native_count == 0 {
            return;
        }

        self.state.sample_remainder = FmSampleRemainder(exact_native - native_count as f64);
        self.state.fm_sync_cursor = current_cycle;

        if self.opl3_native_buffer.len() < native_count {
            self.opl3_native_buffer
                .resize(native_count, YmfmOutput4 { data: [0; 4] });
        }
        self.opl3
            .generate(&mut self.opl3_native_buffer[..native_count]);
        self.opl3_pending_native
            .extend_from_slice(&self.opl3_native_buffer[..native_count]);
    }

    fn apply_opl3_busy(&mut self, busy_clocks: u32, current_cycle: u64) {
        if busy_clocks != 0 {
            let cpu_clocks =
                u64::from(busy_clocks) * u64::from(self.cpu_clock_hz) / u64::from(YMF262_CLOCK);
            self.state.busy_end_cycle = current_cycle + cpu_clocks;
        }
    }

    /// Reads the OPL3 status register.
    pub fn read_opl3_status(&mut self, current_cycle: u64) -> u8 {
        self.sync_to_cycle(current_cycle);
        self.opl3_action_cycle = current_cycle;
        self.opl3.read_status()
    }

    /// Writes the OPL3 low-bank address latch.
    pub fn write_opl3_address_lo(&mut self, value: u8, current_cycle: u64) {
        self.state.opl3_address = value as u16;
        self.sync_to_cycle(current_cycle);
        self.opl3_action_cycle = current_cycle;
        let busy_clocks = self.opl3.write_address(value);
        self.apply_opl3_busy(busy_clocks, current_cycle);
    }

    /// Writes OPL3 data to the previously latched register.
    pub fn write_opl3_data(&mut self, value: u8, current_cycle: u64) {
        self.sync_to_cycle(current_cycle);
        self.opl3_action_cycle = current_cycle;
        let busy_clocks = self.opl3.write_data(value);
        self.apply_opl3_busy(busy_clocks, current_cycle);
    }

    /// Writes the OPL3 high-bank address latch.
    pub fn write_opl3_address_hi(&mut self, value: u8, current_cycle: u64) {
        self.sync_to_cycle(current_cycle);
        self.opl3_action_cycle = current_cycle;
        let busy_clocks = self.opl3.write_address_hi(value);
        self.apply_opl3_busy(busy_clocks, current_cycle);
        self.state.opl3_address = 0x100 | u16::from(value);
        // The Ymf262 internally manages the full address; we just track the raw write.
    }

    /// Writes to the DSP reset port.
    pub fn write_dsp_reset(&mut self, value: u8) {
        if value == 0xC6 {
            // Special reset used by Creative's PC-98 driver for ASP detection.
            // Clears output buffer and resets command state, but does NOT
            // push 0xAA and does NOT block subsequent commands.
            self.state.dsp.output_buffer.clear();
            self.state.dsp.command = 0;
            self.state.dsp.params_needed = 0;
            self.state.dsp.input_buffer.clear();
            if self.state.dsp.dma_active {
                self.state.dsp.dma_active = false;
                self.pending_dsp_actions.push(SoundboardSb16Action::StopDma);
            }
        } else if value & 1 != 0 {
            // Enter reset
            self.state.dsp.reset_active = true;
            self.state.dsp.command = 0;
            self.state.dsp.params_needed = 0;
            self.state.dsp.input_buffer.clear();
            self.state.dsp.output_buffer.clear();
            if self.state.dsp.dma_active {
                self.state.dsp.dma_active = false;
                self.pending_dsp_actions.push(SoundboardSb16Action::StopDma);
            }
        } else if self.state.dsp.reset_active {
            // Exit reset: push ready byte
            self.state.dsp.reset_active = false;
            self.state.dsp.output_buffer.push_back(0xAA);
        }
    }

    /// Reads the DSP reset port.
    pub fn read_dsp_reset(&self) -> u8 {
        0xFF
    }

    /// Writes a command or parameter byte to the DSP.
    pub fn write_dsp_command(&mut self, value: u8) {
        if self.state.dsp.reset_active {
            return;
        }

        if self.state.dsp.params_needed > 0 {
            self.state.dsp.input_buffer.push(value);
            self.state.dsp.params_needed -= 1;
            if self.state.dsp.params_needed == 0 {
                self.execute_dsp_command();
            }
        } else {
            self.state.dsp.command = value;
            self.state.dsp.input_buffer.clear();
            let needed = dsp_params_needed(value);
            if needed > 0 {
                self.state.dsp.params_needed = needed;
            } else {
                self.execute_dsp_command();
            }
        }
    }

    /// Reads data from the DSP output buffer.
    pub fn read_dsp_data(&mut self) -> u8 {
        self.state.dsp.output_buffer.pop_front().unwrap_or(0xFF)
    }

    /// Reads the DSP write status (bit 7 = not ready to accept data).
    pub fn read_dsp_write_status(&self) -> u8 {
        // Always ready
        0x00
    }

    /// Reads the DSP read-buffer status for 8-bit mode, clearing 8-bit IRQ.
    pub fn read_dsp_status_8bit(&mut self) -> u8 {
        self.state.dsp.irq_pending_8bit = false;
        self.update_merged_irq();
        if self.state.dsp.output_buffer.is_empty() {
            0x00
        } else {
            0x80
        }
    }

    /// Reads the DSP read-buffer status for 16-bit mode, clearing 16-bit IRQ.
    pub fn read_dsp_status_16bit(&mut self) -> u8 {
        self.state.dsp.irq_pending_16bit = false;
        self.update_merged_irq();
        if self.state.dsp.output_buffer.is_empty() {
            0x00
        } else {
            0x80
        }
    }

    fn execute_dsp_command(&mut self) {
        let cmd = self.state.dsp.command;
        let params = &self.state.dsp.input_buffer;

        match cmd {
            // Get ASP version (stub)
            0x02 => {}

            // Set ASP mode register (stub)
            0x04 => {
                self.state.dsp.output_buffer.clear();
                self.state.dsp.output_buffer.push_back(0xFF);
            }

            // Set ASP codec param (stub)
            0x05 => {}

            // Write ASP register (stub)
            0x0E => {}

            // Read ASP register (stub)
            0x0F => {
                self.state.dsp.output_buffer.clear();
                self.state.dsp.output_buffer.push_back(0x00);
            }

            // 8-bit direct DAC output (PIO)
            0x10 if !params.is_empty() => {
                let sample = params[0];
                self.write_pcm_byte(sample);
            }

            // 8-bit DMA single transfer
            0x14 if params.len() >= 2 => {
                let length = params[0] as u32 | ((params[1] as u32) << 8);
                self.start_dma(DMA_FORMAT_UNSIGNED_8_MONO, length + 1, false, false);
            }

            // 8-bit auto-init DMA
            0x1C => {
                self.start_dma(
                    DMA_FORMAT_UNSIGNED_8_MONO,
                    self.state.dsp.dma_block_size,
                    true,
                    false,
                );
            }

            // Set time constant
            0x40 if !params.is_empty() => {
                self.state.dsp.time_constant = params[0];
                let tc = params[0] as u32;
                if tc < 256 {
                    self.state.dsp.sample_rate = 1_000_000 / (256 - tc);
                    self.pcm_rate_dirty = true;
                }
            }

            // Set output/input sampling rate
            0x41 | 0x42 if params.len() >= 2 => {
                let rate = ((params[0] as u32) << 8) | params[1] as u32;
                if rate > 0 {
                    self.state.dsp.sample_rate = rate;
                    self.pcm_rate_dirty = true;
                }
            }

            // Set block size
            0x48 if params.len() >= 2 => {
                self.state.dsp.dma_block_size = (params[0] as u32 | ((params[1] as u32) << 8)) + 1;
            }

            // Pause DAC duration (stub)
            0x80 => {}

            // SB16-style 16-bit DMA (0xB0-0xBF)
            0xB0..=0xBF if params.len() >= 3 => {
                let auto_init = cmd & 0x04 != 0;
                let is_input = cmd & 0x08 != 0;
                let mode = params[0];
                let length = (params[1] as u32 | ((params[2] as u32) << 8)) + 1;
                let stereo = mode & 0x20 != 0;
                let format = if stereo {
                    DMA_FORMAT_SIGNED_16_STEREO
                } else {
                    DMA_FORMAT_SIGNED_16_MONO
                };
                // Length counts individual 16-bit samples (both channels
                // interleaved for stereo). Each sample is 2 bytes.
                let byte_length = length * 2;
                self.start_dma(format, byte_length, auto_init, is_input);
            }

            // SB16-style 8-bit DMA (0xC0-0xCF)
            0xC0..=0xCF if params.len() >= 3 => {
                let auto_init = cmd & 0x04 != 0;
                let is_input = cmd & 0x08 != 0;
                let mode = params[0];
                let length = (params[1] as u32 | ((params[2] as u32) << 8)) + 1;
                let stereo = mode & 0x20 != 0;
                let format = if stereo {
                    DMA_FORMAT_UNSIGNED_8_STEREO
                } else {
                    DMA_FORMAT_UNSIGNED_8_MONO
                };
                self.start_dma(format, length, auto_init, is_input);
            }

            // Pause 8-bit DMA
            0xD0 if self.state.dsp.dma_active => {
                self.state.dsp.dma_active = false;
                self.pending_dsp_actions.push(SoundboardSb16Action::StopDma);
            }

            // Speaker on
            0xD1 => {
                self.state.dsp.speaker_enabled = true;
            }

            // Speaker off
            0xD3 => {
                self.state.dsp.speaker_enabled = false;
            }

            // Continue 8-bit DMA
            0xD4 if !self.state.dsp.dma_active && self.state.dsp.dma_bytes_remaining > 0 => {
                self.state.dsp.dma_active = true;
                self.pending_dsp_actions
                    .push(SoundboardSb16Action::StartDma {
                        channel: self.state.dsp.dma_channel,
                    });
            }

            // Pause 16-bit DMA
            0xD5 if self.state.dsp.dma_active => {
                self.state.dsp.dma_active = false;
                self.pending_dsp_actions.push(SoundboardSb16Action::StopDma);
            }

            // Continue 16-bit DMA
            0xD6 if !self.state.dsp.dma_active && self.state.dsp.dma_bytes_remaining > 0 => {
                self.state.dsp.dma_active = true;
                self.pending_dsp_actions
                    .push(SoundboardSb16Action::StartDma {
                        channel: self.state.dsp.dma_channel,
                    });
            }

            // Get speaker status
            0xD8 => {
                self.state
                    .dsp
                    .output_buffer
                    .push_back(if self.state.dsp.speaker_enabled {
                        0xFF
                    } else {
                        0x00
                    });
            }

            // Exit 16-bit auto-init
            0xD9 => {
                self.state.dsp.dma_auto_init = false;
            }

            // Exit 8-bit auto-init
            0xDA => {
                self.state.dsp.dma_auto_init = false;
            }

            // DSP identification
            0xE0 if !params.is_empty() => {
                self.state.dsp.output_buffer.push_back(!params[0]);
            }

            // Get version
            0xE1 => {
                self.state.dsp.output_buffer.push_back(DSP_VERSION_MAJOR);
                self.state.dsp.output_buffer.push_back(DSP_VERSION_MINOR);
            }

            // Get copyright string
            0xE3 => {
                for &byte in COPYRIGHT_STRING {
                    self.state.dsp.output_buffer.push_back(byte);
                }
            }

            // Write test register
            0xE4 if !params.is_empty() => {
                self.state.dsp.test_register = params[0];
            }

            // Read test register
            0xE8 => {
                self.state
                    .dsp
                    .output_buffer
                    .push_back(self.state.dsp.test_register);
            }

            // Force 8-bit IRQ
            0xF2 => {
                self.state.dsp.irq_pending_8bit = true;
                self.update_merged_irq();
            }

            // Force 16-bit IRQ
            0xF3 => {
                self.state.dsp.irq_pending_16bit = true;
                self.update_merged_irq();
            }

            _ => {}
        }

        self.state.dsp.command = 0;
        self.state.dsp.input_buffer.clear();
    }

    fn start_dma(&mut self, format: u8, byte_length: u32, auto_init: bool, recording: bool) {
        self.state.dsp.dma_active = true;
        self.state.dsp.dma_format = format;
        self.state.dsp.dma_auto_init = auto_init;
        self.state.dsp.dma_is_recording = recording;
        if byte_length > 0 {
            self.state.dsp.dma_block_size = byte_length;
        }
        self.state.dsp.dma_bytes_remaining = self.state.dsp.dma_block_size;
        self.pending_dsp_actions
            .push(SoundboardSb16Action::StartDma {
                channel: self.state.dsp.dma_channel,
            });
    }

    fn write_pcm_byte(&mut self, byte: u8) {
        let wp = self.state.dsp.pcm_write_pos;
        self.state.dsp.pcm_buffer[wp] = byte;
        self.state.dsp.pcm_write_pos = (wp + 1) & DMA_RING_BUF_MASK;
        if self.state.dsp.pcm_buffered < DMA_RING_BUF_SIZE {
            self.state.dsp.pcm_buffered += 1;
        }
    }

    /// Writes to the mixer address register.
    pub fn write_mixer_address(&mut self, value: u8) {
        self.state.mixer.address = value;
    }

    /// Reads the mixer address register.
    pub fn read_mixer_address(&self) -> u8 {
        self.state.mixer.address
    }

    /// Writes to the mixer data register.
    pub fn write_mixer_data(&mut self, value: u8) {
        let addr = self.state.mixer.address;
        match addr {
            0x00 => {
                // Reset mixer
                *self.state.mixer.registers = mixer_default_registers();
            }
            0x80 => {
                // IRQ configuration - PC-98 CT2720 maps ISA IRQ numbers
                // to PC-98 IRQ lines differently from standard ISA.
                self.state.mixer.registers[addr as usize] = value;
                self.state.irq_line = match value & 0x0F {
                    0x01 => 3,  // ISA IRQ 2 -> PC-98 IRQ 3
                    0x02 => 10, // ISA IRQ 5 -> PC-98 IRQ 10
                    0x04 => 12, // ISA IRQ 7 -> PC-98 IRQ 12
                    0x08 => 5,  // ISA IRQ 10 -> PC-98 IRQ 5
                    _ => self.state.irq_line,
                };
            }
            0x81 => {
                // DMA channel configuration.
                // Bits 0,1,3: low DMA channel (0/1/3).
                // Bits 5,6,7: high DMA channel (5/6/7).
                // On PC-98 CT2720, both low and high channels map to
                // PC-98 DMA channel 0 or 3. Reserved bits 2 and 4 are
                // masked off. The raw value (with reserved bits cleared)
                // is stored for high-DMA detection during 16-bit transfers.
                self.state.mixer.registers[addr as usize] = value;
                let masked = value & !0x14;
                self.state.dsp.dma_channel_register = masked;
                // DMA 0 (bit 0) or DMA 5 (bit 5) -> PC-98 DMA ch 0
                if masked & 0x21 != 0 {
                    self.state.dsp.dma_channel = 0;
                }
                // DMA 1 (bit 1), DMA 3 (bit 3), DMA 6 (bit 6), DMA 7 (bit 7) -> PC-98 DMA ch 3
                if masked & 0xCA != 0 {
                    self.state.dsp.dma_channel = 3;
                }
            }
            // Legacy SB Pro volume mapping
            0x04 => {
                // Voice volume (old stereo packed format)
                self.state.mixer.registers[0x32] = (value & 0xF0) | 0x08;
                self.state.mixer.registers[0x33] = ((value & 0x0F) << 4) | 0x08;
                self.state.mixer.registers[addr as usize] = value;
            }
            0x0A => {
                // Mic volume (old format)
                self.state.mixer.registers[0x3A] = (value & 0x07) << 5;
                self.state.mixer.registers[addr as usize] = value;
            }
            0x22 => {
                // Master volume (old stereo packed format)
                self.state.mixer.registers[0x30] = (value & 0xF0) | 0x08;
                self.state.mixer.registers[0x31] = ((value & 0x0F) << 4) | 0x08;
                self.state.mixer.registers[addr as usize] = value;
            }
            0x26 => {
                // FM/MIDI volume (old stereo packed format)
                self.state.mixer.registers[0x34] = (value & 0xF0) | 0x08;
                self.state.mixer.registers[0x35] = ((value & 0x0F) << 4) | 0x08;
                self.state.mixer.registers[addr as usize] = value;
            }
            0x28 => {
                // CD volume (old stereo packed format)
                self.state.mixer.registers[0x36] = (value & 0xF0) | 0x08;
                self.state.mixer.registers[0x37] = ((value & 0x0F) << 4) | 0x08;
                self.state.mixer.registers[addr as usize] = value;
            }
            0x2E => {
                // Line volume (old stereo packed format)
                self.state.mixer.registers[0x38] = (value & 0xF0) | 0x08;
                self.state.mixer.registers[0x39] = ((value & 0x0F) << 4) | 0x08;
                self.state.mixer.registers[addr as usize] = value;
            }
            _ => {
                self.state.mixer.registers[addr as usize] = value;
            }
        }
    }

    /// Reads from the mixer data register.
    pub fn read_mixer_data(&self) -> u8 {
        let addr = self.state.mixer.address;
        match addr {
            0x80 => {
                // IRQ config readback (PC-98 IRQ -> mixer register value)
                match self.state.irq_line {
                    3 => 0x01,
                    10 => 0x02,
                    12 => 0x04,
                    5 => 0x08,
                    _ => 0x00,
                }
            }
            0x81 => {
                // DMA config readback.
                self.state.mixer.registers[addr as usize]
            }
            0x82 => {
                // IRQ pending status
                let mut status = 0u8;
                if self.state.dsp.irq_pending_8bit {
                    status |= 0x01;
                }
                if self.state.dsp.irq_pending_16bit {
                    status |= 0x02;
                }
                status
            }
            _ => self.state.mixer.registers[addr as usize],
        }
    }

    fn update_merged_irq(&mut self) {
        let dsp_irq = self.state.dsp.irq_pending_8bit || self.state.dsp.irq_pending_16bit;
        let merged = self.state.opl3_irq_asserted || dsp_irq;
        if merged != self.state.irq_asserted {
            self.state.irq_asserted = merged;
            if merged {
                self.pending_dsp_actions
                    .push(SoundboardSb16Action::AssertIrq {
                        irq: self.state.irq_line,
                    });
            } else {
                self.pending_dsp_actions
                    .push(SoundboardSb16Action::DeassertIrq {
                        irq: self.state.irq_line,
                    });
            }
        }
    }

    /// Notifies the OPL3 chip that a timer has expired.
    pub fn timer_expired(&mut self, timer_id: u32, current_cycle: u64) {
        self.sync_to_cycle(current_cycle);
        self.opl3_action_cycle = current_cycle;
        self.opl3.timer_expired(timer_id);
    }

    /// Called when the `Sb16DspDma` scheduler event fires.
    /// Returns `true` if DMA should be rescheduled.
    pub fn dma_transfer_pending(&self) -> bool {
        self.state.dsp.dma_active
    }

    /// Accepts DMA data read from system memory.
    pub fn accept_dma_data(&mut self, data: &[u8]) {
        for &byte in data {
            self.write_pcm_byte(byte);
        }

        let consumed = data.len() as u32;
        if consumed >= self.state.dsp.dma_bytes_remaining {
            self.state.dsp.dma_bytes_remaining = 0;
            self.dma_block_complete();
        } else {
            self.state.dsp.dma_bytes_remaining -= consumed;
        }
    }

    /// Advances recording DMA progress after bytes have been written to memory.
    pub fn advance_dma_recording(&mut self, bytes_written: u32) {
        if bytes_written >= self.state.dsp.dma_bytes_remaining {
            self.state.dsp.dma_bytes_remaining = 0;
            self.dma_block_complete();
        } else {
            self.state.dsp.dma_bytes_remaining -= bytes_written;
        }
    }

    /// Called when the DMA controller signals terminal count.
    pub fn dma_terminal_count(&mut self) {
        self.state.dsp.dma_bytes_remaining = 0;
        self.dma_block_complete();
    }

    fn dma_block_complete(&mut self) {
        // Fire IRQ
        if dma_format_is_16bit(self.state.dsp.dma_format) {
            self.state.dsp.irq_pending_16bit = true;
        } else {
            self.state.dsp.irq_pending_8bit = true;
        }
        self.update_merged_irq();

        if self.state.dsp.dma_auto_init {
            // Restart block
            self.state.dsp.dma_bytes_remaining = self.state.dsp.dma_block_size;
        } else {
            // Stop DMA
            self.state.dsp.dma_active = false;
            self.pending_dsp_actions.push(SoundboardSb16Action::StopDma);
        }
    }

    /// Drains pending actions from OPL3 and DSP.
    pub fn drain_actions(&mut self) -> &[SoundboardSb16Action] {
        self.action_buffer.clear();

        let current_cycle = self.opl3_action_cycle;
        let cpu_clock_hz = self.cpu_clock_hz;

        let was_merged = self.state.irq_asserted;

        for (timer_id, kind) in [(0, EventKind::Sb16OplTimerA), (1, EventKind::Sb16OplTimerB)] {
            let Some(update) = self.opl3.take_timer_update(timer_id) else {
                continue;
            };
            match update {
                YmfmTimerUpdate::Cancel => self
                    .action_buffer
                    .push(SoundboardSb16Action::CancelTimer { kind }),
                YmfmTimerUpdate::Schedule(duration_in_clocks) => {
                    let cpu_cycles = u64::from(duration_in_clocks) * u64::from(cpu_clock_hz)
                        / u64::from(YMF262_CLOCK);
                    self.action_buffer
                        .push(SoundboardSb16Action::ScheduleTimer {
                            kind,
                            fire_cycle: current_cycle + cpu_cycles,
                        });
                }
            }
        }

        if let Some(asserted) = self.opl3.take_irq_update() {
            self.state.opl3_irq_asserted = asserted;
        }

        // Drain DSP actions
        self.action_buffer.append(&mut self.pending_dsp_actions);

        // Update merged IRQ
        let dsp_irq = self.state.dsp.irq_pending_8bit || self.state.dsp.irq_pending_16bit;
        let merged = self.state.opl3_irq_asserted || dsp_irq;
        if merged != was_merged {
            self.state.irq_asserted = merged;
            if merged {
                self.action_buffer.push(SoundboardSb16Action::AssertIrq {
                    irq: self.state.irq_line,
                });
            } else {
                self.action_buffer.push(SoundboardSb16Action::DeassertIrq {
                    irq: self.state.irq_line,
                });
            }
        }

        self.action_buffer.as_slice()
    }

    /// Generates resampled stereo audio and mixes it into `output`.
    pub fn generate_samples(
        &mut self,
        current_cycle: u64,
        cpu_clock_hz: u32,
        volume: f32,
        output: &mut [f32],
    ) {
        if output.is_empty() {
            self.sync_to_cycle(current_cycle);
            self.opl3_pending_native.clear();
            self.state.audio_frame_start_cycle = current_cycle;
            self.state.fm_sync_cursor = current_cycle;
            return;
        }

        // Rebuild PCM resampler if sample rate changed
        if self.pcm_rate_dirty {
            self.pcm_rate_dirty = false;
            let pcm_rate = self.state.dsp.sample_rate.max(1);
            self.pcm_resampler = ResamplerFir::new_from_hz(
                2,
                pcm_rate,
                self.sample_rate,
                RESAMPLER_LATENCY,
                RESAMPLER_ATTENUATION,
            );
            self.pcm_resample_output
                .resize(self.pcm_resampler.buffer_size_output(), 0.0);
        }

        // Generate remaining OPL3 native samples from fm_sync_cursor to current_cycle.
        let sync_cursor = self.state.fm_sync_cursor;
        let gap_cycles = current_cycle.saturating_sub(sync_cursor);
        let remaining_native = if gap_cycles > 0 {
            let native_rate = u64::from(self.opl3_native_rate);
            let exact_native = (gap_cycles as f64 * native_rate as f64) / f64::from(cpu_clock_hz)
                + self.state.sample_remainder.0;
            let count = exact_native as usize;
            self.state.sample_remainder = FmSampleRemainder(exact_native - count as f64);
            count
        } else {
            0
        };

        let pending_count = self.opl3_pending_native.len();
        let total_from_timing = pending_count + remaining_native;

        // Ensure the resampler receives enough input to fill the output.
        let output_frames = output.len() / 2;
        let min_native = (output_frames as u64 * u64::from(self.opl3_native_rate))
            .div_ceil(u64::from(self.sample_rate))
            + 1;
        let total_native = total_from_timing.max(min_native as usize);
        let remaining_native = total_native - pending_count;

        if remaining_native > 0 {
            if self.opl3_native_buffer.len() < remaining_native {
                self.opl3_native_buffer
                    .resize(remaining_native, YmfmOutput4 { data: [0; 4] });
            }
            self.opl3
                .generate(&mut self.opl3_native_buffer[..remaining_native]);
        }

        // OPL3 resampling and mixing
        if total_native > 0 {
            let total_interleaved = total_native * 2;
            if self.opl3_resample_input.len() < total_interleaved {
                self.opl3_resample_input.resize(total_interleaved, 0.0);
            }

            // OPL3 has 4 outputs: 0+2 = left, 1+3 = right
            const OPL3_SCALE: f32 = 1.0 / 32768.0;
            let fm_vol_left = mixer_volume_scale(self.state.mixer.registers[0x34]);
            let fm_vol_right = mixer_volume_scale(self.state.mixer.registers[0x35]);
            let master_left = mixer_volume_scale(self.state.mixer.registers[0x30]);
            let master_right = mixer_volume_scale(self.state.mixer.registers[0x31]);

            for i in 0..pending_count {
                let s = &self.opl3_pending_native[i];
                let left = (s.data[0] + s.data[2]) as f32 * OPL3_SCALE;
                let right = (s.data[1] + s.data[3]) as f32 * OPL3_SCALE;
                self.opl3_resample_input[i * 2] = left * fm_vol_left * master_left;
                self.opl3_resample_input[i * 2 + 1] = right * fm_vol_right * master_right;
            }
            for i in 0..remaining_native {
                let s = &self.opl3_native_buffer[i];
                let left = (s.data[0] + s.data[2]) as f32 * OPL3_SCALE;
                let right = (s.data[1] + s.data[3]) as f32 * OPL3_SCALE;
                let j = pending_count + i;
                self.opl3_resample_input[j * 2] = left * fm_vol_left * master_left;
                self.opl3_resample_input[j * 2 + 1] = right * fm_vol_right * master_right;
            }

            let mut input_offset = 0;
            let mut output_offset = 0;
            let sample_count = output.len();
            while input_offset < total_interleaved && output_offset < sample_count {
                let Ok((consumed, produced)) = self.opl3_resampler.resample(
                    &self.opl3_resample_input[input_offset..total_interleaved],
                    &mut self.opl3_resample_output,
                ) else {
                    break;
                };
                let usable = produced.min(sample_count - output_offset);
                for (out, &resampled) in output[output_offset..output_offset + usable]
                    .iter_mut()
                    .zip(&self.opl3_resample_output[..usable])
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

        // PCM mixing from DSP ring buffer
        if self.state.dsp.speaker_enabled && self.state.dsp.pcm_buffered > 0 {
            let pcm_vol_left = mixer_volume_scale(self.state.mixer.registers[0x32]);
            let pcm_vol_right = mixer_volume_scale(self.state.mixer.registers[0x33]);
            let master_left = mixer_volume_scale(self.state.mixer.registers[0x30]);
            let master_right = mixer_volume_scale(self.state.mixer.registers[0x31]);

            let stereo_frames = output.len() / 2;
            let format = self.state.dsp.dma_format;
            let bytes_per_sample = dma_format_bytes_per_sample(format) as usize;

            let available_samples = self
                .state
                .dsp
                .pcm_buffered
                .checked_div(bytes_per_sample)
                .unwrap_or(0);

            // Only drain as many input frames as the resampler needs to fill the
            // output buffer. This prevents data loss: without this limit, we could
            // drain far more PCM data than the resampler can place into the output,
            // silently discarding the excess.
            let pcm_rate = self.state.dsp.sample_rate.max(1) as usize;
            let needed_input = (stereo_frames * pcm_rate).div_ceil(self.sample_rate as usize) + 2;
            let samples_to_drain = available_samples.min(needed_input);
            if samples_to_drain > 0 {
                let total_interleaved = samples_to_drain * 2;
                if self.pcm_resample_input.len() < total_interleaved {
                    self.pcm_resample_input.resize(total_interleaved, 0.0);
                }

                for i in 0..samples_to_drain {
                    let (left, right) = self.drain_one_pcm_sample(format);
                    self.pcm_resample_input[i * 2] = left * pcm_vol_left * master_left;
                    self.pcm_resample_input[i * 2 + 1] = right * pcm_vol_right * master_right;
                }

                let mut input_offset = 0;
                let mut output_offset = 0;
                let sample_count = output.len();
                while input_offset < total_interleaved && output_offset < sample_count {
                    let Ok((consumed, produced)) = self.pcm_resampler.resample(
                        &self.pcm_resample_input[input_offset..total_interleaved],
                        &mut self.pcm_resample_output,
                    ) else {
                        break;
                    };
                    let usable = produced.min(sample_count - output_offset);
                    for i in 0..usable {
                        output[output_offset + i] += self.pcm_resample_output[i] * volume;
                    }
                    input_offset += consumed;
                    output_offset += usable;
                    if consumed == 0 {
                        break;
                    }
                }
            }
        }

        self.opl3_pending_native.clear();
        self.state.audio_frame_start_cycle = current_cycle;
        self.state.fm_sync_cursor = current_cycle;
    }

    fn read_pcm_byte(&mut self) -> u8 {
        if self.state.dsp.pcm_buffered == 0 {
            // Silence: 0x80 for unsigned 8-bit, 0x00 for signed 16-bit.
            return if dma_format_is_16bit(self.state.dsp.dma_format) {
                0x00
            } else {
                0x80
            };
        }
        let rp = self.state.dsp.pcm_read_pos;
        let byte = self.state.dsp.pcm_buffer[rp];
        self.state.dsp.pcm_read_pos = (rp + 1) & DMA_RING_BUF_MASK;
        self.state.dsp.pcm_buffered -= 1;
        byte
    }

    fn drain_one_pcm_sample(&mut self, format: u8) -> (f32, f32) {
        match format {
            DMA_FORMAT_UNSIGNED_8_MONO => {
                let sample = self.read_pcm_byte();
                let f = (sample as f32 - 128.0) / 128.0;
                (f, f)
            }
            DMA_FORMAT_UNSIGNED_8_STEREO => {
                let left = self.read_pcm_byte();
                let right = self.read_pcm_byte();
                (
                    (left as f32 - 128.0) / 128.0,
                    (right as f32 - 128.0) / 128.0,
                )
            }
            DMA_FORMAT_SIGNED_16_MONO => {
                let lo = self.read_pcm_byte();
                let hi = self.read_pcm_byte();
                let sample = i16::from_le_bytes([lo, hi]) as f32 / 32768.0;
                (sample, sample)
            }
            DMA_FORMAT_SIGNED_16_STEREO => {
                let lo_l = self.read_pcm_byte();
                let hi_l = self.read_pcm_byte();
                let lo_r = self.read_pcm_byte();
                let hi_r = self.read_pcm_byte();
                let left = i16::from_le_bytes([lo_l, hi_l]) as f32 / 32768.0;
                let right = i16::from_le_bytes([lo_r, hi_r]) as f32 / 32768.0;
                (left, right)
            }
            _ => (0.0, 0.0),
        }
    }

    /// Creates a snapshot of the current state.
    pub fn save_state(&self) -> SoundBlaster16State {
        self.state.clone()
    }

    /// Restores from a saved state.
    pub fn load_state(
        &mut self,
        saved: &SoundBlaster16State,
        cpu_clock_hz: u32,
        sample_rate: u32,
        current_cycle: u64,
    ) {
        self.state = saved.clone();
        self.pending_dsp_actions.clear();
        self.pcm_rate_dirty = false;

        self.cpu_clock_hz = cpu_clock_hz;
        self.opl3_action_cycle = current_cycle;
        self.opl3 = Ymf262::new();
        self.opl3.reset();
        self.opl3_native_rate = self.opl3.sample_rate(YMF262_CLOCK);
        self.opl3_resampler = ResamplerFir::new_from_hz(
            2,
            self.opl3_native_rate,
            sample_rate,
            RESAMPLER_LATENCY,
            RESAMPLER_ATTENUATION,
        );
        self.opl3_resample_output
            .resize(self.opl3_resampler.buffer_size_output(), 0.0);

        let pcm_rate = self.state.dsp.sample_rate.max(1);
        self.pcm_resampler = ResamplerFir::new_from_hz(
            2,
            pcm_rate,
            sample_rate,
            RESAMPLER_LATENCY,
            RESAMPLER_ATTENUATION,
        );
        self.pcm_resample_output
            .resize(self.pcm_resampler.buffer_size_output(), 0.0);
        self.sample_rate = sample_rate;
        self.cpu_clock_hz = cpu_clock_hz;
        self.opl3_pending_native.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sb16() -> SoundBlaster16 {
        SoundBlaster16::new(8_000_000, 48000)
    }

    #[test]
    fn dsp_reset_returns_ready_byte() {
        let mut sb16 = make_sb16();
        sb16.write_dsp_reset(0x01);
        sb16.write_dsp_reset(0x00);
        assert_eq!(sb16.read_dsp_data(), 0xAA);
    }

    #[test]
    fn dsp_version_returns_4_12() {
        let mut sb16 = make_sb16();
        sb16.write_dsp_command(0xE1);
        assert_eq!(sb16.read_dsp_data(), DSP_VERSION_MAJOR);
        assert_eq!(sb16.read_dsp_data(), DSP_VERSION_MINOR);
    }

    #[test]
    fn dsp_id_returns_complement() {
        let mut sb16 = make_sb16();
        sb16.write_dsp_command(0xE0);
        sb16.write_dsp_command(0x55);
        assert_eq!(sb16.read_dsp_data(), !0x55);
    }

    #[test]
    fn dsp_test_register_round_trip() {
        let mut sb16 = make_sb16();
        sb16.write_dsp_command(0xE4);
        sb16.write_dsp_command(0xAB);
        sb16.write_dsp_command(0xE8);
        assert_eq!(sb16.read_dsp_data(), 0xAB);
    }

    #[test]
    fn dsp_speaker_on_off() {
        let mut sb16 = make_sb16();
        assert!(!sb16.state.dsp.speaker_enabled);
        sb16.write_dsp_command(0xD1);
        assert!(sb16.state.dsp.speaker_enabled);
        sb16.write_dsp_command(0xD3);
        assert!(!sb16.state.dsp.speaker_enabled);
    }

    #[test]
    fn dsp_speaker_status() {
        let mut sb16 = make_sb16();
        sb16.write_dsp_command(0xD1);
        sb16.write_dsp_command(0xD8);
        assert_eq!(sb16.read_dsp_data(), 0xFF);

        sb16.write_dsp_command(0xD3);
        sb16.write_dsp_command(0xD8);
        assert_eq!(sb16.read_dsp_data(), 0x00);
    }

    #[test]
    fn dsp_set_sample_rate() {
        let mut sb16 = make_sb16();
        // Set rate to 44100 (0xAC44)
        sb16.write_dsp_command(0x41);
        sb16.write_dsp_command(0xAC);
        sb16.write_dsp_command(0x44);
        assert_eq!(sb16.state.dsp.sample_rate, 44100);
    }

    #[test]
    fn dsp_set_time_constant() {
        let mut sb16 = make_sb16();
        // TC = 211 -> rate = 1000000 / (256 - 211) = 22222
        sb16.write_dsp_command(0x40);
        sb16.write_dsp_command(211);
        assert_eq!(sb16.state.dsp.sample_rate, 1_000_000 / (256 - 211));
    }

    #[test]
    fn dsp_set_block_size() {
        let mut sb16 = make_sb16();
        // Block size = 0x07FF + 1 = 2048
        sb16.write_dsp_command(0x48);
        sb16.write_dsp_command(0xFF);
        sb16.write_dsp_command(0x07);
        assert_eq!(sb16.state.dsp.dma_block_size, 2048);
    }

    #[test]
    fn mixer_reset_restores_defaults() {
        let mut sb16 = make_sb16();
        sb16.write_mixer_address(0x30);
        sb16.write_mixer_data(0x00);
        assert_eq!(sb16.state.mixer.registers[0x30], 0x00);

        sb16.write_mixer_address(0x00);
        sb16.write_mixer_data(0x00);
        assert_eq!(sb16.state.mixer.registers[0x30], 0xC0);
    }

    #[test]
    fn mixer_register_round_trip() {
        let mut sb16 = make_sb16();
        sb16.write_mixer_address(0x32);
        sb16.write_mixer_data(0xA8);
        sb16.write_mixer_address(0x32);
        assert_eq!(sb16.read_mixer_data(), 0xA8);
    }

    #[test]
    fn mixer_irq_config() {
        let mut sb16 = make_sb16();
        sb16.write_mixer_address(0x80);
        sb16.write_mixer_data(0x01); // IRQ 3
        assert_eq!(sb16.state.irq_line, 3);

        sb16.write_mixer_address(0x80);
        sb16.write_mixer_data(0x04); // ISA IRQ 7 -> PC-98 IRQ 12
        assert_eq!(sb16.state.irq_line, 12);

        sb16.write_mixer_address(0x80);
        assert_eq!(sb16.read_mixer_data(), 0x04);
    }

    #[test]
    fn mixer_dma_config() {
        let mut sb16 = make_sb16();
        sb16.write_mixer_address(0x81);
        sb16.write_mixer_data(0x01); // DMA 0
        assert_eq!(sb16.state.dsp.dma_channel, 0);

        sb16.write_mixer_address(0x81);
        sb16.write_mixer_data(0x08); // DMA 3
        assert_eq!(sb16.state.dsp.dma_channel, 3);

        sb16.write_mixer_address(0x81);
        assert_eq!(sb16.read_mixer_data(), 0x08);
    }

    #[test]
    fn mixer_irq_pending_status() {
        let mut sb16 = make_sb16();
        sb16.state.dsp.irq_pending_8bit = true;
        sb16.write_mixer_address(0x82);
        assert_eq!(sb16.read_mixer_data(), 0x01);

        sb16.state.dsp.irq_pending_16bit = true;
        assert_eq!(sb16.read_mixer_data(), 0x03);
    }

    #[test]
    fn dsp_force_8bit_irq() {
        let mut sb16 = make_sb16();
        sb16.write_dsp_command(0xF2);
        assert!(sb16.state.dsp.irq_pending_8bit);
        assert!(sb16.state.irq_asserted);
    }

    #[test]
    fn dsp_read_status_clears_irq() {
        let mut sb16 = make_sb16();
        sb16.write_dsp_command(0xF2);
        assert!(sb16.state.dsp.irq_pending_8bit);
        sb16.read_dsp_status_8bit();
        assert!(!sb16.state.dsp.irq_pending_8bit);
    }

    #[test]
    fn dsp_copyright_string() {
        let mut sb16 = make_sb16();
        sb16.write_dsp_command(0xE3);
        let mut result = Vec::new();
        while let Some(&front) = sb16.state.dsp.output_buffer.front() {
            result.push(sb16.read_dsp_data());
            if front == 0 {
                break;
            }
        }
        assert_eq!(
            &result[..result.len() - 1],
            COPYRIGHT_STRING[..COPYRIGHT_STRING.len() - 1].as_ref()
        );
    }
}
