//! CD-DA audio playback engine.
//!
//! Reads raw audio sectors from a `CdImage` and produces resampled stereo
//! PCM output that can be mixed into the emulator's audio stream. Each CD
//! audio sector is 2352 bytes containing 588 interleaved 16-bit signed
//! little-endian stereo samples at 44100 Hz.

use resampler::{Attenuation, Latency, ResamplerFir, ResamplerFirState};

use crate::cdrom::CdImage;

/// CD-DA native sample rate.
const CD_SAMPLE_RATE: u32 = 44100;

/// Bytes per raw audio sector.
const SECTOR_BYTES: usize = 2352;

/// Stereo samples per sector (2352 / 4).
const SAMPLES_PER_SECTOR: usize = SECTOR_BYTES / 4;

/// Resampler tuning.
const RESAMPLER_LATENCY: Latency = Latency::Sample32;
const RESAMPLER_ATTENUATION: Attenuation = Attenuation::Db90;

save_state::runtime_state_enum! {
/// Playback state of the CD audio engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdAudioState {
    /// No audio is playing.
    Stopped = 0,
    /// Audio is actively being generated.
    Playing = 1,
    /// Audio was playing and has been paused.
    Paused = 2,
}}

save_state::runtime_state! {
/// Per-output-channel mapping and volume.
#[derive(Debug, Clone)]
pub struct AudioChannelControl {
    /// Which input channel (0 or 1) feeds each of the four output slots.
    pub input_channel: [u8; 4],
    /// Volume for each output slot (0-255).
    pub volume: [u8; 4],
}}

save_state::runtime_state! {
/// Complete CD audio playback and resampler state.
#[derive(Clone)]
pub struct CdAudioPlayerState {
    state: CdAudioState,
    start_lba: u32,
    end_lba: u32,
    current_lba: u32,
    sector_buffer: Vec<f32>,
    buffer_position: usize,
    channels: AudioChannelControl,
    resampler: ResamplerFirState,
    resample_output: Vec<f32>,
    resample_output_position: usize,
    resample_output_available: usize,
    output_sample_rate: u32,
}}

impl Default for AudioChannelControl {
    fn default() -> Self {
        Self {
            input_channel: [0, 1, 2, 3],
            volume: [255, 255, 255, 255],
        }
    }
}

/// CD-DA audio player.
///
/// Reads audio sectors from a `CdImage`, resamples from 44100 Hz to the
/// emulator's output sample rate, and mixes the result into the audio
/// output buffer.
#[derive(Debug)]
pub struct CdAudioPlayer {
    state: CdAudioState,
    start_lba: u32,
    end_lba: u32,
    current_lba: u32,
    sector_buffer: Vec<f32>,
    buffer_position: usize,
    channels: AudioChannelControl,
    resampler: ResamplerFir,
    resample_output: Vec<f32>,
    resample_output_position: usize,
    resample_output_available: usize,
    output_sample_rate: u32,
}

impl CdAudioPlayer {
    /// Creates a new idle player targeting the given output sample rate.
    pub fn new(output_sample_rate: u32) -> Self {
        let resampler = ResamplerFir::new_from_hz(
            2,
            CD_SAMPLE_RATE,
            output_sample_rate,
            RESAMPLER_LATENCY,
            RESAMPLER_ATTENUATION,
        );
        let resample_output = vec![0.0; resampler.buffer_size_output()];

        Self {
            state: CdAudioState::Stopped,
            start_lba: 0,
            end_lba: 0,
            current_lba: 0,
            sector_buffer: Vec::with_capacity(SAMPLES_PER_SECTOR * 2),
            buffer_position: 0,
            channels: AudioChannelControl::default(),
            resampler,
            resample_output,
            resample_output_position: 0,
            resample_output_available: 0,
            output_sample_rate,
        }
    }

    /// Captures playback position and every pending audio sample.
    pub fn capture_state(&self) -> CdAudioPlayerState {
        CdAudioPlayerState {
            state: self.state,
            start_lba: self.start_lba,
            end_lba: self.end_lba,
            current_lba: self.current_lba,
            sector_buffer: self.sector_buffer.clone(),
            buffer_position: self.buffer_position,
            channels: self.channels.clone(),
            resampler: self.resampler.capture_state(),
            resample_output: self.resample_output.clone(),
            resample_output_position: self.resample_output_position,
            resample_output_available: self.resample_output_available,
            output_sample_rate: self.output_sample_rate,
        }
    }

    /// Validates playback history against the configured output stream.
    pub fn validate_state(
        &self,
        state: &CdAudioPlayerState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.output_sample_rate != self.output_sample_rate
            || state.buffer_position > state.sector_buffer.len()
            || state.resample_output_position > state.resample_output_available
            || state.resample_output_available > state.resample_output.len()
            || state.current_lba < state.start_lba
            || state.current_lba > state.end_lba.saturating_add(1)
        {
            return Err(save_state::StateValidationError::new(
                "CD audio playback position is invalid",
            ));
        }
        self.resampler.validate_state(&state.resampler)
    }

    /// Restores playback without reloading or advancing a disc sector.
    pub fn restore_state(
        &mut self,
        state: CdAudioPlayerState,
    ) -> Result<(), save_state::StateValidationError> {
        self.validate_state(&state)?;
        self.resampler.restore_state(state.resampler)?;
        self.state = state.state;
        self.start_lba = state.start_lba;
        self.end_lba = state.end_lba;
        self.current_lba = state.current_lba;
        self.sector_buffer = state.sector_buffer;
        self.buffer_position = state.buffer_position;
        self.channels = state.channels;
        self.resample_output = state.resample_output;
        self.resample_output_position = state.resample_output_position;
        self.resample_output_available = state.resample_output_available;
        Ok(())
    }

    /// Begins playback from `start_lba` for `sector_count` sectors.
    pub fn play(&mut self, cd_image: &CdImage, start_lba: u32, sector_count: u32) {
        self.start_lba = start_lba;
        self.end_lba = start_lba + sector_count;
        self.current_lba = start_lba;
        self.buffer_position = 0;
        self.sector_buffer.clear();
        self.resample_output_position = 0;
        self.resample_output_available = 0;
        self.load_sector(cd_image);
        self.state = CdAudioState::Playing;
    }

    /// Stops playback. If currently playing, transitions to Paused so that
    /// `resume()` can continue. If already stopped/paused, stays Stopped.
    pub fn stop(&mut self) {
        match self.state {
            CdAudioState::Playing => self.state = CdAudioState::Paused,
            CdAudioState::Paused => {}
            CdAudioState::Stopped => {}
        }
    }

    /// Resumes from a paused state.
    pub fn resume(&mut self, cd_image: &CdImage) {
        if self.state == CdAudioState::Paused {
            if self.sector_buffer.is_empty() {
                self.load_sector(cd_image);
            }
            self.state = CdAudioState::Playing;
        }
    }

    /// Resets the player to stopped state, clearing all positions.
    pub fn reset(&mut self) {
        self.state = CdAudioState::Stopped;
        self.start_lba = 0;
        self.end_lba = 0;
        self.current_lba = 0;
        self.buffer_position = 0;
        self.sector_buffer.clear();
        self.resample_output_position = 0;
        self.resample_output_available = 0;
    }

    /// Returns the current playback state.
    pub fn state(&self) -> CdAudioState {
        self.state
    }

    /// Returns `(current_lba, start_lba, end_lba)`.
    pub fn current_position(&self) -> (u32, u32, u32) {
        (self.current_lba, self.start_lba, self.end_lba)
    }

    /// Returns a reference to the current channel control settings.
    pub fn channels(&self) -> &AudioChannelControl {
        &self.channels
    }

    /// Sets the channel control mapping and volume.
    pub fn set_channels(&mut self, control: AudioChannelControl) {
        self.channels = control;
    }

    /// Generates resampled stereo audio and additively mixes it into `output`.
    ///
    /// `output` is interleaved stereo `[L, R, L, R, ...]`. `volumes` scales
    /// the mixed result per channel (`[left, right]`). `cd_image` is used to
    /// read additional sectors as playback advances.
    pub fn generate_samples(&mut self, cd_image: &CdImage, volumes: [f32; 2], output: &mut [f32]) {
        if self.state != CdAudioState::Playing || output.is_empty() {
            return;
        }

        let mut output_position = 0;

        while output_position < output.len() {
            // Drain any leftover resampled output from a previous call/iteration.
            if self.resample_output_position < self.resample_output_available {
                let available = self.resample_output_available - self.resample_output_position;
                let needed = output.len() - output_position;
                let mix_count = available.min(needed);
                for i in 0..mix_count {
                    let volume = volumes[(output_position + i) & 1];
                    output[output_position + i] +=
                        self.resample_output[self.resample_output_position + i] * volume;
                }
                output_position += mix_count;
                self.resample_output_position += mix_count;
                continue;
            }

            // Load the next sector when the current one is exhausted.
            if self.buffer_position >= self.sector_buffer.len() {
                if self.current_lba >= self.end_lba {
                    self.state = CdAudioState::Stopped;
                    return;
                }
                self.load_sector(cd_image);
                if self.sector_buffer.is_empty() {
                    self.state = CdAudioState::Stopped;
                    return;
                }
            }

            let input_remaining = &self.sector_buffer[self.buffer_position..];
            let (consumed, produced) = self
                .resampler
                .resample(input_remaining, &mut self.resample_output)
                .unwrap_or((0, 0));

            self.buffer_position += consumed;
            self.resample_output_position = 0;
            self.resample_output_available = produced;

            if consumed == 0 && produced == 0 {
                break;
            }
        }
    }

    /// Updates the output sample rate, rebuilding the resampler.
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        if sample_rate == self.output_sample_rate {
            return;
        }
        self.output_sample_rate = sample_rate;
        self.resampler = ResamplerFir::new_from_hz(
            2,
            CD_SAMPLE_RATE,
            sample_rate,
            RESAMPLER_LATENCY,
            RESAMPLER_ATTENUATION,
        );
        self.resample_output
            .resize(self.resampler.buffer_size_output(), 0.0);
    }

    /// Loads the next audio sector from `cd_image` into the internal buffer,
    /// converting 16-bit signed LE samples to f32 with channel mapping applied.
    fn load_sector(&mut self, cd_image: &CdImage) {
        self.sector_buffer.clear();
        self.buffer_position = 0;

        let mut raw = [0u8; SECTOR_BYTES];
        let Some(_) = cd_image.read_sector_raw(self.current_lba, &mut raw) else {
            return;
        };
        self.current_lba += 1;

        let left_input = self.channels.input_channel[0].min(1) as usize;
        let right_input = self.channels.input_channel[1].min(1) as usize;
        let left_volume = self.channels.volume[0] as f32 / 255.0;
        let right_volume = self.channels.volume[1] as f32 / 255.0;

        self.sector_buffer.reserve(SAMPLES_PER_SECTOR * 2);
        for i in 0..SAMPLES_PER_SECTOR {
            let offset = i * 4;
            let sample_left = i16::from_le_bytes([raw[offset], raw[offset + 1]]) as f32 / 32768.0;
            let sample_right =
                i16::from_le_bytes([raw[offset + 2], raw[offset + 3]]) as f32 / 32768.0;

            let inputs = [sample_left, sample_right];
            self.sector_buffer.push(inputs[left_input] * left_volume);
            self.sector_buffer.push(inputs[right_input] * right_volume);
        }
    }
}
