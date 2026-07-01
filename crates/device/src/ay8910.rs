//! AY-3-8910 programmable sound generator.
//!
//! Three square-wave tone channels, one noise generator, one envelope
//! generator and two 8-bit parallel I/O ports. The tone/noise/envelope core
//! and register file mirror the SSG block of the Yamaha OPN family; the
//! amplitude table uses the AY-3-8910 logarithmic volume law (16 levels)
//! rather than the OPN SSG curve.
//!
//! Samples are generated analytically once per audio frame. Register writes
//! take effect immediately and are sampled at frame granularity, which is
//! sufficient for the PSG's slowly-changing control state.

/// AY-3-8910 amplitude law, expanded to 32 entries so the shared core (which
/// produces a 0..31 volume index) can index it directly. Each AY level spans
/// two adjacent entries, giving the chip's native 16-level resolution for both
/// fixed amplitude and the envelope. Peak matches the OPN SSG scale (16382).
static AY_AMPLITUDES: [i16; 32] = [
    0, 0, 128, 128, 181, 181, 256, 256, 362, 362, 512, 512, 724, 724, 1024, 1024, 1448, 1448, 2048,
    2048, 2896, 2896, 4096, 4096, 5793, 5793, 8192, 8192, 11584, 11584, 16382, 16382,
];

fn bit(value: u32, start: u32) -> u32 {
    (value >> start) & 1
}

fn bitfield(value: u32, start: u32, length: u32) -> u32 {
    (value >> start) & ((1 << length) - 1)
}

/// The 16-byte AY-3-8910 register file with typed accessors.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AyRegisters {
    regdata: [u8; 0x10],
}

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

/// The tone/noise/envelope generator core.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AyEngine {
    tone_count: [u32; 3],
    tone_state: [u32; 3],
    envelope_count: u32,
    envelope_state: u32,
    noise_count: u32,
    noise_state: u32,
    regs: AyRegisters,
}

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

    /// Computes the three channel amplitudes for the current state.
    fn output(&mut self) -> [i32; 3] {
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
            let tone_on = self.regs.ch_tone_enable_n(channel) | self.tone_state[channel as usize];

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

            *slot = AY_AMPLITUDES[volume as usize] as i32;
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

/// AY-3-8910 device.
#[derive(Debug, Clone)]
pub struct Ay8910 {
    engine: AyEngine,
    address: u8,
    frame_start_cycle: u64,
    sample_remainder: f64,
    clock_accumulator: f64,
    port_a_input: u8,
    port_b_input: u8,
}

impl Default for Ay8910 {
    fn default() -> Self {
        Self::new()
    }
}

impl Ay8910 {
    /// Creates a PSG in the power-on state.
    pub fn new() -> Self {
        Self {
            engine: AyEngine::new(),
            address: 0,
            frame_start_cycle: 0,
            sample_remainder: 0.0,
            clock_accumulator: 0.0,
            port_a_input: 0xFF,
            port_b_input: 0xFF,
        }
    }

    /// Resets the PSG.
    pub fn reset(&mut self) {
        self.engine.reset();
        self.address = 0;
        self.clock_accumulator = 0.0;
    }

    /// Latches the register address (port 0xA0).
    pub fn address_w(&mut self, value: u8) {
        self.address = value & 0x0F;
    }

    /// Writes the latched register (port 0xA1).
    pub fn data_w(&mut self, value: u8) {
        self.engine.write(self.address as u32, value);
    }

    /// Reads the latched register (port 0xA2). The two parallel I/O ports
    /// return their external input when configured as inputs.
    pub fn data_r(&self) -> u8 {
        match self.address {
            0x0E if !self.engine.regs.port_a_is_output() => self.port_a_input,
            0x0F if !self.engine.regs.port_b_is_output() => self.port_b_input,
            _ => self.engine.regs.read(self.address as u32),
        }
    }

    /// Drives the external input of parallel port A (joystick state plus the
    /// sync flags on the PC-6001).
    pub fn set_port_a_input(&mut self, value: u8) {
        self.port_a_input = value;
    }

    /// Drives the external input of parallel port B.
    pub fn set_port_b_input(&mut self, value: u8) {
        self.port_b_input = value;
    }

    /// Returns the parallel port B output latch (the joystick multiplexer
    /// control on the PC-6001).
    pub fn port_b_output(&self) -> u8 {
        self.engine.regs.read(0x0F)
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
        let frame_cycles = frame_end_cycle.saturating_sub(self.frame_start_cycle);
        if frame_cycles == 0 || sample_rate == 0 || cpu_clock_hz == 0 {
            self.frame_start_cycle = frame_end_cycle;
            return 0;
        }

        let frame_capacity = output.len() / 2;
        let exact_samples = (frame_cycles as f64 * f64::from(sample_rate))
            / f64::from(cpu_clock_hz)
            + self.sample_remainder;
        let frame_count = (exact_samples as usize).min(frame_capacity);
        self.sample_remainder = exact_samples - frame_count as f64;

        if frame_count == 0 {
            self.frame_start_cycle = frame_end_cycle;
            return 0;
        }

        let ticks_per_sample = f64::from(input_clock_hz / CLOCK_DIVIDER) / f64::from(sample_rate);

        for frame in 0..frame_count {
            self.clock_accumulator += ticks_per_sample;
            while self.clock_accumulator >= 1.0 {
                self.engine.clock();
                self.clock_accumulator -= 1.0;
            }

            let channels = self.engine.output();
            let mixed = (channels[0] + channels[1] + channels[2]) as f32 / MIX_PEAK;
            let sample = mixed * volume;
            output[frame * 2] = sample;
            output[frame * 2 + 1] = sample;
        }

        self.frame_start_cycle = frame_end_cycle;
        frame_count * 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_write_and_read_round_trips() {
        let mut psg = Ay8910::new();
        psg.address_w(0x08);
        psg.data_w(0x0F);
        psg.address_w(0x08);
        assert_eq!(psg.data_r(), 0x0F);
    }

    #[test]
    fn input_ports_read_external_state_when_configured_as_input() {
        let mut psg = Ay8910::new();
        psg.set_port_a_input(0xA5);
        // Register 0x07 defaults to 0: port A is an input.
        psg.address_w(0x0E);
        assert_eq!(psg.data_r(), 0xA5);
    }

    #[test]
    fn enabled_tone_produces_a_non_silent_frame() {
        let mut psg = Ay8910::new();
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
    fn envelope_shape_write_resets_state() {
        let mut psg = Ay8910::new();
        psg.address_w(0x0D);
        psg.data_w(0x08);
        assert_eq!(psg.engine.envelope_state, 0);
    }
}
