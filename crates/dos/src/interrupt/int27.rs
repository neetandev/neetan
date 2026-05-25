//! INT 27h: Terminate and Stay Resident (legacy).

use crate::{CpuAccess, MemoryAccess, NeetanDos};

impl NeetanDos {
    /// INT 27h: TSR (old-style). DX = number of bytes to keep resident.
    pub(crate) fn int27h(&mut self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let bytes_to_keep = cpu.dx() as u32;
        let keep_paragraphs = bytes_to_keep.div_ceil(16) as u16;
        self.terminate_process_tsr(cpu, memory, 0, keep_paragraphs);
    }
}
