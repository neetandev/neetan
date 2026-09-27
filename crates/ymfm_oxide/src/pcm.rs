use crate::{
    fm::EnvelopeState,
    helpers::{bit, bitfield, clamp},
    tables::{attenuation_increment, attenuation_to_volume},
};

// PCM register map:
//
//      System-wide registers:
//        00-01 xxxxxxxx LSI Test
//           02 -------x Memory access mode (0=sound gen, 1=read/write)
//              ------x- Memory type (0=ROM, 1=ROM+SRAM)
//              ---xxx-- Wave table header
//              xxx----- Device ID (=1 for YMF278B)
//           03 --xxxxxx Memory address high
//           04 xxxxxxxx Memory address mid
//           05 xxxxxxxx Memory address low
//           06 xxxxxxxx Memory data
//           F8 --xxx--- Mix control (FM_R)
//              -----xxx Mix control (FM_L)
//           F9 --xxx--- Mix control (PCM_R)
//              -----xxx Mix control (PCM_L)
//
//      Channel-specific registers:
//        08-1F xxxxxxxx Wave table number low
//        20-37 -------x Wave table number high
//              xxxxxxx- F-number low
//        38-4F -----xxx F-number high
//              ----x--- Pseudo-reverb
//              xxxx---- Octave
//        50-67 xxxxxxx- Total level
//              -------x Level direct
//        68-7F x------- Key on
//              -x------ Damp
//              --x----- LFO reset
//              ---x---- Output channel
//              ----xxxx Panpot
//        80-97 --xxx--- LFO speed
//              -----xxx Vibrato
//        98-AF xxxx---- Attack rate
//              ----xxxx Decay rate
//        B0-C7 xxxx---- Sustain level
//              ----xxxx Sustain rate
//        C8-DF xxxx---- Rate correction
//              ----xxxx Release rate
//        E0-F7 -----xxx AM depth

/// Number of PCM outputs: two stereo pairs.
pub(crate) const PCM_OUTPUTS: usize = 4;
/// Number of PCM channels.
pub(crate) const PCM_CHANNELS: usize = 24;
/// Mask that selects every PCM channel.
pub(crate) const PCM_ALL_CHANNELS: u32 = (1 << PCM_CHANNELS) - 1;
/// Number of PCM registers.
const PCM_REGISTERS: usize = 0x100;
/// Mask of the 22 wave memory address lines.
const PCM_ADDRESS_MASK: u32 = 0x3F_FFFF;

/// Key state bit for a channel that is keyed on.
const KEY_ON: u8 = 0x01;
/// Key state bit for a pending key on.
const KEY_PENDING_ON: u8 = 0x02;
/// Key state bit for a pending key change.
const KEY_PENDING: u8 = 0x04;

/// Envelope attenuation above which a channel produces no output.
const EG_QUIET: u32 = 0x200;

/// Samples between two prepare sweeps without register writes.
const PREPARE_INTERVAL: u32 = 4096;

/// LFO step per sample for each LFO speed, added to an x.18 counter.
const LFO_STEPS: [u8; 8] = [1, 12, 19, 25, 31, 35, 37, 42];
/// AM LFO depth for each AM depth setting.
const AM_DEPTHS: [u8; 8] = [0, 0x14, 0x20, 0x28, 0x30, 0x40, 0x50, 0x80];
/// PM LFO depth in F-number units for each vibrato setting.
const PM_DEPTHS: [u8; 8] = [0, 2, 3, 4, 6, 12, 24, 48];

/// Wave memory of a YMF278B: ROM mapped from address 0 and RAM mapped at `ram_base`.
///
/// RAM takes priority over ROM where both overlap. Unmapped reads return 0 and
/// writes outside RAM are ignored.
pub(crate) struct PcmMemory<'a> {
    pub(crate) rom: &'a [u8],
    pub(crate) ram: &'a mut [u8],
    pub(crate) ram_base: u32,
}

impl PcmMemory<'_> {
    /// Reads the byte at `address`.
    pub(crate) fn read(&self, address: u32) -> u8 {
        let address = address & PCM_ADDRESS_MASK;
        let ram_offset = address.wrapping_sub(self.ram_base) as usize;
        if let Some(value) = self.ram.get(ram_offset) {
            return *value;
        }
        self.rom.get(address as usize).copied().unwrap_or(0)
    }

    /// Writes `data` to `address` when RAM is mapped there.
    pub(crate) fn write(&mut self, address: u32, data: u8) {
        let address = address & PCM_ADDRESS_MASK;
        let ram_offset = address.wrapping_sub(self.ram_base) as usize;
        if let Some(value) = self.ram.get_mut(ram_offset) {
            *value = data;
        }
    }
}

save_state::runtime_state! {
/// Channel values computed from the registers at prepare time.
#[derive(Clone)]
pub(crate) struct PcmCache {
    step: u32,
    total_level: u32,
    pan_left: u32,
    pan_right: u32,
    eg_sustain: u32,
    eg_rate: [u8; EnvelopeState::STATES],
    lfo_step: u8,
    am_depth: u8,
    pm_depth: u8,
}}

impl PcmCache {
    fn new() -> Self {
        Self {
            step: 0,
            total_level: 0,
            pan_left: 0,
            pan_right: 0,
            eg_sustain: 0,
            eg_rate: [0; EnvelopeState::STATES],
            lfo_step: 0,
            am_depth: 0,
            pm_depth: 0,
        }
    }
}

save_state::runtime_state! {
/// PCM register file.
#[derive(Clone)]
pub(crate) struct PcmRegisters {
    regdata: [u8; PCM_REGISTERS],
}}

impl PcmRegisters {
    fn new() -> Self {
        Self {
            regdata: [0; PCM_REGISTERS],
        }
    }

    fn reset(&mut self) {
        self.regdata.fill(0);
        self.regdata[0xF8] = 0x1B;
    }

    fn byte(&self, offset: u32, start: i32, count: i32) -> u32 {
        bitfield(self.regdata[offset as usize] as u32, start, count)
    }

    fn memory_access_mode(&self) -> u32 {
        self.byte(0x02, 0, 1)
    }

    fn wave_table_header(&self) -> u32 {
        self.byte(0x02, 2, 3)
    }

    fn memory_address(&self) -> u32 {
        (self.byte(0x03, 0, 6) << 16) | (self.byte(0x04, 0, 8) << 8) | self.byte(0x05, 0, 8)
    }

    /// Returns the FM right mix control.
    pub(crate) fn mix_fm_r(&self) -> u32 {
        self.byte(0xF8, 3, 3)
    }

    /// Returns the FM left mix control.
    pub(crate) fn mix_fm_l(&self) -> u32 {
        self.byte(0xF8, 0, 3)
    }

    /// Returns the PCM right mix control.
    pub(crate) fn mix_pcm_r(&self) -> u32 {
        self.byte(0xF9, 3, 3)
    }

    /// Returns the PCM left mix control.
    pub(crate) fn mix_pcm_l(&self) -> u32 {
        self.byte(0xF9, 0, 3)
    }

    fn ch_wave_table_num(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0x08, 0, 8) | (self.byte(choffs + 0x20, 0, 1) << 8)
    }

    fn ch_fnumber(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0x20, 1, 7) | (self.byte(choffs + 0x38, 0, 3) << 7)
    }

    fn ch_pseudo_reverb(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0x38, 3, 1)
    }

    fn ch_octave(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0x38, 4, 4)
    }

    fn ch_total_level(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0x50, 1, 7)
    }

    fn ch_level_direct(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0x50, 0, 1)
    }

    fn ch_damp(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0x68, 6, 1)
    }

    fn ch_lfo_reset(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0x68, 5, 1)
    }

    fn ch_output_channel(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0x68, 4, 1)
    }

    fn ch_panpot(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0x68, 0, 4)
    }

    fn ch_lfo_speed(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0x80, 3, 3)
    }

    fn ch_vibrato(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0x80, 0, 3)
    }

    fn ch_attack_rate(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0x98, 4, 4)
    }

    fn ch_decay_rate(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0x98, 0, 4)
    }

    fn ch_sustain_level(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0xB0, 4, 4)
    }

    fn ch_sustain_rate(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0xB0, 0, 4)
    }

    fn ch_rate_correction(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0xC8, 4, 4)
    }

    fn ch_release_rate(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0xC8, 0, 4)
    }

    fn ch_am_depth(&self, choffs: u32) -> u32 {
        self.byte(choffs + 0xE0, 0, 3)
    }

    /// Returns the memory address and advances it by one.
    fn memory_address_autoinc(&mut self) -> u32 {
        let result = self.memory_address();
        let next = result.wrapping_add(1);
        self.regdata[0x05] = next as u8;
        self.regdata[0x04] = (next >> 8) as u8;
        self.regdata[0x03] = ((next >> 16) & 0x3F) as u8;
        result
    }

    /// Returns a raw rate combined with the key scaling `correction`.
    fn effective_rate(raw: u32, correction: i32) -> u8 {
        match raw {
            0 => 0,
            15 => 63,
            _ => clamp((raw * 4) as i32 + correction, 0, 63) as u8,
        }
    }

    fn cache_channel_data(&self, choffs: u32, cache: &mut PcmCache) {
        // The step is a .16 value. Octave -8 gives a shift count of -1, which
        // wraps modulo 32.
        let octave = (((self.ch_octave(choffs) << 4) as u8 as i8) >> 4) as i32;
        let fnum = self.ch_fnumber(choffs);
        cache.step = (0x400 | fnum).wrapping_shl((octave + 7) as u32) >> 2;

        cache.total_level = self.ch_total_level(choffs) << 10;

        let panpot = (((self.ch_panpot(choffs) << 4) as u8 as i8) >> 4) as i32;
        if panpot >= 0 {
            cache.pan_left = if panpot == 7 {
                0x3FF
            } else {
                0x20 * panpot as u32
            };
            cache.pan_right = 0;
        } else if panpot >= -7 {
            cache.pan_left = 0;
            cache.pan_right = if panpot == -7 {
                0x3FF
            } else {
                (-0x20 * panpot) as u32
            };
        } else {
            cache.pan_left = 0x3FF;
            cache.pan_right = 0x3FF;
        }

        cache.lfo_step = LFO_STEPS[self.ch_lfo_speed(choffs) as usize];
        cache.am_depth = AM_DEPTHS[self.ch_am_depth(choffs) as usize];
        cache.pm_depth = PM_DEPTHS[self.ch_vibrato(choffs) as usize];

        // 4-bit sustain level, where 15 means 31
        let sustain = self.ch_sustain_level(choffs);
        cache.eg_sustain = (sustain | ((sustain + 1) & 0x10)) << 5;

        // rate correction 15 disables key scaling
        let correction = match self.ch_rate_correction(choffs) {
            15 => 0,
            correction => (octave + correction as i32) * 2 + bit(fnum, 9) as i32,
        };

        cache.eg_rate[EnvelopeState::Attack as usize] =
            Self::effective_rate(self.ch_attack_rate(choffs), correction);
        cache.eg_rate[EnvelopeState::Decay as usize] =
            Self::effective_rate(self.ch_decay_rate(choffs), correction);
        cache.eg_rate[EnvelopeState::Sustain as usize] =
            Self::effective_rate(self.ch_sustain_rate(choffs), correction);
        cache.eg_rate[EnvelopeState::Release as usize] =
            Self::effective_rate(self.ch_release_rate(choffs), correction);
        cache.eg_rate[EnvelopeState::Reverb as usize] = 5;

        // damping decays at rate 48 down to -12dB, then at the maximum rate
        if self.ch_damp(choffs) != 0 {
            cache.eg_rate[EnvelopeState::Decay as usize] = 48;
            cache.eg_rate[EnvelopeState::Sustain as usize] = 63;
            cache.eg_rate[EnvelopeState::Release as usize] = 63;
            cache.eg_sustain = 0x80;
        }
    }
}

save_state::runtime_state! {
/// Playback and envelope state of one PCM channel.
#[derive(Clone)]
pub(crate) struct PcmChannel {
    choffs: u32,
    baseaddr: u32,
    endpos: u32,
    looppos: u32,
    curpos: u32,
    nextpos: u32,
    lfo_counter: u32,
    eg_state: EnvelopeState,
    env_attenuation: u16,
    total_level: u32,
    format: u8,
    key_state: u8,
    cache: PcmCache,
}}

impl PcmChannel {
    fn new(choffs: u32) -> Self {
        Self {
            choffs,
            baseaddr: 0,
            endpos: 0,
            looppos: 0,
            curpos: 0,
            nextpos: 0,
            lfo_counter: 0,
            eg_state: EnvelopeState::Release,
            env_attenuation: 0x3FF,
            total_level: 0x7F << 10,
            format: 0,
            key_state: 0,
            cache: PcmCache::new(),
        }
    }

    fn reset(&mut self) {
        self.baseaddr = 0;
        self.endpos = 0;
        self.looppos = 0;
        self.curpos = 0;
        self.nextpos = 0;
        self.lfo_counter = 0;
        self.eg_state = EnvelopeState::Release;
        self.env_attenuation = 0x3FF;
        self.total_level = 0x7F << 10;
        self.format = 0;
        self.key_state = 0;
    }

    /// Updates the cache and applies a pending key change.
    fn prepare(&mut self, regs: &PcmRegisters) {
        regs.cache_channel_data(self.choffs, &mut self.cache);

        if self.key_state & KEY_PENDING != 0 {
            let old_state = self.key_state;
            self.key_state = (self.key_state >> 1) & KEY_ON;
            if (old_state ^ self.key_state) & KEY_ON != 0 {
                if self.key_state & KEY_ON != 0 {
                    self.start_attack(regs);
                } else {
                    self.start_release();
                }
            }
        }

        if regs.ch_level_direct(self.choffs) != 0 {
            self.total_level = self.cache.total_level;
        }
    }

    fn clock(&mut self, env_counter: u32, regs: &PcmRegisters) {
        // the LFO is an x.18 value
        self.lfo_counter = self.lfo_counter.wrapping_add(self.cache.lfo_step as u32);

        self.clock_envelope(env_counter, regs);

        let mut step = self.cache.step;
        if self.cache.pm_depth != 0 {
            // shift the LFO by 1/4 cycle for PM so that it starts at 0
            let lfo_shifted = self.lfo_counter.wrapping_add(1 << 16);
            let mut lfo_value = bitfield(lfo_shifted, 10, 7) as i32;
            if bit(lfo_shifted, 17) != 0 {
                lfo_value ^= 0x7F;
            }
            lfo_value -= 0x40;
            step = step.wrapping_add(((lfo_value * self.cache.pm_depth as i32) >> 7) as u32);
        }

        self.curpos = self.nextpos;
        self.nextpos = self.curpos.wrapping_add(step);
        if self.nextpos >= self.endpos {
            self.nextpos = self
                .nextpos
                .wrapping_add(self.looppos.wrapping_sub(self.endpos));
        }

        // the level falls by 19/1024 and rises by 38/1024 per sample
        if self.total_level != self.cache.total_level {
            let target = self.cache.total_level as i32;
            self.total_level = if self.total_level < self.cache.total_level {
                (self.total_level as i32 + 19).min(target) as u32
            } else {
                (self.total_level as i32 - 38).max(target) as u32
            };
        }
    }

    fn output(&self, output: &mut [i32; PCM_OUTPUTS], regs: &PcmRegisters, memory: &PcmMemory) {
        let mut envelope = self.env_attenuation as u32;
        if envelope > EG_QUIET {
            return;
        }

        if self.cache.am_depth != 0 {
            let mut lfo_value = bitfield(self.lfo_counter, 10, 7);
            if bit(self.lfo_counter, 17) != 0 {
                lfo_value ^= 0x7F;
            }
            envelope += (lfo_value * self.cache.am_depth as u32) >> 7;
        }

        // the total level is a .10 value
        envelope += self.total_level >> 8;

        let left_envelope = (envelope + self.cache.pan_left).min(0x3FF);
        let right_envelope = (envelope + self.cache.pan_right).min(0x3FF);

        // volumes are .11 fractions
        let left_volume = attenuation_to_volume(left_envelope << 2) as i32;
        let right_volume = attenuation_to_volume(right_envelope << 2) as i32;

        let sample = self.fetch_sample(memory) as i32;
        let outnum = regs.ch_output_channel(self.choffs) as usize * 2;
        output[outnum] += (left_volume * sample) >> 15;
        output[outnum + 1] += (right_volume * sample) >> 15;
    }

    fn keyonoff(&mut self, on: bool) {
        self.key_state |= KEY_PENDING | if on { KEY_PENDING_ON } else { 0 };
    }

    /// Loads the wave table header of the selected wave and returns the five
    /// register values it holds for registers 0x80, 0x98, 0xB0, 0xC8 and 0xE0.
    fn load_wavetable(&mut self, regs: &PcmRegisters, memory: &PcmMemory) -> [u8; 5] {
        let wavnum = regs.ch_wave_table_num(self.choffs);
        let mut wavheader = 12 * wavnum;

        // waves from 384 up can come from another bank
        if wavnum >= 384 {
            let bank = regs.wave_table_header();
            if bank != 0 {
                wavheader = 512 * 1024 * bank + (wavnum - 384) * 12;
            }
        }

        // 22-bit base address and 2-bit format
        let first = memory.read(wavheader) as u32;
        self.format = bitfield(first, 6, 2) as u8;
        self.baseaddr = bitfield(first, 0, 6) << 16;
        self.baseaddr |= (memory.read(wavheader + 1) as u32) << 8;
        self.baseaddr |= memory.read(wavheader + 2) as u32;

        self.looppos = (memory.read(wavheader + 3) as u32) << 8;
        self.looppos |= memory.read(wavheader + 4) as u32;
        self.looppos <<= 16;

        // the end position is stored negated
        self.endpos = (memory.read(wavheader + 5) as u32) << 8;
        self.endpos |= memory.read(wavheader + 6) as u32;
        self.endpos = (self.endpos as i32).wrapping_neg().wrapping_shl(16) as u32;

        let registers = core::array::from_fn(|index| memory.read(wavheader + 7 + index as u32));

        // restart the envelope for the new sample
        self.env_attenuation = 0x3FF;
        registers
    }

    fn start_attack(&mut self, regs: &PcmRegisters) {
        if self.eg_state == EnvelopeState::Attack {
            return;
        }
        self.eg_state = EnvelopeState::Attack;

        if regs.ch_lfo_reset(self.choffs) != 0 {
            self.lfo_counter = 0;
        }

        // attack rate 63 starts at full volume
        if self.cache.eg_rate[EnvelopeState::Attack as usize] == 63 {
            self.env_attenuation = 0;
        }

        self.curpos = 0;
        self.nextpos = 0;
    }

    fn start_release(&mut self) {
        if self.eg_state as u32 >= EnvelopeState::Release as u32 {
            return;
        }
        self.eg_state = EnvelopeState::Release;
    }

    fn clock_envelope(&mut self, env_counter: u32, regs: &PcmRegisters) {
        if self.eg_state == EnvelopeState::Attack && self.env_attenuation == 0 {
            self.eg_state = EnvelopeState::Decay;
        }

        if self.eg_state == EnvelopeState::Decay
            && self.env_attenuation as u32 >= self.cache.eg_sustain
        {
            self.eg_state = EnvelopeState::Sustain;
        }

        let rate = self.cache.eg_rate[self.eg_state as usize] as u32;

        // shift the counter into a 5.11 fixed point value
        let rate_shift = rate >> 2;
        let env_counter = env_counter << rate_shift;

        // clock only when the fractional part is 0
        if bitfield(env_counter, 0, 11) != 0 {
            return;
        }

        let relevant_bits = bitfield(env_counter, rate_shift.max(11) as i32, 3);
        let increment = attenuation_increment(rate, relevant_bits);

        if self.eg_state == EnvelopeState::Attack {
            let attenuation = self.env_attenuation as u32;
            self.env_attenuation =
                attenuation.wrapping_add((!attenuation).wrapping_mul(increment) >> 4) as u16;
        } else {
            self.env_attenuation += increment as u16;

            if self.env_attenuation >= 0x400 {
                self.env_attenuation = 0x3FF;
            }

            // switch to reverb at -18dB when enabled
            if self.env_attenuation >= 0xC0
                && (self.eg_state as u32) < EnvelopeState::Reverb as u32
                && regs.ch_pseudo_reverb(self.choffs) != 0
            {
                self.eg_state = EnvelopeState::Reverb;
            }
        }
    }

    fn fetch_sample(&self, memory: &PcmMemory) -> i16 {
        let mut address = self.baseaddr;
        let position = self.curpos >> 16;

        match self.format {
            // 8-bit samples fill the upper byte
            0 => ((memory.read(address.wrapping_add(position)) as u16) << 8) as i16,
            // 16-bit samples are stored high byte first
            2 => {
                address = address.wrapping_add(position * 2);
                (((memory.read(address) as u16) << 8) | memory.read(address.wrapping_add(1)) as u16)
                    as i16
            }
            // 12-bit samples pack two samples into three bytes
            _ => {
                address = address.wrapping_add((position / 2) * 3);
                let middle = memory.read(address.wrapping_add(1)) as u16;
                if position & 1 == 0 {
                    (((memory.read(address) as u16) << 8) | ((middle << 4) & 0xF0)) as i16
                } else {
                    (((memory.read(address.wrapping_add(2)) as u16) << 8) | (middle & 0xF0)) as i16
                }
            }
        }
    }
}

save_state::runtime_state! {
/// Wavetable PCM engine of the YMF278B.
#[derive(Clone)]
pub(crate) struct PcmEngine {
    env_counter: u32,
    modified_channels: u32,
    prepare_count: u32,
    channels: [PcmChannel; PCM_CHANNELS],
    regs: PcmRegisters,
}}

impl PcmEngine {
    pub(crate) fn new() -> Self {
        Self {
            env_counter: 0,
            modified_channels: PCM_ALL_CHANNELS,
            prepare_count: 0,
            channels: core::array::from_fn(|index| PcmChannel::new(index as u32)),
            regs: PcmRegisters::new(),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.regs.reset();
        for channel in &mut self.channels {
            channel.reset();
        }
    }

    /// Returns the PCM registers.
    pub(crate) fn regs(&self) -> &PcmRegisters {
        &self.regs
    }

    pub(crate) fn clock(&mut self, chanmask: u32) {
        // prepare after writes, and every 4096 samples to catch ending notes
        let sweep = self.modified_channels != 0 || {
            let count = self.prepare_count;
            self.prepare_count = self.prepare_count.wrapping_add(1);
            count >= PREPARE_INTERVAL
        };
        if sweep {
            for (index, channel) in self.channels.iter_mut().enumerate() {
                if bit(chanmask, index as i32) != 0 {
                    channel.prepare(&self.regs);
                }
            }
            self.modified_channels = 0;
            self.prepare_count = 0;
        }

        // the envelope clocks every other sample to line up with the FM envelopes
        self.env_counter = self.env_counter.wrapping_add(1);

        for (index, channel) in self.channels.iter_mut().enumerate() {
            if bit(chanmask, index as i32) != 0 {
                channel.clock(self.env_counter >> 1, &self.regs);
            }
        }
    }

    pub(crate) fn output(
        &self,
        output: &mut [i32; PCM_OUTPUTS],
        chanmask: u32,
        memory: &PcmMemory,
    ) {
        for (index, channel) in self.channels.iter().enumerate() {
            if bit(chanmask, index as i32) != 0 {
                channel.output(output, &self.regs, memory);
            }
        }
    }

    /// Reads PCM register `regnum`. Register 0x06 reads wave memory in memory access mode.
    pub(crate) fn read(&mut self, regnum: u32, memory: &PcmMemory) -> u8 {
        if regnum == 0x06 && self.regs.memory_access_mode() != 0 {
            let address = self.regs.memory_address_autoinc();
            return memory.read(address);
        }
        self.regs.regdata[regnum as usize]
    }

    /// Writes PCM register `regnum`. Register 0x06 writes wave memory in memory access mode.
    pub(crate) fn write(&mut self, regnum: u32, data: u8, memory: &mut PcmMemory) {
        if regnum == 0x06 && self.regs.memory_access_mode() != 0 {
            let address = self.regs.memory_address_autoinc();
            memory.write(address, data);
            return;
        }

        self.modified_channels = PCM_ALL_CHANNELS;
        self.regs.regdata[regnum as usize] = data;

        if (0x68..=0x7F).contains(&regnum) {
            self.channels[(regnum - 0x68) as usize].keyonoff(bit(data as u32, 7) != 0);
        } else if (0x08..=0x1F).contains(&regnum) {
            let choffs = regnum - 0x08;
            let registers = self.channels[choffs as usize].load_wavetable(&self.regs, memory);
            for (base, value) in [0x80, 0x98, 0xB0, 0xC8, 0xE0].into_iter().zip(registers) {
                self.regs.regdata[(base + choffs) as usize] = value;
            }
        }
    }
}
