use crate::{
    fm::{FmRegisters, OpdataCache, effective_rate, operator_list},
    helpers::{bit, bitfield},
    tables::{abs_sin_attenuation, detune_adjustment},
};

// OPM register map:
//
//      System-wide registers:
//           01 xxxxxx-x Test register
//              ------x- LFO reset
//           08 -x------ Key on/off operator 4
//              --x----- Key on/off operator 3
//              ---x---- Key on/off operator 2
//              ----x--- Key on/off operator 1
//              -----xxx Channel select
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
//           18 xxxxxxxx LFO frequency
//           19 0xxxxxxx AM LFO depth
//              1xxxxxxx PM LFO depth
//           1B xx------ CT (2 output data lines)
//              ------xx LFO waveform
//
//     Per-channel registers (channel in address bits 0-2)
//        20-27 x------- Pan right
//              -x------ Pan left
//              --xxx--- Feedback level for operator 1 (0-7)
//              -----xxx Operator connection algorithm (0-7)
//        28-2F -xxxxxxx Key code
//        30-37 xxxxxx-- Key fraction
//        38-3F -xxx---- LFO PM sensitivity
//              ------xx LFO AM shift
//
//     Per-operator registers (channel in address bits 0-2, operator in bits 3-4)
//        40-5F -xxx---- Detune value (0-7)
//              ----xxxx Multiple value (0-15)
//        60-7F -xxxxxxx Total level (0-127)
//        80-9F xx------ Key scale rate (0-3)
//              ---xxxxx Attack rate (0-31)
//        A0-BF x------- LFO AM enable
//              ---xxxxx Decay rate (0-31)
//        C0-DF xx------ Detune 2 value (0-3)
//              ---xxxxx Sustain rate (0-31)
//        E0-FF xxxx---- Sustain level (0-15)
//              ----xxxx Release rate (0-15)
//
//     Internal (fake) registers:
//           1A -xxxxxxx PM depth

const WAVEFORM_LENGTH: usize = 0x400;
const LFO_WAVEFORM_LENGTH: usize = 256;

// Envelope state indices into OpdataCache::eg_rate.
const EG_ATTACK: usize = 1;
const EG_DECAY: usize = 2;
const EG_SUSTAIN: usize = 3;
const EG_RELEASE: usize = 4;

// Detune 2 delta, in 1/64ths of a semitone. The manual gives the coarse
// detune values in cents (0, 600, 781, 950); each is converted as
// (cents * 64 + 50) / 100 and rounded, giving 0, 384, 500, 608.
const DETUNE2_DELTA: [i16; 4] = [0, 384, 500, 608];

// Converts an OPM concatenated block (3 bits), keycode (4 bits) and key
// fraction (6 bits) to a 0.10 phase step after applying the given delta. The
// table comes from David Viens' analysis of a real chip.
fn opm_key_code_to_phase_step(block_freq: u32, delta: i32) -> u32 {
    static PHASE_STEP: [u32; 12 * 64] = [
        41568, 41600, 41632, 41664, 41696, 41728, 41760, 41792, 41856, 41888, 41920, 41952, 42016,
        42048, 42080, 42112, 42176, 42208, 42240, 42272, 42304, 42336, 42368, 42400, 42464, 42496,
        42528, 42560, 42624, 42656, 42688, 42720, 42784, 42816, 42848, 42880, 42912, 42944, 42976,
        43008, 43072, 43104, 43136, 43168, 43232, 43264, 43296, 43328, 43392, 43424, 43456, 43488,
        43552, 43584, 43616, 43648, 43712, 43744, 43776, 43808, 43872, 43904, 43936, 43968, 44032,
        44064, 44096, 44128, 44192, 44224, 44256, 44288, 44352, 44384, 44416, 44448, 44512, 44544,
        44576, 44608, 44672, 44704, 44736, 44768, 44832, 44864, 44896, 44928, 44992, 45024, 45056,
        45088, 45152, 45184, 45216, 45248, 45312, 45344, 45376, 45408, 45472, 45504, 45536, 45568,
        45632, 45664, 45728, 45760, 45792, 45824, 45888, 45920, 45984, 46016, 46048, 46080, 46144,
        46176, 46208, 46240, 46304, 46336, 46368, 46400, 46464, 46496, 46528, 46560, 46656, 46688,
        46720, 46752, 46816, 46848, 46880, 46912, 46976, 47008, 47072, 47104, 47136, 47168, 47232,
        47264, 47328, 47360, 47392, 47424, 47488, 47520, 47552, 47584, 47648, 47680, 47744, 47776,
        47808, 47840, 47904, 47936, 48032, 48064, 48096, 48128, 48192, 48224, 48288, 48320, 48384,
        48416, 48448, 48480, 48544, 48576, 48640, 48672, 48736, 48768, 48800, 48832, 48896, 48928,
        48992, 49024, 49088, 49120, 49152, 49184, 49248, 49280, 49344, 49376, 49440, 49472, 49504,
        49536, 49600, 49632, 49696, 49728, 49792, 49824, 49856, 49888, 49952, 49984, 50048, 50080,
        50144, 50176, 50208, 50240, 50304, 50336, 50400, 50432, 50496, 50528, 50560, 50592, 50656,
        50688, 50752, 50784, 50880, 50912, 50944, 50976, 51040, 51072, 51136, 51168, 51232, 51264,
        51328, 51360, 51424, 51456, 51488, 51520, 51616, 51648, 51680, 51712, 51776, 51808, 51872,
        51904, 51968, 52000, 52064, 52096, 52160, 52192, 52224, 52256, 52384, 52416, 52448, 52480,
        52544, 52576, 52640, 52672, 52736, 52768, 52832, 52864, 52928, 52960, 52992, 53024, 53120,
        53152, 53216, 53248, 53312, 53344, 53408, 53440, 53504, 53536, 53600, 53632, 53696, 53728,
        53792, 53824, 53920, 53952, 54016, 54048, 54112, 54144, 54208, 54240, 54304, 54336, 54400,
        54432, 54496, 54528, 54592, 54624, 54688, 54720, 54784, 54816, 54880, 54912, 54976, 55008,
        55072, 55104, 55168, 55200, 55264, 55296, 55360, 55392, 55488, 55520, 55584, 55616, 55680,
        55712, 55776, 55808, 55872, 55936, 55968, 56032, 56064, 56128, 56160, 56224, 56288, 56320,
        56384, 56416, 56480, 56512, 56576, 56608, 56672, 56736, 56768, 56832, 56864, 56928, 56960,
        57024, 57120, 57152, 57216, 57248, 57312, 57376, 57408, 57472, 57536, 57568, 57632, 57664,
        57728, 57792, 57824, 57888, 57952, 57984, 58048, 58080, 58144, 58208, 58240, 58304, 58368,
        58400, 58464, 58496, 58560, 58624, 58656, 58720, 58784, 58816, 58880, 58912, 58976, 59040,
        59072, 59136, 59200, 59232, 59296, 59328, 59392, 59456, 59488, 59552, 59648, 59680, 59744,
        59776, 59840, 59904, 59936, 60000, 60064, 60128, 60160, 60224, 60288, 60320, 60384, 60416,
        60512, 60544, 60608, 60640, 60704, 60768, 60800, 60864, 60928, 60992, 61024, 61088, 61152,
        61184, 61248, 61280, 61376, 61408, 61472, 61536, 61600, 61632, 61696, 61760, 61824, 61856,
        61920, 61984, 62048, 62080, 62144, 62208, 62272, 62304, 62368, 62432, 62496, 62528, 62592,
        62656, 62720, 62752, 62816, 62880, 62944, 62976, 63040, 63104, 63200, 63232, 63296, 63360,
        63424, 63456, 63520, 63584, 63648, 63680, 63744, 63808, 63872, 63904, 63968, 64032, 64096,
        64128, 64192, 64256, 64320, 64352, 64416, 64480, 64544, 64608, 64672, 64704, 64768, 64832,
        64896, 64928, 65024, 65056, 65120, 65184, 65248, 65312, 65376, 65408, 65504, 65536, 65600,
        65664, 65728, 65792, 65856, 65888, 65984, 66016, 66080, 66144, 66208, 66272, 66336, 66368,
        66464, 66496, 66560, 66624, 66688, 66752, 66816, 66848, 66944, 66976, 67040, 67104, 67168,
        67232, 67296, 67328, 67424, 67456, 67520, 67584, 67648, 67712, 67776, 67808, 67904, 67936,
        68000, 68064, 68128, 68192, 68256, 68288, 68384, 68448, 68512, 68544, 68640, 68672, 68736,
        68800, 68896, 68928, 68992, 69056, 69120, 69184, 69248, 69280, 69376, 69440, 69504, 69536,
        69632, 69664, 69728, 69792, 69920, 69952, 70016, 70080, 70144, 70208, 70272, 70304, 70400,
        70464, 70528, 70560, 70656, 70688, 70752, 70816, 70912, 70976, 71040, 71104, 71136, 71232,
        71264, 71360, 71424, 71488, 71552, 71616, 71648, 71744, 71776, 71872, 71968, 72032, 72096,
        72160, 72192, 72288, 72320, 72416, 72480, 72544, 72608, 72672, 72704, 72800, 72832, 72928,
        72992, 73056, 73120, 73184, 73216, 73312, 73344, 73440, 73504, 73568, 73632, 73696, 73728,
        73824, 73856, 73952, 74080, 74144, 74208, 74272, 74304, 74400, 74432, 74528, 74592, 74656,
        74720, 74784, 74816, 74912, 74944, 75040, 75136, 75200, 75264, 75328, 75360, 75456, 75488,
        75584, 75648, 75712, 75776, 75840, 75872, 75968, 76000, 76096, 76224, 76288, 76352, 76416,
        76448, 76544, 76576, 76672, 76736, 76800, 76864, 76928, 77024, 77120, 77152, 77248, 77344,
        77408, 77472, 77536, 77568, 77664, 77696, 77792, 77856, 77920, 77984, 78048, 78144, 78240,
        78272, 78368, 78464, 78528, 78592, 78656, 78688, 78784, 78816, 78912, 78976, 79040, 79104,
        79168, 79264, 79360, 79392, 79488, 79616, 79680, 79744, 79808, 79840, 79936, 79968, 80064,
        80128, 80192, 80256, 80320, 80416, 80512, 80544, 80640, 80768, 80832, 80896, 80960, 80992,
        81088, 81120, 81216, 81280, 81344, 81408, 81472, 81568, 81664, 81696, 81792, 81952, 82016,
        82080, 82144, 82176, 82272, 82304, 82400, 82464, 82528, 82592, 82656, 82752, 82848, 82880,
        82976,
    ];

    // extract the block (octave) first
    let mut block = bitfield(block_freq, 10, 3);

    // the keycode (bits 6-9) is "gappy", mapping 12 values over 16 in each
    // octave; to correct for this, we multiply the 4-bit value by 3/4 (or
    // rather subtract 1/4)
    let adjusted_code = bitfield(block_freq, 6, 4) - bitfield(block_freq, 8, 2);

    // now re-insert the 6-bit fraction
    let mut eff_freq = ((adjusted_code << 6) | bitfield(block_freq, 0, 6)) as i32;

    // now that the gaps are removed, add the delta
    eff_freq += delta;

    // handle over/underflow by adjusting the block
    if (eff_freq as u32) >= 768 {
        if eff_freq < 0 {
            // minimum delta is -512 (PM), so we can only underflow by 1 octave
            eff_freq += 768;
            if block == 0 {
                return PHASE_STEP[0] >> 7;
            }
            block -= 1;
        } else {
            // maximum delta is +512+608 (PM+detune), so we can overflow by up
            // to 2 octaves
            eff_freq -= 768;
            if eff_freq >= 768 {
                block += 1;
                eff_freq -= 768;
            }
            if block >= 7 {
                return PHASE_STEP[767];
            }
            block += 1;
        }
    }

    // look up the phase shift for the key code, then shift by octave
    PHASE_STEP[eff_freq as usize] >> (block ^ 7)
}

save_state::runtime_state! {
/// Authoritative OPM register, noise, and timer state.
#[derive(Clone)]
pub(crate) struct OpmRegisters {
    lfo_counter: u32,
    noise_lfsr: u32,
    noise_counter: u8,
    noise_state: u8,
    lfo_am: u8,
    regdata: [u8; 0x100],
    // LFO waveforms; AM in the low 8 bits, PM in the upper 8.
    lfo_waveform: [[i16; LFO_WAVEFORM_LENGTH]; 4],
    waveform: [u16; WAVEFORM_LENGTH],
}}

impl OpmRegisters {
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
    fn lfo_reset(&self) -> u32 {
        self.byte(0x01, 1, 1, 0)
    }

    fn noise_frequency(&self) -> u32 {
        self.byte(0x0F, 0, 5, 0) ^ 0x1F
    }

    fn lfo_rate(&self) -> u32 {
        self.byte(0x18, 0, 8, 0)
    }

    fn lfo_am_depth(&self) -> u32 {
        self.byte(0x19, 0, 7, 0)
    }

    fn lfo_pm_depth(&self) -> u32 {
        self.byte(0x1A, 0, 7, 0)
    }

    fn lfo_waveform_select(&self) -> u32 {
        self.byte(0x1B, 0, 2, 0)
    }

    // per-channel registers
    fn ch_block_freq(&self, choffs: u32) -> u32 {
        self.word(0x28, 0, 7, 0x30, 2, 6, choffs)
    }

    fn ch_lfo_pm_sens(&self, choffs: u32) -> u32 {
        self.byte(0x38, 4, 3, choffs)
    }

    fn ch_lfo_am_sens(&self, choffs: u32) -> u32 {
        self.byte(0x38, 0, 2, choffs)
    }

    // per-operator registers
    fn op_detune(&self, opoffs: u32) -> u32 {
        self.byte(0x40, 4, 3, opoffs)
    }

    fn op_multiple(&self, opoffs: u32) -> u32 {
        self.byte(0x40, 0, 4, opoffs)
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

    fn op_decay_rate(&self, opoffs: u32) -> u32 {
        self.byte(0xA0, 0, 5, opoffs)
    }

    fn op_detune2(&self, opoffs: u32) -> u32 {
        self.byte(0xC0, 6, 2, opoffs)
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

    fn phase_step_impl(
        &self,
        choffs: u32,
        opoffs: u32,
        cache: &OpdataCache,
        lfo_raw_pm: i32,
    ) -> u32 {
        // OPM logic is rather unique here, due to extra detune and the use of
        // key codes (not to be confused with keycode)

        // start with coarse detune delta
        let mut delta = DETUNE2_DELTA[self.op_detune2(opoffs) as usize] as i32;

        // add in the PM delta
        let pm_sensitivity = self.ch_lfo_pm_sens(choffs);
        if pm_sensitivity != 0 {
            // raw PM value is -127..128 which is +/- 200 cents; roughly
            // corresponds to shifting the 200-cent value
            if pm_sensitivity < 6 {
                delta += lfo_raw_pm >> (6 - pm_sensitivity);
            } else {
                delta = delta.wrapping_add(((lfo_raw_pm as u32) << (pm_sensitivity - 5)) as i32);
            }
        }

        // apply delta and convert to a frequency number
        let mut phase_step = opm_key_code_to_phase_step(cache.block_freq, delta);

        // apply detune based on the keycode
        phase_step = (phase_step as i32 + cache.detune) as u32;

        // apply frequency multiplier (which is cached as an x.1 value)
        (phase_step * cache.multiple) >> 1
    }
}

impl FmRegisters for OpmRegisters {
    const OUTPUTS: usize = 2;
    const CHANNELS: usize = 8;
    const ALL_CHANNELS: u32 = (1 << 8) - 1;
    const OPERATORS: usize = 32;
    const DEFAULT_PRESCALE: u32 = 2;
    const EG_CLOCK_DIVIDER: u32 = 3;
    const CSM_TRIGGER_MASK: u32 = (1 << 8) - 1;
    const REG_MODE: u32 = 0x14;
    const EG_HAS_DEPRESS: bool = false;
    const EG_HAS_REVERB: bool = false;
    const EG_HAS_SSG: bool = false;
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
            lfo_counter: 0,
            noise_lfsr: 1,
            noise_counter: 0,
            noise_state: 0,
            lfo_am: 0,
            regdata: [0; 0x100],
            lfo_waveform,
            waveform,
        }
    }

    fn reset(&mut self) {
        self.regdata.fill(0);
        // enable output on both channels by default
        for offset in 0x20..=0x27 {
            self.regdata[offset] = 0xC0;
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
        // LFO AM/PM depth are written to the same register (0x19); redirect the
        // PM depth to an unused neighbor (0x1a)
        if index == 0x19 {
            self.regdata[(index + bit(data as u32, 7)) as usize] = data;
        } else if index != 0x1A {
            self.regdata[index as usize] = data;
        }

        // handle writes to the key on index
        if index == 0x08 {
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
        self.byte(0x20, 6, 2, choffs)
    }

    fn ch_output_0(&self, choffs: u32) -> u32 {
        self.byte(0x20, 6, 1, choffs)
    }

    fn ch_output_1(&self, choffs: u32) -> u32 {
        self.byte(0x20, 7, 1, choffs)
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
        // OPM always uses waveform 0 (pure sine)
        cache.waveform_index = 0;

        // get frequency from the channel
        let block_freq = self.ch_block_freq(choffs);
        cache.block_freq = block_freq;

        // the 5-bit keycode is just the top 5 bits (block + top 2 bits of the
        // key code)
        let keycode = bitfield(block_freq, 8, 5);

        // detune adjustment
        cache.detune = detune_adjustment(self.op_detune(opoffs), keycode);

        // multiple value, as an x.1 value (0 means 0.5)
        cache.multiple = self.op_multiple(opoffs) * 2;
        if cache.multiple == 0 {
            cache.multiple = 1;
        }

        // phase step, or PHASE_STEP_DYNAMIC if PM is active
        if self.lfo_pm_depth() == 0 || self.ch_lfo_pm_sens(choffs) == 0 {
            cache.phase_step = self.phase_step_impl(choffs, opoffs, cache, 0);
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
    }

    fn compute_phase_step(
        &self,
        choffs: u32,
        opoffs: u32,
        cache: &OpdataCache,
        lfo_raw_pm: i32,
    ) -> u32 {
        self.phase_step_impl(choffs, opoffs, cache, lfo_raw_pm)
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

        // treat the rate as a 4.4 floating-point step value with implied
        // leading 1
        let rate = self.lfo_rate();
        self.lfo_counter = self
            .lfo_counter
            .wrapping_add((0x10 | bitfield(rate, 0, 4)) << bitfield(rate, 4, 4));

        // bit 1 of the test register holds the LFO in reset while active
        if self.lfo_reset() != 0 {
            self.lfo_counter = 0;
        }

        // now pull out the non-fractional LFO value
        let lfo = bitfield(self.lfo_counter, 22, 8);

        // fill in the noise entry 1 ahead of our current position; this ensures
        // the current value remains stable for a full LFO clock
        let lfo_noise = bitfield(self.noise_lfsr, 17, 8);
        self.lfo_waveform[3][((lfo + 1) & 0xFF) as usize] = (lfo_noise | (lfo_noise << 8)) as i16;

        // fetch the AM/PM values based on the waveform; AM is unsigned in the
        // low 8 bits, PM is signed in the upper 8 bits
        let ampm = self.lfo_waveform[self.lfo_waveform_select() as usize][lfo as usize] as i32;

        // apply depth to the AM value and store for later
        self.lfo_am = (((ampm & 0xFF) * self.lfo_am_depth() as i32) >> 7) as u8;

        // apply depth to the PM value and return it
        ((ampm >> 8) * self.lfo_pm_depth() as i32) >> 7
    }

    fn lfo_am_offset(&self, choffs: u32) -> u32 {
        // OPM maps AM quite differently from OPN; shift value for AM
        // sensitivity is [*, 0, 1, 2]
        let am_sensitivity = self.ch_lfo_am_sens(choffs);
        if am_sensitivity == 0 {
            return 0;
        }

        // raw LFO AM value on OPM is 0-FF, already a factor of 2 larger than OPN
        (self.lfo_am as u32) << (am_sensitivity - 1)
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
        self.byte(0x0F, 7, 1, 0)
    }

    fn rhythm_enable(&self) -> u32 {
        0
    }
}
