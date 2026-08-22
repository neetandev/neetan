//! Audio engine for Neetan.
//!
//! Provides an [`AudioEngine`] that streams audio samples from the emulated
//! machine to the host via SDL3's push-based audio API.
//!
//! The audio device's sample consumption rate drives the emulation speed.
//! [`AudioEngine::frames_needed`] reports how many stereo frames the emulator
//! should produce to keep the audio pipeline filled to its target level.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

use common::{Context, Machine, info, warn};
pub use common::{Error, Result};
use sdl3::audio::{AudioFormat, AudioSpec, AudioStreamOwner, AudioSubsystem};

/// Audio sample rate in Hz used by the emulator.
pub const SAMPLE_RATE: i32 = 48000;

/// Desired number of stereo frames buffered in the audio pipeline (50 ms at 48 kHz).
const TARGET_BUFFER_FRAMES: usize = 2400;

/// Frames to produce per chunk (~5 ms at 48 kHz).
pub const STEP_FRAMES: usize = 240;

/// Bytes per stereo frame: 2 channels * 4 bytes per f32.
const BYTES_PER_FRAME: i32 = 8;

/// Number of stereo frames over which to fade in/out (250 ms at 48 kHz).
const FADE_FRAMES: u32 = 12_000;

/// Streams audio samples from the emulated machine to the host sound device.
pub struct AudioEngine {
    stream: AudioStreamOwner,
    sample_buffer: Vec<f32>,
    silence_buffer: Vec<f32>,
    volume: f32,
    /// Remaining stereo frames to fade out (counts down to 0, then SDL3 pauses).
    fade_out_remaining: u32,
    /// Remaining stereo frames to fade in (counts down to 0).
    fade_in_remaining: u32,
}

impl AudioEngine {
    /// Creates a new audio engine, consuming the audio subsystem.
    ///
    /// The subsystem lifetime is managed by the resulting audio stream.
    pub fn new(audio_subsystem: AudioSubsystem, volume: f32) -> Result<Self> {
        let spec = AudioSpec {
            freq: Some(SAMPLE_RATE),
            channels: Some(2),
            format: Some(AudioFormat::f32_sys()),
        };

        let device = audio_subsystem
            .open_playback_device(&spec)
            .context("Failed to open audio playback device")?;

        let stream = device
            .open_device_stream(Some(&spec))
            .context("Failed to open audio device stream")?;

        let silence = vec![0.0f32; TARGET_BUFFER_FRAMES * 2];
        stream
            .put_data_f32(&silence)
            .context("Failed to pre-fill audio stream")?;

        stream.resume().context("Failed to resume audio stream")?;

        info!(
            "Audio output initialized: {SAMPLE_RATE} Hz, stereo, f32 (push-based, target buffer: {TARGET_BUFFER_FRAMES} frames)"
        );

        Ok(Self {
            stream,
            sample_buffer: vec![0.0; STEP_FRAMES * 2],
            silence_buffer: vec![0.0; TARGET_BUFFER_FRAMES * 2],
            volume,
            fade_out_remaining: 0,
            fade_in_remaining: 0,
        })
    }

    /// Sets the playback volume (0.0–1.0).
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
    }

    /// Returns the number of stereo frames currently buffered in the audio pipeline.
    pub fn buffered_frames(&self) -> u32 {
        let bytes = self.stream.queued_bytes().unwrap_or(0).max(0);
        (bytes / BYTES_PER_FRAME) as u32
    }

    /// Returns how many new stereo frames the emulator should produce to
    /// maintain the target buffer level. Returns 0 when the buffer is full.
    ///
    /// Always returns a fixed chunk size (`STEP_FRAMES`) to ensure uniform
    /// audio generation intervals. Variable chunk sizes cause the FM
    /// resampler to produce fewer output frames than expected (the native
    /// FM rate ~7987 Hz means small chunks can round to 0 native samples),
    /// leaving silence gaps that manifest as crackling.
    pub fn frames_needed(&self) -> u32 {
        let current = self.buffered_frames() as usize;
        if current >= TARGET_BUFFER_FRAMES {
            return 0;
        }
        STEP_FRAMES as u32
    }

    /// Begins a fade-out. The SDL3 device is paused once the fade completes
    /// inside [`push_samples`].
    pub fn pause(&mut self) {
        self.fade_out_remaining = FADE_FRAMES;
        self.fade_in_remaining = 0;
    }

    /// Resumes audio playback with a fade-in to prevent pops.
    pub fn resume(&mut self) {
        if self.fade_out_remaining > 0 {
            // Fade-out was still in progress; cancel it.
            self.fade_out_remaining = 0;
        } else {
            // Device was actually paused; restart it.
            let _ = self.stream.clear();
            let _ = self.stream.put_data_f32(&self.silence_buffer);
            let _ = self.stream.resume();
        }
        self.fade_in_remaining = FADE_FRAMES;
    }

    /// Discards all queued audio and re-fills the buffer with silence.
    pub fn reset_buffer(&mut self) {
        let _ = self.stream.clear();
        let _ = self.stream.put_data_f32(&self.silence_buffer);
    }

    /// Drains pending audio samples from the machine without sending them to
    /// the device.
    pub fn discard_samples(&mut self, machine: &mut dyn Machine) {
        self.sample_buffer.fill(0.0);
        let _ = machine.generate_audio_samples(self.volume, &mut self.sample_buffer);
    }

    /// Drains pending audio samples from the machine and pushes them to the audio stream.
    pub fn push_samples(&mut self, machine: &mut dyn Machine) {
        self.sample_buffer.fill(0.0);
        let written = machine.generate_audio_samples(self.volume, &mut self.sample_buffer);
        if written > 0 {
            // Soft clipping via tanh() to prevent digital clipping.
            self.sample_buffer[..written]
                .iter_mut()
                .for_each(|sample| *sample = fast_tanh(*sample));

            let frames = written / 2;
            let was_fading_out = self.fade_out_remaining > 0;

            // Apply fade-out ramp toward pause.
            if was_fading_out {
                for i in 0..frames {
                    if self.fade_out_remaining > 0 {
                        self.fade_out_remaining -= 1;
                        let gain = self.fade_out_remaining as f32 / FADE_FRAMES as f32;
                        self.sample_buffer[i * 2] *= gain;
                        self.sample_buffer[i * 2 + 1] *= gain;
                    } else {
                        self.sample_buffer[i * 2] = 0.0;
                        self.sample_buffer[i * 2 + 1] = 0.0;
                    }
                }
            }

            // Apply fade-in ramp after resume.
            if self.fade_in_remaining > 0 {
                for i in 0..frames {
                    if self.fade_in_remaining > 0 {
                        let gain = 1.0 - self.fade_in_remaining as f32 / FADE_FRAMES as f32;
                        self.sample_buffer[i * 2] *= gain;
                        self.sample_buffer[i * 2 + 1] *= gain;
                        self.fade_in_remaining -= 1;
                    }
                }
            }

            if let Err(error) = self.stream.put_data_f32(&self.sample_buffer[..written]) {
                warn!("Audio stream put_data_f32 failed: {error}");
            }

            // Once the fade-out completes, pause the SDL3 device.
            if was_fading_out && self.fade_out_remaining == 0 {
                let _ = self.stream.clear();
                let _ = self.stream.put_data_f32(&self.silence_buffer);
                let _ = self.stream.pause();
            }
        } else {
            warn!("Empty audio generation");
        }
    }
}

// [3/2] Pade approximant: tanh(x) ≈ x(15 + x²) / (15 + 6x²)
//
// Used for auto-vectorization.
//
// Clamped to [-1, 1] for |x| >= 3
//
// Source: https://mathr.co.uk/blog/2017-09-06_approximating_hyperbolic_tangent.html
#[inline(always)]
fn fast_tanh(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (15.0 + x2) / (15.0 + 6.0 * x2)
}
