use crate::{
    fm::{FmRegisters, OpdataCache, effective_rate, operator_list},
    helpers::{bit, bitfield},
    ssg::{SsgEngine, SsgOutput},
    tables::{abs_sin_attenuation, detune_adjustment, opn_lfo_pm_phase_adjustment},
};

// OPN register map:
//
//      System-wide registers:
//           21 xxxxxxxx Test register
//           22 ----x--- LFO enable [OPNA+ only]
//              -----xxx LFO rate [OPNA+ only]
//           24 xxxxxxxx Timer A value (upper 8 bits)
//           25 ------xx Timer A value (lower 2 bits)
//           26 xxxxxxxx Timer B value
//           27 xx------ CSM/Multi-frequency mode for channel #2
//              --x----- Reset timer B
//              ---x---- Reset timer A
//              ----x--- Enable timer B
//              -----x-- Enable timer A
//              ------x- Load timer B
//              -------x Load timer A
//           28 x------- Key on/off operator 4
//              -x------ Key on/off operator 3
//              --x----- Key on/off operator 2
//              ---x---- Key on/off operator 1
//              ------xx Channel select
//
//     Per-channel registers (channel in address bits 0-1)
//     Note that all these apply to address+100 as well on OPNA+
//        A0-A3 xxxxxxxx Frequency number lower 8 bits
//        A4-A7 --xxx--- Block (0-7)
//              -----xxx Frequency number upper 3 bits
//        B0-B3 --xxx--- Feedback level for operator 1 (0-7)
//              -----xxx Operator connection algorithm (0-7)
//        B4-B7 x------- Pan left [OPNA]
//              -x------ Pan right [OPNA]
//              --xx---- LFO AM shift (0-3) [OPNA+ only]
//              -----xxx LFO PM depth (0-7) [OPNA+ only]
//
//     Per-operator registers (channel in address bits 0-1, operator in bits 2-3)
//     Note that all these apply to address+100 as well on OPNA+
//        30-3F -xxx---- Detune value (0-7)
//              ----xxxx Multiple value (0-15)
//        40-4F -xxxxxxx Total level (0-127)
//        50-5F xx------ Key scale rate (0-3)
//              ---xxxxx Attack rate (0-31)
//        60-6F x------- LFO AM enable [OPNA]
//              ---xxxxx Decay rate (0-31)
//        70-7F ---xxxxx Sustain rate (0-31)
//        80-8F xxxx---- Sustain level (0-15)
//              ----xxxx Release rate (0-15)
//        90-9F ----x--- SSG-EG enable
//              -----xxx SSG-EG envelope (0-7)
//
//     Special multi-frequency registers (channel implicitly #2; operator in address bits 0-1)
//        A8-AB xxxxxxxx Frequency number lower 8 bits
//        AC-AF --xxx--- Block (0-7)
//              -----xxx Frequency number upper 3 bits
//
//     Internal (fake) registers:
//        B8-BB --xxxxxx Latched frequency number upper bits (from A4-A7)
//        BC-BF --xxxxxx Latched frequency number upper bits (from AC-AF)

const WAVEFORM_LENGTH: usize = 0x400;

fn reg_byte(regdata: &[u8], offset: u32, start: i32, count: i32, extra_offset: u32) -> u32 {
    bitfield(
        regdata[(offset + extra_offset) as usize] as u32,
        start,
        count,
    )
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn reg_word(
    regdata: &[u8],
    offset1: u32,
    start1: i32,
    count1: i32,
    offset2: u32,
    start2: i32,
    count2: i32,
    extra_offset: u32,
) -> u32 {
    (reg_byte(regdata, offset1, start1, count1, extra_offset) << count2)
        | reg_byte(regdata, offset2, start2, count2, extra_offset)
}

fn opn_ch_block_freq(regdata: &[u8], choffs: u32) -> u32 {
    reg_word(regdata, 0xA4, 0, 6, 0xA0, 0, 8, choffs)
}

fn opn_multi_block_freq(regdata: &[u8], num: u32) -> u32 {
    reg_word(regdata, 0xAC, 0, 6, 0xA8, 0, 8, num)
}

fn opn_cache_operator_data<const IS_OPNA: bool>(
    regdata: &[u8],
    choffs: u32,
    opoffs: u32,
    cache: &mut OpdataCache,
) {
    // get frequency from the channel
    let mut block_freq = opn_ch_block_freq(regdata, choffs);
    cache.block_freq = block_freq;

    // if multi-frequency mode is enabled and this is channel 2,
    // fetch one of the special frequencies
    let multi_freq_on = reg_byte(regdata, 0x27, 6, 2, 0) != 0;
    if multi_freq_on && choffs == 2 {
        if opoffs == 2 {
            block_freq = opn_multi_block_freq(regdata, 1);
            cache.block_freq = block_freq;
        } else if opoffs == 10 {
            block_freq = opn_multi_block_freq(regdata, 2);
            cache.block_freq = block_freq;
        } else if opoffs == 6 {
            block_freq = opn_multi_block_freq(regdata, 0);
            cache.block_freq = block_freq;
        }
    }

    // compute the keycode: block_freq is:
    //
    //     BBBFFFFFFFFFFF
    //     ^^^^???
    //
    // the 5-bit keycode uses the top 4 bits plus a magic formula
    // for the final bit
    let mut keycode = bitfield(block_freq, 10, 4) << 1;

    // lowest bit is determined by a mix of next lower FNUM bits
    // according to this equation from the YM2608 manual:
    //
    //   (F11 & (F10 | F9 | F8)) | (!F11 & F10 & F9 & F8)
    //
    // for speed, we just look it up in a 16-bit constant
    keycode |= bitfield(0xFE80, bitfield(block_freq, 7, 4) as i32, 1);

    let detune_val = reg_byte(regdata, 0x30, 4, 3, opoffs);
    cache.detune = detune_adjustment(detune_val, keycode);

    // multiple value, as an x.1 value (0 means 0.5)
    let multiple = reg_byte(regdata, 0x30, 0, 4, opoffs);
    cache.multiple = multiple * 2;
    if cache.multiple == 0 {
        cache.multiple = 1;
    }

    let lfo_enable = if IS_OPNA {
        reg_byte(regdata, 0x22, 3, 1, 0)
    } else {
        0
    };
    let pm_sens = if IS_OPNA {
        reg_byte(regdata, 0xB4, 0, 3, choffs)
    } else {
        0
    };

    // phase step, or PHASE_STEP_DYNAMIC if PM is active; this depends on
    // block_freq, detune, and multiple, so compute it after we've done those
    if !IS_OPNA || lfo_enable == 0 || pm_sens == 0 {
        cache.phase_step = opn_compute_phase_step::<IS_OPNA>(regdata, choffs, cache, 0);
    } else {
        cache.phase_step = OpdataCache::PHASE_STEP_DYNAMIC;
    }

    // total level, scaled by 8
    cache.total_level = reg_byte(regdata, 0x40, 0, 7, opoffs) << 3;

    // 4-bit sustain level, but 15 means 31 so effectively 5 bits
    cache.eg_sustain = reg_byte(regdata, 0x80, 4, 4, opoffs);
    cache.eg_sustain |= (cache.eg_sustain + 1) & 0x10;
    cache.eg_sustain <<= 5;

    // determine KSR adjustment for envelope rates
    let ksr = reg_byte(regdata, 0x50, 6, 2, opoffs);
    let ksrval = keycode >> (ksr ^ 3);
    cache.eg_rate[1] = effective_rate(reg_byte(regdata, 0x50, 0, 5, opoffs) * 2, ksrval) as u8;
    cache.eg_rate[2] = effective_rate(reg_byte(regdata, 0x60, 0, 5, opoffs) * 2, ksrval) as u8;
    cache.eg_rate[3] = effective_rate(reg_byte(regdata, 0x70, 0, 5, opoffs) * 2, ksrval) as u8;
    cache.eg_rate[4] = effective_rate(reg_byte(regdata, 0x80, 0, 4, opoffs) * 4 + 2, ksrval) as u8;

    // OPN always uses waveform 0 (pure sine)
    cache.waveform_index = 0;
}

// OPN phase calculation has only a single detune parameter
// and uses FNUMs instead of keycodes
fn opn_compute_phase_step<const IS_OPNA: bool>(
    regdata: &[u8],
    choffs: u32,
    cache: &OpdataCache,
    lfo_raw_pm: i32,
) -> u32 {
    // extract frequency number (low 11 bits of block_freq)
    let mut fnum = bitfield(cache.block_freq, 0, 11) << 1;

    // if there's a non-zero PM sensitivity, compute the adjustment
    let pm_sensitivity = if IS_OPNA {
        reg_byte(regdata, 0xB4, 0, 3, choffs)
    } else {
        0
    };
    if pm_sensitivity != 0 {
        // apply the phase adjustment based on the upper 7 bits
        // of FNUM and the PM depth parameters
        fnum = (fnum as i32
            + opn_lfo_pm_phase_adjustment(
                bitfield(cache.block_freq, 4, 7),
                pm_sensitivity,
                lfo_raw_pm,
            )) as u32;
        // keep fnum to 12 bits
        fnum &= 0xFFF;
    }

    // apply block shift to compute phase step
    let block = bitfield(cache.block_freq, 11, 3);
    let mut phase_step = (fnum << block) >> 2;

    // apply detune based on the keycode
    phase_step = (phase_step as i32 + cache.detune) as u32;

    // clamp to 17 bits in case detune overflows
    // QUESTION: is this specific to the YM2612/3438?
    phase_step &= 0x1FFFF;

    // apply frequency multiplier (which is cached as an x.1 value)
    (phase_step * cache.multiple) >> 1
}

fn opn_clock_noise_and_lfo<const IS_OPNA: bool>(
    regdata: &[u8],
    lfo_counter: &mut u32,
    lfo_am: &mut u8,
) -> i32 {
    // OPN has no noise generation

    // if LFO not enabled (not present on OPN), quick exit with 0s
    let lfo_enable = if IS_OPNA {
        reg_byte(regdata, 0x22, 3, 1, 0)
    } else {
        0
    };

    if !IS_OPNA || lfo_enable == 0 {
        *lfo_counter = 0;
        // special case: if LFO is disabled on OPNA, it basically just keeps the counter
        // at 0; since position 0 gives an AM value of 0x3f, it is important to reflect
        // that here; for example, MegaDrive Venom plays some notes with LFO globally
        // disabled but enabling LFO on the operators, and it expects this added attenuation
        *lfo_am = if IS_OPNA { 0x3F } else { 0x00 };
        return 0;
    }

    // this table is based on converting the frequencies in the applications
    // manual to clock dividers, based on the assumption of a 7-bit LFO value
    static LFO_MAX_COUNT: [u8; 8] = [109, 78, 72, 68, 63, 45, 9, 6];
    let subcount = *lfo_counter as u8;
    *lfo_counter = lfo_counter.wrapping_add(1);

    // when we cross the divider count, add enough to zero it and cause an
    // increment at bit 8; the 7-bit value lives from bits 8-14
    let lfo_rate = reg_byte(regdata, 0x22, 0, 3, 0);
    if subcount >= LFO_MAX_COUNT[lfo_rate as usize] {
        // note: to match the published values this should be 0x100 - subcount;
        // however, tests on the hardware and nuked bear out an off-by-one
        // error exists that causes the max LFO rate to be faster than published
        *lfo_counter = lfo_counter.wrapping_add(0x101u32.wrapping_sub(subcount as u32));
    }

    // AM value is 7 bits, starting at bit 8; grab the low 6 directly
    *lfo_am = bitfield(*lfo_counter, 8, 6) as u8;
    // first half of the AM period (bit 6 == 0) is inverted
    if bitfield(*lfo_counter, 14, 1) == 0 {
        *lfo_am ^= 0x3F;
    }

    // PM value is 5 bits, starting at bit 10; grab the low 3 directly
    let mut pm = bitfield(*lfo_counter, 10, 3) as i32;
    // PM is reflected based on bit 3
    if bitfield(*lfo_counter, 13, 1) != 0 {
        pm ^= 7;
    }

    // PM is negated based on bit 4
    if bitfield(*lfo_counter, 14, 1) != 0 {
        -pm
    } else {
        pm
    }
}

fn opn_lfo_am_offset<const IS_OPNA: bool>(regdata: &[u8], lfo_am: u8, choffs: u32) -> u32 {
    // shift value for AM sensitivity is [7, 3, 1, 0],
    // mapping to values of [0, 1.4, 5.9, and 11.8dB]
    let am_sens = if IS_OPNA {
        reg_byte(regdata, 0xB4, 4, 2, choffs)
    } else {
        0
    };
    let am_shift = (1u32 << (am_sens ^ 3)) - 1;

    // QUESTION: max sensitivity should give 11.8dB range, but this value
    // is directly added to an x.8 attenuation value, which will only give
    // 126/256 or ~4.9dB range -- what am I missing? The calculation below
    // matches several other emulators, including the Nuked implementation.

    // raw LFO AM value on OPN is 0-3F, scale that up by a factor of 2
    // (giving 7 bits) before applying the final shift
    ((lfo_am as u32) << 1) >> am_shift
}

fn opn_write<const IS_OPNA: bool>(
    regdata: &mut [u8],
    index: u16,
    data: u8,
    channel: &mut u32,
    opmask: &mut u32,
) -> bool {
    // writes in the 0xa0-af/0x1a0-af region are handled as latched pairs
    // borrow unused registers 0xb8-bf as temporary holding locations
    if (index & 0xF0) == 0xA0 {
        if bitfield(index as u32, 0, 2) == 3 {
            return false;
        }

        let latchindex = 0xB8u16 | (bitfield(index as u32, 3, 1) as u16);

        // writes to the upper half just latch (only low 6 bits matter)
        if bitfield(index as u32, 2, 1) != 0 {
            regdata[latchindex as usize] = data & 0x3F;
        // writes to the lower half always commit using the current latch
        } else {
            regdata[index as usize] = data;
            regdata[(index | 4) as usize] = regdata[latchindex as usize];
        }
        return false;
    } else if (index & 0xF8) == 0xB8 {
        // registers 0xb8-0xbf are used internally
        return false;
    }

    regdata[index as usize] = data;

    if index == 0x28 {
        *channel = bitfield(data as u32, 0, 2);
        if *channel == 3 {
            return false;
        }
        if IS_OPNA {
            *channel += bitfield(data as u32, 2, 1) * 3;
        }
        *opmask = bitfield(data as u32, 4, 4);
        return true;
    }

    false
}

save_state::runtime_state! {
/// Authoritative OPN register and timer state.
#[derive(Clone)]
pub(crate) struct OpnRegisters {
    lfo_counter: u32,
    lfo_am: u8,
    regdata: [u8; 0x100],
    waveform: [u16; WAVEFORM_LENGTH],
}}

impl FmRegisters for OpnRegisters {
    const OUTPUTS: usize = 1;
    const CHANNELS: usize = 3;
    const ALL_CHANNELS: u32 = (1 << 3) - 1;
    const OPERATORS: usize = 12;
    const DEFAULT_PRESCALE: u32 = 6;
    const EG_CLOCK_DIVIDER: u32 = 3;
    const CSM_TRIGGER_MASK: u32 = 1 << 2;
    const REG_MODE: u32 = 0x27;
    const EG_HAS_DEPRESS: bool = false;
    const EG_HAS_REVERB: bool = false;
    const EG_HAS_SSG: bool = true;
    const MODULATOR_DELAY: bool = false;
    const DYNAMIC_OPS: bool = false;

    const STATUS_TIMERA: u8 = 0x01;
    const STATUS_TIMERB: u8 = 0x02;
    const STATUS_BUSY: u8 = 0x80;
    const STATUS_IRQ: u8 = 0;

    const RHYTHM_CHANNEL: u32 = 0xFF;

    fn new() -> Self {
        let mut waveform = [0u16; WAVEFORM_LENGTH];
        for (index, entry) in waveform.iter_mut().enumerate() {
            *entry = (abs_sin_attenuation(index as u32) | (bit(index as u32, 9) << 15)) as u16;
        }
        Self {
            lfo_counter: 0,
            lfo_am: 0,
            regdata: [0; 0x100],
            waveform,
        }
    }

    fn reset(&mut self) {
        self.regdata.fill(0);
    }

    // Note that the channel index order is 0,2,1,3, so we bitswap the index.
    //
    // This is because the order in the map is:
    //    carrier 1, carrier 2, modulator 1, modulator 2
    //
    // But when wiring up the connections, the more natural order is:
    //    carrier 1, modulator 1, carrier 2, modulator 2
    fn operator_map(&self, index: usize) -> u32 {
        const FIXED_MAP: [u32; 3] = [
            operator_list(0, 6, 3, 9),
            operator_list(1, 7, 4, 10),
            operator_list(2, 8, 5, 11),
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
        opn_write::<false>(
            &mut self.regdata,
            index as u16,
            data,
            keyon_channel,
            keyon_opmask,
        )
    }

    fn channel_offset(chnum: u32) -> u32 {
        chnum
    }

    fn operator_offset(opnum: u32) -> u32 {
        opnum + opnum / 3
    }

    fn op_ssg_eg_enable(&self, opoffs: u32) -> u32 {
        reg_byte(&self.regdata, 0x90, 3, 1, opoffs)
    }

    fn op_ssg_eg_mode(&self, opoffs: u32) -> u32 {
        reg_byte(&self.regdata, 0x90, 0, 3, opoffs)
    }

    fn op_lfo_am_enable(&self, _opoffs: u32) -> u32 {
        0
    }

    fn ch_output_any(&self, _choffs: u32) -> u32 {
        1
    }

    fn ch_output_0(&self, _choffs: u32) -> u32 {
        1
    }

    fn ch_output_1(&self, _choffs: u32) -> u32 {
        0
    }

    fn ch_output_2(&self, _choffs: u32) -> u32 {
        0
    }

    fn ch_output_3(&self, _choffs: u32) -> u32 {
        0
    }

    fn ch_feedback(&self, choffs: u32) -> u32 {
        reg_byte(&self.regdata, 0xB0, 3, 3, choffs)
    }

    fn ch_algorithm(&self, choffs: u32) -> u32 {
        reg_byte(&self.regdata, 0xB0, 0, 3, choffs)
    }

    fn noise_state(&self) -> u32 {
        0
    }

    fn timer_a_value(&self) -> u32 {
        reg_word(&self.regdata, 0x24, 0, 8, 0x25, 0, 2, 0)
    }

    fn timer_b_value(&self) -> u32 {
        reg_byte(&self.regdata, 0x26, 0, 8, 0)
    }

    fn csm(&self) -> u32 {
        if reg_byte(&self.regdata, 0x27, 6, 2, 0) == 2 {
            1
        } else {
            0
        }
    }

    fn reset_timer_a(&self) -> u32 {
        reg_byte(&self.regdata, 0x27, 4, 1, 0)
    }

    fn reset_timer_b(&self) -> u32 {
        reg_byte(&self.regdata, 0x27, 5, 1, 0)
    }

    fn enable_timer_a(&self) -> u32 {
        reg_byte(&self.regdata, 0x27, 2, 1, 0)
    }

    fn enable_timer_b(&self) -> u32 {
        reg_byte(&self.regdata, 0x27, 3, 1, 0)
    }

    fn load_timer_a(&self) -> u32 {
        reg_byte(&self.regdata, 0x27, 0, 1, 0)
    }

    fn load_timer_b(&self) -> u32 {
        reg_byte(&self.regdata, 0x27, 1, 1, 0)
    }

    fn cache_operator_data(&self, choffs: u32, opoffs: u32, cache: &mut OpdataCache) {
        opn_cache_operator_data::<false>(&self.regdata, choffs, opoffs, cache);
    }

    fn compute_phase_step(
        &self,
        choffs: u32,
        _opoffs: u32,
        cache: &OpdataCache,
        lfo_raw_pm: i32,
    ) -> u32 {
        opn_compute_phase_step::<false>(&self.regdata, choffs, cache, lfo_raw_pm)
    }

    fn clock_noise_and_lfo(&mut self) -> i32 {
        opn_clock_noise_and_lfo::<false>(&self.regdata, &mut self.lfo_counter, &mut self.lfo_am)
    }

    fn lfo_am_offset(&self, choffs: u32) -> u32 {
        opn_lfo_am_offset::<false>(&self.regdata, self.lfo_am, choffs)
    }

    fn waveform(&self, _index: u32, phase: u32) -> u16 {
        self.waveform[(phase & (WAVEFORM_LENGTH as u32 - 1)) as usize]
    }

    fn status_mask(&self) -> u8 {
        0
    }

    fn irq_reset(&self) -> u32 {
        0
    }

    fn noise_enable(&self) -> u32 {
        0
    }

    fn rhythm_enable(&self) -> u32 {
        0
    }
}

save_state::runtime_state! {
/// Authoritative OPNA register and timer state.
#[derive(Clone)]
pub(crate) struct OpnaRegisters {
    lfo_counter: u32,
    lfo_am: u8,
    regdata: [u8; 0x200],
    waveform: [u16; WAVEFORM_LENGTH],
}}

impl FmRegisters for OpnaRegisters {
    const OUTPUTS: usize = 2;
    const CHANNELS: usize = 6;
    const ALL_CHANNELS: u32 = (1 << 6) - 1;
    const OPERATORS: usize = 24;
    const DEFAULT_PRESCALE: u32 = 6;
    const EG_CLOCK_DIVIDER: u32 = 3;
    const CSM_TRIGGER_MASK: u32 = 1 << 2;
    const REG_MODE: u32 = 0x27;
    const EG_HAS_DEPRESS: bool = false;
    const EG_HAS_REVERB: bool = false;
    const EG_HAS_SSG: bool = true;
    const MODULATOR_DELAY: bool = false;
    const DYNAMIC_OPS: bool = false;

    const STATUS_TIMERA: u8 = 0x01;
    const STATUS_TIMERB: u8 = 0x02;
    const STATUS_BUSY: u8 = 0x80;
    const STATUS_IRQ: u8 = 0;

    const RHYTHM_CHANNEL: u32 = 0xFF;

    fn new() -> Self {
        let mut waveform = [0u16; WAVEFORM_LENGTH];
        for (index, entry) in waveform.iter_mut().enumerate() {
            *entry = (abs_sin_attenuation(index as u32) | (bit(index as u32, 9) << 15)) as u16;
        }
        Self {
            lfo_counter: 0,
            lfo_am: 0,
            regdata: [0; 0x200],
            waveform,
        }
    }

    fn reset(&mut self) {
        self.regdata.fill(0);
        self.regdata[0xB4] = 0xC0;
        self.regdata[0xB5] = 0xC0;
        self.regdata[0xB6] = 0xC0;
        self.regdata[0x1B4] = 0xC0;
        self.regdata[0x1B5] = 0xC0;
        self.regdata[0x1B6] = 0xC0;
    }

    // Note that the channel index order is 0,2,1,3, so we bitswap the index.
    //
    // This is because the order in the map is:
    //    carrier 1, carrier 2, modulator 1, modulator 2
    //
    // But when wiring up the connections, the more natural order is:
    //    carrier 1, modulator 1, carrier 2, modulator 2
    fn operator_map(&self, index: usize) -> u32 {
        const FIXED_MAP: [u32; 6] = [
            operator_list(0, 6, 3, 9),
            operator_list(1, 7, 4, 10),
            operator_list(2, 8, 5, 11),
            operator_list(12, 18, 15, 21),
            operator_list(13, 19, 16, 22),
            operator_list(14, 20, 17, 23),
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
        opn_write::<true>(
            &mut self.regdata,
            index as u16,
            data,
            keyon_channel,
            keyon_opmask,
        )
    }

    fn channel_offset(chnum: u32) -> u32 {
        (chnum % 3) + 0x100 * (chnum / 3)
    }

    fn operator_offset(opnum: u32) -> u32 {
        (opnum % 12) + ((opnum % 12) / 3) + 0x100 * (opnum / 12)
    }

    fn op_ssg_eg_enable(&self, opoffs: u32) -> u32 {
        reg_byte(&self.regdata, 0x90, 3, 1, opoffs)
    }

    fn op_ssg_eg_mode(&self, opoffs: u32) -> u32 {
        reg_byte(&self.regdata, 0x90, 0, 3, opoffs)
    }

    fn op_lfo_am_enable(&self, opoffs: u32) -> u32 {
        reg_byte(&self.regdata, 0x60, 7, 1, opoffs)
    }

    fn ch_output_any(&self, choffs: u32) -> u32 {
        reg_byte(&self.regdata, 0xB4, 6, 2, choffs)
    }

    fn ch_output_0(&self, choffs: u32) -> u32 {
        reg_byte(&self.regdata, 0xB4, 7, 1, choffs)
    }

    fn ch_output_1(&self, choffs: u32) -> u32 {
        reg_byte(&self.regdata, 0xB4, 6, 1, choffs)
    }

    fn ch_output_2(&self, _choffs: u32) -> u32 {
        0
    }

    fn ch_output_3(&self, _choffs: u32) -> u32 {
        0
    }

    fn ch_feedback(&self, choffs: u32) -> u32 {
        reg_byte(&self.regdata, 0xB0, 3, 3, choffs)
    }

    fn ch_algorithm(&self, choffs: u32) -> u32 {
        reg_byte(&self.regdata, 0xB0, 0, 3, choffs)
    }

    fn noise_state(&self) -> u32 {
        0
    }

    fn timer_a_value(&self) -> u32 {
        reg_word(&self.regdata, 0x24, 0, 8, 0x25, 0, 2, 0)
    }

    fn timer_b_value(&self) -> u32 {
        reg_byte(&self.regdata, 0x26, 0, 8, 0)
    }

    fn csm(&self) -> u32 {
        if reg_byte(&self.regdata, 0x27, 6, 2, 0) == 2 {
            1
        } else {
            0
        }
    }

    fn reset_timer_a(&self) -> u32 {
        reg_byte(&self.regdata, 0x27, 4, 1, 0)
    }

    fn reset_timer_b(&self) -> u32 {
        reg_byte(&self.regdata, 0x27, 5, 1, 0)
    }

    fn enable_timer_a(&self) -> u32 {
        reg_byte(&self.regdata, 0x27, 2, 1, 0)
    }

    fn enable_timer_b(&self) -> u32 {
        reg_byte(&self.regdata, 0x27, 3, 1, 0)
    }

    fn load_timer_a(&self) -> u32 {
        reg_byte(&self.regdata, 0x27, 0, 1, 0)
    }

    fn load_timer_b(&self) -> u32 {
        reg_byte(&self.regdata, 0x27, 1, 1, 0)
    }

    fn cache_operator_data(&self, choffs: u32, opoffs: u32, cache: &mut OpdataCache) {
        opn_cache_operator_data::<true>(&self.regdata, choffs, opoffs, cache);
    }

    fn compute_phase_step(
        &self,
        choffs: u32,
        _opoffs: u32,
        cache: &OpdataCache,
        lfo_raw_pm: i32,
    ) -> u32 {
        opn_compute_phase_step::<true>(&self.regdata, choffs, cache, lfo_raw_pm)
    }

    fn clock_noise_and_lfo(&mut self) -> i32 {
        opn_clock_noise_and_lfo::<true>(&self.regdata, &mut self.lfo_counter, &mut self.lfo_am)
    }

    fn lfo_am_offset(&self, choffs: u32) -> u32 {
        opn_lfo_am_offset::<true>(&self.regdata, self.lfo_am, choffs)
    }

    fn waveform(&self, _index: u32, phase: u32) -> u16 {
        self.waveform[(phase & (WAVEFORM_LENGTH as u32 - 1)) as usize]
    }

    fn status_mask(&self) -> u8 {
        0
    }

    fn irq_reset(&self) -> u32 {
        0
    }

    fn noise_enable(&self) -> u32 {
        0
    }

    fn rhythm_enable(&self) -> u32 {
        0
    }
}

save_state::runtime_state_enum! {
/// Active SSG resampling operation.
#[derive(Clone, Copy)]
enum ResampleModeKind {
    Nop = 0,
    N1 = 1,
    OneN = 2,
    TwoNine = 3,
    TwoThree = 4,
    FourThree = 5,
}}

save_state::runtime_state! {
/// One pending SSG resampling operation and its operands.
#[derive(Clone)]
struct ResampleMode {
    kind: ResampleModeKind,
    factor: u32,
}}

save_state::runtime_state! {
/// Authoritative SSG resampler history and pending operation.
#[derive(Clone)]
pub(crate) struct SsgResampler {
    mode: ResampleMode,
    sampindex: u32,
    last: SsgOutput,
    mix_to_1: bool,
    first_output: usize,
}}

impl SsgResampler {
    pub(crate) fn new(mix_to_1: bool, first_output: usize) -> Self {
        Self {
            mode: ResampleMode {
                kind: ResampleModeKind::Nop,
                factor: 0,
            },
            sampindex: 0,
            last: SsgOutput { data: [0; 3] },
            mix_to_1,
            first_output,
        }
    }

    pub(crate) fn sampindex(&self) -> u32 {
        self.sampindex
    }

    pub(crate) fn configure(&mut self, outsamples: u8, srcsamples: u8) {
        let key = outsamples as u32 * 10 + srcsamples as u32;
        self.mode = match key {
            41 => ResampleMode {
                kind: ResampleModeKind::N1,
                factor: 4,
            },
            21 => ResampleMode {
                kind: ResampleModeKind::N1,
                factor: 2,
            },
            43 => ResampleMode {
                kind: ResampleModeKind::FourThree,
                factor: 0,
            },
            11 => ResampleMode {
                kind: ResampleModeKind::N1,
                factor: 1,
            },
            23 => ResampleMode {
                kind: ResampleModeKind::TwoThree,
                factor: 0,
            },
            13 => ResampleMode {
                kind: ResampleModeKind::OneN,
                factor: 3,
            },
            29 => ResampleMode {
                kind: ResampleModeKind::TwoNine,
                factor: 0,
            },
            16 => ResampleMode {
                kind: ResampleModeKind::OneN,
                factor: 6,
            },
            0 => ResampleMode {
                kind: ResampleModeKind::Nop,
                factor: 0,
            },
            _ => ResampleMode {
                kind: ResampleModeKind::Nop,
                factor: 0,
            },
        };
    }

    pub(crate) fn resample(&mut self, ssg: &mut SsgEngine, output: &mut [i32], num_outputs: usize) {
        match self.mode.kind {
            ResampleModeKind::Nop => self.resample_nop(num_outputs),
            ResampleModeKind::N1 => self.resample_n_1(ssg, output, num_outputs, self.mode.factor),
            ResampleModeKind::OneN => self.resample_1_n(ssg, output, num_outputs, self.mode.factor),
            ResampleModeKind::TwoNine => self.resample_2_9(ssg, output, num_outputs),
            ResampleModeKind::TwoThree => self.resample_2_3(ssg, output, num_outputs),
            ResampleModeKind::FourThree => self.resample_4_3(ssg, output, num_outputs),
        }
    }

    fn output_stride(&self) -> usize {
        if self.mix_to_1 {
            self.first_output + 1
        } else {
            self.first_output + 3
        }
    }

    fn add_last(&self, sum0: &mut i32, sum1: &mut i32, sum2: &mut i32, scale: i32) {
        *sum0 += self.last.data[0] * scale;
        *sum1 += self.last.data[1] * scale;
        *sum2 += self.last.data[2] * scale;
    }

    fn clock_and_add(
        &mut self,
        ssg: &mut SsgEngine,
        sum0: &mut i32,
        sum1: &mut i32,
        sum2: &mut i32,
        scale: i32,
    ) {
        ssg.clock();
        ssg.output(&mut self.last);
        *sum0 += self.last.data[0] * scale;
        *sum1 += self.last.data[1] * scale;
        *sum2 += self.last.data[2] * scale;
    }

    fn write_to_output(
        &mut self,
        output: &mut [i32],
        offset: usize,
        sum0: i32,
        sum1: i32,
        sum2: i32,
        divisor: i32,
    ) {
        if self.mix_to_1 {
            output[offset + self.first_output] = (sum0 + sum1 + sum2) * 2 / (3 * divisor);
        } else {
            output[offset + self.first_output] = sum0 / divisor;
            output[offset + self.first_output + 1] = sum1 / divisor;
            output[offset + self.first_output + 2] = sum2 / divisor;
        }
        self.sampindex += 1;
    }

    fn resample_nop(&mut self, num_outputs: usize) {
        self.sampindex += num_outputs as u32;
    }

    fn resample_n_1(
        &mut self,
        ssg: &mut SsgEngine,
        output: &mut [i32],
        num_outputs: usize,
        multiplier: u32,
    ) {
        let stride = self.output_stride();
        for samp in 0..num_outputs {
            if self.sampindex.is_multiple_of(multiplier) {
                ssg.clock();
                ssg.output(&mut self.last);
            }
            self.write_to_output(
                output,
                samp * stride,
                self.last.data[0],
                self.last.data[1],
                self.last.data[2],
                1,
            );
        }
    }

    fn resample_1_n(
        &mut self,
        ssg: &mut SsgEngine,
        output: &mut [i32],
        num_outputs: usize,
        divisor: u32,
    ) {
        let stride = self.output_stride();
        for samp in 0..num_outputs {
            let mut sum0: i32 = 0;
            let mut sum1: i32 = 0;
            let mut sum2: i32 = 0;
            for _rep in 0..divisor {
                self.clock_and_add(ssg, &mut sum0, &mut sum1, &mut sum2, 1);
            }
            self.write_to_output(output, samp * stride, sum0, sum1, sum2, divisor as i32);
        }
    }

    fn resample_2_9(&mut self, ssg: &mut SsgEngine, output: &mut [i32], num_outputs: usize) {
        let stride = self.output_stride();
        for samp in 0..num_outputs {
            let mut sum0: i32 = 0;
            let mut sum1: i32 = 0;
            let mut sum2: i32 = 0;
            if bitfield(self.sampindex, 0, 1) != 0 {
                self.add_last(&mut sum0, &mut sum1, &mut sum2, 1);
            }
            self.clock_and_add(ssg, &mut sum0, &mut sum1, &mut sum2, 2);
            self.clock_and_add(ssg, &mut sum0, &mut sum1, &mut sum2, 2);
            self.clock_and_add(ssg, &mut sum0, &mut sum1, &mut sum2, 2);
            self.clock_and_add(ssg, &mut sum0, &mut sum1, &mut sum2, 2);
            if bitfield(self.sampindex, 0, 1) == 0 {
                self.clock_and_add(ssg, &mut sum0, &mut sum1, &mut sum2, 1);
            }
            self.write_to_output(output, samp * stride, sum0, sum1, sum2, 9);
        }
    }

    fn resample_2_3(&mut self, ssg: &mut SsgEngine, output: &mut [i32], num_outputs: usize) {
        let stride = self.output_stride();
        for samp in 0..num_outputs {
            let mut sum0: i32 = 0;
            let mut sum1: i32 = 0;
            let mut sum2: i32 = 0;
            if bitfield(self.sampindex, 0, 1) == 0 {
                self.clock_and_add(ssg, &mut sum0, &mut sum1, &mut sum2, 2);
                self.clock_and_add(ssg, &mut sum0, &mut sum1, &mut sum2, 1);
            } else {
                self.add_last(&mut sum0, &mut sum1, &mut sum2, 1);
                self.clock_and_add(ssg, &mut sum0, &mut sum1, &mut sum2, 2);
            }
            self.write_to_output(output, samp * stride, sum0, sum1, sum2, 3);
        }
    }

    fn resample_4_3(&mut self, ssg: &mut SsgEngine, output: &mut [i32], num_outputs: usize) {
        let stride = self.output_stride();
        for samp in 0..num_outputs {
            let mut sum0: i32 = 0;
            let mut sum1: i32 = 0;
            let mut sum2: i32 = 0;
            let step = bitfield(self.sampindex, 0, 2) as i32;
            self.add_last(&mut sum0, &mut sum1, &mut sum2, step);
            if step != 3 {
                self.clock_and_add(ssg, &mut sum0, &mut sum1, &mut sum2, 3 - step);
            }
            self.write_to_output(output, samp * stride, sum0, sum1, sum2, 3);
        }
    }
}
