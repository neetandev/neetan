use crate::{
    helpers::{bit, bitfield},
    tables::SSG_AMPLITUDES,
};

// SSG register map:
//
//      System-wide registers:
//           06 ---xxxxx Noise period
//           07 x------- I/O B in(0) or out(1)
//              -x------ I/O A in(0) or out(1)
//              --x----- Noise enable(0) or disable(1) for channel C
//              ---x---- Noise enable(0) or disable(1) for channel B
//              ----x--- Noise enable(0) or disable(1) for channel A
//              -----x-- Tone enable(0) or disable(1) for channel C
//              ------x- Tone enable(0) or disable(1) for channel B
//              -------x Tone enable(0) or disable(1) for channel A
//           0B xxxxxxxx Envelope period fine
//           0C xxxxxxxx Envelope period coarse
//           0D ----x--- Envelope shape: continue
//              -----x-- Envelope shape: attack/decay
//              ------x- Envelope shape: alternate
//              -------x Envelope shape: hold
//           0E xxxxxxxx 8-bit parallel I/O port A
//           0F xxxxxxxx 8-bit parallel I/O port B
//
//      Per-channel registers:
//     00,02,04 xxxxxxxx Tone period (fine) for channel A,B,C
//     01,03,05 ----xxxx Tone period (coarse) for channel A,B,C
//     08,09,0A ---x---- Mode: fixed(0) or variable(1) for channel A,B,C
//              ----xxxx Amplitude for channel A,B,C
save_state::runtime_state! {
/// Authoritative SSG register file.
#[derive(Clone)]
pub(crate) struct SsgRegisters {
    regdata: [u8; 0x10],
}}

impl SsgRegisters {
    pub(crate) fn new() -> Self {
        Self { regdata: [0; 0x10] }
    }

    pub(crate) fn reset(&mut self) {
        self.regdata.fill(0);
    }

    pub(crate) fn read(&self, index: u32) -> u8 {
        self.regdata[index as usize]
    }

    pub(crate) fn write(&mut self, index: u32, data: u8) {
        self.regdata[index as usize] = data;
    }

    pub(crate) fn noise_period(&self) -> u32 {
        bitfield(self.regdata[0x06] as u32, 0, 5)
    }

    pub(crate) fn envelope_period(&self) -> u32 {
        self.regdata[0x0B] as u32 | ((self.regdata[0x0C] as u32) << 8)
    }

    pub(crate) fn envelope_continue(&self) -> u32 {
        bit(self.regdata[0x0D] as u32, 3)
    }

    pub(crate) fn envelope_attack(&self) -> u32 {
        bit(self.regdata[0x0D] as u32, 2)
    }

    pub(crate) fn envelope_alternate(&self) -> u32 {
        bit(self.regdata[0x0D] as u32, 1)
    }

    pub(crate) fn envelope_hold(&self) -> u32 {
        bit(self.regdata[0x0D] as u32, 0)
    }

    pub(crate) fn ch_noise_enable_n(&self, choffs: u32) -> u32 {
        bit(self.regdata[0x07] as u32, (3 + choffs) as i32)
    }

    pub(crate) fn ch_tone_enable_n(&self, choffs: u32) -> u32 {
        bit(self.regdata[0x07] as u32, choffs as i32)
    }

    pub(crate) fn ch_tone_period(&self, choffs: u32) -> u32 {
        self.regdata[(2 * choffs) as usize] as u32
            | ((bitfield(self.regdata[(0x01 + 2 * choffs) as usize] as u32, 0, 4)) << 8)
    }

    pub(crate) fn ch_envelope_enable(&self, choffs: u32) -> u32 {
        bit(self.regdata[(0x08 + choffs) as usize] as u32, 4)
    }

    pub(crate) fn ch_amplitude(&self, choffs: u32) -> u32 {
        bitfield(self.regdata[(0x08 + choffs) as usize] as u32, 0, 4)
    }
}

save_state::runtime_state! {
/// Current three-channel SSG output values.
#[derive(Clone)]
pub(crate) struct SsgOutput {
    pub(crate) data: [i32; 3],
}}

impl SsgOutput {}

save_state::runtime_state! {
/// Authoritative SSG tone, noise, and envelope progress.
#[derive(Clone)]
pub(crate) struct SsgEngine {
    tone_count: [u32; 3],
    tone_state: [u32; 3],
    envelope_count: u32,
    envelope_state: u32,
    noise_count: u32,
    noise_state: u32,
    regs: SsgRegisters,
}}

impl SsgEngine {
    pub(crate) fn new() -> Self {
        Self {
            tone_count: [0; 3],
            tone_state: [0; 3],
            envelope_count: 0,
            envelope_state: 0,
            noise_count: 0,
            noise_state: 1,
            regs: SsgRegisters::new(),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.regs.reset();
        self.tone_count = [0; 3];
        self.tone_state = [0; 3];
        self.envelope_count = 0;
        self.envelope_state = 0;
        self.noise_count = 0;
        self.noise_state = 1;
    }

    pub(crate) fn clock(&mut self) {
        // Clock tones; tone period units are clock/16 but since we run at clock/8
        // that works out for us to toggle the state (50% duty cycle) at twice the
        // programmed period.
        for chan in 0..3 {
            self.tone_count[chan] += 1;
            if self.tone_count[chan] >= self.regs.ch_tone_period(chan as u32) {
                self.tone_state[chan] ^= 1;
                self.tone_count[chan] = 0;
            }
        }

        // Clock noise; noise period units are clock/16 but since we run at clock/8,
        // our counter needs a right shift prior to compare; note that a period of 0
        // should produce an identical result to a period of 1, so add a special
        // check against that case.
        // Noise LFSR: 17-bit polynomial (taps at bits 0 and 3).
        self.noise_count += 1;
        if (self.noise_count >> 1) >= self.regs.noise_period() && self.noise_count != 1 {
            self.noise_state ^= (bit(self.noise_state, 0) ^ bit(self.noise_state, 3)) << 17;
            self.noise_state >>= 1;
            self.noise_count = 0;
        }

        // Clock envelope; envelope period units are clock/8 (manual says clock/256
        // but that's for all 32 steps).
        self.envelope_count += 1;
        if self.envelope_count >= self.regs.envelope_period() {
            self.envelope_state += 1;
            self.envelope_count = 0;
        }
    }

    pub(crate) fn output(&mut self, output: &mut SsgOutput) {
        // Compute the envelope volume.
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

        for chan in 0..3usize {
            // Noise/tone enable bits are inverted: 0=enabled, 1=disabled.
            // OR with the current state means disabled channels always read as "on".
            let noise_on = self.regs.ch_noise_enable_n(chan as u32) | (self.noise_state & 1);
            let tone_on = self.regs.ch_tone_enable_n(chan as u32) | self.tone_state[chan];

            let volume;
            if (noise_on & tone_on) == 0 {
                volume = 0;
            } else if self.regs.ch_envelope_enable(chan as u32) != 0 {
                volume = envelope_volume;
            } else {
                // Scale the tone amplitude up to match envelope values;
                // according to the datasheet, amplitude 15 maps to envelope 31.
                let mut v = self.regs.ch_amplitude(chan as u32) * 2;
                if v != 0 {
                    v |= 1;
                }
                volume = v;
            }

            // Volume-to-amplitude table taken from MAME's implementation, biased so that 0 == 0.
            output.data[chan] = SSG_AMPLITUDES[volume as usize] as i32;
        }
    }

    pub(crate) fn read(&self, regnum: u32) -> u8 {
        self.regs.read(regnum)
    }

    pub(crate) fn write(&mut self, regnum: u32, data: u8) {
        self.regs.write(regnum, data);
        // Writes to the envelope shape register reset the envelope state.
        if regnum == 0x0D {
            self.envelope_state = 0;
        }
    }
}
