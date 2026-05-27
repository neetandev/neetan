//! Memory Control block (MCB) chain management (allocate, free, resize).

mod best_fit;
pub(crate) mod memory_manager;

use crate::{DosState, MemoryAccess, tables::*};

const MCB_TYPE_M: u8 = 0x4D;
const MCB_TYPE_Z: u8 = 0x5A;
const MAX_CHAIN_WALK: usize = 4096;
const CONVENTIONAL_MEMORY_TOTAL_BYTES: u32 = 640 * 1024;
const HMA_TOTAL_BYTES: u32 = 0xFFF0;

// DOS error codes
const ERR_MCB_DESTROYED: u8 = 7;
const ERR_INSUFFICIENT_MEMORY: u8 = 8;
const ERR_INVALID_BLOCK: u8 = 9;
const UMB_LINK_MCB_NAME: &[u8; 8] = b"CS\0\0\0\0\0\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemoryAmount {
    pub(crate) total_bytes: u32,
    pub(crate) used_bytes: u32,
    pub(crate) free_bytes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HmaMemoryAmount {
    pub(crate) total_bytes: u32,
    pub(crate) used_bytes: u32,
    pub(crate) free_bytes: u32,
    pub(crate) allocated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemoryOverview {
    pub(crate) conventional: MemoryAmount,
    pub(crate) umb: MemoryAmount,
    pub(crate) hma: HmaMemoryAmount,
    pub(crate) ems: MemoryAmount,
    pub(crate) xms: MemoryAmount,
    pub(crate) extended_pool: MemoryAmount,
    pub(crate) largest_conventional_free_bytes: u32,
    pub(crate) largest_umb_free_bytes: u32,
}

fn mcb_addr(segment: u16) -> u32 {
    (segment as u32) << 4
}

fn read_mcb_type(mem: &dyn MemoryAccess, segment: u16) -> u8 {
    mem.read_byte(mcb_addr(segment) + MCB_OFF_TYPE)
}

fn read_mcb_owner(mem: &dyn MemoryAccess, segment: u16) -> u16 {
    mem.read_word(mcb_addr(segment) + MCB_OFF_OWNER)
}

fn read_mcb_size(mem: &dyn MemoryAccess, segment: u16) -> u16 {
    mem.read_word(mcb_addr(segment) + MCB_OFF_SIZE)
}

fn walk_mcb_chain_usage_until(
    mem: &dyn MemoryAccess,
    first_segment: u16,
    stop_before_segment: Option<u16>,
) -> (u32, u32) {
    let mut used_paragraphs: u32 = 0;
    let mut free_paragraphs: u32 = 0;
    let mut current = first_segment;

    for _ in 0..MAX_CHAIN_WALK {
        let block_type = read_mcb_type(mem, current);
        if !is_valid_mcb_type(block_type) {
            break;
        }

        let owner = read_mcb_owner(mem, current);
        let size = read_mcb_size(mem, current) as u32;
        if owner == MCB_OWNER_FREE {
            free_paragraphs += size;
        } else {
            used_paragraphs += size;
        }

        let next = current + size as u16 + 1;
        if block_type == MCB_TYPE_Z || stop_before_segment.is_some_and(|segment| next == segment) {
            break;
        }

        current = next;
    }

    (used_paragraphs, free_paragraphs)
}

fn walk_mcb_chain_usage(mem: &dyn MemoryAccess, first_segment: u16) -> (u32, u32) {
    walk_mcb_chain_usage_until(mem, first_segment, None)
}

fn largest_free_block_paragraphs_until(
    mem: &dyn MemoryAccess,
    first_segment: u16,
    stop_before_segment: Option<u16>,
) -> u32 {
    let mut largest: u32 = 0;
    let mut current = first_segment;

    for _ in 0..MAX_CHAIN_WALK {
        let block_type = read_mcb_type(mem, current);
        if !is_valid_mcb_type(block_type) {
            break;
        }

        let owner = read_mcb_owner(mem, current);
        let size = read_mcb_size(mem, current) as u32;
        if owner == MCB_OWNER_FREE && size > largest {
            largest = size;
        }

        let next = current + size as u16 + 1;
        if block_type == MCB_TYPE_Z || stop_before_segment.is_some_and(|segment| next == segment) {
            break;
        }

        current = next;
    }

    largest
}

fn largest_free_block_paragraphs(mem: &dyn MemoryAccess, first_segment: u16) -> u32 {
    largest_free_block_paragraphs_until(mem, first_segment, None)
}

#[cfg(test)]
pub(crate) fn largest_free_block_paragraphs_pub(mem: &dyn MemoryAccess, first_segment: u16) -> u16 {
    largest_free_block_paragraphs(mem, first_segment).min(u16::MAX as u32) as u16
}

fn conventional_stop_before_segment(umb_first_mcb_segment: Option<u16>) -> Option<u16> {
    umb_first_mcb_segment.map(|_| MEMORY_TOP_SEGMENT)
}

fn dos_strategy_view(
    umb_first_mcb_segment: Option<u16>,
    strategy: u16,
) -> (u16, Option<u16>, bool, bool) {
    let fit_only = strategy & 0x000F;
    let high_first = strategy & 0x0040 != 0;
    let high_only = strategy & 0x0080 != 0;
    let umb = umb_first_mcb_segment.filter(|_| high_first || high_only);

    if umb.is_some() {
        (fit_only, umb, high_first, high_only)
    } else {
        (fit_only, None, false, false)
    }
}

fn walk_chain_for_data_segment(
    mem: &dyn MemoryAccess,
    first_mcb_segment: u16,
    data_segment: u16,
) -> Result<Option<u16>, u8> {
    let target_mcb = data_segment.wrapping_sub(1);
    let mut current = first_mcb_segment;
    let mut prev = None;

    for _ in 0..MAX_CHAIN_WALK {
        let block_type = read_mcb_type(mem, current);
        if !is_valid_mcb_type(block_type) {
            return Err(ERR_MCB_DESTROYED);
        }

        if current == target_mcb {
            return Ok(Some(prev.unwrap_or(current)));
        }

        if block_type == MCB_TYPE_Z {
            return Ok(None);
        }

        let block_size = read_mcb_size(mem, current);
        prev = Some(current);
        current = current + block_size + 1;
    }

    Ok(None)
}

fn find_chain_containing_block(
    mem: &dyn MemoryAccess,
    first_mcb_segment: u16,
    umb_first_mcb_segment: Option<u16>,
    data_segment: u16,
) -> Result<Option<(u16, u16)>, u8> {
    if let Some(prev_or_target) = walk_chain_for_data_segment(mem, first_mcb_segment, data_segment)?
    {
        return Ok(Some((first_mcb_segment, prev_or_target)));
    }

    if let Some(umb_segment) = umb_first_mcb_segment
        && let Some(prev_or_target) = walk_chain_for_data_segment(mem, umb_segment, data_segment)?
    {
        return Ok(Some((umb_segment, prev_or_target)));
    }

    Ok(None)
}

fn write_mcb_type(mem: &mut dyn MemoryAccess, segment: u16, block_type: u8) {
    mem.write_byte(mcb_addr(segment) + MCB_OFF_TYPE, block_type);
}

fn write_mcb_owner(mem: &mut dyn MemoryAccess, segment: u16, owner: u16) {
    mem.write_word(mcb_addr(segment) + MCB_OFF_OWNER, owner);
}

fn write_mcb_size(mem: &mut dyn MemoryAccess, segment: u16, size: u16) {
    mem.write_word(mcb_addr(segment) + MCB_OFF_SIZE, size);
}

fn clear_mcb_name(mem: &mut dyn MemoryAccess, segment: u16) {
    let addr = mcb_addr(segment);
    for i in 0..8u32 {
        mem.write_byte(addr + MCB_OFF_NAME + i, 0x00);
    }
}

fn is_valid_mcb_type(t: u8) -> bool {
    t == MCB_TYPE_M || t == MCB_TYPE_Z
}

#[derive(Clone, Copy)]
pub(crate) struct InitialMcbLayout {
    pub env_segment: u16,
    pub env_paragraphs: u16,
    pub command_mcb_segment: u16,
    pub psp_segment: u16,
    pub free_mcb_segment: u16,
}

pub(crate) fn initial_mcb_layout(env_paragraphs: u16) -> InitialMcbLayout {
    let env_segment = FIRST_MCB_SEGMENT + 1;
    let command_mcb_segment = env_segment + env_paragraphs;
    let psp_segment = command_mcb_segment + 1;
    let free_mcb_segment = psp_segment + COMMAND_BLOCK_PARAGRAPHS;

    InitialMcbLayout {
        env_segment,
        env_paragraphs,
        command_mcb_segment,
        psp_segment,
        free_mcb_segment,
    }
}

pub(crate) fn collect_memory_overview(state: &DosState, mem: &dyn MemoryAccess) -> MemoryOverview {
    let (conventional_used_paragraphs, _) =
        walk_mcb_chain_usage_until(mem, FIRST_MCB_SEGMENT, Some(MEMORY_TOP_SEGMENT));
    let conventional_used_bytes = conventional_used_paragraphs * 16;
    let conventional = MemoryAmount {
        total_bytes: CONVENTIONAL_MEMORY_TOTAL_BYTES,
        used_bytes: conventional_used_bytes,
        free_bytes: CONVENTIONAL_MEMORY_TOTAL_BYTES.saturating_sub(conventional_used_bytes),
    };

    let memory_manager = state.memory_manager.as_ref();

    let umb = if let Some(manager) = memory_manager {
        if manager.is_umb_enabled() {
            let (umb_used_paragraphs, umb_free_paragraphs) =
                walk_mcb_chain_usage(mem, manager.umb_first_mcb_segment());
            let total_bytes = (umb_used_paragraphs + umb_free_paragraphs) * 16;
            let used_bytes = umb_used_paragraphs * 16;
            MemoryAmount {
                total_bytes,
                used_bytes,
                free_bytes: total_bytes.saturating_sub(used_bytes),
            }
        } else {
            MemoryAmount {
                total_bytes: 0,
                used_bytes: 0,
                free_bytes: 0,
            }
        }
    } else {
        MemoryAmount {
            total_bytes: 0,
            used_bytes: 0,
            free_bytes: 0,
        }
    };

    let hma_total_bytes = if memory_manager.is_some_and(|manager| manager.hma_exists()) {
        HMA_TOTAL_BYTES
    } else {
        0
    };
    let hma_allocated = memory_manager.is_some_and(|manager| manager.hma_is_allocated());
    let hma = HmaMemoryAmount {
        total_bytes: hma_total_bytes,
        used_bytes: if hma_allocated { hma_total_bytes } else { 0 },
        free_bytes: if hma_allocated { 0 } else { hma_total_bytes },
        allocated: hma_allocated,
    };

    let ems_used_bytes = memory_manager.map_or(0, |manager| manager.ems_allocated_kb() * 1024);
    let ems_free_bytes = memory_manager.map_or(0, |manager| manager.ems_free_kb() * 1024);
    let ems = MemoryAmount {
        total_bytes: ems_used_bytes + ems_free_bytes,
        used_bytes: ems_used_bytes,
        free_bytes: ems_free_bytes,
    };

    let xms_total_bytes = memory_manager.map_or(0, |manager| manager.xms_total_kb() * 1024);
    let xms_free_bytes = memory_manager.map_or(0, |manager| manager.xms_free_kb() * 1024);
    let xms = MemoryAmount {
        total_bytes: xms_total_bytes,
        used_bytes: xms_total_bytes.saturating_sub(xms_free_bytes),
        free_bytes: xms_free_bytes,
    };

    let extended_pool = if let Some(manager) = memory_manager {
        MemoryAmount {
            total_bytes: manager.extended_pool_total_bytes(),
            used_bytes: manager.extended_pool_used_bytes(),
            free_bytes: manager.extended_pool_free_bytes(),
        }
    } else {
        MemoryAmount {
            total_bytes: 0,
            used_bytes: 0,
            free_bytes: 0,
        }
    };

    MemoryOverview {
        conventional,
        umb,
        hma,
        ems,
        xms,
        extended_pool,
        largest_conventional_free_bytes: largest_free_block_paragraphs_until(
            mem,
            FIRST_MCB_SEGMENT,
            Some(MEMORY_TOP_SEGMENT),
        ) * 16,
        largest_umb_free_bytes: if umb.total_bytes > 0 {
            memory_manager
                .map(|manager| {
                    largest_free_block_paragraphs(mem, manager.umb_first_mcb_segment()) * 16
                })
                .unwrap_or(0)
        } else {
            0
        },
    }
}

pub(crate) fn format_host_memory_overview(overview: &MemoryOverview) -> Vec<String> {
    vec![
        "Memory overview (HLE DOS)".to_string(),
        format!(
            "Conventional: total={} used={} free={}",
            format_debug_size(overview.conventional.total_bytes),
            format_debug_size(overview.conventional.used_bytes),
            format_debug_size(overview.conventional.free_bytes),
        ),
        format!(
            "UMB: total={} used={} free={}",
            format_debug_size(overview.umb.total_bytes),
            format_debug_size(overview.umb.used_bytes),
            format_debug_size(overview.umb.free_bytes),
        ),
        format!(
            "HMA: total={} used={} free={} state={}",
            format_debug_size(overview.hma.total_bytes),
            format_debug_size(overview.hma.used_bytes),
            format_debug_size(overview.hma.free_bytes),
            if overview.hma.allocated {
                "allocated"
            } else {
                "free"
            },
        ),
        format!(
            "EMS: total={} used={} free={}",
            format_debug_size(overview.ems.total_bytes),
            format_debug_size(overview.ems.used_bytes),
            format_debug_size(overview.ems.free_bytes),
        ),
        format!(
            "XMS: total={} used={} free={}",
            format_debug_size(overview.xms.total_bytes),
            format_debug_size(overview.xms.used_bytes),
            format_debug_size(overview.xms.free_bytes),
        ),
        format!(
            "Extended backing pool (EMS+XMS): total={} used={} free={}",
            format_debug_size(overview.extended_pool.total_bytes),
            format_debug_size(overview.extended_pool.used_bytes),
            format_debug_size(overview.extended_pool.free_bytes),
        ),
        "Note: EMS and XMS share the same backing pool.".to_string(),
    ]
}

fn format_debug_size(bytes: u32) -> String {
    if bytes == 0 {
        return "0 bytes".to_string();
    }
    if bytes.is_multiple_of(1024) {
        return format!(
            "{}K ({} bytes)",
            format_debug_number(bytes / 1024),
            format_debug_number(bytes),
        );
    }
    format!("{} bytes", format_debug_number(bytes))
}

fn format_debug_number(value: u32) -> String {
    let digits = value.to_string();
    let mut result = String::new();
    for (index, ch) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result.chars().rev().collect()
}

pub(crate) fn read_mcb_size_pub(mem: &dyn MemoryAccess, segment: u16) -> u16 {
    read_mcb_size(mem, segment)
}

/// Writes a 16-byte MCB header at the given segment.
pub(crate) fn write_mcb(
    mem: &mut dyn MemoryAccess,
    segment: u16,
    block_type: u8,
    owner: u16,
    size: u16,
    name: &[u8; 8],
) {
    let addr = (segment as u32) << 4;
    mem.write_byte(addr + MCB_OFF_TYPE, block_type);
    mem.write_word(addr + MCB_OFF_OWNER, owner);
    mem.write_word(addr + MCB_OFF_SIZE, size);
    // Reserved bytes (offset 5-7) are zero.
    mem.write_byte(addr + 5, 0);
    mem.write_byte(addr + 6, 0);
    mem.write_byte(addr + 7, 0);
    mem.write_block(addr + MCB_OFF_NAME, name);
}

/// Creates the initial 3-MCB chain after DOS data structures:
///   MCB[0]: environment block (owner=DOS, name="SD")
///   MCB[1]: COMMAND.COM PSP + code stub (owner=PSP_SEGMENT, name="COMMAND\0")
///   MCB[2]: free memory to 640 KB (owner=free)
pub(crate) fn write_initial_mcb_chain(mem: &mut dyn MemoryAccess, layout: InitialMcbLayout) {
    // MCB[0]: environment block
    write_mcb(
        mem,
        FIRST_MCB_SEGMENT,
        0x4D, // 'M'
        MCB_OWNER_DOS,
        layout.env_paragraphs,
        b"SD\0\0\0\0\0\0",
    );

    // MCB[1]: COMMAND.COM (PSP + code stub)
    write_mcb(
        mem,
        layout.command_mcb_segment,
        0x4D, // 'M'
        layout.psp_segment,
        COMMAND_BLOCK_PARAGRAPHS,
        b"COMMAND\0",
    );

    // MCB[2]: free memory (Z block)
    let free_paragraphs = MEMORY_TOP_SEGMENT - layout.free_mcb_segment - 1;
    write_mcb(
        mem,
        layout.free_mcb_segment,
        0x5A, // 'Z'
        MCB_OWNER_FREE,
        free_paragraphs,
        b"\0\0\0\0\0\0\0\0",
    );
}

/// DOS INT 21h 48h entry point: allocates from conventional memory and,
/// when `umb_first_mcb_segment` is `Some` (UMB is linked per DOS 5803h),
/// honours the high-memory preference flags `+0x40` (high-first) and
/// `+0x80` (high-only) in `strategy`.
///
/// Returns Ok(data_segment) on success, Err((error_code, largest_available))
/// on failure. `largest_available` is the largest free block across all
/// searched chains.
pub(crate) fn allocate_dos(
    mem: &mut dyn MemoryAccess,
    first_mcb_segment: u16,
    umb_first_mcb_segment: Option<u16>,
    paragraphs: u16,
    owner: u16,
    strategy: u16,
) -> Result<u16, (u8, u16)> {
    let (fit_only, umb, high_first, high_only) = dos_strategy_view(umb_first_mcb_segment, strategy);
    let conventional_stop_before = conventional_stop_before_segment(umb_first_mcb_segment);

    let first_result = match (umb, high_only) {
        (Some(umb_seg), true) => allocate(mem, umb_seg, paragraphs, owner, fit_only),
        (Some(umb_seg), false) => allocate(mem, umb_seg, paragraphs, owner, fit_only),
        (None, _) => allocate_with_limit(
            mem,
            first_mcb_segment,
            conventional_stop_before,
            paragraphs,
            owner,
            fit_only,
        ),
    };

    match first_result {
        Ok(segment) => Ok(segment),
        Err((code, first_largest)) => {
            // high_only: no fallback allowed.
            if high_only {
                return Err((code, first_largest));
            }
            // high_first: fall back to conventional.
            if high_first {
                match allocate_with_limit(
                    mem,
                    first_mcb_segment,
                    conventional_stop_before,
                    paragraphs,
                    owner,
                    fit_only,
                ) {
                    Ok(segment) => Ok(segment),
                    Err((code2, conv_largest)) => Err((code2, first_largest.max(conv_largest))),
                }
            } else {
                Err((code, first_largest))
            }
        }
    }
}

pub(crate) fn largest_available_dos(
    mem: &dyn MemoryAccess,
    first_mcb_segment: u16,
    umb_first_mcb_segment: Option<u16>,
    strategy: u16,
) -> u16 {
    let (_fit_only, umb, high_first, high_only) =
        dos_strategy_view(umb_first_mcb_segment, strategy);
    let conventional_largest = largest_free_block_paragraphs_until(
        mem,
        first_mcb_segment,
        conventional_stop_before_segment(umb_first_mcb_segment),
    ) as u16;
    let umb_largest = umb
        .map(|segment| largest_free_block_paragraphs(mem, segment) as u16)
        .unwrap_or(0);

    if high_only {
        umb_largest
    } else if high_first {
        conventional_largest.max(umb_largest)
    } else {
        conventional_largest
    }
}

/// Allocates a block of `paragraphs` paragraphs from the MCB chain.
///
/// Strategy: 0 = first fit, 1 = best fit, 2 = last fit.
/// Returns Ok(data_segment) on success, where data_segment = MCB segment + 1.
/// Returns Err((error_code, largest_available)) on failure.
pub(crate) fn allocate(
    mem: &mut dyn MemoryAccess,
    first_mcb_segment: u16,
    paragraphs: u16,
    owner: u16,
    strategy: u16,
) -> Result<u16, (u8, u16)> {
    allocate_with_limit(mem, first_mcb_segment, None, paragraphs, owner, strategy)
}

fn allocate_with_limit(
    mem: &mut dyn MemoryAccess,
    first_mcb_segment: u16,
    stop_before_segment: Option<u16>,
    paragraphs: u16,
    owner: u16,
    strategy: u16,
) -> Result<u16, (u8, u16)> {
    // Walk the chain once to find the best candidate and the largest free block.
    let mut current = first_mcb_segment;
    let mut largest_free: u16 = 0;
    let mut candidate: Option<(u16, u16)> = None; // (segment, size)

    for _ in 0..MAX_CHAIN_WALK {
        let block_type = read_mcb_type(mem, current);
        if !is_valid_mcb_type(block_type) {
            return Err((ERR_MCB_DESTROYED, largest_free));
        }

        let block_owner = read_mcb_owner(mem, current);
        let mut block_size = read_mcb_size(mem, current);

        // Lazy coalescing: real MS-DOS merges adjacent free blocks during the
        // ALLOC chain walk, not in FREE. Greedily merge any free blocks
        // immediately following this one before evaluating fit.
        if block_owner == MCB_OWNER_FREE {
            loop {
                let block_type_now = read_mcb_type(mem, current);
                if block_type_now != MCB_TYPE_M {
                    break;
                }
                let next = current + block_size + 1;
                let next_type = read_mcb_type(mem, next);
                if !is_valid_mcb_type(next_type) {
                    break;
                }
                if read_mcb_owner(mem, next) != MCB_OWNER_FREE {
                    break;
                }
                coalesce_forward(mem, current);
                block_size = read_mcb_size(mem, current);
            }
        }

        if block_owner == MCB_OWNER_FREE {
            if block_size > largest_free {
                largest_free = block_size;
            }

            if block_size >= paragraphs {
                let use_this = match strategy {
                    // First fit: take the first match.
                    0 => candidate.is_none(),
                    // Best fit: take the smallest sufficient block.
                    1 => candidate.is_none() || block_size < candidate.unwrap().1,
                    // Last fit: always prefer the later block.
                    2 => true,
                    // Unknown strategy: fall back to first fit.
                    _ => candidate.is_none(),
                };

                if use_this {
                    candidate = Some((current, block_size));
                    // For first fit we can stop scanning immediately.
                    if strategy == 0 {
                        commit_allocation(mem, current, block_size, paragraphs, owner, false);
                        return Ok(current + 1);
                    }
                }
            }
        }

        let next = current + block_size + 1;
        if block_type == MCB_TYPE_Z || stop_before_segment.is_some_and(|segment| next == segment) {
            break;
        }

        current = next;
    }

    // For best-fit and last-fit, commit the chosen candidate after the full walk.
    if let Some((segment, size)) = candidate {
        let allocate_from_high = strategy == 2;
        Ok(commit_allocation(
            mem,
            segment,
            size,
            paragraphs,
            owner,
            allocate_from_high,
        ))
    } else {
        Err((ERR_INSUFFICIENT_MEMORY, largest_free))
    }
}

/// Commits an allocation at `segment` by setting the owner and optionally splitting.
fn commit_allocation(
    mem: &mut dyn MemoryAccess,
    segment: u16,
    block_size: u16,
    paragraphs: u16,
    owner: u16,
    allocate_from_high: bool,
) -> u16 {
    let block_type = read_mcb_type(mem, segment);

    let allocated_segment = if allocate_from_high && block_size > paragraphs + 1 {
        let remaining_free_size = block_size - paragraphs - 1;
        let allocated_segment = segment + remaining_free_size + 1;

        write_mcb_type(mem, segment, MCB_TYPE_M);
        write_mcb_owner(mem, segment, MCB_OWNER_FREE);
        write_mcb_size(mem, segment, remaining_free_size);
        clear_mcb_name(mem, segment);

        write_mcb_type(mem, allocated_segment, block_type);
        write_mcb_size(mem, allocated_segment, paragraphs);
        allocated_segment
    } else {
        if block_size > paragraphs + 1 {
            // Split: create a new free MCB after the allocated block.
            let remainder_segment = segment + 1 + paragraphs;
            let remainder_size = block_size - paragraphs - 1;

            write_mcb_type(mem, remainder_segment, block_type);
            write_mcb_owner(mem, remainder_segment, MCB_OWNER_FREE);
            write_mcb_size(mem, remainder_segment, remainder_size);
            clear_mcb_name(mem, remainder_segment);
            let addr = mcb_addr(remainder_segment);
            mem.write_byte(addr + 5, 0);
            mem.write_byte(addr + 6, 0);
            mem.write_byte(addr + 7, 0);

            // Current block becomes M type (there's a block after it now).
            write_mcb_type(mem, segment, MCB_TYPE_M);
            write_mcb_size(mem, segment, paragraphs);
        }

        segment
    };

    write_mcb_owner(mem, allocated_segment, owner);
    clear_mcb_name(mem, allocated_segment);

    allocated_segment + 1
}

/// Frees the memory block whose data starts at `data_segment`.
///
/// Returns Ok(()) on success.
/// Returns Err(error_code) on failure.
pub(crate) fn free(
    mem: &mut dyn MemoryAccess,
    first_mcb_segment: u16,
    data_segment: u16,
) -> Result<(), u8> {
    let target_mcb = data_segment.wrapping_sub(1);

    // Walk the chain to verify the target MCB exists.
    let mut current = first_mcb_segment;
    let mut found = false;

    for _ in 0..MAX_CHAIN_WALK {
        let block_type = read_mcb_type(mem, current);
        if !is_valid_mcb_type(block_type) {
            return Err(ERR_MCB_DESTROYED);
        }

        if current == target_mcb {
            found = true;
            break;
        }

        if block_type == MCB_TYPE_Z {
            break;
        }

        let block_size = read_mcb_size(mem, current);
        current = current + block_size + 1;
    }

    if !found {
        return Err(ERR_INVALID_BLOCK);
    }

    if read_mcb_owner(mem, target_mcb) == MCB_OWNER_FREE {
        return Err(ERR_INVALID_BLOCK);
    }

    // Real MS-DOS only flips the owner to zero on FREE; it does not
    // coalesce. Coalescing of adjacent free blocks happens lazily during
    // the next AH=48h ALLOC chain walk.
    write_mcb_owner(mem, target_mcb, MCB_OWNER_FREE);
    clear_mcb_name(mem, target_mcb);

    Ok(())
}

pub(crate) fn free_dos(
    mem: &mut dyn MemoryAccess,
    first_mcb_segment: u16,
    umb_first_mcb_segment: Option<u16>,
    data_segment: u16,
) -> Result<(), u8> {
    match find_chain_containing_block(mem, first_mcb_segment, umb_first_mcb_segment, data_segment)?
    {
        Some((chain_start, _)) => free(mem, chain_start, data_segment),
        None => Err(ERR_INVALID_BLOCK),
    }
}

/// Resizes the memory block at `data_segment` to `new_paragraphs`.
///
/// Returns Ok(()) on success.
/// Returns Err((error_code, max_available)) on failure, where max_available
/// is the largest contiguous free block remaining in the chain (per the
/// MS-DOS INT 21h 4Ah convention).
pub(crate) fn resize(
    mem: &mut dyn MemoryAccess,
    first_mcb_segment: u16,
    data_segment: u16,
    new_paragraphs: u16,
) -> Result<(), (u8, u16)> {
    let target_mcb = data_segment.wrapping_sub(1);

    // Walk chain to verify MCB exists
    let mut current = first_mcb_segment;
    let mut found = false;

    for _ in 0..MAX_CHAIN_WALK {
        let block_type = read_mcb_type(mem, current);
        if !is_valid_mcb_type(block_type) {
            let largest = largest_free_block_paragraphs(mem, first_mcb_segment);
            return Err((ERR_MCB_DESTROYED, largest.min(0xFFFF) as u16));
        }

        if current == target_mcb {
            found = true;
            break;
        }

        if block_type == MCB_TYPE_Z {
            break;
        }

        let block_size = read_mcb_size(mem, current);
        current = current + block_size + 1;
    }

    if !found {
        let largest = largest_free_block_paragraphs(mem, first_mcb_segment);
        return Err((ERR_INVALID_BLOCK, largest.min(0xFFFF) as u16));
    }

    let current_size = read_mcb_size(mem, target_mcb);
    let block_type = read_mcb_type(mem, target_mcb);

    if block_type != MCB_TYPE_Z {
        let next_segment = target_mcb + current_size + 1;
        if is_valid_mcb_type(read_mcb_type(mem, next_segment))
            && read_mcb_owner(mem, next_segment) == MCB_OWNER_FREE
        {
            coalesce_chain_from(mem, next_segment);
        }
    }

    // Compute the maximum size this particular block could grow to (current
    // plus any immediately-following free block). Used for both the
    // query-only path (new_paragraphs == 0xFFFF) and as the error-return
    // "max size" on grow failure.
    let max_growable = if block_type == MCB_TYPE_Z {
        current_size
    } else {
        let next_segment = target_mcb + current_size + 1;
        let next_type = read_mcb_type(mem, next_segment);
        if !is_valid_mcb_type(next_type) {
            current_size
        } else if read_mcb_owner(mem, next_segment) == MCB_OWNER_FREE {
            let next_size = read_mcb_size(mem, next_segment);
            ((current_size as u32) + 1 + next_size as u32).min(0xFFFF) as u16
        } else {
            current_size
        }
    };

    if new_paragraphs == current_size {
        return Ok(());
    }

    if new_paragraphs < current_size {
        // Shrink: split the block and create a free remainder
        if current_size > new_paragraphs + 1 {
            let remainder_segment = target_mcb + 1 + new_paragraphs;
            let remainder_size = current_size - new_paragraphs - 1;

            write_mcb_type(mem, remainder_segment, block_type);
            write_mcb_owner(mem, remainder_segment, MCB_OWNER_FREE);
            write_mcb_size(mem, remainder_segment, remainder_size);
            clear_mcb_name(mem, remainder_segment);
            let addr = mcb_addr(remainder_segment);
            mem.write_byte(addr + 5, 0);
            mem.write_byte(addr + 6, 0);
            mem.write_byte(addr + 7, 0);

            write_mcb_type(mem, target_mcb, MCB_TYPE_M);
            write_mcb_size(mem, target_mcb, new_paragraphs);

            // Coalesce new free block with next if also free
            coalesce_forward(mem, remainder_segment);
        }
        // If current_size == new_paragraphs + 1, can't split; leave as-is (waste 1 paragraph)

        Ok(())
    } else {
        // Grow: check if next block is free and has enough space
        if block_type == MCB_TYPE_Z {
            // Z block: can't grow (nothing after it)
            return Err((ERR_INSUFFICIENT_MEMORY, max_growable));
        }

        let next_segment = target_mcb + current_size + 1;
        let next_type = read_mcb_type(mem, next_segment);
        if !is_valid_mcb_type(next_type) {
            return Err((ERR_MCB_DESTROYED, max_growable));
        }

        let next_owner = read_mcb_owner(mem, next_segment);
        let next_size = read_mcb_size(mem, next_segment);

        if next_owner != MCB_OWNER_FREE {
            // Next block is not free; can't grow
            return Err((ERR_INSUFFICIENT_MEMORY, max_growable));
        }

        // Total available = current size + 1 (MCB of next) + next size
        let total_available = current_size as u32 + 1 + next_size as u32;
        if (new_paragraphs as u32) > total_available {
            let merged_size = total_available.min(0xFFFF) as u16;

            if current_size != merged_size {
                write_mcb_type(mem, target_mcb, next_type);
                write_mcb_size(mem, target_mcb, merged_size);
            }

            return Err((ERR_INSUFFICIENT_MEMORY, merged_size));
        }

        // Merge with next block
        let merged_size = (total_available) as u16;

        if new_paragraphs == merged_size {
            // Use entire merged space
            write_mcb_type(mem, target_mcb, next_type);
            write_mcb_size(mem, target_mcb, merged_size);
        } else if merged_size > new_paragraphs + 1 {
            // Split: grow target, create new free remainder
            let remainder_segment = target_mcb + 1 + new_paragraphs;
            let remainder_size = merged_size - new_paragraphs - 1;

            write_mcb_type(mem, remainder_segment, next_type);
            write_mcb_owner(mem, remainder_segment, MCB_OWNER_FREE);
            write_mcb_size(mem, remainder_segment, remainder_size);
            clear_mcb_name(mem, remainder_segment);
            let addr = mcb_addr(remainder_segment);
            mem.write_byte(addr + 5, 0);
            mem.write_byte(addr + 6, 0);
            mem.write_byte(addr + 7, 0);

            write_mcb_type(mem, target_mcb, MCB_TYPE_M);
            write_mcb_size(mem, target_mcb, new_paragraphs);
        } else {
            // merged_size == new_paragraphs + 1: give the extra paragraph
            write_mcb_type(mem, target_mcb, next_type);
            write_mcb_size(mem, target_mcb, merged_size);
        }

        Ok(())
    }
}

fn conventional_link_anchor_segment(
    mem: &dyn MemoryAccess,
    first_mcb_segment: u16,
) -> Result<u16, u8> {
    let mut current = first_mcb_segment;

    for _ in 0..MAX_CHAIN_WALK {
        let block_type = read_mcb_type(mem, current);
        if !is_valid_mcb_type(block_type) {
            return Err(ERR_MCB_DESTROYED);
        }

        let next_segment = current + read_mcb_size(mem, current) + 1;
        if next_segment == MEMORY_TOP_SEGMENT {
            return Ok(current);
        }

        if block_type == MCB_TYPE_Z {
            return Err(ERR_MCB_DESTROYED);
        }

        current = next_segment;
    }

    Err(ERR_MCB_DESTROYED)
}

pub(crate) fn set_dos_umb_link_state(
    mem: &mut dyn MemoryAccess,
    first_mcb_segment: u16,
    umb_first_mcb_segment: Option<u16>,
    linked: bool,
) -> Result<(), u8> {
    let Some(umb_segment) = umb_first_mcb_segment else {
        return Ok(());
    };

    if !is_valid_mcb_type(read_mcb_type(mem, umb_segment)) {
        return Err(ERR_MCB_DESTROYED);
    }

    let anchor_segment = conventional_link_anchor_segment(mem, first_mcb_segment)?;
    if linked {
        write_mcb_type(mem, anchor_segment, MCB_TYPE_M);
        write_mcb(
            mem,
            MEMORY_TOP_SEGMENT,
            MCB_TYPE_M,
            MCB_OWNER_DOS,
            umb_segment - MEMORY_TOP_SEGMENT - 1,
            UMB_LINK_MCB_NAME,
        );
    } else {
        write_mcb_type(mem, anchor_segment, MCB_TYPE_Z);
    }

    Ok(())
}

pub(crate) fn resize_dos(
    mem: &mut dyn MemoryAccess,
    first_mcb_segment: u16,
    umb_first_mcb_segment: Option<u16>,
    data_segment: u16,
    new_paragraphs: u16,
) -> Result<(), (u8, u16)> {
    match find_chain_containing_block(mem, first_mcb_segment, umb_first_mcb_segment, data_segment) {
        Ok(Some((chain_start, _))) => resize(mem, chain_start, data_segment, new_paragraphs),
        Ok(None) => {
            let largest = largest_available_dos(mem, first_mcb_segment, umb_first_mcb_segment, 0);
            Err((ERR_INVALID_BLOCK, largest))
        }
        Err(error_code) => {
            let largest = largest_available_dos(mem, first_mcb_segment, umb_first_mcb_segment, 0);
            Err((error_code, largest))
        }
    }
}

/// Frees all MCB blocks owned by `owner_psp`.
///
/// Walks the chain, collects data segments, then frees each one.
/// `free()` does not coalesce; the next ALLOC will lazily merge adjacent
/// free blocks during its chain walk.
pub(crate) fn free_process_blocks(
    mem: &mut dyn MemoryAccess,
    first_mcb_segment: u16,
    owner_psp: u16,
) {
    let mut to_free = Vec::new();
    let mut current = first_mcb_segment;
    for _ in 0..MAX_CHAIN_WALK {
        let block_type = read_mcb_type(mem, current);
        if !is_valid_mcb_type(block_type) {
            break;
        }
        if read_mcb_owner(mem, current) == owner_psp {
            to_free.push(current + 1);
        }
        if block_type == MCB_TYPE_Z {
            break;
        }
        current = current + read_mcb_size(mem, current) + 1;
    }
    for data_seg in to_free.into_iter().rev() {
        let _ = free(mem, first_mcb_segment, data_seg);
    }
    coalesce_chain(mem, first_mcb_segment);
}

pub(crate) fn free_process_blocks_dos(
    mem: &mut dyn MemoryAccess,
    first_mcb_segment: u16,
    umb_first_mcb_segment: Option<u16>,
    owner_psp: u16,
) {
    free_process_blocks(mem, first_mcb_segment, owner_psp);
    if let Some(umb_segment) = umb_first_mcb_segment {
        free_process_blocks(mem, umb_segment, owner_psp);
    }
}

/// Frees all MCB blocks owned by `owner_psp` except the PSP's own block,
/// which is resized to `keep_paragraphs` (for TSR termination).
pub(crate) fn free_process_blocks_tsr(
    mem: &mut dyn MemoryAccess,
    first_mcb_segment: u16,
    owner_psp: u16,
    keep_paragraphs: u16,
) {
    let mut to_free = Vec::new();
    let mut current = first_mcb_segment;
    for _ in 0..MAX_CHAIN_WALK {
        let block_type = read_mcb_type(mem, current);
        if !is_valid_mcb_type(block_type) {
            break;
        }
        if read_mcb_owner(mem, current) == owner_psp {
            if current + 1 == owner_psp {
                // PSP's own MCB: resize instead of freeing.
                let _ = resize(mem, first_mcb_segment, owner_psp, keep_paragraphs);
            } else {
                to_free.push(current + 1);
            }
        }
        if block_type == MCB_TYPE_Z {
            break;
        }
        current = current + read_mcb_size(mem, current) + 1;
    }
    for data_seg in to_free.into_iter().rev() {
        let _ = free(mem, first_mcb_segment, data_seg);
    }
    coalesce_chain(mem, first_mcb_segment);
}

pub(crate) fn free_process_blocks_tsr_dos(
    mem: &mut dyn MemoryAccess,
    first_mcb_segment: u16,
    umb_first_mcb_segment: Option<u16>,
    owner_psp: u16,
    keep_paragraphs: u16,
) {
    free_process_blocks_tsr(mem, first_mcb_segment, owner_psp, keep_paragraphs);
    if let Some(umb_segment) = umb_first_mcb_segment {
        free_process_blocks_tsr(mem, umb_segment, owner_psp, keep_paragraphs);
    }
}

/// Walks the chain starting at `first_mcb_segment` and merges every run of
/// adjacent free MCBs. Used by bulk-cleanup paths (process termination)
/// since single FREE no longer coalesces.
fn coalesce_chain(mem: &mut dyn MemoryAccess, first_mcb_segment: u16) {
    let mut current = first_mcb_segment;
    for _ in 0..MAX_CHAIN_WALK {
        let block_type = read_mcb_type(mem, current);
        if !is_valid_mcb_type(block_type) {
            return;
        }
        if read_mcb_owner(mem, current) == MCB_OWNER_FREE {
            loop {
                let block_type_now = read_mcb_type(mem, current);
                if block_type_now != MCB_TYPE_M {
                    break;
                }
                let next = current + read_mcb_size(mem, current) + 1;
                let next_type = read_mcb_type(mem, next);
                if !is_valid_mcb_type(next_type) {
                    break;
                }
                if read_mcb_owner(mem, next) != MCB_OWNER_FREE {
                    break;
                }
                coalesce_forward(mem, current);
            }
        }
        if read_mcb_type(mem, current) == MCB_TYPE_Z {
            return;
        }
        current = current + read_mcb_size(mem, current) + 1;
    }
}

fn coalesce_chain_from(mem: &mut dyn MemoryAccess, segment: u16) {
    loop {
        let block_type = read_mcb_type(mem, segment);
        if block_type != MCB_TYPE_M || read_mcb_owner(mem, segment) != MCB_OWNER_FREE {
            return;
        }

        let next_segment = segment + read_mcb_size(mem, segment) + 1;
        let next_type = read_mcb_type(mem, next_segment);
        if !is_valid_mcb_type(next_type) || read_mcb_owner(mem, next_segment) != MCB_OWNER_FREE {
            return;
        }

        coalesce_forward(mem, segment);
    }
}

/// Coalesces a free MCB with the next MCB if it is also free.
fn coalesce_forward(mem: &mut dyn MemoryAccess, segment: u16) {
    let block_type = read_mcb_type(mem, segment);
    if block_type != MCB_TYPE_M {
        return; // Z block or invalid: nothing after it
    }

    let block_size = read_mcb_size(mem, segment);
    let next_segment = segment + block_size + 1;

    let next_type = read_mcb_type(mem, next_segment);
    if !is_valid_mcb_type(next_type) {
        return;
    }

    let next_owner = read_mcb_owner(mem, next_segment);
    if next_owner != MCB_OWNER_FREE {
        return;
    }

    // Merge: absorb next block (its MCB + data) into current
    let next_size = read_mcb_size(mem, next_segment);
    let merged_size = block_size + 1 + next_size;
    write_mcb_size(mem, segment, merged_size);
    write_mcb_type(mem, segment, next_type); // Inherit M/Z from next
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockMemory {
        data: Vec<u8>,
    }

    impl MockMemory {
        fn new(size: usize) -> Self {
            Self {
                data: vec![0; size],
            }
        }
    }

    impl MemoryAccess for MockMemory {
        fn read_byte(&self, address: u32) -> u8 {
            self.data[address as usize]
        }

        fn write_byte(&mut self, address: u32, value: u8) {
            self.data[address as usize] = value;
        }

        fn read_word(&self, address: u32) -> u16 {
            let lo = self.read_byte(address) as u16;
            let hi = self.read_byte(address + 1) as u16;
            lo | (hi << 8)
        }

        fn write_word(&mut self, address: u32, value: u16) {
            self.write_byte(address, value as u8);
            self.write_byte(address + 1, (value >> 8) as u8);
        }

        fn read_block(&self, address: u32, buf: &mut [u8]) {
            let start = address as usize;
            let end = start + buf.len();
            buf.copy_from_slice(&self.data[start..end]);
        }

        fn write_block(&mut self, address: u32, data: &[u8]) {
            let start = address as usize;
            let end = start + data.len();
            self.data[start..end].copy_from_slice(data);
        }
    }

    #[test]
    fn free_process_blocks_frees_in_reverse_to_fully_coalesce() {
        let mut mem = MockMemory::new(0x20000);
        let first_mcb = 0x1000;
        let owner = 0x2222;

        write_mcb(
            &mut mem,
            first_mcb,
            MCB_TYPE_M,
            MCB_OWNER_DOS,
            1,
            b"DOS     ",
        );
        write_mcb(&mut mem, 0x1002, MCB_TYPE_M, owner, 2, b"OWN1    ");
        write_mcb(&mut mem, 0x1005, MCB_TYPE_M, owner, 3, b"OWN2    ");
        write_mcb(&mut mem, 0x1009, MCB_TYPE_Z, MCB_OWNER_FREE, 4, b"FREE    ");

        free_process_blocks(&mut mem, first_mcb, owner);

        assert_eq!(read_mcb_owner(&mem, 0x1002), MCB_OWNER_FREE);
        assert_eq!(read_mcb_type(&mem, 0x1002), MCB_TYPE_Z);
        assert_eq!(
            read_mcb_size(&mem, 0x1002),
            2 + 1 + 3 + 1 + 4,
            "owned blocks should coalesce with the trailing free block into one Z block"
        );
    }

    #[test]
    fn free_process_blocks_tsr_frees_following_blocks_in_reverse_to_fully_coalesce() {
        let mut mem = MockMemory::new(0x20000);
        let first_mcb = 0x1000;
        let owner_psp = 0x1003;

        write_mcb(
            &mut mem,
            first_mcb,
            MCB_TYPE_M,
            MCB_OWNER_DOS,
            1,
            b"DOS     ",
        );
        write_mcb(&mut mem, 0x1002, MCB_TYPE_M, owner_psp, 2, b"PSP     ");
        write_mcb(&mut mem, 0x1005, MCB_TYPE_M, owner_psp, 2, b"AUX1    ");
        write_mcb(&mut mem, 0x1008, MCB_TYPE_M, owner_psp, 3, b"AUX2    ");
        write_mcb(&mut mem, 0x100C, MCB_TYPE_Z, MCB_OWNER_FREE, 4, b"FREE    ");

        free_process_blocks_tsr(&mut mem, first_mcb, owner_psp, 2);

        assert_eq!(
            read_mcb_owner(&mem, 0x1002),
            owner_psp,
            "TSR should retain the PSP block"
        );
        assert_eq!(read_mcb_size(&mem, 0x1002), 2);
        assert_eq!(read_mcb_owner(&mem, 0x1005), MCB_OWNER_FREE);
        assert_eq!(read_mcb_type(&mem, 0x1005), MCB_TYPE_Z);
        assert_eq!(
            read_mcb_size(&mem, 0x1005),
            2 + 1 + 3 + 1 + 4,
            "all non-PSP blocks should coalesce into one trailing free Z block"
        );
    }

    #[test]
    fn free_dos_releases_umb_block() {
        let mut mem = MockMemory::new(0x200000);
        write_mcb(
            &mut mem,
            0x1000,
            MCB_TYPE_Z,
            MCB_OWNER_FREE,
            0x20,
            b"CONVFREE",
        );
        write_mcb(
            &mut mem,
            UMB_FIRST_MCB_SEGMENT,
            MCB_TYPE_Z,
            0x2222,
            0x10,
            b"UMBALLOC",
        );

        free_dos(
            &mut mem,
            0x1000,
            Some(UMB_FIRST_MCB_SEGMENT),
            UMB_FIRST_MCB_SEGMENT + 1,
        )
        .expect("UMB block should be found by DOS-aware free");

        assert_eq!(read_mcb_owner(&mem, UMB_FIRST_MCB_SEGMENT), MCB_OWNER_FREE);
    }

    #[test]
    fn allocate_last_fit_uses_high_end_of_selected_block() {
        let mut mem = MockMemory::new(0x200000);
        let first_mcb = 0x1000;

        write_mcb(
            &mut mem,
            first_mcb,
            MCB_TYPE_Z,
            MCB_OWNER_FREE,
            0x30,
            b"FREEBLK ",
        );

        let data_segment =
            allocate(&mut mem, first_mcb, 0x10, 0x2222, 2).expect("last-fit should allocate");

        assert_eq!(
            data_segment, 0x1021,
            "last-fit should allocate from the high end of the free block"
        );
        assert_eq!(
            read_mcb_owner(&mem, first_mcb),
            MCB_OWNER_FREE,
            "lower portion should remain free after high-end split"
        );
        assert_eq!(
            read_mcb_type(&mem, first_mcb),
            MCB_TYPE_M,
            "lower free block should now be a middle block"
        );
        assert_eq!(
            read_mcb_size(&mem, first_mcb),
            0x1F,
            "lower free block size should shrink by the allocation plus its MCB"
        );
        assert_eq!(
            read_mcb_type(&mem, 0x1020),
            MCB_TYPE_Z,
            "new high-end allocation should inherit the original block terminator type"
        );
        assert_eq!(read_mcb_owner(&mem, 0x1020), 0x2222);
        assert_eq!(read_mcb_size(&mem, 0x1020), 0x10);
    }

    #[test]
    fn resize_dos_resizes_umb_block() {
        let mut mem = MockMemory::new(0x200000);
        write_mcb(
            &mut mem,
            0x1000,
            MCB_TYPE_Z,
            MCB_OWNER_FREE,
            0x20,
            b"CONVFREE",
        );
        write_mcb(
            &mut mem,
            UMB_FIRST_MCB_SEGMENT,
            MCB_TYPE_Z,
            0x3333,
            0x20,
            b"UMBALLOC",
        );

        resize_dos(
            &mut mem,
            0x1000,
            Some(UMB_FIRST_MCB_SEGMENT),
            UMB_FIRST_MCB_SEGMENT + 1,
            0x10,
        )
        .expect("UMB block should be found by DOS-aware resize");

        assert_eq!(read_mcb_size(&mem, UMB_FIRST_MCB_SEGMENT), 0x10);
        assert_eq!(read_mcb_owner(&mem, UMB_FIRST_MCB_SEGMENT), 0x3333);
        assert_eq!(
            read_mcb_owner(&mem, UMB_FIRST_MCB_SEGMENT + 0x11),
            MCB_OWNER_FREE
        );
    }

    #[test]
    fn free_process_blocks_dos_reclaims_umb_chain() {
        let mut mem = MockMemory::new(0x200000);
        let owner = 0x4444;

        write_mcb(
            &mut mem,
            0x1000,
            MCB_TYPE_Z,
            MCB_OWNER_FREE,
            0x20,
            b"CONVFREE",
        );
        write_mcb(
            &mut mem,
            UMB_FIRST_MCB_SEGMENT,
            MCB_TYPE_M,
            owner,
            0x08,
            b"UMBPSP  ",
        );
        write_mcb(
            &mut mem,
            UMB_FIRST_MCB_SEGMENT + 0x09,
            MCB_TYPE_Z,
            owner,
            0x08,
            b"UMBAUX  ",
        );

        free_process_blocks_dos(&mut mem, 0x1000, Some(UMB_FIRST_MCB_SEGMENT), owner);

        assert_eq!(read_mcb_owner(&mem, UMB_FIRST_MCB_SEGMENT), MCB_OWNER_FREE);
        assert_eq!(read_mcb_type(&mem, UMB_FIRST_MCB_SEGMENT), MCB_TYPE_Z);
        assert_eq!(read_mcb_size(&mem, UMB_FIRST_MCB_SEGMENT), 0x11);
    }

    #[test]
    fn resize_grow_failure_resizes_to_maximum_and_returns_error() {
        let mut mem = MockMemory::new(0x200000);
        let first_mcb = 0x1000;

        write_mcb(&mut mem, first_mcb, MCB_TYPE_M, 0x2222, 4, b"ALLOC   ");
        write_mcb(&mut mem, 0x1005, MCB_TYPE_Z, MCB_OWNER_FREE, 3, b"FREE    ");

        assert_eq!(
            resize(&mut mem, first_mcb, first_mcb + 1, 9),
            Err((ERR_INSUFFICIENT_MEMORY, 8))
        );
        assert_eq!(read_mcb_size(&mem, first_mcb), 8);
        assert_eq!(read_mcb_type(&mem, first_mcb), MCB_TYPE_Z);
        assert_eq!(read_mcb_owner(&mem, first_mcb), 0x2222);
    }

    #[test]
    fn resize_with_ffff_grows_to_maximum_before_failing() {
        let mut mem = MockMemory::new(0x200000);
        let first_mcb = 0x1000;

        write_mcb(&mut mem, first_mcb, MCB_TYPE_M, 0x2222, 4, b"ALLOC   ");
        write_mcb(&mut mem, 0x1005, MCB_TYPE_Z, MCB_OWNER_FREE, 3, b"FREE    ");

        assert_eq!(
            resize(&mut mem, first_mcb, first_mcb + 1, 0xFFFF),
            Err((ERR_INSUFFICIENT_MEMORY, 8))
        );
        assert_eq!(read_mcb_size(&mem, first_mcb), 8);
        assert_eq!(read_mcb_type(&mem, first_mcb), MCB_TYPE_Z);
    }

    #[test]
    fn largest_available_dos_ignores_high_only_without_linked_umb() {
        let mut mem = MockMemory::new(0x200000);

        write_mcb(
            &mut mem,
            0x1000,
            MCB_TYPE_Z,
            MCB_OWNER_FREE,
            0x20,
            b"CONVFREE",
        );
        write_mcb(
            &mut mem,
            UMB_FIRST_MCB_SEGMENT,
            MCB_TYPE_Z,
            MCB_OWNER_FREE,
            0x40,
            b"UMBFREE ",
        );

        assert_eq!(largest_available_dos(&mem, 0x1000, None, 0x0080), 0x20);
    }
}
