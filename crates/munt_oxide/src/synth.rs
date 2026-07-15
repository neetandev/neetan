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

use crate::{
    enumerations::{DacInputMode, MidiDelayMode, ReverbMode},
    memory_region, part, partial, partial_manager,
    state::*,
    structures::*,
};

// MIDI interface data transfer rate in samples. Used to simulate the transfer delay.
const MIDI_DATA_TRANSFER_RATE: f64 = SAMPLE_RATE as f64 / 31250.0 * 8.0;

const DEFAULT_MASTER_VOLUME: u8 = 100; // Confirmed

static OLD_MT32_ELDER: ControlROMFeatureSet = ControlROMFeatureSet {
    quirk_base_pitch_overflow: true,
    quirk_pitch_envelope_overflow: true,
    quirk_ring_modulation_no_mix: true,
    quirk_tva_zero_env_levels: true,
    quirk_pan_mult: true,
    quirk_key_shift: true,
    quirk_tvf_base_cutoff_limit: true,
    quirk_fast_pitch_changes: false,
    quirk_display_custom_message_priority: true,
    old_mt32_display_features: true,
    new_gen_note_cancellation: false,
    default_reverb_mt32_compatible: true,
    old_mt32_analog_lpf: true,
};

static OLD_MT32_LATER: ControlROMFeatureSet = ControlROMFeatureSet {
    quirk_base_pitch_overflow: true,
    quirk_pitch_envelope_overflow: true,
    quirk_ring_modulation_no_mix: true,
    quirk_tva_zero_env_levels: true,
    quirk_pan_mult: true,
    quirk_key_shift: true,
    quirk_tvf_base_cutoff_limit: true,
    quirk_fast_pitch_changes: false,
    quirk_display_custom_message_priority: false,
    old_mt32_display_features: true,
    new_gen_note_cancellation: false,
    default_reverb_mt32_compatible: true,
    old_mt32_analog_lpf: true,
};

static NEW_MT32_COMPATIBLE: ControlROMFeatureSet = ControlROMFeatureSet {
    quirk_base_pitch_overflow: false,
    quirk_pitch_envelope_overflow: false,
    quirk_ring_modulation_no_mix: false,
    quirk_tva_zero_env_levels: false,
    quirk_pan_mult: false,
    quirk_key_shift: false,
    quirk_tvf_base_cutoff_limit: false,
    quirk_fast_pitch_changes: false,
    quirk_display_custom_message_priority: false,
    old_mt32_display_features: false,
    new_gen_note_cancellation: true,
    default_reverb_mt32_compatible: false,
    old_mt32_analog_lpf: false,
};

static CM32LN_COMPATIBLE: ControlROMFeatureSet = ControlROMFeatureSet {
    quirk_base_pitch_overflow: false,
    quirk_pitch_envelope_overflow: false,
    quirk_ring_modulation_no_mix: false,
    quirk_tva_zero_env_levels: false,
    quirk_pan_mult: false,
    quirk_key_shift: false,
    quirk_tvf_base_cutoff_limit: false,
    quirk_fast_pitch_changes: true,
    quirk_display_custom_message_priority: false,
    old_mt32_display_features: false,
    new_gen_note_cancellation: true,
    default_reverb_mt32_compatible: false,
    old_mt32_analog_lpf: false,
};

//     ID                Features        PCMmap PCMc tmbrA  tmbrAO tmbrAC tmbrB   tmbrBO  tmbrBC tmbrR  trC rhythm rhyC rsrv   panpot  prog   rhyMax patMax sysMax timMax sndGrp sGC stMsg  sErMsg
#[rustfmt::skip]
static CONTROL_ROM_MAPS: &[ControlROMMap] = &[
    ControlROMMap { short_name: "ctrl_mt32_1_04",   feature_set: &OLD_MT32_ELDER,     pcm_table: 0x3000, pcm_count: 128, timbre_a_map: 0x8000, timbre_a_offset: 0x0000, timbre_a_compressed: false, timbre_b_map: 0xC000, timbre_b_offset: 0x4000, timbre_b_compressed: false, timbre_r_map: 0x3200, timbre_r_count: 30, rhythm_settings: 0x73A6, rhythm_settings_count: 85, reserve_settings: 0x57C7, pan_settings: 0x57E2, program_settings: 0x57D0, rhythm_max_table: 0x5252, patch_max_table: 0x525E, system_max_table: 0x526E, timbre_max_table: 0x520A, sound_groups_table: 0x7064, sound_groups_count: 19, startup_message: 0x217A, sysex_error_message: 0x4BB6 },
    ControlROMMap { short_name: "ctrl_mt32_1_05",   feature_set: &OLD_MT32_ELDER,     pcm_table: 0x3000, pcm_count: 128, timbre_a_map: 0x8000, timbre_a_offset: 0x0000, timbre_a_compressed: false, timbre_b_map: 0xC000, timbre_b_offset: 0x4000, timbre_b_compressed: false, timbre_r_map: 0x3200, timbre_r_count: 30, rhythm_settings: 0x7414, rhythm_settings_count: 85, reserve_settings: 0x57C7, pan_settings: 0x57E2, program_settings: 0x57D0, rhythm_max_table: 0x5252, patch_max_table: 0x525E, system_max_table: 0x526E, timbre_max_table: 0x520A, sound_groups_table: 0x70CA, sound_groups_count: 19, startup_message: 0x217A, sysex_error_message: 0x4BB6 },
    ControlROMMap { short_name: "ctrl_mt32_1_06",   feature_set: &OLD_MT32_LATER,     pcm_table: 0x3000, pcm_count: 128, timbre_a_map: 0x8000, timbre_a_offset: 0x0000, timbre_a_compressed: false, timbre_b_map: 0xC000, timbre_b_offset: 0x4000, timbre_b_compressed: false, timbre_r_map: 0x3200, timbre_r_count: 30, rhythm_settings: 0x7414, rhythm_settings_count: 85, reserve_settings: 0x57D9, pan_settings: 0x57F4, program_settings: 0x57E2, rhythm_max_table: 0x5264, patch_max_table: 0x5270, system_max_table: 0x5280, timbre_max_table: 0x521C, sound_groups_table: 0x70CA, sound_groups_count: 19, startup_message: 0x217A, sysex_error_message: 0x4BBA },
    ControlROMMap { short_name: "ctrl_mt32_1_07",   feature_set: &OLD_MT32_LATER,     pcm_table: 0x3000, pcm_count: 128, timbre_a_map: 0x8000, timbre_a_offset: 0x0000, timbre_a_compressed: false, timbre_b_map: 0xC000, timbre_b_offset: 0x4000, timbre_b_compressed: false, timbre_r_map: 0x3200, timbre_r_count: 30, rhythm_settings: 0x73FE, rhythm_settings_count: 85, reserve_settings: 0x57B1, pan_settings: 0x57CC, program_settings: 0x57BA, rhythm_max_table: 0x523C, patch_max_table: 0x5248, system_max_table: 0x5258, timbre_max_table: 0x51F4, sound_groups_table: 0x70B0, sound_groups_count: 19, startup_message: 0x217A, sysex_error_message: 0x4B92 },
    ControlROMMap { short_name: "ctrl_mt32_bluer",  feature_set: &OLD_MT32_LATER,     pcm_table: 0x3000, pcm_count: 128, timbre_a_map: 0x8000, timbre_a_offset: 0x0000, timbre_a_compressed: false, timbre_b_map: 0xC000, timbre_b_offset: 0x4000, timbre_b_compressed: false, timbre_r_map: 0x3200, timbre_r_count: 30, rhythm_settings: 0x741C, rhythm_settings_count: 85, reserve_settings: 0x57E5, pan_settings: 0x5800, program_settings: 0x57EE, rhythm_max_table: 0x5270, patch_max_table: 0x527C, system_max_table: 0x528C, timbre_max_table: 0x5228, sound_groups_table: 0x70CE, sound_groups_count: 19, startup_message: 0x217A, sysex_error_message: 0x4BC6 },
    ControlROMMap { short_name: "ctrl_mt32_2_03",   feature_set: &NEW_MT32_COMPATIBLE, pcm_table: 0x8100, pcm_count: 128, timbre_a_map: 0x8000, timbre_a_offset: 0x8000, timbre_a_compressed: true,  timbre_b_map: 0x8080, timbre_b_offset: 0x8000, timbre_b_compressed: true,  timbre_r_map: 0x8500, timbre_r_count: 64, rhythm_settings: 0x8580, rhythm_settings_count: 85, reserve_settings: 0x4F49, pan_settings: 0x4F64, program_settings: 0x4F52, rhythm_max_table: 0x4885, patch_max_table: 0x4889, system_max_table: 0x48A2, timbre_max_table: 0x48B9, sound_groups_table: 0x5A44, sound_groups_count: 19, startup_message: 0x1EF0, sysex_error_message: 0x4066 },
    ControlROMMap { short_name: "ctrl_mt32_2_04",   feature_set: &NEW_MT32_COMPATIBLE, pcm_table: 0x8100, pcm_count: 128, timbre_a_map: 0x8000, timbre_a_offset: 0x8000, timbre_a_compressed: true,  timbre_b_map: 0x8080, timbre_b_offset: 0x8000, timbre_b_compressed: true,  timbre_r_map: 0x8500, timbre_r_count: 64, rhythm_settings: 0x8580, rhythm_settings_count: 85, reserve_settings: 0x4F5D, pan_settings: 0x4F78, program_settings: 0x4F66, rhythm_max_table: 0x4899, patch_max_table: 0x489D, system_max_table: 0x48B6, timbre_max_table: 0x48CD, sound_groups_table: 0x5A58, sound_groups_count: 19, startup_message: 0x1EF0, sysex_error_message: 0x406D },
    ControlROMMap { short_name: "ctrl_mt32_2_06",   feature_set: &NEW_MT32_COMPATIBLE, pcm_table: 0x8100, pcm_count: 128, timbre_a_map: 0x8000, timbre_a_offset: 0x8000, timbre_a_compressed: true,  timbre_b_map: 0x8080, timbre_b_offset: 0x8000, timbre_b_compressed: true,  timbre_r_map: 0x8500, timbre_r_count: 64, rhythm_settings: 0x8580, rhythm_settings_count: 85, reserve_settings: 0x4F69, pan_settings: 0x4F84, program_settings: 0x4F72, rhythm_max_table: 0x48A5, patch_max_table: 0x48A9, system_max_table: 0x48C2, timbre_max_table: 0x48D9, sound_groups_table: 0x5A64, sound_groups_count: 19, startup_message: 0x1EF0, sysex_error_message: 0x4021 },
    ControlROMMap { short_name: "ctrl_mt32_2_07",   feature_set: &NEW_MT32_COMPATIBLE, pcm_table: 0x8100, pcm_count: 128, timbre_a_map: 0x8000, timbre_a_offset: 0x8000, timbre_a_compressed: true,  timbre_b_map: 0x8080, timbre_b_offset: 0x8000, timbre_b_compressed: true,  timbre_r_map: 0x8500, timbre_r_count: 64, rhythm_settings: 0x8580, rhythm_settings_count: 85, reserve_settings: 0x4F81, pan_settings: 0x4F9C, program_settings: 0x4F8A, rhythm_max_table: 0x48B9, patch_max_table: 0x48BD, system_max_table: 0x48D6, timbre_max_table: 0x48ED, sound_groups_table: 0x5A78, sound_groups_count: 19, startup_message: 0x1EE7, sysex_error_message: 0x4035 },
    ControlROMMap { short_name: "ctrl_cm32l_1_00",  feature_set: &NEW_MT32_COMPATIBLE, pcm_table: 0x8100, pcm_count: 256, timbre_a_map: 0x8000, timbre_a_offset: 0x8000, timbre_a_compressed: true,  timbre_b_map: 0x8080, timbre_b_offset: 0x8000, timbre_b_compressed: true,  timbre_r_map: 0x8500, timbre_r_count: 64, rhythm_settings: 0x8580, rhythm_settings_count: 85, reserve_settings: 0x4F65, pan_settings: 0x4F80, program_settings: 0x4F6E, rhythm_max_table: 0x48A1, patch_max_table: 0x48A5, system_max_table: 0x48BE, timbre_max_table: 0x48D5, sound_groups_table: 0x5A6C, sound_groups_count: 19, startup_message: 0x1EF0, sysex_error_message: 0x401D },
    ControlROMMap { short_name: "ctrl_cm32l_1_02",  feature_set: &NEW_MT32_COMPATIBLE, pcm_table: 0x8100, pcm_count: 256, timbre_a_map: 0x8000, timbre_a_offset: 0x8000, timbre_a_compressed: true,  timbre_b_map: 0x8080, timbre_b_offset: 0x8000, timbre_b_compressed: true,  timbre_r_map: 0x8500, timbre_r_count: 64, rhythm_settings: 0x8580, rhythm_settings_count: 85, reserve_settings: 0x4F93, pan_settings: 0x4FAE, program_settings: 0x4F9C, rhythm_max_table: 0x48CB, patch_max_table: 0x48CF, system_max_table: 0x48E8, timbre_max_table: 0x48FF, sound_groups_table: 0x5A96, sound_groups_count: 19, startup_message: 0x1EE7, sysex_error_message: 0x4047 },
    ControlROMMap { short_name: "ctrl_cm32ln_1_00", feature_set: &CM32LN_COMPATIBLE,   pcm_table: 0x8100, pcm_count: 256, timbre_a_map: 0x8000, timbre_a_offset: 0x8000, timbre_a_compressed: true,  timbre_b_map: 0x8080, timbre_b_offset: 0x8000, timbre_b_compressed: true,  timbre_r_map: 0x8500, timbre_r_count: 64, rhythm_settings: 0x8580, rhythm_settings_count: 85, reserve_settings: 0x4EC7, pan_settings: 0x4EE2, program_settings: 0x4ED0, rhythm_max_table: 0x47FF, patch_max_table: 0x4803, system_max_table: 0x481C, timbre_max_table: 0x4833, sound_groups_table: 0x55A2, sound_groups_count: 19, startup_message: 0x1F59, sysex_error_message: 0x3F7C },
];

fn get_control_rom_map(short_name: &str) -> Option<&'static ControlROMMap> {
    CONTROL_ROM_MAPS.iter().find(|m| m.short_name == short_name)
}

pub(crate) fn calc_sysex_checksum(data: &[u8], init_checksum: u8) -> u8 {
    let mut checksum = (-(init_checksum as i32)) as u32;
    for &byte in data {
        checksum = checksum.wrapping_sub(byte as u32);
    }
    (checksum & 0x7F) as u8
}

fn load_control_rom(state: &mut MuntState, control_rom_data: &[u8], short_name: &str) -> bool {
    let rom_map = match get_control_rom_map(short_name) {
        Some(m) => m,
        None => return false,
    };

    let copy_len = CONTROL_ROM_SIZE.min(control_rom_data.len());
    state.rom.control_rom_data[..copy_len].copy_from_slice(&control_rom_data[..copy_len]);
    state.rom.control_rom_map = Some(rom_map);
    state.rom.control_rom_features = rom_map.feature_set.clone();
    true
}

fn load_pcm_rom(state: &mut MuntState, pcm_rom_data: &[u8]) -> bool {
    let pcm_rom_size = state.rom.pcm_rom_data.len();
    if pcm_rom_data.len() != pcm_rom_size * 2 {
        return false;
    }

    let order: [usize; 16] = [0, 9, 1, 2, 3, 4, 5, 6, 7, 10, 11, 12, 13, 14, 15, 8];
    let mut file_pos = 0;
    for i in 0..pcm_rom_size {
        let s = pcm_rom_data[file_pos];
        file_pos += 1;
        let c = pcm_rom_data[file_pos];
        file_pos += 1;

        let mut log: i16 = 0;
        for (u, &order_val) in order.iter().enumerate() {
            let bit = if order_val < 8 {
                ((s >> (7 - order_val)) & 0x1) as i16
            } else {
                ((c >> (7 - (order_val - 8))) & 0x1) as i16
            };
            log |= bit << (15 - u);
        }
        state.rom.pcm_rom_data[i] = log;
    }
    true
}

fn init_pcm_list(state: &mut MuntState) -> bool {
    let rom_map = match state.rom.control_rom_map {
        Some(m) => m,
        None => return false,
    };
    let map_address = rom_map.pcm_table as usize;
    let count = rom_map.pcm_count as usize;
    let pcm_rom_size = state.rom.pcm_rom_data.len();

    state.rom.pcm_waves.clear();
    state.rom.pcm_waves.resize(count, PCMWaveEntry::default());
    state.rom.pcm_rom_structs.clear();
    state
        .rom
        .pcm_rom_structs
        .resize(count, ControlROMPCMStruct::default());

    for i in 0..count {
        let struct_offset = map_address + i * ControlROMPCMStruct::SIZE;
        let tps = ControlROMPCMStruct::from_bytes(
            &state.rom.control_rom_data[struct_offset..struct_offset + ControlROMPCMStruct::SIZE],
        );

        let r_addr = (tps.pos as u32) * 0x800;
        let r_len_exp = ((tps.len & 0x70) >> 4) as u32;
        let r_len = 0x800u32 << r_len_exp;
        if (r_addr + r_len) as usize > pcm_rom_size {
            return false;
        }
        state.rom.pcm_rom_structs[i] = tps;
        state.rom.pcm_waves[i].addr = r_addr;
        state.rom.pcm_waves[i].len = r_len;
        state.rom.pcm_waves[i].loop_flag = (tps.len & 0x80) != 0;
        state.rom.pcm_waves[i].control_rom_pcm_struct = Some(i);
    }
    false // C++ returns false here too (misleading but faithful port)
}

fn init_compressed_timbre(state: &mut MuntState, timbre_num: usize, src: &[u8]) -> bool {
    // "Compressed" here means that muted partials aren't present in ROM
    // (except in the case of partial 0 being muted).
    // Instead the data from the previous unmuted partial is used.
    if src.len() < CommonParam::SIZE {
        return false;
    }

    // Write common param
    let timbre_offset = timbre_num * PaddedTimbre::SIZE;
    let dest = &mut state.mt32_ram.raw;
    let timbres_base = MemParams::TIMBRES_OFFSET;
    dest[timbres_base + timbre_offset..timbres_base + timbre_offset + CommonParam::SIZE]
        .copy_from_slice(&src[..CommonParam::SIZE]);

    let mut src_pos = CommonParam::SIZE;
    let mut mem_pos = CommonParam::SIZE;

    // Read the partial mute from what we just wrote
    let partial_mute = src[8]; // common.partialMute offset is 8

    for t in 0..4 {
        if t != 0 && ((partial_mute >> t) & 0x1) == 0x00 {
            // This partial is muted - copy the previously copied partial
            src_pos -= PartialParam::SIZE;
        } else if src_pos + PartialParam::SIZE >= src.len() {
            return false;
        }
        let dest_off = timbres_base + timbre_offset + mem_pos;
        dest[dest_off..dest_off + PartialParam::SIZE]
            .copy_from_slice(&src[src_pos..src_pos + PartialParam::SIZE]);
        src_pos += PartialParam::SIZE;
        mem_pos += PartialParam::SIZE;
    }
    true
}

fn init_timbres(
    state: &mut MuntState,
    map_address: u16,
    offset: u16,
    count: u16,
    start_timbre: u16,
    compressed: bool,
) -> bool {
    let timbre_map_start = map_address as usize;
    let mut start = start_timbre;

    for i in 0..(count as usize) * 2 {
        if i % 2 != 0 {
            continue;
        }
        let idx = timbre_map_start + i;
        if idx + 1 >= CONTROL_ROM_SIZE {
            return false;
        }
        let address = (state.rom.control_rom_data[idx + 1] as u16) << 8
            | state.rom.control_rom_data[idx] as u16;
        let address = (address + offset) as usize;

        if compressed {
            let rom_len = CONTROL_ROM_SIZE.saturating_sub(address);
            let src: Vec<u8> = state.rom.control_rom_data[address..address + rom_len].to_vec();
            if !init_compressed_timbre(state, start as usize, &src) {
                return false;
            }
        } else {
            if address + TimbreParam::SIZE > CONTROL_ROM_SIZE {
                return false;
            }
            let timbres_base = MemParams::TIMBRES_OFFSET;
            let timbre_offset = start as usize * PaddedTimbre::SIZE;
            let src: Vec<u8> =
                state.rom.control_rom_data[address..address + TimbreParam::SIZE].to_vec();
            state.mt32_ram.raw
                [timbres_base + timbre_offset..timbres_base + timbre_offset + TimbreParam::SIZE]
                .copy_from_slice(&src);
        }
        start += 1;
    }
    true
}

fn init_reverb_models(state: &mut MuntState, mt32_compatible_mode: bool) {
    for mode in 0..4 {
        let reverb_mode = match mode {
            0 => ReverbMode::Room,
            1 => ReverbMode::Hall,
            2 => ReverbMode::Plate,
            _ => ReverbMode::TapDelay,
        };
        state.reverb_models[mode] = BReverbModelState::new(reverb_mode, mt32_compatible_mode);
    }
}

fn init_sound_groups(state: &mut MuntState) {
    let rom_map = match state.rom.control_rom_map {
        Some(m) => m,
        None => return,
    };
    let table_addr = rom_map.sound_groups_table as usize;
    let ix_start = table_addr - 128;

    // Copy soundGroupIx
    for i in 0..128 {
        state.rom.sound_group_ix[i] = state.rom.control_rom_data[ix_start + i];
    }

    // Copy sound group names
    state.rom.sound_group_names.clear();
    for i in 0..rom_map.sound_groups_count as usize {
        let offset = table_addr + i * SoundGroup::SIZE;
        let mut name = [0u8; 9];
        let copy_len = 9.min(SoundGroup::SIZE);
        name[..copy_len].copy_from_slice(&state.rom.control_rom_data[offset..offset + copy_len]);
        state.rom.sound_group_names.push(name);
    }
}

fn init_memory_regions(state: &mut MuntState) {
    state.memory_regions = MemoryRegionDescriptors::new();

    // Build padded timbre max table
    let rom_map = match state.rom.control_rom_map {
        Some(m) => m,
        None => return,
    };
    let max_table_addr = rom_map.timbre_max_table as usize;
    let common_size = CommonParam::SIZE;
    let partial_size = PartialParam::SIZE;

    state.rom.padded_timbre_max_table = vec![0u8; PaddedTimbre::SIZE];
    // Copy commonParam + one partialParam
    let src_len = common_size + partial_size;
    if max_table_addr + src_len <= CONTROL_ROM_SIZE {
        let source = state.rom.control_rom_data[max_table_addr..max_table_addr + src_len].to_vec();
        state.rom.padded_timbre_max_table[..src_len].copy_from_slice(&source);
    }
    // Replicate the single partialParam for the remaining 3
    let partial_src_start = max_table_addr + common_size;
    let mut pos = common_size + partial_size;
    for _ in 0..3 {
        if partial_src_start + partial_size <= CONTROL_ROM_SIZE {
            let src: Vec<u8> = state.rom.control_rom_data
                [partial_src_start..partial_src_start + partial_size]
                .to_vec();
            state.rom.padded_timbre_max_table[pos..pos + partial_size].copy_from_slice(&src);
        }
        pos += partial_size;
    }
    // Zero the 10-byte padding
    let pad_start = pos;
    let pad_end = (pad_start + 10).min(PaddedTimbre::SIZE);
    state.rom.padded_timbre_max_table[pad_start..pad_end].fill(0);
}

pub(crate) fn open(
    state: &mut MuntState,
    control_rom_data: &[u8],
    pcm_rom_data: &[u8],
    control_rom_short_name: &str,
) -> bool {
    if state.opened {
        return false;
    }
    state.partial_count = DEFAULT_MAX_PARTIALS as u32;
    state.aborting_poly_index = None;
    state.extensions.aborting_part_ix = 0;

    // This is to help detect bugs
    state.mt32_ram.raw.fill(b'?');

    if !load_control_rom(state, control_rom_data, control_rom_short_name) {
        return false;
    }

    init_memory_regions(state);

    // 512KB PCM ROM for MT-32, etc.
    // 1MB PCM ROM for CM-32L, LAPC-I, CM-64, CM-500
    // Note that the size below is given in samples (16-bit), therefore half the number of bytes in the ROM
    let rom_map = state.rom.control_rom_map.unwrap();
    let pcm_rom_size = if rom_map.pcm_count == 256 {
        512 * 1024
    } else {
        256 * 1024
    };
    state.rom.pcm_rom_data = vec![0i16; pcm_rom_size];

    if !load_pcm_rom(state, pcm_rom_data) {
        return false;
    }

    let mt32_compatible_reverb = state
        .rom
        .control_rom_features
        .default_reverb_mt32_compatible;
    init_reverb_models(state, mt32_compatible_reverb);

    // Initialise Timbre Bank A
    if !init_timbres(
        state,
        rom_map.timbre_a_map,
        rom_map.timbre_a_offset,
        0x40,
        0,
        rom_map.timbre_a_compressed,
    ) {
        return false;
    }

    // Initialise Timbre Bank B
    if !init_timbres(
        state,
        rom_map.timbre_b_map,
        rom_map.timbre_b_offset,
        0x40,
        64,
        rom_map.timbre_b_compressed,
    ) {
        return false;
    }

    // Initialise Timbre Bank R
    if !init_timbres(
        state,
        rom_map.timbre_r_map,
        0,
        rom_map.timbre_r_count,
        192,
        true,
    ) {
        return false;
    }

    if rom_map.timbre_r_count == 30 {
        // We must initialise all 64 rhythm timbres to avoid undefined behaviour.
        // SEMI-CONFIRMED: Old-gen MT-32 units likely map timbres 30..59 to 0..29.
        // Attempts to play rhythm timbres 60..63 exhibit undefined behaviour.
        // We want to emulate the wrap around, so merely copy the entire set of standard
        // timbres once more. The last 4 dangerous timbres are zeroed out.
        let timbres_base = MemParams::TIMBRES_OFFSET;
        let src_start = timbres_base + 192 * PaddedTimbre::SIZE;
        let src_end = src_start + 30 * PaddedTimbre::SIZE;
        let dst_start = timbres_base + 222 * PaddedTimbre::SIZE;
        let src_data: Vec<u8> = state.mt32_ram.raw[src_start..src_end].to_vec();
        state.mt32_ram.raw[dst_start..dst_start + src_data.len()].copy_from_slice(&src_data);
        // Zero out the last 4 dangerous timbres
        let zero_start = timbres_base + 252 * PaddedTimbre::SIZE;
        let zero_end = zero_start + 4 * PaddedTimbre::SIZE;
        state.mt32_ram.raw[zero_start..zero_end].fill(0);
    }

    // CM-64 seems to initialise all bytes in this bank to 0.
    let timbres_base = MemParams::TIMBRES_OFFSET;
    let bank_m_start = timbres_base + 128 * PaddedTimbre::SIZE;
    let bank_m_end = bank_m_start + 64 * PaddedTimbre::SIZE;
    state.mt32_ram.raw[bank_m_start..bank_m_end].fill(0);

    // Allocate partials
    partial_manager::init(state);

    // Initialise PCM List
    init_pcm_list(state);

    // Initialise Rhythm Temp
    let rhythm_addr = rom_map.rhythm_settings as usize;
    let rhythm_count = rom_map.rhythm_settings_count as usize;
    let rhythm_size = rhythm_count * RhythmTemp::SIZE;
    let rhythm_base = MemParams::RHYTHM_TEMP_OFFSET;
    if rhythm_addr + rhythm_size <= CONTROL_ROM_SIZE {
        let src: Vec<u8> =
            state.rom.control_rom_data[rhythm_addr..rhythm_addr + rhythm_size].to_vec();
        state.mt32_ram.raw[rhythm_base..rhythm_base + rhythm_size].copy_from_slice(&src);
    }

    // Initialise Patches
    let patches_base = MemParams::PATCHES_OFFSET;
    for i in 0..128u8 {
        let offset = patches_base + i as usize * PatchParam::SIZE;
        let mut patch_bytes = [0u8; PatchParam::SIZE];
        patch_bytes[0] = i / 64; // timbreGroup
        patch_bytes[1] = i % 64; // timbreNum
        patch_bytes[2] = 24; // keyShift
        patch_bytes[3] = 50; // fineTune
        patch_bytes[4] = 12; // benderRange
        patch_bytes[5] = 0; // assignMode
        patch_bytes[6] = 1; // reverbSwitch
        patch_bytes[7] = 0; // dummy
        state.mt32_ram.raw[offset..offset + PatchParam::SIZE].copy_from_slice(&patch_bytes);
    }

    // Initialise System
    let system_base = MemParams::SYSTEM_OFFSET;
    // The MT-32 manual claims that "Standard pitch" is 442Hz.
    state.mt32_ram.raw[system_base + SYSTEM_MASTER_TUNE_OFF] = 0x4A; // Confirmed on CM-64
    state.mt32_ram.raw[system_base + SYSTEM_REVERB_MODE_OFF] = 0; // Confirmed
    state.mt32_ram.raw[system_base + SYSTEM_REVERB_TIME_OFF] = 5; // Confirmed
    state.mt32_ram.raw[system_base + SYSTEM_REVERB_LEVEL_OFF] = 3; // Confirmed

    // Reserve settings
    let reserve_addr = rom_map.reserve_settings as usize;
    if reserve_addr + 9 <= CONTROL_ROM_SIZE {
        let src: Vec<u8> = state.rom.control_rom_data[reserve_addr..reserve_addr + 9].to_vec();
        state.mt32_ram.raw[system_base + SYSTEM_RESERVE_SETTINGS_START_OFF
            ..system_base + SYSTEM_RESERVE_SETTINGS_START_OFF + 9]
            .copy_from_slice(&src);
    }

    // Channel assignments: default {1, 2, 3, 4, 5, 6, 7, 8, 9}
    for i in 0..9u8 {
        state.mt32_ram.raw[system_base + SYSTEM_CHAN_ASSIGN_START_OFF + i as usize] = i + 1;
    }
    state.mt32_ram.raw[system_base + SYSTEM_MASTER_VOL_OFF] = DEFAULT_MASTER_VOLUME;

    let old_reverb_overridden = state.reverb_overridden;
    state.reverb_overridden = false;
    refresh_system(state);
    reset_master_tune_pitch_delta(state);
    state.reverb_overridden = old_reverb_overridden;

    init_sound_groups(state);

    // Initialise parts and patchTemp
    let patch_temp_base = MemParams::PATCH_TEMP_OFFSET;
    for i in 0..9 {
        let pt_offset = patch_temp_base + i * PatchTemp::SIZE;
        // Set default patch temp fields
        state.mt32_ram.raw[pt_offset] = 0; // patch.timbreGroup
        state.mt32_ram.raw[pt_offset + 1] = 0; // patch.timbreNum
        state.mt32_ram.raw[pt_offset + 2] = 24; // patch.keyShift
        state.mt32_ram.raw[pt_offset + 3] = 50; // patch.fineTune
        state.mt32_ram.raw[pt_offset + 4] = 12; // patch.benderRange
        state.mt32_ram.raw[pt_offset + 5] = 0; // patch.assignMode
        state.mt32_ram.raw[pt_offset + 6] = 1; // patch.reverbSwitch
        state.mt32_ram.raw[pt_offset + 7] = 0; // patch.dummy

        state.mt32_ram.raw[pt_offset + PatchParam::SIZE] = 80; // outputLevel
        let pan_addr = rom_map.pan_settings as usize + i;
        if pan_addr < CONTROL_ROM_SIZE {
            state.mt32_ram.raw[pt_offset + PatchParam::SIZE + 1] =
                state.rom.control_rom_data[pan_addr]; // panpot
        }
        // Zero all dummyv bytes, then set dummyv[1]
        for d in 10..16 {
            state.mt32_ram.raw[pt_offset + d] = 0;
        }
        state.mt32_ram.raw[pt_offset + 11] = 127; // dummyv[1]

        if i < 8 {
            part::init_part(state, i);
            let prog_addr = rom_map.program_settings as usize + i;
            if prog_addr < CONTROL_ROM_SIZE {
                let program = state.rom.control_rom_data[prog_addr];
                part::set_program(state, i, program as u32);
            }
        } else {
            part::init_rhythm_part(state, i);
        }
    }

    // For resetting MT-32 mid-execution
    state.mt32_default = state.mt32_ram.clone();

    // Initialize MIDI queue
    state.midi_queue.init(DEFAULT_MIDI_EVENT_QUEUE_SIZE as u32);

    // Set up analog output (coarse mode)
    state.analog.old_mt32_analog_lpf = state.rom.control_rom_features.old_mt32_analog_lpf;
    state.analog.set_synth_output_gain(state.output_gain);
    let mt32_compat = state
        .rom
        .control_rom_features
        .default_reverb_mt32_compatible;
    state
        .analog
        .set_reverb_output_gain(state.reverb_output_gain, mt32_compat);

    if state.extensions.master_volume_override < DEFAULT_MASTER_VOLUME {
        let system_base = MemParams::SYSTEM_OFFSET;
        state.mt32_ram.raw[system_base + SYSTEM_MASTER_VOL_OFF] =
            state.extensions.master_volume_override;
        refresh_system_master_vol(state);
    }

    state.opened = true;
    state.activated = false;
    true
}

pub(crate) fn close(state: &mut MuntState) {
    if state.opened {
        state.opened = false;
    }
}

fn add_midi_interface_delay(state: &mut MuntState, len: u32, mut timestamp: u32) -> u32 {
    let transfer_time = (len as f64 * MIDI_DATA_TRANSFER_RATE) as u32;
    // Dealing with wrapping
    if (timestamp as i32).wrapping_sub(state.last_received_midi_event_timestamp as i32) < 0 {
        timestamp = state.last_received_midi_event_timestamp;
    }
    timestamp = timestamp.wrapping_add(transfer_time);
    state.last_received_midi_event_timestamp = timestamp;
    timestamp
}

pub(crate) fn get_short_message_length(msg: u32) -> u32 {
    if (msg & 0xF0) == 0xF0 {
        return match msg & 0xFF {
            0xF1 | 0xF3 => 2,
            0xF2 => 3,
            _ => 1,
        };
    }
    // NOTE: This calculation isn't quite correct
    // as it doesn't consider the running status byte
    if (msg & 0xE0) == 0xC0 { 2 } else { 3 }
}

pub(crate) fn play_msg(state: &mut MuntState, msg: u32) -> bool {
    play_msg_with_timestamp(state, msg, state.rendered_sample_count)
}

pub(crate) fn play_msg_with_timestamp(state: &mut MuntState, msg: u32, mut timestamp: u32) -> bool {
    if (msg & 0xF8) == 0xF8 {
        // System realtime message - ignored in this port
        return true;
    }
    if state.midi_delay_mode != MidiDelayMode::Immediate {
        timestamp = add_midi_interface_delay(state, get_short_message_length(msg), timestamp);
    }
    if !state.activated {
        state.activated = true;
    }
    state.midi_queue.push_short_message(msg, timestamp)
}

pub(crate) fn play_sysex(state: &mut MuntState, sysex: &[u8]) -> bool {
    play_sysex_with_timestamp(state, sysex, state.rendered_sample_count)
}

pub(crate) fn play_sysex_with_timestamp(
    state: &mut MuntState,
    sysex: &[u8],
    mut timestamp: u32,
) -> bool {
    if state.midi_delay_mode == MidiDelayMode::DelayAll {
        timestamp = add_midi_interface_delay(state, sysex.len() as u32, timestamp);
    }
    if !state.activated {
        state.activated = true;
    }
    state.midi_queue.push_sysex(sysex, timestamp)
}

pub(crate) fn play_msg_now(state: &mut MuntState, msg: u32) {
    if !state.opened {
        return;
    }

    let command = ((msg & 0x0000F0) >> 4) as u8;
    let chan = (msg & 0x00000F) as u8;
    let data1 = ((msg & 0x00FF00) >> 8) as u8;
    let data2 = ((msg & 0xFF0000) >> 16) as u8;

    if data1 > 127 || data2 > 127 {
        return;
    }

    let chan_parts = state.extensions.chan_table[chan as usize];
    if chan_parts[0] > 8 {
        return;
    }
    let start_ix = state.extensions.aborting_part_ix as usize;
    for (i, &chan_part) in chan_parts.iter().enumerate().skip(start_ix) {
        let part_num = chan_part as u32;
        if part_num > 8 {
            break;
        }
        play_unpacked_short_message(state, part_num as u8, command, data1, data2);
        if state.aborting_poly_index.is_some() {
            state.extensions.aborting_part_ix = i as u32;
            break;
        } else if state.extensions.aborting_part_ix != 0 {
            state.extensions.aborting_part_ix = 0;
        }
    }
}

fn play_unpacked_short_message(
    state: &mut MuntState,
    part_num: u8,
    command: u8,
    data1: u8,
    data2: u8,
) {
    if !state.activated {
        state.activated = true;
    }
    let p = part_num as usize;

    match command {
        0x8 => {
            // The MT-32 ignores velocity for note off
            part::note_off(state, p, data1 as u32);
        }
        0x9 => {
            if data2 == 0 {
                // MIDI defines note-on with velocity 0 as being the same as note-off with velocity 40
                part::note_off(state, p, data1 as u32);
            } else if part::get_volume_override(state, p) > 0 {
                part::note_on(state, p, data1 as u32, data2 as u32);
            }
        }
        0xB => {
            // Control change
            match data1 {
                0x01 => part::set_modulation(state, p, data2 as u32),
                0x06 => part::set_data_entry_msb(state, p, data2),
                0x07 => part::set_volume(state, p, data2 as u32),
                0x0A => part::set_pan(state, p, data2 as u32),
                0x0B => part::set_expression(state, p, data2 as u32),
                0x40 => part::set_hold_pedal(state, p, data2 >= 64),
                0x62 | 0x63 => part::set_nrpn(state, p),
                0x64 => part::set_rpn_lsb(state, p, data2),
                0x65 => part::set_rpn_msb(state, p, data2),
                0x79 => part::reset_all_controllers(state, p),
                0x7B => part::all_notes_off(state, p),
                0x7C..=0x7F => {
                    // CONFIRMED:Mok: A real LAPC-I responds to these controllers as follows:
                    part::set_hold_pedal(state, p, false);
                    part::all_notes_off(state, p);
                }
                _ => {}
            }
        }
        0xC => {
            // Program change
            part::set_program(state, p, data1 as u32);
        }
        0xE => {
            // Pitch bender
            part::set_bend(state, p, (((data2 as u16) << 7) | data1 as u16) as u32);
        }
        _ => {}
    }
}

pub(crate) fn play_sysex_now(state: &mut MuntState, sysex: &[u8]) {
    if sysex.len() < 2 {
        return;
    }
    if sysex[0] != 0xF0 {
        return;
    }
    // Find end marker
    let mut end_pos = 1;
    while end_pos < sysex.len() {
        if sysex[end_pos] == 0xF7 {
            break;
        }
        end_pos += 1;
    }
    if end_pos == sysex.len() {
        return;
    }
    play_sysex_without_framing(state, &sysex[1..end_pos]);
}

fn play_sysex_without_framing(state: &mut MuntState, sysex: &[u8]) {
    if sysex.len() < 4 {
        return;
    }
    if sysex[0] != SYSEX_MANUFACTURER_ROLAND {
        return;
    }
    if sysex[2] != SYSEX_MDL_MT32 {
        return;
    }
    play_sysex_without_header(state, sysex[1], sysex[3], &sysex[4..]);
}

fn play_sysex_without_header(state: &mut MuntState, device: u8, command: u8, sysex: &[u8]) {
    if device > 0x10 {
        return;
    }

    // All models process the checksum before anything else and ignore messages
    // lacking the checksum, or containing the checksum only.
    if sysex.len() < 2 {
        return;
    }
    let checksum = calc_sysex_checksum(&sysex[..sysex.len() - 1], 0);
    if checksum != sysex[sysex.len() - 1] {
        return;
    }
    let sysex = &sysex[..sysex.len() - 1]; // Exclude checksum

    if command == SYSEX_CMD_EOD {
        return;
    }
    match command {
        SYSEX_CMD_WSD => {}
        SYSEX_CMD_DAT | SYSEX_CMD_DT1 => {
            write_sysex(state, device, sysex);
        }
        SYSEX_CMD_RQD | SYSEX_CMD_RQ1 => {
            // Read sysex - NYI
        }
        _ => {}
    }
}

fn write_sysex(state: &mut MuntState, device: u8, sysex: &[u8]) {
    if !state.opened || sysex.is_empty() {
        return;
    }

    // This is checked early in the real devices (before any sysex length checks or further processing)
    if sysex[0] == 0x7F {
        reset(state);
        return;
    }

    if sysex.len() < 3 {
        return;
    }

    let raw_addr = ((sysex[0] as u32) << 16) | ((sysex[1] as u32) << 8) | (sysex[2] as u32);
    let mut addr = memaddr(raw_addr);
    let data = &sysex[3..];
    let _len = data.len() as u32;

    // Process channel-specific sysex by converting it to device-global
    if device < 0x10 {
        if addr < memaddr(0x010000) {
            let base = memaddr(0x030000);
            let global_addr = addr + base;
            let chan_parts = state.extensions.chan_table[device as usize];
            if chan_parts[0] > 8 {
                // Channel not mapped
            } else {
                for &chan_part in &chan_parts {
                    if chan_part > 8 {
                        break;
                    }
                    let offset = if chan_part == 8 {
                        0
                    } else {
                        chan_part as u32 * PatchTemp::SIZE as u32
                    };
                    write_sysex_global(state, global_addr + offset, data);
                }
                return;
            }
            addr = global_addr;
        } else if addr < memaddr(0x020000) {
            addr = addr + memaddr(0x030110) - memaddr(0x010000);
        } else if addr < memaddr(0x030000) {
            let base = memaddr(0x040000) - memaddr(0x020000);
            let global_addr = addr + base;
            let chan_parts = state.extensions.chan_table[device as usize];
            if chan_parts[0] > 8 {
                // Channel not mapped
            } else {
                for &chan_part in &chan_parts {
                    if chan_part > 8 {
                        break;
                    }
                    let offset = if chan_part == 8 {
                        0
                    } else {
                        chan_part as u32 * TimbreParam::SIZE as u32
                    };
                    write_sysex_global(state, global_addr + offset, data);
                }
                return;
            }
            addr = global_addr;
        } else {
            return;
        }
    }
    write_sysex_global(state, addr, data);
}

fn write_sysex_global(state: &mut MuntState, mut addr: u32, data: &[u8]) {
    let mut remaining = data;
    loop {
        let region_type = state.memory_regions.find_region(addr);
        let Some(region_type) = region_type else {
            break;
        };

        let descriptor = state.memory_regions.get_descriptor(region_type);
        let clamped_len = descriptor.get_clamped_len(addr, remaining.len() as u32);
        let next = descriptor.next_region_len(addr, remaining.len() as u32);

        write_memory_region(state, region_type, addr, &remaining[..clamped_len as usize]);

        if next == 0 {
            break;
        }
        addr += next;
        remaining = &remaining[next as usize..];
    }
}

fn write_memory_region(
    state: &mut MuntState,
    region_type: memory_region::MemoryRegionType,
    addr: u32,
    data: &[u8],
) {
    let descriptor = state.memory_regions.get_descriptor(region_type);
    let first = descriptor.first_touched(addr) as usize;
    let last = descriptor.last_touched(addr, data.len() as u32) as usize;
    let off = descriptor.first_touched_offset(addr) as usize;

    use memory_region::MemoryRegionType::*;
    match region_type {
        PatchTemp => {
            descriptor.write(
                Some(&mut state.mt32_ram.raw[MemParams::PATCH_TEMP_OFFSET..]),
                None,
                first as u32,
                off as u32,
                data,
                data.len() as u32,
                false,
            );
            for i in first..=last {
                if i != 8 {
                    if i == first && off > 2 {
                        // Not updating timbre, since those values weren't touched
                    } else {
                        part::reset_timbre(state, i);
                    }
                }
                part::refresh(state, i);
            }
        }
        RhythmTemp => {
            descriptor.write(
                Some(&mut state.mt32_ram.raw[MemParams::RHYTHM_TEMP_OFFSET..]),
                None,
                first as u32,
                off as u32,
                data,
                data.len() as u32,
                false,
            );
            part::refresh(state, 8);
        }
        TimbreTemp => {
            descriptor.write(
                Some(&mut state.mt32_ram.raw[MemParams::TIMBRE_TEMP_OFFSET..]),
                None,
                first as u32,
                off as u32,
                data,
                data.len() as u32,
                false,
            );
            for i in first..=last {
                part::refresh(state, i);
            }
        }
        Patches => {
            descriptor.write(
                Some(&mut state.mt32_ram.raw[MemParams::PATCHES_OFFSET..]),
                None,
                first as u32,
                off as u32,
                data,
                data.len() as u32,
                false,
            );
        }
        Timbres => {
            let timbre_first = first + 128;
            let timbre_last = last + 128;
            descriptor.write(
                Some(&mut state.mt32_ram.raw[MemParams::TIMBRES_OFFSET..]),
                None,
                timbre_first as u32,
                off as u32,
                data,
                data.len() as u32,
                false,
            );
            for timbre_idx in timbre_first..=timbre_last {
                for part_idx in 0..9 {
                    part::refresh_timbre(state, part_idx, timbre_idx as u32);
                }
            }
        }
        System => {
            descriptor.write(
                Some(&mut state.mt32_ram.raw[MemParams::SYSTEM_OFFSET..]),
                None,
                0,
                off as u32,
                data,
                data.len() as u32,
                false,
            );
            let len = data.len();
            if off == SYSTEM_MASTER_TUNE_OFF && off + len > SYSTEM_MASTER_TUNE_OFF {
                refresh_system_master_tune(state);
            }
            if off <= SYSTEM_REVERB_LEVEL_OFF && off + len > SYSTEM_REVERB_MODE_OFF {
                refresh_system_reverb_parameters(state);
            }
            if off <= SYSTEM_RESERVE_SETTINGS_END_OFF
                && off + len > SYSTEM_RESERVE_SETTINGS_START_OFF
            {
                refresh_system_reserve_settings(state);
            }
            if off <= SYSTEM_CHAN_ASSIGN_END_OFF && off + len > SYSTEM_CHAN_ASSIGN_START_OFF {
                let first_part = off.saturating_sub(SYSTEM_CHAN_ASSIGN_START_OFF);
                let last_part = (off + len - SYSTEM_CHAN_ASSIGN_START_OFF).min(8);
                refresh_system_chan_assign(state, first_part as u8, last_part as u8);
            }
            if off <= SYSTEM_MASTER_VOL_OFF && off + len > SYSTEM_MASTER_VOL_OFF {
                if state.extensions.master_volume_override <= DEFAULT_MASTER_VOLUME {
                    let system_base = MemParams::SYSTEM_OFFSET;
                    state.mt32_ram.raw[system_base + SYSTEM_MASTER_VOL_OFF] =
                        state.extensions.master_volume_override;
                } else {
                    refresh_system_master_vol(state);
                }
            }
        }
        Reset => {
            reset(state);
        }
    }
}

fn refresh_system_master_tune(state: &mut MuntState) {
    // 171 is ~half a semitone.
    let system_base = MemParams::SYSTEM_OFFSET;
    let master_tune = state.mt32_ram.raw[system_base + SYSTEM_MASTER_TUNE_OFF] as i32;
    // PORTABILITY NOTE: Assumes arithmetic shift.
    state.extensions.master_tune_pitch_delta = ((master_tune - 64) * 171) >> 6;
}

fn refresh_system_reverb_parameters(state: &mut MuntState) {
    if state.reverb_overridden {
        return;
    }
    let system_base = MemParams::SYSTEM_OFFSET;
    let reverb_mode = state.mt32_ram.raw[system_base + SYSTEM_REVERB_MODE_OFF];
    let reverb_time = state.mt32_ram.raw[system_base + SYSTEM_REVERB_TIME_OFF];
    let reverb_level = state.mt32_ram.raw[system_base + SYSTEM_REVERB_LEVEL_OFF];

    let new_model_index = reverb_mode as usize;
    if reverb_time == 0 && reverb_level == 0 {
        // Setting both time and level to 0 effectively disables wet reverb output on real devices.
        // Use a sentinel to indicate "no reverb".
        state.active_reverb_model = 4; // out-of-range = disabled
    } else {
        let old_model = state.active_reverb_model;
        state.active_reverb_model = new_model_index;
        if old_model != new_model_index {
            state.reverb_models[new_model_index].mute();
        }
    }

    if state.active_reverb_model < 4 {
        let idx = state.active_reverb_model;
        state.reverb_models[idx].open();
        state.reverb_models[idx].set_parameters(reverb_time, reverb_level);
    }
}

fn refresh_system_reserve_settings(state: &mut MuntState) {
    let system_base = MemParams::SYSTEM_OFFSET;
    let mut rset = [0u8; 9];
    rset.copy_from_slice(
        &state.mt32_ram.raw[system_base + SYSTEM_RESERVE_SETTINGS_START_OFF
            ..system_base + SYSTEM_RESERVE_SETTINGS_START_OFF + 9],
    );
    partial_manager::set_reserve(state, &rset);
}

fn refresh_system_chan_assign(state: &mut MuntState, first_part: u8, last_part: u8) {
    state.extensions.chan_table = [[0xFF; 9]; 16];

    // CONFIRMED: In the case of assigning a MIDI channel to multiple parts,
    //            the messages received on that MIDI channel are handled by all the parts.
    let system_base = MemParams::SYSTEM_OFFSET;
    for i in 0..=8u8 {
        if i >= first_part && i <= last_part {
            part::all_sound_off(state, i as usize);
            part::reset_all_controllers(state, i as usize);
        }
        let chan = state.mt32_ram.raw[system_base + SYSTEM_CHAN_ASSIGN_START_OFF + i as usize];
        if chan > 15 {
            continue;
        }
        let chan_parts = &mut state.extensions.chan_table[chan as usize];
        for chan_part in chan_parts.iter_mut() {
            if *chan_part > 8 {
                *chan_part = i;
                break;
            }
        }
    }
}

fn refresh_system_master_vol(_state: &mut MuntState) {
    // No-op in this port (the value is read directly from mt32_ram when needed)
}

fn refresh_system(state: &mut MuntState) {
    refresh_system_master_tune(state);
    refresh_system_reverb_parameters(state);
    refresh_system_reserve_settings(state);
    refresh_system_chan_assign(state, 0, 8);
    refresh_system_master_vol(state);
}

fn reset(state: &mut MuntState) {
    if !state.opened {
        return;
    }
    partial_manager::deactivate_all(state);
    state.mt32_ram = state.mt32_default.clone();
    for i in 0..9 {
        part::reset(state, i);
        if i != 8 {
            let rom_map = state.rom.control_rom_map.unwrap();
            let prog_addr = rom_map.program_settings as usize + i;
            if prog_addr < CONTROL_ROM_SIZE {
                let program = state.rom.control_rom_data[prog_addr];
                part::set_program(state, i, program as u32);
            }
        } else {
            part::refresh(state, 8);
        }
    }
    if state.extensions.master_volume_override < DEFAULT_MASTER_VOLUME {
        let system_base = MemParams::SYSTEM_OFFSET;
        state.mt32_ram.raw[system_base + SYSTEM_MASTER_VOL_OFF] =
            state.extensions.master_volume_override;
    }
    refresh_system(state);
    reset_master_tune_pitch_delta(state);
    is_active(state);
}

fn has_active_partials(state: &MuntState) -> bool {
    if !state.opened {
        return false;
    }
    for partial_num in 0..state.partial_count as usize {
        if partial::is_active(state, partial_num) {
            return true;
        }
    }
    false
}

fn is_active(state: &mut MuntState) -> bool {
    if !state.opened {
        return false;
    }
    if !state.midi_queue.is_empty() || has_active_partials(state) {
        return true;
    }
    if state.active_reverb_model < 4 && state.reverb_models[state.active_reverb_model].is_active() {
        return true;
    }
    state.activated = false;
    false
}

fn reset_master_tune_pitch_delta(state: &mut MuntState) {
    // This effectively resets master tune to 440.0Hz.
    // Despite that the manual claims 442.0Hz is the default setting for master tune,
    // it doesn't actually take effect upon a reset due to a bug in the reset routine.
    // CONFIRMED: This bug is present in all supported Control ROMs.
    state.extensions.master_tune_pitch_delta = 0;
}

pub(crate) fn render(state: &mut MuntState, stereo_stream: &mut [f32], len: u32) {
    if !state.opened {
        let count = (len as usize * 2).min(stereo_stream.len());
        stereo_stream[..count].fill(0.0);
        return;
    }
    do_render(state, stereo_stream, len);
}

fn do_render(state: &mut MuntState, stereo_stream: &mut [f32], len: u32) {
    if !state.activated {
        let dac_len = len; // In coarse mode, DAC streams length == output length
        state.rendered_sample_count = state.rendered_sample_count.wrapping_add(dac_len);
        let count = (len as usize * 2).min(stereo_stream.len());
        stereo_stream[..count].fill(0.0);
        return;
    }

    let mut remaining = len;
    let mut stream_offset = 0usize;

    while remaining > 0 {
        let this_pass_len = remaining.min(MAX_SAMPLES_PER_RUN as u32);
        let dac_len = this_pass_len; // Coarse mode: 1:1

        do_render_streams(state, dac_len);

        // Run analog processing to mix down to stereo
        let out_slice =
            &mut stereo_stream[stream_offset..stream_offset + this_pass_len as usize * 2];
        state.analog.process(
            out_slice,
            &state.renderer.tmp_non_reverb_left[..dac_len as usize],
            &state.renderer.tmp_non_reverb_right[..dac_len as usize],
            &state.renderer.tmp_reverb_dry_left[..dac_len as usize],
            &state.renderer.tmp_reverb_dry_right[..dac_len as usize],
            &state.renderer.tmp_reverb_wet_left[..dac_len as usize],
            &state.renderer.tmp_reverb_wet_right[..dac_len as usize],
            this_pass_len,
        );

        stream_offset += this_pass_len as usize * 2;
        remaining -= this_pass_len;
    }
}

fn do_render_streams(state: &mut MuntState, len: u32) {
    let mut remaining = len;
    let mut stream_offset: usize = 0;
    while remaining > 0 {
        // We need to ensure zero-duration notes will play so add minimum 1-sample delay.
        let mut this_len = 1u32;
        if state.aborting_poly_index.is_none() {
            let samples_to_next_event = state
                .midi_queue
                .peek()
                .map_or(MAX_SAMPLES_PER_RUN as i32, |e| {
                    e.timestamp as i32 - state.rendered_sample_count as i32
                });

            if samples_to_next_event > 0 {
                this_len = remaining.min(MAX_SAMPLES_PER_RUN as u32);
                if this_len > samples_to_next_event as u32 {
                    this_len = samples_to_next_event as u32;
                }
            } else {
                let event_index = state.midi_queue.start_position as usize;
                let is_sysex = state.midi_queue.ring_buffer[event_index].is_sysex;
                let short_msg = state.midi_queue.ring_buffer[event_index].short_message_data;
                if is_sysex {
                    let mut scratch = state
                        .midi_sysex_scratch
                        .take()
                        .expect("MT-32 SysEx scratch must be available between events");
                    let sysex_length = state.midi_queue.copy_front_sysex(&mut scratch).unwrap_or(0);
                    play_sysex_now(state, &scratch[..sysex_length]);
                    state.midi_queue.drop_front();
                    state.midi_sysex_scratch = Some(scratch);
                } else {
                    play_msg_now(state, short_msg);
                    // If a poly is aborting we don't drop the event from the queue.
                    if state.aborting_poly_index.is_none() {
                        state.midi_queue.drop_front();
                    }
                }
            }
        }
        produce_streams(state, stream_offset, this_len);
        stream_offset += this_len as usize;
        remaining -= this_len;
    }
}

fn produce_streams(state: &mut MuntState, offset: usize, len: u32) {
    let len_usize = len as usize;
    let end = offset + len_usize;

    if state.activated {
        // Clear temp buffers at the correct offset
        state.renderer.tmp_non_reverb_left[offset..end].fill(0.0);
        state.renderer.tmp_non_reverb_right[offset..end].fill(0.0);
        state.renderer.tmp_reverb_dry_left[offset..end].fill(0.0);
        state.renderer.tmp_reverb_dry_right[offset..end].fill(0.0);

        // Render partials
        let partial_count = state.partial_count as usize;
        for i in 0..partial_count {
            let use_reverb = partial_manager::should_reverb(state, i);
            state.renderer.tmp_partial_left[..len_usize].fill(0.0);
            state.renderer.tmp_partial_right[..len_usize].fill(0.0);
            if partial_manager::produce_output(state, i, len) {
                if use_reverb {
                    for j in 0..len_usize {
                        state.renderer.tmp_reverb_dry_left[offset + j] +=
                            state.renderer.tmp_partial_left[j];
                        state.renderer.tmp_reverb_dry_right[offset + j] +=
                            state.renderer.tmp_partial_right[j];
                    }
                } else {
                    for j in 0..len_usize {
                        state.renderer.tmp_non_reverb_left[offset + j] +=
                            state.renderer.tmp_partial_left[j];
                        state.renderer.tmp_non_reverb_right[offset + j] +=
                            state.renderer.tmp_partial_right[j];
                    }
                }
            }
        }

        // Apply DAC input mode processing to reverb dry
        apply_la32_output(
            &mut state.renderer.tmp_reverb_dry_left[offset..end],
            state.dac_input_mode,
        );
        apply_la32_output(
            &mut state.renderer.tmp_reverb_dry_right[offset..end],
            state.dac_input_mode,
        );

        // Process reverb
        let reverb_enabled = state.active_reverb_model < 4;
        if reverb_enabled {
            let reverb_model = &mut state.reverb_models[state.active_reverb_model];
            let renderer = &mut state.renderer;
            renderer.tmp_reverb_wet_left[offset..end].fill(0.0);
            renderer.tmp_reverb_wet_right[offset..end].fill(0.0);
            reverb_model.process(
                &renderer.tmp_reverb_dry_left[offset..end],
                &renderer.tmp_reverb_dry_right[offset..end],
                &mut renderer.tmp_reverb_wet_left[offset..end],
                &mut renderer.tmp_reverb_wet_right[offset..end],
                len,
            );
        } else {
            state.renderer.tmp_reverb_wet_left[offset..end].fill(0.0);
            state.renderer.tmp_reverb_wet_right[offset..end].fill(0.0);
        }

        // Apply DAC input mode processing to non-reverb
        apply_la32_output(
            &mut state.renderer.tmp_non_reverb_left[offset..end],
            state.dac_input_mode,
        );
        apply_la32_output(
            &mut state.renderer.tmp_non_reverb_right[offset..end],
            state.dac_input_mode,
        );
    } else {
        state.renderer.tmp_non_reverb_left[offset..end].fill(0.0);
        state.renderer.tmp_non_reverb_right[offset..end].fill(0.0);
        state.renderer.tmp_reverb_dry_left[offset..end].fill(0.0);
        state.renderer.tmp_reverb_dry_right[offset..end].fill(0.0);
        state.renderer.tmp_reverb_wet_left[offset..end].fill(0.0);
        state.renderer.tmp_reverb_wet_right[offset..end].fill(0.0);
    }

    partial_manager::clear_already_outputed(state);
    state.rendered_sample_count = state.rendered_sample_count.wrapping_add(len);
}

fn apply_la32_output(buffer: &mut [f32], dac_input_mode: DacInputMode) {
    if dac_input_mode == DacInputMode::Nice {
        // Note, we do not do any clamping for floats here to avoid introducing distortions.
        for sample in buffer.iter_mut() {
            *sample *= 2.0;
        }
    }
}
