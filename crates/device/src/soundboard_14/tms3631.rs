//! TMS3631-RI104 synthesizer LSI (Texas Instruments).
//!
//! Used on the PC-9801-14 Music Generator Board. Produces 8 channels of
//! top-octave-synthesizer (TOS) based square-wave tones. Seven of the eight
//! channels combine four octave-doubled square waves (16', 8', 4', 2' feet)
//! through a 16-entry additive lookup table; CH1 exposes only the 2' output.
//!
//! Reference: Transistor Gijutsu June 1983, pp. 261-270
//! https://electro-music.com/forum/topic-31625.html
//!
//! Frequency generation uses the exact TOS divisors from Table 2; this captures
//! the chip's real ~0.08% deviation from equal temperament,

/// Chip master clock on a PC-9801-14 board: 8 MHz / 4 = 1.9968 MHz.
///
/// The datasheet's ideal calibration of 1.99936 MHz would produce A4 = 440 Hz
/// exactly, but real PC-98 boards use the 8 MHz system clock divided by 4,
/// producing A4 ~ 439.44 Hz (about 0.13% flat).
pub const TMS3631_FCLK: u32 = 1_996_800;

/// Top-octave TOS divisors from datasheet Table 2 (p. 263).
///
/// Indexed by note: `[C, C#, D, D#, E, F, F#, G, G#, A, A#, B]`. At the top
/// octave the output frequency is `FCLK / divisor`. Lower octaves divide the
/// divisor by successive factors of 2.
const TOP_OCTAVE_DIVISORS: [u32; 12] = [
    478, // C   (FCLK/478 = 4182.762 Hz at top octave)
    451, // C#  (FCLK/451 = 4433.171 Hz)
    426, // D   (FCLK/426 = 4693.333 Hz)
    402, // D#  (FCLK/402 = 4973.532 Hz)
    379, // E   (FCLK/379 = 5275.356 Hz)
    358, // F   (FCLK/358 = 5584.804 Hz)
    338, // F#  (FCLK/338 = 5915.266 Hz)
    319, // G   (FCLK/319 = 6267.586 Hz)
    301, // G#  (FCLK/301 = 6642.392 Hz)
    284, // A   (FCLK/284 = 7040.000 Hz; reference A=440 when divided by 16)
    268, // A#  (FCLK/268 = 7460.299 Hz)
    253, // B   (FCLK/253 = 7902.609 Hz)
];

/// Default foot volumes, in `[2', 4', 8', 16']` order.
///
/// Datasheet Figure 13(b) gives the example mix `Va:Vb:Vc:Vd = 8:4:2:1`,
/// with the 16' drawbar (the fundamental) receiving the heaviest weight and
/// the 2' the lightest. The feet-index bit 0 represents the 2' foot (see
/// `PHASE_FREQ_BIT`), so the translated array is `[1, 2, 4, 8]`.
const DEFAULT_FEET: [u8; 4] = [1, 2, 4, 8];

/// Default L/R output gain for centre channels (CH1, CH2).
const DEFAULT_CHANNEL_LEVEL: u8 = 15;

/// Phase accumulator bit representing the 2' square-wave output (8x the
/// fundamental).
const PHASE_FREQ_BIT: u32 = 16;

/// Oversampling factor applied when rendering each output sample to reduce
/// aliasing from the high-harmonic feet.
const PHASE_MUL: u32 = 4;

/// Number of channels on the chip.
pub const CHANNEL_COUNT: usize = 8;

save_state::runtime_state! {
/// Per-channel phase state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelState {
    /// Per-sub-sample phase increment (0 = silent).
    pub increment: u32,
    /// Current 32-bit phase accumulator.
    pub phase: u32,
}}

save_state::runtime_state! {
/// Complete mutable TMS3631 synthesis state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tms3631State {
    channels: [ChannelState; CHANNEL_COUNT],
    enable: u8,
}}

/// Precomputed per-sample-rate tables.
#[derive(Debug, Clone)]
pub struct Tms3631Config {
    /// Per-sub-sample phase increment for each possible 6-bit key value.
    ///
    /// Key format (datasheet Figure 4):
    /// - bits 5:4 = octave (00 = lowest, 11 = highest)
    /// - bits 3:0 = note (0 = silent, 1 = C, 2 = C#, ..., 12 = B)
    ///
    /// Entries with note = 0 or invalid notes are zero (silent).
    pub freq_table: [u32; 64],
    /// Signed 16-entry drawbar-mix table for channels CH2-8.
    pub feet: [i32; 16],
    /// Amplitude of CH1's single 2' foot output.
    pub ch1_amplitude: i32,
    /// Left-channel output weight (centre channels only).
    pub left_level: i32,
    /// Right-channel output weight (centre channels only).
    pub right_level: i32,
}

impl Tms3631Config {
    /// Builds the frequency and feet tables for the given target sample rate.
    pub fn new(sample_rate: u32) -> Self {
        Self {
            freq_table: Self::compute_freq_table(sample_rate),
            feet: Self::compute_feet(&DEFAULT_FEET),
            ch1_amplitude: i32::from(DEFAULT_FEET[0]) << 5,
            left_level: i32::from(DEFAULT_CHANNEL_LEVEL) << 5,
            right_level: i32::from(DEFAULT_CHANNEL_LEVEL) << 5,
        }
    }

    fn compute_freq_table(sample_rate: u32) -> [u32; 64] {
        // The TOS divisors in `TOP_OCTAVE_DIVISORS` produce the chip's
        // *internal* top-octave frequencies (e.g. FCLK/284 = 7040 Hz for A).
        // The TOS output then feeds a 4-bit flip-flop divider chain before
        // reaching the keyboard output pins, adding an extra /16. So the
        // keyboard's "highest octave" (octave bits = 11) uses an effective
        // divisor of `TOP_OCTAVE_DIVISORS[note] * 16`, and each lower
        // octave multiplies by an additional factor of 2.
        //
        // Example: key = octave 11 | note A -> divisor = 284 * 16 = 4544,
        // giving FCLK/4544 ~ 439.44 Hz (A4) at FCLK = 1.9968 MHz.
        const KEYBOARD_OUTPUT_SHIFT: u32 = 4;

        let mut table = [0u32; 64];
        let sample_rate = u64::from(sample_rate);
        let multiplier = u64::from(TMS3631_FCLK) << (PHASE_FREQ_BIT + 1);
        let mul = u64::from(PHASE_MUL);
        for octave in 0..4u32 {
            let shift = KEYBOARD_OUTPUT_SHIFT + (3 - octave);
            for note_index in 0..12u32 {
                let divisor = u64::from(TOP_OCTAVE_DIVISORS[note_index as usize]) << shift;
                let denom = divisor * sample_rate;
                let increment = (multiplier * mul / denom) as u32;
                let key = (octave << 4) | (note_index + 1);
                table[key as usize] = increment;
            }
        }
        table
    }

    fn compute_feet(volumes: &[u8; 4]) -> [i32; 16] {
        let mut feet = [0i32; 16];
        for (i, slot) in feet.iter_mut().enumerate() {
            let mut sum: i32 = 0;
            for (bit, volume) in volumes.iter().enumerate() {
                let sign = if (i >> bit) & 1 == 1 { 1 } else { -1 };
                sum += sign * i32::from(*volume);
            }
            *slot = sum << 5;
        }
        feet
    }
}

/// TMS3631-RI104 synthesis state.
#[derive(Debug, Clone)]
pub struct Tms3631 {
    channels: [ChannelState; CHANNEL_COUNT],
    enable: u8,
    config: Tms3631Config,
}

impl Tms3631 {
    /// Creates a new TMS3631 configured for the target sample rate.
    pub fn new(sample_rate: u32) -> Self {
        Self {
            channels: [ChannelState::default(); CHANNEL_COUNT],
            enable: 0,
            config: Tms3631Config::new(sample_rate),
        }
    }

    /// Captures oscillator phases and the enable mask.
    pub fn capture_state(&self) -> Tms3631State {
        Tms3631State {
            channels: self.channels,
            enable: self.enable,
        }
    }

    /// Restores oscillator phases without rebuilding them from keys.
    pub fn restore_state(&mut self, state: Tms3631State) {
        self.channels = state.channels;
        self.enable = state.enable;
    }

    /// Resets all channel phases and the enable mask.
    pub fn reset(&mut self) {
        self.channels = [ChannelState::default(); CHANNEL_COUNT];
        self.enable = 0;
    }

    /// Loads a 6-bit key value into the given channel.
    ///
    /// Key 0 silences the channel. Bits above the low six are ignored; the
    /// hardware only latches `key & 0x3F`.
    pub fn set_key(&mut self, channel: u8, key: u8) {
        let ch = usize::from(channel & 0x07);
        let increment = self.config.freq_table[usize::from(key & 0x3F)];
        self.channels[ch].increment = increment;
    }

    /// Sets the 8-bit channel-enable mask (port 0x0188 write). Bit `n`
    /// enables channel `n` (hardware CH`n+1`).
    pub fn set_enable(&mut self, mask: u8) {
        self.enable = mask;
    }

    /// Returns the current 8-bit enable mask.
    pub fn enable_mask(&self) -> u8 {
        self.enable
    }

    /// Returns the phase-increment configured for a channel (silent = 0).
    pub fn channel_increment(&self, channel: u8) -> u32 {
        self.channels[usize::from(channel & 0x07)].increment
    }

    /// Renders interleaved stereo samples into `output`, additively.
    ///
    /// Each entry of `output` accumulates the mixed contribution of the
    /// active channels multiplied by `scale`. `output.len()` must be even.
    pub fn render(&mut self, output: &mut [f32], scale: f32) {
        if self.enable == 0 {
            return;
        }
        let frame_count = output.len() / 2;
        let feet = self.config.feet;
        let ch1_amplitude = self.config.ch1_amplitude;
        let left_level = self.config.left_level;
        let right_level = self.config.right_level;

        for frame in 0..frame_count {
            let mut centre: i32 = 0;
            let mut ch1_square: i32 = 0;
            let mut left: i32 = 0;
            let mut right: i32 = 0;

            for ch_index in 0..CHANNEL_COUNT {
                if (self.enable >> ch_index) & 1 == 0 {
                    continue;
                }
                let channel = &mut self.channels[ch_index];
                if channel.increment == 0 {
                    continue;
                }
                for _ in 0..PHASE_MUL {
                    channel.phase = channel.phase.wrapping_add(channel.increment);
                    match ch_index {
                        // CH1: single 2' foot output; bit `PHASE_FREQ_BIT` is
                        // the 50%-duty square wave.
                        0 => {
                            let high = (channel.phase >> PHASE_FREQ_BIT) & 1 == 1;
                            ch1_square += if high { 1 } else { -1 };
                        }
                        // CH2: full drawbar, centre.
                        1 => {
                            let index = (channel.phase >> PHASE_FREQ_BIT) & 0x0F;
                            centre += feet[index as usize];
                        }
                        // CH3-5: full drawbar, LEFT only.
                        2..=4 => {
                            let index = (channel.phase >> PHASE_FREQ_BIT) & 0x0F;
                            left += feet[index as usize];
                        }
                        // CH6-8: full drawbar, RIGHT only.
                        5..=7 => {
                            let index = (channel.phase >> PHASE_FREQ_BIT) & 0x0F;
                            right += feet[index as usize];
                        }
                        _ => unreachable!(),
                    }
                }
            }

            let centre_mix = centre + ch1_square * ch1_amplitude;
            let pcm_left = centre_mix * left_level + left * i32::from(DEFAULT_CHANNEL_LEVEL);
            let pcm_right = centre_mix * right_level + right * i32::from(DEFAULT_CHANNEL_LEVEL);

            output[frame * 2] += pcm_left as f32 * scale;
            output[frame * 2 + 1] += pcm_right as f32 * scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: u32 = 48_000;

    #[test]
    fn key_zero_is_silent() {
        let config = Tms3631Config::new(SAMPLE_RATE);
        assert_eq!(config.freq_table[0], 0);
        for octave in 0..4u8 {
            let key = octave << 4;
            assert_eq!(config.freq_table[key as usize], 0);
        }
    }

    #[test]
    fn highest_octave_a_matches_datasheet_a4() {
        // Key = octave bits 11 << 4 | note A (10) = 0x3A. At the highest
        // keyboard octave, the chip's output for A is FCLK / (284 * 16)
        // ~ 439.44 Hz (A4, slightly flat from 440 Hz due to using the real
        // 1.9968 MHz board clock rather than the datasheet's ideal
        // 1.99936 MHz calibration).
        let config = Tms3631Config::new(SAMPLE_RATE);
        let key = (3u8 << 4) | 10;
        let increment = config.freq_table[key as usize];

        // Per output sample, phase advances by PHASE_MUL * increment. Bit
        // PHASE_FREQ_BIT toggles every 2^PHASE_FREQ_BIT units of phase; at
        // the keyboard output, that bit is the 2' foot (8x fundamental).
        let toggles_per_sample =
            f64::from(PHASE_MUL * increment) / (1u64 << (PHASE_FREQ_BIT + 1)) as f64;
        let f_fundamental = toggles_per_sample * f64::from(SAMPLE_RATE) / 16.0;
        let expected = f64::from(TMS3631_FCLK) / 284.0 / 16.0;
        // 32-bit integer truncation on the increment introduces a small
        // quantisation error; a 0.1 Hz tolerance is within ~0.03% of the
        // target and comfortably tighter than the chip's datasheet-stated
        // ~0.08% equal-temperament deviation.
        let diff = (f_fundamental - expected).abs();
        assert!(
            diff < 0.1,
            "expected ~{expected} Hz, got {f_fundamental} Hz"
        );
    }

    #[test]
    fn lower_octaves_divide_by_two() {
        let config = Tms3631Config::new(SAMPLE_RATE);
        for note in 1..=12u8 {
            let top = u64::from(config.freq_table[usize::from((3u8 << 4) | note)]);
            assert_eq!(
                u64::from(config.freq_table[usize::from((2u8 << 4) | note)]),
                top / 2
            );
            assert_eq!(
                u64::from(config.freq_table[usize::from((1u8 << 4) | note)]),
                top / 4
            );
            assert_eq!(u64::from(config.freq_table[usize::from(note)]), top / 8);
        }
    }

    #[test]
    fn feet_mix_matches_datasheet_example() {
        let feet = Tms3631Config::compute_feet(&[1, 2, 4, 8]);
        // All bits zero -> -1 -2 -4 -8 = -15 (scaled by 32 -> -480).
        assert_eq!(feet[0], -15 << 5);
        // All bits set -> +1 +2 +4 +8 = +15.
        assert_eq!(feet[15], 15 << 5);
        // i=8 (only 16' bit set, all others negative): -1 -2 -4 +8 = +1.
        assert_eq!(feet[8], 1 << 5);
    }

    #[test]
    fn ch1_swings_both_polarities() {
        let mut chip = Tms3631::new(SAMPLE_RATE);
        let key = (3u8 << 4) | 10;
        chip.set_key(0, key);
        chip.set_enable(0x01);
        let mut buffer = vec![0.0f32; 2 * 2000];
        chip.render(&mut buffer, 1.0 / 32768.0);

        let mut seen_pos = false;
        let mut seen_neg = false;
        for frame in 40..buffer.len() / 2 {
            if buffer[frame * 2] > 1e-9 {
                seen_pos = true;
            } else if buffer[frame * 2] < -1e-9 {
                seen_neg = true;
            }
        }
        assert!(seen_pos && seen_neg);
    }

    #[test]
    fn ch2_full_drawbar_produces_multiple_levels() {
        let mut chip = Tms3631::new(SAMPLE_RATE);
        let key = (3u8 << 4) | 10;
        chip.set_key(1, key);
        chip.set_enable(0x02);
        let mut buffer = vec![0.0f32; 2 * 2000];
        chip.render(&mut buffer, 1.0 / 32768.0);

        let mut seen = std::collections::BTreeSet::new();
        for frame in 40..buffer.len() / 2 {
            let bucket = (buffer[frame * 2] * 1e6).round() as i64;
            seen.insert(bucket);
        }
        assert!(
            seen.len() > 3,
            "CH2 drawbar should produce more than three levels, saw {seen:?}"
        );
    }

    #[test]
    fn enable_mask_gates_channels() {
        let mut chip = Tms3631::new(SAMPLE_RATE);
        chip.set_key(3, (3u8 << 4) | 10);
        chip.set_enable(0x00);
        let mut buffer = vec![0.0f32; 2 * 480];
        chip.render(&mut buffer, 1.0);
        assert!(buffer.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn channel_increment_follows_key() {
        let mut chip = Tms3631::new(SAMPLE_RATE);
        chip.set_key(4, (3u8 << 4) | 10);
        assert_ne!(chip.channel_increment(4), 0);
        chip.set_key(4, 0);
        assert_eq!(chip.channel_increment(4), 0);
    }
}
