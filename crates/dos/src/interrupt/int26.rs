//! INT 26h: Absolute Disk Write.

use crate::{CpuAccess, DiskIo, MemoryAccess, NeetanDos, tables};

impl NeetanDos {
    /// INT 26h: Absolute disk write.
    /// AL = drive number (0=A), CX = sector count, DX = start sector, DS:BX = buffer.
    pub(crate) fn int26h(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        let drive_index = (cpu.ax() & 0xFF) as u8;
        let sector_count = cpu.cx() as u32;
        let start_sector = cpu.dx() as u32;
        let buf_addr = ((cpu.ds() as u32) << 4) + cpu.bx() as u32;

        let da_ua = memory
            .read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_DAUA_TABLE + drive_index as u32);
        if da_ua == 0 {
            set_int26_carry(cpu, memory, true);
            cpu.set_ax(0x000F);
            return;
        }

        let sector_size = disk.sector_size(da_ua).unwrap_or(512) as usize;
        let total_bytes = sector_count as usize * sector_size;
        let mut data = vec![0u8; total_bytes];
        memory.read_block(buf_addr, &mut data);

        match disk.write_sectors(da_ua, start_sector, &data) {
            Ok(()) => set_int26_carry(cpu, memory, false),
            Err(err) => {
                cpu.set_ax(err as u16);
                set_int26_carry(cpu, memory, true);
            }
        }
    }
}

fn set_int26_carry(cpu: &dyn CpuAccess, mem: &mut dyn MemoryAccess, carry: bool) {
    let flags_addr = ((cpu.ss() as u32) << 4) + cpu.sp() as u32 + 4;
    let mut flags = mem.read_word(flags_addr);
    if carry {
        flags |= 0x0001;
    } else {
        flags &= !0x0001;
    }
    mem.write_word(flags_addr, flags);
}
