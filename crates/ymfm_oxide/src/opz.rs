use crate::{
    fm::{FmRegisters, OpdataCache, effective_rate, operator_list},
    helpers::{bit, bitfield},
    opm::{DETUNE2_DELTA, opm_key_code_to_phase_step},
    tables::{abs_sin_attenuation, detune_adjustment},
};

// OPZ register map:
//
//      System-wide registers:
//           08 -----xxx Load preset
//           0F x------- Noise enable
//              ---xxxxx Noise frequency
//           10 xxxxxxxx Timer A value (upper 8 bits)
//           11 ------xx Timer A value (lower 2 bits)
//           12 xxxxxxxx Timer B value
//           14 x------- CSM mode
//              --x----- Reset timer B
//              ---x---- Reset timer A
//              ----x--- Enable timer B
//              -----x-- Enable timer A
//              ------x- Load timer B
//              -------x Load timer A
//           16 xxxxxxxx LFO #2 frequency
//           17 0xxxxxxx AM LFO #2 depth
//              1xxxxxxx PM LFO #2 depth
//           18 xxxxxxxx LFO frequency
//           19 0xxxxxxx AM LFO depth
//              1xxxxxxx PM LFO depth
//           1B xx------ CT (2 output data lines)
//              --x----- LFO #2 sync
//              ---x---- LFO sync
//              ----xx-- LFO #2 waveform
//              ------xx LFO waveform
//
//     Per-channel registers (channel in address bits 0-2)
//        00-07 xxxxxxxx Channel volume
//        20-27 x------- Pan right
//              -x------ Key on (0)/off(1)
//              --xxx--- Feedback level for operator 1 (0-7)
//              -----xxx Operator connection algorithm (0-7)
//        28-2F -xxxxxxx Key code
//        30-37 xxxxxx-- Key fraction
//              -------x Mono mode
//        38-3F 0xxx---- LFO PM sensitivity
//              -----0xx LFO AM shift
//              1xxx---- LFO #2 PM sensitivity
//              -----1xx LFO #2 AM shift
//
//     Per-operator registers (channel in address bits 0-2, operator in bits 3-4)
//        40-5F 0xxx---- Detune value (0-7)
//              0---xxxx Multiple value (0-15)
//              0xxx---- Fix range (0-15)
//              0---xxxx Fix frequency (0-15)
//              1xxx---- Oscillator waveform (0-7)
//              1---xxxx Fine (0-15)
//        60-7F -xxxxxxx Total level (0-127)
//        80-9F xx------ Key scale rate (0-3)
//              --x----- Fix frequency mode
//              ---xxxxx Attack rate (0-31)
//        A0-BF x------- LFO AM enable
//              ---xxxxx Decay rate (0-31)
//        C0-DF xx0----- Detune 2 value (0-3)
//              --0xxxxx Sustain rate (0-31)
//              xx1----- Envelope generator shift (0-3)
//              --1--xxx Reverb rate (0-7)
//        E0-FF xxxx---- Sustain level (0-15)
//              ----xxxx Release rate (0-15)
//
//     Internal (fake) registers:
//      100-11F -xxx---- Oscillator waveform (0-7)
//              ----xxxx Fine (0-15)
//      120-13F xx------ Envelope generator shift (0-3)
//              -----xxx Reverb rate (0-7)
//      140-15F xxxx---- Preset sustain level (0-15)
//              ----xxxx Preset release rate (0-15)
//      160-17F xx------ Envelope generator shift (0-3)
//              -----xxx Reverb rate (0-7)
//      180-187 -xxx---- LFO #2 PM sensitivity
//              ---- xxx LFO #2 AM shift
//          188 -xxxxxxx LFO #2 PM depth
//          189 -xxxxxxx LFO PM depth

const WAVEFORM_LENGTH: usize = 0x400;
const WAVEFORMS: usize = 8;
const LFO_WAVEFORM_LENGTH: usize = 256;
const REGISTERS: usize = 0x190;
const OPERATORS: usize = 32;

// Envelope state indices into OpdataCache::eg_rate.
const EG_ATTACK: usize = 1;
const EG_DECAY: usize = 2;
const EG_SUSTAIN: usize = 3;
const EG_RELEASE: usize = 4;
const EG_REVERB: usize = 5;

save_state::runtime_state! {
/// Authoritative OPZ register, LFO, noise, and fixed frequency state.
#[derive(Clone)]
pub(crate) struct OpzRegisters {
    lfo_counter: [u32; 2],
    noise_lfsr: u32,
    noise_counter: u8,
    noise_state: u8,
    lfo_am: [u8; 2],
    regdata: [u8; REGISTERS],
    // Fixed frequency phase substeps with 12 bits of extra resolution.
    phase_substep: [u16; OPERATORS],
    // LFO waveforms; AM in the low 8 bits, PM in the upper 8.
    lfo_waveform: [[i16; LFO_WAVEFORM_LENGTH]; 4],
    waveform: [[u16; WAVEFORM_LENGTH]; WAVEFORMS],
}}

impl OpzRegisters {
    fn byte(&self, offset: u32, start: i32, count: i32, extra_offset: u32) -> u32 {
        bitfield(
            self.regdata[(offset + extra_offset) as usize] as u32,
            start,
            count,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn word(
        &self,
        offset1: u32,
        start1: i32,
        count1: i32,
        offset2: u32,
        start2: i32,
        count2: i32,
        extra_offset: u32,
    ) -> u32 {
        (self.byte(offset1, start1, count1, extra_offset) << count2)
            | self.byte(offset2, start2, count2, extra_offset)
    }

    // system-wide registers
    fn noise_frequency(&self) -> u32 {
        self.byte(0x0F, 0, 5, 0)
    }

    fn lfo2_pm_depth(&self) -> u32 {
        self.byte(0x188, 0, 7, 0)
    }

    fn lfo2_rate(&self) -> u32 {
        self.byte(0x16, 0, 8, 0)
    }

    fn lfo2_am_depth(&self) -> u32 {
        self.byte(0x17, 0, 7, 0)
    }

    fn lfo_rate(&self) -> u32 {
        self.byte(0x18, 0, 8, 0)
    }

    fn lfo_am_depth(&self) -> u32 {
        self.byte(0x19, 0, 7, 0)
    }

    fn lfo_pm_depth(&self) -> u32 {
        self.byte(0x189, 0, 7, 0)
    }

    fn lfo2_sync(&self) -> u32 {
        self.byte(0x1B, 5, 1, 0)
    }

    fn lfo_sync(&self) -> u32 {
        self.byte(0x1B, 4, 1, 0)
    }

    fn lfo2_waveform(&self) -> u32 {
        self.byte(0x1B, 2, 2, 0)
    }

    fn lfo_waveform(&self) -> u32 {
        self.byte(0x1B, 0, 2, 0)
    }

    // per-channel registers
    fn ch_key_on(&self, choffs: u32) -> u32 {
        self.byte(0x20, 6, 1, choffs)
    }

    fn ch_block_freq(&self, choffs: u32) -> u32 {
        self.word(0x28, 0, 7, 0x30, 2, 6, choffs)
    }

    fn ch_lfo_pm_sens(&self, choffs: u32) -> u32 {
        self.byte(0x38, 4, 3, choffs)
    }

    fn ch_lfo_am_sens(&self, choffs: u32) -> u32 {
        self.byte(0x38, 0, 2, choffs)
    }

    fn ch_lfo2_pm_sens(&self, choffs: u32) -> u32 {
        self.byte(0x180, 4, 3, choffs)
    }

    fn ch_lfo2_am_sens(&self, choffs: u32) -> u32 {
        self.byte(0x180, 0, 2, choffs)
    }

    // per-operator registers
    fn op_detune(&self, opoffs: u32) -> u32 {
        self.byte(0x40, 4, 3, opoffs)
    }

    fn op_multiple(&self, opoffs: u32) -> u32 {
        self.byte(0x40, 0, 4, opoffs)
    }

    fn op_fix_range(&self, opoffs: u32) -> u32 {
        self.byte(0x40, 4, 3, opoffs)
    }

    fn op_fix_frequency(&self, opoffs: u32) -> u32 {
        self.byte(0x40, 0, 4, opoffs)
    }

    fn op_waveform(&self, opoffs: u32) -> u32 {
        self.byte(0x100, 4, 3, opoffs)
    }

    fn op_fine(&self, opoffs: u32) -> u32 {
        self.byte(0x100, 0, 4, opoffs)
    }

    fn op_total_level(&self, opoffs: u32) -> u32 {
        self.byte(0x60, 0, 7, opoffs)
    }

    fn op_ksr(&self, opoffs: u32) -> u32 {
        self.byte(0x80, 6, 2, opoffs)
    }

    fn op_fix_mode(&self, opoffs: u32) -> u32 {
        self.byte(0x80, 5, 1, opoffs)
    }

    fn op_attack_rate(&self, opoffs: u32) -> u32 {
        self.byte(0x80, 0, 5, opoffs)
    }

    fn op_decay_rate(&self, opoffs: u32) -> u32 {
        self.byte(0xA0, 0, 5, opoffs)
    }

    fn op_detune2(&self, opoffs: u32) -> u32 {
        self.byte(0xC0, 6, 2, opoffs)
    }

    fn op_sustain_rate(&self, opoffs: u32) -> u32 {
        self.byte(0xC0, 0, 5, opoffs)
    }

    fn op_eg_shift(&self, opoffs: u32) -> u32 {
        self.byte(0x120, 6, 2, opoffs)
    }

    fn op_reverb_rate(&self, opoffs: u32) -> u32 {
        self.byte(0x120, 0, 3, opoffs)
    }

    fn op_sustain_level(&self, opoffs: u32) -> u32 {
        self.byte(0xE0, 4, 4, opoffs)
    }

    fn op_release_rate(&self, opoffs: u32) -> u32 {
        self.byte(0xE0, 0, 4, opoffs)
    }

    // Phase step outside of fixed frequency mode; follows OPM with a second
    // PM source in the upper 8 bits of the raw PM value.
    fn keyed_phase_step(
        &self,
        choffs: u32,
        opoffs: u32,
        cache: &OpdataCache,
        lfo_raw_pm: i32,
    ) -> u32 {
        // start with coarse detune delta
        let mut delta = DETUNE2_DELTA[self.op_detune2(opoffs) as usize] as i32;

        // add in the PM deltas; raw PM values are -127..128 for +/- 200 cents
        let pm_sensitivity = self.ch_lfo_pm_sens(choffs);
        if pm_sensitivity != 0 {
            let pm = lfo_raw_pm as i8 as i32;
            if pm_sensitivity < 6 {
                delta += pm >> (6 - pm_sensitivity);
            } else {
                delta += pm << (pm_sensitivity - 5);
            }
        }
        let pm_sensitivity2 = self.ch_lfo2_pm_sens(choffs);
        if pm_sensitivity2 != 0 {
            let pm = (lfo_raw_pm >> 8) as i8 as i32;
            if pm_sensitivity2 < 6 {
                delta += pm >> (6 - pm_sensitivity2);
            } else {
                delta += pm << (pm_sensitivity2 - 5);
            }
        }

        // apply delta and convert to a frequency number like OPM
        let phase_step =
            opm_key_code_to_phase_step(cache.block_freq, delta).wrapping_add(cache.detune as u32);

        // apply frequency multiplier (which is cached as an x.4 value)
        phase_step.wrapping_mul(cache.multiple) >> 4
    }
}

impl FmRegisters for OpzRegisters {
    const OUTPUTS: usize = 2;
    const CHANNELS: usize = 8;
    const ALL_CHANNELS: u32 = (1 << 8) - 1;
    const OPERATORS: usize = OPERATORS;
    const DEFAULT_PRESCALE: u32 = 2;
    const EG_CLOCK_DIVIDER: u32 = 3;
    const CSM_TRIGGER_MASK: u32 = (1 << 8) - 1;
    const REG_MODE: u32 = 0x14;
    const EG_HAS_DEPRESS: bool = false;
    const EG_HAS_REVERB: bool = true;
    const EG_HAS_SSG: bool = false;
    const MODULATOR_DELAY: bool = false;
    const DYNAMIC_OPS: bool = false;

    const STATUS_TIMERA: u8 = 0x01;
    const STATUS_TIMERB: u8 = 0x02;
    const STATUS_BUSY: u8 = 0x80;
    const STATUS_IRQ: u8 = 0;

    const RHYTHM_CHANNEL: u32 = 0xFF;

    fn new() -> Self {
        let mut waveform = [[0u16; WAVEFORM_LENGTH]; WAVEFORMS];
        for (index, entry) in waveform[0].iter_mut().enumerate() {
            *entry = (abs_sin_attenuation(index as u32) | (bit(index as u32, 9) << 15)) as u16;
        }

        // waveform 1 is taken as sin^2, which doubles the logarithmic
        // attenuation
        let zero_value = waveform[0][0];
        let sine = waveform[0];
        for (index, entry) in waveform[1].iter_mut().enumerate() {
            *entry = (2 * (sine[index] & 0x7FFF)).min(zero_value)
                | ((bit(index as u32, 9) as u16) << 15);
        }

        // the remaining waveforms derive from the first two; the second half
        // of each holds the zero value
        let sine_squared = waveform[1];
        for index in 0..WAVEFORM_LENGTH {
            if bit(index as u32, 9) != 0 {
                for table in &mut waveform[2..] {
                    table[index] = zero_value;
                }
            } else {
                waveform[2][index] = sine[index];
                waveform[3][index] = sine_squared[index];
                waveform[4][index] = sine[index * 2];
                waveform[5][index] = sine_squared[index * 2];
                waveform[6][index] = sine[(index * 2) & 0x1FF];
                waveform[7][index] = sine_squared[(index * 2) & 0x1FF];
            }
        }

        // create the LFO waveforms; AM in the low 8 bits, PM in the upper 8;
        // waveforms are adjusted to match the pictures in the application manual
        let mut lfo_waveform = [[0i16; LFO_WAVEFORM_LENGTH]; 4];
        for index in 0..LFO_WAVEFORM_LENGTH as u32 {
            // waveform 0 is a sawtooth
            let am = (index ^ 0xFF) as u8;
            let pm = index as u8;
            lfo_waveform[0][index as usize] = (am as u16 | ((pm as u16) << 8)) as i16;

            // waveform 1 is a square wave
            let am = if bit(index, 7) != 0 { 0u8 } else { 0xFF };
            let pm = am ^ 0x80;
            lfo_waveform[1][index as usize] = (am as u16 | ((pm as u16) << 8)) as i16;

            // waveform 2 is a triangle wave
            let am = if bit(index, 7) != 0 {
                (index << 1) as u8
            } else {
                ((index ^ 0xFF) << 1) as u8
            };
            let pm = if bit(index, 6) != 0 { am } else { !am };
            lfo_waveform[2][index as usize] = (am as u16 | ((pm as u16) << 8)) as i16;

            // waveform 3 is noise; it is filled in dynamically
            lfo_waveform[3][index as usize] = 0;
        }

        Self {
            lfo_counter: [0; 2],
            noise_lfsr: 1,
            noise_counter: 0,
            noise_state: 0,
            lfo_am: [0; 2],
            regdata: [0; REGISTERS],
            phase_substep: [0; OPERATORS],
            lfo_waveform,
            waveform,
        }
    }

    fn reset(&mut self) {
        self.regdata.fill(0);
        self.phase_substep.fill(0);
        // enable output on both channels by default
        for offset in 0x30..=0x37 {
            self.regdata[offset] = 0x01;
        }
    }

    // Note that the channel index order is 0,2,1,3, so we bitswap the index.
    //
    // This is because the order in the map is:
    //    carrier 1, carrier 2, modulator 1, modulator 2
    //
    // But when wiring up the connections, the more natural order is:
    //    carrier 1, modulator 1, carrier 2, modulator 2
    fn operator_map(&self, index: usize) -> u32 {
        const FIXED_MAP: [u32; 8] = [
            operator_list(0, 16, 8, 24),
            operator_list(1, 17, 9, 25),
            operator_list(2, 18, 10, 26),
            operator_list(3, 19, 11, 27),
            operator_list(4, 20, 12, 28),
            operator_list(5, 21, 13, 29),
            operator_list(6, 22, 14, 30),
            operator_list(7, 23, 15, 31),
        ];
        FIXED_MAP[index]
    }

    fn write(
        &mut self,
        index: u32,
        data: u8,
        keyon_channel: &mut u32,
        keyon_opmask: &mut u32,
    ) -> bool {
        let data_bits = data as u32;
        let operator_index = (index & 0x1F) as usize;

        // bit 7 (bit 5 for C0-DF) of the data redirects some writes to the
        // internal registers
        if index == 0x17 && bit(data_bits, 7) != 0 {
            self.regdata[0x188] = data;
        } else if index == 0x19 && bit(data_bits, 7) != 0 {
            self.regdata[0x189] = data;
        } else if (index & 0xF8) == 0x38 && bit(data_bits, 7) != 0 {
            self.regdata[0x180 + (index & 7) as usize] = data;
        } else if (index & 0xE0) == 0x40 && bit(data_bits, 7) != 0 {
            self.regdata[0x100 + operator_index] = data;
        } else if (index & 0xE0) == 0xC0 && bit(data_bits, 5) != 0 {
            self.regdata[0x120 + operator_index] = data;
        } else if index < 0x100 {
            self.regdata[index as usize] = data;
        }

        // a write to 08 restores the sustain level, release rate, envelope
        // shift and reverb rate of a channel from the preset memory
        if index == 0x08 {
            let channel = bitfield(data_bits, 0, 3) as usize;
            for operator in [0, 8, 16, 24] {
                self.regdata[0xE0 + channel + operator] = self.regdata[0x140 + channel + operator];
                self.regdata[0x120 + channel + operator] = self.regdata[0x160 + channel + operator];
            }
        }

        // a waveform write (40-5F with bit 7) marks the operator; the next
        // E0-FF write stores the preset and clears the mark
        if bit(self.regdata[0x100 + operator_index] as u32, 7) != 0 {
            if (index & 0xE0) == 0xE0 {
                self.regdata[0x140 + operator_index] = data;
                self.regdata[0x100 + operator_index] &= 0x7F;
            } else if (index & 0xE0) == 0xC0 && bit(data_bits, 5) != 0 {
                self.regdata[0x160 + operator_index] = data;
            }
        }

        // writes to 20-27 key the channel selected by register 08
        if (index & 0xF8) == 0x20
            && bitfield(index, 0, 3) == bitfield(self.regdata[0x08] as u32, 0, 3)
        {
            *keyon_channel = bitfield(index, 0, 3);
            *keyon_opmask = if self.ch_key_on(*keyon_channel) != 0 {
                0xF
            } else {
                0
            };

            // the sync option resets the LFOs at each note on
            if *keyon_opmask != 0 {
                if self.lfo_sync() != 0 {
                    self.lfo_counter[0] = 0;
                }
                if self.lfo2_sync() != 0 {
                    self.lfo_counter[1] = 0;
                }
            }
            return true;
        }
        false
    }

    fn channel_offset(chnum: u32) -> u32 {
        chnum
    }

    fn operator_offset(opnum: u32) -> u32 {
        opnum
    }

    fn op_ssg_eg_enable(&self, _opoffs: u32) -> u32 {
        0
    }

    fn op_ssg_eg_mode(&self, _opoffs: u32) -> u32 {
        0
    }

    fn op_lfo_am_enable(&self, opoffs: u32) -> u32 {
        self.byte(0xA0, 7, 1, opoffs)
    }

    fn ch_output_any(&self, choffs: u32) -> u32 {
        self.byte(0x20, 7, 1, choffs) | self.byte(0x30, 0, 1, choffs)
    }

    fn ch_output_0(&self, choffs: u32) -> u32 {
        self.byte(0x30, 0, 1, choffs)
    }

    fn ch_output_1(&self, choffs: u32) -> u32 {
        self.byte(0x20, 7, 1, choffs) | self.byte(0x30, 0, 1, choffs)
    }

    fn ch_output_2(&self, _choffs: u32) -> u32 {
        0
    }

    fn ch_output_3(&self, _choffs: u32) -> u32 {
        0
    }

    fn ch_feedback(&self, choffs: u32) -> u32 {
        self.byte(0x20, 3, 3, choffs)
    }

    fn ch_algorithm(&self, choffs: u32) -> u32 {
        self.byte(0x20, 0, 3, choffs)
    }

    fn noise_state(&self) -> u32 {
        self.noise_state as u32
    }

    fn timer_a_value(&self) -> u32 {
        self.word(0x10, 0, 8, 0x11, 0, 2, 0)
    }

    fn timer_b_value(&self) -> u32 {
        self.byte(0x12, 0, 8, 0)
    }

    fn csm(&self) -> u32 {
        self.byte(0x14, 7, 1, 0)
    }

    fn reset_timer_a(&self) -> u32 {
        self.byte(0x14, 4, 1, 0)
    }

    fn reset_timer_b(&self) -> u32 {
        self.byte(0x14, 5, 1, 0)
    }

    fn enable_timer_a(&self) -> u32 {
        self.byte(0x14, 2, 1, 0)
    }

    fn enable_timer_b(&self) -> u32 {
        self.byte(0x14, 3, 1, 0)
    }

    fn load_timer_a(&self) -> u32 {
        self.byte(0x14, 0, 1, 0)
    }

    fn load_timer_b(&self) -> u32 {
        self.byte(0x14, 1, 1, 0)
    }

    fn cache_operator_data(&self, choffs: u32, opoffs: u32, cache: &mut OpdataCache) {
        cache.waveform_index = self.op_waveform(opoffs);

        // get frequency from the channel
        let block_freq = self.ch_block_freq(choffs);
        cache.block_freq = block_freq;

        // the 5-bit keycode is just the top 5 bits (block + top 2 bits of the
        // key code)
        let keycode = bitfield(block_freq, 8, 5);

        // detune adjustment
        cache.detune = detune_adjustment(self.op_detune(opoffs), keycode);

        // multiple value, as an x.4 value (0 means 0.5); the fine control
        // provides the fractional bits
        cache.multiple = self.op_multiple(opoffs) << 4;
        if cache.multiple == 0 {
            cache.multiple = 0x08;
        }
        cache.multiple |= self.op_fine(opoffs);

        // phase step, or PHASE_STEP_DYNAMIC if PM or fixed frequency mode is
        // active
        if self.op_fix_mode(opoffs) == 0
            && (self.lfo_pm_depth() == 0 || self.ch_lfo_pm_sens(choffs) == 0)
            && (self.lfo2_pm_depth() == 0 || self.ch_lfo2_pm_sens(choffs) == 0)
        {
            cache.phase_step = self.keyed_phase_step(choffs, opoffs, cache, 0);
        } else {
            cache.phase_step = OpdataCache::PHASE_STEP_DYNAMIC;
        }

        // total level, scaled by 8
        cache.total_level = self.op_total_level(opoffs) << 3;

        // 4-bit sustain level, but 15 means 31 so effectively 5 bits
        cache.eg_sustain = self.op_sustain_level(opoffs);
        cache.eg_sustain |= (cache.eg_sustain + 1) & 0x10;
        cache.eg_sustain <<= 5;

        // determine KSR adjustment for envelope rates
        let ksrval = keycode >> (self.op_ksr(opoffs) ^ 3);
        cache.eg_rate[EG_ATTACK] = effective_rate(self.op_attack_rate(opoffs) * 2, ksrval) as u8;
        cache.eg_rate[EG_DECAY] = effective_rate(self.op_decay_rate(opoffs) * 2, ksrval) as u8;
        cache.eg_rate[EG_SUSTAIN] = effective_rate(self.op_sustain_rate(opoffs) * 2, ksrval) as u8;
        cache.eg_rate[EG_RELEASE] =
            effective_rate(self.op_release_rate(opoffs) * 4 + 2, ksrval) as u8;
        cache.eg_rate[EG_REVERB] = cache.eg_rate[EG_RELEASE];
        let reverb = self.op_reverb_rate(opoffs);
        if reverb != 0 {
            cache.eg_rate[EG_REVERB] =
                (effective_rate(reverb * 4 + 2, ksrval) as u8).min(cache.eg_rate[EG_REVERB]);
        }

        // operator 1 has no envelope shift
        cache.eg_shift = if (opoffs & 0x18) == 0 {
            0
        } else {
            self.op_eg_shift(opoffs) as u8
        };
    }

    fn compute_phase_step(
        &mut self,
        choffs: u32,
        opoffs: u32,
        cache: &OpdataCache,
        lfo_raw_pm: i32,
    ) -> u32 {
        if self.op_fix_mode(opoffs) == 0 {
            return self.keyed_phase_step(choffs, opoffs, cache, lfo_raw_pm);
        }

        // the base frequency of 8-255Hz comes from the fix frequency and fine
        // registers and is shifted up by the 3-bit range
        let mut freq = self.op_fix_frequency(opoffs) << 4;
        if freq == 0 {
            freq = 8;
        }
        freq |= self.op_fine(opoffs);
        freq <<= self.op_fix_range(opoffs);

        // a per-operator substep adds 12 bits of resolution to the phase step
        let substep = self.phase_substep[opoffs as usize] as u32 + 75 * freq;
        self.phase_substep[opoffs as usize] = (substep & 0xFFF) as u16;
        substep >> 12
    }

    fn clock_noise_and_lfo(&mut self) -> i32 {
        // base noise frequency is measured at 2x 1/2 FM frequency; this means
        // each tick counts as two steps against the noise counter
        let freq = self.noise_frequency();
        for _ in 0..2 {
            // the LFSR is clocked continually and just sampled at the noise
            // frequency for output purposes
            self.noise_lfsr <<= 1;
            self.noise_lfsr |= bit(self.noise_lfsr, 17) ^ bit(self.noise_lfsr, 14) ^ 1;

            // compare against the frequency and latch when we exceed it
            let previous = self.noise_counter;
            self.noise_counter = self.noise_counter.wrapping_add(1);
            if previous as u32 >= freq {
                self.noise_counter = 0;
                self.noise_state = bit(self.noise_lfsr, 17) as u8;
            }
        }

        // treat the rates as 4.4 floating-point step values with implied
        // leading 1
        let rate0 = self.lfo_rate();
        let rate1 = self.lfo2_rate();
        self.lfo_counter[0] = self.lfo_counter[0]
            .wrapping_add((0x10 | bitfield(rate0, 0, 4)) << bitfield(rate0, 4, 4));
        self.lfo_counter[1] = self.lfo_counter[1]
            .wrapping_add((0x10 | bitfield(rate1, 0, 4)) << bitfield(rate1, 4, 4));
        let lfo0 = bitfield(self.lfo_counter[0], 22, 8);
        let lfo1 = bitfield(self.lfo_counter[1], 22, 8);

        // fill in the noise entry 1 ahead of our current position; this ensures
        // the current value remains stable for a full LFO clock
        let lfo_noise = bitfield(self.noise_lfsr, 17, 8);
        let noise_entry = (lfo_noise | (lfo_noise << 8)) as i16;
        self.lfo_waveform[3][((lfo0 + 1) & 0xFF) as usize] = noise_entry;
        self.lfo_waveform[3][((lfo1 + 1) & 0xFF) as usize] = noise_entry;

        // fetch the AM/PM values based on the waveform; AM is unsigned in the
        // low 8 bits, PM is signed in the upper 8 bits
        let ampm0 = self.lfo_waveform[self.lfo_waveform() as usize][lfo0 as usize] as i32;
        let ampm1 = self.lfo_waveform[self.lfo2_waveform() as usize][lfo1 as usize] as i32;

        // apply depth to the AM values and store for later
        self.lfo_am[0] = (((ampm0 & 0xFF) * self.lfo_am_depth() as i32) >> 7) as u8;
        self.lfo_am[1] = (((ampm1 & 0xFF) * self.lfo2_am_depth() as i32) >> 7) as u8;

        // apply depth to the PM values and pack both into the result
        let pm0 = ((ampm0 >> 8) * self.lfo_pm_depth() as i32) >> 7;
        let pm1 = ((ampm1 >> 8) * self.lfo2_pm_depth() as i32) >> 7;
        (pm0 & 0xFF) | (pm1 << 8)
    }

    fn lfo_am_offset(&self, choffs: u32) -> u32 {
        // the two AM LFOs add up; shift value for AM sensitivity is
        // [*, 0, 1, 2]
        let mut result = 0;
        let am_sensitivity = self.ch_lfo_am_sens(choffs);
        if am_sensitivity != 0 {
            result = (self.lfo_am[0] as u32) << (am_sensitivity - 1);
        }
        let am_sensitivity2 = self.ch_lfo2_am_sens(choffs);
        if am_sensitivity2 != 0 {
            result += (self.lfo_am[1] as u32) << (am_sensitivity2 - 1);
        }
        result
    }

    fn waveform(&self, index: u32, phase: u32) -> u16 {
        self.waveform[index as usize][(phase & (WAVEFORM_LENGTH as u32 - 1)) as usize]
    }

    fn status_mask(&self) -> u8 {
        0
    }

    fn irq_reset(&self) -> u32 {
        0
    }

    fn noise_enable(&self) -> u32 {
        self.byte(0x0F, 7, 1, 0)
    }

    fn rhythm_enable(&self) -> u32 {
        0
    }
}
