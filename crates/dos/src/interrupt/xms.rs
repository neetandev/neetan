//! XMS driver entry point handler (via INT FEh trampoline).

use crate::{CpuAccess, MemoryAccess, NeetanDos, SegmentRegister};

impl NeetanDos {
    pub(crate) fn xms_entry(&mut self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let mm = match self.state.memory_manager {
            Some(ref mut mm) if mm.is_xms_enabled() => mm,
            _ => {
                cpu.set_ax(0);
                cpu.set_bx((cpu.bx() & 0xFF00) | 0x0080);
                return;
            }
        };

        let ah = (cpu.ax() >> 8) as u8;
        match ah {
            0x00 => {
                let (version, revision, hma_exists) = mm.xms_version();
                cpu.set_ax(version);
                cpu.set_bx(revision);
                cpu.set_dx(hma_exists);
            }
            0x01 => {
                let size = cpu.dx();
                match mm.xms_request_hma(size) {
                    Ok(()) => cpu.set_ax(1),
                    Err(code) => {
                        cpu.set_ax(0);
                        cpu.set_dx(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                    }
                }
            }
            0x02 => match mm.xms_release_hma() {
                Ok(()) => cpu.set_ax(1),
                Err(code) => {
                    cpu.set_ax(0);
                    cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                }
            },
            0x03 => {
                // Global Enable A20. On PC-98 A20 is always physically
                // enabled; track the XMS-visible state only.
                mm.xms_global_enable_a20();
                cpu.set_ax(1);
            }
            0x04 => {
                // Global Disable A20. Fails (BL=0x94) if a local enable is
                // still outstanding.
                match mm.xms_global_disable_a20() {
                    Ok(()) => cpu.set_ax(1),
                    Err(code) => {
                        cpu.set_ax(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                    }
                }
            }
            0x05 => {
                // Local Enable A20 (increment nesting count).
                mm.xms_local_enable_a20();
                cpu.set_ax(1);
            }
            0x06 => {
                // Local Disable A20 (decrement nesting count). Fails
                // (BL=0x94) if the count is already zero.
                match mm.xms_local_disable_a20() {
                    Ok(()) => cpu.set_ax(1),
                    Err(code) => {
                        cpu.set_ax(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                    }
                }
            }
            0x07 => {
                // Query A20 reports the XMS-visible state, not the PC-98
                // physical line state.
                if mm.xms_query_a20() {
                    cpu.set_ax(1);
                    cpu.set_bx(cpu.bx() & 0xFF00);
                } else {
                    cpu.set_ax(0);
                    cpu.set_bx(cpu.bx() & 0xFF00);
                }
            }
            0x08 => {
                let (largest, total) = mm.xms_query_free();
                cpu.set_ax(largest);
                cpu.set_dx(total);
                if total == 0 {
                    cpu.set_bx((cpu.bx() & 0xFF00) | 0x00A0);
                } else {
                    cpu.set_bx(cpu.bx() & 0xFF00);
                }
            }
            0x09 => {
                let size_kb = cpu.dx();
                match mm.xms_allocate(size_kb) {
                    Ok(handle) => {
                        cpu.set_ax(1);
                        cpu.set_dx(handle);
                    }
                    Err(code) => {
                        cpu.set_ax(0);
                        cpu.set_dx(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                    }
                }
            }
            0x0A => {
                let handle = cpu.dx();
                match mm.xms_free(handle) {
                    Ok(()) => cpu.set_ax(1),
                    Err(code) => {
                        cpu.set_ax(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                    }
                }
            }
            0x0B => {
                let params = cpu.linear_address(SegmentRegister::DS, cpu.si());
                match mm.xms_move(memory, params) {
                    Ok(()) => cpu.set_ax(1),
                    Err(code) => {
                        cpu.set_ax(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                    }
                }
            }
            0x0C => {
                let handle = cpu.dx();
                match mm.xms_lock(handle) {
                    Ok(addr) => {
                        cpu.set_ax(1);
                        cpu.set_dx((addr >> 16) as u16);
                        cpu.set_bx(addr as u16);
                    }
                    Err(code) => {
                        cpu.set_ax(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                    }
                }
            }
            0x0D => {
                let handle = cpu.dx();
                match mm.xms_unlock(handle) {
                    Ok(()) => cpu.set_ax(1),
                    Err(code) => {
                        cpu.set_ax(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                    }
                }
            }
            0x0E => {
                let handle = cpu.dx();
                match mm.xms_handle_info(handle) {
                    Ok((lock_count, free_handles, size_kb)) => {
                        cpu.set_ax(1);
                        cpu.set_bx(((lock_count as u16) << 8) | (free_handles & 0xFF));
                        cpu.set_dx(size_kb);
                    }
                    Err(code) => {
                        cpu.set_ax(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                    }
                }
            }
            0x0F => {
                let new_size = cpu.bx();
                let handle = cpu.dx();
                match mm.xms_reallocate(handle, new_size, memory) {
                    Ok(()) => cpu.set_ax(1),
                    Err(code) => {
                        cpu.set_ax(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                    }
                }
            }
            0x10 => {
                let paragraphs = cpu.dx();
                match mm.umb_allocate(paragraphs, memory) {
                    Ok((segment, size)) => {
                        cpu.set_ax(1);
                        cpu.set_bx(segment);
                        cpu.set_dx(size);
                    }
                    Err((code, largest)) => {
                        cpu.set_ax(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                        cpu.set_dx(largest);
                    }
                }
            }
            0x11 => {
                let segment = cpu.dx();
                match mm.umb_free(segment, memory) {
                    Ok(()) => cpu.set_ax(1),
                    Err(code) => {
                        cpu.set_ax(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                    }
                }
            }
            0x12 => {
                let new_size = cpu.bx();
                let segment = cpu.dx();
                match mm.umb_reallocate(segment, new_size, memory) {
                    Ok(()) => cpu.set_ax(1),
                    Err((code, largest)) => {
                        cpu.set_ax(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                        cpu.set_dx(largest);
                    }
                }
            }
            0x88 => {
                if !mm.is_xms_32_enabled() {
                    cpu.set_ax(0);
                    cpu.set_bx((cpu.bx() & 0xFF00) | 0x0080);
                    return;
                }
                let (largest, total) = mm.xms_query_free_32();
                cpu.set_eax(largest);
                cpu.set_edx(total);
                cpu.set_ecx(
                    crate::memory::memory_manager::EXTENDED_RAM_BASE
                        + mm.extended_memory_size_bytes()
                        - 1,
                );
                if total == 0 {
                    cpu.set_bx((cpu.bx() & 0xFF00) | 0x00A0);
                } else {
                    cpu.set_bx(cpu.bx() & 0xFF00);
                }
            }
            0x89 => {
                if !mm.is_xms_32_enabled() {
                    cpu.set_ax(0);
                    cpu.set_bx((cpu.bx() & 0xFF00) | 0x0080);
                    return;
                }
                let size_kb = cpu.edx();
                match mm.xms_allocate_32(size_kb) {
                    Ok(handle) => {
                        cpu.set_ax(1);
                        cpu.set_dx(handle);
                    }
                    Err(code) => {
                        cpu.set_ax(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                    }
                }
            }
            0x8E => {
                if !mm.is_xms_32_enabled() {
                    cpu.set_ax(0);
                    cpu.set_bx((cpu.bx() & 0xFF00) | 0x0080);
                    return;
                }
                let handle = cpu.dx();
                match mm.xms_handle_info_32(handle) {
                    Ok((lock_count, free_handles, size_kb)) => {
                        cpu.set_ax(1);
                        cpu.set_bx((lock_count as u16) << 8);
                        cpu.set_cx(free_handles);
                        cpu.set_edx(size_kb);
                    }
                    Err(code) => {
                        cpu.set_ax(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                    }
                }
            }
            0x8F => {
                if !mm.is_xms_32_enabled() {
                    cpu.set_ax(0);
                    cpu.set_bx((cpu.bx() & 0xFF00) | 0x0080);
                    return;
                }
                let new_size = cpu.ebx();
                let handle = cpu.dx();
                match mm.xms_reallocate_32(handle, new_size, memory) {
                    Ok(()) => cpu.set_ax(1),
                    Err(code) => {
                        cpu.set_ax(0);
                        cpu.set_bx((cpu.bx() & 0xFF00) | code as u16);
                    }
                }
            }
            _ => {
                cpu.set_ax(0);
                cpu.set_bx((cpu.bx() & 0xFF00) | 0x0080);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        CpuAccess, MemoryAccess, NeetanDos,
        memory::{self, memory_manager::MemoryManager},
        tables::UMB_FIRST_MCB_SEGMENT,
        test_support::{MockCpu, MockMemory},
    };

    fn prepare_dos_with_xms() -> (NeetanDos, MockMemory) {
        let mut dos = NeetanDos::new();
        let mut memory = MockMemory::with_extended_memory(0x200000, 0x200000);
        dos.state.memory_manager = Some(MemoryManager::new(
            memory.extended_memory_size(),
            false,
            true,
            true,
            &mut memory,
        ));
        (dos, memory)
    }

    #[test]
    fn xms_query_a20_clears_bl_on_success() {
        let (mut dos, mut memory) = prepare_dos_with_xms();
        let mut cpu = MockCpu::default();

        cpu.set_ax(0x0700);
        cpu.set_bx(0x12FF);
        dos.xms_entry(&mut cpu, &mut memory);

        // HIMEM globally enables A20 at load, so the query reports enabled.
        assert_eq!(cpu.ax(), 1);
        assert_eq!(cpu.bx(), 0x1200);
    }

    #[test]
    fn xms_allocate_failure_clears_dx_handle() {
        let (mut dos, mut memory) = prepare_dos_with_xms();
        let mut cpu = MockCpu::default();

        cpu.set_ax(0x0900);
        cpu.set_dx(0x1234);
        cpu.set_bx(0);
        cpu.set_dx(2000);
        dos.xms_entry(&mut cpu, &mut memory);

        assert_eq!(cpu.ax(), 0);
        assert_eq!(cpu.bx() & 0x00FF, 0x00A0);
        assert_eq!(cpu.dx(), 0);
    }

    #[test]
    fn xms_umb_reallocate_failure_returns_largest_free_umb() {
        let (mut dos, mut memory) = prepare_dos_with_xms();
        let mm = dos
            .state
            .memory_manager
            .as_ref()
            .expect("XMS memory manager should exist");
        let (segment, _) = mm.umb_allocate(4, &mut memory).unwrap();
        let (_second_segment, _) = mm.umb_allocate(4, &mut memory).unwrap();
        let expected_largest = memory::read_mcb_size_pub(&memory, UMB_FIRST_MCB_SEGMENT + 10);

        let mut cpu = MockCpu::default();
        cpu.set_ax(0x1200);
        cpu.set_bx(0xFFFF);
        cpu.set_dx(segment);
        dos.xms_entry(&mut cpu, &mut memory);

        assert_eq!(cpu.ax(), 0);
        assert_eq!(cpu.bx() & 0x00FF, 0x00B0);
        assert_eq!(cpu.dx(), expected_largest);
        assert_eq!(memory::read_mcb_size_pub(&memory, segment - 1), 4);
    }
}
