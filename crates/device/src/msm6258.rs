//! OKI MSM6258 ADPCM voice synthesizer.
//!
//! The chip decodes 4-bit OKI ADPCM nibbles in the 12-bit signal domain and
//! plays the result through the built-in DAC at full output swing. One
//! command register starts and stops playback, and one data register accepts
//! an encoded byte holding two nibbles. The host paces data consumption with
//! one decode tick per encoded byte: the tick decodes both nibbles (low nibble
//! first) or, when no byte arrived in time, repeats the held sample with a
//! slow decay. Each decoded sample is emitted at the selected sampling rate,
//! derived from the master clock divided by the divider ratio, and rendered
//! internally on a fixed 62,500 Hz native grid.
//!
//! The signal processing follows XEiJ, not MAME: the ADPCM signal is decoded
//! in the full 12-bit domain and played at full output swing, instead of
//! MAME's 10-bit clamp that saturates loud content and plays it at a quarter
//! of the mix scale.

use resampler::{Attenuation, Latency, ResamplerFir};

/// High master clock in Hz.
pub const MSM6258_CLOCK_HIGH_HZ: u32 = 8_000_000;
/// Low master clock in Hz.
pub const MSM6258_CLOCK_LOW_HZ: u32 = 4_000_000;
/// Fixed internal generation rate in Hz shared by every sampling rate.
pub const MSM6258_NATIVE_SAMPLE_RATE: u32 = 62_500;

/// Length of an output-line gain transition on the native sample grid.
const PAN_FADE_SAMPLES: u16 = 1024;

/// Status bit reporting active playback.
const STATUS_PLAYING: u8 = 0x80;
/// Status bits that always read as set.
const STATUS_FIXED: u8 = 0x40;
/// Command bit requesting playback stop.
const COMMAND_STOP: u8 = 0x01;
/// Command bit requesting playback start.
const COMMAND_PLAY: u8 = 0x02;

/// Quantizer step sizes indexed by the predictor index: floor(16 * 1.1^p).
const STEP_TABLE: [i32; 49] = [
    16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130,
    143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796,
    876, 963, 1060, 1166, 1282, 1411, 1552,
];

/// Predictor index adjustment indexed by the low three nibble bits.
const PREDICTOR_ADJUSTMENT: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

/// Decoded samples are held shifted left by four bits so the under-run decay
/// moves in sixteenth steps of one 12-bit unit.
const SAMPLE_SHIFT: i32 = 4;
/// Lower clamp of the 12-bit ADPCM signal in the shifted domain.
const SAMPLE_MINIMUM: i32 = -2048 << SAMPLE_SHIFT;
/// Upper clamp of the 12-bit ADPCM signal in the shifted domain.
const SAMPLE_MAXIMUM: i32 = 2047 << SAMPLE_SHIFT;
/// Scale converting the shifted signal through the 16-bit PCM mix domain.
const SAMPLE_NORMALIZATION: f32 = 32768.0;

/// Resampler latency selection shared with the other sound devices.
const RESAMPLER_LATENCY: Latency = Latency::Sample64;
/// Resampler attenuation selection shared with the other sound devices.
const RESAMPLER_ATTENUATION: Attenuation = Attenuation::Db60;

/// Playback state change reported by a command write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msm6258Command {
    /// Playback started; the host should begin requesting data.
    Started,
    /// Playback stopped; the host should stop requesting data.
    Stopped,
    /// The command changed nothing.
    Unchanged,
}

save_state::runtime_state_enum! {
/// Sampling-clock divider selected by the X68000 PPI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msm6258Divider {
    /// Divide the master clock by 1024.
    Divide1024 = 0,
    /// Divide the master clock by 768.
    Divide768 = 1,
    /// Divide the master clock by 512.
    Divide512 = 2,
    /// Reserved selection that inhibits sample pacing.
    Inhibited = 3,
}}

impl Msm6258Divider {
    /// Decodes the two PPI divider bits.
    const fn from_bits(bits: u8) -> Self {
        match bits & 3 {
            0 => Self::Divide1024,
            1 => Self::Divide768,
            2 => Self::Divide512,
            _ => Self::Inhibited,
        }
    }

    /// Returns the divider ratio, or `None` when pacing is inhibited.
    pub const fn ratio(self) -> Option<u32> {
        match self {
            Self::Divide1024 => Some(1024),
            Self::Divide768 => Some(768),
            Self::Divide512 => Some(512),
            Self::Inhibited => None,
        }
    }
}

save_state::runtime_state! {
/// Pending stereo-pan envelope progress.
#[derive(Debug, Clone, Copy)]
struct PanEnvelope {
    position: u16,
    enabled: bool,
}}

save_state::runtime_state! {
/// Complete MSM6258 voice and resampling state.
#[derive(Clone)]
pub struct Msm6258State {
    playing: bool,
    pending_byte: Option<u8>,
    predictor_index: u8,
    held_sample: i32,
    clock_low: bool,
    divider: Msm6258Divider,
    last_valid_divider: Msm6258Divider,
    left_pan: PanEnvelope,
    right_pan: PanEnvelope,
    native_buffer: Vec<f32>,
    resampler: resampler::ResamplerFirState,
    resample_output: Vec<f32>,
    sample_rate: u32,
}}

impl PanEnvelope {
    const fn disabled() -> Self {
        Self {
            position: 0,
            enabled: false,
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn next_gain(&mut self) -> f32 {
        let gain = match self.position {
            0 => 0.0,
            PAN_FADE_SAMPLES => 1.0,
            position => {
                let phase =
                    std::f32::consts::PI * f32::from(position) / f32::from(PAN_FADE_SAMPLES);
                0.5 - 0.5 * phase.cos()
            }
        };
        if self.enabled {
            self.position = self.position.saturating_add(1).min(PAN_FADE_SAMPLES);
        } else {
            self.position = self.position.saturating_sub(1);
        }
        gain
    }
}

/// An OKI MSM6258 ADPCM voice synthesizer.
pub struct Msm6258 {
    playing: bool,
    pending_byte: Option<u8>,
    predictor_index: u8,
    held_sample: i32,
    clock_low: bool,
    divider: Msm6258Divider,
    last_valid_divider: Msm6258Divider,
    left_pan: PanEnvelope,
    right_pan: PanEnvelope,
    native_buffer: Vec<f32>,
    resampler: ResamplerFir,
    resample_output: Vec<f32>,
    sample_rate: u32,
}

impl Msm6258 {
    /// Creates a chip rendering at `sample_rate` Hz host output.
    pub fn new(sample_rate: u32) -> Self {
        let resampler = ResamplerFir::new_from_hz(
            2,
            MSM6258_NATIVE_SAMPLE_RATE,
            sample_rate,
            RESAMPLER_LATENCY,
            RESAMPLER_ATTENUATION,
        );
        let resample_output_size = resampler.buffer_size_output();
        Self {
            playing: false,
            pending_byte: None,
            predictor_index: 0,
            held_sample: 0,
            clock_low: false,
            divider: Msm6258Divider::Divide512,
            last_valid_divider: Msm6258Divider::Divide512,
            left_pan: PanEnvelope::disabled(),
            right_pan: PanEnvelope::disabled(),
            native_buffer: Vec::new(),
            resampler,
            resample_output: vec![0.0; resample_output_size],
            sample_rate,
        }
    }

    /// Captures voice, queued samples, filter history, and audio phase.
    pub fn capture_state(&self) -> Msm6258State {
        Msm6258State {
            playing: self.playing,
            pending_byte: self.pending_byte,
            predictor_index: self.predictor_index,
            held_sample: self.held_sample,
            clock_low: self.clock_low,
            divider: self.divider,
            last_valid_divider: self.last_valid_divider,
            left_pan: self.left_pan,
            right_pan: self.right_pan,
            native_buffer: self.native_buffer.clone(),
            resampler: self.resampler.capture_state(),
            resample_output: self.resample_output.clone(),
            sample_rate: self.sample_rate,
        }
    }

    /// Restores voice, queued samples, filter history, and audio phase.
    pub fn restore_state(
        &mut self,
        state: Msm6258State,
    ) -> Result<(), save_state::StateValidationError> {
        if state.sample_rate != self.sample_rate
            || state.native_buffer.len() > 1 << 20
            || state.resample_output.len() != self.resample_output.len()
            || state.predictor_index > 48
        {
            return Err(save_state::StateValidationError::new(
                "MSM6258 state is invalid",
            ));
        }
        self.resampler.restore_state(state.resampler)?;
        self.playing = state.playing;
        self.pending_byte = state.pending_byte;
        self.predictor_index = state.predictor_index;
        self.held_sample = state.held_sample;
        self.clock_low = state.clock_low;
        self.divider = state.divider;
        self.last_valid_divider = state.last_valid_divider;
        self.left_pan = state.left_pan;
        self.right_pan = state.right_pan;
        self.native_buffer = state.native_buffer;
        self.resample_output = state.resample_output;
        Ok(())
    }

    /// Resets the chip to its power-on state.
    pub fn reset(&mut self) {
        self.playing = false;
        self.pending_byte = None;
        self.predictor_index = 0;
        self.held_sample = 0;
        self.clock_low = false;
        self.divider = Msm6258Divider::Divide512;
        self.last_valid_divider = Msm6258Divider::Divide512;
        self.left_pan = PanEnvelope::disabled();
        self.right_pan = PanEnvelope::disabled();
        self.native_buffer.clear();
        self.resampler = ResamplerFir::new_from_hz(
            2,
            MSM6258_NATIVE_SAMPLE_RATE,
            self.sample_rate,
            RESAMPLER_LATENCY,
            RESAMPLER_ATTENUATION,
        );
    }

    /// Reads the status register.
    pub fn read_status(&self) -> u8 {
        STATUS_FIXED | if self.playing { STATUS_PLAYING } else { 0 }
    }

    /// Writes the command register, reporting the playback state change.
    pub fn write_command(&mut self, value: u8) -> Msm6258Command {
        if value & COMMAND_STOP != 0 {
            let was_playing = self.playing;
            self.playing = false;
            self.pending_byte = None;
            self.predictor_index = 0;
            return if was_playing {
                Msm6258Command::Stopped
            } else {
                Msm6258Command::Unchanged
            };
        }
        if value & COMMAND_PLAY != 0 && !self.playing {
            self.playing = true;
            self.pending_byte = None;
            return Msm6258Command::Started;
        }
        Msm6258Command::Unchanged
    }

    /// Writes one encoded byte, returning whether playback accepted it.
    pub fn write_data(&mut self, value: u8) -> bool {
        if !self.playing {
            return false;
        }
        self.pending_byte = Some(value);
        true
    }

    /// Selects the master clock (true selects the low 4 MHz clock).
    pub fn set_clock_low(&mut self, low: bool) {
        self.clock_low = low;
    }

    /// Selects the sampling-rate divider from the two divider bits.
    pub fn set_divider(&mut self, bits: u8) {
        self.divider = Msm6258Divider::from_bits(bits);
        if self.divider != Msm6258Divider::Inhibited {
            self.last_valid_divider = self.divider;
        }
    }

    /// Enables or disables the left and right output lines.
    pub fn set_output_enable(&mut self, left: bool, right: bool) {
        self.left_pan.set_enabled(left);
        self.right_pan.set_enabled(right);
    }

    /// Returns whether playback is active.
    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Returns the selected master clock in Hz.
    pub fn master_clock_hz(&self) -> u32 {
        if self.clock_low {
            MSM6258_CLOCK_LOW_HZ
        } else {
            MSM6258_CLOCK_HIGH_HZ
        }
    }

    /// Returns the selected divider ratio.
    pub fn divider(&self) -> Msm6258Divider {
        self.divider
    }

    /// Returns the selected divider ratio when sample pacing is enabled.
    pub fn divider_ratio(&self) -> Option<u32> {
        self.divider.ratio()
    }

    /// Returns the selected sampling rate in Hz, rounded down.
    pub fn sampling_rate_hz(&self) -> Option<u32> {
        self.divider_ratio()
            .map(|divider| self.master_clock_hz() / divider)
    }

    /// Returns how often each decoded sample repeats on the native grid.
    fn repeat_count(&self) -> Option<u32> {
        self.divider_ratio()
            .map(|divider| (MSM6258_NATIVE_SAMPLE_RATE * divider / self.master_clock_hz()).max(1))
    }

    /// Returns whether an encoded byte is already waiting for the next tick.
    pub fn data_pending(&self) -> bool {
        self.pending_byte.is_some()
    }

    /// Returns enough native samples to fill `frame_count` output frames.
    fn native_frames_needed(&self, frame_count: usize) -> usize {
        (frame_count * MSM6258_NATIVE_SAMPLE_RATE as usize)
            .div_ceil(self.sample_rate.max(1) as usize)
            + 2
    }

    /// Performs one decode tick covering one encoded byte (two samples),
    /// returning whether playback continues and wants the next byte.
    pub fn consume_byte_tick(&mut self) -> bool {
        if !self.playing || self.divider == Msm6258Divider::Inhibited {
            return false;
        }
        let repeat = self
            .repeat_count()
            .expect("valid divider has a repeat count");
        if let Some(byte) = self.pending_byte.take() {
            let low_nibble = if byte == 0x00 && self.held_sample >= 0 {
                0x08
            } else if byte == 0x88 && self.held_sample < 0 {
                0x00
            } else {
                byte & 0x0F
            };
            self.decode_nibble(low_nibble);
            self.push_native_sample(repeat);
            self.decode_nibble(byte >> 4);
            self.push_native_sample(repeat);
        } else {
            self.decay_held_sample();
            self.push_native_sample(repeat);
            self.decay_held_sample();
            self.push_native_sample(repeat);
        }
        true
    }

    /// Decodes one nibble into the held sample and predictor index.
    fn decode_nibble(&mut self, nibble: u8) {
        let step = STEP_TABLE[self.predictor_index as usize];
        let mut magnitude = step >> 3;
        if nibble & 4 != 0 {
            magnitude += step;
        }
        if nibble & 2 != 0 {
            magnitude += step >> 1;
        }
        if nibble & 1 != 0 {
            magnitude += step >> 2;
        }
        let delta = if nibble & 8 != 0 {
            -magnitude
        } else {
            magnitude
        };
        self.held_sample =
            (self.held_sample + (delta << SAMPLE_SHIFT)).clamp(SAMPLE_MINIMUM, SAMPLE_MAXIMUM);
        let adjusted =
            i32::from(self.predictor_index) + PREDICTOR_ADJUSTMENT[(nibble & 7) as usize];
        self.predictor_index = adjusted.clamp(0, 48) as u8;
    }

    /// Moves the held sample one shifted unit toward silence.
    fn decay_held_sample(&mut self) {
        self.held_sample -= self.held_sample.signum();
    }

    /// Emits the held sample `repeat` times onto the native grid.
    fn push_native_sample(&mut self, repeat: u32) {
        let value = self.held_sample as f32 / SAMPLE_NORMALIZATION;
        for _ in 0..repeat {
            self.native_buffer.push(value * self.left_pan.next_gain());
            self.native_buffer.push(value * self.right_pan.next_gain());
        }
    }

    /// Extends the native queue with release-decay samples while stopped,
    /// bounded to what the current output block consumes so the decay stays
    /// aligned with real time instead of delaying later playback.
    fn push_release_samples(&mut self, native_needed: usize) {
        let divider = self
            .divider_ratio()
            .or_else(|| self.last_valid_divider.ratio())
            .expect("the remembered divider is valid");
        let repeat = (MSM6258_NATIVE_SAMPLE_RATE * divider / self.master_clock_hz()).max(1);
        while self.held_sample != 0 && self.native_buffer.len() < native_needed * 2 {
            self.decay_held_sample();
            self.push_native_sample(repeat);
        }
    }

    /// Resamples pending native samples and additively mixes them into
    /// `output` (interleaved stereo) at `volume`, honoring the output lines.
    pub fn generate_samples(&mut self, volume: f32, output: &mut [f32]) {
        let frame_count = output.len() / 2;
        if !self.playing && self.held_sample != 0 {
            self.push_release_samples(self.native_frames_needed(frame_count));
        }
        if self.native_buffer.is_empty() {
            return;
        }
        let total_native = self
            .native_buffer
            .len()
            .min(self.native_frames_needed(frame_count) * 2)
            / 2
            * 2;
        let mut input_offset = 0;
        let mut output_offset = 0;
        while input_offset < total_native && output_offset < output.len() {
            // Match the SB16 PCM path: feed only enough queued PCM to satisfy
            // this output buffer, and cap each resampler write to the remaining
            // output space. Unused queued native samples stay in native_buffer.
            let remaining_samples =
                (output.len() - output_offset).min(self.resample_output.len()) / 2 * 2;
            let out_buffer = &mut self.resample_output[..remaining_samples];
            let Ok((consumed, produced)) = self
                .resampler
                .resample(&self.native_buffer[input_offset..total_native], out_buffer)
            else {
                break;
            };
            for (index, sample) in out_buffer[..produced].iter().enumerate() {
                output[output_offset + index] += sample * volume;
            }
            input_offset += consumed;
            output_offset += produced;
            if consumed == 0 && produced == 0 {
                break;
            }
        }
        self.native_buffer.drain(..input_offset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reports_the_playback_bit() {
        let mut chip = Msm6258::new(48_000);
        assert_eq!(chip.read_status(), 0x40);
        assert_eq!(chip.write_command(0x02), Msm6258Command::Started);
        assert_eq!(chip.read_status(), 0xC0);
        assert_eq!(chip.write_command(0x01), Msm6258Command::Stopped);
        assert_eq!(chip.read_status(), 0x40);
    }

    #[test]
    fn commands_report_only_real_state_changes() {
        let mut chip = Msm6258::new(48_000);
        assert_eq!(chip.write_command(0x01), Msm6258Command::Unchanged);
        assert_eq!(chip.write_command(0x02), Msm6258Command::Started);
        assert_eq!(chip.write_command(0x02), Msm6258Command::Unchanged);
        assert_eq!(chip.write_command(0x03), Msm6258Command::Stopped);
    }

    #[test]
    fn data_writes_require_active_playback() {
        let mut chip = Msm6258::new(48_000);
        assert!(!chip.write_data(0x11));
        chip.write_command(0x02);
        assert!(chip.write_data(0x11));
    }

    #[test]
    fn sampling_rates_follow_the_clock_and_divider() {
        let mut chip = Msm6258::new(48_000);
        let expectations = [
            (false, 0, 7_812, 8),
            (false, 1, 10_416, 6),
            (false, 2, 15_625, 4),
            (true, 0, 3_906, 16),
            (true, 1, 5_208, 12),
            (true, 2, 7_812, 8),
        ];
        for (clock_low, divider, rate, repeat) in expectations {
            chip.set_clock_low(clock_low);
            chip.set_divider(divider);
            assert_eq!(chip.sampling_rate_hz(), Some(rate));
            assert_eq!(chip.repeat_count(), Some(repeat));
        }
        chip.set_divider(3);
        assert_eq!(chip.divider(), Msm6258Divider::Inhibited);
        assert_eq!(chip.divider_ratio(), None);
        assert_eq!(chip.sampling_rate_hz(), None);
        assert_eq!(chip.repeat_count(), None);
    }

    #[test]
    fn decode_matches_hand_computed_oki_steps() {
        let mut chip = Msm6258::new(48_000);
        chip.write_command(0x02);

        // Nibble 0x2 at index 0: step 16, delta 8 + 2 = 10, index stays 0.
        // Nibble 0x7 at index 0: delta 16 + 8 + 4 + 2 = 30, index rises to 8.
        chip.write_data(0x72);
        assert!(chip.consume_byte_tick());
        assert_eq!(chip.held_sample >> SAMPLE_SHIFT, 40);
        assert_eq!(chip.predictor_index, 8);

        // Nibble 0xF at index 8: step 34, delta -(34+17+8+4) = -63, index 16.
        // Nibble 0x0 at index 16: step 73, delta 9, index falls to 15.
        chip.write_data(0x0F);
        assert!(chip.consume_byte_tick());
        assert_eq!(chip.held_sample >> SAMPLE_SHIFT, 40 - 63 + 9);
        assert_eq!(chip.predictor_index, 15);
    }

    #[test]
    fn nibble_order_decodes_the_low_nibble_first() {
        let mut low_first = Msm6258::new(48_000);
        low_first.left_pan.position = PAN_FADE_SAMPLES;
        low_first.left_pan.enabled = true;
        low_first.write_command(0x02);
        low_first.write_data(0x08);
        assert!(low_first.consume_byte_tick());
        // Low nibble 0x8 subtracts, high nibble 0x0 adds afterwards; the
        // intermediate sample after the low nibble must be negative.
        let samples = &low_first.native_buffer;
        assert!(samples[0] < 0.0, "low nibble decodes first: {samples:?}");
    }

    #[test]
    fn predictor_index_clamps_at_both_ends() {
        let mut chip = Msm6258::new(48_000);
        chip.write_command(0x02);
        for _ in 0..10 {
            chip.write_data(0x00);
            chip.consume_byte_tick();
        }
        assert_eq!(chip.predictor_index, 0);
        for _ in 0..30 {
            chip.write_data(0x77);
            chip.consume_byte_tick();
        }
        assert_eq!(chip.predictor_index, 48);
    }

    #[test]
    fn held_sample_clamps_to_ten_bits() {
        let mut chip = Msm6258::new(48_000);
        chip.write_command(0x02);
        for _ in 0..200 {
            chip.write_data(0x77);
            chip.consume_byte_tick();
        }
        assert_eq!(chip.held_sample, SAMPLE_MAXIMUM);
        for _ in 0..400 {
            chip.write_data(0xFF);
            chip.consume_byte_tick();
        }
        assert_eq!(chip.held_sample, SAMPLE_MINIMUM);
    }

    #[test]
    fn under_run_repeats_the_held_sample_with_decay() {
        let mut chip = Msm6258::new(48_000);
        chip.write_command(0x02);
        chip.write_data(0x77);
        chip.consume_byte_tick();
        let held_before = chip.held_sample;
        chip.native_buffer.clear();
        assert!(chip.consume_byte_tick());
        assert_eq!(chip.held_sample, held_before - 2);
        assert_eq!(
            chip.native_buffer.len(),
            4 * chip.repeat_count().unwrap() as usize
        );
    }

    #[test]
    fn each_sample_repeats_on_the_native_grid() {
        let mut chip = Msm6258::new(48_000);
        chip.set_clock_low(true);
        chip.set_divider(0);
        chip.left_pan.position = PAN_FADE_SAMPLES;
        chip.left_pan.enabled = true;
        chip.write_command(0x02);
        chip.write_data(0x11);
        chip.consume_byte_tick();
        assert_eq!(chip.native_buffer.len(), 64);
        assert_eq!(chip.native_buffer[0], chip.native_buffer[30]);
        assert_ne!(chip.native_buffer[30], chip.native_buffer[32]);
    }

    #[test]
    fn pan_lines_gate_each_output_channel() {
        let mut chip = Msm6258::new(48_000);
        chip.set_divider(2);
        chip.write_command(0x02);
        chip.set_output_enable(true, false);
        for _ in 0..2_000 {
            chip.write_data(0x77);
            chip.consume_byte_tick();
        }
        let mut output = vec![0.0f32; 512];
        chip.generate_samples(1.0, &mut output);
        let left_energy: f32 = output.iter().step_by(2).map(|sample| sample.abs()).sum();
        let right_energy: f32 = output
            .iter()
            .skip(1)
            .step_by(2)
            .map(|sample| sample.abs())
            .sum();
        assert!(left_energy > 0.0);
        assert_eq!(right_energy, 0.0);
    }

    #[test]
    fn generate_samples_limits_pcm_drain_to_output_capacity() {
        let mut chip = Msm6258::new(48_000);
        chip.set_output_enable(true, true);
        chip.native_buffer.resize(10_000, 0.25);

        let mut output = vec![0.0f32; 64];
        let before = chip.native_buffer.len();
        let drain_limit = chip.native_frames_needed(output.len() / 2) * 2;
        chip.generate_samples(1.0, &mut output);

        assert!(before - chip.native_buffer.len() <= drain_limit);
        assert!(!chip.native_buffer.is_empty());
    }

    #[test]
    fn stop_defers_the_release_decay_to_generation_time() {
        let mut chip = Msm6258::new(48_000);
        chip.set_output_enable(true, true);
        chip.write_command(0x02);
        chip.write_data(0x77);
        chip.consume_byte_tick();
        chip.native_buffer.clear();
        chip.write_command(0x01);
        assert!(
            chip.native_buffer.is_empty(),
            "stopping must not queue samples ahead of real time"
        );
        let held_at_stop = chip.held_sample;
        assert_ne!(held_at_stop, 0);

        let mut output = [0.0f32; 64];
        chip.generate_samples(1.0, &mut output);
        assert!(
            chip.held_sample.abs() < held_at_stop.abs(),
            "generation while stopped must decay the held sample"
        );
        for _ in 0..1_000 {
            if chip.held_sample == 0 {
                break;
            }
            output.fill(0.0);
            chip.generate_samples(1.0, &mut output);
        }
        assert_eq!(chip.held_sample, 0, "the release must reach silence");
    }

    #[test]
    fn zero_byte_correction_matches_golden_states() {
        let mut positive = Msm6258::new(48_000);
        positive.write_command(0x02);
        positive.write_data(0x77);
        positive.consume_byte_tick();
        assert_eq!(
            (
                positive.held_sample >> SAMPLE_SHIFT,
                positive.predictor_index
            ),
            (93, 16)
        );
        positive.write_data(0x00);
        positive.consume_byte_tick();
        assert_eq!(
            (
                positive.held_sample >> SAMPLE_SHIFT,
                positive.predictor_index
            ),
            (92, 14)
        );
        positive.write_data(0x88);
        positive.consume_byte_tick();
        assert_eq!(
            (
                positive.held_sample >> SAMPLE_SHIFT,
                positive.predictor_index
            ),
            (79, 12)
        );

        let mut negative = Msm6258::new(48_000);
        negative.write_command(0x02);
        negative.write_data(0xFF);
        negative.consume_byte_tick();
        assert_eq!(
            (
                negative.held_sample >> SAMPLE_SHIFT,
                negative.predictor_index
            ),
            (-93, 16)
        );
        negative.write_data(0x88);
        negative.consume_byte_tick();
        assert_eq!(
            (
                negative.held_sample >> SAMPLE_SHIFT,
                negative.predictor_index
            ),
            (-92, 14)
        );
    }

    #[test]
    fn twelve_bit_signal_uses_the_full_sixteen_bit_mix_scale() {
        let mut chip = Msm6258::new(48_000);
        chip.left_pan.position = PAN_FADE_SAMPLES;
        chip.left_pan.enabled = true;
        chip.held_sample = SAMPLE_MAXIMUM;
        chip.push_native_sample(1);
        assert_eq!(chip.native_buffer[0], 2047.0 / 2048.0);
        chip.native_buffer.clear();
        chip.held_sample = SAMPLE_MINIMUM;
        chip.push_native_sample(1);
        assert_eq!(chip.native_buffer[0], -1.0);
    }

    #[test]
    fn pan_envelope_fades_for_1024_native_samples_and_reverses() {
        let mut envelope = PanEnvelope::disabled();
        envelope.set_enabled(true);
        let gains: Vec<f32> = (0..PAN_FADE_SAMPLES)
            .map(|_| envelope.next_gain())
            .collect();
        assert_eq!(gains[0], 0.0);
        assert!(gains.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(envelope.position, PAN_FADE_SAMPLES);
        assert_eq!(envelope.next_gain(), 1.0);

        for _ in 0..200 {
            envelope.set_enabled(false);
            envelope.next_gain();
        }
        let reversed_at = envelope.position;
        envelope.set_enabled(true);
        envelope.next_gain();
        assert_eq!(envelope.position, reversed_at + 1);
    }

    #[test]
    fn release_samples_use_the_selected_rate() {
        let release_length = |divider| {
            let mut chip = Msm6258::new(48_000);
            chip.set_divider(divider);
            chip.held_sample = 8;
            chip.write_command(0x01);
            assert!(chip.native_buffer.is_empty());
            chip.push_release_samples(1_000);
            chip.native_buffer.len()
        };
        assert_eq!(release_length(0), 8 * 8 * 2);
        assert_eq!(release_length(1), 8 * 6 * 2);
        assert_eq!(release_length(2), 8 * 4 * 2);
    }

    #[test]
    fn release_samples_stay_bounded_by_the_block_need() {
        let mut chip = Msm6258::new(48_000);
        chip.held_sample = 8_000;
        chip.write_command(0x01);
        chip.push_release_samples(16);
        assert!(
            chip.native_buffer.len() < 64,
            "the release must not run ahead of the requested block"
        );
        assert_ne!(chip.held_sample, 0);
    }
}
