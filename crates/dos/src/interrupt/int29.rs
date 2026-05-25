//! INT 29h: DOS Fast Console Output.
//!
//! Outputs the character in AL directly to the console, bypassing normal
//! DOS I/O buffering.

use crate::{CpuAccess, MemoryAccess, NeetanDos};

impl NeetanDos {
    /// INT 29h: Fast console output. Character is in AL.
    pub(crate) fn int29h(&mut self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let al = (cpu.ax() & 0xFF) as u8;
        self.console.process_byte(memory, al);
    }
}
