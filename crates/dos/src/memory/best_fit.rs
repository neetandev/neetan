//! Exact best-fit allocator.
//!
//! DOS memory managers commonly query the largest free XMS block and then
//! allocate that exact size. This allocator favors that compatibility contract
//! over constant-time allocation by keeping free regions indexed by address for
//! coalescing and by `(size, offset)` for exact best-fit selection.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Allocation {
    offset: u32,
    allocation_id: u32,
    generation: u32,
}

impl Allocation {
    #[inline(always)]
    pub(crate) fn offset(&self) -> u32 {
        self.offset
    }
}

const ALIGN_SIZE_LOG2: u32 = 2;
const ALIGN_SIZE: u32 = 1 << ALIGN_SIZE_LOG2;
pub(crate) const ALIGN_MASK: u64 = (ALIGN_SIZE as u64) - 1;

const BLOCK_SIZE_MIN: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FreeRegion {
    size: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveAllocation {
    offset: u32,
    size: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AllocationSlot {
    allocation: Option<ActiveAllocation>,
    generation: u32,
}

impl AllocationSlot {
    fn new() -> Self {
        Self {
            allocation: None,
            generation: 0,
        }
    }
}

#[derive(Debug)]
pub(crate) struct BestFitAllocator {
    size: u32,
    free_by_offset: BTreeMap<u32, FreeRegion>,
    free_by_size: BTreeSet<(u32, u32)>,
    active_allocations: Vec<AllocationSlot>,
    reusable_allocation_ids: Vec<u32>,
    active_count: u32,
    total_free_size: u32,
}

impl BestFitAllocator {
    pub(crate) fn new(size: u32) -> Self {
        let aligned_size = (size / ALIGN_SIZE) * ALIGN_SIZE;
        let size = if aligned_size >= BLOCK_SIZE_MIN {
            aligned_size
        } else {
            0
        };

        let mut allocator = Self {
            size,
            free_by_offset: BTreeMap::new(),
            free_by_size: BTreeSet::new(),
            active_allocations: Vec::new(),
            reusable_allocation_ids: Vec::new(),
            active_count: 0,
            total_free_size: 0,
        };
        allocator.reset();
        allocator
    }

    pub(crate) fn reset(&mut self) {
        self.free_by_offset.clear();
        self.free_by_size.clear();
        self.reusable_allocation_ids.clear();
        self.active_count = 0;
        self.total_free_size = 0;

        for (allocation_id, slot) in self.active_allocations.iter_mut().enumerate().rev() {
            slot.allocation = None;
            slot.generation = slot.generation.wrapping_add(1);
            self.reusable_allocation_ids.push(allocation_id as u32);
        }

        if self.size > 0 {
            self.insert_free_region(0, self.size);
        }
    }

    pub(crate) fn allocate(&mut self, size: u32) -> Option<Allocation> {
        let request_size = normalize_allocation_size(size)?;
        let (region_size, region_offset) = self
            .free_by_size
            .range((request_size, 0)..)
            .next()
            .copied()?;

        self.remove_free_region(region_offset);

        let allocation_size = if region_size >= request_size.saturating_add(BLOCK_SIZE_MIN) {
            request_size
        } else {
            region_size
        };
        let remainder_size = region_size - allocation_size;
        if remainder_size > 0 {
            self.insert_free_region(region_offset + allocation_size, remainder_size);
        }

        let allocation = self.create_allocation(region_offset, allocation_size);
        Some(allocation)
    }

    pub(crate) fn deallocate(&mut self, allocation: Allocation) {
        let Some(slot_index) = self.validate_allocation_index(allocation) else {
            return;
        };

        let slot = &mut self.active_allocations[slot_index];
        let active = slot
            .allocation
            .take()
            .expect("validated allocation disappeared");
        slot.generation = slot.generation.wrapping_add(1);
        self.reusable_allocation_ids.push(allocation.allocation_id);
        self.active_count -= 1;

        self.insert_coalesced_free_region(active.offset, active.size);
    }

    pub(crate) fn reallocate_in_place(
        &mut self,
        allocation: Allocation,
        size: u32,
    ) -> Option<Allocation> {
        let request_size = normalize_allocation_size(size)?;
        let slot_index = self.validate_allocation_index(allocation)?;
        let active = self.active_allocations[slot_index].allocation?;

        if request_size == active.size {
            return Some(allocation);
        }

        if request_size < active.size {
            let remainder_size = active.size - request_size;
            if remainder_size >= BLOCK_SIZE_MIN {
                self.active_allocations[slot_index].allocation = Some(ActiveAllocation {
                    offset: active.offset,
                    size: request_size,
                });
                self.insert_coalesced_free_region(active.offset + request_size, remainder_size);
            }
            return Some(allocation);
        }

        let next_offset = active.offset + active.size;
        let next_region = self.free_by_offset.get(&next_offset).copied()?;
        let growth_size = request_size - active.size;
        if next_region.size < growth_size {
            return None;
        }

        self.remove_free_region(next_offset);
        let remainder_size = next_region.size - growth_size;
        let allocation_size = if remainder_size >= BLOCK_SIZE_MIN {
            self.insert_free_region(next_offset + growth_size, remainder_size);
            request_size
        } else {
            active.size + next_region.size
        };

        self.active_allocations[slot_index].allocation = Some(ActiveAllocation {
            offset: active.offset,
            size: allocation_size,
        });

        Some(allocation)
    }

    pub(crate) fn total_free_size(&self) -> u32 {
        self.total_free_size
    }

    pub(crate) fn largest_free_block_size(&self) -> u32 {
        self.free_by_size
            .last()
            .map(|(size, _offset)| *size)
            .unwrap_or(0)
    }

    #[cfg(test)]
    pub(crate) fn allocation_size(&self, allocation: Allocation) -> Option<u32> {
        self.validate_allocation_index(allocation)
            .and_then(|slot_index| self.active_allocations[slot_index].allocation)
            .map(|allocation| allocation.size)
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.active_count == 0
    }

    fn create_allocation(&mut self, offset: u32, size: u32) -> Allocation {
        let allocation_id = if let Some(allocation_id) = self.reusable_allocation_ids.pop() {
            allocation_id
        } else {
            let allocation_id = self.active_allocations.len() as u32;
            self.active_allocations.push(AllocationSlot::new());
            allocation_id
        };

        let slot = &mut self.active_allocations[allocation_id as usize];
        debug_assert!(
            slot.allocation.is_none(),
            "allocation slot is already active"
        );
        slot.allocation = Some(ActiveAllocation { offset, size });
        self.active_count += 1;

        Allocation {
            offset,
            allocation_id,
            generation: slot.generation,
        }
    }

    fn validate_allocation_index(&self, allocation: Allocation) -> Option<usize> {
        let slot_index = allocation.allocation_id as usize;
        let Some(slot) = self.active_allocations.get(slot_index) else {
            debug_assert!(false, "invalid allocation metadata");
            return None;
        };

        debug_assert_eq!(
            slot.generation, allocation.generation,
            "stale allocation metadata"
        );
        let Some(active) = slot.allocation else {
            debug_assert!(false, "double free detected");
            return None;
        };
        debug_assert_eq!(
            active.offset, allocation.offset,
            "allocation offset mismatch"
        );

        if slot.generation != allocation.generation || active.offset != allocation.offset {
            return None;
        }

        Some(slot_index)
    }

    fn insert_coalesced_free_region(&mut self, mut offset: u32, mut size: u32) {
        if size == 0 {
            return;
        }

        if let Some((&previous_offset, &previous_region)) =
            self.free_by_offset.range(..offset).next_back()
        {
            let previous_end = previous_offset + previous_region.size;
            debug_assert!(
                previous_end <= offset,
                "free region overlaps deallocated range"
            );
            if previous_end == offset {
                self.remove_free_region(previous_offset);
                offset = previous_offset;
                size += previous_region.size;
            }
        }

        if let Some((&next_offset, &next_region)) = self.free_by_offset.range(offset..).next() {
            let end = offset + size;
            debug_assert!(end <= next_offset, "free region overlaps deallocated range");
            if end == next_offset {
                self.remove_free_region(next_offset);
                size += next_region.size;
            }
        }

        self.insert_free_region(offset, size);
    }

    fn insert_free_region(&mut self, offset: u32, size: u32) {
        debug_assert!(size >= BLOCK_SIZE_MIN, "free region is too small");
        debug_assert_eq!(offset & (ALIGN_SIZE - 1), 0, "unaligned free offset");
        debug_assert_eq!(size & (ALIGN_SIZE - 1), 0, "unaligned free size");
        debug_assert!(
            offset.checked_add(size).is_some_and(|end| end <= self.size),
            "free region exceeds allocator pool"
        );

        let old_region = self.free_by_offset.insert(offset, FreeRegion { size });
        debug_assert!(old_region.is_none(), "duplicate free region offset");
        let inserted = self.free_by_size.insert((size, offset));
        debug_assert!(inserted, "duplicate free region size index");
        self.total_free_size += size;
    }

    fn remove_free_region(&mut self, offset: u32) -> FreeRegion {
        let region = self
            .free_by_offset
            .remove(&offset)
            .expect("free region missing from address index");
        let removed = self.free_by_size.remove(&(region.size, offset));
        debug_assert!(removed, "free region missing from size index");
        self.total_free_size -= region.size;
        region
    }

    #[cfg(test)]
    fn assert_invariants(&self) {
        let mut computed_free_size = 0u32;
        let mut previous_free_end = None;
        for (&offset, &region) in &self.free_by_offset {
            debug_assert!(region.size >= BLOCK_SIZE_MIN);
            debug_assert_eq!(offset & (ALIGN_SIZE - 1), 0);
            debug_assert_eq!(region.size & (ALIGN_SIZE - 1), 0);
            let end = offset + region.size;
            debug_assert!(end <= self.size);

            if let Some(previous_end) = previous_free_end {
                debug_assert!(
                    previous_end < offset,
                    "adjacent free regions were not coalesced"
                );
            }
            previous_free_end = Some(end);

            debug_assert!(
                self.free_by_size.contains(&(region.size, offset)),
                "free region missing from size index"
            );
            computed_free_size += region.size;
        }
        debug_assert_eq!(computed_free_size, self.total_free_size);
        debug_assert_eq!(self.free_by_offset.len(), self.free_by_size.len());

        for &(size, offset) in &self.free_by_size {
            let region = self
                .free_by_offset
                .get(&offset)
                .expect("size-index entry points to missing free region");
            debug_assert_eq!(region.size, size);
        }

        let mut active_ranges = Vec::new();
        let mut computed_active_count = 0u32;
        let mut computed_active_size = 0u32;
        for slot in &self.active_allocations {
            if let Some(allocation) = slot.allocation {
                computed_active_count += 1;
                computed_active_size += allocation.size;
                debug_assert!(allocation.size >= BLOCK_SIZE_MIN);
                debug_assert_eq!(allocation.offset & (ALIGN_SIZE - 1), 0);
                debug_assert_eq!(allocation.size & (ALIGN_SIZE - 1), 0);
                debug_assert!(allocation.offset + allocation.size <= self.size);
                active_ranges.push((allocation.offset, allocation.size));
            }
        }
        debug_assert_eq!(computed_active_count, self.active_count);
        debug_assert_eq!(computed_active_size + computed_free_size, self.size);

        active_ranges.sort_unstable_by_key(|&(offset, _size)| offset);
        let mut previous_active_end = None;
        for &(offset, size) in &active_ranges {
            if let Some(previous_end) = previous_active_end {
                debug_assert!(previous_end <= offset, "active allocations overlap");
            }
            previous_active_end = Some(offset + size);
        }

        let mut free_iter = self
            .free_by_offset
            .iter()
            .map(|(&offset, &region)| (offset, region.size))
            .peekable();
        for &(active_offset, active_size) in &active_ranges {
            let active_end = active_offset + active_size;
            while let Some(&(free_offset, free_size)) = free_iter.peek() {
                let free_end = free_offset + free_size;
                if free_end <= active_offset {
                    free_iter.next();
                    continue;
                }
                debug_assert!(
                    active_end <= free_offset,
                    "active allocation overlaps free region"
                );
                break;
            }
        }

        let expected_largest = self
            .free_by_offset
            .values()
            .map(|region| region.size)
            .max()
            .unwrap_or(0);
        debug_assert_eq!(self.largest_free_block_size(), expected_largest);
        if expected_largest > 0 {
            debug_assert_eq!(
                normalize_allocation_size(expected_largest),
                Some(expected_largest),
                "largest free block is not directly allocatable"
            );
        }
    }
}

fn normalize_allocation_size(size: u32) -> Option<u32> {
    let aligned_size = size.checked_add(ALIGN_SIZE - 1)? & !(ALIGN_SIZE - 1);
    Some(aligned_size.max(BLOCK_SIZE_MIN))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_allocator_initialization() {
        let allocator = BestFitAllocator::new(1024);
        assert!(allocator.is_empty());
        assert_eq!(allocator.total_free_size(), 1024);
        assert_eq!(allocator.largest_free_block_size(), 1024);
        allocator.assert_invariants();
    }

    #[test]
    fn test_zero_sized_pool_initialization() {
        let allocator = BestFitAllocator::new(0);
        assert!(allocator.is_empty());
        assert_eq!(allocator.total_free_size(), 0);
        assert_eq!(allocator.largest_free_block_size(), 0);
        allocator.assert_invariants();
    }

    #[test]
    fn test_unaligned_pool_size_is_truncated() {
        let mut allocator = BestFitAllocator::new(1025);

        assert_allocator_state(&allocator, 1024, 1024);

        let allocation = allocator.allocate(1024).unwrap();
        assert_eq!(allocation.offset(), 0);
        assert_eq!(allocator.allocation_size(allocation), Some(1024));
        assert_allocator_state(&allocator, 0, 0);
    }

    #[test]
    fn test_tiny_pool_sizes() {
        for size in 0..BLOCK_SIZE_MIN {
            let mut allocator = BestFitAllocator::new(size);
            assert!(allocator.allocate(1).is_none());
            assert_allocator_state(&allocator, 0, 0);
        }

        let mut allocator = BestFitAllocator::new(BLOCK_SIZE_MIN);
        let allocation = allocator.allocate(1).unwrap();
        assert_eq!(allocator.allocation_size(allocation), Some(BLOCK_SIZE_MIN));
        assert_allocator_state(&allocator, 0, 0);
    }

    #[test]
    fn test_full_pool_allocation() {
        let mut allocator = BestFitAllocator::new(1024);
        let allocation = allocator.allocate(1024).unwrap();

        assert_eq!(allocation.offset(), 0);
        assert_eq!(allocator.allocation_size(allocation), Some(1024));
        assert_eq!(allocator.total_free_size(), 0);
        assert_eq!(allocator.largest_free_block_size(), 0);
        allocator.assert_invariants();
    }

    #[test]
    fn test_allocate_exact_largest_free_block() {
        let mut allocator = BestFitAllocator::new(512);
        let _guard1 = allocator.allocate(128).unwrap();
        let middle = allocator.allocate(64).unwrap();
        let _guard2 = allocator.allocate(128).unwrap();
        let tail = allocator.allocate(64).unwrap();

        allocator.deallocate(middle);
        allocator.deallocate(tail);

        let largest = allocator.largest_free_block_size();
        let allocation = allocator.allocate(largest).unwrap();
        assert_eq!(allocator.allocation_size(allocation), Some(largest));
        allocator.assert_invariants();
    }

    #[test]
    fn test_best_fit_choice_among_multiple_free_regions() {
        let mut allocator = BestFitAllocator::new(1024);
        let _guard1 = allocator.allocate(64).unwrap();
        let large1 = allocator.allocate(256).unwrap();
        let _guard2 = allocator.allocate(64).unwrap();
        let best = allocator.allocate(128).unwrap();
        let best_offset = best.offset();
        let _guard3 = allocator.allocate(64).unwrap();
        let large2 = allocator.allocate(256).unwrap();
        let _guard4 = allocator.allocate(64).unwrap();

        allocator.deallocate(large1);
        allocator.deallocate(best);
        allocator.deallocate(large2);

        let allocation = allocator.allocate(120).unwrap();
        assert_eq!(allocation.offset(), best_offset);
        assert_eq!(allocator.allocation_size(allocation), Some(128));
        allocator.assert_invariants();
    }

    #[test]
    fn test_split_on_allocation() {
        let mut allocator = BestFitAllocator::new(128);
        let allocation = allocator.allocate(32).unwrap();

        assert_eq!(allocator.allocation_size(allocation), Some(32));
        assert_eq!(allocator.total_free_size(), 96);
        assert_eq!(allocator.largest_free_block_size(), 96);
        allocator.assert_invariants();
    }

    #[test]
    fn test_unaligned_allocation_requests_are_normalized() {
        let cases = [(1, 16), (15, 16), (17, 20)];

        for (request_size, expected_size) in cases {
            let mut allocator = BestFitAllocator::new(64);
            let allocation = allocator.allocate(request_size).unwrap();

            assert_eq!(allocation.offset(), 0);
            assert_eq!(allocator.allocation_size(allocation), Some(expected_size));
            assert_allocator_state(&allocator, 64 - expected_size, 64 - expected_size);
        }
    }

    #[test]
    fn test_oversized_allocation_failure_does_not_mutate_state() {
        let mut allocator = BestFitAllocator::new(128);
        let first = allocator.allocate(32).unwrap();
        let second = allocator.allocate(32).unwrap();
        let third = allocator.allocate(32).unwrap();
        let _fourth = allocator.allocate(32).unwrap();

        allocator.deallocate(first);
        allocator.deallocate(third);
        assert_allocator_state(&allocator, 64, 32);

        assert!(allocator.allocate(48).is_none());
        assert_allocator_state(&allocator, 64, 32);

        allocator.deallocate(second);
        assert_allocator_state(&allocator, 96, 96);
    }

    #[test]
    fn test_overflow_allocation_request_does_not_mutate_state() {
        let mut allocator = BestFitAllocator::new(64);

        assert!(allocator.allocate(u32::MAX).is_none());
        assert_allocator_state(&allocator, 64, 64);
    }

    #[test]
    fn test_coalesce_with_previous_free_region() {
        let mut allocator = BestFitAllocator::new(96);
        let first = allocator.allocate(32).unwrap();
        let second = allocator.allocate(32).unwrap();
        let _third = allocator.allocate(32).unwrap();

        allocator.deallocate(first);
        allocator.deallocate(second);

        assert_eq!(allocator.largest_free_block_size(), 64);
        allocator.assert_invariants();
    }

    #[test]
    fn test_coalesce_with_next_free_region() {
        let mut allocator = BestFitAllocator::new(96);
        let first = allocator.allocate(32).unwrap();
        let second = allocator.allocate(32).unwrap();
        let _third = allocator.allocate(32).unwrap();

        allocator.deallocate(second);
        allocator.deallocate(first);

        assert_eq!(allocator.largest_free_block_size(), 64);
        allocator.assert_invariants();
    }

    #[test]
    fn test_coalesce_with_previous_and_next_free_regions() {
        let mut allocator = BestFitAllocator::new(128);
        let first = allocator.allocate(32).unwrap();
        let second = allocator.allocate(32).unwrap();
        let third = allocator.allocate(32).unwrap();
        let _fourth = allocator.allocate(32).unwrap();

        allocator.deallocate(first);
        allocator.deallocate(third);
        allocator.deallocate(second);

        assert_eq!(allocator.largest_free_block_size(), 96);
        allocator.assert_invariants();
    }

    #[test]
    fn test_shrinking_in_place_reallocation() {
        let mut allocator = BestFitAllocator::new(128);
        let allocation = allocator.allocate(128).unwrap();

        let resized = allocator.reallocate_in_place(allocation, 64).unwrap();

        assert_eq!(resized.offset(), allocation.offset());
        assert_eq!(allocator.allocation_size(resized), Some(64));
        assert_eq!(allocator.largest_free_block_size(), 64);
        allocator.assert_invariants();
    }

    #[test]
    fn test_shrink_keeps_allocation_when_remainder_is_too_small() {
        let mut allocator = BestFitAllocator::new(64);
        let allocation = allocator.allocate(64).unwrap();

        let resized = allocator.reallocate_in_place(allocation, 52).unwrap();

        assert_eq!(resized.offset(), allocation.offset());
        assert_eq!(allocator.allocation_size(resized), Some(64));
        assert_allocator_state(&allocator, 0, 0);
    }

    #[test]
    fn test_growing_in_place_reallocation_into_adjacent_free_region() {
        let mut allocator = BestFitAllocator::new(96);
        let first = allocator.allocate(32).unwrap();
        let second = allocator.allocate(32).unwrap();
        let _third = allocator.allocate(32).unwrap();
        allocator.deallocate(second);

        let resized = allocator.reallocate_in_place(first, 64).unwrap();

        assert_eq!(resized.offset(), first.offset());
        assert_eq!(allocator.allocation_size(resized), Some(64));
        assert_eq!(allocator.total_free_size(), 0);
        allocator.assert_invariants();
    }

    #[test]
    fn test_growing_in_place_absorbs_small_remainder() {
        let mut allocator = BestFitAllocator::new(80);
        let first = allocator.allocate(32).unwrap();
        let second = allocator.allocate(32).unwrap();
        allocator.deallocate(second);

        let resized = allocator.reallocate_in_place(first, 68).unwrap();

        assert_eq!(resized.offset(), first.offset());
        assert_eq!(allocator.allocation_size(resized), Some(80));
        assert_allocator_state(&allocator, 0, 0);
    }

    #[test]
    fn test_growing_in_place_reallocation_fails_when_next_region_is_used_or_too_small() {
        let mut next_used = BestFitAllocator::new(96);
        let first = next_used.allocate(32).unwrap();
        let _second = next_used.allocate(32).unwrap();
        let _third = next_used.allocate(32).unwrap();

        assert!(next_used.reallocate_in_place(first, 64).is_none());
        assert_eq!(next_used.allocation_size(first), Some(32));
        assert_allocator_state(&next_used, 0, 0);

        let mut too_small = BestFitAllocator::new(96);
        let first = too_small.allocate(32).unwrap();
        let second = too_small.allocate(16).unwrap();
        let _third = too_small.allocate(48).unwrap();
        too_small.deallocate(second);

        assert!(too_small.reallocate_in_place(first, 64).is_none());
        assert_eq!(too_small.allocation_size(first), Some(32));
        assert_allocator_state(&too_small, 16, 16);
    }

    #[test]
    fn test_overflow_reallocation_request_does_not_mutate_state() {
        let mut allocator = BestFitAllocator::new(64);
        let allocation = allocator.allocate(32).unwrap();

        assert!(
            allocator
                .reallocate_in_place(allocation, u32::MAX)
                .is_none()
        );
        assert_eq!(allocator.allocation_size(allocation), Some(32));
        assert_allocator_state(&allocator, 32, 32);
    }

    #[test]
    fn test_zero_size_allocation_uses_minimum_block_size() {
        let mut allocator = BestFitAllocator::new(64);
        let allocation = allocator.allocate(0).unwrap();

        assert_eq!(allocation.offset(), 0);
        assert_eq!(allocator.allocation_size(allocation), Some(BLOCK_SIZE_MIN));
        assert_eq!(allocator.total_free_size(), 48);
        allocator.assert_invariants();
    }

    #[test]
    fn test_stale_allocation_metadata_does_not_corrupt_state() {
        let mut allocator = BestFitAllocator::new(64);
        let stale = allocator.allocate(16).unwrap();
        allocator.deallocate(stale);
        assert_allocator_state(&allocator, 64, 64);

        let current = allocator.allocate(16).unwrap();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            allocator.deallocate(stale);
        }));

        assert_eq!(allocator.allocation_size(current), Some(16));
        assert_allocator_state(&allocator, 48, 48);

        allocator.deallocate(current);
        assert_allocator_state(&allocator, 64, 64);
    }

    #[test]
    fn test_double_free_does_not_corrupt_state() {
        let mut allocator = BestFitAllocator::new(64);
        let allocation = allocator.allocate(16).unwrap();
        allocator.deallocate(allocation);
        assert_allocator_state(&allocator, 64, 64);

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            allocator.deallocate(allocation);
        }));

        assert_allocator_state(&allocator, 64, 64);
    }

    #[test]
    fn test_reset_restores_pool_and_invalidates_old_allocations() {
        let mut allocator = BestFitAllocator::new(128);
        let first = allocator.allocate(32).unwrap();
        let _second = allocator.allocate(32).unwrap();

        allocator.reset();
        assert!(allocator.is_empty());
        assert_allocator_state(&allocator, 128, 128);

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            allocator.deallocate(first);
        }));

        assert!(allocator.is_empty());
        assert_allocator_state(&allocator, 128, 128);

        let allocation = allocator.allocate(128).unwrap();
        assert_eq!(allocator.allocation_size(allocation), Some(128));
        assert_allocator_state(&allocator, 0, 0);
    }

    #[test]
    fn test_randomized_allocate_free_stress() {
        let mut allocator = BestFitAllocator::new(4096);
        let mut allocations = Vec::new();
        let mut seed = 0x4E45_4554_u64;

        for _ in 0..5000 {
            let random = next_random(&mut seed);
            if allocations.is_empty() || random % 100 < 60 {
                let size = next_random(&mut seed) % 512;
                if let Some(allocation) = allocator.allocate(size) {
                    allocations.push(allocation);
                }
            } else {
                let index = (next_random(&mut seed) as usize) % allocations.len();
                let allocation = allocations.swap_remove(index);
                allocator.deallocate(allocation);
            }

            allocator.assert_invariants();
            let largest = allocator.largest_free_block_size();
            if largest > 0 {
                let allocation = allocator.allocate(largest).unwrap();
                assert_eq!(allocator.allocation_size(allocation), Some(largest));
                allocator.deallocate(allocation);
            }
        }

        for allocation in allocations {
            allocator.deallocate(allocation);
        }

        assert!(allocator.is_empty());
        assert_eq!(allocator.total_free_size(), 4096);
        assert_eq!(allocator.largest_free_block_size(), 4096);
        allocator.assert_invariants();
    }

    #[test]
    fn test_randomized_model_stress() {
        const POOL_SIZE: u32 = 4096;
        let seeds = [
            0x4E45_4554_u64,
            0x1234_5678_u64,
            0xDEAD_BEEF_u64,
            0xA11C_A70C_u64,
        ];

        for mut seed in seeds {
            let mut allocator = BestFitAllocator::new(POOL_SIZE);
            let mut allocations = Vec::new();

            for _ in 0..1500 {
                let random = next_random(&mut seed);
                if allocations.is_empty() || random % 100 < 62 {
                    let request_size = next_random(&mut seed) % 768;
                    let expected_request_size = normalize_allocation_size(request_size);
                    let allocation = allocator.allocate(request_size);

                    if let Some(allocation) = allocation {
                        let allocation_size = allocator.allocation_size(allocation).unwrap();
                        allocations.push(ModeledAllocation {
                            allocation,
                            offset: allocation.offset(),
                            size: allocation_size,
                        });
                    } else if let Some(expected_request_size) = expected_request_size {
                        assert!(
                            modeled_largest_free_block(&allocations, POOL_SIZE)
                                < expected_request_size
                        );
                    }
                } else {
                    let index = (next_random(&mut seed) as usize) % allocations.len();
                    let allocation = allocations.swap_remove(index);
                    allocator.deallocate(allocation.allocation);
                }

                assert_matches_model(&mut allocator, &allocations, POOL_SIZE);
            }

            for allocation in allocations {
                allocator.deallocate(allocation.allocation);
            }

            assert!(allocator.is_empty());
            assert_allocator_state(&allocator, POOL_SIZE, POOL_SIZE);
        }
    }

    #[derive(Debug)]
    struct ModeledAllocation {
        allocation: Allocation,
        offset: u32,
        size: u32,
    }

    fn assert_allocator_state(
        allocator: &BestFitAllocator,
        expected_total_free_size: u32,
        expected_largest_free_block_size: u32,
    ) {
        assert_eq!(allocator.total_free_size(), expected_total_free_size);
        assert_eq!(
            allocator.largest_free_block_size(),
            expected_largest_free_block_size
        );
        allocator.assert_invariants();
    }

    fn assert_matches_model(
        allocator: &mut BestFitAllocator,
        allocations: &[ModeledAllocation],
        pool_size: u32,
    ) {
        let modeled_total_free_size = modeled_total_free_size(allocations, pool_size);
        let modeled_largest_free_block = modeled_largest_free_block(allocations, pool_size);

        assert_allocator_state(
            allocator,
            modeled_total_free_size,
            modeled_largest_free_block,
        );

        if modeled_largest_free_block > 0 {
            let allocation = allocator.allocate(modeled_largest_free_block).unwrap();
            assert_eq!(
                allocator.allocation_size(allocation),
                Some(modeled_largest_free_block)
            );
            allocator.deallocate(allocation);
            assert_allocator_state(
                allocator,
                modeled_total_free_size,
                modeled_largest_free_block,
            );
        }
    }

    fn modeled_total_free_size(allocations: &[ModeledAllocation], pool_size: u32) -> u32 {
        let used_size: u32 = allocations.iter().map(|allocation| allocation.size).sum();
        pool_size - used_size
    }

    fn modeled_largest_free_block(allocations: &[ModeledAllocation], pool_size: u32) -> u32 {
        let mut ranges: Vec<(u32, u32)> = allocations
            .iter()
            .map(|allocation| (allocation.offset, allocation.size))
            .collect();
        ranges.sort_unstable_by_key(|&(offset, _size)| offset);

        let mut largest = 0;
        let mut next_free_offset = 0;
        for (offset, size) in ranges {
            assert!(offset >= next_free_offset, "modeled allocations overlap");
            largest = largest.max(offset - next_free_offset);
            next_free_offset = offset + size;
        }
        largest.max(pool_size - next_free_offset)
    }

    fn next_random(seed: &mut u64) -> u32 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (*seed >> 32) as u32
    }
}
