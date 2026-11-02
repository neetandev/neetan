//! INT 25h: Absolute Disk Read.

use crate::{CpuAccess, DiskIo, MemoryAccess, NeetanDos, SegmentRegister};

impl NeetanDos {
    /// INT 25h: Absolute disk read.
    /// AL = drive number (0=A), CX = sector count, DX = start sector, DS:BX = buffer.
    /// The sectors are logical sectors of the volume.
    pub(crate) fn int25h(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        let drive_index = (cpu.ax() & 0xFF) as u8;
        let sector_count = cpu.cx() as u32;
        let start_sector = cpu.dx() as u32;
        let buf_addr = cpu.linear_address(SegmentRegister::DS, cpu.bx());

        let span =
            match self
                .state
                .logical_sector_span(drive_index, start_sector, sector_count, disk)
            {
                Ok(span) => span,
                Err(error) => {
                    cpu.set_ax(error);
                    set_int25_carry(cpu, memory, true);
                    return;
                }
            };

        match disk.read_sectors(span.device_id, span.first_lba, span.sector_count) {
            Ok(data) => {
                memory.write_block(buf_addr, &data);
                set_int25_carry(cpu, memory, false);
            }
            Err(err) => {
                cpu.set_ax(err as u16);
                set_int25_carry(cpu, memory, true);
            }
        }
    }
}

fn set_int25_carry(cpu: &dyn CpuAccess, mem: &mut dyn MemoryAccess, carry: bool) {
    let flags_addr = cpu
        .linear_address(SegmentRegister::SS, cpu.sp())
        .wrapping_add(4);
    let mut flags = mem.read_word(flags_addr);
    if carry {
        flags |= 0x0001;
    } else {
        flags &= !0x0001;
    }
    mem.write_word(flags_addr, flags);
}
