// Copyright (C) 2003, 2004, 2005, 2006, 2008, 2009 Dean Beeler, Jerome Fisher
// Copyright (C) 2011-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
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

/// MT32EMU_MEMADDR() converts from sysex-padded addresses.
/// Roland provides documentation using the sysex-padded addresses, so we tend to use that in code and output.
pub(crate) const fn memaddr(x: u32) -> u32 {
    ((x & 0x7F0000) >> 2) | ((x & 0x7F00) >> 1) | (x & 0x7F)
}

// The following structures represent the MT-32's memory.
// Since sysex allows this memory to be written to in blocks of bytes,
// the packed structs use manual from_bytes/to_bytes for raw byte access.

#[derive(Clone, Default)]
/// Wave-generator parameters retained by one partial.
pub(crate) struct WGParam {
    pub(crate) pitch_coarse: u8,                 // 0-96 (C1,C#1-C9)
    pub(crate) pitch_fine: u8,                   // 0-100 (-50 to +50 (cents - confirmed by Mok))
    pub(crate) pitch_keyfollow: u8, // 0-16 (-1, -1/2, -1/4, 0, 1/8, 1/4, 3/8, 1/2, 5/8, 3/4, 7/8, 1, 5/4, 3/2, 2, s1, s2)
    pub(crate) pitch_bender_enabled: u8, // 0-1 (OFF, ON)
    pub(crate) waveform: u8, // MT-32: 0-1 (SQU/SAW); LAPC-I: WG WAVEFORM/PCM BANK 0 - 3 (SQU/1, SAW/1, SQU/2, SAW/2)
    pub(crate) pcm_wave: u8, // 0-127 (1-128)
    pub(crate) pulse_width: u8, // 0-100
    pub(crate) pulse_width_velo_sensitivity: u8, // 0-14 (-7 - +7)
}

impl WGParam {
    pub(crate) const SIZE: usize = 8;

    pub(crate) fn from_bytes(data: &[u8]) -> Self {
        Self {
            pitch_coarse: data[0],
            pitch_fine: data[1],
            pitch_keyfollow: data[2],
            pitch_bender_enabled: data[3],
            waveform: data[4],
            pcm_wave: data[5],
            pulse_width: data[6],
            pulse_width_velo_sensitivity: data[7],
        }
    }

    pub(crate) fn to_bytes(&self, out: &mut [u8]) {
        out[0] = self.pitch_coarse;
        out[1] = self.pitch_fine;
        out[2] = self.pitch_keyfollow;
        out[3] = self.pitch_bender_enabled;
        out[4] = self.waveform;
        out[5] = self.pcm_wave;
        out[6] = self.pulse_width;
        out[7] = self.pulse_width_velo_sensitivity;
    }
}

#[derive(Clone, Default)]
/// Pitch-envelope parameters retained by one partial.
pub(crate) struct PitchEnvParam {
    pub(crate) depth: u8,            // 0-10
    pub(crate) velo_sensitivity: u8, // 0-100
    pub(crate) time_keyfollow: u8,   // 0-4
    pub(crate) time: [u8; 4],        // 0-100
    pub(crate) level: [u8; 5],       // 0-100 (-50 - +50) // [3]: SUSTAIN LEVEL, [4]: END LEVEL
}

impl PitchEnvParam {
    pub(crate) const SIZE: usize = 12;

    pub(crate) fn from_bytes(data: &[u8]) -> Self {
        let mut time = [0u8; 4];
        time.copy_from_slice(&data[3..7]);
        let mut level = [0u8; 5];
        level.copy_from_slice(&data[7..12]);
        Self {
            depth: data[0],
            velo_sensitivity: data[1],
            time_keyfollow: data[2],
            time,
            level,
        }
    }

    pub(crate) fn to_bytes(&self, out: &mut [u8]) {
        out[0] = self.depth;
        out[1] = self.velo_sensitivity;
        out[2] = self.time_keyfollow;
        out[3..7].copy_from_slice(&self.time);
        out[7..12].copy_from_slice(&self.level);
    }
}

#[derive(Clone, Default)]
/// Pitch LFO parameters retained by one partial.
pub(crate) struct PitchLFOParam {
    pub(crate) rate: u8,            // 0-100
    pub(crate) depth: u8,           // 0-100
    pub(crate) mod_sensitivity: u8, // 0-100
}

impl PitchLFOParam {
    pub(crate) const SIZE: usize = 3;

    pub(crate) fn from_bytes(data: &[u8]) -> Self {
        Self {
            rate: data[0],
            depth: data[1],
            mod_sensitivity: data[2],
        }
    }

    pub(crate) fn to_bytes(&self, out: &mut [u8]) {
        out[0] = self.rate;
        out[1] = self.depth;
        out[2] = self.mod_sensitivity;
    }
}

#[derive(Clone, Default)]
/// Time-variant filter parameters retained by one partial.
pub(crate) struct TVFParam {
    pub(crate) cutoff: u8,               // 0-100
    pub(crate) resonance: u8,            // 0-30
    pub(crate) keyfollow: u8, // -1, -1/2, -1/4, 0, 1/8, 1/4, 3/8, 1/2, 5/8, 3/4, 7/8, 1, 5/4, 3/2, 2
    pub(crate) bias_point: u8, // 0-127 (<1A-<7C >1A-7C)
    pub(crate) bias_level: u8, // 0-14 (-7 - +7)
    pub(crate) env_depth: u8, // 0-100
    pub(crate) env_velo_sensitivity: u8, // 0-100
    pub(crate) env_depth_keyfollow: u8, // DEPTH KEY FOLLOW 0-4
    pub(crate) env_time_keyfollow: u8, // TIME KEY FOLLOW 0-4
    pub(crate) env_time: [u8; 5], // 0-100
    pub(crate) env_level: [u8; 4], // 0-100 // [3]: SUSTAIN LEVEL
}

impl TVFParam {
    pub(crate) const SIZE: usize = 18;

    pub(crate) fn from_bytes(data: &[u8]) -> Self {
        let mut env_time = [0u8; 5];
        env_time.copy_from_slice(&data[9..14]);
        let mut env_level = [0u8; 4];
        env_level.copy_from_slice(&data[14..18]);
        Self {
            cutoff: data[0],
            resonance: data[1],
            keyfollow: data[2],
            bias_point: data[3],
            bias_level: data[4],
            env_depth: data[5],
            env_velo_sensitivity: data[6],
            env_depth_keyfollow: data[7],
            env_time_keyfollow: data[8],
            env_time,
            env_level,
        }
    }

    pub(crate) fn to_bytes(&self, out: &mut [u8]) {
        out[0] = self.cutoff;
        out[1] = self.resonance;
        out[2] = self.keyfollow;
        out[3] = self.bias_point;
        out[4] = self.bias_level;
        out[5] = self.env_depth;
        out[6] = self.env_velo_sensitivity;
        out[7] = self.env_depth_keyfollow;
        out[8] = self.env_time_keyfollow;
        out[9..14].copy_from_slice(&self.env_time);
        out[14..18].copy_from_slice(&self.env_level);
    }
}

#[derive(Clone, Default)]
/// Time-variant amplifier parameters retained by one partial.
pub(crate) struct TVAParam {
    pub(crate) level: u8,                     // 0-100
    pub(crate) velo_sensitivity: u8,          // 0-100
    pub(crate) bias_point1: u8,               // 0-127 (<1A-<7C >1A-7C)
    pub(crate) bias_level1: u8,               // 0-12 (-12 - 0)
    pub(crate) bias_point2: u8,               // 0-127 (<1A-<7C >1A-7C)
    pub(crate) bias_level2: u8,               // 0-12 (-12 - 0)
    pub(crate) env_time_keyfollow: u8,        // TIME KEY FOLLOW 0-4
    pub(crate) env_time_velo_sensitivity: u8, // VELOS KEY FOLLOW 0-4
    pub(crate) env_time: [u8; 5],             // 0-100
    pub(crate) env_level: [u8; 4],            // 0-100 // [3]: SUSTAIN LEVEL
}

impl TVAParam {
    pub(crate) const SIZE: usize = 17;

    pub(crate) fn from_bytes(data: &[u8]) -> Self {
        let mut env_time = [0u8; 5];
        env_time.copy_from_slice(&data[8..13]);
        let mut env_level = [0u8; 4];
        env_level.copy_from_slice(&data[13..17]);
        Self {
            level: data[0],
            velo_sensitivity: data[1],
            bias_point1: data[2],
            bias_level1: data[3],
            bias_point2: data[4],
            bias_level2: data[5],
            env_time_keyfollow: data[6],
            env_time_velo_sensitivity: data[7],
            env_time,
            env_level,
        }
    }

    pub(crate) fn to_bytes(&self, out: &mut [u8]) {
        out[0] = self.level;
        out[1] = self.velo_sensitivity;
        out[2] = self.bias_point1;
        out[3] = self.bias_level1;
        out[4] = self.bias_point2;
        out[5] = self.bias_level2;
        out[6] = self.env_time_keyfollow;
        out[7] = self.env_time_velo_sensitivity;
        out[8..13].copy_from_slice(&self.env_time);
        out[13..17].copy_from_slice(&self.env_level);
    }
}

#[derive(Clone, Default)]
/// Complete synthesis parameters for one partial.
pub(crate) struct PartialParam {
    pub(crate) wg: WGParam,
    pub(crate) pitch_env: PitchEnvParam,
    pub(crate) pitch_lfo: PitchLFOParam,
    pub(crate) tvf: TVFParam,
    pub(crate) tva: TVAParam,
}

impl PartialParam {
    pub(crate) const SIZE: usize =
        WGParam::SIZE + PitchEnvParam::SIZE + PitchLFOParam::SIZE + TVFParam::SIZE + TVAParam::SIZE; // 58

    pub(crate) fn from_bytes(data: &[u8]) -> Self {
        let mut offset = 0;
        let wg = WGParam::from_bytes(&data[offset..]);
        offset += WGParam::SIZE;
        let pitch_env = PitchEnvParam::from_bytes(&data[offset..]);
        offset += PitchEnvParam::SIZE;
        let pitch_lfo = PitchLFOParam::from_bytes(&data[offset..]);
        offset += PitchLFOParam::SIZE;
        let tvf = TVFParam::from_bytes(&data[offset..]);
        offset += TVFParam::SIZE;
        let tva = TVAParam::from_bytes(&data[offset..]);
        let _ = offset + TVAParam::SIZE;
        Self {
            wg,
            pitch_env,
            pitch_lfo,
            tvf,
            tva,
        }
    }

    pub(crate) fn to_bytes(&self, out: &mut [u8]) {
        let mut offset = 0;
        self.wg.to_bytes(&mut out[offset..]);
        offset += WGParam::SIZE;
        self.pitch_env.to_bytes(&mut out[offset..]);
        offset += PitchEnvParam::SIZE;
        self.pitch_lfo.to_bytes(&mut out[offset..]);
        offset += PitchLFOParam::SIZE;
        self.tvf.to_bytes(&mut out[offset..]);
        offset += TVFParam::SIZE;
        self.tva.to_bytes(&mut out[offset..]);
    }
}

#[derive(Clone, Default)]
pub(crate) struct CommonParam {
    pub(crate) name: [u8; 10],          // char name[10]
    pub(crate) partial_structure12: u8, // 1 & 2  0-12 (1-13)
    pub(crate) partial_structure34: u8, // 3 & 4  0-12 (1-13)
    pub(crate) partial_mute: u8,        // 0-15 (0000-1111)
    pub(crate) no_sustain: u8,          // ENV MODE 0-1 (Normal, No sustain)
}

impl CommonParam {
    pub(crate) const SIZE: usize = 14;

    pub(crate) fn from_bytes(data: &[u8]) -> Self {
        let mut name = [0u8; 10];
        name.copy_from_slice(&data[0..10]);
        Self {
            name,
            partial_structure12: data[10],
            partial_structure34: data[11],
            partial_mute: data[12],
            no_sustain: data[13],
        }
    }

    pub(crate) fn to_bytes(&self, out: &mut [u8]) {
        out[0..10].copy_from_slice(&self.name);
        out[10] = self.partial_structure12;
        out[11] = self.partial_structure34;
        out[12] = self.partial_mute;
        out[13] = self.no_sustain;
    }
}

#[derive(Clone, Default)]
pub(crate) struct TimbreParam {
    pub(crate) common: CommonParam,
    pub(crate) partial: [PartialParam; 4],
}

impl TimbreParam {
    pub(crate) const SIZE: usize = CommonParam::SIZE + PartialParam::SIZE * 4; // 246

    pub(crate) fn from_bytes(data: &[u8]) -> Self {
        let common = CommonParam::from_bytes(&data[0..]);
        let mut offset = CommonParam::SIZE;
        let partial = core::array::from_fn(|_| {
            let p = PartialParam::from_bytes(&data[offset..]);
            offset += PartialParam::SIZE;
            p
        });
        Self { common, partial }
    }

    pub(crate) fn to_bytes(&self, out: &mut [u8]) {
        self.common.to_bytes(&mut out[0..]);
        let mut offset = CommonParam::SIZE;
        for p in &self.partial {
            p.to_bytes(&mut out[offset..]);
            offset += PartialParam::SIZE;
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct PatchParam {
    pub(crate) timbre_group: u8, // TIMBRE GROUP  0-3 (group A, group B, Memory, Rhythm)
    pub(crate) timbre_num: u8,   // TIMBRE NUMBER 0-63
    pub(crate) key_shift: u8,    // KEY SHIFT 0-48 (-24 - +24 semitones)
    pub(crate) fine_tune: u8,    // FINE TUNE 0-100 (-50 - +50 cents)
    pub(crate) bender_range: u8, // BENDER RANGE 0-24
    pub(crate) assign_mode: u8,  // ASSIGN MODE 0-3 (POLY1, POLY2, POLY3, POLY4)
    pub(crate) reverb_switch: u8, // REVERB SWITCH 0-1 (OFF,ON)
    pub(crate) dummy: u8,        // (DUMMY)
}

impl PatchParam {
    pub(crate) const SIZE: usize = 8;

    pub(crate) fn from_bytes(data: &[u8]) -> Self {
        Self {
            timbre_group: data[0],
            timbre_num: data[1],
            key_shift: data[2],
            fine_tune: data[3],
            bender_range: data[4],
            assign_mode: data[5],
            reverb_switch: data[6],
            dummy: data[7],
        }
    }

    pub(crate) fn to_bytes(&self, out: &mut [u8]) {
        out[0] = self.timbre_group;
        out[1] = self.timbre_num;
        out[2] = self.key_shift;
        out[3] = self.fine_tune;
        out[4] = self.bender_range;
        out[5] = self.assign_mode;
        out[6] = self.reverb_switch;
        out[7] = self.dummy;
    }
}

pub(crate) const SYSTEM_MASTER_TUNE_OFF: usize = 0;
pub(crate) const SYSTEM_REVERB_MODE_OFF: usize = 1;
pub(crate) const SYSTEM_REVERB_TIME_OFF: usize = 2;
pub(crate) const SYSTEM_REVERB_LEVEL_OFF: usize = 3;
pub(crate) const SYSTEM_RESERVE_SETTINGS_START_OFF: usize = 4;
pub(crate) const SYSTEM_RESERVE_SETTINGS_END_OFF: usize = 12;
pub(crate) const SYSTEM_CHAN_ASSIGN_START_OFF: usize = 13;
pub(crate) const SYSTEM_CHAN_ASSIGN_END_OFF: usize = 21;
pub(crate) const SYSTEM_MASTER_VOL_OFF: usize = 22;

// NOTE: The MT-32 documentation only specifies PatchTemp areas for parts 1-8.
// The LAPC-I documentation specified an additional area for rhythm at the end,
// where all parameters but fine tune, assign mode and output level are ignored
#[derive(Clone, Default)]
pub(crate) struct PatchTemp {
    pub(crate) patch: PatchParam,
    pub(crate) output_level: u8, // OUTPUT LEVEL 0-100
    pub(crate) panpot: u8,       // PANPOT 0-14 (R-L)
    pub(crate) dummyv: [u8; 6],
}

impl PatchTemp {
    pub(crate) const SIZE: usize = PatchParam::SIZE + 1 + 1 + 6; // 16

    pub(crate) fn from_bytes(data: &[u8]) -> Self {
        let patch = PatchParam::from_bytes(&data[0..]);
        let mut dummyv = [0u8; 6];
        dummyv.copy_from_slice(&data[10..16]);
        Self {
            patch,
            output_level: data[PatchParam::SIZE],
            panpot: data[PatchParam::SIZE + 1],
            dummyv,
        }
    }

    pub(crate) fn to_bytes(&self, out: &mut [u8]) {
        self.patch.to_bytes(&mut out[0..]);
        out[PatchParam::SIZE] = self.output_level;
        out[PatchParam::SIZE + 1] = self.panpot;
        out[10..16].copy_from_slice(&self.dummyv);
    }
}

#[derive(Clone, Default)]
pub(crate) struct RhythmTemp {
    pub(crate) timbre: u8, // TIMBRE  0-94 (M1-M64,R1-30,OFF); LAPC-I: 0-127 (M01-M64,R01-R63)
    pub(crate) output_level: u8, // OUTPUT LEVEL 0-100
    pub(crate) panpot: u8, // PANPOT 0-14 (R-L)
    pub(crate) reverb_switch: u8, // REVERB SWITCH 0-1 (OFF,ON)
}

impl RhythmTemp {
    pub(crate) const SIZE: usize = 4;

    pub(crate) fn from_bytes(data: &[u8]) -> Self {
        Self {
            timbre: data[0],
            output_level: data[1],
            panpot: data[2],
            reverb_switch: data[3],
        }
    }
}

// NOTE: There are only 30 timbres in the "rhythm" bank for MT-32; the additional 34 are for LAPC-I and above
#[allow(dead_code)]
#[derive(Clone, Default)]
pub(crate) struct PaddedTimbre {
    pub(crate) timbre: TimbreParam,
    pub(crate) padding: [u8; 10],
}

impl PaddedTimbre {
    pub(crate) const SIZE: usize = TimbreParam::SIZE + 10; // 256

    pub(crate) fn from_bytes(data: &[u8]) -> Self {
        let timbre = TimbreParam::from_bytes(&data[0..]);
        let mut padding = [0u8; 10];
        padding.copy_from_slice(&data[TimbreParam::SIZE..TimbreParam::SIZE + 10]);
        Self { timbre, padding }
    }
}

#[allow(dead_code)]
#[derive(Clone, Default)]
pub(crate) struct SystemParam {
    pub(crate) master_tune: u8,           // MASTER TUNE 0-127 432.1-457.6Hz
    pub(crate) reverb_mode: u8,           // REVERB MODE 0-3 (room, hall, plate, tap delay)
    pub(crate) reverb_time: u8,           // REVERB TIME 0-7 (1-8)
    pub(crate) reverb_level: u8,          // REVERB LEVEL 0-7 (1-8)
    pub(crate) reserve_settings: [u8; 9], // PARTIAL RESERVE (PART 1) 0-32
    pub(crate) chan_assign: [u8; 9],      // MIDI CHANNEL (PART1) 0-16 (1-16,OFF)
    pub(crate) master_vol: u8,            // MASTER VOLUME 0-100
}

impl SystemParam {
    pub(crate) const SIZE: usize = 23;

    pub(crate) fn from_bytes(data: &[u8]) -> Self {
        let mut reserve_settings = [0u8; 9];
        reserve_settings.copy_from_slice(&data[4..13]);
        let mut chan_assign = [0u8; 9];
        chan_assign.copy_from_slice(&data[13..22]);
        Self {
            master_tune: data[0],
            reverb_mode: data[1],
            reverb_time: data[2],
            reverb_level: data[3],
            reserve_settings,
            chan_assign,
            master_vol: data[22],
        }
    }
}

// MemParams is a flat byte buffer representing the MT-32's sysex-addressable memory.
// The C++ code treats this as raw memory that sysex messages can write to at arbitrary
// byte offsets. We preserve this design for faithful sysex handling.
#[derive(Clone)]
/// Mutable MT-32 parameter memory image.
pub(crate) struct MemParams {
    pub(crate) raw: Vec<u8>,
}

impl Default for MemParams {
    fn default() -> Self {
        Self {
            raw: vec![0u8; Self::SIZE],
        }
    }
}

impl MemParams {
    pub(crate) const SIZE: usize = PatchTemp::SIZE * 9
        + RhythmTemp::SIZE * 85
        + TimbreParam::SIZE * 8
        + PatchParam::SIZE * 128
        + PaddedTimbre::SIZE * 256
        + SystemParam::SIZE; // 69035

    /// Byte offsets for each region within the flat buffer.
    pub(crate) const PATCH_TEMP_OFFSET: usize = 0;
    pub(crate) const RHYTHM_TEMP_OFFSET: usize = Self::PATCH_TEMP_OFFSET + PatchTemp::SIZE * 9;
    pub(crate) const TIMBRE_TEMP_OFFSET: usize = Self::RHYTHM_TEMP_OFFSET + RhythmTemp::SIZE * 85;
    pub(crate) const PATCHES_OFFSET: usize = Self::TIMBRE_TEMP_OFFSET + TimbreParam::SIZE * 8;
    pub(crate) const TIMBRES_OFFSET: usize = Self::PATCHES_OFFSET + PatchParam::SIZE * 128;
    pub(crate) const SYSTEM_OFFSET: usize = Self::TIMBRES_OFFSET + PaddedTimbre::SIZE * 256;

    pub(crate) fn patch_temp(&self, index: usize) -> PatchTemp {
        let offset = Self::PATCH_TEMP_OFFSET + index * PatchTemp::SIZE;
        PatchTemp::from_bytes(&self.raw[offset..])
    }

    pub(crate) fn set_patch_temp(&mut self, index: usize, value: &PatchTemp) {
        let offset = Self::PATCH_TEMP_OFFSET + index * PatchTemp::SIZE;
        value.to_bytes(&mut self.raw[offset..]);
    }

    pub(crate) fn rhythm_temp(&self, index: usize) -> RhythmTemp {
        let offset = Self::RHYTHM_TEMP_OFFSET + index * RhythmTemp::SIZE;
        RhythmTemp::from_bytes(&self.raw[offset..])
    }

    pub(crate) fn timbre_temp(&self, index: usize) -> TimbreParam {
        let offset = Self::TIMBRE_TEMP_OFFSET + index * TimbreParam::SIZE;
        TimbreParam::from_bytes(&self.raw[offset..])
    }

    pub(crate) fn set_timbre_temp(&mut self, index: usize, value: &TimbreParam) {
        let offset = Self::TIMBRE_TEMP_OFFSET + index * TimbreParam::SIZE;
        value.to_bytes(&mut self.raw[offset..]);
    }

    pub(crate) fn patch(&self, index: usize) -> PatchParam {
        let offset = Self::PATCHES_OFFSET + index * PatchParam::SIZE;
        PatchParam::from_bytes(&self.raw[offset..])
    }

    pub(crate) fn timbre(&self, index: usize) -> PaddedTimbre {
        let offset = Self::TIMBRES_OFFSET + index * PaddedTimbre::SIZE;
        PaddedTimbre::from_bytes(&self.raw[offset..])
    }

    pub(crate) fn system(&self) -> SystemParam {
        SystemParam::from_bytes(&self.raw[Self::SYSTEM_OFFSET..])
    }
}

#[allow(dead_code)]
#[derive(Clone, Default)]
pub(crate) struct SoundGroup {
    pub(crate) timbre_number_table_addr_low: u8,
    pub(crate) timbre_number_table_addr_high: u8,
    pub(crate) display_position: u8,
    pub(crate) name: [u8; 9],
    pub(crate) timbre_count: u8,
    pub(crate) pad: u8,
}

impl SoundGroup {
    pub(crate) const SIZE: usize = 14;
}

#[allow(dead_code)]
#[derive(Clone, Default)]
pub(crate) struct ControlROMFeatureSet {
    pub(crate) quirk_base_pitch_overflow: bool,
    pub(crate) quirk_pitch_envelope_overflow: bool,
    pub(crate) quirk_ring_modulation_no_mix: bool,
    pub(crate) quirk_tva_zero_env_levels: bool,
    pub(crate) quirk_pan_mult: bool,
    pub(crate) quirk_key_shift: bool,
    pub(crate) quirk_tvf_base_cutoff_limit: bool,
    pub(crate) quirk_fast_pitch_changes: bool,
    pub(crate) quirk_display_custom_message_priority: bool,
    pub(crate) old_mt32_display_features: bool,
    pub(crate) new_gen_note_cancellation: bool,
    /// Features below don't actually depend on control ROM version, which is used to identify hardware model
    pub(crate) default_reverb_mt32_compatible: bool,
    pub(crate) old_mt32_analog_lpf: bool,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct ControlROMMap {
    pub(crate) short_name: &'static str,
    pub(crate) feature_set: &'static ControlROMFeatureSet,
    pub(crate) pcm_table: u16, // 4 * pcmCount bytes
    pub(crate) pcm_count: u16,
    pub(crate) timbre_a_map: u16, // 128 bytes
    pub(crate) timbre_a_offset: u16,
    pub(crate) timbre_a_compressed: bool,
    pub(crate) timbre_b_map: u16, // 128 bytes
    pub(crate) timbre_b_offset: u16,
    pub(crate) timbre_b_compressed: bool,
    pub(crate) timbre_r_map: u16, // 2 * timbreRCount bytes
    pub(crate) timbre_r_count: u16,
    pub(crate) rhythm_settings: u16, // 4 * rhythmSettingsCount bytes
    pub(crate) rhythm_settings_count: u16,
    pub(crate) reserve_settings: u16,   // 9 bytes
    pub(crate) pan_settings: u16,       // 8 bytes
    pub(crate) program_settings: u16,   // 8 bytes
    pub(crate) rhythm_max_table: u16,   // 4 bytes
    pub(crate) patch_max_table: u16,    // 16 bytes
    pub(crate) system_max_table: u16,   // 23 bytes
    pub(crate) timbre_max_table: u16,   // 72 bytes
    pub(crate) sound_groups_table: u16, // 14 bytes each entry
    pub(crate) sound_groups_count: u16,
    pub(crate) startup_message: u16, // 20 characters + NULL terminator
    pub(crate) sysex_error_message: u16, // 20 characters + NULL terminator
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ControlROMPCMStruct {
    pub(crate) pos: u8,
    pub(crate) len: u8,
    pub(crate) pitch_lsb: u8,
    pub(crate) pitch_msb: u8,
}

impl ControlROMPCMStruct {
    pub(crate) const SIZE: usize = 4;

    pub(crate) fn from_bytes(data: &[u8]) -> Self {
        Self {
            pos: data[0],
            len: data[1],
            pitch_lsb: data[2],
            pitch_msb: data[3],
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct PCMWaveEntry {
    pub(crate) addr: u32,
    pub(crate) len: u32,
    pub(crate) loop_flag: bool,
    pub(crate) control_rom_pcm_struct: Option<usize>, // Index into a table, replacing the raw pointer
}

// This is basically a per-partial, pre-processed combination of timbre and patch/rhythm settings
#[derive(Clone, Default)]
/// Register-derived synthesis parameters cached for one partial.
pub(crate) struct PatchCache {
    pub(crate) play_partial: bool,
    pub(crate) pcm_partial: bool,
    pub(crate) pcm: i32,
    pub(crate) waveform: u8,

    pub(crate) structure_mix: u32,
    pub(crate) structure_position: i32,
    pub(crate) structure_pair: i32,

    /// The following fields are actually common to all partials in the timbre
    pub(crate) dirty: bool,
    pub(crate) partial_count: u32,
    pub(crate) sustain: bool,
    pub(crate) reverb: bool,

    pub(crate) src_partial: PartialParam,
}

crate::impl_state_codec!(WGParam {
    pitch_coarse,
    pitch_fine,
    pitch_keyfollow,
    pitch_bender_enabled,
    waveform,
    pcm_wave,
    pulse_width,
    pulse_width_velo_sensitivity,
});

crate::impl_state_codec!(PitchEnvParam {
    depth,
    velo_sensitivity,
    time_keyfollow,
    time,
    level,
});

crate::impl_state_codec!(PitchLFOParam {
    rate,
    depth,
    mod_sensitivity,
});

crate::impl_state_codec!(TVFParam {
    cutoff,
    resonance,
    keyfollow,
    bias_point,
    bias_level,
    env_depth,
    env_velo_sensitivity,
    env_depth_keyfollow,
    env_time_keyfollow,
    env_time,
    env_level,
});

crate::impl_state_codec!(TVAParam {
    level,
    velo_sensitivity,
    bias_point1,
    bias_level1,
    bias_point2,
    bias_level2,
    env_time_keyfollow,
    env_time_velo_sensitivity,
    env_time,
    env_level,
});

crate::impl_state_codec!(PartialParam {
    wg,
    pitch_env,
    pitch_lfo,
    tvf,
    tva,
});

crate::impl_state_codec!(MemParams { raw });

crate::impl_state_codec!(PatchCache {
    play_partial,
    pcm_partial,
    pcm,
    waveform,
    structure_mix,
    structure_position,
    structure_pair,
    dirty,
    partial_count,
    sustain,
    reverb,
    src_partial,
});
