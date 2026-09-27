use crate::{
    fm::{FmRegisters, OpdataCache, effective_rate, operator_list},
    helpers::{bit, bitfield},
    tables::{abs_sin_attenuation, detune_adjustment, opn_lfo_pm_phase_adjustment},
};

// OPQ register map:
//
//      System-wide registers:
//           03 xxxxxxxx Timer control (unknown; 0x71 causes interrupts at ~10ms)
//           04 ----x--- LFO disable
//              -----xxx LFO frequency (0=~4Hz, 6=~10Hz, 7=~47Hz)
//           05 -x------ Key on/off operator 4
//              --x----- Key on/off operator 3
//              ---x---- Key on/off operator 2
//              ----x--- Key on/off operator 1
//              -----xxx Channel select
//
//     Per-channel registers (channel in address bits 0-2)
//        10-17 x------- Pan right
//              -x------ Pan left
//              --xxx--- Feedback level for operator 1 (0-7)
//              -----xxx Operator connection algorithm (0-7)
//        18-1F x------- Reverb
//              -xxx---- PM sensitivity
//              ------xx AM shift
//        20-27 -xxx---- Block (0-7), Operator 2 & 4
//              ----xxxx Frequency number upper 4 bits, Operator 2 & 4
//        28-2F -xxx---- Block (0-7), Operator 1 & 3
//              ----xxxx Frequency number upper 4 bits, Operator 1 & 3
//        30-37 xxxxxxxx Frequency number lower 8 bits, Operator 2 & 4
//        38-3F xxxxxxxx Frequency number lower 8 bits, Operator 1 & 3
//
//     Per-operator registers (channel in address bits 0-2, operator in bits 3-4)
//        40-5F 0-xxxxxx Detune value (0-63)
//              1---xxxx Multiple value (0-15)
//        60-7F -xxxxxxx Total level (0-127)
//        80-9F xx------ Key scale rate (0-3)
//              ---xxxxx Attack rate (0-31)
//        A0-BF x------- LFO AM enable, retrigger disable
//               x------ Waveform select
//              ---xxxxx Decay rate (0-31)
//        C0-DF ---xxxxx Sustain rate (0-31)
//        E0-FF xxxx---- Sustain level (0-15)
//              ----xxxx Release rate (0-15)
//
//     Internal (fake) registers:
//      100-11F ----xxxx Multiple value (0-15), remapped from 40-5F

const WAVEFORM_LENGTH: usize = 0x400;
const WAVEFORMS: usize = 2;
const REGISTERS: usize = 0x120;

// Envelope state indices into OpdataCache::eg_rate.
const EG_ATTACK: usize = 1;
const EG_DECAY: usize = 2;
const EG_SUSTAIN: usize = 3;
const EG_RELEASE: usize = 4;
const EG_REVERB: usize = 5;

// Frequency multiples as x.1 values.
const MULTIPLE_MAP: [u32; 16] = [1, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 24, 30, 32, 34, 36];

// LFO clock dividers, derived from the frequencies in the application manual
// for a 7-bit LFO value.
const LFO_MAX_COUNT: [u32; 8] = [109, 78, 72, 68, 63, 45, 9, 6];

save_state::runtime_state! {
/// Authoritative OPQ register and LFO state.
#[derive(Clone)]
pub(crate) struct OpqRegisters {
    lfo_counter: u32,
    lfo_am: u8,
    regdata: [u8; REGISTERS],
    waveform: [[u16; WAVEFORM_LENGTH]; WAVEFORMS],
}}

impl OpqRegisters {
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
    fn lfo_enable(&self) -> u32 {
        self.byte(0x04, 3, 1, 0) ^ 1
    }

    fn lfo_rate(&self) -> u32 {
        self.byte(0x04, 0, 3, 0)
    }

    // per-channel registers
    fn ch_reverb(&self, choffs: u32) -> u32 {
        self.byte(0x18, 7, 1, choffs)
    }

    fn ch_lfo_pm_sens(&self, choffs: u32) -> u32 {
        self.byte(0x18, 4, 3, choffs)
    }

    fn ch_lfo_am_sens(&self, choffs: u32) -> u32 {
        self.byte(0x18, 0, 2, choffs)
    }

    fn ch_block_freq_24(&self, choffs: u32) -> u32 {
        self.word(0x20, 0, 7, 0x30, 0, 8, choffs)
    }

    fn ch_block_freq_13(&self, choffs: u32) -> u32 {
        self.word(0x28, 0, 7, 0x38, 0, 8, choffs)
    }

    // per-operator registers
    fn op_detune(&self, opoffs: u32) -> u32 {
        self.byte(0x40, 0, 6, opoffs)
    }

    fn op_multiple(&self, opoffs: u32) -> u32 {
        self.byte(0x100, 0, 4, opoffs)
    }

    fn op_total_level(&self, opoffs: u32) -> u32 {
        self.byte(0x60, 0, 7, opoffs)
    }

    fn op_ksr(&self, opoffs: u32) -> u32 {
        self.byte(0x80, 6, 2, opoffs)
    }

    fn op_attack_rate(&self, opoffs: u32) -> u32 {
        self.byte(0x80, 0, 5, opoffs)
    }

    fn op_waveform(&self, opoffs: u32) -> u32 {
        self.byte(0xA0, 6, 1, opoffs)
    }

    fn op_decay_rate(&self, opoffs: u32) -> u32 {
        self.byte(0xA0, 0, 5, opoffs)
    }

    fn op_sustain_rate(&self, opoffs: u32) -> u32 {
        self.byte(0xC0, 0, 5, opoffs)
    }

    fn op_sustain_level(&self, opoffs: u32) -> u32 {
        self.byte(0xE0, 4, 4, opoffs)
    }

    fn op_release_rate(&self, opoffs: u32) -> u32 {
        self.byte(0xE0, 0, 4, opoffs)
    }

    fn phase_step_impl(&self, choffs: u32, cache: &OpdataCache, lfo_raw_pm: i32) -> u32 {
        // OPN-style phase calculation with a single detune parameter

        // extract frequency number (low 12 bits of block_freq)
        let mut fnum = bitfield(cache.block_freq, 0, 12);

        // apply the PM adjustment based on the upper 7 bits of FNUM
        let pm_sensitivity = self.ch_lfo_pm_sens(choffs);
        if pm_sensitivity != 0 {
            fnum = fnum.wrapping_add(opn_lfo_pm_phase_adjustment(
                bitfield(cache.block_freq, 5, 7),
                pm_sensitivity,
                lfo_raw_pm,
            ) as u32);
            fnum &= 0xFFF;
        }

        // apply block shift to compute phase step
        let block = bitfield(cache.block_freq, 12, 3);
        let mut phase_step = (fnum << block) >> 2;

        // apply detune based on the keycode, clamped to 17 bits
        phase_step = phase_step.wrapping_add(cache.detune as u32);
        phase_step &= 0x1FFFF;

        // apply frequency multiplier (which is cached as an x.1 value)
        (phase_step * cache.multiple) >> 1
    }
}

impl FmRegisters for OpqRegisters {
    const OUTPUTS: usize = 2;
    const CHANNELS: usize = 8;
    const ALL_CHANNELS: u32 = (1 << 8) - 1;
    const OPERATORS: usize = 32;
    const DEFAULT_PRESCALE: u32 = 2;
    const EG_CLOCK_DIVIDER: u32 = 3;
    const CSM_TRIGGER_MASK: u32 = (1 << 8) - 1;
    const REG_MODE: u32 = 0x03;
    const EG_HAS_DEPRESS: bool = false;
    const EG_HAS_REVERB: bool = true;
    const EG_HAS_SSG: bool = false;
    const MODULATOR_DELAY: bool = false;
    const DYNAMIC_OPS: bool = false;

    const STATUS_TIMERA: u8 = 0;
    const STATUS_TIMERB: u8 = 0x04;
    const STATUS_BUSY: u8 = 0x80;
    const STATUS_IRQ: u8 = 0;

    const RHYTHM_CHANNEL: u32 = 0xFF;

    fn new() -> Self {
        let mut waveform = [[0u16; WAVEFORM_LENGTH]; WAVEFORMS];
        for (index, entry) in waveform[0].iter_mut().enumerate() {
            *entry = (abs_sin_attenuation(index as u32) | (bit(index as u32, 9) << 15)) as u16;
        }

        // waveform 1 is a half sine that holds the zero value for the second half
        let sine = waveform[0];
        for (index, entry) in waveform[1].iter_mut().enumerate() {
            *entry = if bit(index as u32, 9) != 0 {
                sine[0]
            } else {
                sine[index]
            };
        }

        Self {
            lfo_counter: 0,
            lfo_am: 0,
            regdata: [0; REGISTERS],
            waveform,
        }
    }

    fn reset(&mut self) {
        self.regdata.fill(0);
        // enable output on both channels by default
        for offset in 0x10..=0x17 {
            self.regdata[offset] = 0xC0;
        }
    }

    // The operators keep their register order.
    fn operator_map(&self, index: usize) -> u32 {
        const FIXED_MAP: [u32; 8] = [
            operator_list(0, 8, 16, 24),
            operator_list(1, 9, 17, 25),
            operator_list(2, 10, 18, 26),
            operator_list(3, 11, 19, 27),
            operator_list(4, 12, 20, 28),
            operator_list(5, 13, 21, 29),
            operator_list(6, 14, 22, 30),
            operator_list(7, 15, 23, 31),
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
        // detune and multiple share a register; bit 7 of the data remaps the
        // multiple value to 100-11F
        let index = if (index & 0xE0) == 0x40 && bit(data as u32, 7) != 0 {
            index + 0xC0
        } else {
            index
        };

        self.regdata[index as usize] = data;

        // handle writes to the key on index
        if index == 0x05 {
            *keyon_channel = bitfield(data as u32, 0, 3);
            *keyon_opmask = bitfield(data as u32, 3, 4);
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
        self.byte(0x10, 6, 2, choffs)
    }

    fn ch_output_0(&self, choffs: u32) -> u32 {
        self.byte(0x10, 6, 1, choffs)
    }

    fn ch_output_1(&self, choffs: u32) -> u32 {
        self.byte(0x10, 7, 1, choffs)
    }

    fn ch_output_2(&self, _choffs: u32) -> u32 {
        0
    }

    fn ch_output_3(&self, _choffs: u32) -> u32 {
        0
    }

    fn ch_feedback(&self, choffs: u32) -> u32 {
        self.byte(0x10, 3, 3, choffs)
    }

    fn ch_algorithm(&self, choffs: u32) -> u32 {
        self.byte(0x10, 0, 3, choffs)
    }

    fn noise_state(&self) -> u32 {
        0
    }

    fn timer_a_value(&self) -> u32 {
        0
    }

    fn timer_b_value(&self) -> u32 {
        self.byte(0x03, 2, 6, 0) | 0xC0
    }

    fn csm(&self) -> u32 {
        0
    }

    fn reset_timer_a(&self) -> u32 {
        0
    }

    fn reset_timer_b(&self) -> u32 {
        self.byte(0x03, 0, 1, 0)
    }

    fn enable_timer_a(&self) -> u32 {
        0
    }

    fn enable_timer_b(&self) -> u32 {
        self.byte(0x03, 0, 1, 0)
    }

    fn load_timer_a(&self) -> u32 {
        0
    }

    fn load_timer_b(&self) -> u32 {
        self.byte(0x03, 0, 1, 0)
    }

    fn cache_operator_data(&self, choffs: u32, opoffs: u32, cache: &mut OpdataCache) {
        cache.waveform_index = self.op_waveform(opoffs);

        // operators 2 and 4 use the 20/30 frequency registers
        let block_freq = if opoffs & 8 != 0 {
            self.ch_block_freq_24(choffs)
        } else {
            self.ch_block_freq_13(choffs)
        };
        cache.block_freq = block_freq;

        // the 5-bit keycode follows OPN: the top 4 bits plus a bit derived
        // from F11 & (F10 | F9 | F8) | !F11 & F10 & F9 & F8
        let mut keycode = bitfield(block_freq, 11, 4) << 1;
        keycode |= bit(0xFE80, bitfield(block_freq, 8, 4) as i32);

        // the 6-bit detune is built from a sum of the 3-bit detune steps
        let detune = self.op_detune(opoffs) as i32 - 0x20;
        let abs_detune = detune.unsigned_abs();
        let adjust = (abs_detune / 3) as i32 * detune_adjustment(3, keycode)
            + detune_adjustment(abs_detune % 3, keycode);
        cache.detune = if detune >= 0 { adjust } else { -adjust };

        // multiple value, as an x.1 value
        cache.multiple = MULTIPLE_MAP[self.op_multiple(opoffs) as usize];

        // phase step, or PHASE_STEP_DYNAMIC if PM is active
        if self.lfo_enable() == 0 || self.ch_lfo_pm_sens(choffs) == 0 {
            cache.phase_step = self.phase_step_impl(choffs, cache, 0);
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
        cache.eg_rate[EG_REVERB] = if self.ch_reverb(choffs) != 0 {
            5 * 4
        } else {
            cache.eg_rate[EG_RELEASE]
        };
        cache.eg_shift = 0;
    }

    fn compute_phase_step(
        &mut self,
        choffs: u32,
        _opoffs: u32,
        cache: &OpdataCache,
        lfo_raw_pm: i32,
    ) -> u32 {
        self.phase_step_impl(choffs, cache, lfo_raw_pm)
    }

    fn clock_noise_and_lfo(&mut self) -> i32 {
        // the LFO follows OPN; a disabled LFO holds the counter and AM at 0
        if self.lfo_enable() == 0 {
            self.lfo_counter = 0;
            self.lfo_am = 0;
            return 0;
        }

        let subcount = self.lfo_counter as u8 as u32;
        self.lfo_counter = self.lfo_counter.wrapping_add(1);

        // crossing the divider count zeroes the low byte and increments the
        // 7-bit value in bits 8-14
        if subcount >= LFO_MAX_COUNT[self.lfo_rate() as usize] {
            self.lfo_counter = self.lfo_counter.wrapping_add(0x101 - subcount);
        }

        // AM value is 7 bits starting at bit 8; the first half of the period
        // is inverted
        self.lfo_am = bitfield(self.lfo_counter, 8, 6) as u8;
        if bit(self.lfo_counter, 8 + 6) == 0 {
            self.lfo_am ^= 0x3F;
        }

        // PM value is 5 bits starting at bit 10; bit 3 reflects it and bit 4
        // negates it
        let mut pm = bitfield(self.lfo_counter, 10, 3) as i32;
        if bit(self.lfo_counter, 10 + 3) != 0 {
            pm ^= 7;
        }
        if bit(self.lfo_counter, 10 + 4) != 0 {
            -pm
        } else {
            pm
        }
    }

    fn lfo_am_offset(&self, choffs: u32) -> u32 {
        // shift value for AM sensitivity is [*, 0, 1, 2]
        let am_sensitivity = self.ch_lfo_am_sens(choffs);
        if am_sensitivity == 0 {
            return 0;
        }
        (self.lfo_am as u32) << (am_sensitivity - 1)
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
        0
    }

    fn rhythm_enable(&self) -> u32 {
        0
    }
}
