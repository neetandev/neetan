//! Unified EMS/XMS/UMB memory manager.
//!
//! All EMS/XMS data lives in the machine's extended RAM (accessed via
//! `MemoryAccess` at addresses 0x100000 + offset). The extended-memory allocator
//! tracks which byte ranges within extended RAM are allocated.

use super::best_fit::{ALIGN_MASK, Allocation, BestFitAllocator};
use crate::{MemoryAccess, memory, tables::*};

pub(crate) const EXTENDED_RAM_BASE: u32 = 0x100000;

pub(crate) struct EmsMoveParams {
    pub region_length: u32,
    pub src_type: u8,
    pub src_handle: u16,
    pub src_offset: u16,
    pub src_seg_page: u16,
    pub dst_type: u8,
    pub dst_handle: u16,
    pub dst_offset: u16,
    pub dst_seg_page: u16,
}
const EMS_PAGE_FRAME_BASE: u32 = 0xC0000;
const EMS_PAGE_SIZE: u32 = 0x4000;
const MAX_EMS_HANDLES: usize = 255;
const MAX_XMS_HANDLES: usize = 128;
const PHYSICAL_PAGES: usize = 4;
const HMA_POOL_RESERVATION_BYTES: u32 = 64 * 1024;
const EMS_OS_RESERVED_HANDLE: usize = 0;

#[derive(Clone, Copy)]
enum EmsTransferRegion {
    Conventional { linear_address: u32 },
    Expanded { handle: u16, region_offset: u32 },
}

#[derive(Clone, Copy)]
pub(crate) struct EmsPageMapCallContext {
    pub(crate) segphys_mode: u8,
    pub(crate) handle: u16,
    pub(crate) old_len: u8,
    pub(crate) old_map_addr: u32,
}

fn physical_page_for_segment(segment: u16) -> Option<usize> {
    match segment {
        0xC000 => Some(0),
        0xC400 => Some(1),
        0xC800 => Some(2),
        0xCC00 => Some(3),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EmsMapping {
    pub(crate) handle: u16,
    pub(crate) logical_page: u16,
    pub(crate) allocation_offset: u32,
}

struct EmsHandle {
    active: bool,
    pages: Vec<Allocation>,
    name: [u8; 8],
    save_context: Option<[Option<EmsMapping>; PHYSICAL_PAGES]>,
}

impl EmsHandle {
    fn new_inactive() -> Self {
        Self {
            active: false,
            pages: Vec::new(),
            name: [0u8; 8],
            save_context: None,
        }
    }
}

struct XmsHandle {
    active: bool,
    allocation: Option<Allocation>,
    size_kb: u32,
    lock_count: u8,
}

impl XmsHandle {
    fn new_inactive() -> Self {
        Self {
            active: false,
            allocation: None,
            size_kb: 0,
            lock_count: 0,
        }
    }
}

pub(crate) struct MemoryManager {
    allocator: BestFitAllocator,
    extended_memory_size: u32,
    allocator_base_offset: u32,
    hma_exists: bool,

    ems_enabled: bool,
    ems_handles: Vec<EmsHandle>,
    ems_page_mapping: [Option<EmsMapping>; PHYSICAL_PAGES],

    xms_enabled: bool,
    xms_32_enabled: bool,
    xms_handles: Vec<XmsHandle>,
    hma_allocated: bool,

    umb_enabled: bool,

    // XMS A20 state. PC-98 hardware has no A20 line (A20 is permanently
    // enabled), so these fields only track API-level state so that
    // applications see the correct nesting-counter semantics required by
    // XMS 3.0 A20 Management. No machine-level gate is driven.
    a20_global_enabled: bool,
    a20_local_enable_count: u32,

    // /HMAMIN= threshold. Request HMA (function 01h) rejects size requests
    // below this KB threshold, letting a larger consumer win the HMA
    // (XMS 3.0 Prioritizing HMA Usage). 0 = first-come-first-served.
    hmamin_kb: u16,

    ems_os_access_key: Option<u32>,
    ems_os_functions_enabled: bool,
    ems_os_next_access_key: u32,
    ems_alt_map_context_save_area: Option<(u16, u16)>,
    ems_page_map_call_stack: Vec<EmsPageMapCallContext>,
}

impl MemoryManager {
    pub(crate) fn new(
        extended_memory_size: u32,
        ems_enabled: bool,
        xms_enabled: bool,
        xms_32_enabled: bool,
        mem: &mut dyn MemoryAccess,
    ) -> Self {
        let hma_exists = xms_enabled && extended_memory_size >= HMA_POOL_RESERVATION_BYTES;
        let allocator_base_offset = if hma_exists {
            HMA_POOL_RESERVATION_BYTES
        } else {
            0
        };
        let allocator_size = extended_memory_size.saturating_sub(allocator_base_offset);
        let allocator = BestFitAllocator::new(allocator_size);

        if ems_enabled {
            mem.enable_ems_page_frame();
        }

        let umb_enabled = ems_enabled || xms_enabled;
        if umb_enabled {
            mem.enable_umb_region();
            memory::write_mcb(
                mem,
                UMB_FIRST_MCB_SEGMENT,
                0x5A,
                MCB_OWNER_FREE,
                UMB_TOTAL_PARAGRAPHS,
                b"\0\0\0\0\0\0\0\0",
            );
        }

        let mut ems_handles = Vec::with_capacity(MAX_EMS_HANDLES);
        for _ in 0..MAX_EMS_HANDLES {
            ems_handles.push(EmsHandle::new_inactive());
        }
        ems_handles[EMS_OS_RESERVED_HANDLE].active = true;

        let mut xms_handles = Vec::with_capacity(MAX_XMS_HANDLES);
        for _ in 0..MAX_XMS_HANDLES {
            xms_handles.push(XmsHandle::new_inactive());
        }

        Self {
            allocator,
            extended_memory_size,
            allocator_base_offset,
            hma_exists,
            ems_enabled,
            ems_handles,
            ems_page_mapping: [None; PHYSICAL_PAGES],
            xms_enabled,
            xms_32_enabled,
            xms_handles,
            hma_allocated: false,
            umb_enabled,
            a20_global_enabled: false,
            a20_local_enable_count: 0,
            hmamin_kb: 0,
            ems_os_access_key: None,
            ems_os_functions_enabled: true,
            ems_os_next_access_key: 0x4E45_4554,
            ems_alt_map_context_save_area: None,
            ems_page_map_call_stack: Vec::new(),
        }
    }

    /// Sets the /HMAMIN= threshold (in KB). Consulted by `xms_request_hma`
    /// to prioritize HMA for larger consumers.
    pub(crate) fn set_hmamin_kb(&mut self, hmamin_kb: u16) {
        self.hmamin_kb = hmamin_kb;
    }

    /// Function 03h: Global Enable A20. Sets the global flag; reports
    /// success. On PC-98 the physical line is always enabled so there is
    /// nothing to toggle.
    pub(crate) fn xms_global_enable_a20(&mut self) {
        self.a20_global_enabled = true;
    }

    /// Function 04h: Global Disable A20. Fails with 0x94 if a local enable
    /// is still outstanding; otherwise clears the global flag.
    pub(crate) fn xms_global_disable_a20(&mut self) -> Result<(), u8> {
        if self.a20_local_enable_count > 0 {
            return Err(0x94);
        }
        self.a20_global_enabled = false;
        Ok(())
    }

    /// Function 05h: Local Enable A20. Increments the nesting counter.
    pub(crate) fn xms_local_enable_a20(&mut self) {
        self.a20_local_enable_count = self.a20_local_enable_count.saturating_add(1);
    }

    /// Function 06h: Local Disable A20. Fails with 0x94 if the counter is
    /// already zero; otherwise decrements it.
    pub(crate) fn xms_local_disable_a20(&mut self) -> Result<(), u8> {
        if self.a20_local_enable_count == 0 {
            return Err(0x94);
        }
        self.a20_local_enable_count -= 1;
        Ok(())
    }

    /// Function 07h: Query A20. PC-98 has no A20 gate so the physical line
    /// is always enabled, but XMS still exposes a virtual visible state.
    pub(crate) fn xms_query_a20(&self) -> bool {
        self.a20_global_enabled || self.a20_local_enable_count > 0
    }

    fn extended_pool_linear_address(&self, allocation_offset: u32) -> u32 {
        EXTENDED_RAM_BASE + self.allocator_base_offset + allocation_offset
    }

    fn allocator_total_bytes(&self) -> u32 {
        self.extended_memory_size
            .saturating_sub(self.allocator_base_offset)
    }

    fn next_ems_os_access_key(&mut self) -> u32 {
        let key = self.ems_os_next_access_key;
        self.ems_os_next_access_key = self.ems_os_next_access_key.wrapping_add(1);
        if self.ems_os_next_access_key == 0 {
            self.ems_os_next_access_key = 1;
        }
        key
    }

    pub(crate) fn hma_exists(&self) -> bool {
        self.hma_exists
    }

    pub(crate) fn ems_os_functions_enabled(&self) -> bool {
        self.ems_os_functions_enabled
    }

    pub(crate) fn ems_alt_map_context_save_area(&self) -> Option<(u16, u16)> {
        self.ems_alt_map_context_save_area
    }

    pub(crate) fn ems_set_alt_map_context_save_area(&mut self, save_area: Option<(u16, u16)>) {
        self.ems_alt_map_context_save_area = save_area;
    }

    pub(crate) fn ems_push_page_map_call_context(&mut self, context: EmsPageMapCallContext) {
        self.ems_page_map_call_stack.push(context);
    }

    pub(crate) fn ems_pop_page_map_call_context(&mut self) -> Option<EmsPageMapCallContext> {
        self.ems_page_map_call_stack.pop()
    }

    fn total_ems_pages(&self) -> u16 {
        (self.allocator_total_bytes() / EMS_PAGE_SIZE) as u16
    }

    fn allocated_ems_pages(&self) -> u16 {
        let mut count: u16 = 0;
        for handle in &self.ems_handles {
            if handle.active {
                count += handle.pages.len() as u16;
            }
        }
        count
    }

    fn find_free_ems_handle(&self) -> Option<u16> {
        for (i, handle) in self.ems_handles.iter().enumerate().skip(1) {
            if !handle.active {
                return Some(i as u16);
            }
        }
        None
    }

    fn find_free_xms_handle(&self) -> Option<u16> {
        for (i, handle) in self.xms_handles.iter().enumerate() {
            if !handle.active {
                return Some(i as u16);
            }
        }
        None
    }

    fn free_xms_handles_count(&self) -> u16 {
        self.xms_handles.iter().filter(|h| !h.active).count() as u16
    }

    fn apply_ems_page_frame_slot_mapping(&self, physical: usize, mem: &mut dyn MemoryAccess) {
        let backing_linear_addr = self.ems_page_mapping[physical]
            .map(|mapping| self.extended_pool_linear_address(mapping.allocation_offset));
        mem.map_ems_page_frame_slot(physical as u8, backing_linear_addr);
    }

    fn xms_vdisk_conflict_present(&self) -> bool {
        // XMS defines error 81h when VDISK occupies the HMA. VDISK support is not
        // historically relevant for the PC-98, so this path stays intentionally disabled.
        false
    }

    pub(crate) fn ems_status(&self) -> u8 {
        if self.ems_enabled { 0x00 } else { 0x84 }
    }

    pub(crate) fn ems_page_frame_segment(&self) -> u16 {
        EMS_PAGE_FRAME_SEGMENT
    }

    pub(crate) fn ems_unallocated_pages(&self) -> (u16, u16) {
        let total = self.total_ems_pages();
        let allocated = self.allocated_ems_pages();
        (total.saturating_sub(allocated), total)
    }

    pub(crate) fn ems_allocate_pages(&mut self, count: u16) -> Result<u16, u8> {
        if count == 0 {
            return Err(0x89);
        }
        let handle_index = self.find_free_ems_handle().ok_or(0x85u8)?;
        // EMS 4.0 Function 43h distinguishes error 87h ("more pages
        // requested than physically exist in the system") from 88h ("more
        // pages requested than currently available"). Check total capacity
        // first, then currently-free pages.
        if count > self.total_ems_pages() {
            return Err(0x87);
        }
        let (free, _total) = self.ems_unallocated_pages();
        if count > free {
            return Err(0x88);
        }

        let mut pages = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let size = (EMS_PAGE_SIZE as u64 + ALIGN_MASK) & !ALIGN_MASK;
            match self.allocator.allocate(size as u32) {
                Some(alloc) => pages.push(alloc),
                None => {
                    for alloc in pages {
                        self.allocator.deallocate(alloc);
                    }
                    return Err(0x88);
                }
            }
        }

        let handle = &mut self.ems_handles[handle_index as usize];
        handle.active = true;
        handle.pages = pages;
        handle.name = [0u8; 8];
        handle.save_context = None;
        Ok(handle_index)
    }

    pub(crate) fn ems_allocate_pages_zero_allowed(&mut self, count: u16) -> Result<u16, u8> {
        let handle_index = self.find_free_ems_handle().ok_or(0x85u8)?;
        if count > 0 {
            if count > self.total_ems_pages() {
                return Err(0x87);
            }
            let (free, _total) = self.ems_unallocated_pages();
            if count > free {
                return Err(0x88);
            }
        }

        let mut pages = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let size = (EMS_PAGE_SIZE as u64 + ALIGN_MASK) & !ALIGN_MASK;
            match self.allocator.allocate(size as u32) {
                Some(alloc) => pages.push(alloc),
                None => {
                    for alloc in pages {
                        self.allocator.deallocate(alloc);
                    }
                    return Err(0x88);
                }
            }
        }

        let handle = &mut self.ems_handles[handle_index as usize];
        handle.active = true;
        handle.pages = pages;
        handle.name = [0u8; 8];
        handle.save_context = None;
        Ok(handle_index)
    }

    pub(crate) fn ems_map_page(
        &mut self,
        handle_index: u16,
        logical_page: u16,
        physical_page: u8,
        mem: &mut dyn MemoryAccess,
    ) -> u8 {
        if physical_page as usize >= PHYSICAL_PAGES {
            return 0x8B;
        }
        if handle_index as usize >= self.ems_handles.len()
            || !self.ems_handles[handle_index as usize].active
        {
            return 0x83;
        }
        let handle = &self.ems_handles[handle_index as usize];
        if logical_page as usize >= handle.pages.len() {
            return 0x8A;
        }
        let allocation_offset = handle.pages[logical_page as usize].offset();

        self.ems_page_mapping[physical_page as usize] = Some(EmsMapping {
            handle: handle_index,
            logical_page,
            allocation_offset,
        });
        self.apply_ems_page_frame_slot_mapping(physical_page as usize, mem);
        0x00
    }

    pub(crate) fn ems_unmap_page(&mut self, physical_page: u8, mem: &mut dyn MemoryAccess) -> u8 {
        if physical_page as usize >= PHYSICAL_PAGES {
            return 0x8B;
        }
        self.ems_page_mapping[physical_page as usize] = None;
        self.apply_ems_page_frame_slot_mapping(physical_page as usize, mem);
        0x00
    }

    pub(crate) fn ems_deallocate(&mut self, handle_index: u16, mem: &mut dyn MemoryAccess) -> u8 {
        if handle_index as usize >= self.ems_handles.len()
            || !self.ems_handles[handle_index as usize].active
        {
            return 0x83;
        }
        if self.ems_handles[handle_index as usize]
            .save_context
            .is_some()
        {
            return 0x86;
        }

        for slot in 0..PHYSICAL_PAGES {
            if let Some(mapping) = self.ems_page_mapping[slot]
                && mapping.handle == handle_index
            {
                self.ems_page_mapping[slot] = None;
                self.apply_ems_page_frame_slot_mapping(slot, mem);
            }
        }

        let handle = &mut self.ems_handles[handle_index as usize];
        let pages: Vec<Allocation> = handle.pages.drain(..).collect();
        for alloc in pages {
            self.allocator.deallocate(alloc);
        }
        handle.name = [0u8; 8];
        handle.save_context = None;
        if handle_index as usize != EMS_OS_RESERVED_HANDLE {
            handle.active = false;
        }
        0x00
    }

    pub(crate) fn ems_save_page_map(&mut self, handle_index: u16) -> u8 {
        if handle_index as usize >= self.ems_handles.len()
            || !self.ems_handles[handle_index as usize].active
        {
            return 0x83;
        }
        let handle = &mut self.ems_handles[handle_index as usize];
        if handle.save_context.is_some() {
            return 0x8D;
        }
        handle.save_context = Some(self.ems_page_mapping);
        0x00
    }

    pub(crate) fn ems_restore_page_map(
        &mut self,
        handle_index: u16,
        mem: &mut dyn MemoryAccess,
    ) -> u8 {
        if handle_index as usize >= self.ems_handles.len()
            || !self.ems_handles[handle_index as usize].active
        {
            return 0x83;
        }
        let saved = match self.ems_handles[handle_index as usize].save_context {
            Some(ctx) => ctx,
            None => return 0x8E,
        };

        self.ems_page_mapping = saved;
        for slot in 0..PHYSICAL_PAGES {
            self.apply_ems_page_frame_slot_mapping(slot, mem);
        }
        self.ems_handles[handle_index as usize].save_context = None;
        0x00
    }

    pub(crate) fn ems_get_page_map(&self) -> [Option<EmsMapping>; PHYSICAL_PAGES] {
        self.ems_page_mapping
    }

    pub(crate) fn ems_set_page_map(
        &mut self,
        saved: [Option<EmsMapping>; PHYSICAL_PAGES],
        mem: &mut dyn MemoryAccess,
    ) {
        self.ems_page_mapping = saved;
        for slot in 0..PHYSICAL_PAGES {
            self.apply_ems_page_frame_slot_mapping(slot, mem);
        }
    }

    pub(crate) fn ems_page_map_size(&self) -> u16 {
        (PHYSICAL_PAGES * (size_of::<u16>() * 2)) as u16
    }

    /// Size of the save array returned by Get Partial Page Map (Function 16
    /// subfunction 02h). Format: 1 byte count + N * 6 bytes per entry
    /// (segment + handle + logical_page).
    pub(crate) fn ems_partial_page_map_size(&self, page_count: u16) -> Result<u8, u8> {
        if page_count as usize > PHYSICAL_PAGES {
            return Err(0x8B);
        }
        Ok(1u8 + (page_count as u8) * 6)
    }

    /// Serializes the current mapping for the specified physical-page frame
    /// segments. Returns the save-array bytes or an error status.
    pub(crate) fn ems_get_partial_page_map(&self, segments: &[u16]) -> Result<Vec<u8>, u8> {
        if segments.len() > PHYSICAL_PAGES {
            return Err(0xA3);
        }
        let mut out = Vec::with_capacity(1 + segments.len() * 6);
        out.push(segments.len() as u8);
        for &seg in segments {
            let slot = match physical_page_for_segment(seg) {
                Some(s) => s,
                None => return Err(0x8B),
            };
            out.extend_from_slice(&seg.to_le_bytes());
            match self.ems_page_mapping[slot] {
                Some(m) => {
                    out.extend_from_slice(&m.handle.to_le_bytes());
                    out.extend_from_slice(&m.logical_page.to_le_bytes());
                }
                None => {
                    out.extend_from_slice(&0xFFFFu16.to_le_bytes());
                    out.extend_from_slice(&0xFFFFu16.to_le_bytes());
                }
            }
        }
        Ok(out)
    }

    /// Restores a partial mapping context from a save-array produced by
    /// `ems_get_partial_page_map`.
    pub(crate) fn ems_set_partial_page_map(
        &mut self,
        src: &[u8],
        mem: &mut dyn MemoryAccess,
    ) -> u8 {
        if src.is_empty() {
            return 0xA3;
        }
        let count = src[0] as usize;
        if count > PHYSICAL_PAGES || src.len() < 1 + count * 6 {
            return 0xA3;
        }
        for i in 0..count {
            let base = 1 + i * 6;
            let seg = u16::from_le_bytes([src[base], src[base + 1]]);
            let handle = u16::from_le_bytes([src[base + 2], src[base + 3]]);
            let logical = u16::from_le_bytes([src[base + 4], src[base + 5]]);
            let physical = match physical_page_for_segment(seg) {
                Some(p) => p as u8,
                None => return 0x8B,
            };
            let status = if handle == 0xFFFF || logical == 0xFFFF {
                self.ems_unmap_page(physical, mem)
            } else {
                self.ems_map_page(handle, logical, physical, mem)
            };
            if status != 0 {
                return status;
            }
        }
        0x00
    }

    pub(crate) fn ems_reallocate(&mut self, handle_index: u16, new_count: u16) -> Result<u16, u8> {
        if handle_index as usize >= self.ems_handles.len()
            || !self.ems_handles[handle_index as usize].active
        {
            return Err(0x83);
        }
        // EMS 4.0 Function 51h distinguishes 87h ("more pages than
        // physically exist") from 88h ("more pages than currently
        // available"). Check total capacity against the new allocation
        // size before checking currently-free pages.
        if new_count > self.total_ems_pages() {
            return Err(0x87);
        }
        let current_count = self.ems_handles[handle_index as usize].pages.len() as u16;
        if new_count == current_count {
            return Ok(new_count);
        }
        if new_count > current_count {
            let extra = new_count - current_count;
            let (free, _) = self.ems_unallocated_pages();
            if extra > free {
                return Err(0x88);
            }
            let mut new_pages: Vec<Allocation> = Vec::with_capacity(extra as usize);
            for _ in 0..extra {
                let size = (EMS_PAGE_SIZE as u64 + ALIGN_MASK) & !ALIGN_MASK;
                match self.allocator.allocate(size as u32) {
                    Some(alloc) => new_pages.push(alloc),
                    None => {
                        for alloc in new_pages {
                            self.allocator.deallocate(alloc);
                        }
                        return Err(0x88);
                    }
                }
            }
            self.ems_handles[handle_index as usize]
                .pages
                .extend(new_pages);
        } else {
            let handle = &mut self.ems_handles[handle_index as usize];
            while handle.pages.len() > new_count as usize {
                if let Some(alloc) = handle.pages.pop() {
                    self.allocator.deallocate(alloc);
                }
            }
        }
        Ok(new_count)
    }

    pub(crate) fn ems_handle_count(&self) -> u16 {
        self.ems_handles.iter().filter(|h| h.active).count() as u16
    }

    pub(crate) fn ems_handle_pages(&self, handle_index: u16) -> Result<u16, u8> {
        if handle_index as usize >= self.ems_handles.len()
            || !self.ems_handles[handle_index as usize].active
        {
            return Err(0x83);
        }
        Ok(self.ems_handles[handle_index as usize].pages.len() as u16)
    }

    pub(crate) fn ems_all_handle_pages(&self) -> Vec<(u16, u16)> {
        let mut result = Vec::new();
        for (i, handle) in self.ems_handles.iter().enumerate() {
            if handle.active {
                result.push((i as u16, handle.pages.len() as u16));
            }
        }
        result
    }

    pub(crate) fn ems_handle_name(&self, handle_index: u16) -> Result<[u8; 8], u8> {
        if handle_index as usize >= self.ems_handles.len()
            || !self.ems_handles[handle_index as usize].active
        {
            return Err(0x83);
        }
        Ok(self.ems_handles[handle_index as usize].name)
    }

    pub(crate) fn ems_set_handle_name(&mut self, handle_index: u16, name: [u8; 8]) -> u8 {
        if handle_index as usize >= self.ems_handles.len()
            || !self.ems_handles[handle_index as usize].active
        {
            return 0x83;
        }
        if name != [0u8; 8] {
            for (i, h) in self.ems_handles.iter().enumerate() {
                if h.active && i as u16 != handle_index && h.name == name {
                    return 0xA1;
                }
            }
        }
        self.ems_handles[handle_index as usize].name = name;
        0x00
    }

    pub(crate) fn ems_handle_directory(&self) -> Vec<(u16, [u8; 8])> {
        let mut result = Vec::new();
        for (i, handle) in self.ems_handles.iter().enumerate() {
            if handle.active {
                result.push((i as u16, handle.name));
            }
        }
        result
    }

    pub(crate) fn ems_search_handle_name(&self, name: &[u8; 8]) -> Result<u16, u8> {
        if *name == [0u8; 8] {
            return Err(0xA1);
        }
        for (i, handle) in self.ems_handles.iter().enumerate() {
            if handle.active && handle.name == *name {
                return Ok(i as u16);
            }
        }
        Err(0xA0)
    }

    pub(crate) fn ems_is_valid_handle(&self, handle_index: u16) -> bool {
        (handle_index as usize) < self.ems_handles.len()
            && self.ems_handles[handle_index as usize].active
    }

    pub(crate) fn ems_page_allocation_offset(
        &self,
        handle_index: u16,
        logical_page: u16,
    ) -> Option<u32> {
        let handle = self.ems_handles.get(handle_index as usize)?;
        if !handle.active {
            return None;
        }
        let alloc = handle.pages.get(logical_page as usize)?;
        Some(alloc.offset())
    }

    pub(crate) fn ems_move_memory(&self, params: &EmsMoveParams, mem: &mut dyn MemoryAccess) -> u8 {
        self.ems_transfer_memory(params, false, mem)
    }

    pub(crate) fn ems_exchange_memory(
        &self,
        params: &EmsMoveParams,
        mem: &mut dyn MemoryAccess,
    ) -> u8 {
        self.ems_transfer_memory(params, true, mem)
    }

    fn ems_transfer_memory(
        &self,
        params: &EmsMoveParams,
        exchange: bool,
        mem: &mut dyn MemoryAccess,
    ) -> u8 {
        let region_length = params.region_length;
        if region_length == 0 {
            return 0x00;
        }
        if region_length > 0x100000 {
            return 0x96;
        }

        let src_region = match self.resolve_ems_transfer_region(
            params.src_type,
            params.src_handle,
            params.src_offset,
            params.src_seg_page,
            region_length,
        ) {
            Ok(region) => region,
            Err(code) => return code,
        };
        let dst_region = match self.resolve_ems_transfer_region(
            params.dst_type,
            params.dst_handle,
            params.dst_offset,
            params.dst_seg_page,
            region_length,
        ) {
            Ok(region) => region,
            Err(code) => return code,
        };

        if self.ems_regions_overlap_page_frame(src_region, dst_region, region_length) {
            return 0x94;
        }

        let mut status = 0x00;
        if let (
            EmsTransferRegion::Expanded {
                handle: src_handle,
                region_offset: src_region_offset,
            },
            EmsTransferRegion::Expanded {
                handle: dst_handle,
                region_offset: dst_region_offset,
            },
        ) = (src_region, dst_region)
            && src_handle == dst_handle
            && Self::ranges_overlap(
                src_region_offset,
                region_length,
                dst_region_offset,
                region_length,
            )
        {
            if exchange {
                return 0x97;
            }
            status = 0x92;
        }

        let src_buffer = self.read_ems_transfer_region(src_region, region_length, mem);
        if exchange {
            let dst_buffer = self.read_ems_transfer_region(dst_region, region_length, mem);
            self.write_ems_transfer_region(src_region, &dst_buffer, mem);
            self.write_ems_transfer_region(dst_region, &src_buffer, mem);
        } else {
            self.write_ems_transfer_region(dst_region, &src_buffer, mem);
        }
        status
    }

    fn resolve_ems_transfer_region(
        &self,
        memory_type: u8,
        handle: u16,
        offset: u16,
        seg_page: u16,
        region_length: u32,
    ) -> Result<EmsTransferRegion, u8> {
        match memory_type {
            0 => {
                let linear_address = (seg_page as u32)
                    .checked_mul(16)
                    .and_then(|base| base.checked_add(offset as u32))
                    .ok_or(0xA2u8)?;
                if linear_address
                    .checked_add(region_length)
                    .is_none_or(|end| end > 0x100000)
                {
                    return Err(0xA2);
                }
                Ok(EmsTransferRegion::Conventional { linear_address })
            }
            1 => {
                if offset as u32 >= EMS_PAGE_SIZE {
                    return Err(0x95);
                }
                if !self.ems_is_valid_handle(handle) {
                    return Err(0x83);
                }
                let page_count = self.ems_handle_pages(handle)? as u32;
                let region_offset = (seg_page as u32)
                    .checked_mul(EMS_PAGE_SIZE)
                    .and_then(|page_offset| page_offset.checked_add(offset as u32))
                    .ok_or(0x93u8)?;
                if seg_page as u32 >= page_count {
                    return Err(0x8A);
                }
                let total_bytes = page_count.checked_mul(EMS_PAGE_SIZE).ok_or(0x93u8)?;
                if region_offset
                    .checked_add(region_length)
                    .is_none_or(|end| end > total_bytes)
                {
                    return Err(0x93);
                }
                Ok(EmsTransferRegion::Expanded {
                    handle,
                    region_offset,
                })
            }
            _ => Err(0x98),
        }
    }

    fn read_ems_transfer_region(
        &self,
        region: EmsTransferRegion,
        region_length: u32,
        mem: &mut dyn MemoryAccess,
    ) -> Vec<u8> {
        let mut buffer = vec![0u8; region_length as usize];
        self.with_ems_transfer_region_chunks(region, region_length, |src_linear, offset, len| {
            mem.read_block(
                src_linear,
                &mut buffer[offset as usize..(offset + len) as usize],
            );
        });
        buffer
    }

    fn write_ems_transfer_region(
        &self,
        region: EmsTransferRegion,
        data: &[u8],
        mem: &mut dyn MemoryAccess,
    ) {
        self.with_ems_transfer_region_chunks(
            region,
            data.len() as u32,
            |dst_linear, offset, len| {
                mem.write_block(dst_linear, &data[offset as usize..(offset + len) as usize]);
            },
        );
    }

    fn with_ems_transfer_region_chunks(
        &self,
        region: EmsTransferRegion,
        region_length: u32,
        mut visitor: impl FnMut(u32, u32, u32),
    ) {
        match region {
            EmsTransferRegion::Conventional { linear_address } => {
                visitor(linear_address, 0, region_length);
            }
            EmsTransferRegion::Expanded {
                handle,
                region_offset,
            } => {
                let mut copied = 0u32;
                while copied < region_length {
                    let absolute_offset = region_offset + copied;
                    let logical_page = (absolute_offset / EMS_PAGE_SIZE) as u16;
                    let page_offset = absolute_offset % EMS_PAGE_SIZE;
                    let chunk_len = (EMS_PAGE_SIZE - page_offset).min(region_length - copied);
                    let allocation_offset = self
                        .ems_page_allocation_offset(handle, logical_page)
                        .unwrap();
                    let linear_address =
                        self.extended_pool_linear_address(allocation_offset + page_offset);
                    visitor(linear_address, copied, chunk_len);
                    copied += chunk_len;
                }
            }
        }
    }

    fn ems_regions_overlap_page_frame(
        &self,
        src_region: EmsTransferRegion,
        dst_region: EmsTransferRegion,
        region_length: u32,
    ) -> bool {
        match (src_region, dst_region) {
            (
                EmsTransferRegion::Conventional { linear_address },
                EmsTransferRegion::Expanded { .. },
            )
            | (
                EmsTransferRegion::Expanded { .. },
                EmsTransferRegion::Conventional { linear_address },
            ) => Self::ranges_overlap(
                linear_address,
                region_length,
                EMS_PAGE_FRAME_BASE,
                (PHYSICAL_PAGES as u32) * EMS_PAGE_SIZE,
            ),
            _ => false,
        }
    }

    fn ranges_overlap(start_a: u32, len_a: u32, start_b: u32, len_b: u32) -> bool {
        let end_a = start_a + len_a;
        let end_b = start_b + len_b;
        start_a < end_b && start_b < end_a
    }

    pub(crate) fn xms_version(&self) -> (u16, u16, u16) {
        (0x0300, 0x0001, if self.hma_exists() { 1 } else { 0 })
    }

    pub(crate) fn xms_request_hma(&mut self, size: u16) -> Result<(), u8> {
        if !self.hma_exists() {
            return Err(0x90);
        }
        if self.xms_vdisk_conflict_present() {
            return Err(0x81);
        }
        if self.hma_allocated {
            return Err(0x91);
        }
        // Size semantics (XMS 3.0 Request HMA): DX is the amount requested
        // in bytes. Applications pass 0xFFFF to bypass the HMAMIN check;
        // TSRs/drivers pass their actual byte requirement and lose the HMA
        // if it is below /HMAMIN=.
        if size == 0xFFFF {
            self.hma_allocated = true;
            return Ok(());
        }
        let hmamin_bytes = (self.hmamin_kb as u32) * 1024;
        if (size as u32) < hmamin_bytes {
            return Err(0x92);
        }
        self.hma_allocated = true;
        Ok(())
    }

    pub(crate) fn xms_release_hma(&mut self) -> Result<(), u8> {
        if !self.hma_exists() {
            return Err(0x90);
        }
        if self.xms_vdisk_conflict_present() {
            return Err(0x81);
        }
        if !self.hma_allocated {
            return Err(0x93);
        }
        self.hma_allocated = false;
        Ok(())
    }

    pub(crate) fn xms_query_free(&self) -> (u16, u16) {
        let total_free_kb = self.allocator.total_free_size() / 1024;
        let largest_free_kb = self.allocator.largest_free_block_size() / 1024;
        (
            largest_free_kb.min(0xFFFF) as u16,
            total_free_kb.min(0xFFFF) as u16,
        )
    }

    pub(crate) fn xms_allocate(&mut self, size_kb: u16) -> Result<u16, u8> {
        let handle_index = self.find_free_xms_handle().ok_or(0xA1u8)?;
        let size_bytes = (size_kb as u64) * 1024;
        let aligned_size = ((size_bytes + ALIGN_MASK) & !ALIGN_MASK) as u32;

        if aligned_size == 0 {
            let handle = &mut self.xms_handles[handle_index as usize];
            handle.active = true;
            handle.allocation = None;
            handle.size_kb = 0;
            handle.lock_count = 0;
            return Ok(handle_index + 1);
        }

        let allocation = self.allocator.allocate(aligned_size).ok_or(0xA0u8)?;
        let handle = &mut self.xms_handles[handle_index as usize];
        handle.active = true;
        handle.allocation = Some(allocation);
        handle.size_kb = size_kb as u32;
        handle.lock_count = 0;
        Ok(handle_index + 1)
    }

    pub(crate) fn xms_free(&mut self, handle_id: u16) -> Result<(), u8> {
        let index = handle_id.checked_sub(1).ok_or(0xA2u8)? as usize;
        if index >= self.xms_handles.len() || !self.xms_handles[index].active {
            return Err(0xA2);
        }
        if self.xms_handles[index].lock_count > 0 {
            return Err(0xAB);
        }
        if let Some(alloc) = self.xms_handles[index].allocation.take() {
            self.allocator.deallocate(alloc);
        }
        self.xms_handles[index].active = false;
        self.xms_handles[index].size_kb = 0;
        Ok(())
    }

    pub(crate) fn xms_move(&self, mem: &mut dyn MemoryAccess, params_addr: u32) -> Result<(), u8> {
        let length =
            mem.read_word(params_addr) as u32 | ((mem.read_word(params_addr + 2) as u32) << 16);
        let src_handle_id = mem.read_word(params_addr + 4);
        let src_offset =
            mem.read_word(params_addr + 6) as u32 | ((mem.read_word(params_addr + 8) as u32) << 16);
        let dst_handle_id = mem.read_word(params_addr + 10);
        let dst_offset = mem.read_word(params_addr + 12) as u32
            | ((mem.read_word(params_addr + 14) as u32) << 16);

        if length == 0 {
            return Ok(());
        }
        // XMS 3.0 Move EMB: "Length must be even."
        if length & 1 != 0 {
            return Err(0xA7);
        }

        let src_addr = if src_handle_id == 0 {
            // XMS 3.0 Move EMB: "If SourceHandle is set to 0000h, the
            // SourceOffset is interpreted as a standard segment:offset
            // pair... stored in Intel DWORD notation" (low 16 = offset,
            // high 16 = segment).
            let seg = (src_offset >> 16) & 0xFFFF;
            let off = src_offset & 0xFFFF;
            (seg << 4) + off
        } else {
            let index = src_handle_id.checked_sub(1).ok_or(0xA3u8)? as usize;
            if index >= self.xms_handles.len() || !self.xms_handles[index].active {
                return Err(0xA3);
            }
            let size_bytes = self.xms_handles[index].size_kb * 1024;
            if src_offset
                .checked_add(length)
                .is_none_or(|end| end > size_bytes)
            {
                return Err(0xA4);
            }
            let alloc = self.xms_handles[index].allocation.as_ref().ok_or(0xA4u8)?;
            self.extended_pool_linear_address(alloc.offset() + src_offset)
        };

        let dst_addr = if dst_handle_id == 0 {
            let seg = (dst_offset >> 16) & 0xFFFF;
            let off = dst_offset & 0xFFFF;
            (seg << 4) + off
        } else {
            let index = dst_handle_id.checked_sub(1).ok_or(0xA5u8)? as usize;
            if index >= self.xms_handles.len() || !self.xms_handles[index].active {
                return Err(0xA5);
            }
            let size_bytes = self.xms_handles[index].size_kb * 1024;
            if dst_offset
                .checked_add(length)
                .is_none_or(|end| end > size_bytes)
            {
                return Err(0xA6);
            }
            let alloc = self.xms_handles[index].allocation.as_ref().ok_or(0xA6u8)?;
            self.extended_pool_linear_address(alloc.offset() + dst_offset)
        };

        // XMS 3.0 Move EMB: "If the source and destination blocks overlap,
        // only forward moves (i.e. where the source base is less than the
        // destination base) are guaranteed to work properly." Backward
        // overlapping moves are therefore invalid per spec; return 0xA8.
        let src_end = src_addr.saturating_add(length);
        let dst_end = dst_addr.saturating_add(length);
        let ranges_overlap = src_addr < dst_end && dst_addr < src_end;
        if ranges_overlap && src_addr > dst_addr {
            return Err(0xA8);
        }

        let mut buf = vec![0u8; length as usize];
        mem.read_block(src_addr, &mut buf);
        mem.write_block(dst_addr, &buf);
        Ok(())
    }

    pub(crate) fn xms_lock(&mut self, handle_id: u16) -> Result<u32, u8> {
        let index = handle_id.checked_sub(1).ok_or(0xA2u8)? as usize;
        if index >= self.xms_handles.len() || !self.xms_handles[index].active {
            return Err(0xA2);
        }
        if self.xms_handles[index].lock_count == 0xFF {
            return Err(0xAC);
        }
        self.xms_handles[index].lock_count += 1;
        let addr = match self.xms_handles[index].allocation {
            Some(ref alloc) => self.extended_pool_linear_address(alloc.offset()),
            None => self.extended_pool_linear_address(0),
        };
        Ok(addr)
    }

    pub(crate) fn xms_unlock(&mut self, handle_id: u16) -> Result<(), u8> {
        let index = handle_id.checked_sub(1).ok_or(0xA2u8)? as usize;
        if index >= self.xms_handles.len() || !self.xms_handles[index].active {
            return Err(0xA2);
        }
        if self.xms_handles[index].lock_count == 0 {
            return Err(0xAA);
        }
        self.xms_handles[index].lock_count -= 1;
        Ok(())
    }

    pub(crate) fn xms_handle_info(&self, handle_id: u16) -> Result<(u8, u16, u16), u8> {
        let index = handle_id.checked_sub(1).ok_or(0xA2u8)? as usize;
        if index >= self.xms_handles.len() || !self.xms_handles[index].active {
            return Err(0xA2);
        }
        let lock_count = self.xms_handles[index].lock_count;
        let free_handles = self.free_xms_handles_count();
        let size_kb = self.xms_handles[index].size_kb as u16;
        Ok((lock_count, free_handles, size_kb))
    }

    pub(crate) fn xms_reallocate(
        &mut self,
        handle_id: u16,
        new_size_kb: u16,
        mem: &mut dyn MemoryAccess,
    ) -> Result<(), u8> {
        self.xms_reallocate_internal(handle_id, new_size_kb as u32, mem)
    }

    fn xms_reallocate_internal(
        &mut self,
        handle_id: u16,
        new_size_kb: u32,
        mem: &mut dyn MemoryAccess,
    ) -> Result<(), u8> {
        let index = handle_id.checked_sub(1).ok_or(0xA2u8)? as usize;
        if index >= self.xms_handles.len() || !self.xms_handles[index].active {
            return Err(0xA2);
        }
        if self.xms_handles[index].lock_count > 0 {
            return Err(0xAB);
        }

        let old_size_kb = self.xms_handles[index].size_kb;
        if new_size_kb == old_size_kb {
            return Ok(());
        }

        if new_size_kb == 0 {
            if let Some(old_alloc) = self.xms_handles[index].allocation.take() {
                self.allocator.deallocate(old_alloc);
            }
            self.xms_handles[index].size_kb = 0;
            return Ok(());
        }

        let size_bytes = (new_size_kb as u64) * 1024;
        let aligned_size = ((size_bytes + ALIGN_MASK) & !ALIGN_MASK) as u32;
        let copy_bytes = old_size_kb.min(new_size_kb) * 1024;

        // XMS only guarantees a locked block stays at a fixed physical
        // address; after unlock and reallocate the driver is free to move
        // it. In practice, DX386 as used by Doom grows an unlocked block
        // and then expects the next lock to return the same base address
        // again. Prefer an in-place resize when possible to preserve that
        // compatibility, and only relocate when the current range cannot
        // satisfy the new size.
        if let Some(old_alloc) = self.xms_handles[index].allocation
            && let Some(resized_alloc) = self.allocator.reallocate_in_place(old_alloc, aligned_size)
        {
            self.xms_handles[index].allocation = Some(resized_alloc);
            self.xms_handles[index].size_kb = new_size_kb;
            return Ok(());
        }

        if let Some(new_alloc) = self.allocator.allocate(aligned_size) {
            if copy_bytes > 0
                && let Some(ref old_alloc) = self.xms_handles[index].allocation
            {
                let old_base = self.extended_pool_linear_address(old_alloc.offset());
                let new_base = self.extended_pool_linear_address(new_alloc.offset());
                let mut buf = vec![0u8; copy_bytes as usize];
                mem.read_block(old_base, &mut buf);
                mem.write_block(new_base, &buf);
            }
            if let Some(old_alloc) = self.xms_handles[index].allocation.take() {
                self.allocator.deallocate(old_alloc);
            }
            self.xms_handles[index].allocation = Some(new_alloc);
            self.xms_handles[index].size_kb = new_size_kb;
            return Ok(());
        }

        let old_alloc = match self.xms_handles[index].allocation.take() {
            Some(a) => a,
            None => return Err(0xA0),
        };

        let old_base = self.extended_pool_linear_address(old_alloc.offset());
        let mut saved_data = vec![0u8; copy_bytes as usize];
        if copy_bytes > 0 {
            mem.read_block(old_base, &mut saved_data);
        }
        self.allocator.deallocate(old_alloc);

        match self.allocator.allocate(aligned_size) {
            Some(new_alloc) => {
                let new_base = self.extended_pool_linear_address(new_alloc.offset());
                if copy_bytes > 0 {
                    mem.write_block(new_base, &saved_data);
                }
                self.xms_handles[index].allocation = Some(new_alloc);
                self.xms_handles[index].size_kb = new_size_kb;
                Ok(())
            }
            None => {
                let old_aligned_size =
                    (((old_size_kb as u64) * 1024 + ALIGN_MASK) & !ALIGN_MASK) as u32;
                if old_aligned_size == 0 {
                    self.xms_handles[index].size_kb = 0;
                    return Err(0xA0);
                }
                match self.allocator.allocate(old_aligned_size) {
                    Some(restored) => {
                        let restored_base = self.extended_pool_linear_address(restored.offset());
                        if copy_bytes > 0 {
                            mem.write_block(restored_base, &saved_data);
                        }
                        self.xms_handles[index].allocation = Some(restored);
                        self.xms_handles[index].size_kb = old_size_kb;
                        Err(0xA0)
                    }
                    None => {
                        self.xms_handles[index].size_kb = 0;
                        Err(0xA0)
                    }
                }
            }
        }
    }

    pub(crate) fn umb_allocate(
        &self,
        paragraphs: u16,
        mem: &mut dyn MemoryAccess,
    ) -> Result<(u16, u16), (u8, u16)> {
        if !self.umb_enabled {
            return Err((0xB1, 0));
        }
        match memory::allocate(mem, UMB_FIRST_MCB_SEGMENT, paragraphs, MCB_OWNER_DOS, 0) {
            Ok(data_segment) => {
                let mcb_segment = data_segment - 1;
                let actual_size = memory::read_mcb_size_pub(mem, mcb_segment);
                Ok((data_segment, actual_size))
            }
            Err((_error_code, largest)) => {
                if largest > 0 {
                    Err((0xB0, largest))
                } else {
                    Err((0xB1, 0))
                }
            }
        }
    }

    pub(crate) fn umb_free(&self, segment: u16, mem: &mut dyn MemoryAccess) -> Result<(), u8> {
        if !self.umb_enabled {
            return Err(0xB2);
        }
        memory::free(mem, UMB_FIRST_MCB_SEGMENT, segment).map_err(|_| 0xB2)
    }

    pub(crate) fn umb_reallocate(
        &self,
        segment: u16,
        new_paragraphs: u16,
        mem: &mut dyn MemoryAccess,
    ) -> Result<(), (u8, u16)> {
        if !self.umb_enabled {
            return Err((0xB2, 0));
        }
        let largest_available =
            memory::largest_free_block_paragraphs_pub(mem, UMB_FIRST_MCB_SEGMENT);
        memory::resize_without_grow_failure(mem, UMB_FIRST_MCB_SEGMENT, segment, new_paragraphs)
            .map_err(|(code, _largest)| {
                if code == 0x09 {
                    (0xB2, 0)
                } else if largest_available == 0 {
                    (0xB1, 0)
                } else {
                    (0xB0, largest_available)
                }
            })
    }

    pub(crate) fn ems_total_kb(&self) -> u32 {
        if self.ems_enabled {
            self.allocator_total_bytes() / 1024
        } else {
            0
        }
    }

    pub(crate) fn ems_free_kb(&self) -> u32 {
        if self.ems_enabled {
            let (free_pages, _) = self.ems_unallocated_pages();
            (free_pages as u32) * (EMS_PAGE_SIZE / 1024)
        } else {
            0
        }
    }

    pub(crate) fn xms_total_kb(&self) -> u32 {
        if self.xms_enabled {
            self.allocator_total_bytes() / 1024
        } else {
            0
        }
    }

    pub(crate) fn xms_free_kb(&self) -> u32 {
        if self.xms_enabled {
            let (_largest, total) = self.xms_query_free();
            total as u32
        } else {
            0
        }
    }

    pub(crate) fn extended_pool_total_bytes(&self) -> u32 {
        self.allocator_total_bytes()
    }

    pub(crate) fn extended_pool_used_bytes(&self) -> u32 {
        self.extended_pool_total_bytes()
            .saturating_sub(self.allocator.total_free_size())
    }

    pub(crate) fn extended_pool_free_bytes(&self) -> u32 {
        self.allocator.total_free_size()
    }

    pub(crate) fn hma_is_allocated(&self) -> bool {
        self.hma_allocated
    }

    pub(crate) fn is_ems_enabled(&self) -> bool {
        self.ems_enabled
    }

    pub(crate) fn is_xms_enabled(&self) -> bool {
        self.xms_enabled
    }

    pub(crate) fn is_xms_32_enabled(&self) -> bool {
        self.xms_32_enabled
    }

    pub(crate) fn is_umb_enabled(&self) -> bool {
        self.umb_enabled
    }

    pub(crate) fn extended_memory_size_bytes(&self) -> u32 {
        self.extended_memory_size
    }

    pub(crate) fn xms_query_free_32(&self) -> (u32, u32) {
        (
            self.allocator.largest_free_block_size() / 1024,
            self.allocator.total_free_size() / 1024,
        )
    }

    pub(crate) fn xms_allocate_32(&mut self, size_kb: u32) -> Result<u16, u8> {
        let handle_index = self.find_free_xms_handle().ok_or(0xA1u8)?;
        let size_bytes = size_kb.checked_mul(1024).ok_or(0xA0u8)? as u64;
        let aligned_size = ((size_bytes + ALIGN_MASK) & !ALIGN_MASK)
            .try_into()
            .map_err(|_| 0xA0u8)?;

        if aligned_size == 0 {
            let handle = &mut self.xms_handles[handle_index as usize];
            handle.active = true;
            handle.allocation = None;
            handle.size_kb = 0;
            handle.lock_count = 0;
            return Ok(handle_index + 1);
        }

        let allocation = self.allocator.allocate(aligned_size).ok_or(0xA0u8)?;
        let handle = &mut self.xms_handles[handle_index as usize];
        handle.active = true;
        handle.allocation = Some(allocation);
        handle.size_kb = size_kb;
        handle.lock_count = 0;
        Ok(handle_index + 1)
    }

    pub(crate) fn xms_handle_info_32(&self, handle_id: u16) -> Result<(u8, u16, u32), u8> {
        let index = handle_id.checked_sub(1).ok_or(0xA2u8)? as usize;
        if index >= self.xms_handles.len() || !self.xms_handles[index].active {
            return Err(0xA2);
        }
        let lock_count = self.xms_handles[index].lock_count;
        let free_handles = self.free_xms_handles_count();
        let size_kb = self.xms_handles[index].size_kb;
        Ok((lock_count, free_handles, size_kb))
    }

    pub(crate) fn xms_reallocate_32(
        &mut self,
        handle_id: u16,
        new_size_kb: u32,
        mem: &mut dyn MemoryAccess,
    ) -> Result<(), u8> {
        let _ = new_size_kb.checked_mul(1024).ok_or(0xA0u8)?;
        self.xms_reallocate_internal(handle_id, new_size_kb, mem)
    }

    pub(crate) fn ems_enable_os_functions(&mut self, provided_key: u32) -> Result<Option<u32>, u8> {
        match self.ems_os_access_key {
            None => {
                let key = self.next_ems_os_access_key();
                self.ems_os_access_key = Some(key);
                self.ems_os_functions_enabled = true;
                Ok(Some(key))
            }
            Some(stored_key) => {
                if provided_key != stored_key {
                    return Err(0xA4);
                }
                self.ems_os_functions_enabled = true;
                Ok(None)
            }
        }
    }

    pub(crate) fn ems_disable_os_functions(
        &mut self,
        provided_key: u32,
    ) -> Result<Option<u32>, u8> {
        match self.ems_os_access_key {
            None => {
                let key = self.next_ems_os_access_key();
                self.ems_os_access_key = Some(key);
                self.ems_os_functions_enabled = false;
                Ok(Some(key))
            }
            Some(stored_key) => {
                if provided_key != stored_key {
                    return Err(0xA4);
                }
                self.ems_os_functions_enabled = false;
                Ok(None)
            }
        }
    }

    pub(crate) fn ems_return_os_access_key(&mut self, provided_key: u32) -> Result<(), u8> {
        match self.ems_os_access_key {
            Some(stored_key) if provided_key == stored_key => {
                self.ems_os_access_key = None;
                self.ems_os_functions_enabled = true;
                Ok(())
            }
            _ => Err(0xA4),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    struct MockMemory {
        data: HashMap<u32, u8>,
        ext_mem_size: u32,
        ems_page_frame_slot_mappings: [Option<u32>; 4],
    }

    impl MockMemory {
        fn new(ext_mem_size_kb: u32) -> Self {
            Self {
                data: HashMap::new(),
                ext_mem_size: ext_mem_size_kb * 1024,
                ems_page_frame_slot_mappings: [None; 4],
            }
        }
    }

    impl MemoryAccess for MockMemory {
        fn read_byte(&self, address: u32) -> u8 {
            if (EMS_PAGE_FRAME_BASE..EMS_PAGE_FRAME_BASE + (PHYSICAL_PAGES as u32) * EMS_PAGE_SIZE)
                .contains(&address)
            {
                let slot = ((address - EMS_PAGE_FRAME_BASE) / EMS_PAGE_SIZE) as usize;
                let slot_offset = (address - EMS_PAGE_FRAME_BASE) % EMS_PAGE_SIZE;
                if let Some(base) = self.ems_page_frame_slot_mappings[slot] {
                    return self
                        .data
                        .get(&(base + slot_offset))
                        .copied()
                        .unwrap_or(0x00);
                }
            }
            self.data.get(&address).copied().unwrap_or(0x00)
        }

        fn write_byte(&mut self, address: u32, value: u8) {
            if (EMS_PAGE_FRAME_BASE..EMS_PAGE_FRAME_BASE + (PHYSICAL_PAGES as u32) * EMS_PAGE_SIZE)
                .contains(&address)
            {
                let slot = ((address - EMS_PAGE_FRAME_BASE) / EMS_PAGE_SIZE) as usize;
                let slot_offset = (address - EMS_PAGE_FRAME_BASE) % EMS_PAGE_SIZE;
                if let Some(base) = self.ems_page_frame_slot_mappings[slot] {
                    self.data.insert(base + slot_offset, value);
                    return;
                }
            }
            self.data.insert(address, value);
        }

        fn read_word(&self, address: u32) -> u16 {
            let lo = self.read_byte(address) as u16;
            let hi = self.read_byte(address + 1) as u16;
            (hi << 8) | lo
        }

        fn write_word(&mut self, address: u32, value: u16) {
            self.write_byte(address, value as u8);
            self.write_byte(address + 1, (value >> 8) as u8);
        }

        fn read_block(&self, address: u32, buf: &mut [u8]) {
            for (i, byte) in buf.iter_mut().enumerate() {
                *byte = self.read_byte(address + i as u32);
            }
        }

        fn write_block(&mut self, address: u32, data: &[u8]) {
            for (i, &byte) in data.iter().enumerate() {
                self.write_byte(address + i as u32, byte);
            }
        }

        fn extended_memory_size(&self) -> u32 {
            self.ext_mem_size
        }

        fn map_ems_page_frame_slot(&mut self, physical_page: u8, backing_linear_addr: Option<u32>) {
            let physical_page = usize::from(physical_page);
            if physical_page < self.ems_page_frame_slot_mappings.len() {
                self.ems_page_frame_slot_mappings[physical_page] = backing_linear_addr;
            }
        }
    }

    fn create_manager(pool_kb: u32) -> (MemoryManager, MockMemory) {
        create_manager_selective(pool_kb, true, true)
    }

    fn create_manager_selective(pool_kb: u32, ems: bool, xms: bool) -> (MemoryManager, MockMemory) {
        let mut mem = MockMemory::new(pool_kb);
        let mm = MemoryManager::new(pool_kb * 1024, ems, xms, true, &mut mem);
        (mm, mem)
    }

    #[test]
    fn test_new_both_enabled() {
        let (mm, _mem) = create_manager(1024);
        assert!(mm.is_ems_enabled());
        assert!(mm.is_xms_enabled());
        assert!(mm.is_umb_enabled());
        assert!(!mm.hma_is_allocated());
    }

    #[test]
    fn test_new_ems_only() {
        let (mm, _mem) = create_manager_selective(1024, true, false);
        assert!(mm.is_ems_enabled());
        assert!(!mm.is_xms_enabled());
        assert!(mm.is_umb_enabled());
    }

    #[test]
    fn test_new_xms_only() {
        let (mm, _mem) = create_manager_selective(1024, false, true);
        assert!(!mm.is_ems_enabled());
        assert!(mm.is_xms_enabled());
        assert!(mm.is_umb_enabled());
    }

    #[test]
    fn test_new_both_disabled() {
        let (mm, _mem) = create_manager_selective(1024, false, false);
        assert!(!mm.is_ems_enabled());
        assert!(!mm.is_xms_enabled());
        assert!(!mm.is_umb_enabled());
    }

    #[test]
    fn test_initial_free_pages() {
        let (mm, _mem) = create_manager(1024);
        let (free, total) = mm.ems_unallocated_pages();
        assert_eq!(total, 60);
        assert_eq!(free, 60);
        let (largest, total_free) = mm.xms_query_free();
        assert_eq!(total_free, 960);
        assert_eq!(largest, 960);
    }

    #[test]
    fn test_ems_status_enabled() {
        let (mm, _mem) = create_manager(1024);
        assert_eq!(mm.ems_status(), 0x00);
    }

    #[test]
    fn test_ems_status_disabled() {
        let (mm, _mem) = create_manager_selective(1024, false, true);
        assert_eq!(mm.ems_status(), 0x84);
    }

    #[test]
    fn test_ems_page_frame_segment() {
        let (mm, _mem) = create_manager(1024);
        assert_eq!(mm.ems_page_frame_segment(), 0xC000);
    }

    #[test]
    fn test_ems_total_and_free_kb() {
        let (mm, _mem) = create_manager(1024);
        assert_eq!(mm.ems_total_kb(), 960);
        assert_eq!(mm.ems_free_kb(), 960);

        let (mm_disabled, _mem) = create_manager_selective(1024, false, true);
        assert_eq!(mm_disabled.ems_total_kb(), 0);
        assert_eq!(mm_disabled.ems_free_kb(), 0);
    }

    #[test]
    fn test_ems_allocate_single_page() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();
        assert_eq!(mm.ems_handle_count(), 2);
        assert_eq!(mm.ems_handle_pages(handle).unwrap(), 1);
        let (free, _) = mm.ems_unallocated_pages();
        assert_eq!(free, 59);
    }

    #[test]
    fn test_ems_allocate_multiple_pages() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(4).unwrap();
        assert_eq!(mm.ems_handle_pages(handle).unwrap(), 4);
        let (free, _) = mm.ems_unallocated_pages();
        assert_eq!(free, 56);
    }

    #[test]
    fn test_ems_allocate_two_handles() {
        let (mut mm, _mem) = create_manager(1024);
        let h1 = mm.ems_allocate_pages(2).unwrap();
        let h2 = mm.ems_allocate_pages(3).unwrap();
        assert_ne!(h1, h2);
        assert_eq!(mm.ems_handle_count(), 3);
        let (free, _) = mm.ems_unallocated_pages();
        assert_eq!(free, 55);
    }

    #[test]
    fn test_ems_allocate_zero_rejected() {
        let (mut mm, _mem) = create_manager(1024);
        assert_eq!(mm.ems_allocate_pages(0), Err(0x89));
    }

    #[test]
    fn test_ems_allocate_zero_allowed() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages_zero_allowed(0).unwrap();
        assert_eq!(mm.ems_handle_pages(handle).unwrap(), 0);
        assert_eq!(mm.ems_handle_count(), 2);
    }

    #[test]
    fn test_ems_allocate_zero_allowed_exceeds_total() {
        let (mut mm, _mem) = create_manager(1024);
        assert_eq!(mm.ems_allocate_pages_zero_allowed(65), Err(0x87));
    }

    #[test]
    fn test_ems_allocate_exceeds_free() {
        let (mut mm, _mem) = create_manager(1024);
        // Total is 64 pages; consume 50 so 14 remain free, then request 20.
        // That is <= total but > free, exercising the 0x88 path only.
        mm.ems_allocate_pages(50).unwrap();
        assert_eq!(mm.ems_allocate_pages(20), Err(0x88));
    }

    #[test]
    fn test_ems_allocate_exceeds_total() {
        let (mut mm, _mem) = create_manager(1024);
        // Total pool is 64 pages; asking for 65 exceeds physical capacity.
        assert_eq!(mm.ems_allocate_pages(65), Err(0x87));
    }

    #[test]
    fn test_ems_allocate_exhausts_handles() {
        let (mut mm, _mem) = create_manager(4096);
        for _ in 1..MAX_EMS_HANDLES {
            mm.ems_allocate_pages_zero_allowed(0).unwrap();
        }
        assert_eq!(mm.ems_allocate_pages_zero_allowed(0), Err(0x85));
    }

    #[test]
    fn test_ems_deallocate_valid() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(4).unwrap();
        assert_eq!(mm.ems_deallocate(handle, &mut mem), 0x00);
        let (free, total) = mm.ems_unallocated_pages();
        assert_eq!(free, total);
        assert!(!mm.ems_is_valid_handle(handle));
        assert_eq!(mm.ems_handle_count(), 1);
    }

    #[test]
    fn test_ems_deallocate_invalid_handle() {
        let (mut mm, mut mem) = create_manager(1024);
        assert_eq!(mm.ems_deallocate(200, &mut mem), 0x83);
    }

    #[test]
    fn test_ems_deallocate_frees_for_reuse() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(60).unwrap();
        assert_eq!(mm.ems_deallocate(handle, &mut mem), 0x00);
        let handle2 = mm.ems_allocate_pages(60).unwrap();
        assert_eq!(mm.ems_handle_pages(handle2).unwrap(), 60);
    }

    #[test]
    fn test_ems_deallocate_saved_context_returns_86() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();

        assert_eq!(mm.ems_save_page_map(handle), 0x00);
        assert_eq!(mm.ems_deallocate(handle, &mut mem), 0x86);
        assert!(mm.ems_is_valid_handle(handle));
    }

    #[test]
    fn test_ems_deallocate_reserved_handle_keeps_os_handle_active() {
        let (mut mm, mut mem) = create_manager(1024);

        assert_eq!(mm.ems_deallocate(0, &mut mem), 0x00);
        assert!(mm.ems_is_valid_handle(0));
        assert_eq!(mm.ems_handle_pages(0).unwrap(), 0);
        assert_eq!(mm.ems_handle_count(), 1);
    }

    #[test]
    fn test_ems_map_page_success() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(2).unwrap();
        assert_eq!(mm.ems_map_page(handle, 0, 0, &mut mem), 0x00);
    }

    #[test]
    fn test_ems_map_page_invalid_physical() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();
        assert_eq!(mm.ems_map_page(handle, 0, 4, &mut mem), 0x8B);
    }

    #[test]
    fn test_ems_map_page_invalid_handle() {
        let (mut mm, mut mem) = create_manager(1024);
        assert_eq!(mm.ems_map_page(200, 0, 0, &mut mem), 0x83);
    }

    #[test]
    fn test_ems_map_page_invalid_logical() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(2).unwrap();
        assert_eq!(mm.ems_map_page(handle, 2, 0, &mut mem), 0x8A);
    }

    #[test]
    fn test_ems_map_and_read_back_data() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();
        let offset = mm.ems_page_allocation_offset(handle, 0).unwrap();

        // Write a pattern into extended RAM at the allocation's offset.
        let pattern = [0xDE, 0xAD, 0xBE, 0xEF];
        mem.write_block(mm.extended_pool_linear_address(offset), &pattern);

        // Map page to physical slot 0 (copies data into page frame at 0xC0000).
        assert_eq!(mm.ems_map_page(handle, 0, 0, &mut mem), 0x00);

        // Read from page frame; should match the pattern.
        let mut buf = [0u8; 4];
        mem.read_block(EMS_PAGE_FRAME_BASE, &mut buf);
        assert_eq!(buf, pattern);
    }

    #[test]
    fn test_ems_unmap_page() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();
        mm.ems_map_page(handle, 0, 0, &mut mem);
        assert_eq!(mm.ems_unmap_page(0, &mut mem), 0x00);
        assert_eq!(mm.ems_unmap_page(4, &mut mem), 0x8B);
    }

    #[test]
    fn test_ems_same_logical_page_aliases_across_physical_slots() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();

        assert_eq!(mm.ems_map_page(handle, 0, 0, &mut mem), 0x00);
        assert_eq!(mm.ems_map_page(handle, 0, 1, &mut mem), 0x00);

        mem.write_byte(EMS_PAGE_FRAME_BASE, 0x5A);

        assert_eq!(mem.read_byte(EMS_PAGE_FRAME_BASE + EMS_PAGE_SIZE), 0x5A);
    }

    #[test]
    fn test_ems_map_saves_previous_content() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(2).unwrap();

        // Map logical page 0 to physical 0 and write a marker.
        mm.ems_map_page(handle, 0, 0, &mut mem);
        mem.write_byte(EMS_PAGE_FRAME_BASE, 0xAA);

        // Map logical page 1 to physical 0 (saves page 0's content back to extended RAM).
        mm.ems_map_page(handle, 1, 0, &mut mem);
        mem.write_byte(EMS_PAGE_FRAME_BASE, 0xBB);

        // Map logical page 0 back to physical 0 (saves page 1, loads page 0).
        mm.ems_map_page(handle, 0, 0, &mut mem);
        assert_eq!(mem.read_byte(EMS_PAGE_FRAME_BASE), 0xAA);
    }

    #[test]
    fn test_ems_save_page_map_success() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();
        assert_eq!(mm.ems_save_page_map(handle), 0x00);
    }

    #[test]
    fn test_ems_save_already_saved() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();
        mm.ems_save_page_map(handle);
        assert_eq!(mm.ems_save_page_map(handle), 0x8D);
    }

    #[test]
    fn test_ems_save_invalid_handle() {
        let (mut mm, _mem) = create_manager(1024);
        assert_eq!(mm.ems_save_page_map(200), 0x83);
    }

    #[test]
    fn test_ems_restore_success() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(2).unwrap();

        mm.ems_map_page(handle, 0, 0, &mut mem);
        mem.write_byte(EMS_PAGE_FRAME_BASE, 0xAA);
        mm.ems_save_page_map(handle);

        mm.ems_map_page(handle, 1, 0, &mut mem);
        mem.write_byte(EMS_PAGE_FRAME_BASE, 0xBB);

        assert_eq!(mm.ems_restore_page_map(handle, &mut mem), 0x00);
        assert_eq!(mem.read_byte(EMS_PAGE_FRAME_BASE), 0xAA);
    }

    #[test]
    fn test_ems_restore_no_save() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();
        assert_eq!(mm.ems_restore_page_map(handle, &mut mem), 0x8E);
    }

    #[test]
    fn test_ems_get_set_page_map() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(2).unwrap();

        mm.ems_map_page(handle, 0, 0, &mut mem);
        mm.ems_map_page(handle, 1, 1, &mut mem);
        mem.write_byte(EMS_PAGE_FRAME_BASE, 0x11);
        mem.write_byte(EMS_PAGE_FRAME_BASE + EMS_PAGE_SIZE, 0x22);

        let saved = mm.ems_get_page_map();
        assert!(saved[0].is_some());
        assert!(saved[1].is_some());
        assert!(saved[2].is_none());
        assert!(saved[3].is_none());

        mm.ems_unmap_page(0, &mut mem);
        mm.ems_unmap_page(1, &mut mem);

        mm.ems_set_page_map(saved, &mut mem);
        assert_eq!(mem.read_byte(EMS_PAGE_FRAME_BASE), 0x11);
        assert_eq!(mem.read_byte(EMS_PAGE_FRAME_BASE + EMS_PAGE_SIZE), 0x22);
    }

    #[test]
    fn test_ems_reallocate_grow() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(2).unwrap();
        assert_eq!(mm.ems_reallocate(handle, 4), Ok(4));
        assert_eq!(mm.ems_handle_pages(handle).unwrap(), 4);
    }

    #[test]
    fn test_ems_reallocate_shrink() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(4).unwrap();
        let (free_before, _) = mm.ems_unallocated_pages();
        assert_eq!(mm.ems_reallocate(handle, 2), Ok(2));
        assert_eq!(mm.ems_handle_pages(handle).unwrap(), 2);
        let (free_after, _) = mm.ems_unallocated_pages();
        assert_eq!(free_after, free_before + 2);
    }

    #[test]
    fn test_ems_reallocate_same() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(3).unwrap();
        assert_eq!(mm.ems_reallocate(handle, 3), Ok(3));
    }

    #[test]
    fn test_ems_reallocate_exceeds_free() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(2).unwrap();
        // Consume more of the pool so free drops below the growth request.
        // Total=64, allocated=52, free=12; growing handle from 2 to 60
        // needs extra=58 which exceeds free but not total.
        mm.ems_allocate_pages(50).unwrap();
        assert_eq!(mm.ems_reallocate(handle, 60), Err(0x88));
    }

    #[test]
    fn test_ems_reallocate_exceeds_total() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(2).unwrap();
        // Total pool is 64 pages; asking for 65 exceeds physical capacity.
        assert_eq!(mm.ems_reallocate(handle, 65), Err(0x87));
    }

    #[test]
    fn test_ems_handle_name_default() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();
        assert_eq!(mm.ems_handle_name(handle).unwrap(), [0u8; 8]);
    }

    #[test]
    fn test_ems_set_and_get_name() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();
        assert_eq!(mm.ems_set_handle_name(handle, *b"TESTNAME"), 0x00);
        assert_eq!(mm.ems_handle_name(handle).unwrap(), *b"TESTNAME");
    }

    #[test]
    fn test_ems_name_duplicate_rejected() {
        let (mut mm, _mem) = create_manager(1024);
        let h1 = mm.ems_allocate_pages(1).unwrap();
        let h2 = mm.ems_allocate_pages(1).unwrap();
        mm.ems_set_handle_name(h1, *b"MYNAME\0\0");
        assert_eq!(mm.ems_set_handle_name(h2, *b"MYNAME\0\0"), 0xA1);
    }

    #[test]
    fn test_ems_name_zero_allows_duplicate() {
        let (mut mm, _mem) = create_manager(1024);
        let h1 = mm.ems_allocate_pages(1).unwrap();
        let h2 = mm.ems_allocate_pages(1).unwrap();
        assert_eq!(mm.ems_set_handle_name(h1, [0u8; 8]), 0x00);
        assert_eq!(mm.ems_set_handle_name(h2, [0u8; 8]), 0x00);
    }

    #[test]
    fn test_ems_search_handle_name() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();
        mm.ems_set_handle_name(handle, *b"FINDME\0\0");
        assert_eq!(mm.ems_search_handle_name(b"FINDME\0\0"), Ok(handle));
        assert_eq!(mm.ems_search_handle_name(b"NOTEXIST"), Err(0xA0));
    }

    #[test]
    fn test_ems_search_zero_name_returns_not_found() {
        let (mut mm, _mem) = create_manager(1024);
        let _handle = mm.ems_allocate_pages(1).unwrap();
        assert_eq!(mm.ems_search_handle_name(&[0u8; 8]), Err(0xA1));
    }

    #[test]
    fn test_ems_handle_count() {
        let (mut mm, mut mem) = create_manager(1024);
        assert_eq!(mm.ems_handle_count(), 1);
        let h1 = mm.ems_allocate_pages(1).unwrap();
        let h2 = mm.ems_allocate_pages(1).unwrap();
        let h3 = mm.ems_allocate_pages(1).unwrap();
        assert_eq!(mm.ems_handle_count(), 4);
        mm.ems_deallocate(h2, &mut mem);
        assert_eq!(mm.ems_handle_count(), 3);
        mm.ems_deallocate(h1, &mut mem);
        mm.ems_deallocate(h3, &mut mem);
    }

    #[test]
    fn test_ems_all_handle_pages() {
        let (mut mm, _mem) = create_manager(1024);
        let h1 = mm.ems_allocate_pages(2).unwrap();
        let h2 = mm.ems_allocate_pages(5).unwrap();
        let all = mm.ems_all_handle_pages();
        assert_eq!(all.len(), 3);
        assert!(all.contains(&(0, 0)));
        assert!(all.contains(&(h1, 2)));
        assert!(all.contains(&(h2, 5)));
    }

    #[test]
    fn test_ems_handle_directory() {
        let (mut mm, _mem) = create_manager(1024);
        let h1 = mm.ems_allocate_pages(1).unwrap();
        let h2 = mm.ems_allocate_pages(1).unwrap();
        let h3 = mm.ems_allocate_pages(1).unwrap();
        mm.ems_set_handle_name(h1, *b"ALPHA\0\0\0");
        mm.ems_set_handle_name(h2, *b"BETA\0\0\0\0");
        mm.ems_set_handle_name(h3, *b"GAMMA\0\0\0");
        let dir = mm.ems_handle_directory();
        assert_eq!(dir.len(), 4);
        assert!(dir.contains(&(0, [0u8; 8])));
        assert!(dir.contains(&(h1, *b"ALPHA\0\0\0")));
        assert!(dir.contains(&(h2, *b"BETA\0\0\0\0")));
        assert!(dir.contains(&(h3, *b"GAMMA\0\0\0")));
    }

    #[test]
    fn test_ems_move_conventional_to_ems() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();

        let pattern = [0xDE, 0xAD, 0xBE, 0xEF];
        mem.write_block(0x1000, &pattern);

        let params = EmsMoveParams {
            region_length: 4,
            src_type: 0,
            src_handle: 0,
            src_offset: 0,
            src_seg_page: 0x100,
            dst_type: 1,
            dst_handle: handle,
            dst_offset: 0,
            dst_seg_page: 0,
        };
        assert_eq!(mm.ems_move_memory(&params, &mut mem), 0x00);

        let offset = mm.ems_page_allocation_offset(handle, 0).unwrap();
        let mut buf = [0u8; 4];
        mem.read_block(mm.extended_pool_linear_address(offset), &mut buf);
        assert_eq!(buf, pattern);
    }

    #[test]
    fn test_ems_move_ems_to_conventional() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();
        let offset = mm.ems_page_allocation_offset(handle, 0).unwrap();

        let pattern = [0xCA, 0xFE, 0xBA, 0xBE];
        mem.write_block(mm.extended_pool_linear_address(offset), &pattern);

        let params = EmsMoveParams {
            region_length: 4,
            src_type: 1,
            src_handle: handle,
            src_offset: 0,
            src_seg_page: 0,
            dst_type: 0,
            dst_handle: 0,
            dst_offset: 0,
            dst_seg_page: 0x300,
        };
        assert_eq!(mm.ems_move_memory(&params, &mut mem), 0x00);

        let mut buf = [0u8; 4];
        mem.read_block(0x3000, &mut buf);
        assert_eq!(buf, pattern);
    }

    #[test]
    fn test_ems_move_reads_updated_page_frame_contents() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();

        assert_eq!(mm.ems_map_page(handle, 0, 0, &mut mem), 0x00);
        mem.write_byte(EMS_PAGE_FRAME_BASE, 0x11);
        mem.write_byte(EMS_PAGE_FRAME_BASE + 1, 0x22);

        let params = EmsMoveParams {
            region_length: 2,
            src_type: 1,
            src_handle: handle,
            src_offset: 0,
            src_seg_page: 0,
            dst_type: 0,
            dst_handle: 0,
            dst_offset: 0,
            dst_seg_page: 0x0200,
        };

        assert_eq!(mm.ems_move_memory(&params, &mut mem), 0x00);
        assert_eq!(mem.read_byte(0x2000), 0x11);
        assert_eq!(mem.read_byte(0x2001), 0x22);
    }

    #[test]
    fn test_ems_move_updates_mapped_page_frame_after_write() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();

        assert_eq!(mm.ems_map_page(handle, 0, 0, &mut mem), 0x00);
        mem.write_byte(0x2000, 0x44);
        mem.write_byte(0x2001, 0x55);

        let params = EmsMoveParams {
            region_length: 2,
            src_type: 0,
            src_handle: 0,
            src_offset: 0,
            src_seg_page: 0x0200,
            dst_type: 1,
            dst_handle: handle,
            dst_offset: 0,
            dst_seg_page: 0,
        };

        assert_eq!(mm.ems_move_memory(&params, &mut mem), 0x00);
        assert_eq!(mem.read_byte(EMS_PAGE_FRAME_BASE), 0x44);
        assert_eq!(mem.read_byte(EMS_PAGE_FRAME_BASE + 1), 0x55);
    }

    #[test]
    fn test_ems_move_ems_to_ems() {
        let (mut mm, mut mem) = create_manager(1024);
        let h1 = mm.ems_allocate_pages(1).unwrap();
        let h2 = mm.ems_allocate_pages(1).unwrap();
        let offset1 = mm.ems_page_allocation_offset(h1, 0).unwrap();

        let pattern = [0x01, 0x02, 0x03, 0x04];
        mem.write_block(mm.extended_pool_linear_address(offset1), &pattern);

        let params = EmsMoveParams {
            region_length: 4,
            src_type: 1,
            src_handle: h1,
            src_offset: 0,
            src_seg_page: 0,
            dst_type: 1,
            dst_handle: h2,
            dst_offset: 0,
            dst_seg_page: 0,
        };
        assert_eq!(mm.ems_move_memory(&params, &mut mem), 0x00);

        let offset2 = mm.ems_page_allocation_offset(h2, 0).unwrap();
        let mut buf = [0u8; 4];
        mem.read_block(mm.extended_pool_linear_address(offset2), &mut buf);
        assert_eq!(buf, pattern);
    }

    #[test]
    fn test_ems_exchange_ems_to_ems_swaps_contents() {
        let (mut mm, mut mem) = create_manager(1024);
        let h1 = mm.ems_allocate_pages(1).unwrap();
        let h2 = mm.ems_allocate_pages(1).unwrap();
        let offset1 = mm.ems_page_allocation_offset(h1, 0).unwrap();
        let offset2 = mm.ems_page_allocation_offset(h2, 0).unwrap();

        let src_pattern = [0x10, 0x11, 0x12, 0x13];
        let dst_pattern = [0x20, 0x21, 0x22, 0x23];
        mem.write_block(mm.extended_pool_linear_address(offset1), &src_pattern);
        mem.write_block(mm.extended_pool_linear_address(offset2), &dst_pattern);

        let params = EmsMoveParams {
            region_length: 4,
            src_type: 1,
            src_handle: h1,
            src_offset: 0,
            src_seg_page: 0,
            dst_type: 1,
            dst_handle: h2,
            dst_offset: 0,
            dst_seg_page: 0,
        };
        assert_eq!(mm.ems_exchange_memory(&params, &mut mem), 0x00);

        let mut src_buf = [0u8; 4];
        let mut dst_buf = [0u8; 4];
        mem.read_block(mm.extended_pool_linear_address(offset1), &mut src_buf);
        mem.read_block(mm.extended_pool_linear_address(offset2), &mut dst_buf);
        assert_eq!(src_buf, dst_pattern);
        assert_eq!(dst_buf, src_pattern);
    }

    #[test]
    fn test_ems_move_zero_length() {
        let (mm, mut mem) = create_manager(1024);
        let params = EmsMoveParams {
            region_length: 0,
            src_type: 0,
            src_handle: 0,
            src_offset: 0,
            src_seg_page: 0,
            dst_type: 0,
            dst_handle: 0,
            dst_offset: 0,
            dst_seg_page: 0,
        };
        assert_eq!(mm.ems_move_memory(&params, &mut mem), 0x00);
    }

    #[test]
    fn test_ems_move_error_cases() {
        let (mm, mut mem) = create_manager(1024);

        let invalid_handle = EmsMoveParams {
            region_length: 4,
            src_type: 1,
            src_handle: 200,
            src_offset: 0,
            src_seg_page: 0,
            dst_type: 0,
            dst_handle: 0,
            dst_offset: 0,
            dst_seg_page: 0,
        };
        assert_eq!(mm.ems_move_memory(&invalid_handle, &mut mem), 0x83);

        let invalid_type = EmsMoveParams {
            region_length: 4,
            src_type: 2,
            src_handle: 0,
            src_offset: 0,
            src_seg_page: 0,
            dst_type: 0,
            dst_handle: 0,
            dst_offset: 0,
            dst_seg_page: 0,
        };
        assert_eq!(mm.ems_move_memory(&invalid_type, &mut mem), 0x98);

        let overflow = EmsMoveParams {
            region_length: 0x100000,
            src_type: 0,
            src_handle: 0,
            src_offset: 0,
            src_seg_page: 0x1000,
            dst_type: 0,
            dst_handle: 0,
            dst_offset: 0,
            dst_seg_page: 0,
        };
        assert_eq!(mm.ems_move_memory(&overflow, &mut mem), 0xA2);
    }

    #[test]
    fn test_ems_move_same_handle_overlap_returns_92() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();
        let offset = mm.ems_page_allocation_offset(handle, 0).unwrap();
        let pattern = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
        mem.write_block(mm.extended_pool_linear_address(offset), &pattern);

        let params = EmsMoveParams {
            region_length: 4,
            src_type: 1,
            src_handle: handle,
            src_offset: 0,
            src_seg_page: 0,
            dst_type: 1,
            dst_handle: handle,
            dst_offset: 2,
            dst_seg_page: 0,
        };
        assert_eq!(mm.ems_move_memory(&params, &mut mem), 0x92);

        let mut buf = [0u8; 6];
        mem.read_block(mm.extended_pool_linear_address(offset), &mut buf);
        assert_eq!(buf, [0x01, 0x02, 0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn test_ems_exchange_same_handle_overlap_returns_97() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();
        let offset = mm.ems_page_allocation_offset(handle, 0).unwrap();
        let pattern = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
        mem.write_block(mm.extended_pool_linear_address(offset), &pattern);

        let params = EmsMoveParams {
            region_length: 4,
            src_type: 1,
            src_handle: handle,
            src_offset: 0,
            src_seg_page: 0,
            dst_type: 1,
            dst_handle: handle,
            dst_offset: 2,
            dst_seg_page: 0,
        };
        assert_eq!(mm.ems_exchange_memory(&params, &mut mem), 0x97);

        let mut buf = [0u8; 6];
        mem.read_block(mm.extended_pool_linear_address(offset), &mut buf);
        assert_eq!(buf, pattern);
    }

    #[test]
    fn test_ems_move_expanded_offset_above_page_returns_95() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();

        let params = EmsMoveParams {
            region_length: 4,
            src_type: 1,
            src_handle: handle,
            src_offset: 0x4000,
            src_seg_page: 0,
            dst_type: 0,
            dst_handle: 0,
            dst_offset: 0,
            dst_seg_page: 0,
        };
        assert_eq!(mm.ems_move_memory(&params, &mut mem), 0x95);
    }

    #[test]
    fn test_ems_move_expanded_region_exceeding_handle_returns_93() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(1).unwrap();

        let params = EmsMoveParams {
            region_length: 4,
            src_type: 1,
            src_handle: handle,
            src_offset: 0x3FFE,
            src_seg_page: 0,
            dst_type: 0,
            dst_handle: 0,
            dst_offset: 0,
            dst_seg_page: 0,
        };
        assert_eq!(mm.ems_move_memory(&params, &mut mem), 0x93);
    }

    #[test]
    fn test_ems_move_conventional_wraps_past_one_megabyte_returns_a2() {
        let (mm, mut mem) = create_manager(1024);
        let params = EmsMoveParams {
            region_length: 0x20,
            src_type: 0,
            src_handle: 0,
            src_offset: 0,
            src_seg_page: 0xFFFF,
            dst_type: 0,
            dst_handle: 0,
            dst_offset: 0,
            dst_seg_page: 0,
        };
        assert_eq!(mm.ems_move_memory(&params, &mut mem), 0xA2);
    }

    #[test]
    fn test_xms_version() {
        let (mm, _mem) = create_manager(1024);
        let (version, revision, has_ext) = mm.xms_version();
        assert_eq!(version, 0x0300);
        assert_eq!(revision, 0x0001);
        assert_eq!(has_ext, 1);

        let (mm_zero, _mem) = create_manager_selective(0, false, true);
        let (_, _, has_ext_zero) = mm_zero.xms_version();
        assert_eq!(has_ext_zero, 0);
    }

    #[test]
    fn test_xms_total_and_free_kb() {
        let (mm, _mem) = create_manager(1024);
        assert_eq!(mm.xms_total_kb(), 960);
        assert_eq!(mm.xms_free_kb(), 960);

        let (mm_disabled, _mem) = create_manager_selective(1024, true, false);
        assert_eq!(mm_disabled.xms_total_kb(), 0);
        assert_eq!(mm_disabled.xms_free_kb(), 0);
    }

    #[test]
    fn test_xms_request_hma() {
        let (mut mm, _mem) = create_manager(1024);
        assert!(mm.xms_request_hma(64).is_ok());
        assert!(mm.hma_is_allocated());
    }

    #[test]
    fn test_xms_request_hma_twice() {
        let (mut mm, _mem) = create_manager(1024);
        mm.xms_request_hma(64).unwrap();
        assert_eq!(mm.xms_request_hma(64), Err(0x91));
    }

    #[test]
    fn test_xms_request_hma_zero_size_allowed_when_hmamin_is_zero() {
        let (mut mm, _mem) = create_manager(1024);
        assert!(mm.xms_request_hma(0).is_ok());
    }

    #[test]
    fn test_xms_request_hma_zero_size_respects_hmamin_threshold() {
        let (mut mm, _mem) = create_manager(1024);
        mm.set_hmamin_kb(32);
        assert_eq!(mm.xms_request_hma(0), Err(0x92));
    }

    #[test]
    fn test_xms_release_hma() {
        let (mut mm, _mem) = create_manager(1024);
        mm.xms_request_hma(64).unwrap();
        assert!(mm.xms_release_hma().is_ok());
        assert!(!mm.hma_is_allocated());
        assert_eq!(mm.xms_release_hma(), Err(0x93));
    }

    #[test]
    fn test_xms_allocate_basic() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        assert!(handle >= 1);
        let (largest, total) = mm.xms_query_free();
        assert!(total < 1024);
        assert!(largest <= total);
    }

    #[test]
    fn test_xms_allocate_reported_largest_free_block() {
        let (mut mm, _mem) = create_manager(16 * 1024);
        let (largest, _total) = mm.xms_query_free();
        let handle = mm.xms_allocate(largest).unwrap();
        let (_, _, size_kb) = mm.xms_handle_info(handle).unwrap();
        assert_eq!(size_kb, largest);
    }

    #[test]
    fn test_xms_allocate_reported_largest_fragmented_free_block() {
        let (mut mm, _mem) = create_manager(1024);
        let initial_free = mm.xms_query_free();

        let first = mm.xms_allocate(128).unwrap();
        let middle = mm.xms_allocate(256).unwrap();
        let second = mm.xms_allocate(128).unwrap();
        let third = mm.xms_allocate(448).unwrap();
        assert_eq!(mm.xms_query_free(), (0, 0));

        mm.xms_free(middle).unwrap();
        assert_eq!(mm.xms_query_free(), (256, 256));

        let largest = mm.xms_query_free().0;
        let exact = mm.xms_allocate(largest).unwrap();
        let (_, _, size_kb) = mm.xms_handle_info(exact).unwrap();
        assert_eq!(size_kb, largest);
        assert_eq!(mm.xms_query_free(), (0, 0));

        mm.xms_free(exact).unwrap();
        assert_eq!(mm.xms_query_free(), (256, 256));

        mm.xms_free(first).unwrap();
        mm.xms_free(second).unwrap();
        mm.xms_free(third).unwrap();
        assert_eq!(mm.xms_query_free(), initial_free);
    }

    #[test]
    fn test_xms_allocate_zero_kb() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.xms_allocate(0).unwrap();
        assert!(handle >= 1);
        let (_, _, size_kb) = mm.xms_handle_info(handle).unwrap();
        assert_eq!(size_kb, 0);
    }

    #[test]
    fn test_xms_allocate_multiple() {
        let (mut mm, _mem) = create_manager(1024);
        let h1 = mm.xms_allocate(100).unwrap();
        let h2 = mm.xms_allocate(200).unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_xms_allocate_exceeds_free() {
        let (mut mm, _mem) = create_manager(1024);
        assert_eq!(mm.xms_allocate(2000), Err(0xA0));
    }

    #[test]
    fn test_xms_allocate_exhausts_handles() {
        let (mut mm, _mem) = create_manager(1024);
        for _ in 0..MAX_XMS_HANDLES {
            mm.xms_allocate(0).unwrap();
        }
        assert_eq!(mm.xms_allocate(0), Err(0xA1));
    }

    #[test]
    fn test_xms_free_basic() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        let (_, before) = mm.xms_query_free();
        assert!(mm.xms_free(handle).is_ok());
        let (_, after) = mm.xms_query_free();
        assert!(after > before);
    }

    #[test]
    fn test_xms_free_invalid_handle() {
        let (mut mm, _mem) = create_manager(1024);
        assert_eq!(mm.xms_free(0), Err(0xA2));
        assert_eq!(mm.xms_free(200), Err(0xA2));
    }

    #[test]
    fn test_xms_free_locked() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        mm.xms_lock(handle).unwrap();
        assert_eq!(mm.xms_free(handle), Err(0xAB));
    }

    #[test]
    fn test_xms_lock_returns_address() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        let addr = mm.xms_lock(handle).unwrap();
        assert!(addr >= EXTENDED_RAM_BASE);
    }

    #[test]
    fn test_xms_lock_increments_count() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        mm.xms_lock(handle).unwrap();
        mm.xms_lock(handle).unwrap();
        let (lock_count, _, _) = mm.xms_handle_info(handle).unwrap();
        assert_eq!(lock_count, 2);
    }

    #[test]
    fn test_xms_lock_overflow() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        for _ in 0..255 {
            mm.xms_lock(handle).unwrap();
        }
        assert_eq!(mm.xms_lock(handle), Err(0xAC));
    }

    #[test]
    fn test_xms_unlock_basic() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        mm.xms_lock(handle).unwrap();
        assert!(mm.xms_unlock(handle).is_ok());
    }

    #[test]
    fn test_xms_unlock_not_locked() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        assert_eq!(mm.xms_unlock(handle), Err(0xAA));
    }

    #[test]
    fn test_xms_handle_info() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        let (lock_count, free_handles, size_kb) = mm.xms_handle_info(handle).unwrap();
        assert_eq!(lock_count, 0);
        assert_eq!(free_handles, (MAX_XMS_HANDLES - 1) as u16);
        assert_eq!(size_kb, 64);
    }

    #[test]
    fn test_xms_handle_info_invalid() {
        let (mm, _mem) = create_manager(1024);
        assert_eq!(mm.xms_handle_info(0), Err(0xA2));
    }

    #[test]
    fn test_xms_reallocate_grow() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        assert!(mm.xms_reallocate(handle, 128, &mut mem).is_ok());
        let (_, _, size) = mm.xms_handle_info(handle).unwrap();
        assert_eq!(size, 128);
    }

    #[test]
    fn test_xms_reallocate_shrink() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate(128).unwrap();
        assert!(mm.xms_reallocate(handle, 32, &mut mem).is_ok());
        let (_, _, size) = mm.xms_handle_info(handle).unwrap();
        assert_eq!(size, 32);
    }

    #[test]
    fn test_xms_reallocate_to_zero() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        assert!(mm.xms_reallocate(handle, 0, &mut mem).is_ok());
        let (_, _, size) = mm.xms_handle_info(handle).unwrap();
        assert_eq!(size, 0);
    }

    #[test]
    fn test_xms_reallocate_locked() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        mm.xms_lock(handle).unwrap();
        assert_eq!(mm.xms_reallocate(handle, 128, &mut mem), Err(0xAB));
    }

    #[test]
    fn test_xms_reallocate_preserves_data_on_grow() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate(4).unwrap();
        let addr = mm.xms_lock(handle).unwrap();
        let pattern = [0x11u8, 0x22, 0x33, 0x44, 0x55];
        mem.write_block(addr, &pattern);
        mm.xms_unlock(handle).unwrap();

        assert!(mm.xms_reallocate(handle, 16, &mut mem).is_ok());
        let new_addr = mm.xms_lock(handle).unwrap();
        let mut buf = [0u8; 5];
        mem.read_block(new_addr, &mut buf);
        assert_eq!(buf, pattern);
    }

    #[test]
    fn test_xms_reallocate_doom_loader_growth_keeps_locked_address_stable() {
        let (mut mm, mut mem) = create_manager(1024);

        // DX386 grows the initial Doom image allocation from 19 KB to 35 KB
        // after unlocking and then expects a subsequent lock to return the
        // same linear address again.
        let handle = mm.xms_allocate(19).unwrap();
        let addr = mm.xms_lock(handle).unwrap();
        let pattern = [0xD0u8, 0x0D, 0x38, 0x36];
        mem.write_block(addr, &pattern);
        mm.xms_unlock(handle).unwrap();

        assert!(mm.xms_reallocate(handle, 35, &mut mem).is_ok());

        let grown_addr = mm.xms_lock(handle).unwrap();
        assert_eq!(grown_addr, addr);
        let mut buf = [0u8; 4];
        mem.read_block(grown_addr, &mut buf);
        assert_eq!(buf, pattern);
    }

    #[test]
    fn test_xms_reallocate_preserves_data_on_shrink() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate(16).unwrap();
        let addr = mm.xms_lock(handle).unwrap();
        let pattern = [0xAAu8, 0xBB, 0xCC, 0xDD];
        mem.write_block(addr, &pattern);
        mm.xms_unlock(handle).unwrap();

        assert!(mm.xms_reallocate(handle, 4, &mut mem).is_ok());
        let new_addr = mm.xms_lock(handle).unwrap();
        let mut buf = [0u8; 4];
        mem.read_block(new_addr, &mut buf);
        assert_eq!(buf, pattern);
    }

    #[test]
    fn test_xms_reallocate_failure_leaves_handle_intact() {
        let (mut mm, mut mem) = create_manager(128);
        let handle = mm.xms_allocate(32).unwrap();
        let addr = mm.xms_lock(handle).unwrap();
        let pattern = [0xDEu8, 0xAD, 0xBE, 0xEF];
        mem.write_block(addr, &pattern);
        mm.xms_unlock(handle).unwrap();

        // Request far more than the pool can hold.
        assert_eq!(mm.xms_reallocate(handle, 4096, &mut mem), Err(0xA0));

        // Handle must still be valid at its original size, with data intact.
        let (lock_count, _, size) = mm.xms_handle_info(handle).unwrap();
        assert_eq!(size, 32);
        assert_eq!(lock_count, 0);
        let restored_addr = mm.xms_lock(handle).unwrap();
        let mut buf = [0u8; 4];
        mem.read_block(restored_addr, &mut buf);
        assert_eq!(buf, pattern);
    }

    #[test]
    fn test_xms_move_rejects_src_offset_plus_length_overflow() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();

        let src_offset: u32 = 64 * 1024 - 2;
        let params_addr = 0x6000u32;
        mem.write_word(params_addr, 4);
        mem.write_word(params_addr + 2, 0);
        mem.write_word(params_addr + 4, handle);
        mem.write_word(params_addr + 6, src_offset as u16);
        mem.write_word(params_addr + 8, (src_offset >> 16) as u16);
        mem.write_word(params_addr + 10, 0);
        mem.write_word(params_addr + 12, 0);
        mem.write_word(params_addr + 14, 0);

        assert_eq!(mm.xms_move(&mut mem, params_addr), Err(0xA4));
    }

    #[test]
    fn test_xms_move_rejects_dst_offset_plus_length_overflow() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();

        let dst_offset: u32 = 64 * 1024 - 2;
        let params_addr = 0x6000u32;
        mem.write_word(params_addr, 4);
        mem.write_word(params_addr + 2, 0);
        mem.write_word(params_addr + 4, 0);
        mem.write_word(params_addr + 6, 0x5000);
        mem.write_word(params_addr + 8, 0);
        mem.write_word(params_addr + 10, handle);
        mem.write_word(params_addr + 12, dst_offset as u16);
        mem.write_word(params_addr + 14, (dst_offset >> 16) as u16);

        assert_eq!(mm.xms_move(&mut mem, params_addr), Err(0xA6));
    }

    #[test]
    fn test_xms_move_at_exact_boundary() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate(4).unwrap();
        let size_bytes = 4 * 1024;

        let pattern = [0x01u8, 0x02, 0x03, 0x04];
        mem.write_block(0x5000, &pattern);

        let params_addr = 0x6000u32;
        mem.write_word(params_addr, 4);
        mem.write_word(params_addr + 2, 0);
        mem.write_word(params_addr + 4, 0);
        mem.write_word(params_addr + 6, 0x5000);
        mem.write_word(params_addr + 8, 0);
        mem.write_word(params_addr + 10, handle);
        mem.write_word(params_addr + 12, (size_bytes - 4) as u16);
        mem.write_word(params_addr + 14, ((size_bytes - 4) >> 16) as u16);

        assert!(mm.xms_move(&mut mem, params_addr).is_ok());

        let addr = mm.xms_lock(handle).unwrap();
        let mut buf = [0u8; 4];
        mem.read_block(addr + size_bytes - 4, &mut buf);
        assert_eq!(buf, pattern);
    }

    #[test]
    fn test_ems_page_map_size_is_16() {
        let (mm, _mem) = create_manager(1024);
        assert_eq!(mm.ems_page_map_size(), 16);
    }

    #[test]
    fn test_ems_reallocate_grow_failure_leaves_handle_intact() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.ems_allocate_pages(2).unwrap();
        // Consume most of the pool so a large grow request exceeds free
        // without exceeding physical total (which would return 0x87).
        mm.ems_allocate_pages(50).unwrap();
        let (free_before, _) = mm.ems_unallocated_pages();

        // Ask for more pages than are currently free but still within the
        // physical pool (total=64, requesting 60 -> extra=58 > free=12).
        assert_eq!(mm.ems_reallocate(handle, 60), Err(0x88));

        // Handle unchanged.
        assert_eq!(mm.ems_handle_pages(handle).unwrap(), 2);
        let (free_after, _) = mm.ems_unallocated_pages();
        assert_eq!(free_after, free_before);
    }

    #[test]
    fn test_ems_move_addr_overflow() {
        let (mm, mut mem) = create_manager(1024);
        let params = EmsMoveParams {
            region_length: 0xFFFF_FFFF,
            src_type: 0,
            src_handle: 0,
            src_offset: 0xFFFF,
            src_seg_page: 0xFFFF,
            dst_type: 0,
            dst_handle: 0,
            dst_offset: 0,
            dst_seg_page: 0,
        };
        assert_eq!(mm.ems_move_memory(&params, &mut mem), 0x96);
    }

    #[test]
    fn test_xms_move_conventional_to_xms() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();

        let pattern = [0xDE, 0xAD, 0xBE, 0xEF];
        mem.write_block(0x5000, &pattern);

        let params_addr = 0x6000u32;
        mem.write_word(params_addr, 4);
        mem.write_word(params_addr + 2, 0);
        mem.write_word(params_addr + 4, 0);
        mem.write_word(params_addr + 6, 0x5000);
        mem.write_word(params_addr + 8, 0);
        mem.write_word(params_addr + 10, handle);
        mem.write_word(params_addr + 12, 0);
        mem.write_word(params_addr + 14, 0);

        assert!(mm.xms_move(&mut mem, params_addr).is_ok());

        let addr = mm.xms_lock(handle).unwrap();
        let mut buf = [0u8; 4];
        mem.read_block(addr, &mut buf);
        assert_eq!(buf, pattern);
    }

    #[test]
    fn test_xms_move_xms_to_xms() {
        let (mut mm, mut mem) = create_manager(1024);
        let h1 = mm.xms_allocate(64).unwrap();
        let h2 = mm.xms_allocate(64).unwrap();

        let addr1 = mm.xms_lock(h1).unwrap();
        let pattern = [0xCA, 0xFE, 0xBA, 0xBE];
        mem.write_block(addr1, &pattern);

        let params_addr = 0x6000u32;
        mem.write_word(params_addr, 4);
        mem.write_word(params_addr + 2, 0);
        mem.write_word(params_addr + 4, h1);
        mem.write_word(params_addr + 6, 0);
        mem.write_word(params_addr + 8, 0);
        mem.write_word(params_addr + 10, h2);
        mem.write_word(params_addr + 12, 0);
        mem.write_word(params_addr + 14, 0);

        assert!(mm.xms_move(&mut mem, params_addr).is_ok());

        let addr2 = mm.xms_lock(h2).unwrap();
        let mut buf = [0u8; 4];
        mem.read_block(addr2, &mut buf);
        assert_eq!(buf, pattern);
    }

    #[test]
    fn test_xms_move_zero_length() {
        let (mm, mut mem) = create_manager(1024);
        let params_addr = 0x6000u32;
        mem.write_word(params_addr, 0);
        mem.write_word(params_addr + 2, 0);
        mem.write_word(params_addr + 4, 0);
        mem.write_word(params_addr + 6, 0);
        mem.write_word(params_addr + 8, 0);
        mem.write_word(params_addr + 10, 0);
        mem.write_word(params_addr + 12, 0);
        mem.write_word(params_addr + 14, 0);
        assert!(mm.xms_move(&mut mem, params_addr).is_ok());
    }

    #[test]
    fn test_umb_allocate_basic() {
        let (mm, mut mem) = create_manager(1024);
        let (segment, size) = mm.umb_allocate(16, &mut mem).unwrap();
        assert!(segment > UMB_FIRST_MCB_SEGMENT);
        assert!(size >= 16);
    }

    #[test]
    fn test_umb_allocate_disabled() {
        let (mm, mut mem) = create_manager_selective(1024, false, false);
        assert_eq!(mm.umb_allocate(16, &mut mem), Err((0xB1, 0)));
    }

    #[test]
    fn test_umb_allocate_too_large() {
        let (mm, mut mem) = create_manager(1024);
        let result = mm.umb_allocate(0xFFFF, &mut mem);
        assert!(result.is_err());
        let (code, largest) = result.unwrap_err();
        assert_eq!(code, 0xB0);
        assert!(largest > 0);
    }

    #[test]
    fn test_umb_free_basic() {
        let (mm, mut mem) = create_manager(1024);
        let (segment, _) = mm.umb_allocate(16, &mut mem).unwrap();
        assert!(mm.umb_free(segment, &mut mem).is_ok());
    }

    #[test]
    fn test_umb_reallocate() {
        let (mm, mut mem) = create_manager(1024);
        let (segment, _) = mm.umb_allocate(32, &mut mem).unwrap();
        assert!(mm.umb_reallocate(segment, 16, &mut mem).is_ok());
    }

    #[test]
    fn test_umb_reallocate_failure_reports_largest_free_umb_without_resizing() {
        let (mm, mut mem) = create_manager_selective(1024, false, true);
        let (segment, _) = mm.umb_allocate(4, &mut mem).unwrap();
        let (_second_segment, _) = mm.umb_allocate(4, &mut mem).unwrap();
        let expected_largest = memory::read_mcb_size_pub(&mem, UMB_FIRST_MCB_SEGMENT + 10);

        assert_eq!(
            mm.umb_reallocate(segment, 0xFFFF, &mut mem),
            Err((0xB0, expected_largest))
        );
        assert_eq!(memory::read_mcb_size_pub(&mem, segment - 1), 4);
    }

    #[test]
    fn test_umb_reallocate_returns_b1_when_no_umbs_free() {
        let (mm, mut mem) = create_manager_selective(1024, false, true);
        let (first_segment, _) = mm.umb_allocate(4, &mut mem).unwrap();
        let (_, largest) = mm.umb_allocate(0xFFFF, &mut mem).unwrap_err();
        let (_, _) = mm.umb_allocate(largest, &mut mem).unwrap();

        assert_eq!(
            memory::largest_free_block_paragraphs_pub(&mem, UMB_FIRST_MCB_SEGMENT),
            0
        );

        assert_eq!(
            mm.umb_reallocate(first_segment, 0xFFFF, &mut mem),
            Err((0xB1, 0))
        );
    }

    #[test]
    fn test_umb_reallocate_returns_b2_for_invalid_segment() {
        let (mm, mut mem) = create_manager_selective(1024, false, true);
        let _ = mm.umb_allocate(4, &mut mem).unwrap();

        assert_eq!(mm.umb_reallocate(0xEEEE, 8, &mut mem), Err((0xB2, 0)));
    }

    #[test]
    fn test_ems_and_xms_share_pool() {
        let (mut mm, _mem) = create_manager(1024);
        let (free_pages, _) = mm.ems_unallocated_pages();
        mm.ems_allocate_pages(free_pages).unwrap();

        // Pool is now fully consumed by EMS. XMS allocation should fail.
        assert!(mm.xms_allocate(1).is_err());
    }

    #[test]
    fn test_allocate_deallocate_cycle() {
        let (mut mm, mut mem) = create_manager(1024);
        let (_, initial_total) = mm.xms_query_free();

        let ems_h = mm.ems_allocate_pages(4).unwrap();
        let xms_h = mm.xms_allocate(64).unwrap();

        let (free_after_alloc, _) = mm.xms_query_free();
        assert!(free_after_alloc < initial_total);

        mm.xms_free(xms_h).unwrap();
        mm.ems_deallocate(ems_h, &mut mem);

        let (free_after_dealloc, _) = mm.xms_query_free();
        assert_eq!(free_after_dealloc, initial_total);
    }

    #[test]
    fn test_xms_32_enabled_flag() {
        let (mm, _mem) = create_manager(1024);
        assert!(mm.is_xms_32_enabled());

        let mut mem = MockMemory::new(1024);
        let mm2 = MemoryManager::new(1024 * 1024, true, true, false, &mut mem);
        assert!(!mm2.is_xms_32_enabled());
    }

    #[test]
    fn test_xms_query_free_32() {
        let (mm, _mem) = create_manager(1024);
        let (largest, total) = mm.xms_query_free_32();
        assert_eq!(largest, 960);
        assert_eq!(total, 960);
    }

    #[test]
    fn test_xms_allocate_32_basic() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.xms_allocate_32(64).unwrap();
        assert!(handle >= 1);
        let (_, _, size_kb) = mm.xms_handle_info_32(handle).unwrap();
        assert_eq!(size_kb, 64);
    }

    #[test]
    fn test_xms_allocate_32_zero() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.xms_allocate_32(0).unwrap();
        assert!(handle >= 1);
        let (_, _, size_kb) = mm.xms_handle_info_32(handle).unwrap();
        assert_eq!(size_kb, 0);
    }

    #[test]
    fn test_xms_allocate_32_exceeds_free() {
        let (mut mm, _mem) = create_manager(1024);
        assert_eq!(mm.xms_allocate_32(2000), Err(0xA0));
    }

    #[test]
    fn test_xms_allocate_32_overflow_rejected() {
        let (mut mm, _mem) = create_manager(1024);
        assert_eq!(mm.xms_allocate_32(u32::MAX), Err(0xA0));
    }

    #[test]
    fn test_xms_allocate_32_exhausts_handles() {
        let (mut mm, _mem) = create_manager(1024);
        for _ in 0..MAX_XMS_HANDLES {
            mm.xms_allocate_32(0).unwrap();
        }
        assert_eq!(mm.xms_allocate_32(0), Err(0xA1));
    }

    #[test]
    fn test_xms_handle_info_32() {
        let (mut mm, _mem) = create_manager(1024);
        let handle = mm.xms_allocate_32(64).unwrap();
        let (lock_count, free_handles, size_kb) = mm.xms_handle_info_32(handle).unwrap();
        assert_eq!(lock_count, 0);
        assert_eq!(free_handles, (MAX_XMS_HANDLES - 1) as u16);
        assert_eq!(size_kb, 64);
    }

    #[test]
    fn test_xms_handle_info_32_invalid() {
        let (mm, _mem) = create_manager(1024);
        assert_eq!(mm.xms_handle_info_32(0), Err(0xA2));
    }

    #[test]
    fn test_xms_reallocate_32_grow() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate_32(64).unwrap();
        assert!(mm.xms_reallocate_32(handle, 128, &mut mem).is_ok());
        let (_, _, size) = mm.xms_handle_info_32(handle).unwrap();
        assert_eq!(size, 128);
    }

    #[test]
    fn test_xms_reallocate_32_shrink() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate_32(128).unwrap();
        assert!(mm.xms_reallocate_32(handle, 32, &mut mem).is_ok());
        let (_, _, size) = mm.xms_handle_info_32(handle).unwrap();
        assert_eq!(size, 32);
    }

    #[test]
    fn test_xms_reallocate_32_to_zero() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate_32(64).unwrap();
        assert!(mm.xms_reallocate_32(handle, 0, &mut mem).is_ok());
        let (_, _, size) = mm.xms_handle_info_32(handle).unwrap();
        assert_eq!(size, 0);
    }

    #[test]
    fn test_xms_reallocate_32_locked() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate_32(64).unwrap();
        mm.xms_lock(handle).unwrap();
        assert_eq!(mm.xms_reallocate_32(handle, 128, &mut mem), Err(0xAB));
    }

    #[test]
    fn test_xms_reallocate_32_overflow_rejected() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate_32(64).unwrap();

        assert_eq!(mm.xms_reallocate_32(handle, u32::MAX, &mut mem), Err(0xA0));
        assert_eq!(mm.xms_handle_info_32(handle).unwrap().2, 64);
    }

    #[test]
    fn test_ems_enable_os_functions_first_call() {
        let (mut mm, _mem) = create_manager(1024);
        let result = mm.ems_enable_os_functions(0);
        assert!(result.is_ok());
        let key = result.unwrap();
        assert!(key.is_some());
        assert_ne!(key.unwrap(), 0);
    }

    #[test]
    fn test_ems_enable_os_functions_subsequent_call() {
        let (mut mm, _mem) = create_manager(1024);
        let key = mm.ems_enable_os_functions(0).unwrap().unwrap();
        let result = mm.ems_enable_os_functions(key);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn test_ems_enable_os_functions_wrong_key() {
        let (mut mm, _mem) = create_manager(1024);
        mm.ems_enable_os_functions(0).unwrap();
        assert_eq!(mm.ems_enable_os_functions(0xDEAD), Err(0xA4));
    }

    #[test]
    fn test_ems_disable_os_functions_first_call() {
        let (mut mm, _mem) = create_manager(1024);
        let result = mm.ems_disable_os_functions(0);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_ems_disable_os_functions_correct_key() {
        let (mut mm, _mem) = create_manager(1024);
        let key = mm.ems_enable_os_functions(0).unwrap().unwrap();
        assert_eq!(mm.ems_disable_os_functions(key), Ok(None));
    }

    #[test]
    fn test_ems_disable_os_functions_wrong_key() {
        let (mut mm, _mem) = create_manager(1024);
        mm.ems_enable_os_functions(0).unwrap();
        assert_eq!(mm.ems_disable_os_functions(0xDEAD), Err(0xA4));
    }

    #[test]
    fn test_ems_return_os_access_key() {
        let (mut mm, _mem) = create_manager(1024);
        let key = mm.ems_enable_os_functions(0).unwrap().unwrap();
        assert!(mm.ems_return_os_access_key(key).is_ok());
    }

    #[test]
    fn test_ems_return_os_access_key_wrong() {
        let (mut mm, _mem) = create_manager(1024);
        mm.ems_enable_os_functions(0).unwrap();
        assert_eq!(mm.ems_return_os_access_key(0xDEAD), Err(0xA4));
    }

    #[test]
    fn test_ems_return_os_access_key_enables_new_key() {
        let (mut mm, _mem) = create_manager(1024);
        let key1 = mm.ems_enable_os_functions(0).unwrap().unwrap();
        mm.ems_return_os_access_key(key1).unwrap();
        let key2 = mm.ems_enable_os_functions(0).unwrap().unwrap();
        assert_ne!(key2, 0);
    }

    fn write_xms_move_params(
        mem: &mut MockMemory,
        params_addr: u32,
        length: u32,
        src_handle: u16,
        src_offset: u32,
        dst_handle: u16,
        dst_offset: u32,
    ) {
        mem.write_word(params_addr, length as u16);
        mem.write_word(params_addr + 2, (length >> 16) as u16);
        mem.write_word(params_addr + 4, src_handle);
        mem.write_word(params_addr + 6, src_offset as u16);
        mem.write_word(params_addr + 8, (src_offset >> 16) as u16);
        mem.write_word(params_addr + 10, dst_handle);
        mem.write_word(params_addr + 12, dst_offset as u16);
        mem.write_word(params_addr + 14, (dst_offset >> 16) as u16);
    }

    #[test]
    fn test_xms_move_backward_overlap_returns_a8() {
        // Same XMS handle, src > dst, ranges overlap -> invalid per XMS 3.0
        // (only forward moves are guaranteed to work).
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        let params_addr = 0x6000u32;
        write_xms_move_params(&mut mem, params_addr, 2048, handle, 1024, handle, 0);
        assert_eq!(mm.xms_move(&mut mem, params_addr), Err(0xA8));
    }

    #[test]
    fn test_xms_move_forward_overlap_succeeds() {
        // src < dst overlap is the spec's guaranteed-safe direction.
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        let addr = mm.xms_lock(handle).unwrap();
        let pattern: Vec<u8> = (0..2048).map(|i| (i & 0xFF) as u8).collect();
        mem.write_block(addr, &pattern);

        let params_addr = 0x6000u32;
        write_xms_move_params(&mut mem, params_addr, 2048, handle, 0, handle, 1024);
        assert!(mm.xms_move(&mut mem, params_addr).is_ok());

        let mut buf = vec![0u8; 2048];
        mem.read_block(addr + 1024, &mut buf);
        assert_eq!(buf, pattern);
    }

    #[test]
    fn test_xms_move_same_handle_non_overlap_succeeds() {
        let (mut mm, mut mem) = create_manager(1024);
        let handle = mm.xms_allocate(64).unwrap();
        let params_addr = 0x6000u32;
        write_xms_move_params(&mut mem, params_addr, 1024, handle, 0, handle, 8192);
        assert!(mm.xms_move(&mut mem, params_addr).is_ok());
    }

    #[test]
    fn test_xms_move_different_handles_no_overlap_check() {
        // Two separate EMBs never alias, so the overlap check must not
        // reject an otherwise-valid move that uses identical offsets on
        // each side.
        let (mut mm, mut mem) = create_manager(1024);
        let h1 = mm.xms_allocate(64).unwrap();
        let h2 = mm.xms_allocate(64).unwrap();
        let params_addr = 0x6000u32;
        write_xms_move_params(&mut mem, params_addr, 2048, h1, 1024, h2, 0);
        assert!(mm.xms_move(&mut mem, params_addr).is_ok());
    }

    #[test]
    fn test_xms_request_hma_app_override_bypasses_hmamin() {
        let (mut mm, _mem) = create_manager(1024);
        // Set HMAMIN to a value that would normally reject small requests;
        // applications signalling DX=0xFFFF must still succeed.
        mm.set_hmamin_kb(32);
        assert!(mm.xms_request_hma(0xFFFF).is_ok());
        assert!(mm.hma_is_allocated());
    }

    #[test]
    fn test_xms_request_hma_tsr_below_hmamin_fails() {
        let (mut mm, _mem) = create_manager(1024);
        mm.set_hmamin_kb(32);
        // TSR-style caller passes an actual byte size; below HMAMIN -> 0x92.
        assert_eq!(mm.xms_request_hma(16 * 1024), Err(0x92));
    }

    #[test]
    fn test_xms_request_hma_tsr_above_hmamin_succeeds() {
        let (mut mm, _mem) = create_manager(1024);
        mm.set_hmamin_kb(32);
        assert!(mm.xms_request_hma(32 * 1024).is_ok());
    }
}
