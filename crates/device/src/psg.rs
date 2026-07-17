//! AY-3-8910 and YM2149 programmable sound generators.
//!
//! Three square-wave tone channels, one noise generator, one envelope
//! generator and two 8-bit parallel I/O ports. The tone/noise/envelope core
//! and register file mirror the SSG block of the Yamaha OPN family; the
//! amplitude table uses the AY-3-8910 logarithmic volume law (16 levels)
//! rather than the OPN SSG curve.
//!
//! Elapsed samples are buffered before timed register writes so host callback
//! boundaries cannot move audible changes backward in time.

/// AY-3-8910 amplitude law, expanded to 32 entries so the shared core (which
/// produces a 0..31 volume index) can index it directly. Each AY level spans
/// two adjacent entries, giving the chip's native 16-level resolution for both
/// fixed amplitude and the envelope. Peak matches the OPN SSG scale (16382).
static AY_AMPLITUDES: [i16; 32] = [
    0, 0, 128, 128, 181, 181, 256, 256, 362, 362, 512, 512, 724, 724, 1024, 1024, 1448, 1448, 2048,
    2048, 2896, 2896, 4096, 4096, 5793, 5793, 8192, 8192, 11584, 11584, 16382, 16382,
];

/// YM2149 amplitude law with all 32 envelope levels.
static YM2149_AMPLITUDES: [i16; 32] = [
    0, 32, 78, 141, 178, 222, 262, 306, 369, 441, 509, 585, 701, 836, 965, 1112, 1334, 1595, 1853,
    2146, 2576, 3081, 3576, 4135, 5000, 6006, 7023, 8155, 9963, 11976, 14132, 16382,
];

fn bit(value: u32, start: u32) -> u32 {
    (value >> start) & 1
}

fn bitfield(value: u32, start: u32, length: u32) -> u32 {
    (value >> start) & ((1 << length) - 1)
}

save_state::runtime_state! {
/// The 16-byte AY-3-8910 register file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AyRegisters {
    regdata: [u8; 0x10],
}}

impl AyRegisters {
    fn new() -> Self {
        Self { regdata: [0; 0x10] }
    }

    fn reset(&mut self) {
        self.regdata.fill(0);
    }

    fn read(&self, index: u32) -> u8 {
        self.regdata[index as usize]
    }

    fn write(&mut self, index: u32, data: u8) {
        self.regdata[index as usize] = data;
    }

    fn noise_period(&self) -> u32 {
        bitfield(self.regdata[0x06] as u32, 0, 5)
    }

    fn envelope_period(&self) -> u32 {
        self.regdata[0x0B] as u32 | ((self.regdata[0x0C] as u32) << 8)
    }

    fn envelope_continue(&self) -> u32 {
        bit(self.regdata[0x0D] as u32, 3)
    }

    fn envelope_attack(&self) -> u32 {
        bit(self.regdata[0x0D] as u32, 2)
    }

    fn envelope_alternate(&self) -> u32 {
        bit(self.regdata[0x0D] as u32, 1)
    }

    fn envelope_hold(&self) -> u32 {
        bit(self.regdata[0x0D] as u32, 0)
    }

    fn ch_noise_enable_n(&self, channel: u32) -> u32 {
        bit(self.regdata[0x07] as u32, 3 + channel)
    }

    fn ch_tone_enable_n(&self, channel: u32) -> u32 {
        bit(self.regdata[0x07] as u32, channel)
    }

    fn ch_tone_period(&self, channel: u32) -> u32 {
        self.regdata[(2 * channel) as usize] as u32
            | (bitfield(self.regdata[(0x01 + 2 * channel) as usize] as u32, 0, 4) << 8)
    }

    fn ch_envelope_enable(&self, channel: u32) -> u32 {
        bit(self.regdata[(0x08 + channel) as usize] as u32, 4)
    }

    fn ch_amplitude(&self, channel: u32) -> u32 {
        bitfield(self.regdata[(0x08 + channel) as usize] as u32, 0, 4)
    }

    fn port_a_is_output(&self) -> bool {
        bit(self.regdata[0x07] as u32, 6) != 0
    }

    fn port_b_is_output(&self) -> bool {
        bit(self.regdata[0x07] as u32, 7) != 0
    }
}

save_state::runtime_state! {
/// The tone, noise, and envelope generator core.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AyEngine {
    tone_count: [u32; 3],
    tone_state: [u32; 3],
    envelope_count: u32,
    envelope_state: u32,
    noise_count: u32,
    noise_state: u32,
    regs: AyRegisters,
}}

impl AyEngine {
    fn new() -> Self {
        Self {
            tone_count: [0; 3],
            tone_state: [0; 3],
            envelope_count: 0,
            envelope_state: 0,
            noise_count: 0,
            noise_state: 1,
            regs: AyRegisters::new(),
        }
    }

    fn reset(&mut self) {
        self.regs.reset();
        self.tone_count = [0; 3];
        self.tone_state = [0; 3];
        self.envelope_count = 0;
        self.envelope_state = 0;
        self.noise_count = 0;
        self.noise_state = 1;
    }

    /// Advances the generators by one internal tick (the input clock divided
    /// by eight).
    fn clock(&mut self) {
        for channel in 0..3 {
            self.tone_count[channel] += 1;
            if self.tone_count[channel] >= self.regs.ch_tone_period(channel as u32) {
                self.tone_state[channel] ^= 1;
                self.tone_count[channel] = 0;
            }
        }

        self.noise_count += 1;
        if (self.noise_count >> 1) >= self.regs.noise_period() && self.noise_count != 1 {
            self.noise_state ^= (bit(self.noise_state, 0) ^ bit(self.noise_state, 3)) << 17;
            self.noise_state >>= 1;
            self.noise_count = 0;
        }

        self.envelope_count += 1;
        if self.envelope_count >= self.regs.envelope_period() {
            self.envelope_state += 1;
            self.envelope_count = 0;
        }
    }

    /// Computes the three channel amplitudes for the current state. A tone whose
    /// period is below `min_audible_tone_period` is ultrasonic for the current
    /// sample rate. Its oscillation is dropped so it cannot alias into the output
    /// band.
    fn output<const VARIANT: u8>(&mut self, min_audible_tone_period: u32) -> [i32; 3] {
        let envelope_volume;
        if (self.regs.envelope_hold() | (self.regs.envelope_continue() ^ 1)) != 0
            && self.envelope_state >= 32
        {
            self.envelope_state = 32;
            envelope_volume = if ((self.regs.envelope_attack() ^ self.regs.envelope_alternate())
                & self.regs.envelope_continue())
                != 0
            {
                31
            } else {
                0
            };
        } else {
            let mut attack = self.regs.envelope_attack();
            if self.regs.envelope_alternate() != 0 {
                attack ^= bit(self.envelope_state, 5);
            }
            envelope_volume = (self.envelope_state & 31) ^ (if attack != 0 { 0 } else { 31 });
        }

        let mut data = [0i32; 3];
        for (channel, slot) in data.iter_mut().enumerate() {
            let channel = channel as u32;
            let noise_on = self.regs.ch_noise_enable_n(channel) | (self.noise_state & 1);
            let tone_bit = if self.regs.ch_tone_period(channel) > min_audible_tone_period {
                self.tone_state[channel as usize]
            } else {
                1
            };
            let tone_on = self.regs.ch_tone_enable_n(channel) | tone_bit;

            let volume = if (noise_on & tone_on) == 0 {
                0
            } else if self.regs.ch_envelope_enable(channel) != 0 {
                envelope_volume
            } else {
                let mut value = self.regs.ch_amplitude(channel) * 2;
                if value != 0 {
                    value |= 1;
                }
                value
            };

            let amplitudes = if VARIANT == PSG_VARIANT_YM2149 {
                &YM2149_AMPLITUDES
            } else {
                &AY_AMPLITUDES
            };
            *slot = amplitudes[volume as usize] as i32;
        }
        data
    }

    fn write(&mut self, regnum: u32, data: u8) {
        self.regs.write(regnum, data);
        if regnum == 0x0D {
            self.envelope_state = 0;
        }
    }
}

/// Internal divider between the input clock and the generator tick rate.
const CLOCK_DIVIDER: u32 = 8;

/// Sum of the three channel peaks, used to normalize the mixed output.
const MIX_PEAK: f32 = 3.0 * 16382.0;

/// AY-3-8910 specialization selector for [`Psg`].
pub const PSG_VARIANT_AY_3_8910: u8 = 0;
/// YM2149 specialization selector for [`Psg`].
pub const PSG_VARIANT_YM2149: u8 = 1;

save_state::runtime_state! {
/// Authoritative PSG device state.
#[derive(Debug, Clone)]
pub struct PsgState {
    engine: AyEngine,
    address: u8,
    input_clock_numerator: u64,
    input_clock_denominator: u32,
    cpu_clock_hz: u32,
    sample_rate: u32,
    frame_start_cycle: u64,
    pending_samples: Vec<i32>,
    sample_remainder: f64,
    clock_accumulator: f64,
    port_a_input: u8,
    port_b_input: u8,
}}

/// Programmable sound generator specialized for one chip variant.
#[derive(Debug, Clone)]
pub struct Psg<const VARIANT: u8> {
    state: PsgState,
}

/// General Instrument AY-3-8910 programmable sound generator.
pub type Ay38910 = Psg<PSG_VARIANT_AY_3_8910>;

/// Yamaha YM2149 programmable sound generator.
pub type Ym2149 = Psg<PSG_VARIANT_YM2149>;

impl<const VARIANT: u8> Default for Psg<VARIANT> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const VARIANT: u8> Psg<VARIANT> {
    /// Creates a PSG in the power-on state.
    pub fn new() -> Self {
        Self {
            state: PsgState {
                engine: AyEngine::new(),
                address: 0,
                input_clock_numerator: 0,
                input_clock_denominator: 0,
                cpu_clock_hz: 0,
                sample_rate: 0,
                frame_start_cycle: 0,
                pending_samples: Vec::new(),
                sample_remainder: 0.0,
                clock_accumulator: 0.0,
                port_a_input: 0xFF,
                port_b_input: 0xFF,
            },
        }
    }

    /// Captures registers, oscillator phases, and sample history.
    pub fn capture_state(&self) -> PsgState {
        self.state.clone()
    }

    /// Restores complete PSG state.
    pub fn restore_state(
        &mut self,
        state: PsgState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.input_clock_numerator != self.state.input_clock_numerator
            || state.input_clock_denominator != self.state.input_clock_denominator
            || state.cpu_clock_hz != self.state.cpu_clock_hz
            || state.sample_rate != self.state.sample_rate
            || state.address > 15
            || !state.sample_remainder.is_finite()
            || !state.clock_accumulator.is_finite()
            || !(0.0..1.0).contains(&state.sample_remainder)
            || !(0.0..1.0).contains(&state.clock_accumulator)
        {
            return Err(save_state::StateValidationError::new(
                "AY-3-8910 address or sample phase is invalid",
            ));
        }
        self.state = state;
        Ok(())
    }

    /// Resets the PSG.
    pub fn reset(&mut self) {
        self.state.engine.reset();
        self.state.address = 0;
        self.state.pending_samples.clear();
        self.state.sample_remainder = 0.0;
        self.state.clock_accumulator = 0.0;
    }

    /// Configures integer input, CPU, and host audio clocks.
    pub fn configure_audio(&mut self, input_clock_hz: u32, cpu_clock_hz: u32, sample_rate: u32) {
        self.configure_audio_rational(u64::from(input_clock_hz), 1, cpu_clock_hz, sample_rate);
    }

    /// Configures rational input, CPU, and host audio clocks.
    pub fn configure_audio_rational(
        &mut self,
        input_clock_numerator: u64,
        input_clock_denominator: u32,
        cpu_clock_hz: u32,
        sample_rate: u32,
    ) {
        self.state.input_clock_numerator = input_clock_numerator;
        self.state.input_clock_denominator = input_clock_denominator;
        self.state.cpu_clock_hz = cpu_clock_hz;
        self.state.sample_rate = sample_rate;
    }

    /// Latches the register address (port 0xA0).
    pub fn address_w(&mut self, value: u8) {
        self.state.address = value & 0x0F;
    }

    /// Writes the latched register (port 0xA1).
    pub fn data_w(&mut self, value: u8) {
        self.state.engine.write(self.state.address as u32, value);
    }

    /// Synchronizes elapsed audio before writing the selected register.
    pub fn data_w_at(&mut self, value: u8, current_cycle: u64) {
        self.sync(current_cycle);
        self.data_w(value);
    }

    /// Reads the latched register (port 0xA2). The two parallel I/O ports
    /// return their external input when configured as inputs.
    pub fn data_r(&self) -> u8 {
        match self.state.address {
            0x0E if !self.state.engine.regs.port_a_is_output() => self.state.port_a_input,
            0x0F if !self.state.engine.regs.port_b_is_output() => self.state.port_b_input,
            _ => self.state.engine.regs.read(self.state.address as u32),
        }
    }

    /// Returns the currently selected register.
    pub const fn selected_register(&self) -> u8 {
        self.state.address
    }

    /// Drives the external input of parallel port A (joystick state plus the
    /// sync flags on the PC-6001).
    pub fn set_port_a_input(&mut self, value: u8) {
        self.state.port_a_input = value;
    }

    /// Drives the external input of parallel port B.
    pub fn set_port_b_input(&mut self, value: u8) {
        self.state.port_b_input = value;
    }

    /// Whether parallel port A is configured as an output.
    pub fn port_a_is_output(&self) -> bool {
        self.state.engine.regs.port_a_is_output()
    }

    /// Whether parallel port B is configured as an output.
    pub fn port_b_is_output(&self) -> bool {
        self.state.engine.regs.port_b_is_output()
    }

    /// Returns the parallel port A output latch.
    pub fn port_a_output(&self) -> u8 {
        self.state.engine.regs.read(0x0E)
    }

    /// Returns the parallel port B output latch (the joystick multiplexer
    /// control on the PC-6001).
    pub fn port_b_output(&self) -> u8 {
        self.state.engine.regs.read(0x0F)
    }

    /// Fills `output` with interleaved stereo samples (`[L, R, ...]`) covering
    /// the interval `[frame_start_cycle, frame_end_cycle)`, returning the
    /// number of `f32` values written.
    pub fn generate_samples(
        &mut self,
        frame_end_cycle: u64,
        input_clock_hz: u32,
        cpu_clock_hz: u32,
        sample_rate: u32,
        volume: f32,
        output: &mut [f32],
    ) -> usize {
        self.generate_samples_rational(
            frame_end_cycle,
            u64::from(input_clock_hz),
            1,
            cpu_clock_hz,
            sample_rate,
            volume,
            output,
        )
    }

    /// Fills `output` using an input clock expressed as a rational frequency.
    #[allow(clippy::too_many_arguments)]
    pub fn generate_samples_rational(
        &mut self,
        frame_end_cycle: u64,
        input_clock_numerator: u64,
        input_clock_denominator: u32,
        cpu_clock_hz: u32,
        sample_rate: u32,
        volume: f32,
        output: &mut [f32],
    ) -> usize {
        if sample_rate == 0 || cpu_clock_hz == 0 || input_clock_denominator == 0 {
            return 0;
        }
        if self.state.input_clock_denominator == 0 {
            self.configure_audio_rational(
                input_clock_numerator,
                input_clock_denominator,
                cpu_clock_hz,
                sample_rate,
            );
        }
        debug_assert_eq!(self.state.input_clock_numerator, input_clock_numerator);
        debug_assert_eq!(self.state.input_clock_denominator, input_clock_denominator);
        debug_assert_eq!(self.state.cpu_clock_hz, cpu_clock_hz);
        debug_assert_eq!(self.state.sample_rate, sample_rate);
        self.sync(frame_end_cycle);

        let frame_count = self.state.pending_samples.len().min(output.len() / 2);
        for (frame, &mixed) in self.state.pending_samples[..frame_count].iter().enumerate() {
            let sample = mixed as f32 / MIX_PEAK * volume;
            output[frame * 2] = sample;
            output[frame * 2 + 1] = sample;
        }
        self.state.pending_samples.drain(..frame_count);
        frame_count * 2
    }

    /// Buffers native output samples elapsed through `current_cycle`.
    pub fn sync(&mut self, current_cycle: u64) {
        let frame_cycles = current_cycle.saturating_sub(self.state.frame_start_cycle);
        if frame_cycles == 0
            || self.state.sample_rate == 0
            || self.state.cpu_clock_hz == 0
            || self.state.input_clock_denominator == 0
        {
            self.state.frame_start_cycle = current_cycle;
            return;
        }

        let exact_samples = frame_cycles as f64 * f64::from(self.state.sample_rate)
            / f64::from(self.state.cpu_clock_hz)
            + self.state.sample_remainder;
        let sample_count = exact_samples as usize;
        self.state.sample_remainder = exact_samples - sample_count as f64;
        let input_clock_hz =
            self.state.input_clock_numerator as f64 / f64::from(self.state.input_clock_denominator);
        let ticks_per_sample =
            input_clock_hz / f64::from(CLOCK_DIVIDER) / f64::from(self.state.sample_rate);
        let min_audible_tone_period =
            (input_clock_hz / (4.0 * f64::from(self.state.sample_rate))) as u32;

        self.state.pending_samples.reserve(sample_count);
        for _ in 0..sample_count {
            self.state.clock_accumulator += ticks_per_sample;
            while self.state.clock_accumulator >= 1.0 {
                self.state.engine.clock();
                self.state.clock_accumulator -= 1.0;
            }
            let channels = self.state.engine.output::<VARIANT>(min_audible_tone_period);
            self.state
                .pending_samples
                .push(channels[0] + channels[1] + channels[2]);
        }
        self.state.frame_start_cycle = current_cycle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_write_and_read_round_trips() {
        let mut psg = Ay38910::new();
        psg.address_w(0x08);
        psg.data_w(0x0F);
        psg.address_w(0x08);
        assert_eq!(psg.data_r(), 0x0F);
    }

    #[test]
    fn input_ports_read_external_state_when_configured_as_input() {
        let mut psg = Ay38910::new();
        psg.set_port_a_input(0xA5);
        // Register 0x07 defaults to 0: port A is an input.
        psg.address_w(0x0E);
        assert_eq!(psg.data_r(), 0xA5);
    }

    #[test]
    fn enabled_tone_produces_a_non_silent_frame() {
        let mut psg = Ay38910::new();
        // Channel A tone period.
        psg.address_w(0x00);
        psg.data_w(0x40);
        psg.address_w(0x01);
        psg.data_w(0x00);
        // Mixer: enable tone A (active-low: clear bit 0), disable noise.
        psg.address_w(0x07);
        psg.data_w(0x3E);
        // Channel A amplitude max.
        psg.address_w(0x08);
        psg.data_w(0x0F);

        let mut output = vec![0.0f32; 4096];
        let written = psg.generate_samples(48_000, 1_996_800, 3_993_600, 48_000, 1.0, &mut output);
        assert!(written > 0);
        assert!(
            output[..written].iter().any(|&s| s != 0.0),
            "an enabled tone should produce a non-silent frame"
        );
    }

    #[test]
    fn ultrasonic_tone_is_silent_not_aliased() {
        // A tone left latched at period 0 (ultrasonic) with tone enabled, noise
        // disabled and maximum amplitude must not oscillate into the audible band:
        // the channel holds a constant level (Galaxian PSG "latched tone" regression).
        let mut psg = Ay38910::new();
        // Channel A tone period 0 (fine and coarse both zero, the power-on state).
        // Mixer: enable tone A (clear bit 0), disable noise (set bits 3-5).
        psg.address_w(0x07);
        psg.data_w(0x3E);
        // Channel A amplitude maximum.
        psg.address_w(0x08);
        psg.data_w(0x0F);

        let mut output = vec![0.0f32; 4096];
        let written = psg.generate_samples(48_000, 2_000_000, 4_000_000, 48_000, 1.0, &mut output);
        assert!(written > 0);
        let first = output[0];
        assert!(
            output[..written].iter().all(|&s| s == first),
            "an ultrasonic tone must hold a constant level, not oscillate"
        );
    }

    #[test]
    fn envelope_shape_write_resets_state() {
        let mut psg = Ay38910::new();
        psg.address_w(0x0D);
        psg.data_w(0x08);
        assert_eq!(psg.state.engine.envelope_state, 0);
    }

    #[test]
    fn ym2149_uses_distinct_envelope_steps() {
        let mut psg = Ym2149::new();
        psg.state.engine.regs.write(0x07, 0x3F);
        psg.state.engine.regs.write(0x08, 0x10);
        psg.state.engine.regs.write(0x0D, 0x04);
        psg.state.engine.envelope_state = 1;
        let first = psg.state.engine.output::<PSG_VARIANT_YM2149>(0)[0];
        psg.state.engine.envelope_state = 2;
        let second = psg.state.engine.output::<PSG_VARIANT_YM2149>(0)[0];
        assert_ne!(first, second);
    }

    #[test]
    fn rational_clock_matches_an_equivalent_integer_clock() {
        let mut rational = Ym2149::new();
        let mut integer = Ym2149::new();
        for psg in [&mut rational, &mut integer] {
            psg.address_w(0x07);
            psg.data_w(0x3E);
            psg.address_w(0x08);
            psg.data_w(0x0F);
        }
        let mut rational_output = [0.0; 256];
        let mut integer_output = [0.0; 256];
        let rational_count = rational.generate_samples_rational(
            10_000,
            3_579_546,
            2,
            3_579_546,
            48_000,
            1.0,
            &mut rational_output,
        );
        let integer_count = integer.generate_samples(
            10_000,
            1_789_773,
            3_579_546,
            48_000,
            1.0,
            &mut integer_output,
        );
        assert_eq!(rational_count, integer_count);
        assert_eq!(rational_output, integer_output);
    }

    #[test]
    fn timed_write_changes_only_later_samples() {
        let mut psg = Ym2149::new();
        psg.configure_audio(800, 1_000, 100);
        psg.address_w(0x07);
        psg.data_w(0x3F);
        psg.address_w(0x08);
        psg.data_w_at(0x0F, 500);

        let mut output = [0.0; 200];
        assert_eq!(
            psg.generate_samples(1_000, 800, 1_000, 100, 1.0, &mut output),
            200
        );
        assert_eq!(output[..100], [0.0; 100]);
        assert!(output[100..].iter().all(|&sample| sample > 0.0));
    }

    #[test]
    fn register_writes_do_not_change_pending_audio() {
        fn configured() -> Ym2149 {
            let mut psg = Ym2149::new();
            psg.configure_audio(800, 1_000, 100);
            psg.address_w(0x07);
            psg.data_w(0x3F);
            psg.address_w(0x08);
            psg.data_w(0x0F);
            psg
        }

        let mut uninterrupted = configured();
        let mut expected = [0.0; 200];
        uninterrupted.generate_samples(1_000, 800, 1_000, 100, 1.0, &mut expected);

        let mut buffered = configured();
        let mut first = [0.0; 20];
        buffered.generate_samples(1_000, 800, 1_000, 100, 1.0, &mut first);
        buffered.data_w_at(0, 1_000);
        let mut second = [0.0; 180];
        buffered.generate_samples(1_000, 800, 1_000, 100, 1.0, &mut second);

        assert_eq!(&expected[..20], &first);
        assert_eq!(&expected[20..], &second);
        let mut after_write = [0.0; 20];
        buffered.generate_samples(1_100, 800, 1_000, 100, 1.0, &mut after_write);
        assert_eq!(after_write, [0.0; 20]);
    }

    #[test]
    fn save_state_preserves_pending_native_samples() {
        let mut psg = Ym2149::new();
        psg.configure_audio(800, 1_000, 100);
        psg.address_w(0x07);
        psg.data_w(0x3F);
        psg.address_w(0x08);
        psg.data_w(0x0F);
        let mut small_output = [0.0; 20];
        psg.generate_samples(1_000, 800, 1_000, 100, 1.0, &mut small_output);
        assert_eq!(psg.state.pending_samples.len(), 90);

        let encoded = save_state::encode_runtime_state(&psg.capture_state());
        let decoded = save_state::decode_runtime_state(&encoded, 1 << 20).unwrap();
        let mut restored = Ym2149::new();
        restored.configure_audio(800, 1_000, 100);
        restored.restore_state(decoded).unwrap();

        assert_eq!(restored.state.pending_samples, psg.state.pending_samples);
        assert_eq!(restored.state.sample_remainder, 0.0);
    }
}
