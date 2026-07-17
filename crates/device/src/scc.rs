//! Konami 051649 SCC sound generator.

/// Number of SCC sound channels.
const CHANNEL_COUNT: usize = 5;
/// Number of signed samples in one waveform.
const WAVEFORM_LENGTH: usize = 32;
/// Maximum frequency register value.
const FREQUENCY_MASK: u16 = 0x0FFF;
/// Frequencies at or below this value halt a channel.
const HALTED_FREQUENCY: u16 = 8;
/// Output divisor used by the reference device.
const OUTPUT_DIVISOR: f32 = 1024.0;
/// Standard SCC specialization selector for [`Scc`].
pub const SCC_VARIANT_STANDARD: u8 = 0;
/// SCC+ specialization selector for [`Scc`].
pub const SCC_VARIANT_PLUS: u8 = 1;

#[derive(Debug, Clone)]
struct Channel {
    counter: u8,
    clock: u16,
    frequency: u16,
    volume: u8,
    sample: i16,
    key: bool,
    waveform: [i8; WAVEFORM_LENGTH],
}

impl Channel {
    const fn new() -> Self {
        Self {
            counter: 0,
            clock: 0,
            frequency: 0,
            volume: 0x0F,
            sample: 0,
            key: false,
            waveform: [0; WAVEFORM_LENGTH],
        }
    }

    fn reset(&mut self) {
        self.counter = 0;
        self.clock = 0;
        self.frequency = 0;
        self.volume = 0x0F;
        self.sample = 0;
        self.key = false;
    }

    fn clock(&mut self) -> i16 {
        if self.frequency > HALTED_FREQUENCY {
            self.clock = self.clock.wrapping_add(1);
            if self.clock > self.frequency {
                self.counter = self.counter.wrapping_add(1) & 0x1F;
                self.clock = 0;
            }
            if self.clock == 0 {
                let sample = if self.key {
                    i16::from(self.waveform[usize::from(self.counter)])
                } else {
                    0
                };
                self.sample = sample * i16::from(self.volume);
            }
        }
        self.sample >> 4
    }
}

/// Konami SCC family sound generator specialized for one chip variant.
#[derive(Debug, Clone)]
pub struct Scc<const VARIANT: u8> {
    channels: [Channel; CHANNEL_COUNT],
    test: u8,
    cpu_clock_hz: u32,
    sample_rate: u32,
    frame_start_cycle: u64,
    pending_samples: Vec<i16>,
    sample_remainder: f64,
    clock_accumulator: f64,
}

save_state::runtime_state! {
/// Mutable state of one SCC sound channel.
#[derive(Debug, Clone)]
pub struct SccChannelState {
    counter: u8,
    clock: u16,
    frequency: u16,
    volume: u8,
    sample: i16,
    key: bool,
    waveform: [i8; WAVEFORM_LENGTH],
}}

save_state::runtime_state! {
/// Complete Konami SCC family sound state.
#[derive(Debug, Clone)]
pub struct SccState {
    channels: [crate::scc::SccChannelState; CHANNEL_COUNT],
    test: u8,
    cpu_clock_hz: u32,
    sample_rate: u32,
    frame_start_cycle: u64,
    pending_samples: Vec<i16>,
    sample_remainder: f64,
    clock_accumulator: f64,
}}

/// Standard Konami 051649 SCC sound generator.
pub type StandardScc = Scc<SCC_VARIANT_STANDARD>;

/// Konami 052539 SCC+ sound generator.
pub type SccPlus = Scc<SCC_VARIANT_PLUS>;

impl<const VARIANT: u8> Scc<VARIANT> {
    /// Creates a reset SCC family device.
    pub const fn new() -> Self {
        Self {
            channels: [
                Channel::new(),
                Channel::new(),
                Channel::new(),
                Channel::new(),
                Channel::new(),
            ],
            test: 0,
            cpu_clock_hz: 0,
            sample_rate: 0,
            frame_start_cycle: 0,
            pending_samples: Vec::new(),
            sample_remainder: 0.0,
            clock_accumulator: 0.0,
        }
    }

    /// Resets registers and channel phases without clearing waveform RAM.
    pub fn reset(&mut self) {
        for channel in &mut self.channels {
            channel.reset();
        }
        self.test = 0;
        self.frame_start_cycle = 0;
        self.pending_samples.clear();
        self.sample_remainder = 0.0;
        self.clock_accumulator = 0.0;
    }

    /// Configures the CPU and host audio clocks.
    pub fn configure_audio(&mut self, cpu_clock_hz: u32, sample_rate: u32) {
        self.cpu_clock_hz = cpu_clock_hz;
        self.sample_rate = sample_rate;
    }

    /// Aligns the audio frame origin after runtime device insertion.
    pub fn synchronize(&mut self, current_cycle: u64) {
        self.frame_start_cycle = current_cycle;
        self.pending_samples.clear();
        self.sample_remainder = 0.0;
        self.clock_accumulator = 0.0;
    }

    /// Captures registers, wave RAM, channel phases, and audio timing.
    pub fn capture_state(&self) -> SccState {
        SccState {
            channels: self.channels.each_ref().map(|channel| SccChannelState {
                counter: channel.counter,
                clock: channel.clock,
                frequency: channel.frequency,
                volume: channel.volume,
                sample: channel.sample,
                key: channel.key,
                waveform: channel.waveform,
            }),
            test: self.test,
            cpu_clock_hz: self.cpu_clock_hz,
            sample_rate: self.sample_rate,
            frame_start_cycle: self.frame_start_cycle,
            pending_samples: self.pending_samples.clone(),
            sample_remainder: self.sample_remainder,
            clock_accumulator: self.clock_accumulator,
        }
    }

    /// Restores registers, wave RAM, channel phases, and audio timing.
    pub fn restore_state(
        &mut self,
        state: SccState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.cpu_clock_hz != self.cpu_clock_hz
            || state.sample_rate != self.sample_rate
            || !state.sample_remainder.is_finite()
            || !state.clock_accumulator.is_finite()
            || !(0.0..1.0).contains(&state.sample_remainder)
            || !(0.0..1.0).contains(&state.clock_accumulator)
            || state.channels.iter().any(|channel| {
                channel.counter >= WAVEFORM_LENGTH as u8
                    || channel.frequency > FREQUENCY_MASK
                    || channel.volume > 0x0F
            })
        {
            return Err(save_state::StateValidationError::new(
                "SCC state is invalid",
            ));
        }
        for (channel, channel_state) in self.channels.iter_mut().zip(state.channels) {
            channel.counter = channel_state.counter;
            channel.clock = channel_state.clock;
            channel.frequency = channel_state.frequency;
            channel.volume = channel_state.volume;
            channel.sample = channel_state.sample;
            channel.key = channel_state.key;
            channel.waveform = channel_state.waveform;
        }
        self.test = state.test;
        self.frame_start_cycle = state.frame_start_cycle;
        self.pending_samples = state.pending_samples;
        self.sample_remainder = state.sample_remainder;
        self.clock_accumulator = state.clock_accumulator;
        Ok(())
    }

    /// Reads one SCC register, returning `None` for write-only registers.
    pub fn read(&self, address: u8) -> Option<u8> {
        let waveform_end = if VARIANT == SCC_VARIANT_PLUS {
            0x9F
        } else {
            0x7F
        };
        match address {
            0x00..=0x9F if address <= waveform_end => {
                let channel = usize::from(address >> 5);
                let counter = if VARIANT == SCC_VARIANT_PLUS {
                    if self.test & 0x40 != 0 {
                        self.channels[channel].counter
                    } else {
                        0
                    }
                } else if self.test & 0xC0 == 0 {
                    0
                } else if address >= 0x60 && self.test & 0xC0 != 0xC0 {
                    self.channels[3 + usize::from((self.test >> 6) & 1)].counter
                } else if self.test & 0x40 != 0 {
                    self.channels[channel].counter
                } else {
                    0
                };
                let index = usize::from(address.wrapping_add(counter) & 0x1F);
                Some(self.channels[channel].waveform[index] as u8)
            }
            0xC0..=0xDF if VARIANT == SCC_VARIANT_PLUS => Some(0xFF),
            0xE0..=0xFF if VARIANT == SCC_VARIANT_STANDARD => Some(0xFF),
            _ => None,
        }
    }

    /// Writes one SCC register and reports whether it was decoded.
    pub fn write(&mut self, address: u8, value: u8) -> bool {
        match address {
            0x00..=0x7F if VARIANT == SCC_VARIANT_STANDARD => self.write_waveform(address, value),
            0x00..=0x9F if VARIANT == SCC_VARIANT_PLUS => self.write_waveform(address, value),
            0x80..=0x89 | 0x90..=0x99 if VARIANT == SCC_VARIANT_STANDARD => {
                self.write_frequency((address & 0x0F) as usize, value)
            }
            0x8A..=0x8E | 0x9A..=0x9E if VARIANT == SCC_VARIANT_STANDARD => {
                let channel = usize::from((address & 0x0F) - 0x0A);
                self.channels[channel].volume = value & 0x0F;
            }
            0x8F | 0x9F if VARIANT == SCC_VARIANT_STANDARD => {
                for (channel, voice) in self.channels.iter_mut().enumerate() {
                    voice.key = value & (1 << channel) != 0;
                }
            }
            0xA0..=0xA9 | 0xB0..=0xB9 if VARIANT == SCC_VARIANT_PLUS => {
                self.write_frequency((address & 0x0F) as usize, value)
            }
            0xAA..=0xAE | 0xBA..=0xBE if VARIANT == SCC_VARIANT_PLUS => {
                let channel = usize::from((address & 0x0F) - 0x0A);
                self.channels[channel].volume = value & 0x0F;
            }
            0xAF | 0xBF if VARIANT == SCC_VARIANT_PLUS => {
                for (channel, voice) in self.channels.iter_mut().enumerate() {
                    voice.key = value & (1 << channel) != 0;
                }
            }
            0xC0..=0xDF if VARIANT == SCC_VARIANT_PLUS => self.test = value,
            0xE0..=0xFF if VARIANT == SCC_VARIANT_STANDARD => self.test = value,
            _ => return false,
        }
        true
    }

    /// Advances one SCC input clock and returns the signed mixed output.
    pub fn clock(&mut self) -> i16 {
        self.channels.iter_mut().map(Channel::clock).sum()
    }

    /// Buffers native samples elapsed through `current_cycle`.
    pub fn sync(&mut self, current_cycle: u64) {
        let frame_cycles = current_cycle.saturating_sub(self.frame_start_cycle);
        if frame_cycles == 0 || self.cpu_clock_hz == 0 || self.sample_rate == 0 {
            self.frame_start_cycle = current_cycle;
            return;
        }

        let exact_samples = frame_cycles as f64 * f64::from(self.sample_rate)
            / f64::from(self.cpu_clock_hz)
            + self.sample_remainder;
        let sample_count = exact_samples as usize;
        self.sample_remainder = exact_samples - sample_count as f64;
        let clocks_per_sample = f64::from(self.cpu_clock_hz) / f64::from(self.sample_rate);

        self.pending_samples.reserve(sample_count);
        for _ in 0..sample_count {
            self.clock_accumulator += clocks_per_sample;
            let mut sample = 0;
            while self.clock_accumulator >= 1.0 {
                sample = self.clock();
                self.clock_accumulator -= 1.0;
            }
            self.pending_samples.push(sample);
        }
        self.frame_start_cycle = current_cycle;
    }

    /// Synchronizes elapsed audio before writing one SCC register.
    pub fn write_at(&mut self, address: u8, value: u8, current_cycle: u64) -> bool {
        self.sync(current_cycle);
        self.write(address, value)
    }

    /// Mixes elapsed SCC output into interleaved stereo samples.
    pub fn mix_samples(
        &mut self,
        frame_end_cycle: u64,
        cpu_clock_hz: u32,
        sample_rate: u32,
        volume: f32,
        output: &mut [f32],
    ) -> usize {
        if cpu_clock_hz == 0 || sample_rate == 0 {
            return 0;
        }
        if self.cpu_clock_hz == 0 && self.sample_rate == 0 {
            self.configure_audio(cpu_clock_hz, sample_rate);
        }
        debug_assert_eq!(self.cpu_clock_hz, cpu_clock_hz);
        debug_assert_eq!(self.sample_rate, sample_rate);
        self.sync(frame_end_cycle);

        let frame_count = self.pending_samples.len().min(output.len() / 2);
        for (frame, &sample) in self.pending_samples[..frame_count].iter().enumerate() {
            let mixed = f32::from(sample) * volume / OUTPUT_DIVISOR;
            output[frame * 2] += mixed;
            output[frame * 2 + 1] += mixed;
        }
        self.pending_samples.drain(..frame_count);
        frame_count * 2
    }

    fn write_waveform(&mut self, address: u8, value: u8) {
        if self.test & 0x40 != 0
            || VARIANT == SCC_VARIANT_STANDARD && self.test & 0x80 != 0 && address >= 0x60
        {
            return;
        }
        let index = usize::from(address & 0x1F);
        if VARIANT == SCC_VARIANT_STANDARD && address >= 0x60 {
            self.channels[3].waveform[index] = value as i8;
            self.channels[4].waveform[index] = value as i8;
        } else {
            self.channels[usize::from(address >> 5)].waveform[index] = value as i8;
        }
    }

    fn write_frequency(&mut self, register: usize, value: u8) {
        let high = register & 1 != 0;
        let channel = register >> 1;
        if channel >= CHANNEL_COUNT {
            return;
        }
        if high {
            self.channels[channel].frequency =
                (self.channels[channel].frequency & 0x00FF) | (u16::from(value) << 8 & 0x0F00);
        } else {
            self.channels[channel].frequency =
                (self.channels[channel].frequency & 0x0F00) | u16::from(value);
        }
        self.channels[channel].frequency &= FREQUENCY_MASK;
        if self.test & 0x20 != 0 {
            self.channels[channel].counter = 0;
        }
        self.channels[channel].clock = u16::MAX;
    }
}

impl<const VARIANT: u8> Default for Scc<VARIANT> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_four_and_five_share_waveform_ram() {
        let mut scc = StandardScc::new();
        assert!(scc.write(0x60, 0xA5));
        assert_eq!(scc.read(0x60), Some(0xA5));
        assert_eq!(scc.read(0x7F), Some(0));
    }

    #[test]
    fn halted_frequency_keeps_silence() {
        let mut scc = StandardScc::new();
        scc.write(0, 0x7F);
        scc.write(0x80, HALTED_FREQUENCY as u8);
        scc.write(0x8F, 1);
        assert!((0..64).all(|_| scc.clock() == 0));
    }

    #[test]
    fn frequency_and_volume_drive_signed_output() {
        let mut scc = StandardScc::new();
        scc.write(0, 0x80);
        scc.write(1, 0x7F);
        scc.write(0x80, 9);
        scc.write(0x81, 0);
        scc.write(0x8A, 0x0F);
        scc.write(0x8F, 1);
        assert_eq!(scc.clock(), -120);
        assert!((0..9).all(|_| scc.clock() == -120));
        assert_eq!(scc.clock(), 119);
    }

    #[test]
    fn plus_has_an_independent_fifth_waveform() {
        let mut scc = SccPlus::new();
        scc.write(0x60, 0x11);
        scc.write(0x80, 0x22);
        assert_eq!(scc.read(0x60), Some(0x11));
        assert_eq!(scc.read(0x80), Some(0x22));
    }

    #[test]
    fn plus_uses_its_native_frequency_register_block() {
        let mut scc = SccPlus::new();
        scc.write(0x80, 0x44);
        scc.write(0xA8, 9);
        scc.write(0xA9, 0);
        scc.write(0xAE, 15);
        scc.write(0xAF, 0x10);
        assert_eq!(scc.read(0x80), Some(0x44));
        assert_eq!(scc.clock(), 63);
    }

    #[test]
    fn host_audio_is_identical_across_frame_chunks() {
        fn configured() -> StandardScc {
            let mut scc = StandardScc::new();
            scc.configure_audio(1_000, 100);
            scc.write(0, 0x40);
            scc.write(0x80, 9);
            scc.write(0x81, 0);
            scc.write(0x8A, 15);
            scc.write(0x8F, 1);
            scc
        }

        let mut whole = configured();
        let mut whole_output = [0.0; 200];
        assert_eq!(
            whole.mix_samples(1_000, 1_000, 100, 1.0, &mut whole_output),
            200
        );

        let mut chunked = configured();
        let mut first = [0.0; 80];
        let mut second = [0.0; 120];
        assert_eq!(chunked.mix_samples(400, 1_000, 100, 1.0, &mut first), 80);
        assert_eq!(
            chunked.mix_samples(1_000, 1_000, 100, 1.0, &mut second),
            120
        );
        assert_eq!(&whole_output[..80], &first);
        assert_eq!(&whole_output[80..], &second);
    }

    #[test]
    fn save_state_preserves_pending_native_samples() {
        let mut scc = StandardScc::new();
        scc.configure_audio(1_000, 100);
        let mut small_output = [0.0; 20];
        assert_eq!(
            scc.mix_samples(1_000, 1_000, 100, 1.0, &mut small_output),
            20
        );
        assert_eq!(scc.pending_samples.len(), 90);
        assert_eq!(scc.sample_remainder, 0.0);

        let encoded = save_state::encode_runtime_state(&scc.capture_state());
        let decoded = save_state::decode_runtime_state(&encoded, 1 << 20).unwrap();
        let mut restored = StandardScc::new();
        restored.configure_audio(1_000, 100);
        restored.restore_state(decoded).unwrap();

        assert_eq!(restored.pending_samples.len(), 90);
        assert_eq!(restored.sample_remainder, 0.0);
    }

    #[test]
    fn register_writes_do_not_change_pending_audio() {
        fn configured() -> StandardScc {
            let mut scc = StandardScc::new();
            scc.configure_audio(1_000, 100);
            for address in 0..WAVEFORM_LENGTH as u8 {
                scc.write(address, 0x40);
            }
            scc.write(0x80, 9);
            scc.write(0x81, 0);
            scc.write(0x8A, 15);
            scc.write(0x8F, 1);
            scc
        }

        let mut uninterrupted = configured();
        let mut expected = [0.0; 200];
        uninterrupted.mix_samples(1_000, 1_000, 100, 1.0, &mut expected);

        let mut buffered = configured();
        let mut first = [0.0; 20];
        buffered.mix_samples(1_000, 1_000, 100, 1.0, &mut first);
        assert_eq!(buffered.pending_samples.len(), 90);
        assert!(buffered.write_at(0x8F, 0, 1_000));

        let mut second = [0.0; 180];
        buffered.mix_samples(1_000, 1_000, 100, 1.0, &mut second);
        assert_eq!(&expected[..20], &first);
        assert_eq!(&expected[20..], &second);

        let mut after_write = [1.0; 20];
        buffered.mix_samples(1_100, 1_000, 100, 1.0, &mut after_write);
        assert_eq!(after_write, [1.0; 20]);
    }
}
