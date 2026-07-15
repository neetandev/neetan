// OPL/OPL2/OPL3/OPL4 register map:
//
//      System-wide registers:
//           01 xxxxxxxx Test register
//              --x----- Enable OPL compatibility mode [OPL2 only] (0 = enable)
//           02 xxxxxxxx Timer A value (4 * OPN)
//           03 xxxxxxxx Timer B value
//           04 x------- RST
//              -x------ Mask timer A
//              --x----- Mask timer B
//              ------x- Load timer B
//              -------x Load timer A
//           08 x------- CSM mode [OPL/OPL2 only]
//              -x------ Note select
//           BD x------- AM depth
//              -x------ PM depth
//              --x----- Rhythm enable
//              ---x---- Bass drum key on
//              ----x--- Snare drum key on
//              -----x-- Tom key on
//              ------x- Top cymbal key on
//              -------x High hat key on
//          101 --xxxxxx Test register 2 [OPL3 only]
//          104 --x----- Channel 6 4-operator mode [OPL3 only]
//              ---x---- Channel 5 4-operator mode [OPL3 only]
//              ----x--- Channel 4 4-operator mode [OPL3 only]
//              -----x-- Channel 3 4-operator mode [OPL3 only]
//              ------x- Channel 2 4-operator mode [OPL3 only]
//              -------x Channel 1 4-operator mode [OPL3 only]
//          105 -------x New [OPL3 only]
//              ------x- New2 [OPL4 only]
//
//     Per-channel registers (channel in address bits 0-3)
//     Note that all these apply to address+100 as well on OPL3+
//        A0-A8 xxxxxxxx F-number (low 8 bits)
//        B0-B8 --x----- Key on
//              ---xxx-- Block (octave, 0-7)
//              ------xx F-number (high two bits)
//        C0-C8 x------- CHD output (to DO0 pin) [OPL3+ only]
//              -x------ CHC output (to DO0 pin) [OPL3+ only]
//              --x----- CHB output (mixed right, to DO2 pin) [OPL3+ only]
//              ---x---- CHA output (mixed left, to DO2 pin) [OPL3+ only]
//              ----xxx- Feedback level for operator 1 (0-7)
//              -------x Operator connection algorithm
//
//     Per-operator registers (operator in bits 0-5)
//     Note that all these apply to address+100 as well on OPL3+
//        20-35 x------- AM enable
//              -x------ PM enable (VIB)
//              --x----- EG type
//              ---x---- Key scale rate
//              ----xxxx Multiple value (0-15)
//        40-55 xx------ Key scale level (0-3)
//              --xxxxxx Total level (0-63)
//        60-75 xxxx---- Attack rate (0-15)
//              ----xxxx Decay rate (0-15)
//        80-95 xxxx---- Sustain level (0-15)
//              ----xxxx Release rate (0-15)
//        E0-F5 ------xx Wave select (0-3) [OPL2 only]
//              -----xxx Wave select (0-7) [OPL3+ only]

use crate::{
    fm::{EnvelopeState, FmRegisters, OpdataCache, effective_rate, operator_list},
    helpers::{bit, bitfield},
    tables::{abs_sin_attenuation, opl_key_scale_atten},
};

const WAVEFORM_LENGTH: usize = 0x400;

// Helper to extract a bitfield from register data.
fn reg_byte(regdata: &[u8], offset: u32, start: u32, count: u32, extra_offset: u32) -> u32 {
    bitfield(
        regdata[(offset + extra_offset) as usize] as u32,
        start as i32,
        count as i32,
    )
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn reg_word(
    regdata: &[u8],
    offset1: u32,
    start1: u32,
    count1: u32,
    offset2: u32,
    start2: u32,
    count2: u32,
    extra_offset: u32,
) -> u32 {
    (reg_byte(regdata, offset1, start1, count1, extra_offset) << count2)
        | reg_byte(regdata, offset2, start2, count2, extra_offset)
}

fn opl_clock_noise_and_lfo(
    noise_lfsr: &mut u32,
    lfo_am_counter: &mut u16,
    lfo_pm_counter: &mut u16,
    lfo_am: &mut u8,
    am_depth: u32,
    pm_depth: u32,
) -> i32 {
    // OPL has a 23-bit noise generator for the rhythm section, running at
    // a constant rate, used only for percussion input
    *noise_lfsr <<= 1;
    *noise_lfsr |= bitfield(*noise_lfsr, 23, 1)
        ^ bitfield(*noise_lfsr, 9, 1)
        ^ bitfield(*noise_lfsr, 8, 1)
        ^ bitfield(*noise_lfsr, 1, 1);

    // the AM LFO has 210*64 steps; at a nominal 50kHz output,
    // this equates to a period of 50000/(210*64) = 3.72Hz
    let am_counter = *lfo_am_counter;
    *lfo_am_counter = lfo_am_counter.wrapping_add(1);
    if am_counter >= 210 * 64 - 1 {
        *lfo_am_counter = 0;
    }

    // low 8 bits are fractional; depth 0 is divided by 2, while depth 1 is times 2
    let shift: i32 = 9 - 2 * am_depth as i32;

    // AM value is the upper bits of the value, inverted across the midpoint
    // to produce a triangle
    let am_val = if (am_counter as u32) < 105 * 64 {
        am_counter as u32
    } else {
        210 * 64 + 63 - am_counter as u32
    };
    *lfo_am = (am_val >> shift) as u8;

    // the PM LFO has 8192 steps, or a nominal period of 6.1Hz
    let pm_counter = *lfo_pm_counter;
    *lfo_pm_counter = lfo_pm_counter.wrapping_add(1);

    // PM LFO is broken into 8 chunks, each lasting 1024 steps; the PM value
    // depends on the upper bits of FNUM, so this value is a fraction and
    // sign to apply to that value, as a 1.3 value
    static PM_SCALE: [i8; 8] = [8, 4, 0, -4, -8, -4, 0, 4];
    (PM_SCALE[bitfield(pm_counter as u32, 10, 3) as usize] as i32) >> (pm_depth ^ 1)
}

fn opl_compute_phase_step(block_freq: u32, multiple: u32, lfo_raw_pm: i32) -> u32 {
    // extract frequency number as a 12-bit fraction
    let mut fnum = bitfield(block_freq, 0, 10) << 2;

    // apply the phase adjustment based on the upper 3 bits
    // of FNUM and the PM depth parameters
    fnum = (fnum as i32 + ((lfo_raw_pm * bitfield(block_freq, 7, 3) as i32) >> 1)) as u32;

    // keep fnum to 12 bits
    fnum &= 0xFFF;

    // apply block shift to compute phase step
    let block = bitfield(block_freq, 10, 3);
    let phase_step = (fnum << block) >> 2;

    // apply frequency multiplier (which is cached as an x.1 value)
    (phase_step * multiple) >> 1
}

fn opl_cache_operator_data<const REVISION: u32>(
    regdata: &[u8],
    num_waveforms: usize,
    choffs: u32,
    opoffs: u32,
    cache: &mut OpdataCache,
) {
    let is_opl3_plus = REVISION >= 3;

    // set up the waveform index
    let wf_idx = {
        let waveform_enable = if REVISION == 2 {
            reg_byte(regdata, 0x01, 5, 1, 0) != 0
        } else {
            REVISION >= 3
        };
        if waveform_enable {
            let new_flag = if is_opl3_plus {
                reg_byte(regdata, 0x105, 0, 1, 0)
            } else {
                0
            };
            let bits = if new_flag != 0 { 3 } else { 2 };
            reg_byte(regdata, 0xE0, 0, bits, opoffs) as usize % num_waveforms
        } else {
            0
        }
    };
    cache.waveform_index = wf_idx as u32;

    // get frequency from the channel
    let block_freq = reg_word(regdata, 0xB0, 0, 5, 0xA0, 0, 8, choffs);
    cache.block_freq = block_freq;

    // compute the keycode: block_freq is:
    //     BBBFFFFFFFFFFFF
    //     ^^^??
    // the 4-bit keycode uses the top 3 bits plus one of the next two bits
    let note_select = reg_byte(regdata, 0x08, 6, 1, 0);
    let mut keycode = bitfield(block_freq, 10, 3) << 1;
    keycode |= bitfield(block_freq, 9 - note_select as i32, 1);

    // no detune adjustment on OPL
    cache.detune = 0;

    // multiple value, as an x.1 value (0 means 0.5)
    // replace the low bit with a table lookup to give 0,1,2,3,4,5,6,7,8,9,10,10,12,12,15,15
    let multiple = reg_byte(regdata, 0x20, 0, 4, opoffs);
    cache.multiple = ((multiple & 0xE) | bitfield(0xC2AA, multiple as i32, 1)) * 2;
    if cache.multiple == 0 {
        cache.multiple = 1;
    }

    // phase step, or PHASE_STEP_DYNAMIC if PM is active
    let pm_enable = reg_byte(regdata, 0x20, 6, 1, opoffs);
    if pm_enable == 0 {
        let pm_val = 0;
        cache.phase_step = opl_compute_phase_step(cache.block_freq, cache.multiple, pm_val);
    } else {
        cache.phase_step = OpdataCache::PHASE_STEP_DYNAMIC;
    }

    // total level, scaled by 8
    cache.total_level = reg_byte(regdata, 0x40, 0, 6, opoffs) << 3;

    // pre-add key scale level
    let ksl_raw = reg_byte(regdata, 0x40, 6, 2, opoffs);
    let ksl = bitfield(ksl_raw, 1, 1) | (bitfield(ksl_raw, 0, 1) << 1);
    if ksl != 0 {
        cache.total_level +=
            opl_key_scale_atten(bitfield(block_freq, 10, 3), bitfield(block_freq, 6, 4)) << ksl;
    }

    // 4-bit sustain level, but 15 means 31 so effectively 5 bits
    cache.eg_sustain = reg_byte(regdata, 0x80, 4, 4, opoffs);
    cache.eg_sustain |= (cache.eg_sustain + 1) & 0x10;
    cache.eg_sustain <<= 5;

    // determine KSR adjustment for envelope rates
    let ksr = reg_byte(regdata, 0x20, 4, 1, opoffs);
    let ksrval = keycode >> (2 * (ksr ^ 1));
    cache.eg_rate[EnvelopeState::Attack as usize] =
        effective_rate(reg_byte(regdata, 0x60, 4, 4, opoffs) * 4, ksrval) as u8;
    cache.eg_rate[EnvelopeState::Decay as usize] =
        effective_rate(reg_byte(regdata, 0x60, 0, 4, opoffs) * 4, ksrval) as u8;
    let eg_sustain_flag = reg_byte(regdata, 0x20, 5, 1, opoffs);
    cache.eg_rate[EnvelopeState::Sustain as usize] = if eg_sustain_flag != 0 {
        0
    } else {
        effective_rate(reg_byte(regdata, 0x80, 0, 4, opoffs) * 4, ksrval) as u8
    };
    cache.eg_rate[EnvelopeState::Release as usize] =
        effective_rate(reg_byte(regdata, 0x80, 0, 4, opoffs) * 4, ksrval) as u8;
    cache.eg_rate[EnvelopeState::Depress as usize] = 0x3F;
}

fn opl_write<const REVISION: u32>(
    regdata: &mut [u8],
    index: u16,
    data: u8,
    channel: &mut u32,
    opmask: &mut u32,
) -> bool {
    let is_opl3_plus = REVISION >= 3;
    let reg_mode: u32 = 0x04;

    // writes to the mode register with high bit set ignore the low bits
    if index as u32 == reg_mode && bitfield(data as u32, 7, 1) != 0 {
        regdata[index as usize] |= 0x80;
    } else {
        regdata[index as usize] = data;
    }

    // handle writes to the rhythm keyons
    if index == 0xBD {
        let rhythm_channel = 0xFF_u32;
        *channel = rhythm_channel;
        *opmask = if bitfield(data as u32, 5, 1) != 0 {
            bitfield(data as u32, 0, 5)
        } else {
            0
        };
        return true;
    }

    // handle writes to the channel keyons
    if (index & 0xF0) == 0xB0 {
        *channel = (index & 0x0F) as u32;
        if *channel < 9 {
            if is_opl3_plus {
                *channel += 9 * bitfield(index as u32, 8, 1);
            }
            *opmask = if bitfield(data as u32, 5, 1) != 0 {
                15
            } else {
                0
            };
            return true;
        }
    }
    false
}

save_state::runtime_state! {
/// OPL registers for the YM3526.
#[derive(Clone)]
pub(crate) struct OplRegisters {
    lfo_am_counter: u16,
    lfo_pm_counter: u16,
    noise_lfsr: u32,
    lfo_am: u8,
    regdata: [u8; 0x100],
    waveform: [[u16; WAVEFORM_LENGTH]; 1],
}}

impl FmRegisters for OplRegisters {
    const OUTPUTS: usize = 1;
    const CHANNELS: usize = 9;
    const ALL_CHANNELS: u32 = (1 << 9) - 1;
    const OPERATORS: usize = 18;
    const DEFAULT_PRESCALE: u32 = 4;
    const EG_CLOCK_DIVIDER: u32 = 1;
    const CSM_TRIGGER_MASK: u32 = (1 << 9) - 1;
    const REG_MODE: u32 = 0x04;
    const EG_HAS_DEPRESS: bool = false;
    const EG_HAS_REVERB: bool = false;
    const EG_HAS_SSG: bool = false;
    const MODULATOR_DELAY: bool = true;
    const DYNAMIC_OPS: bool = false;

    const STATUS_TIMERA: u8 = 0x40;
    const STATUS_TIMERB: u8 = 0x20;
    const STATUS_BUSY: u8 = 0;
    const STATUS_IRQ: u8 = 0x80;

    const RHYTHM_CHANNEL: u32 = 0xFF;

    fn new() -> Self {
        let mut waveform = [[0u16; WAVEFORM_LENGTH]; 1];
        for (index, entry) in waveform[0].iter_mut().enumerate() {
            *entry = (abs_sin_attenuation(index as u32) | (bit(index as u32, 9) << 15)) as u16;
        }
        Self {
            lfo_am_counter: 0,
            lfo_pm_counter: 0,
            noise_lfsr: 1,
            lfo_am: 0,
            regdata: [0; 0x100],
            waveform,
        }
    }

    fn reset(&mut self) {
        self.regdata.fill(0);
    }

    fn operator_map(&self, index: usize) -> u32 {
        const FIXED_MAP: [u32; 9] = [
            operator_list(0, 3, 0xFF, 0xFF),
            operator_list(1, 4, 0xFF, 0xFF),
            operator_list(2, 5, 0xFF, 0xFF),
            operator_list(6, 9, 0xFF, 0xFF),
            operator_list(7, 10, 0xFF, 0xFF),
            operator_list(8, 11, 0xFF, 0xFF),
            operator_list(12, 15, 0xFF, 0xFF),
            operator_list(13, 16, 0xFF, 0xFF),
            operator_list(14, 17, 0xFF, 0xFF),
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
        opl_write::<1>(
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
        opnum + 2 * (opnum / 6)
    }

    fn op_ssg_eg_enable(&self, _opoffs: u32) -> u32 {
        0
    }

    fn op_ssg_eg_mode(&self, _opoffs: u32) -> u32 {
        0
    }

    fn op_lfo_am_enable(&self, opoffs: u32) -> u32 {
        reg_byte(&self.regdata, 0x20, 7, 1, opoffs)
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
        reg_byte(&self.regdata, 0xC0, 1, 3, choffs)
    }

    fn ch_algorithm(&self, choffs: u32) -> u32 {
        reg_byte(&self.regdata, 0xC0, 0, 1, choffs)
    }

    fn noise_state(&self) -> u32 {
        self.noise_lfsr >> 23
    }

    fn timer_a_value(&self) -> u32 {
        reg_byte(&self.regdata, 0x02, 0, 8, 0) * 4
    }

    fn timer_b_value(&self) -> u32 {
        reg_byte(&self.regdata, 0x03, 0, 8, 0)
    }

    fn csm(&self) -> u32 {
        reg_byte(&self.regdata, 0x08, 7, 1, 0)
    }

    fn reset_timer_a(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 7, 1, 0) | reg_byte(&self.regdata, 0x04, 6, 1, 0)
    }

    fn reset_timer_b(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 7, 1, 0) | reg_byte(&self.regdata, 0x04, 5, 1, 0)
    }

    fn enable_timer_a(&self) -> u32 {
        1
    }

    fn enable_timer_b(&self) -> u32 {
        1
    }

    fn load_timer_a(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 0, 1, 0)
    }

    fn load_timer_b(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 1, 1, 0)
    }

    fn cache_operator_data(&self, choffs: u32, opoffs: u32, cache: &mut OpdataCache) {
        opl_cache_operator_data::<1>(&self.regdata, 1, choffs, opoffs, cache);
    }

    fn compute_phase_step(
        &self,
        _choffs: u32,
        opoffs: u32,
        cache: &OpdataCache,
        lfo_raw_pm: i32,
    ) -> u32 {
        let pm_enable = reg_byte(&self.regdata, 0x20, 6, 1, opoffs);
        opl_compute_phase_step(
            cache.block_freq,
            cache.multiple,
            if pm_enable != 0 { lfo_raw_pm } else { 0 },
        )
    }

    fn clock_noise_and_lfo(&mut self) -> i32 {
        let am_depth = reg_byte(&self.regdata, 0xBD, 7, 1, 0);
        let pm_depth = reg_byte(&self.regdata, 0xBD, 6, 1, 0);
        opl_clock_noise_and_lfo(
            &mut self.noise_lfsr,
            &mut self.lfo_am_counter,
            &mut self.lfo_pm_counter,
            &mut self.lfo_am,
            am_depth,
            pm_depth,
        )
    }

    fn lfo_am_offset(&self, _choffs: u32) -> u32 {
        self.lfo_am as u32
    }

    fn waveform(&self, _index: u32, phase: u32) -> u16 {
        self.waveform[0][(phase & (WAVEFORM_LENGTH as u32 - 1)) as usize]
    }

    fn status_mask(&self) -> u8 {
        self.regdata[0x04] & 0x78
    }

    fn irq_reset(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 7, 1, 0)
    }

    fn noise_enable(&self) -> u32 {
        0
    }

    fn rhythm_enable(&self) -> u32 {
        reg_byte(&self.regdata, 0xBD, 5, 1, 0)
    }
}

save_state::runtime_state! {
/// OPL2 registers for the YM3812.
#[derive(Clone)]
pub(crate) struct Opl2Registers {
    lfo_am_counter: u16,
    lfo_pm_counter: u16,
    noise_lfsr: u32,
    lfo_am: u8,
    regdata: [u8; 0x100],
    waveform: [[u16; WAVEFORM_LENGTH]; 4],
}}

impl FmRegisters for Opl2Registers {
    const OUTPUTS: usize = 1;
    const CHANNELS: usize = 9;
    const ALL_CHANNELS: u32 = (1 << 9) - 1;
    const OPERATORS: usize = 18;
    const DEFAULT_PRESCALE: u32 = 4;
    const EG_CLOCK_DIVIDER: u32 = 1;
    const CSM_TRIGGER_MASK: u32 = (1 << 9) - 1;
    const REG_MODE: u32 = 0x04;
    const EG_HAS_DEPRESS: bool = false;
    const EG_HAS_REVERB: bool = false;
    const EG_HAS_SSG: bool = false;
    const MODULATOR_DELAY: bool = true;
    const DYNAMIC_OPS: bool = false;

    const STATUS_TIMERA: u8 = 0x40;
    const STATUS_TIMERB: u8 = 0x20;
    const STATUS_BUSY: u8 = 0;
    const STATUS_IRQ: u8 = 0x80;

    const RHYTHM_CHANNEL: u32 = 0xFF;

    fn new() -> Self {
        let mut waveform = [[0u16; WAVEFORM_LENGTH]; 4];
        // wf0: full sine
        for (index, entry) in waveform[0].iter_mut().enumerate() {
            *entry = (abs_sin_attenuation(index as u32) | (bit(index as u32, 9) << 15)) as u16;
        }
        let zeroval = waveform[0][0];
        let [wf0, wf1, wf2, wf3] = &mut waveform;
        for index in 0..WAVEFORM_LENGTH {
            // wf1: half sine (zero for negative half)
            wf1[index] = if bit(index as u32, 9) != 0 {
                zeroval
            } else {
                wf0[index]
            };
            // wf2: absolute sine (no sign bit)
            wf2[index] = wf0[index] & 0x7FFF;
            // wf3: quarter sine (zero for 2nd and 4th quarters)
            wf3[index] = if bit(index as u32, 8) != 0 {
                zeroval
            } else {
                wf0[index] & 0x7FFF
            };
        }
        Self {
            lfo_am_counter: 0,
            lfo_pm_counter: 0,
            noise_lfsr: 1,
            lfo_am: 0,
            regdata: [0; 0x100],
            waveform,
        }
    }

    fn reset(&mut self) {
        self.regdata.fill(0);
    }

    fn operator_map(&self, index: usize) -> u32 {
        const FIXED_MAP: [u32; 9] = [
            operator_list(0, 3, 0xFF, 0xFF),
            operator_list(1, 4, 0xFF, 0xFF),
            operator_list(2, 5, 0xFF, 0xFF),
            operator_list(6, 9, 0xFF, 0xFF),
            operator_list(7, 10, 0xFF, 0xFF),
            operator_list(8, 11, 0xFF, 0xFF),
            operator_list(12, 15, 0xFF, 0xFF),
            operator_list(13, 16, 0xFF, 0xFF),
            operator_list(14, 17, 0xFF, 0xFF),
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
        opl_write::<2>(
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
        opnum + 2 * (opnum / 6)
    }

    fn op_ssg_eg_enable(&self, _opoffs: u32) -> u32 {
        0
    }

    fn op_ssg_eg_mode(&self, _opoffs: u32) -> u32 {
        0
    }

    fn op_lfo_am_enable(&self, opoffs: u32) -> u32 {
        reg_byte(&self.regdata, 0x20, 7, 1, opoffs)
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
        reg_byte(&self.regdata, 0xC0, 1, 3, choffs)
    }

    fn ch_algorithm(&self, choffs: u32) -> u32 {
        reg_byte(&self.regdata, 0xC0, 0, 1, choffs)
    }

    fn noise_state(&self) -> u32 {
        self.noise_lfsr >> 23
    }

    fn timer_a_value(&self) -> u32 {
        reg_byte(&self.regdata, 0x02, 0, 8, 0) * 4
    }

    fn timer_b_value(&self) -> u32 {
        reg_byte(&self.regdata, 0x03, 0, 8, 0)
    }

    fn csm(&self) -> u32 {
        reg_byte(&self.regdata, 0x08, 7, 1, 0)
    }

    fn reset_timer_a(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 7, 1, 0) | reg_byte(&self.regdata, 0x04, 6, 1, 0)
    }

    fn reset_timer_b(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 7, 1, 0) | reg_byte(&self.regdata, 0x04, 5, 1, 0)
    }

    fn enable_timer_a(&self) -> u32 {
        1
    }

    fn enable_timer_b(&self) -> u32 {
        1
    }

    fn load_timer_a(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 0, 1, 0)
    }

    fn load_timer_b(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 1, 1, 0)
    }

    fn cache_operator_data(&self, choffs: u32, opoffs: u32, cache: &mut OpdataCache) {
        opl_cache_operator_data::<2>(&self.regdata, 4, choffs, opoffs, cache);
    }

    fn compute_phase_step(
        &self,
        _choffs: u32,
        opoffs: u32,
        cache: &OpdataCache,
        lfo_raw_pm: i32,
    ) -> u32 {
        let pm_enable = reg_byte(&self.regdata, 0x20, 6, 1, opoffs);
        opl_compute_phase_step(
            cache.block_freq,
            cache.multiple,
            if pm_enable != 0 { lfo_raw_pm } else { 0 },
        )
    }

    fn clock_noise_and_lfo(&mut self) -> i32 {
        let am_depth = reg_byte(&self.regdata, 0xBD, 7, 1, 0);
        let pm_depth = reg_byte(&self.regdata, 0xBD, 6, 1, 0);
        opl_clock_noise_and_lfo(
            &mut self.noise_lfsr,
            &mut self.lfo_am_counter,
            &mut self.lfo_pm_counter,
            &mut self.lfo_am,
            am_depth,
            pm_depth,
        )
    }

    fn lfo_am_offset(&self, _choffs: u32) -> u32 {
        self.lfo_am as u32
    }

    fn waveform(&self, index: u32, phase: u32) -> u16 {
        self.waveform[index as usize % 4][(phase & (WAVEFORM_LENGTH as u32 - 1)) as usize]
    }

    fn status_mask(&self) -> u8 {
        self.regdata[0x04] & 0x78
    }

    fn irq_reset(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 7, 1, 0)
    }

    fn noise_enable(&self) -> u32 {
        0
    }

    fn rhythm_enable(&self) -> u32 {
        reg_byte(&self.regdata, 0xBD, 5, 1, 0)
    }
}

save_state::runtime_state! {
/// OPL3 registers for the YMF262.
#[derive(Clone)]
pub(crate) struct Opl3Registers {
    lfo_am_counter: u16,
    lfo_pm_counter: u16,
    noise_lfsr: u32,
    lfo_am: u8,
    regdata: [u8; 0x200],
    waveform: [[u16; WAVEFORM_LENGTH]; 8],
}}

impl Opl3Registers {
    pub(crate) fn newflag(&self) -> u32 {
        reg_byte(&self.regdata, 0x105, 0, 1, 0)
    }

    fn fourop_enable(&self) -> u32 {
        reg_byte(&self.regdata, 0x104, 0, 6, 0)
    }
}

impl FmRegisters for Opl3Registers {
    const OUTPUTS: usize = 4;
    const CHANNELS: usize = 18;
    const ALL_CHANNELS: u32 = (1 << 18) - 1;
    const OPERATORS: usize = 36;
    const DEFAULT_PRESCALE: u32 = 8;
    const EG_CLOCK_DIVIDER: u32 = 1;
    const CSM_TRIGGER_MASK: u32 = (1 << 18) - 1;
    const REG_MODE: u32 = 0x04;
    const EG_HAS_DEPRESS: bool = false;
    const EG_HAS_REVERB: bool = false;
    const EG_HAS_SSG: bool = false;
    const MODULATOR_DELAY: bool = false;
    const DYNAMIC_OPS: bool = true;

    const STATUS_TIMERA: u8 = 0x40;
    const STATUS_TIMERB: u8 = 0x20;
    const STATUS_BUSY: u8 = 0;
    const STATUS_IRQ: u8 = 0x80;

    const RHYTHM_CHANNEL: u32 = 0xFF;

    fn new() -> Self {
        let mut waveform = [[0u16; WAVEFORM_LENGTH]; 8];
        // wf0: full sine
        for (index, entry) in waveform[0].iter_mut().enumerate() {
            *entry = (abs_sin_attenuation(index as u32) | (bit(index as u32, 9) << 15)) as u16;
        }
        let zeroval = waveform[0][0];
        let [wf0, wf1, wf2, wf3, wf4, wf5, wf6, wf7] = &mut waveform;
        for index in 0..WAVEFORM_LENGTH {
            // wf1: half sine
            wf1[index] = if bit(index as u32, 9) != 0 {
                zeroval
            } else {
                wf0[index]
            };
            // wf2: absolute sine
            wf2[index] = wf0[index] & 0x7FFF;
            // wf3: quarter sine
            wf3[index] = if bit(index as u32, 8) != 0 {
                zeroval
            } else {
                wf0[index] & 0x7FFF
            };
            // wf4: alternating sine (doubled frequency, half sine)
            wf4[index] = if bit(index as u32, 9) != 0 {
                zeroval
            } else {
                wf0[(index * 2) % WAVEFORM_LENGTH]
            };
            // wf5: absolute alternating sine (doubled frequency, abs half sine)
            wf5[index] = if bit(index as u32, 9) != 0 {
                zeroval
            } else {
                wf0[(index * 2) & 0x1FF]
            };
            // wf6: square wave
            wf6[index] = (bit(index as u32, 9) << 15) as u16;
            // wf7: derived exponential
            let idx = index as u32;
            wf7[index] = ((if bit(idx, 9) != 0 { idx ^ 0x13FF } else { idx }) << 3) as u16;
        }

        let mut regs = Self {
            lfo_am_counter: 0,
            lfo_pm_counter: 0,
            noise_lfsr: 1,
            lfo_am: 0,
            regdata: [0; 0x200],
            waveform,
        };
        // OPL3 has dynamic operators, so initialize the fourop_enable value here
        regs.regdata[0x104] = 0;
        regs
    }

    fn reset(&mut self) {
        self.regdata.fill(0);
    }

    fn operator_map(&self, index: usize) -> u32 {
        let fourop = self.fourop_enable();
        match index {
            0 => {
                if bit(fourop, 0) != 0 {
                    operator_list(0, 3, 6, 9)
                } else {
                    operator_list(0, 3, 0xFF, 0xFF)
                }
            }
            1 => {
                if bit(fourop, 1) != 0 {
                    operator_list(1, 4, 7, 10)
                } else {
                    operator_list(1, 4, 0xFF, 0xFF)
                }
            }
            2 => {
                if bit(fourop, 2) != 0 {
                    operator_list(2, 5, 8, 11)
                } else {
                    operator_list(2, 5, 0xFF, 0xFF)
                }
            }
            3 => {
                if bit(fourop, 0) != 0 {
                    operator_list(0xFF, 0xFF, 0xFF, 0xFF)
                } else {
                    operator_list(6, 9, 0xFF, 0xFF)
                }
            }
            4 => {
                if bit(fourop, 1) != 0 {
                    operator_list(0xFF, 0xFF, 0xFF, 0xFF)
                } else {
                    operator_list(7, 10, 0xFF, 0xFF)
                }
            }
            5 => {
                if bit(fourop, 2) != 0 {
                    operator_list(0xFF, 0xFF, 0xFF, 0xFF)
                } else {
                    operator_list(8, 11, 0xFF, 0xFF)
                }
            }
            6 => operator_list(12, 15, 0xFF, 0xFF),
            7 => operator_list(13, 16, 0xFF, 0xFF),
            8 => operator_list(14, 17, 0xFF, 0xFF),
            9 => {
                if bit(fourop, 3) != 0 {
                    operator_list(18, 21, 24, 27)
                } else {
                    operator_list(18, 21, 0xFF, 0xFF)
                }
            }
            10 => {
                if bit(fourop, 4) != 0 {
                    operator_list(19, 22, 25, 28)
                } else {
                    operator_list(19, 22, 0xFF, 0xFF)
                }
            }
            11 => {
                if bit(fourop, 5) != 0 {
                    operator_list(20, 23, 26, 29)
                } else {
                    operator_list(20, 23, 0xFF, 0xFF)
                }
            }
            12 => {
                if bit(fourop, 3) != 0 {
                    operator_list(0xFF, 0xFF, 0xFF, 0xFF)
                } else {
                    operator_list(24, 27, 0xFF, 0xFF)
                }
            }
            13 => {
                if bit(fourop, 4) != 0 {
                    operator_list(0xFF, 0xFF, 0xFF, 0xFF)
                } else {
                    operator_list(25, 28, 0xFF, 0xFF)
                }
            }
            14 => {
                if bit(fourop, 5) != 0 {
                    operator_list(0xFF, 0xFF, 0xFF, 0xFF)
                } else {
                    operator_list(26, 29, 0xFF, 0xFF)
                }
            }
            15 => operator_list(30, 33, 0xFF, 0xFF),
            16 => operator_list(31, 34, 0xFF, 0xFF),
            17 => operator_list(32, 35, 0xFF, 0xFF),
            _ => operator_list(0xFF, 0xFF, 0xFF, 0xFF),
        }
    }

    fn write(
        &mut self,
        index: u32,
        data: u8,
        keyon_channel: &mut u32,
        keyon_opmask: &mut u32,
    ) -> bool {
        opl_write::<3>(
            &mut self.regdata,
            index as u16,
            data,
            keyon_channel,
            keyon_opmask,
        )
    }

    fn channel_offset(chnum: u32) -> u32 {
        (chnum % 9) + 0x100 * (chnum / 9)
    }

    fn operator_offset(opnum: u32) -> u32 {
        (opnum % 18) + 2 * ((opnum % 18) / 6) + 0x100 * (opnum / 18)
    }

    fn op_ssg_eg_enable(&self, _opoffs: u32) -> u32 {
        0
    }

    fn op_ssg_eg_mode(&self, _opoffs: u32) -> u32 {
        0
    }

    fn op_lfo_am_enable(&self, opoffs: u32) -> u32 {
        reg_byte(&self.regdata, 0x20, 7, 1, opoffs)
    }

    fn ch_output_any(&self, choffs: u32) -> u32 {
        if self.newflag() != 0 {
            reg_byte(&self.regdata, 0xC0 + choffs, 4, 4, 0)
        } else {
            1
        }
    }

    fn ch_output_0(&self, choffs: u32) -> u32 {
        if self.newflag() != 0 {
            reg_byte(&self.regdata, 0xC0 + choffs, 4, 1, 0)
        } else {
            1
        }
    }

    fn ch_output_1(&self, choffs: u32) -> u32 {
        if self.newflag() != 0 {
            reg_byte(&self.regdata, 0xC0 + choffs, 5, 1, 0)
        } else {
            1
        }
    }

    fn ch_output_2(&self, choffs: u32) -> u32 {
        if self.newflag() != 0 {
            reg_byte(&self.regdata, 0xC0 + choffs, 6, 1, 0)
        } else {
            0
        }
    }

    fn ch_output_3(&self, choffs: u32) -> u32 {
        if self.newflag() != 0 {
            reg_byte(&self.regdata, 0xC0 + choffs, 7, 1, 0)
        } else {
            0
        }
    }

    fn ch_feedback(&self, choffs: u32) -> u32 {
        reg_byte(&self.regdata, 0xC0, 1, 3, choffs)
    }

    fn ch_algorithm(&self, choffs: u32) -> u32 {
        // OPL3: algorithm combines current channel's bit 0 with paired channel's bit 0
        // producing algorithm values 0-1 for 2-op or 8-11 for 4-op
        reg_byte(&self.regdata, 0xC0, 0, 1, choffs)
            | 8
            | (reg_byte(&self.regdata, 0xC3, 0, 1, choffs) << 1)
    }

    fn noise_state(&self) -> u32 {
        self.noise_lfsr >> 23
    }

    fn timer_a_value(&self) -> u32 {
        reg_byte(&self.regdata, 0x02, 0, 8, 0) * 4
    }

    fn timer_b_value(&self) -> u32 {
        reg_byte(&self.regdata, 0x03, 0, 8, 0)
    }

    fn csm(&self) -> u32 {
        0 // OPL3 does not support CSM
    }

    fn reset_timer_a(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 7, 1, 0) | reg_byte(&self.regdata, 0x04, 6, 1, 0)
    }

    fn reset_timer_b(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 7, 1, 0) | reg_byte(&self.regdata, 0x04, 5, 1, 0)
    }

    fn enable_timer_a(&self) -> u32 {
        1
    }

    fn enable_timer_b(&self) -> u32 {
        1
    }

    fn load_timer_a(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 0, 1, 0)
    }

    fn load_timer_b(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 1, 1, 0)
    }

    fn cache_operator_data(&self, choffs: u32, opoffs: u32, cache: &mut OpdataCache) {
        opl_cache_operator_data::<3>(&self.regdata, 8, choffs, opoffs, cache);
    }

    fn compute_phase_step(
        &self,
        _choffs: u32,
        opoffs: u32,
        cache: &OpdataCache,
        lfo_raw_pm: i32,
    ) -> u32 {
        let pm_enable = reg_byte(&self.regdata, 0x20, 6, 1, opoffs);
        opl_compute_phase_step(
            cache.block_freq,
            cache.multiple,
            if pm_enable != 0 { lfo_raw_pm } else { 0 },
        )
    }

    fn clock_noise_and_lfo(&mut self) -> i32 {
        let am_depth = reg_byte(&self.regdata, 0xBD, 7, 1, 0);
        let pm_depth = reg_byte(&self.regdata, 0xBD, 6, 1, 0);
        opl_clock_noise_and_lfo(
            &mut self.noise_lfsr,
            &mut self.lfo_am_counter,
            &mut self.lfo_pm_counter,
            &mut self.lfo_am,
            am_depth,
            pm_depth,
        )
    }

    fn lfo_am_offset(&self, _choffs: u32) -> u32 {
        self.lfo_am as u32
    }

    fn waveform(&self, index: u32, phase: u32) -> u16 {
        self.waveform[index as usize % 8][(phase & (WAVEFORM_LENGTH as u32 - 1)) as usize]
    }

    fn status_mask(&self) -> u8 {
        self.regdata[0x04] & 0x78
    }

    fn irq_reset(&self) -> u32 {
        reg_byte(&self.regdata, 0x04, 7, 1, 0)
    }

    fn noise_enable(&self) -> u32 {
        0
    }

    fn rhythm_enable(&self) -> u32 {
        reg_byte(&self.regdata, 0xBD, 5, 1, 0)
    }
}
