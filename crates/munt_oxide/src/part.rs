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

// In the C++ code, Part is a base class and RhythmPart is a derived class.
// In Rust, we use a single PartState with an `is_rhythm` flag and dispatch
// "virtual" methods via if/else on that flag.

use crate::{
    partial_manager, poly,
    state::{DRUM_CACHE_COUNT, MuntState},
    structures::{PatchCache, TimbreParam},
};

static PARTIAL_STRUCT: [u8; 13] = [0, 0, 2, 2, 1, 3, 3, 0, 3, 0, 2, 1, 3];

static PARTIAL_MIX_STRUCT: [u8; 13] = [0, 1, 0, 1, 1, 0, 1, 3, 3, 2, 2, 2, 2];

// Identifies which PatchCache array to operate on.
enum CacheSource {
    Part,
    Drum(usize),
}

pub(crate) fn init_part(state: &mut MuntState, part_index: usize) {
    let part_num = part_index as u32;
    let part = &mut state.parts[part_index];
    part.part_num = part_num;
    part.hold_pedal = false;

    if part_index == 8 {
        // The Rhythm part doesn't have just a single timbre associated.
        part.is_rhythm = true;
        let name = b"Rhythm\0\0";
        part.name[..8].copy_from_slice(name);
    } else {
        part.is_rhythm = false;
        // "Part N" where N = partNum + 1
        let label = format!("Part {}", part_num + 1);
        let bytes = label.as_bytes();
        let len = bytes.len().min(7);
        part.name[..len].copy_from_slice(&bytes[..len]);
        part.name[len] = 0;
    }

    part.current_instr[0] = 0;
    part.current_instr[10] = 0;
    part.volume_override = 255;
    part.modulation = 0;
    part.expression = 100;
    part.pitch_bend = 0;
    part.active_partial_count = 0;
    part.active_non_releasing_poly_count = 0;
    part.patch_cache = core::array::from_fn(|_| PatchCache::default());

    if part.is_rhythm {
        part.drum_cache = vec![core::array::from_fn(|_| PatchCache::default()); DRUM_CACHE_COUNT];
    }
}

pub(crate) fn init_rhythm_part(state: &mut MuntState, part_index: usize) {
    init_part(state, part_index);
    refresh(state, part_index);
}

pub(crate) fn set_data_entry_msb(
    state: &mut MuntState,
    part_index: usize,
    midi_data_entry_msb: u8,
) {
    if state.parts[part_index].nrpn {
        // The last RPN-related control change was for an NRPN,
        // which the real synths don't support.
        return;
    }
    if state.parts[part_index].rpn != 0 {
        // The RPN has been set to something other than 0,
        // which is the only RPN that these synths support
        return;
    }
    let mut pt = state.mt32_ram.patch_temp(part_index);
    pt.patch.bender_range = if midi_data_entry_msb > 24 {
        24
    } else {
        midi_data_entry_msb
    };
    state.mt32_ram.set_patch_temp(part_index, &pt);
    update_pitch_bender_range(state, part_index);
}

pub(crate) fn set_nrpn(state: &mut MuntState, part_index: usize) {
    state.parts[part_index].nrpn = true;
}

pub(crate) fn set_rpn_lsb(state: &mut MuntState, part_index: usize, midi_rpn_lsb: u8) {
    state.parts[part_index].nrpn = false;
    state.parts[part_index].rpn = (state.parts[part_index].rpn & 0xFF00) | midi_rpn_lsb as u16;
}

pub(crate) fn set_rpn_msb(state: &mut MuntState, part_index: usize, midi_rpn_msb: u8) {
    state.parts[part_index].nrpn = false;
    state.parts[part_index].rpn =
        (state.parts[part_index].rpn & 0x00FF) | ((midi_rpn_msb as u16) << 8);
}

pub(crate) fn set_hold_pedal(state: &mut MuntState, part_index: usize, pressed: bool) {
    if state.parts[part_index].hold_pedal && !pressed {
        state.parts[part_index].hold_pedal = false;
        stop_pedal_hold(state, part_index);
    } else {
        state.parts[part_index].hold_pedal = pressed;
    }
}

pub(crate) fn set_bend(state: &mut MuntState, part_index: usize, midi_bend: u32) {
    // CONFIRMED:
    let pitch_bender_range = state.parts[part_index].pitch_bender_range;
    // PORTABILITY NOTE: Assumes arithmetic shift
    state.parts[part_index].pitch_bend =
        ((midi_bend as i32 - 8192) * pitch_bender_range as i32) >> 14;
}

pub(crate) fn set_modulation(state: &mut MuntState, part_index: usize, midi_modulation: u32) {
    state.parts[part_index].modulation = midi_modulation as u8;
}

pub(crate) fn reset_all_controllers(state: &mut MuntState, part_index: usize) {
    state.parts[part_index].modulation = 0;
    state.parts[part_index].expression = 100;
    state.parts[part_index].pitch_bend = 0;
    set_hold_pedal(state, part_index, false);
}

pub(crate) fn reset(state: &mut MuntState, part_index: usize) {
    reset_all_controllers(state, part_index);
    all_sound_off(state, part_index);
    state.parts[part_index].rpn = 0xFFFF;
}

/// Dispatches to rhythm or melodic refresh.
pub(crate) fn refresh(state: &mut MuntState, part_index: usize) {
    if state.parts[part_index].is_rhythm {
        rhythm_refresh(state, part_index);
    } else {
        melodic_refresh(state, part_index);
    }
}

fn rhythm_refresh(state: &mut MuntState, part_index: usize) {
    // (Re-)cache all the mapped timbres ahead of time
    let rhythm_settings_count = state
        .rom
        .control_rom_map
        .as_ref()
        .map_or(0, |m| m.rhythm_settings_count as u32);
    for drum_num in 0..rhythm_settings_count as usize {
        let drum_timbre_num = state.mt32_ram.rhythm_temp(drum_num).timbre;
        if drum_timbre_num >= 127 {
            // 94 on MT-32
            continue;
        }
        backup_cache_to_partials(state, part_index, &CacheSource::Drum(drum_num));
        for t in 0..4 {
            // Common parameters, stored redundantly
            state.parts[part_index].drum_cache[drum_num][t].dirty = true;
            state.parts[part_index].drum_cache[drum_num][t].reverb =
                state.mt32_ram.rhythm_temp(drum_num).reverb_switch > 0;
        }
    }
    update_pitch_bender_range(state, part_index);
}

fn melodic_refresh(state: &mut MuntState, part_index: usize) {
    backup_cache_to_partials(state, part_index, &CacheSource::Part);
    for t in 0..4 {
        // Common parameters, stored redundantly
        state.parts[part_index].patch_cache[t].dirty = true;
        state.parts[part_index].patch_cache[t].reverb =
            state.mt32_ram.patch_temp(part_index).patch.reverb_switch > 0;
    }
    let mut name_buf = [0u8; 10];
    let tt = state.mt32_ram.timbre_temp(part_index);
    name_buf.copy_from_slice(&tt.common.name);
    state.parts[part_index].current_instr[..10].copy_from_slice(&name_buf);
    // synth->newTimbreSet(partNum) - notification only, no-op in our port
    update_pitch_bender_range(state, part_index);
}

/// Dispatches to rhythm or melodic refreshTimbre.
pub(crate) fn refresh_timbre(state: &mut MuntState, part_index: usize, abs_timbre_num: u32) {
    if state.parts[part_index].is_rhythm {
        rhythm_refresh_timbre(state, part_index, abs_timbre_num);
    } else {
        melodic_refresh_timbre(state, part_index, abs_timbre_num);
    }
}

fn rhythm_refresh_timbre(state: &mut MuntState, part_index: usize, abs_timbre_num: u32) {
    for m in 0..85 {
        if state.mt32_ram.rhythm_temp(m).timbre as u32 == abs_timbre_num.wrapping_sub(128) {
            state.parts[part_index].drum_cache[m][0].dirty = true;
        }
    }
}

fn melodic_refresh_timbre(state: &mut MuntState, part_index: usize, abs_timbre_num: u32) {
    if get_abs_timbre_num(state, part_index) == abs_timbre_num {
        let mut name_buf = [0u8; 10];
        let tt = state.mt32_ram.timbre_temp(part_index);
        name_buf.copy_from_slice(&tt.common.name);
        state.parts[part_index].current_instr[..10].copy_from_slice(&name_buf);
        state.parts[part_index].patch_cache[0].dirty = true;
    }
}

fn set_patch(state: &mut MuntState, part_index: usize, patch_num: usize) {
    let patch = state.mt32_ram.patch(patch_num);
    let mut pt = state.mt32_ram.patch_temp(part_index);
    pt.patch = patch;
    state.mt32_ram.set_patch_temp(part_index, &pt);
}

/// Dispatches to rhythm or melodic resetTimbre.
pub(crate) fn reset_timbre(state: &mut MuntState, part_index: usize) {
    if state.parts[part_index].is_rhythm {
        // rhythm_reset_timbre: doesn't make sense for rhythm
        return;
    }
    melodic_reset_timbre(state, part_index);
}

fn melodic_reset_timbre(state: &mut MuntState, part_index: usize) {
    state.parts[part_index].hold_pedal = false;
    all_sound_off(state, part_index);
    let abs_timbre_num = get_abs_timbre_num(state, part_index) as usize;
    let timbre = state.mt32_ram.timbre(abs_timbre_num).timbre;
    state.mt32_ram.set_timbre_temp(part_index, &timbre);
}

/// Dispatches to rhythm or melodic getAbsTimbreNum.
pub(crate) fn get_abs_timbre_num(state: &MuntState, part_index: usize) -> u32 {
    if state.parts[part_index].is_rhythm {
        // Doesn't make sense for rhythm
        0
    } else {
        let pt = state.mt32_ram.patch_temp(part_index);
        (pt.patch.timbre_group as u32 * 64) + pt.patch.timbre_num as u32
    }
}

/// Dispatches to rhythm or melodic setProgram.
pub(crate) fn set_program(state: &mut MuntState, part_index: usize, midi_program: u32) {
    if state.parts[part_index].is_rhythm {
        // Attempt to set program on rhythm is invalid - no-op
        return;
    }
    melodic_set_program(state, part_index, midi_program);
}

fn melodic_set_program(state: &mut MuntState, part_index: usize, patch_num: u32) {
    set_patch(state, part_index, patch_num as usize);
    reset_timbre(state, part_index);
    refresh(state, part_index);
}

pub(crate) fn update_pitch_bender_range(state: &mut MuntState, part_index: usize) {
    state.parts[part_index].pitch_bender_range =
        state.mt32_ram.patch_temp(part_index).patch.bender_range as u16 * 683;
}

fn backup_cache_to_partials(state: &mut MuntState, part_index: usize, source: &CacheSource) {
    // check if any partials are still playing with the old patch cache
    // if so then duplicate the cached data from the part to the partial so that
    // we can change the part's cache without affecting the partial.
    // We delay this until now to avoid a copy operation with every note played
    let mut poly_index = state.parts[part_index].active_polys_first;
    while let Some(pi) = poly_index {
        let next = state.polys[pi].next_index;
        let cache = get_cache_array(state, part_index, source);
        poly::poly_backup_cache_to_partials(state, pi, &cache);
        poly_index = next;
    }
}

fn get_cache_array(state: &MuntState, part_index: usize, source: &CacheSource) -> [PatchCache; 4] {
    match source {
        CacheSource::Part => state.parts[part_index].patch_cache.clone(),
        CacheSource::Drum(drum_num) => state.parts[part_index].drum_cache[*drum_num].clone(),
    }
}

fn get_cache_ref<'a>(
    state: &'a MuntState,
    part_index: usize,
    source: &CacheSource,
    t: usize,
) -> &'a PatchCache {
    match source {
        CacheSource::Part => &state.parts[part_index].patch_cache[t],
        CacheSource::Drum(drum_num) => &state.parts[part_index].drum_cache[*drum_num][t],
    }
}

fn cache_timbre(
    state: &mut MuntState,
    part_index: usize,
    source: &CacheSource,
    timbre: &TimbreParam,
) {
    backup_cache_to_partials(state, part_index, source);

    let timbre_common_partial_mute = timbre.common.partial_mute;
    let timbre_common_partial_structure12 = timbre.common.partial_structure12;
    let timbre_common_partial_structure34 = timbre.common.partial_structure34;
    let timbre_common_no_sustain = timbre.common.no_sustain;

    let mut partial_count: u32 = 0;
    for t in 0..4usize {
        let play_partial = ((timbre_common_partial_mute >> t) & 0x1) == 1;

        // Calculate and cache common parameters
        let src_partial = timbre.partial[t].clone();
        let pcm = timbre.partial[t].wg.pcm_wave as i32;
        let waveform = timbre.partial[t].wg.waveform;

        let cache = match source {
            CacheSource::Part => &mut state.parts[part_index].patch_cache[t],
            CacheSource::Drum(drum_num) => &mut state.parts[part_index].drum_cache[*drum_num][t],
        };

        if play_partial {
            cache.play_partial = true;
            partial_count += 1;
        } else {
            cache.play_partial = false;
            continue;
        }

        cache.src_partial = src_partial;
        cache.pcm = pcm;

        match t {
            0 => {
                cache.pcm_partial =
                    (PARTIAL_STRUCT[timbre_common_partial_structure12 as usize] & 0x2) != 0;
                cache.structure_mix =
                    PARTIAL_MIX_STRUCT[timbre_common_partial_structure12 as usize] as u32;
                cache.structure_position = 0;
                cache.structure_pair = 1;
            }
            1 => {
                cache.pcm_partial =
                    (PARTIAL_STRUCT[timbre_common_partial_structure12 as usize] & 0x1) != 0;
                cache.structure_mix =
                    PARTIAL_MIX_STRUCT[timbre_common_partial_structure12 as usize] as u32;
                cache.structure_position = 1;
                cache.structure_pair = 0;
            }
            2 => {
                cache.pcm_partial =
                    (PARTIAL_STRUCT[timbre_common_partial_structure34 as usize] & 0x2) != 0;
                cache.structure_mix =
                    PARTIAL_MIX_STRUCT[timbre_common_partial_structure34 as usize] as u32;
                cache.structure_position = 0;
                cache.structure_pair = 3;
            }
            3 => {
                cache.pcm_partial =
                    (PARTIAL_STRUCT[timbre_common_partial_structure34 as usize] & 0x1) != 0;
                cache.structure_mix =
                    PARTIAL_MIX_STRUCT[timbre_common_partial_structure34 as usize] as u32;
                cache.structure_position = 1;
                cache.structure_pair = 2;
            }
            _ => {}
        }

        cache.waveform = waveform;
    }
    let sustain = timbre_common_no_sustain == 0;
    for t in 0..4usize {
        // Common parameters, stored redundantly
        let cache = match source {
            CacheSource::Part => &mut state.parts[part_index].patch_cache[t],
            CacheSource::Drum(drum_num) => &mut state.parts[part_index].drum_cache[*drum_num][t],
        };
        cache.dirty = false;
        cache.partial_count = partial_count;
        cache.sustain = sustain;
    }
}

pub(crate) fn set_volume(state: &mut MuntState, part_index: usize, midi_volume: u32) {
    // CONFIRMED: This calculation matches the table used in the control ROM
    let mut pt = state.mt32_ram.patch_temp(part_index);
    pt.output_level = (midi_volume * 100 / 127) as u8;
    state.mt32_ram.set_patch_temp(part_index, &pt);
}

pub(crate) fn get_volume_override(state: &MuntState, part_index: usize) -> u8 {
    state.parts[part_index].volume_override
}

pub(crate) fn set_expression(state: &mut MuntState, part_index: usize, midi_expression: u32) {
    // CONFIRMED: This calculation matches the table used in the control ROM
    state.parts[part_index].expression = (midi_expression * 100 / 127) as u8;
}

/// Dispatches to rhythm or melodic setPan.
pub(crate) fn set_pan(state: &mut MuntState, part_index: usize, midi_pan: u32) {
    if state.parts[part_index].is_rhythm {
        // CONFIRMED: This does change patchTemp, but has no actual effect on playback.
    }
    set_pan_inner(state, part_index, midi_pan);
}

fn set_pan_inner(state: &mut MuntState, part_index: usize, midi_pan: u32) {
    // NOTE: Panning is inverted compared to GM.
    let quirk_pan_mult = state.rom.control_rom_features.quirk_pan_mult;

    let mut pt = state.mt32_ram.patch_temp(part_index);
    if quirk_pan_mult {
        // MT-32: Divide by 9
        pt.panpot = (midi_pan / 9) as u8;
    } else {
        // CM-32L: Divide by 8.5
        pt.panpot = ((midi_pan << 3) / 68) as u8;
    }
    state.mt32_ram.set_patch_temp(part_index, &pt);
}

/// Applies key shift to a MIDI key and converts it into an internal key value in the range 12-108.
fn midi_key_to_key(state: &MuntState, part_index: usize, midi_key: u32) -> u32 {
    let quirk_key_shift = state.rom.control_rom_features.quirk_key_shift;

    if quirk_key_shift {
        // NOTE: On MT-32 GEN0, key isn't adjusted, and keyShift is applied further in TVP, unlike newer units:
        return midi_key;
    }
    let mut key = midi_key as i32 + state.mt32_ram.patch_temp(part_index).patch.key_shift as i32;
    if key < 36 {
        // After keyShift is applied, key < 36, so move up by octaves
        while key < 36 {
            key += 12;
        }
    } else if key > 132 {
        // After keyShift is applied, key > 132, so move down by octaves
        while key > 132 {
            key -= 12;
        }
    }
    key -= 24;
    key as u32
}

/// Dispatches to rhythm or melodic noteOn.
pub(crate) fn note_on(state: &mut MuntState, part_index: usize, midi_key: u32, velocity: u32) {
    if state.parts[part_index].is_rhythm {
        rhythm_note_on(state, part_index, midi_key, velocity);
    } else {
        melodic_note_on(state, part_index, midi_key, velocity);
    }
}

fn rhythm_note_on(state: &mut MuntState, part_index: usize, midi_key: u32, velocity: u32) {
    if !(24..=108).contains(&midi_key) {
        // > 87 on MT-32
        return;
    }
    // synth->rhythmNotePlayed() - notification only, no-op in our port

    let drum_num = (midi_key - 24) as usize;
    let drum_timbre_num = state.mt32_ram.rhythm_temp(drum_num).timbre as i32;
    let timbre_r_count = state
        .rom
        .control_rom_map
        .as_ref()
        .map_or(0, |m| m.timbre_r_count as i32);
    let drum_timbre_count = 64 + timbre_r_count; // 94 on MT-32, 128 on LAPC-I/CM32-L
    if drum_timbre_num == 127 || drum_timbre_num >= drum_timbre_count {
        // timbre #127 is OFF, no sense to play it
        return;
    }
    // CONFIRMED: Two special cases described by Mok
    let key = if drum_timbre_num == 64 + 6 {
        note_off(state, part_index, 0);
        1
    } else if drum_timbre_num == 64 + 7 {
        // This noteOff(0) is not performed on MT-32, only LAPC-I
        note_off(state, part_index, 0);
        0
    } else {
        midi_key
    };
    let abs_timbre_num = (drum_timbre_num + 128) as usize;

    let mut name_buf = [0u8; 10];
    name_buf.copy_from_slice(&state.mt32_ram.timbre(abs_timbre_num).timbre.common.name);
    state.parts[part_index].current_instr[..10].copy_from_slice(&name_buf);

    if state.parts[part_index].drum_cache[drum_num][0].dirty {
        let timbre = state.mt32_ram.timbre(abs_timbre_num).timbre;
        cache_timbre(state, part_index, &CacheSource::Drum(drum_num), &timbre);
    }
    play_poly(
        state,
        part_index,
        &CacheSource::Drum(drum_num),
        Some(drum_num),
        midi_key,
        key,
        velocity,
    );
}

fn melodic_note_on(state: &mut MuntState, part_index: usize, midi_key: u32, velocity: u32) {
    let key = midi_key_to_key(state, part_index, midi_key);
    if state.parts[part_index].patch_cache[0].dirty {
        let timbre = state.mt32_ram.timbre_temp(part_index);
        cache_timbre(state, part_index, &CacheSource::Part, &timbre);
    }
    play_poly(
        state,
        part_index,
        &CacheSource::Part,
        None,
        midi_key,
        key,
        velocity,
    );
}

fn abort_first_poly_by_key(state: &mut MuntState, part_index: usize, key: u32) -> bool {
    let mut poly_index = state.parts[part_index].active_polys_first;
    while let Some(pi) = poly_index {
        if state.polys[pi].key == key {
            return poly::poly_start_abort(state, pi);
        }
        poly_index = state.polys[pi].next_index;
    }
    false
}

fn play_poly(
    state: &mut MuntState,
    part_index: usize,
    source: &CacheSource,
    rhythm_drum_num: Option<usize>,
    _midi_key: u32,
    key: u32,
    velocity: u32,
) {
    // CONFIRMED: Even in single-assign mode, we don't abort playing polys if the timbre to play is completely muted.
    let need_partials = get_cache_ref(state, part_index, source, 0).partial_count;
    if need_partials == 0 {
        return;
    }

    if (state.mt32_ram.patch_temp(part_index).patch.assign_mode & 2) == 0 {
        // Single-assign mode
        abort_first_poly_by_key(state, part_index, key);
        if state.aborting_poly_index.is_some() {
            return;
        }
    }

    if !partial_manager::free_partials(state, need_partials, part_index as i32) {
        return;
    }
    if state.aborting_poly_index.is_some() {
        return;
    }

    let poly_index = match partial_manager::assign_poly_to_part(state, part_index) {
        Some(pi) => pi,
        None => return,
    };

    if (state.mt32_ram.patch_temp(part_index).patch.assign_mode & 1) != 0 {
        // Priority to data first received
        poly_list_prepend(state, part_index, poly_index);
    } else {
        poly_list_append(state, part_index, poly_index);
    }

    let sustain = get_cache_ref(state, part_index, source, 0).sustain;

    let mut partial_indices: [Option<usize>; 4] = [None; 4];
    for (x, partial_index_slot) in partial_indices.iter_mut().enumerate() {
        if get_cache_ref(state, part_index, source, x).play_partial {
            let partial_idx = partial_manager::alloc_partial(state, part_index as i32);
            if let Some(idx) = partial_idx {
                *partial_index_slot = Some(idx);
                state.parts[part_index].active_partial_count += 1;
            }
        }
    }

    poly::poly_reset(state, poly_index, key, velocity, sustain, partial_indices);

    // Start each partial
    for x in 0..4 {
        if let Some(partial_idx) = partial_indices[x] {
            let pair_idx = {
                let pair_t = get_cache_ref(state, part_index, source, x).structure_pair as usize;
                partial_indices[pair_t]
            };
            // Copy cache for this partial
            let cache_copy = get_cache_ref(state, part_index, source, x).clone();
            crate::partial::start_partial(
                state,
                partial_idx,
                part_index,
                poly_index,
                &cache_copy,
                rhythm_drum_num,
                pair_idx,
            );
        }
    }
    // synth->reportHandler->onPolyStateChanged(partNum) - notification only
}

pub(crate) fn all_notes_off(state: &mut MuntState, part_index: usize) {
    // The MIDI specification states - and Mok confirms - that all notes off (0x7B)
    // should treat the hold pedal as usual.
    let hold_pedal = state.parts[part_index].hold_pedal;
    let mut poly_index = state.parts[part_index].active_polys_first;
    while let Some(pi) = poly_index {
        let next = state.polys[pi].next_index;
        // FIXME: The real devices are found to be ignoring non-sustaining polys while processing AllNotesOff. Need to be confirmed.
        if state.polys[pi].sustain {
            poly::poly_note_off(state, pi, hold_pedal);
        }
        poly_index = next;
    }
}

pub(crate) fn all_sound_off(state: &mut MuntState, part_index: usize) {
    // MIDI "All sound off" (0x78) should release notes immediately regardless of the hold pedal.
    // This controller is not actually implemented by the synths, though (according to the docs and Mok) -
    // we're only using this method internally.
    let mut poly_index = state.parts[part_index].active_polys_first;
    while let Some(pi) = poly_index {
        let next = state.polys[pi].next_index;
        poly::poly_start_decay(state, pi);
        poly_index = next;
    }
}

fn stop_pedal_hold(state: &mut MuntState, part_index: usize) {
    let mut poly_index = state.parts[part_index].active_polys_first;
    while let Some(pi) = poly_index {
        let next = state.polys[pi].next_index;
        poly::poly_stop_pedal_hold(state, pi);
        poly_index = next;
    }
}

/// Dispatches to rhythm or melodic noteOff.
pub(crate) fn note_off(state: &mut MuntState, part_index: usize, midi_key: u32) {
    if state.parts[part_index].is_rhythm {
        // Rhythm: stopNote with raw key
        stop_note(state, part_index, midi_key);
    } else {
        // Melodic: stopNote with adjusted key
        let key = midi_key_to_key(state, part_index, midi_key);
        stop_note(state, part_index, key);
    }
}

fn stop_note(state: &mut MuntState, part_index: usize, key: u32) {
    let hold_pedal = state.parts[part_index].hold_pedal;
    let mut poly_index = state.parts[part_index].active_polys_first;
    while let Some(pi) = poly_index {
        let next = state.polys[pi].next_index;
        // Generally, non-sustaining instruments ignore note off. They die away eventually anyway.
        // Key 0 (only used by special cases on rhythm part) reacts to note off even if non-sustaining or pedal held.
        if state.polys[pi].key == key
            && (state.polys[pi].sustain || key == 0)
            && poly::poly_note_off(state, pi, hold_pedal && key != 0)
        {
            break;
        }
        poly_index = next;
    }
}

/// This should only be called by Poly
pub(crate) fn partial_deactivated(state: &mut MuntState, part_index: usize, poly_index: usize) {
    state.parts[part_index].active_partial_count = state.parts[part_index]
        .active_partial_count
        .saturating_sub(1);
    if !poly::poly_is_active(state, poly_index) {
        poly_list_remove(state, part_index, poly_index);
        partial_manager::poly_freed(state, poly_index);
        // synth->reportHandler->onPolyStateChanged(partNum) - notification only
    }
}

// PolyList operations: index-based linked list over state.polys[].

fn poly_list_prepend(state: &mut MuntState, part_index: usize, poly_index: usize) {
    state.polys[poly_index].next_index = state.parts[part_index].active_polys_first;
    state.parts[part_index].active_polys_first = Some(poly_index);
    if state.parts[part_index].active_polys_last.is_none() {
        state.parts[part_index].active_polys_last = Some(poly_index);
    }
}

fn poly_list_append(state: &mut MuntState, part_index: usize, poly_index: usize) {
    state.polys[poly_index].next_index = None;
    if let Some(last) = state.parts[part_index].active_polys_last {
        state.polys[last].next_index = Some(poly_index);
    }
    state.parts[part_index].active_polys_last = Some(poly_index);
    if state.parts[part_index].active_polys_first.is_none() {
        state.parts[part_index].active_polys_first = Some(poly_index);
    }
}

fn poly_list_take_first(state: &mut MuntState, part_index: usize) -> Option<usize> {
    let first = state.parts[part_index].active_polys_first?;
    let next = state.polys[first].next_index;
    state.parts[part_index].active_polys_first = next;
    if next.is_none() {
        state.parts[part_index].active_polys_last = None;
    }
    state.polys[first].next_index = None;
    Some(first)
}

fn poly_list_remove(state: &mut MuntState, part_index: usize, poly_to_remove: usize) {
    if state.parts[part_index].active_polys_first == Some(poly_to_remove) {
        poly_list_take_first(state, part_index);
        return;
    }
    let mut poly_index = state.parts[part_index].active_polys_first;
    while let Some(pi) = poly_index {
        if state.polys[pi].next_index == Some(poly_to_remove) {
            if state.parts[part_index].active_polys_last == Some(poly_to_remove) {
                state.parts[part_index].active_polys_last = Some(pi);
            }
            state.polys[pi].next_index = state.polys[poly_to_remove].next_index;
            state.polys[poly_to_remove].next_index = None;
            break;
        }
        poly_index = state.polys[pi].next_index;
    }
}
