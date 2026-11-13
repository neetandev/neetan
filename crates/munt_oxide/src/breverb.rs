// Copyright (C) 2003, 2004, 2005, 2006, 2008, 2009 Dean Beeler, Jerome Fisher
// Copyright (C) 2011-2022 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
//  This program is free software: you can redistribute it and/or modify
//  it under the terms of the GNU Lesser General Public License as published by
//  the Free Software Foundation, either version 2.1 of the License, or
//  (at your option) any later version.
//
//  This program is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU Lesser General Public License for more details.
//
//  You should have received a copy of the GNU Lesser General Public License
//  along with this program.  If not, see <http://www.gnu.org/licenses/>.

// Analysing of state of reverb RAM address lines gives exact sizes of the buffers of filters used. This also indicates that
// the reverb model implemented in the real devices consists of three series allpass filters preceded by a non-feedback comb (or a delay with a LPF)
// and followed by three parallel comb filters

use crate::{
    enumerations::ReverbMode,
    state::{
        BReverbModelState, ReverbAllpassState, ReverbCombState, ReverbDelayWithLpfState,
        ReverbRingBuffer, ReverbTapDelayCombState,
    },
};

// Because LA-32 chip makes it's output available to process by the Boss chip with a significant delay,
// the Boss chip puts to the buffer the LA32 dry output when it is ready and performs processing of the _previously_ latched data.
// Of course, the right way would be to use a dedicated variable for this, but our reverb model is way higher level,
// so we can simply increase the input buffer size.
const PROCESS_DELAY: u32 = 1;

const MODE_3_ADDITIONAL_DELAY: u32 = 1;
const MODE_3_FEEDBACK_DELAY: u32 = 1;

// Avoid denormals degrading performance, using biased input
const BIAS: f32 = 1e-20;

struct BReverbSettings {
    number_of_allpasses: u32,
    allpass_sizes: &'static [u32],
    number_of_combs: u32,
    comb_sizes: &'static [u32],
    out_l_positions: &'static [u32],
    out_r_positions: &'static [u32],
    filter_factors: &'static [u8],
    feedback_factors: &'static [u8],
    dry_amps: &'static [u8],
    wet_levels: &'static [u8],
    lpf_amp: u8,
}

// Default reverb settings for "new" reverb model implemented in CM-32L / LAPC-I.
// Found by tracing reverb RAM data lines (thanks go to Lord_Nightmare & balrog).
fn get_cm32l_lapc_settings(mode: ReverbMode) -> &'static BReverbSettings {
    static MODE_0_ALLPASSES: [u32; 3] = [994, 729, 78];
    // Well, actually there are 3 comb filters, but the entrance LPF + delay can be processed via a hacked comb.
    static MODE_0_COMBS: [u32; 4] = [705 + PROCESS_DELAY, 2349, 2839, 3632];
    static MODE_0_OUTL: [u32; 3] = [2349, 141, 1960];
    static MODE_0_OUTR: [u32; 3] = [1174, 1570, 145];
    static MODE_0_COMB_FACTOR: [u8; 4] = [0xA0, 0x60, 0x60, 0x60];
    #[rustfmt::skip]
    static MODE_0_COMB_FEEDBACK: [u8; 32] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x28, 0x48, 0x60, 0x78, 0x80, 0x88, 0x90, 0x98,
        0x28, 0x48, 0x60, 0x78, 0x80, 0x88, 0x90, 0x98,
        0x28, 0x48, 0x60, 0x78, 0x80, 0x88, 0x90, 0x98,
    ];
    static MODE_0_DRY_AMP: [u8; 8] = [0xA0, 0xA0, 0xA0, 0xA0, 0xB0, 0xB0, 0xB0, 0xD0];
    static MODE_0_WET_AMP: [u8; 8] = [0x10, 0x30, 0x50, 0x70, 0x90, 0xC0, 0xF0, 0xF0];
    static MODE_0_LPF_AMP: u8 = 0x60;

    static MODE_1_ALLPASSES: [u32; 3] = [1324, 809, 176];
    // Same as for mode 0 above
    static MODE_1_COMBS: [u32; 4] = [961 + PROCESS_DELAY, 2619, 3545, 4519];
    static MODE_1_OUTL: [u32; 3] = [2618, 1760, 4518];
    static MODE_1_OUTR: [u32; 3] = [1300, 3532, 2274];
    static MODE_1_COMB_FACTOR: [u8; 4] = [0x80, 0x60, 0x60, 0x60];
    #[rustfmt::skip]
    static MODE_1_COMB_FEEDBACK: [u8; 32] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x28, 0x48, 0x60, 0x70, 0x78, 0x80, 0x90, 0x98,
        0x28, 0x48, 0x60, 0x78, 0x80, 0x88, 0x90, 0x98,
        0x28, 0x48, 0x60, 0x78, 0x80, 0x88, 0x90, 0x98,
    ];
    static MODE_1_DRY_AMP: [u8; 8] = [0xA0, 0xA0, 0xB0, 0xB0, 0xB0, 0xB0, 0xB0, 0xE0];
    static MODE_1_WET_AMP: [u8; 8] = [0x10, 0x30, 0x50, 0x70, 0x90, 0xC0, 0xF0, 0xF0];
    static MODE_1_LPF_AMP: u8 = 0x60;

    static MODE_2_ALLPASSES: [u32; 3] = [969, 644, 157];
    // Same as for mode 0 above
    static MODE_2_COMBS: [u32; 4] = [116 + PROCESS_DELAY, 2259, 2839, 3539];
    static MODE_2_OUTL: [u32; 3] = [2259, 718, 1769];
    static MODE_2_OUTR: [u32; 3] = [1136, 2128, 1];
    static MODE_2_COMB_FACTOR: [u8; 4] = [0, 0x20, 0x20, 0x20];
    #[rustfmt::skip]
    static MODE_2_COMB_FEEDBACK: [u8; 32] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x30, 0x58, 0x78, 0x88, 0xA0, 0xB8, 0xC0, 0xD0,
        0x30, 0x58, 0x78, 0x88, 0xA0, 0xB8, 0xC0, 0xD0,
        0x30, 0x58, 0x78, 0x88, 0xA0, 0xB8, 0xC0, 0xD0,
    ];
    static MODE_2_DRY_AMP: [u8; 8] = [0xA0, 0xA0, 0xB0, 0xB0, 0xB0, 0xB0, 0xC0, 0xE0];
    static MODE_2_WET_AMP: [u8; 8] = [0x10, 0x30, 0x50, 0x70, 0x90, 0xC0, 0xF0, 0xF0];
    static MODE_2_LPF_AMP: u8 = 0x80;

    static MODE_3_DELAY: [u32; 1] =
        [16000 + MODE_3_FEEDBACK_DELAY + PROCESS_DELAY + MODE_3_ADDITIONAL_DELAY];
    static MODE_3_OUTL: [u32; 8] = [400, 624, 960, 1488, 2256, 3472, 5280, 8000];
    static MODE_3_OUTR: [u32; 8] = [800, 1248, 1920, 2976, 4512, 6944, 10560, 16000];
    static MODE_3_COMB_FACTOR: [u8; 1] = [0x68];
    static MODE_3_COMB_FEEDBACK: [u8; 2] = [0x68, 0x60];
    #[rustfmt::skip]
    static MODE_3_DRY_AMP: [u8; 16] = [
        0x20, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50,
        0x20, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50,
    ];
    static MODE_3_WET_AMP: [u8; 8] = [0x18, 0x18, 0x28, 0x40, 0x60, 0x80, 0xA8, 0xF8];

    static REVERB_MODE_0_SETTINGS: BReverbSettings = BReverbSettings {
        number_of_allpasses: 3,
        allpass_sizes: &MODE_0_ALLPASSES,
        number_of_combs: 4,
        comb_sizes: &MODE_0_COMBS,
        out_l_positions: &MODE_0_OUTL,
        out_r_positions: &MODE_0_OUTR,
        filter_factors: &MODE_0_COMB_FACTOR,
        feedback_factors: &MODE_0_COMB_FEEDBACK,
        dry_amps: &MODE_0_DRY_AMP,
        wet_levels: &MODE_0_WET_AMP,
        lpf_amp: MODE_0_LPF_AMP,
    };
    static REVERB_MODE_1_SETTINGS: BReverbSettings = BReverbSettings {
        number_of_allpasses: 3,
        allpass_sizes: &MODE_1_ALLPASSES,
        number_of_combs: 4,
        comb_sizes: &MODE_1_COMBS,
        out_l_positions: &MODE_1_OUTL,
        out_r_positions: &MODE_1_OUTR,
        filter_factors: &MODE_1_COMB_FACTOR,
        feedback_factors: &MODE_1_COMB_FEEDBACK,
        dry_amps: &MODE_1_DRY_AMP,
        wet_levels: &MODE_1_WET_AMP,
        lpf_amp: MODE_1_LPF_AMP,
    };
    static REVERB_MODE_2_SETTINGS: BReverbSettings = BReverbSettings {
        number_of_allpasses: 3,
        allpass_sizes: &MODE_2_ALLPASSES,
        number_of_combs: 4,
        comb_sizes: &MODE_2_COMBS,
        out_l_positions: &MODE_2_OUTL,
        out_r_positions: &MODE_2_OUTR,
        filter_factors: &MODE_2_COMB_FACTOR,
        feedback_factors: &MODE_2_COMB_FEEDBACK,
        dry_amps: &MODE_2_DRY_AMP,
        wet_levels: &MODE_2_WET_AMP,
        lpf_amp: MODE_2_LPF_AMP,
    };
    static REVERB_MODE_3_SETTINGS: BReverbSettings = BReverbSettings {
        number_of_allpasses: 0,
        allpass_sizes: &[],
        number_of_combs: 1,
        comb_sizes: &MODE_3_DELAY,
        out_l_positions: &MODE_3_OUTL,
        out_r_positions: &MODE_3_OUTR,
        filter_factors: &MODE_3_COMB_FACTOR,
        feedback_factors: &MODE_3_COMB_FEEDBACK,
        dry_amps: &MODE_3_DRY_AMP,
        wet_levels: &MODE_3_WET_AMP,
        lpf_amp: 0,
    };

    static REVERB_SETTINGS: [&BReverbSettings; 4] = [
        &REVERB_MODE_0_SETTINGS,
        &REVERB_MODE_1_SETTINGS,
        &REVERB_MODE_2_SETTINGS,
        &REVERB_MODE_3_SETTINGS,
    ];

    REVERB_SETTINGS[mode as usize]
}

// Default reverb settings for "old" reverb model implemented in MT-32.
// Found by tracing reverb RAM data lines (thanks go to Lord_Nightmare & balrog).
fn get_mt32_settings(mode: ReverbMode) -> &'static BReverbSettings {
    static MODE_0_ALLPASSES: [u32; 3] = [994, 729, 78];
    // Same as above in the new model implementation
    static MODE_0_COMBS: [u32; 4] = [575 + PROCESS_DELAY, 2040, 2752, 3629];
    static MODE_0_OUTL: [u32; 3] = [2040, 687, 1814];
    static MODE_0_OUTR: [u32; 3] = [1019, 2072, 1];
    static MODE_0_COMB_FACTOR: [u8; 4] = [0xB0, 0x60, 0x60, 0x60];
    #[rustfmt::skip]
    static MODE_0_COMB_FEEDBACK: [u8; 32] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x28, 0x48, 0x60, 0x70, 0x78, 0x80, 0x90, 0x98,
        0x28, 0x48, 0x60, 0x78, 0x80, 0x88, 0x90, 0x98,
        0x28, 0x48, 0x60, 0x78, 0x80, 0x88, 0x90, 0x98,
    ];
    static MODE_0_DRY_AMP: [u8; 8] = [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80];
    static MODE_0_WET_AMP: [u8; 8] = [0x10, 0x20, 0x30, 0x40, 0x50, 0x70, 0xA0, 0xE0];
    static MODE_0_LPF_AMP: u8 = 0x80;

    static MODE_1_ALLPASSES: [u32; 3] = [1324, 809, 176];
    // Same as above in the new model implementation
    static MODE_1_COMBS: [u32; 4] = [961 + PROCESS_DELAY, 2619, 3545, 4519];
    static MODE_1_OUTL: [u32; 3] = [2618, 1760, 4518];
    static MODE_1_OUTR: [u32; 3] = [1300, 3532, 2274];
    static MODE_1_COMB_FACTOR: [u8; 4] = [0x90, 0x60, 0x60, 0x60];
    #[rustfmt::skip]
    static MODE_1_COMB_FEEDBACK: [u8; 32] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x28, 0x48, 0x60, 0x70, 0x78, 0x80, 0x90, 0x98,
        0x28, 0x48, 0x60, 0x78, 0x80, 0x88, 0x90, 0x98,
        0x28, 0x48, 0x60, 0x78, 0x80, 0x88, 0x90, 0x98,
    ];
    static MODE_1_DRY_AMP: [u8; 8] = [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80];
    static MODE_1_WET_AMP: [u8; 8] = [0x10, 0x20, 0x30, 0x40, 0x50, 0x70, 0xA0, 0xE0];
    static MODE_1_LPF_AMP: u8 = 0x80;

    static MODE_2_ALLPASSES: [u32; 3] = [969, 644, 157];
    // Same as above in the new model implementation
    static MODE_2_COMBS: [u32; 4] = [116 + PROCESS_DELAY, 2259, 2839, 3539];
    static MODE_2_OUTL: [u32; 3] = [2259, 718, 1769];
    static MODE_2_OUTR: [u32; 3] = [1136, 2128, 1];
    static MODE_2_COMB_FACTOR: [u8; 4] = [0, 0x60, 0x60, 0x60];
    #[rustfmt::skip]
    static MODE_2_COMB_FEEDBACK: [u8; 32] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x28, 0x48, 0x60, 0x70, 0x78, 0x80, 0x90, 0x98,
        0x28, 0x48, 0x60, 0x78, 0x80, 0x88, 0x90, 0x98,
        0x28, 0x48, 0x60, 0x78, 0x80, 0x88, 0x90, 0x98,
    ];
    static MODE_2_DRY_AMP: [u8; 8] = [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80];
    static MODE_2_WET_AMP: [u8; 8] = [0x10, 0x20, 0x30, 0x40, 0x50, 0x70, 0xA0, 0xE0];
    static MODE_2_LPF_AMP: u8 = 0x80;

    static MODE_3_DELAY: [u32; 1] =
        [16000 + MODE_3_FEEDBACK_DELAY + PROCESS_DELAY + MODE_3_ADDITIONAL_DELAY];
    static MODE_3_OUTL: [u32; 8] = [400, 624, 960, 1488, 2256, 3472, 5280, 8000];
    static MODE_3_OUTR: [u32; 8] = [800, 1248, 1920, 2976, 4512, 6944, 10560, 16000];
    static MODE_3_COMB_FACTOR: [u8; 1] = [0x68];
    static MODE_3_COMB_FEEDBACK: [u8; 2] = [0x68, 0x60];
    #[rustfmt::skip]
    static MODE_3_DRY_AMP: [u8; 16] = [
        0x10, 0x10, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
        0x10, 0x20, 0x20, 0x10, 0x20, 0x10, 0x20, 0x10,
    ];
    static MODE_3_WET_AMP: [u8; 8] = [0x08, 0x18, 0x28, 0x40, 0x60, 0x80, 0xA8, 0xF8];

    static REVERB_MODE_0_SETTINGS: BReverbSettings = BReverbSettings {
        number_of_allpasses: 3,
        allpass_sizes: &MODE_0_ALLPASSES,
        number_of_combs: 4,
        comb_sizes: &MODE_0_COMBS,
        out_l_positions: &MODE_0_OUTL,
        out_r_positions: &MODE_0_OUTR,
        filter_factors: &MODE_0_COMB_FACTOR,
        feedback_factors: &MODE_0_COMB_FEEDBACK,
        dry_amps: &MODE_0_DRY_AMP,
        wet_levels: &MODE_0_WET_AMP,
        lpf_amp: MODE_0_LPF_AMP,
    };
    static REVERB_MODE_1_SETTINGS: BReverbSettings = BReverbSettings {
        number_of_allpasses: 3,
        allpass_sizes: &MODE_1_ALLPASSES,
        number_of_combs: 4,
        comb_sizes: &MODE_1_COMBS,
        out_l_positions: &MODE_1_OUTL,
        out_r_positions: &MODE_1_OUTR,
        filter_factors: &MODE_1_COMB_FACTOR,
        feedback_factors: &MODE_1_COMB_FEEDBACK,
        dry_amps: &MODE_1_DRY_AMP,
        wet_levels: &MODE_1_WET_AMP,
        lpf_amp: MODE_1_LPF_AMP,
    };
    static REVERB_MODE_2_SETTINGS: BReverbSettings = BReverbSettings {
        number_of_allpasses: 3,
        allpass_sizes: &MODE_2_ALLPASSES,
        number_of_combs: 4,
        comb_sizes: &MODE_2_COMBS,
        out_l_positions: &MODE_2_OUTL,
        out_r_positions: &MODE_2_OUTR,
        filter_factors: &MODE_2_COMB_FACTOR,
        feedback_factors: &MODE_2_COMB_FEEDBACK,
        dry_amps: &MODE_2_DRY_AMP,
        wet_levels: &MODE_2_WET_AMP,
        lpf_amp: MODE_2_LPF_AMP,
    };
    static REVERB_MODE_3_SETTINGS: BReverbSettings = BReverbSettings {
        number_of_allpasses: 0,
        allpass_sizes: &[],
        number_of_combs: 1,
        comb_sizes: &MODE_3_DELAY,
        out_l_positions: &MODE_3_OUTL,
        out_r_positions: &MODE_3_OUTR,
        filter_factors: &MODE_3_COMB_FACTOR,
        feedback_factors: &MODE_3_COMB_FEEDBACK,
        dry_amps: &MODE_3_DRY_AMP,
        wet_levels: &MODE_3_WET_AMP,
        lpf_amp: 0,
    };

    static REVERB_SETTINGS: [&BReverbSettings; 4] = [
        &REVERB_MODE_0_SETTINGS,
        &REVERB_MODE_1_SETTINGS,
        &REVERB_MODE_2_SETTINGS,
        &REVERB_MODE_3_SETTINGS,
    ];

    REVERB_SETTINGS[mode as usize]
}

fn get_settings(mode: ReverbMode, mt32_compatible: bool) -> &'static BReverbSettings {
    if mt32_compatible {
        get_mt32_settings(mode)
    } else {
        get_cm32l_lapc_settings(mode)
    }
}

fn weird_mul(sample: f32, add_mask: u8, _carry_mask: u8) -> f32 {
    sample * add_mask as f32 / 256.0
}

fn halve_sample(sample: f32) -> f32 {
    0.5 * sample
}

fn quarter_sample(sample: f32) -> f32 {
    0.25 * sample
}

fn add_dc_bias(sample: f32) -> f32 {
    sample + BIAS
}

// NOTE:
//   Thanks to Mok for discovering, the adder in BOSS reverb chip is found to perform addition with saturation to avoid integer overflow.
//   Analysing of the algorithm suggests that the overflow is most probable when the combs output is added below.
//   So, despite this isn't actually accurate, we only add the check here for performance reasons.
fn mix_combs(out1: f32, out2: f32, out3: f32) -> f32 {
    1.5 * (out1 + out2) + out3
}

const SAMPLE_VALUE_THRESHOLD: f32 = 0.001;

impl ReverbRingBuffer {
    fn next(&mut self) -> f32 {
        self.index += 1;
        if self.index >= self.size {
            self.index = 0;
        }
        self.buffer[self.index as usize]
    }

    fn is_empty(&self) -> bool {
        !self.buffer[..self.size as usize]
            .iter()
            .any(|&sample| sample < -SAMPLE_VALUE_THRESHOLD || sample > SAMPLE_VALUE_THRESHOLD)
    }

    fn mute(&mut self) {
        for sample in self.buffer.iter_mut() {
            *sample = 0.0;
        }
    }
}

impl ReverbAllpassState {
    // This model corresponds to the allpass filter implementation of the real CM-32L device
    // found from sample analysis
    fn process(&mut self, input: f32) -> f32 {
        let buffer_out = self.ring.next();

        // store input - feedback / 2
        self.ring.buffer[self.ring.index as usize] = input - halve_sample(buffer_out);

        // return buffer output + feedforward / 2
        buffer_out + halve_sample(self.ring.buffer[self.ring.index as usize])
    }
}

impl ReverbCombState {
    // This model corresponds to the comb filter implementation of the real CM-32L device
    fn process(&mut self, input: f32) {
        // the previously stored value
        let last = self.ring.buffer[self.ring.index as usize];

        // prepare input + feedback
        let filter_in = input + weird_mul(self.ring.next(), self.feedback_factor, 0xF0);

        // store input + feedback processed by a low-pass filter
        self.ring.buffer[self.ring.index as usize] =
            weird_mul(last, self.filter_factor, 0xC0) - filter_in;
    }

    fn get_output_at(&self, out_index: u32) -> f32 {
        self.ring.buffer[((self.ring.size + self.ring.index - out_index) % self.ring.size) as usize]
    }
}

impl ReverbDelayWithLpfState {
    fn process(&mut self, input: f32) {
        // the previously stored value
        let last = self.comb.ring.buffer[self.comb.ring.index as usize];

        // move to the next index
        self.comb.ring.next();

        // low-pass filter process
        let lpf_out = weird_mul(last, self.comb.filter_factor, 0xFF) + input;

        // store lpfOut multiplied by LPF amp factor
        self.comb.ring.buffer[self.comb.ring.index as usize] = weird_mul(lpf_out, self.amp, 0xFF);
    }
}

impl ReverbTapDelayCombState {
    fn process(&mut self, input: f32) {
        // the previously stored value
        let last = self.comb.ring.buffer[self.comb.ring.index as usize];

        // move to the next index
        self.comb.ring.next();

        // prepare input + feedback
        // Actually, the size of the filter varies with the TIME parameter, the feedback sample is taken from the position just below the right output
        let filter_in = input
            + weird_mul(
                self.comb.get_output_at(self.out_r + MODE_3_FEEDBACK_DELAY),
                self.comb.feedback_factor,
                0xF0,
            );

        // store input + feedback processed by a low-pass filter
        self.comb.ring.buffer[self.comb.ring.index as usize] =
            weird_mul(last, self.comb.filter_factor, 0xF0) - filter_in;
    }

    fn get_left_output(&self) -> f32 {
        self.comb
            .get_output_at(self.out_l + PROCESS_DELAY + MODE_3_ADDITIONAL_DELAY)
    }

    fn get_right_output(&self) -> f32 {
        self.comb
            .get_output_at(self.out_r + PROCESS_DELAY + MODE_3_ADDITIONAL_DELAY)
    }
}

impl BReverbModelState {
    pub(crate) fn new(mode: ReverbMode, mt32_compatible: bool) -> BReverbModelState {
        if mode == ReverbMode::TapDelay {
            BReverbModelState::TapDelay {
                tap_delay_comb: ReverbTapDelayCombState::default(),
                dry_amp: 0,
                wet_level: 0,
                mt32_compatible,
                opened: false,
            }
        } else {
            BReverbModelState::Standard {
                allpasses: Vec::new(),
                entrance_delay: ReverbDelayWithLpfState::default(),
                combs: Vec::new(),
                dry_amp: 0,
                wet_level: 0,
                mt32_compatible,
                mode,
                opened: false,
            }
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        match self {
            BReverbModelState::Standard { opened, .. } => *opened,
            BReverbModelState::TapDelay { opened, .. } => *opened,
            BReverbModelState::Closed => false,
        }
    }

    /// After construction or a close(), open() must be called at least once before any other call (with the exception of close()).
    pub(crate) fn open(&mut self) {
        match self {
            BReverbModelState::Standard {
                allpasses,
                entrance_delay,
                combs,
                mt32_compatible,
                mode,
                opened,
                ..
            } => {
                if *opened {
                    return;
                }
                let settings = get_settings(*mode, *mt32_compatible);

                *allpasses = Vec::with_capacity(settings.number_of_allpasses as usize);
                for &size in &settings.allpass_sizes[..settings.number_of_allpasses as usize] {
                    allpasses.push(ReverbAllpassState {
                        ring: ReverbRingBuffer {
                            buffer: vec![0.0; size as usize],
                            size,
                            index: 0,
                        },
                    });
                }

                // combs[0] is the entrance delay with LPF
                *entrance_delay = ReverbDelayWithLpfState {
                    comb: ReverbCombState {
                        ring: ReverbRingBuffer {
                            buffer: vec![0.0; settings.comb_sizes[0] as usize],
                            size: settings.comb_sizes[0],
                            index: 0,
                        },
                        filter_factor: settings.filter_factors[0],
                        feedback_factor: 0,
                    },
                    amp: settings.lpf_amp,
                };

                *combs = Vec::with_capacity((settings.number_of_combs - 1) as usize);
                for i in 1..settings.number_of_combs as usize {
                    combs.push(ReverbCombState {
                        ring: ReverbRingBuffer {
                            buffer: vec![0.0; settings.comb_sizes[i] as usize],
                            size: settings.comb_sizes[i],
                            index: 0,
                        },
                        filter_factor: settings.filter_factors[i],
                        feedback_factor: 0,
                    });
                }

                *opened = true;
                // Mute all ring buffers through the individual fields we already have bound.
                for allpass in allpasses.iter_mut() {
                    allpass.ring.mute();
                }
                entrance_delay.comb.ring.mute();
                for comb in combs.iter_mut() {
                    comb.ring.mute();
                }
            }
            BReverbModelState::TapDelay {
                tap_delay_comb,
                mt32_compatible,
                opened,
                ..
            } => {
                if *opened {
                    return;
                }
                let settings = get_settings(ReverbMode::TapDelay, *mt32_compatible);

                *tap_delay_comb = ReverbTapDelayCombState {
                    comb: ReverbCombState {
                        ring: ReverbRingBuffer {
                            buffer: vec![0.0; settings.comb_sizes[0] as usize],
                            size: settings.comb_sizes[0],
                            index: 0,
                        },
                        filter_factor: settings.filter_factors[0],
                        feedback_factor: 0,
                    },
                    out_l: 0,
                    out_r: 0,
                };

                *opened = true;
                tap_delay_comb.comb.ring.mute();
            }
            BReverbModelState::Closed => {}
        }
    }

    pub(crate) fn mute(&mut self) {
        match self {
            BReverbModelState::Standard {
                allpasses,
                entrance_delay,
                combs,
                ..
            } => {
                for allpass in allpasses.iter_mut() {
                    allpass.ring.mute();
                }
                entrance_delay.comb.ring.mute();
                for comb in combs.iter_mut() {
                    comb.ring.mute();
                }
            }
            BReverbModelState::TapDelay { tap_delay_comb, .. } => {
                tap_delay_comb.comb.ring.mute();
            }
            BReverbModelState::Closed => {}
        }
    }

    pub(crate) fn set_parameters(&mut self, time: u8, level: u8) {
        if !self.is_open() {
            return;
        }
        let level = level & 7;
        let time = time & 7;

        match self {
            BReverbModelState::TapDelay {
                tap_delay_comb,
                dry_amp,
                wet_level,
                mt32_compatible,
                ..
            } => {
                let settings = get_settings(ReverbMode::TapDelay, *mt32_compatible);
                tap_delay_comb.out_l = settings.out_l_positions[time as usize];
                tap_delay_comb.out_r = settings.out_r_positions[(time & 7) as usize];
                tap_delay_comb.comb.feedback_factor =
                    settings.feedback_factors[if (level < 3) || (time < 6) { 0 } else { 1 }];

                if time == 0 && level == 0 {
                    *dry_amp = 0;
                    *wet_level = 0;
                } else {
                    // Looks like MT-32 implementation has some minor quirks in this mode:
                    // for odd level values, the output level changes sometimes depending on the time value which doesn't seem right.
                    if (time == 0) || (time == 1 && level == 1) {
                        *dry_amp = settings.dry_amps[(level + 8) as usize];
                    } else {
                        *dry_amp = settings.dry_amps[level as usize];
                    }
                    *wet_level = settings.wet_levels[level as usize];
                }
            }
            BReverbModelState::Standard {
                combs,
                dry_amp,
                wet_level,
                mt32_compatible,
                mode,
                ..
            } => {
                let settings = get_settings(*mode, *mt32_compatible);
                // combs in the state vec are indexed 0..2, but correspond to settings combs 1..3
                for (i, comb) in combs.iter_mut().enumerate() {
                    comb.feedback_factor =
                        settings.feedback_factors[((i + 1) << 3) + time as usize];
                }

                if time == 0 && level == 0 {
                    *dry_amp = 0;
                    *wet_level = 0;
                } else {
                    *dry_amp = settings.dry_amps[level as usize];
                    *wet_level = settings.wet_levels[level as usize];
                }
            }
            BReverbModelState::Closed => {}
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        if !self.is_open() {
            return false;
        }
        match self {
            BReverbModelState::Standard {
                allpasses,
                entrance_delay,
                combs,
                ..
            } => {
                for allpass in allpasses.iter() {
                    if !allpass.ring.is_empty() {
                        return true;
                    }
                }
                if !entrance_delay.comb.ring.is_empty() {
                    return true;
                }
                for comb in combs.iter() {
                    if !comb.ring.is_empty() {
                        return true;
                    }
                }
                false
            }
            BReverbModelState::TapDelay { tap_delay_comb, .. } => {
                !tap_delay_comb.comb.ring.is_empty()
            }
            BReverbModelState::Closed => false,
        }
    }

    pub(crate) fn process(
        &mut self,
        in_left: &[f32],
        in_right: &[f32],
        out_left: &mut [f32],
        out_right: &mut [f32],
        num_samples: u32,
    ) -> bool {
        if !self.is_open() {
            for i in 0..num_samples as usize {
                out_left[i] = 0.0;
                out_right[i] = 0.0;
            }
            return true;
        }

        match self {
            BReverbModelState::TapDelay {
                tap_delay_comb,
                dry_amp,
                wet_level,
                ..
            } => {
                let dry_amp_val = *dry_amp;
                let wet_level_val = *wet_level;

                for i in 0..num_samples as usize {
                    let dry = halve_sample(in_left[i]) + halve_sample(in_right[i]);

                    // Looks like dryAmp doesn't change in MT-32 but it does in CM-32L / LAPC-I
                    let dry = weird_mul(add_dc_bias(dry), dry_amp_val, 0xFF);

                    tap_delay_comb.process(dry);
                    out_left[i] = weird_mul(tap_delay_comb.get_left_output(), wet_level_val, 0xFF);
                    out_right[i] =
                        weird_mul(tap_delay_comb.get_right_output(), wet_level_val, 0xFF);
                }
            }
            BReverbModelState::Standard {
                allpasses,
                entrance_delay,
                combs,
                dry_amp,
                wet_level,
                mt32_compatible,
                mode,
                ..
            } => {
                let settings = get_settings(*mode, *mt32_compatible);
                let dry_amp_val = *dry_amp;
                let wet_level_val = *wet_level;

                for i in 0..num_samples as usize {
                    let dry = quarter_sample(in_left[i]) + quarter_sample(in_right[i]);

                    // Looks like dryAmp doesn't change in MT-32 but it does in CM-32L / LAPC-I
                    let dry = weird_mul(add_dc_bias(dry), dry_amp_val, 0xFF);

                    // If the output position is equal to the comb size, get it now in order not to loose it
                    let mut link = entrance_delay
                        .comb
                        .get_output_at(settings.comb_sizes[0] - 1);

                    // Entrance LPF. Note, comb.process() differs a bit here.
                    entrance_delay.process(dry);

                    link = allpasses[0].process(link);
                    link = allpasses[1].process(link);
                    link = allpasses[2].process(link);

                    // If the output position is equal to the comb size, get it now in order not to loose it
                    let out_l1 = combs[0].get_output_at(settings.out_l_positions[0] - 1);

                    combs[0].process(link);
                    combs[1].process(link);
                    combs[2].process(link);

                    {
                        let out_l2 = combs[1].get_output_at(settings.out_l_positions[1]);
                        let out_l3 = combs[2].get_output_at(settings.out_l_positions[2]);
                        let out_sample = mix_combs(out_l1, out_l2, out_l3);
                        out_left[i] = weird_mul(out_sample, wet_level_val, 0xFF);
                    }
                    {
                        let out_r1 = combs[0].get_output_at(settings.out_r_positions[0]);
                        let out_r2 = combs[1].get_output_at(settings.out_r_positions[1]);
                        let out_r3 = combs[2].get_output_at(settings.out_r_positions[2]);
                        let out_sample = mix_combs(out_r1, out_r2, out_r3);
                        out_right[i] = weird_mul(out_sample, wet_level_val, 0xFF);
                    }
                }
            }
            BReverbModelState::Closed => {
                for i in 0..num_samples as usize {
                    out_left[i] = 0.0;
                    out_right[i] = 0.0;
                }
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Checksums (sum_l, sum_r, abs_l, abs_r) from C++ BReverbModel reference.
    // Impulse response: 4096 samples, time=3, level=5.

    fn run_breverb_impulse(mode: ReverbMode, mt32_compat: bool) -> (f64, f64, f64, f64) {
        let n = 4096;
        let mut in_left = vec![0.0f32; n];
        let mut in_right = vec![0.0f32; n];
        in_left[0] = 1.0;
        in_right[0] = 1.0;

        let mut state = BReverbModelState::new(mode, mt32_compat);
        state.open();
        state.set_parameters(3, 5);
        let mut out_left = vec![0.0f32; n];
        let mut out_right = vec![0.0f32; n];
        state.process(&in_left, &in_right, &mut out_left, &mut out_right, n as u32);

        let sum_l: f64 = out_left.iter().map(|&x| x as f64).sum();
        let sum_r: f64 = out_right.iter().map(|&x| x as f64).sum();
        let abs_l: f64 = out_left.iter().map(|&x| (x as f64).abs()).sum();
        let abs_r: f64 = out_right.iter().map(|&x| (x as f64).abs()).sum();
        (sum_l, sum_r, abs_l, abs_r)
    }

    fn assert_breverb(label: &str, got: (f64, f64, f64, f64), exp: (f64, f64, f64, f64)) {
        let tol_l = exp.2 * 1e-3 + 1e-6;
        let tol_r = exp.3 * 1e-3 + 1e-6;
        assert!((got.0 - exp.0).abs() < tol_l, "{label}: sum_l mismatch");
        assert!((got.1 - exp.1).abs() < tol_r, "{label}: sum_r mismatch");
        assert!((got.2 - exp.2).abs() < tol_l, "{label}: abs_l mismatch");
        assert!((got.3 - exp.3).abs() < tol_r, "{label}: abs_r mismatch");
    }

    #[test]
    fn breverb_impulse_room() {
        let r = run_breverb_impulse(ReverbMode::Room, false);
        assert_breverb(
            "room",
            r,
            (-0.6549793641, -1.0117096206, 2.5468091166, 3.1644621854),
        );
        let r = run_breverb_impulse(ReverbMode::Room, true);
        assert_breverb(
            "room_mt32",
            r,
            (-0.3793763951, -0.5236728140, 1.7434764339, 2.0377044937),
        );
    }

    #[test]
    fn breverb_impulse_hall() {
        let r = run_breverb_impulse(ReverbMode::Hall, false);
        assert_breverb(
            "hall",
            r,
            (-0.3014732961, -0.2887027442, 0.5056161747, 0.6971030055),
        );
        let r = run_breverb_impulse(ReverbMode::Hall, true);
        assert_breverb(
            "hall_mt32",
            r,
            (-0.1927734376, -0.1846074637, 0.3233101222, 0.4423651226),
        );
    }

    #[test]
    fn breverb_impulse_plate() {
        let r = run_breverb_impulse(ReverbMode::Plate, false);
        assert_breverb(
            "plate",
            r,
            (-0.6192358738, -0.5617078223, 2.4246543810, 2.7599044222),
        );
        let r = run_breverb_impulse(ReverbMode::Plate, true);
        assert_breverb(
            "plate_mt32",
            r,
            (-0.3636870103, -0.3276498314, 1.4303798514, 1.6041774345),
        );
    }

    #[test]
    fn breverb_impulse_tap_delay() {
        let r = run_breverb_impulse(ReverbMode::TapDelay, false);
        assert_breverb(
            "tap",
            r,
            (-0.2631578947, -0.2631578947, 0.2631578947, 0.2631578947),
        );
        let r = run_breverb_impulse(ReverbMode::TapDelay, true);
        assert_breverb(
            "tap_mt32",
            r,
            (-0.1052631579, -0.1052631579, 0.1052631579, 0.1052631579),
        );
    }

    #[test]
    fn breverb_closed_produces_zeros() {
        let mut state = BReverbModelState::new(ReverbMode::Room, false);
        // Don't open it.
        let in_left = vec![1.0f32; 64];
        let in_right = vec![1.0f32; 64];
        let mut out_left = vec![1.0f32; 64];
        let mut out_right = vec![1.0f32; 64];
        state.process(&in_left, &in_right, &mut out_left, &mut out_right, 64);
        assert!(out_left.iter().all(|&x| x == 0.0));
        assert!(out_right.iter().all(|&x| x == 0.0));
    }
}
