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

// PartialManager - allocates and frees partials from the global pool.

use crate::{
    enumerations::PolyState,
    poly,
    state::{MAX_PARTS, MuntState, PartialState, PolyStateData},
};

pub(crate) fn init(state: &mut MuntState) {
    let partial_count = state.partial_count as usize;

    // Allocate the global partial and poly pools (C++: PartialManager constructor).
    state.partials = vec![PartialState::default(); partial_count];
    state.polys = vec![PolyStateData::default(); partial_count];

    state.partial_manager.inactive_partial_count = partial_count as u32;
    state.partial_manager.inactive_partials = vec![0i32; partial_count];
    // Initialize the free poly stack: all polys start as free.
    // C++ stores pointers in freePolys[]; we store indices.
    // The order matches C++: freePolys[0] = &polys[0], freePolys[1] = &polys[1], ...
    state.partial_manager.free_polys = (0..partial_count).collect();
    for (i, inactive_partial) in state
        .partial_manager
        .inactive_partials
        .iter_mut()
        .enumerate()
    {
        *inactive_partial = (partial_count - i - 1) as i32;
    }
}

pub(crate) fn clear_already_outputed(state: &mut MuntState) {
    let partial_count = state.partial_count as usize;
    for partial in &mut state.partials[..partial_count] {
        partial.already_outputed = false;
    }
}

pub(crate) fn should_reverb(state: &MuntState, i: usize) -> bool {
    let partial = &state.partials[i];
    if partial.owner_part < 0 {
        return false;
    }
    partial.patch_cache.reverb
}

pub(crate) fn produce_output(state: &mut MuntState, i: usize, buffer_length: u32) -> bool {
    crate::partial::produce_output(state, i, buffer_length)
}

pub(crate) fn deactivate_all(state: &mut MuntState) {
    let partial_count = state.partial_count as usize;
    for i in 0..partial_count {
        // Mirrors Partial::deactivate() - only acts if active.
        if state.partials[i].owner_part >= 0 {
            state.partials[i].owner_part = -1;
            state.partials[i].active = false;
            partial_deactivated(state, i as i32);
        }
    }
}

pub(crate) fn set_reserve(state: &mut MuntState, rset: &[u8; MAX_PARTS]) -> u32 {
    let mut pr: u32 = 0;
    for (x, &value) in rset.iter().enumerate().take(9) {
        state.partial_manager.num_reserved_partials_for_part[x] = value;
        pr += value as u32;
    }
    pr
}

pub(crate) fn alloc_partial(state: &mut MuntState, part_num: i32) -> Option<usize> {
    if state.partial_manager.inactive_partial_count > 0 {
        state.partial_manager.inactive_partial_count -= 1;
        let idx = state.partial_manager.inactive_partials
            [state.partial_manager.inactive_partial_count as usize] as usize;
        // This just marks the partial as being assigned to a part
        state.partials[idx].owner_part = part_num;
        state.partials[idx].active = true;
        return Some(idx);
    }
    // C++: printDebug("PartialManager Error: No inactive partials to allocate for part %d", partNum)
    None
}

pub(crate) fn get_free_partial_count(state: &MuntState) -> u32 {
    state.partial_manager.inactive_partial_count
}

// Finds the lowest-priority part that is exceeding its reserved partial allocation and has a poly
// in POLY_Releasing, then kills its first releasing poly.
// Parts with higher priority than minPart are not checked.
// Assumes that getFreePartials() has been called to make numReservedPartialsForPart up-to-date.
fn abort_first_releasing_poly_where_reserve_exceeded(state: &mut MuntState, min_part: i32) -> bool {
    let mut min_part = min_part;
    if min_part == 8 {
        // Rhythm is highest priority
        min_part = -1;
    }
    let mut part_num = 7i32;
    while part_num >= min_part {
        let use_part_num = if part_num == -1 {
            8usize
        } else {
            part_num as usize
        };
        if state.parts[use_part_num].active_partial_count
            > state.partial_manager.num_reserved_partials_for_part[use_part_num] as u32
        {
            // This part has exceeded its reserved partial count.
            // If it has any releasing polys, kill its first one and we're done.
            if part_abort_first_poly(state, use_part_num, PolyState::Releasing) {
                return true;
            }
        }
        part_num -= 1;
    }
    false
}

// Finds the lowest-priority part that is exceeding its reserved partial allocation and has a poly, then kills
// its first poly in POLY_Held - or failing that, its first poly in any state.
// Parts with higher priority than minPart are not checked.
// Assumes that getFreePartials() has been called to make numReservedPartialsForPart up-to-date.
fn abort_first_poly_prefer_held_where_reserve_exceeded(
    state: &mut MuntState,
    min_part: i32,
) -> bool {
    let mut min_part = min_part;
    if min_part == 8 {
        // Rhythm is highest priority
        min_part = -1;
    }
    let mut part_num = 7i32;
    while part_num >= min_part {
        let use_part_num = if part_num == -1 {
            8usize
        } else {
            part_num as usize
        };
        if state.parts[use_part_num].active_partial_count
            > state.partial_manager.num_reserved_partials_for_part[use_part_num] as u32
        {
            // This part has exceeded its reserved partial count.
            // If it has any polys, kill its first (preferably held) one and we're done.
            if part_abort_first_poly_prefer_held(state, use_part_num) {
                return true;
            }
        }
        part_num -= 1;
    }
    false
}

fn abort_first_poly_prefer_releasing_then_held_where_reserve_exceeded(
    state: &mut MuntState,
    min_part_num: i32,
) -> bool {
    let mut candidate_part_num = 7i32;
    while candidate_part_num >= min_part_num {
        if state.parts[candidate_part_num as usize].active_partial_count
            > state.partial_manager.num_reserved_partials_for_part[candidate_part_num as usize]
                as u32
        {
            return abort_first_poly_on_part_prefer_releasing_then_held(
                state,
                candidate_part_num as usize,
            );
        }
        candidate_part_num -= 1;
    }
    false
}

fn abort_first_poly_on_part_prefer_releasing_then_held(
    state: &mut MuntState,
    part_num: usize,
) -> bool {
    if part_abort_first_poly(state, part_num, PolyState::Releasing) {
        return true;
    }
    part_abort_first_poly_prefer_held(state, part_num)
}

pub(crate) fn free_partials(state: &mut MuntState, needed: u32, part_num: i32) -> bool {
    // NOTE: Currently, we don't consider peculiarities of partial allocation when a timbre involves structures with ring modulation.

    let new_gen = state.rom.control_rom_features.new_gen_note_cancellation;

    if new_gen {
        return free_partials_new_gen(state, needed, part_num);
    }

    while !is_aborting_poly(state) && get_free_partial_count(state) < needed {
        let part_idx = part_num as usize;
        let active_non_releasing = get_active_non_releasing_partial_count(state, part_idx);
        if active_non_releasing + needed
            > state.partial_manager.num_reserved_partials_for_part[part_idx] as u32
        {
            // If priority is given to earlier polys, there's nothing we can do.
            if state.mt32_ram.patch_temp(part_idx).patch.assign_mode & 1 != 0 {
                return false;
            }

            if needed <= state.partial_manager.num_reserved_partials_for_part[part_idx] as u32 {
                // No more partials needed than reserved for this part, only attempt to free partials on the same part.
                abort_first_poly_on_part_prefer_releasing_then_held(state, part_idx);
                continue;
            }

            // More partials desired than reserved, try to borrow some from parts with lesser priority.
            // NOTE: there's a bug here in the old-gen program, so that the device exhibits undefined behaviour when trying to play
            // a note on the rhythm part beyond the reserve while none of the voice parts has exceeded their reserve.
            // We don't emulate this here, assuming that the intention was to traverse all voice parts, then check the rhythm part.
            let min = if part_num < 8 { part_num } else { 0 };
            if abort_first_poly_prefer_releasing_then_held_where_reserve_exceeded(state, min) {
                continue;
            }

            // At this point, old-gen devices try to borrow partials from the rhythm part if it's exceeding the reservation.
            if state.parts[8].active_partial_count
                > state.partial_manager.num_reserved_partials_for_part[8] as u32
                && abort_first_poly_on_part_prefer_releasing_then_held(state, 8)
            {
                continue;
            }

            // Alas, this one will be muted.
            return false;
        }

        // OK, we're not going to exceed the reserve. Reclaim our partials from other parts starting from 7 to 0.
        if abort_first_poly_prefer_releasing_then_held_where_reserve_exceeded(state, 0) {
            continue;
        }

        // Now try the rhythm part.
        if state.parts[8].active_partial_count
            > state.partial_manager.num_reserved_partials_for_part[8] as u32
            && abort_first_poly_on_part_prefer_releasing_then_held(state, 8)
        {
            continue;
        }

        // Lastly, try to silence a poly on this part.
        if abort_first_poly_on_part_prefer_releasing_then_held(state, part_idx) {
            continue;
        }

        // Fair enough, there's no room for it.
        return false;
    }
    true
}

fn free_partials_new_gen(state: &mut MuntState, needed: u32, part_num: i32) -> bool {
    // CONFIRMED: Barring bugs, this matches the real LAPC-I according to information from Mok.

    // BUG: There's a bug in the LAPC-I implementation:
    // When allocating for rhythm part, or when allocating for a part that is using fewer partials than it has reserved,
    // held and playing polys on the rhythm part can potentially be aborted before releasing polys on the rhythm part.
    // This bug isn't present on MT-32.
    // I consider this to be a bug because I think that playing polys should always have priority over held polys,
    // and held polys should always have priority over releasing polys.

    // NOTE: This code generally aborts polys in parts (according to certain conditions) in the following order:
    // 7, 6, 5, 4, 3, 2, 1, 0, 8 (rhythm)
    // (from lowest priority, meaning most likely to have polys aborted, to highest priority, meaning least likely)

    if needed == 0 {
        return true;
    }

    if get_free_partial_count(state) >= needed {
        return true;
    }

    loop {
        // Abort releasing polys in non-rhythm parts that have exceeded their partial reservation (working backwards from part 7)
        if !abort_first_releasing_poly_where_reserve_exceeded(state, 0) {
            break;
        }
        if is_aborting_poly(state) || get_free_partial_count(state) >= needed {
            return true;
        }
    }

    let part_idx = part_num as usize;
    let active_non_releasing = get_active_non_releasing_partial_count(state, part_idx);
    if active_non_releasing + needed
        > state.partial_manager.num_reserved_partials_for_part[part_idx] as u32
    {
        // With the new partials we're freeing for, we would end up using more partials than we have reserved.
        if state.mt32_ram.patch_temp(part_idx).patch.assign_mode & 1 != 0 {
            // Priority is given to earlier polys, so just give up
            return false;
        }
        // Only abort held polys in the target part and parts that have a lower priority
        // (higher part number = lower priority, except for rhythm, which has the highest priority).
        loop {
            if !abort_first_poly_prefer_held_where_reserve_exceeded(state, part_num) {
                break;
            }
            if is_aborting_poly(state) || get_free_partial_count(state) >= needed {
                return true;
            }
        }
        if needed > state.partial_manager.num_reserved_partials_for_part[part_idx] as u32 {
            return false;
        }
    } else {
        // At this point, we're certain that we've reserved enough partials to play our poly.
        // Check all parts from lowest to highest priority to see whether they've exceeded their
        // reserve, and abort their polys until we have enough free partials or they're within
        // their reserve allocation.
        loop {
            if !abort_first_poly_prefer_held_where_reserve_exceeded(state, -1) {
                break;
            }
            if is_aborting_poly(state) || get_free_partial_count(state) >= needed {
                return true;
            }
        }
    }

    // Abort polys in the target part until there are enough free partials for the new one
    loop {
        if !part_abort_first_poly_prefer_held(state, part_idx) {
            break;
        }
        if is_aborting_poly(state) || get_free_partial_count(state) >= needed {
            return true;
        }
    }

    // Aww, not enough partials for you.
    false
}

pub(crate) fn assign_poly_to_part(state: &mut MuntState, part_index: usize) -> Option<usize> {
    let poly_index = state.partial_manager.free_polys.pop()?;
    state.polys[poly_index].part_index = Some(part_index);
    Some(poly_index)
}

pub(crate) fn poly_freed(state: &mut MuntState, poly_index: usize) {
    // C++ pushes to the front (--firstFreePolyIndex; freePolys[firstFreePolyIndex] = poly).
    state.partial_manager.free_polys.insert(0, poly_index);
    state.polys[poly_index].part_index = None;
}

pub(crate) fn partial_deactivated(state: &mut MuntState, partial_index: i32) {
    let partial_count = state.partial_count;
    if state.partial_manager.inactive_partial_count < partial_count {
        let count = state.partial_manager.inactive_partial_count as usize;
        state.partial_manager.inactive_partials[count] = partial_index;
        state.partial_manager.inactive_partial_count += 1;
    } else {
        // C++: printDebug("PartialManager Error: Cannot return deactivated partial %d", partialIndex)
    }
}

// Helper: check whether the synth is currently aborting a poly.
fn is_aborting_poly(state: &MuntState) -> bool {
    state.aborting_poly_index.is_some()
}

// Helper: get active non-releasing partial count for a part.
// This counts partials owned by polys that are not in the Releasing state.
// Mirrors Part::getActiveNonReleasingPartialCount().
fn get_active_non_releasing_partial_count(state: &MuntState, part_index: usize) -> u32 {
    let part = &state.parts[part_index];
    let mut count: u32 = 0;
    let mut current = part.active_polys_first;
    while let Some(poly_idx) = current {
        let poly = &state.polys[poly_idx];
        if poly.state != PolyState::Releasing && poly.state != PolyState::Inactive {
            count += poly.active_partial_count;
        }
        current = poly.next_index;
    }
    count
}

// Helper: abort the first poly on a part matching a given PolyState.
// Mirrors Part::abortFirstPoly(PolyState).
// Returns true if a poly was found and aborted.
fn part_abort_first_poly(
    state: &mut MuntState,
    part_index: usize,
    target_state: PolyState,
) -> bool {
    let mut current = state.parts[part_index].active_polys_first;
    while let Some(poly_idx) = current {
        if state.polys[poly_idx].state == target_state {
            return poly::poly_start_abort(state, poly_idx);
        }
        current = state.polys[poly_idx].next_index;
    }
    false
}

// Helper: abort the first poly on a part, preferring Held, then any state.
// Mirrors Part::abortFirstPolyPreferHeld().
fn part_abort_first_poly_prefer_held(state: &mut MuntState, part_index: usize) -> bool {
    // First try to find a Held poly.
    if part_abort_first_poly(state, part_index, PolyState::Held) {
        return true;
    }
    // Otherwise abort the very first active poly regardless of state.
    // Mirrors Part::abortFirstPoly() (no args).
    let current = state.parts[part_index].active_polys_first;
    if let Some(poly_idx) = current {
        return poly::poly_start_abort(state, poly_idx);
    }
    false
}
