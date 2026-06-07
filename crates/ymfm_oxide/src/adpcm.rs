use crate::{
    helpers::{bit, bitfield, clamp},
    tables::{ADPCM_A_STEP_INC, ADPCM_A_STEPS, ADPCM_B_STEP_SCALE},
};

fn read_memory(memory: &[u8], address: u32) -> u8 {
    memory
        .get(address as usize % memory.len())
        .copied()
        .unwrap_or(0)
}

fn read_optional_memory(memory: &Option<&mut [u8]>, address: u32) -> u8 {
    memory
        .as_ref()
        .and_then(|memory| memory.get(address as usize % memory.len()).copied())
        .unwrap_or(0)
}

fn write_optional_memory(memory: &mut Option<&mut [u8]>, address: u32, value: u8) {
    if let Some(memory) = memory.as_deref_mut() {
        memory[address as usize % memory.len()] = value;
    }
}

// ADPCM-A register map:
//
//      System-wide registers:
//           00 x------- Dump (disable=1) or keyon (0) control
//              --xxxxxx Mask of channels to dump or keyon
//           01 --xxxxxx Total level
//           02 xxxxxxxx Test register
//        08-0D x------- Pan left
//              -x------ Pan right
//              ---xxxxx Instrument level
//        10-15 xxxxxxxx Start address (low)
//        18-1D xxxxxxxx Start address (high)
//        20-25 xxxxxxxx End address (low)
//        28-2D xxxxxxxx End address (high)
pub(crate) struct AdpcmARegisters {
    regdata: [u8; 0x30],
}

impl AdpcmARegisters {
    pub(crate) fn new() -> Self {
        Self { regdata: [0; 0x30] }
    }

    pub(crate) fn reset(&mut self) {
        self.regdata.fill(0);
        // Initialize pans to on by default, and max instrument volume;
        // some neogeo homebrews (for example ffeast) rely on this.
        self.regdata[0x08] = 0xDF;
        self.regdata[0x09] = 0xDF;
        self.regdata[0x0A] = 0xDF;
        self.regdata[0x0B] = 0xDF;
        self.regdata[0x0C] = 0xDF;
        self.regdata[0x0D] = 0xDF;
    }

    pub(crate) fn write(&mut self, index: u32, data: u8) {
        self.regdata[index as usize] = data;
    }

    pub(crate) fn total_level(&self) -> u32 {
        bitfield(self.regdata[0x01] as u32, 0, 6)
    }

    pub(crate) fn ch_pan_left(&self, choffs: u32) -> u32 {
        bit(self.regdata[(choffs + 0x08) as usize] as u32, 7)
    }

    pub(crate) fn ch_pan_right(&self, choffs: u32) -> u32 {
        bit(self.regdata[(choffs + 0x08) as usize] as u32, 6)
    }

    pub(crate) fn ch_instrument_level(&self, choffs: u32) -> u32 {
        bitfield(self.regdata[(choffs + 0x08) as usize] as u32, 0, 5)
    }

    pub(crate) fn ch_start(&self, choffs: u32) -> u32 {
        self.regdata[(choffs + 0x10) as usize] as u32
            | ((self.regdata[(choffs + 0x18) as usize] as u32) << 8)
    }

    pub(crate) fn ch_end(&self, choffs: u32) -> u32 {
        self.regdata[(choffs + 0x20) as usize] as u32
            | ((self.regdata[(choffs + 0x28) as usize] as u32) << 8)
    }

    pub(crate) fn write_start(&mut self, choffs: u32, address: u32) {
        self.write(choffs + 0x10, address as u8);
        self.write(choffs + 0x18, (address >> 8) as u8);
    }

    pub(crate) fn write_end(&mut self, choffs: u32, address: u32) {
        self.write(choffs + 0x20, address as u8);
        self.write(choffs + 0x28, (address >> 8) as u8);
    }
}

pub(crate) struct AdpcmAChannel {
    choffs: u32,
    address_shift: u32,
    playing: u32,
    curnibble: u32,
    curbyte: u32,
    curaddress: u32,
    accumulator: i32,
    step_index: i32,
}

impl AdpcmAChannel {
    pub(crate) fn new(choffs: u32, addrshift: u32) -> Self {
        Self {
            choffs,
            address_shift: addrshift,
            playing: 0,
            curnibble: 0,
            curbyte: 0,
            curaddress: 0,
            accumulator: 0,
            step_index: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.playing = 0;
        self.curnibble = 0;
        self.curbyte = 0;
        self.curaddress = 0;
        self.accumulator = 0;
        self.step_index = 0;
    }

    pub(crate) fn keyonoff(&mut self, on: bool, regs: &AdpcmARegisters) {
        // QUESTION: repeated key ons restart the sample?
        self.playing = on as u32;
        if self.playing != 0 {
            self.curaddress = regs.ch_start(self.choffs) << self.address_shift;
            self.curnibble = 0;
            self.curbyte = 0;
            self.accumulator = 0;
            self.step_index = 0;
        }
    }

    pub(crate) fn clock(&mut self, regs: &AdpcmARegisters, memory: &[u8]) -> bool {
        if self.playing == 0 {
            self.accumulator = 0;
            return false;
        }

        // If we're about to read nibble 0, fetch the data.
        let data;
        if self.curnibble == 0 {
            // Stop when we hit the end address; apparently only low 20 bits are used for
            // comparison on the YM2610: this affects sample playback in some games, for
            // example twinspri character select screen music will skip some samples if
            // this is not correct.
            //
            // Note also: end address is inclusive, so wait until we are about to fetch
            // the sample just after the end before stopping; this is needed for nitd's
            // jump sound, for example.
            let end = (regs.ch_end(self.choffs).wrapping_add(1)) << self.address_shift;
            if (self.curaddress ^ end) & 0xFFFFF == 0 {
                self.playing = 0;
                self.accumulator = 0;
                return true;
            }

            self.curbyte = read_memory(memory, self.curaddress) as u32;
            self.curaddress += 1;
            data = self.curbyte >> 4;
            self.curnibble = 1;
        } else {
            data = self.curbyte & 0xF;
            self.curnibble = 0;
        }

        // Compute the ADPCM delta.
        let delta = (2 * bitfield(data, 0, 3) as i32 + 1)
            * ADPCM_A_STEPS[self.step_index as usize] as i32
            / 8;
        let delta = if bit(data, 3) != 0 { -delta } else { delta };

        // The 12-bit accumulator wraps on the YM2610 and YM2608 (like the MSM5205).
        self.accumulator = (self.accumulator + delta) & 0xFFF;

        self.step_index = clamp(
            self.step_index + ADPCM_A_STEP_INC[bitfield(data, 0, 3) as usize] as i32,
            0,
            48,
        );

        false
    }

    pub(crate) fn output<const N: usize>(&self, output: &mut [i32; N], regs: &AdpcmARegisters) {
        // Volume combines instrument and total levels.
        let vol = (regs.ch_instrument_level(self.choffs) ^ 0x1F) + (regs.total_level() ^ 0x3F);

        // If combined is maximum, don't add to outputs.
        if vol >= 63 {
            return;
        }

        // Convert into a shift and a multiplier.
        // QUESTION: verify this from other sources.
        let mul: i8 = 15 - (vol & 7) as i8;
        let shift: u8 = 4 + 1 + (vol >> 3) as u8;

        // accumulator is a 12-bit value; shift up to sign-extend;
        // the downshift is incorporated into 'shift'.
        let value = (((self.accumulator << 4) as i16 as i32 * mul as i32) >> shift) & !3;
        let value = value as i16;

        if N == 1 || regs.ch_pan_left(self.choffs) != 0 {
            output[0] += value as i32;
        }
        if N > 1 && regs.ch_pan_right(self.choffs) != 0 {
            output[1] += value as i32;
        }
    }
}

pub(crate) struct AdpcmAEngine {
    channels: [AdpcmAChannel; 6],
    regs: AdpcmARegisters,
}

impl AdpcmAEngine {
    pub(crate) fn new(addrshift: u32) -> Self {
        Self {
            channels: core::array::from_fn(|i| AdpcmAChannel::new(i as u32, addrshift)),
            regs: AdpcmARegisters::new(),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.regs.reset();
        for ch in &mut self.channels {
            ch.reset();
        }
    }

    pub(crate) fn clock(&mut self, chanmask: u32, memory: &[u8]) -> u32 {
        assert!(!memory.is_empty(), "ADPCM-A memory must not be empty");
        let mut result = 0u32;
        for chnum in 0..6 {
            if bit(chanmask, chnum as i32) != 0 && self.channels[chnum].clock(&self.regs, memory) {
                result |= 1 << chnum;
            }
        }
        result
    }

    pub(crate) fn output<const N: usize>(&self, output: &mut [i32; N], chanmask: u32) {
        for chnum in 0..6 {
            if bit(chanmask, chnum as i32) != 0 {
                self.channels[chnum].output(output, &self.regs);
            }
        }
    }

    pub(crate) fn write(&mut self, regnum: u32, data: u8) {
        // Store the raw value to the register array;
        // most writes are passive, consumed only when needed.
        self.regs.write(regnum, data);

        // Actively handle writes to the control register.
        if regnum == 0x00 {
            for chnum in 0..6 {
                if bit(data as u32, chnum as i32) != 0 {
                    self.channels[chnum].keyonoff(bit(!data as u32, 7) != 0, &self.regs);
                }
            }
        }
    }

    pub(crate) fn set_start_end(&mut self, chnum: u8, start: u16, end: u16) {
        let choffs = chnum as u32;
        self.regs.write_start(choffs, start as u32);
        self.regs.write_end(choffs, end as u32);
    }
}

// ADPCM-B register map:
//
//      System-wide registers:
//           00 x------- Start of synthesis/analysis
//              -x------ Record
//              --x----- External/manual driving
//              ---x---- Repeat playback
//              ----x--- Speaker off
//              -------x Reset
//           01 x------- Pan left
//              -x------ Pan right
//              ----x--- Start conversion
//              -----x-- DAC enable
//              ------x- DRAM access (1=8-bit granularity; 0=1-bit)
//              -------x RAM/ROM (1=ROM, 0=RAM)
//           02 xxxxxxxx Start address (low)
//           03 xxxxxxxx Start address (high)
//           04 xxxxxxxx End address (low)
//           05 xxxxxxxx End address (high)
//           06 xxxxxxxx Prescale value (low)
//           07 -----xxx Prescale value (high)
//           08 xxxxxxxx CPU data/buffer
//           09 xxxxxxxx Delta-N frequency scale (low)
//           0a xxxxxxxx Delta-N frequency scale (high)
//           0b xxxxxxxx Level control
//           0c xxxxxxxx Limit address (low)
//           0d xxxxxxxx Limit address (high)
//           0e xxxxxxxx DAC data [YM2608/10]
//           0f xxxxxxxx PCM data [YM2608/10]
//           0e xxxxxxxx DAC data high [Y8950]
//           0f xx------ DAC data low [Y8950]
//           10 -----xxx DAC data exponent [Y8950]
pub(crate) struct AdpcmBRegisters {
    regdata: [u8; 0x11],
}

impl AdpcmBRegisters {
    pub(crate) fn new() -> Self {
        Self { regdata: [0; 0x11] }
    }

    pub(crate) fn reset(&mut self) {
        self.regdata.fill(0);
        // Default limit to wide open.
        self.regdata[0x0C] = 0xFF;
        self.regdata[0x0D] = 0xFF;
    }

    pub(crate) fn write(&mut self, index: u32, data: u8) {
        self.regdata[index as usize] = data;
    }

    pub(crate) fn execute(&self) -> u32 {
        bit(self.regdata[0x00] as u32, 7)
    }

    pub(crate) fn record(&self) -> u32 {
        bit(self.regdata[0x00] as u32, 6)
    }

    pub(crate) fn external(&self) -> u32 {
        bit(self.regdata[0x00] as u32, 5)
    }

    pub(crate) fn repeat(&self) -> u32 {
        bit(self.regdata[0x00] as u32, 4)
    }

    pub(crate) fn resetflag(&self) -> u32 {
        bit(self.regdata[0x00] as u32, 0)
    }

    pub(crate) fn pan_left(&self) -> u32 {
        bit(self.regdata[0x01] as u32, 7)
    }

    pub(crate) fn pan_right(&self) -> u32 {
        bit(self.regdata[0x01] as u32, 6)
    }

    pub(crate) fn dram_8bit(&self) -> u32 {
        bit(self.regdata[0x01] as u32, 1)
    }
    pub(crate) fn rom_ram(&self) -> u32 {
        bit(self.regdata[0x01] as u32, 0)
    }

    pub(crate) fn start(&self) -> u32 {
        self.regdata[0x02] as u32 | ((self.regdata[0x03] as u32) << 8)
    }

    pub(crate) fn end(&self) -> u32 {
        self.regdata[0x04] as u32 | ((self.regdata[0x05] as u32) << 8)
    }

    pub(crate) fn cpudata(&self) -> u32 {
        self.regdata[0x08] as u32
    }

    pub(crate) fn delta_n(&self) -> u32 {
        self.regdata[0x09] as u32 | ((self.regdata[0x0A] as u32) << 8)
    }

    pub(crate) fn level(&self) -> u32 {
        self.regdata[0x0B] as u32
    }

    pub(crate) fn limit(&self) -> u32 {
        self.regdata[0x0C] as u32 | ((self.regdata[0x0D] as u32) << 8)
    }
}

pub(crate) struct AdpcmBChannel {
    address_shift: u32,
    status: u32,
    buffer: u32,
    nibbles: u32,
    position: u32,
    curaddress: u32,
    accumulator: i32,
    output: i32,
    prev_output: i32,
    adpcm_step: i32,
}

impl AdpcmBChannel {
    const STEP_MIN: i32 = 127;
    const STEP_MAX: i32 = 24576;
    const LATCH_ADDRESS: u32 = 0xFFFFFFFF;

    pub(crate) const STATUS_EOS: u32 = 0x01;
    pub(crate) const STATUS_BRDY: u32 = 0x02;
    pub(crate) const STATUS_PLAYING: u32 = 0x04;

    const STATUS_EXTERNAL: u32 = Self::STATUS_EOS | Self::STATUS_BRDY | Self::STATUS_PLAYING;
    const STATUS_INTERNAL_DRAIN: u32 = 0x08;
    const STATUS_INTERNAL_PLAYING: u32 = 0x10;
    const STATUS_INTERNAL_SUPPRESS_WRITE: u32 = 0x20;

    pub(crate) fn new(addrshift: u32) -> Self {
        Self {
            address_shift: addrshift,
            status: Self::STATUS_BRDY,
            buffer: 0,
            nibbles: 0,
            position: 0,
            curaddress: 0,
            accumulator: 0,
            output: 0,
            prev_output: 0,
            adpcm_step: Self::STEP_MIN,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.status = Self::STATUS_BRDY;
        self.buffer = 0;
        self.nibbles = 0;
        self.position = 0;
        self.curaddress = 0;
        self.accumulator = 0;
        self.output = 0;
        self.prev_output = 0;
        self.adpcm_step = Self::STEP_MIN;
    }

    pub(crate) fn status(&self) -> u8 {
        (self.status & Self::STATUS_EXTERNAL) as u8
    }

    pub(crate) fn clear_status(&mut self, status: u32) {
        self.status &= !(status & Self::STATUS_EXTERNAL);
    }

    fn set_reset_status(&mut self, set: u32, reset: u32) {
        self.status = (self.status & !reset) | set;
    }

    fn address_shift(&self, regs: &AdpcmBRegisters) -> u32 {
        // If a constant address shift, just provide that.
        if self.address_shift != 0 {
            return self.address_shift;
        }
        // If ROM or 8-bit DRAM, shift is 5 bits.
        if regs.rom_ram() != 0 {
            return 5;
        }
        if regs.dram_8bit() != 0 {
            return 5;
        }
        // Otherwise, shift is 2 bits.
        2
    }

    fn advance_address(&mut self, regs: &AdpcmBRegisters) -> bool {
        let shift = self.address_shift(regs);
        let mask = (1 << shift) - 1;

        debug_assert!(self.curaddress != Self::LATCH_ADDRESS);

        if (self.curaddress & mask) == mask {
            let unitaddr = self.curaddress >> shift;
            if unitaddr == regs.end() {
                return true;
            } else if unitaddr == regs.limit() {
                self.curaddress = 0;
                return false;
            }
        }

        self.curaddress = (self.curaddress + 1) & 0xFFFFFF;
        false
    }

    fn latch_addresses(&mut self, regs: &AdpcmBRegisters) {
        self.curaddress = if regs.external() != 0 {
            regs.start() << self.address_shift(regs)
        } else {
            0
        };
    }

    fn append_buffer_byte(&mut self, data: u8) {
        self.buffer |= (data as u32) << (24 - 4 * self.nibbles);
        self.nibbles += 2;
    }

    fn consume_nibbles(&mut self, count: u32) -> u32 {
        let result = self.buffer >> (32 - 4 * count);
        self.buffer <<= 4 * count;
        self.nibbles = self.nibbles.saturating_sub(count);
        result
    }

    fn request_data(&mut self, regs: &mut AdpcmBRegisters, memory: &mut Option<&mut [u8]>) -> bool {
        if self.curaddress == Self::LATCH_ADDRESS {
            self.latch_addresses(regs);
        }

        if regs.external() == 0 {
            if (self.status & Self::STATUS_BRDY) == 0 {
                self.append_buffer_byte(regs.cpudata() as u8);
            }
            self.set_reset_status(Self::STATUS_BRDY, 0);
            return false;
        }

        let data = read_optional_memory(memory, self.curaddress);
        self.append_buffer_byte(data);
        regs.write(0x08, data);

        self.advance_address(regs)
    }

    fn read_ram(&mut self, regs: &mut AdpcmBRegisters, memory: &mut Option<&mut [u8]>) -> u8 {
        if self.curaddress == Self::LATCH_ADDRESS {
            self.set_reset_status(0, Self::STATUS_INTERNAL_DRAIN);
            while self.nibbles < 4 {
                self.append_buffer_byte(regs.cpudata() as u8);
            }
        }

        let result = self.consume_nibbles(2) as u8;
        self.set_reset_status(Self::STATUS_BRDY, 0);

        if (self.status & Self::STATUS_INTERNAL_DRAIN) != 0 {
            if self.nibbles == 0 {
                self.set_reset_status(Self::STATUS_EOS, Self::STATUS_INTERNAL_DRAIN);

                if regs.repeat() != 0 {
                    self.append_buffer_byte(regs.cpudata() as u8);
                    self.curaddress = regs.start() << self.address_shift(regs);
                    self.request_data(regs, memory);
                } else {
                    self.curaddress = Self::LATCH_ADDRESS;
                }
            }
        } else if self.request_data(regs, memory) {
            self.set_reset_status(Self::STATUS_INTERNAL_DRAIN, 0);
        }

        result
    }

    fn write_ram(&mut self, value: u8, regs: &mut AdpcmBRegisters, memory: &mut Option<&mut [u8]>) {
        if (self.status & Self::STATUS_INTERNAL_SUPPRESS_WRITE) == 0 {
            if self.curaddress == Self::LATCH_ADDRESS {
                self.latch_addresses(regs);
            }

            write_optional_memory(memory, self.curaddress, value);
            self.set_reset_status(Self::STATUS_BRDY, 0);

            if self.advance_address(regs) {
                self.set_reset_status(Self::STATUS_EOS, 0);
                self.curaddress = Self::LATCH_ADDRESS;

                if regs.repeat() != 0 {
                    self.set_reset_status(Self::STATUS_INTERNAL_SUPPRESS_WRITE, 0);
                }
            }
        }

        if (self.status & Self::STATUS_INTERNAL_SUPPRESS_WRITE) != 0 {
            self.buffer = (value as u32) << 16;
            self.nibbles = 4;
            self.read_ram(regs, memory);
        }
    }

    pub(crate) fn clock(&mut self, regs: &mut AdpcmBRegisters, memory: &mut Option<&mut [u8]>) {
        // Only process if active and not recording (which we don't support).
        if regs.execute() == 0
            || regs.record() != 0
            || (self.status & Self::STATUS_INTERNAL_PLAYING) == 0
        {
            self.prev_output = self.output;
            self.position = 0;
            self.set_reset_status(0, Self::STATUS_INTERNAL_PLAYING);
            return;
        }

        // Advance the step using delta-N position advancement.
        let position = self.position.wrapping_add(regs.delta_n());
        self.position = position as u16 as u32;
        if position < 0x10000 {
            return;
        }

        // If we have nibbles available, process them.
        if self.nibbles != 0 {
            let data = self.consume_nibbles(1);

            // Forecast to next forecast: 1/8, 3/8, 5/8, 7/8, 9/8, 11/8, 13/8, 15/8
            let delta = (2 * bitfield(data, 0, 3) as i32 + 1) * self.adpcm_step / 8;
            let delta = if bitfield(data, 3, 1) != 0 {
                -delta
            } else {
                delta
            };

            // Add and clamp to 16 bits.
            self.accumulator = clamp(self.accumulator + delta, -32768, 32767);

            // Scale the ADPCM step: 0.9, 0.9, 0.9, 0.9, 1.2, 1.6, 2.0, 2.4
            self.adpcm_step = clamp(
                (self.adpcm_step * ADPCM_B_STEP_SCALE[bitfield(data, 0, 3) as usize] as i32) / 64,
                Self::STEP_MIN,
                Self::STEP_MAX,
            );

            // Make the new output equal to the accumulator.
            self.prev_output = self.output;
            self.output = self.accumulator;

            // If we've drained all the nibbles, that means we're at a repeat point or end of sample.
            if self.nibbles == 0 {
                // Reset the ADPCM state (but leave output alone).
                self.accumulator = 0;
                self.adpcm_step = Self::STEP_MIN;

                // Always set EOS bit, even if repeating.
                self.set_reset_status(Self::STATUS_EOS, 0);

                // Clear playing flag if we're not repeating.
                if regs.repeat() == 0 {
                    self.set_reset_status(0, Self::STATUS_INTERNAL_PLAYING);
                }
            }
        }

        // If we don't have at least 3 nibbles in the buffer, request more data.
        if (self.status & Self::STATUS_INTERNAL_PLAYING) != 0
            && self.nibbles < 3
            && self.request_data(regs, memory)
        {
            // The final 3 samples are not played; chop them from the stream.
            self.consume_nibbles(3);

            // This should always end up as 1; the logic above assumes we will hit
            // 0 nibbles after processing the next one.
            debug_assert!(self.nibbles == 1);

            // If repeating, set the current address back to start for next fetch.
            if regs.repeat() != 0 {
                self.latch_addresses(regs);
            }
        }
    }

    pub(crate) fn output<const N: usize>(
        &self,
        output: &mut [i32; N],
        regs: &AdpcmBRegisters,
        rshift: u32,
    ) {
        // Do a linear interpolation between samples.
        let result = (self.prev_output as i64 * ((self.position ^ 0xFFFF) + 1) as i64
            + self.output as i64 * self.position as i64)
            >> 16;
        // Apply volume (level) in a linear fashion and reduce.
        let result = ((result as i32) * regs.level() as i32) >> (8 + rshift);

        if N == 1 || regs.pan_left() != 0 {
            output[0] += result;
        }
        if N > 1 && regs.pan_right() != 0 {
            output[1] += result;
        }
    }

    // Register 8 reads over the bus under some conditions.
    pub(crate) fn read(
        &mut self,
        regnum: u32,
        regs: &mut AdpcmBRegisters,
        memory: &mut Option<&mut [u8]>,
    ) -> u8 {
        let mut result = 0u8;
        if regnum == 0x08 && regs.execute() == 0 && regs.record() == 0 && regs.external() != 0 {
            result = self.read_ram(regs, memory);
        }
        result
    }

    pub(crate) fn write_reg(
        &mut self,
        regnum: u32,
        value: u8,
        regs: &mut AdpcmBRegisters,
        memory: &mut Option<&mut [u8]>,
    ) {
        if regnum == 0x00 {
            // Reset flag stops playback and holds output, but does not clear the
            // externally-visible playing flag.
            if regs.resetflag() != 0 {
                self.set_reset_status(
                    Self::STATUS_BRDY
                        | if (self.status & Self::STATUS_INTERNAL_PLAYING) != 0 {
                            Self::STATUS_EOS
                        } else {
                            0
                        },
                    Self::STATUS_INTERNAL_PLAYING,
                );
            } else {
                // All other modes set up for an operation.
                self.set_reset_status(
                    Self::STATUS_BRDY,
                    Self::STATUS_PLAYING
                        | Self::STATUS_INTERNAL_DRAIN
                        | Self::STATUS_INTERNAL_PLAYING
                        | Self::STATUS_INTERNAL_SUPPRESS_WRITE,
                );

                // Flag the address to be latched at the next access.
                self.curaddress = Self::LATCH_ADDRESS;

                // If playing, set the playing status.
                if regs.execute() != 0 {
                    self.buffer = 0;
                    self.nibbles = 0;
                    self.position = 0;
                    self.accumulator = 0;
                    self.adpcm_step = Self::STEP_MIN;
                    self.output = 0;

                    self.set_reset_status(
                        Self::STATUS_PLAYING | Self::STATUS_INTERNAL_PLAYING,
                        Self::STATUS_EOS,
                    );
                }
            }
        } else if regnum == 0x08 {
            // Writing during execute.
            if regs.execute() != 0 {
                // If writing from the CPU during execute, clear the ready flag; data will be
                // picked up on next fetch.
                if regs.record() == 0 && regs.external() == 0 {
                    self.set_reset_status(0, Self::STATUS_BRDY);
                }
            // If writing to external data, process record mode, which writes data to RAM.
            } else if regs.external() != 0 && regs.record() != 0 {
                self.write_ram(value, regs, memory);
            // Writes in external non-record mode appear to behave like a read.
            } else {
                self.read_ram(regs, memory);
            }
        }
    }
}

pub(crate) struct AdpcmBEngine {
    channel: AdpcmBChannel,
    regs: AdpcmBRegisters,
}

impl AdpcmBEngine {
    pub(crate) fn new(addrshift: u32) -> Self {
        Self {
            channel: AdpcmBChannel::new(addrshift),
            regs: AdpcmBRegisters::new(),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.regs.reset();
        self.channel.reset();
    }

    pub(crate) fn clock(&mut self, memory: Option<&mut [u8]>) {
        assert_optional_memory(memory.as_ref());
        let mut memory = memory;
        self.channel.clock(&mut self.regs, &mut memory);
    }

    pub(crate) fn output<const N: usize>(&self, output: &mut [i32; N], rshift: u32) {
        self.channel.output(output, &self.regs, rshift);
    }

    pub(crate) fn read(&mut self, regnum: u32, memory: Option<&mut [u8]>) -> u32 {
        assert_optional_memory(memory.as_ref());
        let mut memory = memory;
        self.channel.read(regnum, &mut self.regs, &mut memory) as u32
    }

    pub(crate) fn write(&mut self, regnum: u32, data: u8, memory: Option<&mut [u8]>) {
        assert_optional_memory(memory.as_ref());
        let mut memory = memory;
        self.regs.write(regnum, data);
        self.channel
            .write_reg(regnum, data, &mut self.regs, &mut memory);
    }

    pub(crate) fn status(&self) -> u8 {
        self.channel.status()
    }

    pub(crate) fn clear_status(&mut self, status: u32) {
        self.channel.clear_status(status);
    }
}

fn assert_optional_memory(memory: Option<&&mut [u8]>) {
    if let Some(memory) = memory {
        assert!(!memory.is_empty(), "ADPCM-B memory must not be empty");
    }
}
