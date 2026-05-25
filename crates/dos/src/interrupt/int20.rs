//! INT 20h: Program Terminate.

use crate::{CpuAccess, MemoryAccess, NeetanDos};

impl NeetanDos {
    /// INT 20h: Terminate current program with return code 0.
    pub(crate) fn int20h(&mut self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        self.terminate_process(cpu, memory, 0, 0);
    }
}
