//! INT 67h: Expanded Memory Manager (EMS 4.0).

use crate::{
    CpuAccess, MemoryAccess, NeetanDos,
    memory::memory_manager::{EmsMoveParams, EmsPageMapCallContext},
    tables,
};

impl NeetanDos {
    pub(crate) fn int67h(&mut self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let mm = match self.state.memory_manager {
            Some(ref mut mm) if mm.is_ems_enabled() => mm,
            _ => {
                cpu.set_ax((cpu.ax() & 0x00FF) | 0x8400);
                return;
            }
        };

        let ah = (cpu.ax() >> 8) as u8;
        match ah {
            0x40 => {
                let status = mm.ems_status();
                cpu.set_ax((cpu.ax() & 0x00FF) | ((status as u16) << 8));
            }
            0x41 => {
                let segment = mm.ems_page_frame_segment();
                cpu.set_bx(segment);
                cpu.set_ax(cpu.ax() & 0x00FF);
            }
            0x42 => {
                let (free, total) = mm.ems_unallocated_pages();
                cpu.set_bx(free);
                cpu.set_dx(total);
                cpu.set_ax(cpu.ax() & 0x00FF);
            }
            0x43 => {
                let count = cpu.bx();
                match mm.ems_allocate_pages(count) {
                    Ok(handle) => {
                        cpu.set_dx(handle);
                        cpu.set_ax(cpu.ax() & 0x00FF);
                    }
                    Err(code) => {
                        cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                    }
                }
            }
            0x44 => {
                let physical_page = cpu.ax() as u8;
                let logical_page = cpu.bx();
                let handle = cpu.dx();
                let status = if logical_page == 0xFFFF {
                    mm.ems_unmap_page(physical_page, memory)
                } else {
                    mm.ems_map_page(handle, logical_page, physical_page, memory)
                };
                cpu.set_ax((cpu.ax() & 0x00FF) | ((status as u16) << 8));
            }
            0x45 => {
                let handle = cpu.dx();
                let status = mm.ems_deallocate(handle, memory);
                cpu.set_ax((cpu.ax() & 0x00FF) | ((status as u16) << 8));
            }
            0x46 => {
                cpu.set_ax(0x0040);
            }
            0x47 => {
                let handle = cpu.dx();
                let status = mm.ems_save_page_map(handle);
                cpu.set_ax((cpu.ax() & 0x00FF) | ((status as u16) << 8));
            }
            0x48 => {
                let handle = cpu.dx();
                let status = mm.ems_restore_page_map(handle, memory);
                cpu.set_ax((cpu.ax() & 0x00FF) | ((status as u16) << 8));
            }
            0x4B => {
                let count = mm.ems_handle_count();
                cpu.set_bx(count);
                cpu.set_ax(cpu.ax() & 0x00FF);
            }
            0x4C => {
                let handle = cpu.dx();
                match mm.ems_handle_pages(handle) {
                    Ok(pages) => {
                        cpu.set_bx(pages);
                        cpu.set_ax(cpu.ax() & 0x00FF);
                    }
                    Err(code) => {
                        cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                    }
                }
            }
            0x4D => {
                let buffer_addr = ((cpu.es() as u32) << 4) + cpu.di() as u32;
                let handles = mm.ems_all_handle_pages();
                for (i, &(handle, pages)) in handles.iter().enumerate() {
                    let entry_addr = buffer_addr + (i as u32) * 4;
                    memory.write_word(entry_addr, handle);
                    memory.write_word(entry_addr + 2, pages);
                }
                cpu.set_bx(handles.len() as u16);
                cpu.set_ax(cpu.ax() & 0x00FF);
            }
            0x4F => {
                let al = cpu.ax() as u8;
                match al {
                    0x00 => {
                        // Get Partial Page Map: save partial mapping context
                        // for segments listed at DS:SI into dest_array at ES:DI.
                        // partial_page_map: WORD count, WORD[] segments.
                        let src_addr = ((cpu.ds() as u32) << 4) + cpu.si() as u32;
                        let count = memory.read_word(src_addr);
                        let count_usize = count as usize;
                        if count_usize > 4 {
                            cpu.set_ax((cpu.ax() & 0x00FF) | 0xA300);
                        } else {
                            let mut segments = Vec::with_capacity(count_usize);
                            for i in 0..count_usize {
                                segments.push(memory.read_word(src_addr + 2 + (i as u32) * 2));
                            }
                            match mm.ems_get_partial_page_map(&segments) {
                                Ok(buf) => {
                                    let dest = ((cpu.es() as u32) << 4) + cpu.di() as u32;
                                    memory.write_block(dest, &buf);
                                    cpu.set_ax(cpu.ax() & 0x00FF);
                                }
                                Err(code) => {
                                    cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                                }
                            }
                        }
                    }
                    0x01 => {
                        // Set Partial Page Map: source_array at DS:SI.
                        let src_addr = ((cpu.ds() as u32) << 4) + cpu.si() as u32;
                        let count = memory.read_byte(src_addr) as usize;
                        if count > 4 {
                            cpu.set_ax((cpu.ax() & 0x00FF) | 0xA300);
                        } else {
                            let total = 1 + count * 6;
                            let mut buf = vec![0u8; total];
                            memory.read_block(src_addr, &mut buf);
                            let status = mm.ems_set_partial_page_map(&buf, memory);
                            cpu.set_ax((cpu.ax() & 0x00FF) | ((status as u16) << 8));
                        }
                    }
                    0x02 => {
                        // Get Size of Partial Page Map Save Array.
                        // BX = number of pages in the partial array.
                        let count = cpu.bx();
                        match mm.ems_partial_page_map_size(count) {
                            Ok(size) => {
                                // AL = size_of_partial_save_array, AH = 0.
                                cpu.set_ax(size as u16);
                            }
                            Err(code) => {
                                cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                            }
                        }
                    }
                    _ => {
                        cpu.set_ax((cpu.ax() & 0x00FF) | 0x8F00);
                    }
                }
            }
            0x55 => {
                // ALTER PAGE MAP & JUMP: apply mapping then JMP FAR to target.
                // Spec: EMS 4.0 Function 22.
                let al = cpu.ax() as u8;
                if al > 0x01 {
                    cpu.set_ax((cpu.ax() & 0x00FF) | 0x8F00);
                    return;
                }
                let handle = cpu.dx();
                let struct_addr = ((cpu.ds() as u32) << 4) + cpu.si() as u32;
                let target_ip = memory.read_word(struct_addr);
                let target_cs = memory.read_word(struct_addr + 2);
                let map_len = memory.read_byte(struct_addr + 4);
                if usize::from(map_len) > 4 {
                    cpu.set_ax((cpu.ax() & 0x00FF) | 0x8B00);
                    return;
                }
                let map_off = memory.read_word(struct_addr + 5);
                let map_seg = memory.read_word(struct_addr + 7);
                let map_addr = ((map_seg as u32) << 4) + map_off as u32;
                let status = apply_log_phys_map(mm, memory, handle, al, map_len, map_addr);
                if status == 0 {
                    // Modify the IRET frame (IP at SS:SP, CS at SS:SP+2,
                    // FLAGS at SS:SP+4) to jump to target_cs:target_ip
                    // preserving the caller's FLAGS.
                    let iret_base = ((cpu.ss() as u32) << 4) + cpu.sp() as u32;
                    memory.write_word(iret_base, target_ip);
                    memory.write_word(iret_base + 2, target_cs);
                    cpu.set_ax(cpu.ax() & 0x00FF);
                } else {
                    cpu.set_ax((cpu.ax() & 0x00FF) | ((status as u16) << 8));
                }
            }
            0x56 => {
                let al = cpu.ax() as u8;
                match al {
                    0x00 | 0x01 => {
                        let handle = cpu.dx();
                        let struct_addr = ((cpu.ds() as u32) << 4) + cpu.si() as u32;
                        let target_ip = memory.read_word(struct_addr);
                        let target_cs = memory.read_word(struct_addr + 2);
                        let new_len = memory.read_byte(struct_addr + 4);
                        let new_off = memory.read_word(struct_addr + 5);
                        let new_seg = memory.read_word(struct_addr + 7);
                        let old_len = memory.read_byte(struct_addr + 9);
                        if usize::from(new_len) > 4 || usize::from(old_len) > 4 {
                            cpu.set_ax((cpu.ax() & 0x00FF) | 0x8B00);
                            return;
                        }
                        let old_off = memory.read_word(struct_addr + 10);
                        let old_seg = memory.read_word(struct_addr + 12);
                        let new_addr = ((new_seg as u32) << 4) + new_off as u32;
                        let old_addr = ((old_seg as u32) << 4) + old_off as u32;
                        let status = apply_log_phys_map(mm, memory, handle, al, new_len, new_addr);
                        if status == 0 {
                            mm.ems_push_page_map_call_context(EmsPageMapCallContext {
                                segphys_mode: al,
                                handle,
                                old_len,
                                old_map_addr: old_addr,
                            });
                            let original_sp = cpu.sp();
                            let iret_base = ((cpu.ss() as u32) << 4) + original_sp as u32;
                            let original_ip = memory.read_word(iret_base);
                            let original_cs = memory.read_word(iret_base + 2);
                            let original_flags = memory.read_word(iret_base + 4);
                            let new_sp = original_sp.wrapping_sub(10);
                            let new_iret_base = ((cpu.ss() as u32) << 4) + new_sp as u32;

                            memory.write_word(new_iret_base, target_ip);
                            memory.write_word(new_iret_base + 2, target_cs);
                            memory.write_word(new_iret_base + 4, original_flags);
                            memory.write_word(new_iret_base + 6, tables::EMS_PGMAPRET_STUB_OFFSET);
                            memory.write_word(new_iret_base + 8, tables::EMS_PGMAPRET_STUB_SEGMENT);
                            memory.write_word(new_iret_base + 10, original_ip);
                            memory.write_word(new_iret_base + 12, original_cs);
                            memory.write_word(new_iret_base + 14, original_flags);
                            cpu.set_sp(new_sp);
                            cpu.set_ax(cpu.ax() & 0x00FF);
                        } else {
                            cpu.set_ax((cpu.ax() & 0x00FF) | ((status as u16) << 8));
                        }
                    }
                    0x02 => {
                        cpu.set_bx(14);
                        cpu.set_ax(cpu.ax() & 0x00FF);
                    }
                    _ => {
                        cpu.set_ax((cpu.ax() & 0x00FF) | 0x8F00);
                    }
                }
            }
            0x4E => {
                let al = cpu.ax() as u8;
                match al {
                    0x00 => {
                        let dest_addr = ((cpu.es() as u32) << 4) + cpu.di() as u32;
                        let map = mm.ems_get_page_map();
                        for (i, slot) in map.iter().enumerate() {
                            let addr = dest_addr + (i as u32) * 4;
                            match slot {
                                Some(m) => {
                                    memory.write_word(addr, m.handle);
                                    memory.write_word(addr + 2, m.logical_page);
                                }
                                None => {
                                    memory.write_word(addr, 0xFFFF);
                                    memory.write_word(addr + 2, 0xFFFF);
                                }
                            }
                        }
                        cpu.set_ax(cpu.ax() & 0x00FF);
                    }
                    0x01 => {
                        let src_addr = ((cpu.ds() as u32) << 4) + cpu.si() as u32;
                        match read_page_map_from_memory(mm, memory, src_addr) {
                            Ok(map) => {
                                mm.ems_set_page_map(map, memory);
                                cpu.set_ax(cpu.ax() & 0x00FF);
                            }
                            Err(code) => {
                                cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                            }
                        }
                    }
                    0x02 => {
                        let dest_addr = ((cpu.es() as u32) << 4) + cpu.di() as u32;
                        let old_map = mm.ems_get_page_map();
                        write_page_map_to_memory(memory, dest_addr, &old_map);

                        let src_addr = ((cpu.ds() as u32) << 4) + cpu.si() as u32;
                        match read_page_map_from_memory(mm, memory, src_addr) {
                            Ok(new_map) => {
                                mm.ems_set_page_map(new_map, memory);
                                cpu.set_ax(cpu.ax() & 0x00FF);
                            }
                            Err(code) => {
                                cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                            }
                        }
                    }
                    0x03 => {
                        cpu.set_ax(mm.ems_page_map_size());
                    }
                    _ => {
                        cpu.set_ax((cpu.ax() & 0x00FF) | 0x8F00);
                    }
                }
            }
            0x50 => {
                let al = cpu.ax() as u8;
                let handle = cpu.dx();
                let count = cpu.cx();
                let src_addr = ((cpu.ds() as u32) << 4) + cpu.si() as u32;
                match al {
                    0x00 | 0x01 => {
                        if count as usize > 4 {
                            cpu.set_ax((cpu.ax() & 0x00FF) | 0x8B00);
                            return;
                        }
                        let mut status = 0x00u8;
                        for i in 0..count {
                            let entry_addr = src_addr + (i as u32) * 4;
                            let logical_page = memory.read_word(entry_addr);
                            let physical_page = memory.read_word(entry_addr + 2);
                            let phys = if al == 0x00 {
                                physical_page as u8
                            } else {
                                match physical_page {
                                    0xC000 => 0,
                                    0xC400 => 1,
                                    0xC800 => 2,
                                    0xCC00 => 3,
                                    _ => {
                                        status = 0x8B;
                                        break;
                                    }
                                }
                            };
                            let result = if logical_page == 0xFFFF {
                                mm.ems_unmap_page(phys, memory)
                            } else {
                                mm.ems_map_page(handle, logical_page, phys, memory)
                            };
                            if result != 0x00 {
                                status = result;
                                break;
                            }
                        }
                        cpu.set_ax((cpu.ax() & 0x00FF) | ((status as u16) << 8));
                    }
                    _ => {
                        cpu.set_ax((cpu.ax() & 0x00FF) | 0x8F00);
                    }
                }
            }
            0x51 => {
                let new_count = cpu.bx();
                let handle = cpu.dx();
                match mm.ems_reallocate(handle, new_count) {
                    Ok(actual) => {
                        cpu.set_bx(actual);
                        cpu.set_ax(cpu.ax() & 0x00FF);
                    }
                    Err(code) => {
                        let prior = mm.ems_handle_pages(handle).unwrap_or(0);
                        cpu.set_bx(prior);
                        cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                    }
                }
            }
            0x52 => {
                let al = cpu.ax() as u8;
                match al {
                    0x00 => {
                        let handle = cpu.dx();
                        if mm.ems_is_valid_handle(handle) {
                            cpu.set_ax(0x0000);
                        } else {
                            cpu.set_ax(0x8300);
                        }
                    }
                    0x01 => {
                        let handle = cpu.dx();
                        let bl = cpu.bx() as u8;
                        if !mm.ems_is_valid_handle(handle) {
                            cpu.set_ax((cpu.ax() & 0x00FF) | 0x8300);
                        } else if bl == 0 {
                            cpu.set_ax(cpu.ax() & 0x00FF);
                        } else if bl == 1 {
                            cpu.set_ax((cpu.ax() & 0x00FF) | 0x9100);
                        } else {
                            cpu.set_ax((cpu.ax() & 0x00FF) | 0x9000);
                        }
                    }
                    0x02 => {
                        // AL = 0: only volatile handles supported.
                        cpu.set_ax(0x0000);
                    }
                    _ => {
                        cpu.set_ax((cpu.ax() & 0x00FF) | 0x8F00);
                    }
                }
            }
            0x53 => {
                let al = cpu.ax() as u8;
                match al {
                    0x00 => {
                        let handle = cpu.dx();
                        match mm.ems_handle_name(handle) {
                            Ok(name) => {
                                let dest = ((cpu.es() as u32) << 4) + cpu.di() as u32;
                                memory.write_block(dest, &name);
                                cpu.set_ax(cpu.ax() & 0x00FF);
                            }
                            Err(code) => {
                                cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                            }
                        }
                    }
                    0x01 => {
                        let handle = cpu.dx();
                        let src = ((cpu.ds() as u32) << 4) + cpu.si() as u32;
                        let mut name = [0u8; 8];
                        memory.read_block(src, &mut name);
                        let status = mm.ems_set_handle_name(handle, name);
                        cpu.set_ax((cpu.ax() & 0x00FF) | ((status as u16) << 8));
                    }
                    _ => {
                        cpu.set_ax((cpu.ax() & 0x00FF) | 0x8F00);
                    }
                }
            }
            0x54 => {
                let al = cpu.ax() as u8;
                match al {
                    0x00 => {
                        let dest = ((cpu.es() as u32) << 4) + cpu.di() as u32;
                        let dir = mm.ems_handle_directory();
                        for (i, &(handle, ref name)) in dir.iter().enumerate() {
                            let addr = dest + (i as u32) * 10;
                            memory.write_word(addr, handle);
                            memory.write_block(addr + 2, name);
                        }
                        cpu.set_ax(dir.len() as u16);
                    }
                    0x01 => {
                        let src = ((cpu.ds() as u32) << 4) + cpu.si() as u32;
                        let mut name = [0u8; 8];
                        memory.read_block(src, &mut name);
                        match mm.ems_search_handle_name(&name) {
                            Ok(handle) => {
                                cpu.set_dx(handle);
                                cpu.set_ax(cpu.ax() & 0x00FF);
                            }
                            Err(code) => {
                                cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                            }
                        }
                    }
                    0x02 => {
                        cpu.set_bx(MAX_EMS_HANDLES as u16);
                        cpu.set_ax(cpu.ax() & 0x00FF);
                    }
                    _ => {
                        cpu.set_ax((cpu.ax() & 0x00FF) | 0x8F00);
                    }
                }
            }
            0x57 => {
                let al = cpu.ax() as u8;
                let src = ((cpu.ds() as u32) << 4) + cpu.si() as u32;
                let params = EmsMoveParams {
                    region_length: memory.read_word(src) as u32
                        | ((memory.read_word(src + 2) as u32) << 16),
                    src_type: memory.read_byte(src + 4),
                    src_handle: memory.read_word(src + 5),
                    src_offset: memory.read_word(src + 7),
                    src_seg_page: memory.read_word(src + 9),
                    dst_type: memory.read_byte(src + 11),
                    dst_handle: memory.read_word(src + 12),
                    dst_offset: memory.read_word(src + 14),
                    dst_seg_page: memory.read_word(src + 16),
                };

                let status = match al {
                    0x00 => mm.ems_move_memory(&params, memory),
                    0x01 => mm.ems_exchange_memory(&params, memory),
                    _ => 0x8F,
                };
                cpu.set_ax((cpu.ax() & 0x00FF) | ((status as u16) << 8));
            }
            0x58 => {
                let al = cpu.ax() as u8;
                match al {
                    0x00 => {
                        let dest = ((cpu.es() as u32) << 4) + cpu.di() as u32;
                        let segments: [u16; 4] = [0xC000, 0xC400, 0xC800, 0xCC00];
                        for (i, &seg) in segments.iter().enumerate() {
                            let addr = dest + (i as u32) * 4;
                            memory.write_word(addr, seg);
                            memory.write_word(addr + 2, i as u16);
                        }
                        cpu.set_cx(4);
                        cpu.set_ax(cpu.ax() & 0x00FF);
                    }
                    0x01 => {
                        cpu.set_cx(4);
                        cpu.set_ax(cpu.ax() & 0x00FF);
                    }
                    _ => {
                        cpu.set_ax((cpu.ax() & 0x00FF) | 0x8F00);
                    }
                }
            }
            0x59 => {
                let al = cpu.ax() as u8;
                match al {
                    0x00 => {
                        if !mm.ems_os_functions_enabled() {
                            cpu.set_ax((cpu.ax() & 0x00FF) | 0xA400);
                            return;
                        }
                        let dest = ((cpu.es() as u32) << 4) + cpu.di() as u32;
                        memory.write_word(dest, 0x0400);
                        memory.write_word(dest + 2, 0x0000);
                        memory.write_word(dest + 4, mm.ems_page_map_size());
                        memory.write_word(dest + 6, 0x0000);
                        memory.write_word(dest + 8, 0x0000);
                        cpu.set_ax(cpu.ax() & 0x00FF);
                    }
                    0x01 => {
                        let (free, total) = mm.ems_unallocated_pages();
                        cpu.set_bx(free);
                        cpu.set_dx(total);
                        cpu.set_ax(cpu.ax() & 0x00FF);
                    }
                    _ => {
                        cpu.set_ax((cpu.ax() & 0x00FF) | 0x8F00);
                    }
                }
            }
            0x5A => {
                let al = cpu.ax() as u8;
                match al {
                    0x00 => {
                        let count = cpu.bx();
                        match mm.ems_allocate_pages_zero_allowed(count) {
                            Ok(handle) => {
                                cpu.set_dx(handle);
                                cpu.set_ax(cpu.ax() & 0x00FF);
                            }
                            Err(code) => {
                                cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                            }
                        }
                    }
                    0x01 => {
                        let count = cpu.bx();
                        match mm.ems_allocate_pages_zero_allowed(count) {
                            Ok(handle) => {
                                cpu.set_dx(handle);
                                cpu.set_ax(cpu.ax() & 0x00FF);
                            }
                            Err(code) => {
                                cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                            }
                        }
                    }
                    _ => {
                        cpu.set_ax((cpu.ax() & 0x00FF) | 0x8F00);
                    }
                }
            }
            0x5B => {
                let al = cpu.ax() as u8;
                if !mm.ems_os_functions_enabled() {
                    cpu.set_ax((cpu.ax() & 0x00FF) | 0xA400);
                    return;
                }
                match al {
                    0x00 => {
                        cpu.set_bx(cpu.bx() & 0xFF00);
                        if let Some((segment, offset)) = mm.ems_alt_map_context_save_area() {
                            cpu.set_es(segment);
                            cpu.set_di(offset);
                            let map = mm.ems_get_page_map();
                            let save_addr = ((segment as u32) << 4) + offset as u32;
                            write_page_map_to_memory(memory, save_addr, &map);
                        } else {
                            cpu.set_es(0);
                            cpu.set_di(0);
                        }
                        cpu.set_ax(cpu.ax() & 0x00FF);
                    }
                    0x01 => {
                        let bl = cpu.bx() as u8;
                        if bl == 0 {
                            let save_area = if cpu.es() == 0 && cpu.di() == 0 {
                                None
                            } else {
                                Some((cpu.es(), cpu.di()))
                            };
                            mm.ems_set_alt_map_context_save_area(save_area);
                            if let Some((segment, offset)) = save_area {
                                let save_addr = ((segment as u32) << 4) + offset as u32;
                                match read_page_map_from_memory(mm, memory, save_addr) {
                                    Ok(map) => mm.ems_set_page_map(map, memory),
                                    Err(code) => {
                                        cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                                        return;
                                    }
                                }
                            }
                            cpu.set_ax(cpu.ax() & 0x00FF);
                        } else {
                            cpu.set_ax((cpu.ax() & 0x00FF) | 0x9C00);
                        }
                    }
                    0x02 => {
                        cpu.set_dx(mm.ems_page_map_size());
                        cpu.set_ax(cpu.ax() & 0x00FF);
                    }
                    0x03 => {
                        cpu.set_bx(cpu.bx() & 0xFF00);
                        cpu.set_ax(cpu.ax() & 0x00FF);
                    }
                    0x04 => {
                        let bl = cpu.bx() as u8;
                        if bl == 0 {
                            cpu.set_ax(cpu.ax() & 0x00FF);
                        } else {
                            cpu.set_ax((cpu.ax() & 0x00FF) | 0x9C00);
                        }
                    }
                    0x05 => {
                        cpu.set_bx(cpu.bx() & 0xFF00);
                        cpu.set_ax(cpu.ax() & 0x00FF);
                    }
                    0x06 | 0x07 => {
                        let bl = cpu.bx() as u8;
                        if bl == 0 {
                            cpu.set_ax(cpu.ax() & 0x00FF);
                        } else {
                            cpu.set_ax((cpu.ax() & 0x00FF) | 0x9C00);
                        }
                    }
                    0x08 => {
                        let bl = cpu.bx() as u8;
                        if bl == 0 {
                            cpu.set_ax(cpu.ax() & 0x00FF);
                        } else {
                            cpu.set_ax((cpu.ax() & 0x00FF) | 0x9C00);
                        }
                    }
                    _ => {
                        cpu.set_ax((cpu.ax() & 0x00FF) | 0x8F00);
                    }
                }
            }
            0x5C => {
                cpu.set_ax(cpu.ax() & 0x00FF);
            }
            0x5D => {
                let al = cpu.ax() as u8;
                match al {
                    0x00 => {
                        let provided_key = ((cpu.bx() as u32) << 16) | cpu.cx() as u32;
                        match mm.ems_enable_os_functions(provided_key) {
                            Ok(Some(key)) => {
                                cpu.set_bx((key >> 16) as u16);
                                cpu.set_cx(key as u16);
                                cpu.set_ax(cpu.ax() & 0x00FF);
                            }
                            Ok(None) => {
                                cpu.set_ax(cpu.ax() & 0x00FF);
                            }
                            Err(code) => {
                                cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                            }
                        }
                    }
                    0x01 => {
                        let provided_key = ((cpu.bx() as u32) << 16) | cpu.cx() as u32;
                        match mm.ems_disable_os_functions(provided_key) {
                            Ok(Some(key)) => {
                                cpu.set_bx((key >> 16) as u16);
                                cpu.set_cx(key as u16);
                                cpu.set_ax(cpu.ax() & 0x00FF);
                            }
                            Ok(None) => {
                                cpu.set_ax(cpu.ax() & 0x00FF);
                            }
                            Err(code) => {
                                cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                            }
                        }
                    }
                    0x02 => {
                        let provided_key = ((cpu.bx() as u32) << 16) | cpu.cx() as u32;
                        match mm.ems_return_os_access_key(provided_key) {
                            Ok(()) => {
                                cpu.set_ax(cpu.ax() & 0x00FF);
                            }
                            Err(code) => {
                                cpu.set_ax((cpu.ax() & 0x00FF) | ((code as u16) << 8));
                            }
                        }
                    }
                    _ => {
                        cpu.set_ax((cpu.ax() & 0x00FF) | 0x8F00);
                    }
                }
            }
            _ => {
                cpu.set_ax((cpu.ax() & 0x00FF) | 0x8400);
            }
        }
    }
}

impl NeetanDos {
    pub(crate) fn int67h_pgmapret(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let mm = match self.state.memory_manager {
            Some(ref mut mm) if mm.is_ems_enabled() => mm,
            _ => {
                cpu.set_ax((cpu.ax() & 0x00FF) | 0x8400);
                return;
            }
        };

        let status = match mm.ems_pop_page_map_call_context() {
            Some(context) => apply_log_phys_map(
                mm,
                memory,
                context.handle,
                context.segphys_mode,
                context.old_len,
                context.old_map_addr,
            ),
            None => 0x80,
        };
        cpu.set_ax((cpu.ax() & 0x00FF) | ((status as u16) << 8));
    }
}

const MAX_EMS_HANDLES: usize = 255;

/// Applies a log_phys_map array (used by Functions 22/23) to the EMS page
/// mapping. `al_mode`: 0 = physical page numbers, 1 = segment addresses.
/// Returns 0 on success or an EMS error code.
fn apply_log_phys_map(
    mm: &mut crate::memory::memory_manager::MemoryManager,
    memory: &mut dyn crate::MemoryAccess,
    handle: u16,
    al_mode: u8,
    map_len: u8,
    map_addr: u32,
) -> u8 {
    for i in 0..map_len {
        let entry = map_addr + (i as u32) * 4;
        let logical_page = memory.read_word(entry);
        let phys_or_seg = memory.read_word(entry + 2);
        let physical = if al_mode == 0 {
            phys_or_seg as u8
        } else {
            match phys_or_seg {
                0xC000 => 0,
                0xC400 => 1,
                0xC800 => 2,
                0xCC00 => 3,
                _ => return 0x8B,
            }
        };
        let status = if logical_page == 0xFFFF {
            mm.ems_unmap_page(physical, memory)
        } else {
            mm.ems_map_page(handle, logical_page, physical, memory)
        };
        if status != 0 {
            return status;
        }
    }
    0
}

fn read_page_map_from_memory(
    mm: &crate::memory::memory_manager::MemoryManager,
    memory: &dyn crate::MemoryAccess,
    src_addr: u32,
) -> Result<[Option<crate::memory::memory_manager::EmsMapping>; 4], u8> {
    let mut map = [None; 4];
    for (i, slot) in map.iter_mut().enumerate() {
        let addr = src_addr + (i as u32) * 4;
        let handle = memory.read_word(addr);
        let page = memory.read_word(addr + 2);
        if handle == 0xFFFF && page == 0xFFFF {
            continue;
        }
        if handle == 0xFFFF || page == 0xFFFF {
            return Err(0xA3);
        }
        let Some(offset) = mm.ems_page_allocation_offset(handle, page) else {
            return Err(0xA3);
        };
        *slot = Some(crate::memory::memory_manager::EmsMapping {
            handle,
            logical_page: page,
            allocation_offset: offset,
        });
    }
    Ok(map)
}

fn write_page_map_to_memory(
    memory: &mut dyn crate::MemoryAccess,
    dest_addr: u32,
    map: &[Option<crate::memory::memory_manager::EmsMapping>; 4],
) {
    for (i, slot) in map.iter().enumerate() {
        let addr = dest_addr + (i as u32) * 4;
        match slot {
            Some(mapping) => {
                memory.write_word(addr, mapping.handle);
                memory.write_word(addr + 2, mapping.logical_page);
            }
            None => {
                memory.write_word(addr, 0xFFFF);
                memory.write_word(addr + 2, 0xFFFF);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        CpuAccess, MemoryAccess, NeetanDos,
        memory::memory_manager::MemoryManager,
        test_support::{MockCpu, MockMemory},
    };

    fn prepare_dos_with_ems() -> (NeetanDos, MockMemory) {
        let mut dos = NeetanDos::new();
        let mut memory = MockMemory::with_extended_memory(0x200000, 0x200000);
        dos.state.memory_manager = Some(MemoryManager::new(
            memory.extended_memory_size(),
            true,
            false,
            false,
            &mut memory,
        ));
        (dos, memory)
    }

    #[test]
    fn int67h_function_51_failure_returns_current_pages_in_bx() {
        let (mut dos, mut memory) = prepare_dos_with_ems();
        let mm = dos
            .state
            .memory_manager
            .as_mut()
            .expect("EMS memory manager should exist");
        let handle = mm.ems_allocate_pages(2).unwrap();
        mm.ems_allocate_pages(70).unwrap();

        let mut cpu = MockCpu::default();
        cpu.set_ax(0x5100);
        cpu.set_bx(60);
        cpu.set_dx(handle);
        dos.int67h(&mut cpu, &mut memory);

        assert_eq!(cpu.ax(), 0x8800);
        assert_eq!(cpu.bx(), 2);
    }

    #[test]
    fn int67h_function_51_invalid_handle_clears_bx() {
        let (mut dos, mut memory) = prepare_dos_with_ems();

        let mut cpu = MockCpu::default();
        cpu.set_ax(0x5100);
        cpu.set_bx(0x1234);
        cpu.set_dx(0xEEEE);
        dos.int67h(&mut cpu, &mut memory);

        assert_eq!(cpu.ax(), 0x8300);
        assert_eq!(cpu.bx(), 0);
    }

    #[test]
    fn int67h_function_5402_returns_total_handles_in_bx() {
        let (mut dos, mut memory) = prepare_dos_with_ems();
        let mut cpu = MockCpu::default();

        cpu.set_ax(0x5402);
        cpu.set_bx(0);
        dos.int67h(&mut cpu, &mut memory);

        assert_eq!(cpu.ax(), 0x0002);
        assert_eq!(cpu.bx(), 255);
    }

    #[test]
    fn int67h_function_5000_rejects_more_than_four_mappings() {
        let (mut dos, mut memory) = prepare_dos_with_ems();
        let handle = dos
            .state
            .memory_manager
            .as_mut()
            .expect("EMS memory manager should exist")
            .ems_allocate_pages(1)
            .unwrap();
        let table_addr = 0x2100;
        for index in 0..5u32 {
            memory.write_word(table_addr + index * 4, 0);
            memory.write_word(table_addr + index * 4 + 2, 0);
        }

        let mut cpu = MockCpu::default();
        cpu.set_ax(0x5000);
        cpu.set_cx(5);
        cpu.set_dx(handle);
        cpu.set_ds((table_addr >> 4) as u16);
        cpu.set_si((table_addr & 0x0F) as u16);
        dos.int67h(&mut cpu, &mut memory);

        assert_eq!(cpu.ax(), 0x8B00);
    }

    #[test]
    fn int67h_function_5500_rejects_invalid_mode() {
        let (mut dos, mut memory) = prepare_dos_with_ems();
        let struct_addr = 0x2200;
        memory.write_word(struct_addr, 0x1234);
        memory.write_word(struct_addr + 2, 0x5678);
        memory.write_byte(struct_addr + 4, 0);
        memory.write_word(struct_addr + 5, 0);
        memory.write_word(struct_addr + 7, 0);

        let mut cpu = MockCpu::default();
        cpu.set_ax(0x5502);
        cpu.set_ds((struct_addr >> 4) as u16);
        cpu.set_si((struct_addr & 0x0F) as u16);
        dos.int67h(&mut cpu, &mut memory);

        assert_eq!(cpu.ax(), 0x8F02);
    }

    #[test]
    fn int67h_function_5500_rejects_more_than_four_mappings() {
        let (mut dos, mut memory) = prepare_dos_with_ems();
        let handle = dos
            .state
            .memory_manager
            .as_mut()
            .expect("EMS memory manager should exist")
            .ems_allocate_pages(1)
            .unwrap();
        let struct_addr = 0x2200;
        let iret_base = ((0x2000u32) << 4) + 0x0100;
        memory.write_word(iret_base, 0xAAAA);
        memory.write_word(iret_base + 2, 0xBBBB);
        memory.write_word(struct_addr, 0x1234);
        memory.write_word(struct_addr + 2, 0x5678);
        memory.write_byte(struct_addr + 4, 5);
        memory.write_word(struct_addr + 5, 0x0300);
        memory.write_word(struct_addr + 7, 0x0000);

        let mut cpu = MockCpu::default();
        cpu.set_ax(0x5500);
        cpu.set_dx(handle);
        cpu.set_ss(0x2000);
        cpu.set_sp(0x0100);
        cpu.set_ds((struct_addr >> 4) as u16);
        cpu.set_si((struct_addr & 0x0F) as u16);
        dos.int67h(&mut cpu, &mut memory);

        assert_eq!(cpu.ax(), 0x8B00);
        assert_eq!(memory.read_word(iret_base), 0xAAAA);
        assert_eq!(memory.read_word(iret_base + 2), 0xBBBB);
    }

    #[test]
    fn int67h_function_5600_rejects_more_than_four_new_or_old_mappings() {
        let (mut dos, mut memory) = prepare_dos_with_ems();
        let handle = dos
            .state
            .memory_manager
            .as_mut()
            .expect("EMS memory manager should exist")
            .ems_allocate_pages(1)
            .unwrap();
        let struct_addr = 0x2200;
        memory.write_word(struct_addr, 0x1234);
        memory.write_word(struct_addr + 2, 0x5678);
        memory.write_byte(struct_addr + 4, 1);
        memory.write_word(struct_addr + 5, 0x0300);
        memory.write_word(struct_addr + 7, 0x0000);
        memory.write_byte(struct_addr + 9, 5);
        memory.write_word(struct_addr + 10, 0x0400);
        memory.write_word(struct_addr + 12, 0x0000);

        let mut cpu = MockCpu::default();
        cpu.set_ax(0x5600);
        cpu.set_dx(handle);
        cpu.set_ss(0x2000);
        cpu.set_sp(0x0100);
        cpu.set_ds((struct_addr >> 4) as u16);
        cpu.set_si((struct_addr & 0x0F) as u16);
        dos.int67h(&mut cpu, &mut memory);

        assert_eq!(cpu.ax(), 0x8B00);
        assert_eq!(cpu.sp(), 0x0100);
    }

    #[test]
    fn int67h_function_5201_distinguishes_unsupported_attributes() {
        let (mut dos, mut memory) = prepare_dos_with_ems();
        let handle = dos
            .state
            .memory_manager
            .as_mut()
            .expect("EMS memory manager should exist")
            .ems_allocate_pages(1)
            .unwrap();
        let mut cpu = MockCpu::default();

        cpu.set_ax(0x5201);
        cpu.set_bx(0x0001);
        cpu.set_dx(handle);
        dos.int67h(&mut cpu, &mut memory);
        assert_eq!(cpu.ax(), 0x9101);

        cpu.set_ax(0x5201);
        cpu.set_bx(0x0002);
        cpu.set_dx(handle);
        dos.int67h(&mut cpu, &mut memory);
        assert_eq!(cpu.ax(), 0x9001);
    }

    #[test]
    fn int67h_function_5401_zero_name_returns_unnamed_handle_error() {
        let (mut dos, mut memory) = prepare_dos_with_ems();
        let zero_name_addr = 0x2400;
        memory.write_block(zero_name_addr, &[0; 8]);

        let mut cpu = MockCpu::default();
        cpu.set_ax(0x5401);
        cpu.set_ds((zero_name_addr >> 4) as u16);
        cpu.set_si((zero_name_addr & 0x0F) as u16);
        dos.int67h(&mut cpu, &mut memory);

        assert_eq!(cpu.ax(), 0xA101);
    }

    #[test]
    fn int67h_function_5b06_and_5b07_accept_only_default_register_set() {
        let (mut dos, mut memory) = prepare_dos_with_ems();
        let mut cpu = MockCpu::default();

        cpu.set_ax(0x5B06);
        cpu.set_bx(0x0000);
        dos.int67h(&mut cpu, &mut memory);
        assert_eq!(cpu.ax(), 0x0006);

        cpu.set_ax(0x5B06);
        cpu.set_bx(0x0001);
        dos.int67h(&mut cpu, &mut memory);
        assert_eq!(cpu.ax(), 0x9C06);

        cpu.set_ax(0x5B07);
        cpu.set_bx(0x0000);
        dos.int67h(&mut cpu, &mut memory);
        assert_eq!(cpu.ax(), 0x0007);

        cpu.set_ax(0x5B07);
        cpu.set_bx(0x0001);
        dos.int67h(&mut cpu, &mut memory);
        assert_eq!(cpu.ax(), 0x9C07);
    }
}
