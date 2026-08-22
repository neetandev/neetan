//! uPD7752 LSI voice synthesizer.
//!
//! Fitted from the PC-6001mkII onward and mapped at four consecutive I/O ports.
//! The chip is driven in external-message mode: the host uploads seven-byte
//! parameter frames that describe a fifth-order lattice IIR vocal-tract filter
//! excited by a pitched impulse train and noise. Each frame is synthesized at a
//! 10 kHz internal rate, resampled to the machine output rate, and queued for
//! the audio mixer to drain.
//!
//! Based on the work of cisc's "PD7752 flavour voice engine" (2004) which is
//! licensed under the zlib license.

use std::collections::VecDeque;

/// Status bit 7: BSY - the chip is synthesizing.
const STATUS_BUSY: u8 = 0x80;
/// Status bit 6: REQ - the chip wants the next parameter byte.
const STATUS_REQUEST: u8 = 0x40;
/// Status bit 5: INT/EXT - parameters are uploaded from the host (external).
const STATUS_EXTERNAL: u8 = 0x20;
/// Status bit 4: ERR - an invalid command was issued.
const STATUS_ERROR: u8 = 0x10;

/// Command selecting external-message (host upload) mode.
const COMMAND_EXTERNAL_MESSAGE: u8 = 0xFE;
/// Command stopping synthesis.
const COMMAND_STOP: u8 = 0xFF;

/// Bytes per uploaded parameter frame.
const FRAME_PARAM_COUNT: usize = 7;

/// Internal synthesis rate of the chip.
const SYNTHESIS_RATE_HZ: u32 = 10_000;

/// Largest number of queued output samples (about a second at 48 kHz), bounding
/// memory if the host uploads an utterance faster than the mixer drains it.
const MAX_QUEUED_SAMPLES: usize = 48_000;

/// Per-sample amplitude curve (16 steps).
const AMP_TABLE: [i32; 16] = [0, 1, 1, 2, 3, 4, 5, 7, 9, 13, 17, 23, 31, 42, 56, 75];

// Polynomial coefficients (ascending power) for the IIR1 and IIR2 tap curves, one polynomial
// per index region: IIR1 spans three regions of the 7-bit frequency index, IIR2 two regions of
// the 6-bit bandwidth index (mirrored across index bit 4). Each polynomial takes the
// region-local index and yields the coefficient for that index.
const IIR1_POLY_REGION_A: [f64; 7] = [
    1.1427853896e4,
    -3.0088657295e1,
    6.9599400365e-1,
    -4.4758410897e-1,
    2.8002578942e-2,
    -1.1634122946e-3,
    1.6075801604e-5,
];
const IIR1_POLY_REGION_B: [f64; 6] = [
    -1.1238937699e4,
    3.3723713072e1,
    8.5831859088e0,
    -6.0085389177e-1,
    4.3365348085e-2,
    -7.7064321401e-4,
];
const IIR1_POLY_REGION_C: [f64; 9] = [
    1.1586802863e4,
    -2.5230190730e1,
    7.1928542621e0,
    -1.0654315388e0,
    7.5635833804e-2,
    -3.0481641418e-3,
    6.8972854733e-5,
    -8.3499775402e-7,
    4.1387532205e-9,
];
const IIR2_POLY_REGION_LOW: [f64; 6] = [
    8.1911798246e3,
    -6.5544308166e1,
    -2.1230342543e1,
    2.7102261454e0,
    -4.6743272210e-1,
    1.8054397873e-2,
];
const IIR2_POLY_REGION_HIGH: [f64; 7] = [
    -6.7393184985e3,
    2.4094539284e2,
    1.9035031920e1,
    5.7453634973e0,
    -7.4321432576e-1,
    7.7670774918e-2,
    -2.9691262498e-3,
];

/// Evaluates a polynomial (ascending-power coefficients) at `x` via Horner's method.
const fn evaluate_polynomial(coefficients: &[f64], x: f64) -> f64 {
    let mut accumulator = 0.0;
    let mut index = coefficients.len();
    while index > 0 {
        index -= 1;
        accumulator = accumulator * x + coefficients[index];
    }
    accumulator
}

/// Rounds to the nearest integer (ties away from zero).
const fn round_to_i32(value: f64) -> i32 {
    if value >= 0.0 {
        (value + 0.5) as i32
    } else {
        (value - 0.5) as i32
    }
}

/// Builds the IIR1 tap curve over the full frequency index.
const fn iir1_table() -> [i32; 128] {
    let mut table = [0i32; 128];
    let mut index = 0;
    while index < 128 {
        let value = if index < 32 {
            evaluate_polynomial(&IIR1_POLY_REGION_A, index as f64)
        } else if index < 64 {
            evaluate_polynomial(&IIR1_POLY_REGION_B, (index - 32) as f64)
        } else {
            evaluate_polynomial(&IIR1_POLY_REGION_C, (index - 64) as f64)
        };
        table[index] = round_to_i32(value);
        index += 1;
    }
    table
}

/// Builds the IIR2 tap curve over the bandwidth index; index bit 4 mirrors the magnitude, so
/// the low four bits select the coefficient.
const fn iir2_table() -> [i32; 64] {
    let mut table = [0i32; 64];
    let mut index = 0;
    while index < 64 {
        let local = (index & 0x0F) as f64;
        let value = if index < 32 {
            evaluate_polynomial(&IIR2_POLY_REGION_LOW, local)
        } else {
            evaluate_polynomial(&IIR2_POLY_REGION_HIGH, local)
        };
        table[index] = round_to_i32(value);
        index += 1;
    }
    table
}

/// `y[n-1]` feedback tap of each formant resonator. Indexed by the frequency parameter it sets
/// the formant centre frequency (the cosine of the pole angle); indexed by `2*bandwidth+1` it
/// supplies the bandwidth-dependent gain.
const IIR1: [i32; 128] = iir1_table();
/// `y[n-2]` feedback tap of each formant resonator: the squared pole radius that sets the
/// resonance bandwidth, indexed by the bandwidth parameter.
const IIR2: [i32; 64] = iir2_table();

/// Sample count per synthesized frame, indexed by the mode register (frame
/// period bit and the two synthesis-speed bits).
const FRAME_SIZE: [usize; 8] = [100, 120, 80, 100, 200, 240, 160, 200];

/// Default filter frequency coefficients at the start of an utterance.
const DEFAULT_F: [i32; 5] = [126, 64, 121, 111, 96];
/// Default filter bandwidth coefficients at the start of an utterance.
const DEFAULT_B: [i32; 5] = [9, 4, 9, 9, 11];
/// Default pitch period at the start of an utterance.
const DEFAULT_PITCH: i32 = 30;

/// Per-stage output clamp applied inside the filter cascade.
const STAGE_CLAMP: i32 = 8192;

/// Converts an integer to 16.16 fixed point.
fn to_fixed(value: i32) -> i32 {
    value << 16
}

/// Converts a 16.16 fixed-point value back to an integer (sign-preserving).
fn from_fixed(value: i32) -> i32 {
    value >> 16
}

save_state::runtime_state! {
/// uPD7752 voice synthesizer with audio output.
#[derive(Clone)]
pub struct Upd7752 {
    status: u8,
    /// Last byte written to the mode port (read back on the mode port).
    mode_register: u8,
    /// Last byte written to the command port (read back on the command port).
    command_register: u8,
    /// Frame length in 10 kHz samples selected by the mode register.
    frame_size: usize,

    /// Filter frequency coefficients (16.16 fixed point).
    coef_f: [i32; 5],
    /// Filter bandwidth coefficients (16.16 fixed point).
    coef_b: [i32; 5],
    /// Amplitude (16.16 fixed point).
    coef_amp: i32,
    /// Pitch period (16.16 fixed point).
    coef_pitch: i32,
    /// Per-stage filter history.
    history: [[i32; 2]; 5],
    /// Samples since the last excitation impulse.
    pitch_count: i32,

    /// Current parameter frame being assembled.
    param_buf: [u8; FRAME_PARAM_COUNT],
    /// Number of parameter bytes received for the current frame.
    param_index: usize,
    /// Remaining repeat count for the active frame.
    repeat_count: u32,
    /// A frame was synthesized and the chip is between frames: it will request
    /// its next parameter frame once that frame has played. The host paces the
    /// re-request via [`Upd7752::arm_request`].
    awaiting_request: bool,

    /// Deterministic noise generator state.
    rng: u32,

    /// Output sample rate the synthesized frames are resampled to.
    sample_rate: u32,
    /// Queued mono output samples, normalized to roughly [-1.0, 1.0].
    output: VecDeque<f32>,
}}

impl Upd7752 {
    /// Creates an idle synthesizer that resamples to `sample_rate`.
    pub fn new(sample_rate: u32) -> Self {
        let mut chip = Self {
            status: 0,
            mode_register: 0,
            command_register: 0,
            frame_size: FRAME_SIZE[0],
            coef_f: [0; 5],
            coef_b: [0; 5],
            coef_amp: 0,
            coef_pitch: to_fixed(DEFAULT_PITCH),
            history: [[0; 2]; 5],
            pitch_count: 0,
            param_buf: [0; FRAME_PARAM_COUNT],
            param_index: 0,
            repeat_count: 0,
            awaiting_request: false,
            rng: 0x1357_9BDF,
            sample_rate,
            output: VecDeque::new(),
        };
        chip.reset_voice_state();
        chip
    }

    /// Captures synthesis, parser, filter, and queued audio state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores synthesis, parser, filter, and queued audio state.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        if state.sample_rate != self.sample_rate
            || state.param_index > FRAME_PARAM_COUNT
            || !FRAME_SIZE.contains(&state.frame_size)
        {
            return Err(save_state::StateValidationError::new(
                "uPD7752 state is invalid",
            ));
        }
        *self = state;
        Ok(())
    }

    /// Reads one of the four chip registers (`offset` is the low two port bits).
    pub fn read(&self, offset: u8) -> u8 {
        match offset & 3 {
            0 => self.status,
            2 => self.mode_register,
            3 => self.command_register,
            _ => 0xFF,
        }
    }

    /// Writes one of the four chip registers (`offset` is the low two port bits).
    pub fn write(&mut self, offset: u8, value: u8) {
        match offset & 3 {
            0 => self.write_parameter(value),
            2 => self.set_mode(value),
            3 => self.command(value),
            _ => {}
        }
    }

    /// Adds the queued voice output to a stereo-interleaved buffer, draining one
    /// queued sample per output frame. `level` scales the contribution.
    pub fn mix_into(&mut self, output: &mut [f32], level: f32) {
        for frame in output.as_chunks_mut::<2>().0 {
            let Some(sample) = self.output.pop_front() else {
                break;
            };
            let value = sample * level;
            frame[0] += value;
            frame[1] += value;
        }
    }

    /// Whether the chip is actively synthesizing speech.
    pub fn is_busy(&self) -> bool {
        self.status & STATUS_BUSY != 0
    }

    /// Whether the chip is asserting its data-request interrupt line: it is busy
    /// in external-message mode and waiting for the next parameter byte. The host
    /// wires this to the voice interrupt so BASIC can feed frames in the
    /// background instead of blocking.
    pub fn wants_data(&self) -> bool {
        const REQUESTING: u8 = STATUS_BUSY | STATUS_REQUEST;
        self.status & REQUESTING == REQUESTING
    }

    /// The number of 10 kHz samples the frame just synthesized plays for, if the
    /// chip is between frames and has not yet requested the next one. The host
    /// converts this to a delay and calls [`Upd7752::arm_request`] afterwards so
    /// the feed is paced to real playback.
    pub fn pending_request_samples(&self) -> Option<usize> {
        self.awaiting_request.then_some(self.frame_size)
    }

    /// Re-asserts the data-request line for the next frame once the previous one
    /// has played, provided the chip is still synthesizing.
    pub fn arm_request(&mut self) {
        if self.awaiting_request && self.status & STATUS_BUSY != 0 {
            self.status |= STATUS_REQUEST;
        }
        self.awaiting_request = false;
    }

    fn set_mode(&mut self, value: u8) {
        self.mode_register = value;
        self.frame_size = FRAME_SIZE[(value & 7) as usize];
        self.reset_voice_state();
        self.status = 0;
    }

    fn command(&mut self, value: u8) {
        self.command_register = value;
        self.abort_voice();
        match value {
            COMMAND_EXTERNAL_MESSAGE => {
                self.status = STATUS_BUSY | STATUS_EXTERNAL | STATUS_REQUEST;
                self.param_index = 0;
                self.repeat_count = 0;
            }
            COMMAND_STOP => {}
            // Internal-voice commands index a ROM table we do not emulate.
            0x00..=0x04 => {}
            _ => self.status = STATUS_ERROR,
        }
    }

    fn write_parameter(&mut self, value: u8) {
        if self.status & (STATUS_BUSY | STATUS_REQUEST) != STATUS_BUSY | STATUS_REQUEST {
            return;
        }

        let collecting = self.repeat_count == 0 || self.param_index > 0;
        let ready = if collecting {
            if self.param_index == 0 {
                self.repeat_count = u32::from(value >> 3);
            }
            self.param_buf[self.param_index] = value;
            self.param_index += 1;
            if self.param_index == FRAME_PARAM_COUNT {
                self.status &= !STATUS_REQUEST;
                self.param_index = 0;
                self.repeat_count = self.repeat_count.saturating_sub(1);
                true
            } else {
                // A leading zero repeat count is the end-of-speech marker.
                self.param_buf[0] >> 3 == 0
            }
        } else {
            // Repeat frame: only the amplitude/pitch byte is refreshed.
            self.param_buf[1..6].fill(0);
            self.param_buf[6] = value;
            self.status &= !STATUS_REQUEST;
            self.param_index = 0;
            self.repeat_count = self.repeat_count.saturating_sub(1);
            true
        };

        if ready {
            self.on_frame_ready();
        }
    }

    fn on_frame_ready(&mut self) {
        if self.param_buf[0] >> 3 == 0 {
            // End-of-speech marker: stop synthesis but stay in external mode so
            // the host can poll BSY going low.
            self.abort_voice();
            return;
        }
        self.synthesize_frame();
        // The request line stays low while this frame plays; the host re-arms it
        // through arm_request once the playback delay elapses.
        self.awaiting_request = true;
    }

    fn abort_voice(&mut self) {
        self.status &= !STATUS_BUSY;
        self.param_index = 0;
        self.repeat_count = 0;
        self.awaiting_request = false;
    }

    fn reset_voice_state(&mut self) {
        for index in 0..5 {
            self.history[index] = [0, 0];
            self.coef_f[index] = to_fixed(DEFAULT_F[index]);
            self.coef_b[index] = to_fixed(DEFAULT_B[index]);
        }
        self.pitch_count = 0;
        self.coef_amp = 0;
        self.coef_pitch = to_fixed(DEFAULT_PITCH);
    }

    /// Deterministic single-bit noise source.
    fn next_noise_bit(&mut self) -> bool {
        // xorshift32.
        let mut state = self.rng;
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        self.rng = state;
        state & 1 != 0
    }

    fn synthesize_frame(&mut self) {
        let param = self.param_buf;
        let quantize_shift = i32::from(param[0] & 4 != 0);

        // Expand the next frame's filter coefficients.
        let mut next_f = [0i32; 5];
        let mut next_b = [0i32; 5];
        for index in 0..5 {
            let mut frequency = ((param[index + 1] >> 3) & 31) as i32;
            if frequency & 16 != 0 {
                frequency -= 32;
            }
            next_f[index] = self.coef_f[index] + to_fixed(frequency << quantize_shift);

            let mut bandwidth = (param[index + 1] & 7) as i32;
            if bandwidth & 4 != 0 {
                bandwidth -= 8;
            }
            next_b[index] = self.coef_b[index] + to_fixed(bandwidth << quantize_shift);
        }

        let next_amp = to_fixed(((param[6] >> 4) & 15) as i32);
        let mut pitch_delta = (param[6] & 7) as i32;
        if pitch_delta & 4 != 0 {
            pitch_delta -= 8;
        }
        let next_pitch = self.coef_pitch + to_fixed(pitch_delta);

        // Linear interpolation increments across the frame.
        let frame_size = self.frame_size as i32;
        let incr_amp = (next_amp - self.coef_amp) / frame_size;
        let incr_pitch = (next_pitch - self.coef_pitch) / frame_size;
        let mut incr_f = [0i32; 5];
        let mut incr_b = [0i32; 5];
        for index in 0..5 {
            incr_f[index] = (next_f[index] - self.coef_f[index]) / frame_size;
            incr_b[index] = (next_b[index] - self.coef_b[index]) / frame_size;
        }

        // Excitation: bit 0 selects voiced (impulse), bit 1 unvoiced (noise).
        let mut excitation = if param[0] & 1 != 0 { 1 } else { 2 };
        if param[6] & 4 != 0 {
            excitation |= 3;
        }

        let mut frame = vec![0i32; self.frame_size];
        for slot in frame.iter_mut() {
            let mut output = 0;

            // Pitched impulse.
            let period = from_fixed(self.coef_pitch);
            if self.pitch_count > if period > 0 { period } else { 128 } {
                if excitation & 1 != 0 {
                    output = AMP_TABLE[self.amp_index()] * 16 - 1;
                }
                self.pitch_count = 0;
            }
            self.pitch_count += 1;

            // Noise.
            if excitation & 2 != 0 && self.next_noise_bit() {
                output += AMP_TABLE[self.amp_index()] * 4 - 1;
            }

            // Fifth-order lattice cascade.
            for stage in 0..5 {
                let frequency_index = (from_fixed(self.coef_f[stage]) & 0x7F) as usize;
                let bandwidth_index = (from_fixed(self.coef_b[stage]) & 0x3F) as usize;
                let bandwidth_index_2 = ((from_fixed(self.coef_b[stage]) * 2 + 1) & 0x7F) as usize;

                let term = self.history[stage][0] * IIR1[frequency_index] / STAGE_CLAMP;
                output += term * IIR1[bandwidth_index_2] / STAGE_CLAMP;
                output -= self.history[stage][1] * IIR2[bandwidth_index] / STAGE_CLAMP;
                output = output.clamp(-STAGE_CLAMP, STAGE_CLAMP - 1);

                self.history[stage][1] = self.history[stage][0];
                self.history[stage][0] = output;
            }

            *slot = output;

            self.coef_amp += incr_amp;
            self.coef_pitch += incr_pitch;
            for stage in 0..5 {
                self.coef_b[stage] += incr_b[stage];
                self.coef_f[stage] += incr_f[stage];
            }
        }

        // Carry the interpolated coefficients into the next frame.
        self.coef_f = next_f;
        self.coef_b = next_b;
        self.coef_amp = next_amp;
        self.coef_pitch = next_pitch;

        self.queue_resampled(&frame);
    }

    /// Clamps the interpolated amplitude into the amplitude-table range.
    fn amp_index(&self) -> usize {
        from_fixed(self.coef_amp).clamp(0, 15) as usize
    }

    /// Resamples a 10 kHz frame to the output rate and queues it.
    fn queue_resampled(&mut self, frame: &[i32]) {
        if self.sample_rate == 0 || frame.is_empty() {
            return;
        }
        let out_count = frame.len() * self.sample_rate as usize / SYNTHESIS_RATE_HZ as usize;
        for index in 0..out_count {
            if self.output.len() >= MAX_QUEUED_SAMPLES {
                break;
            }
            let source = index * frame.len() / out_count;
            self.output
                .push_back(frame[source] as f32 / STAGE_CLAMP as f32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: u32 = 48_000;

    /// Drives an external utterance of `frames` voiced frames, returning the chip.
    /// The chip drops its request line between frames and the host re-arms it
    /// once each frame has played, so the helper mirrors that pacing.
    fn voiced_utterance(frames: usize) -> Upd7752 {
        let mut voice = Upd7752::new(SAMPLE_RATE);
        voice.write(2, 0x00); // mode: 10 ms/frame, normal speed
        voice.write(3, COMMAND_EXTERNAL_MESSAGE);
        for _ in 0..frames {
            // Repeat count 1, voiced, mid-range coefficients and amplitude.
            for byte in [0x08, 0x55, 0x55, 0x55, 0x55, 0x55, 0xF4] {
                voice.write(0, byte);
            }
            voice.arm_request();
        }
        voice
    }

    #[test]
    fn idle_status_is_clear_so_polling_proceeds() {
        let voice = Upd7752::new(SAMPLE_RATE);
        assert_eq!(voice.read(0), 0);
    }

    #[test]
    fn mode_register_selects_the_frame_size() {
        let mut voice = Upd7752::new(SAMPLE_RATE);
        voice.write(2, 0x04); // 20 ms/frame, normal speed
        assert_eq!(voice.frame_size, 200);
        assert_eq!(voice.read(2), 0x04);
    }

    #[test]
    fn external_message_command_requests_data() {
        let mut voice = Upd7752::new(SAMPLE_RATE);
        voice.write(3, COMMAND_EXTERNAL_MESSAGE);
        assert_eq!(voice.read(0) & STATUS_EXTERNAL, STATUS_EXTERNAL);
        assert_eq!(voice.read(0) & STATUS_REQUEST, STATUS_REQUEST);
        assert_eq!(voice.read(0) & STATUS_BUSY, STATUS_BUSY);
    }

    #[test]
    fn data_is_ignored_until_external_mode_is_selected() {
        let mut voice = Upd7752::new(SAMPLE_RATE);
        voice.write(0, 0x42);
        assert!(voice.output.is_empty());
    }

    #[test]
    fn a_completed_frame_produces_output_and_paces_the_next_request() {
        let mut voice = Upd7752::new(SAMPLE_RATE);
        voice.write(2, 0x00);
        voice.write(3, COMMAND_EXTERNAL_MESSAGE);
        for byte in [0x08, 0x55, 0x55, 0x55, 0x55, 0x55, 0xF4] {
            voice.write(0, byte);
        }
        // One 10 ms frame at 48 kHz yields 480 queued samples.
        assert_eq!(voice.output.len(), 480);
        assert!(voice.is_busy());
        // The request line stays low until the frame has played; the chip
        // reports the pending frame's length so the host can pace the re-request.
        assert_eq!(voice.read(0) & STATUS_REQUEST, 0);
        assert_eq!(voice.pending_request_samples(), Some(100));
        voice.arm_request();
        assert_eq!(voice.read(0) & STATUS_REQUEST, STATUS_REQUEST);
        assert_eq!(voice.pending_request_samples(), None);
        // At least one sample is non-zero, i.e. audio was actually synthesized.
        assert!(voice.output.iter().any(|&s| s != 0.0));
    }

    #[test]
    fn synthesis_is_deterministic() {
        let first = voiced_utterance(2);
        let second = voiced_utterance(2);
        assert_eq!(
            first.output.into_iter().collect::<Vec<_>>(),
            second.output.into_iter().collect::<Vec<_>>()
        );
    }

    #[test]
    fn terminator_frame_clears_busy() {
        let mut voice = voiced_utterance(1);
        assert!(voice.is_busy());
        // A frame whose repeat field is zero ends the utterance.
        for byte in [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00] {
            voice.write(0, byte);
        }
        assert!(!voice.is_busy());
        assert_eq!(voice.read(0) & STATUS_EXTERNAL, STATUS_EXTERNAL);
    }

    #[test]
    fn mix_into_drains_queue_onto_both_channels() {
        let mut voice = voiced_utterance(1);
        let queued = voice.output.len();
        let mut buffer = vec![0.0f32; queued * 2];
        voice.mix_into(&mut buffer, 1.0);
        assert!(voice.output.is_empty(), "queue drained");
        // Left and right channels carry the same mono sample.
        for frame in buffer.as_chunks::<2>().0 {
            assert_eq!(frame[0], frame[1]);
        }
        assert!(buffer.iter().any(|&s| s != 0.0));
    }
}
