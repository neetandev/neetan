//! INT 00h: divide error of the kernel entry.

use crate::{CpuAccess, MemoryAccess, NeetanDos};

/// Message of a divide error.
const DIVIDE_OVERFLOW_MESSAGE: &[u8] = b"\r\nDivide overflow\r\n";

/// Termination type of a program that ends like a Ctrl-C abort.
const TERMINATION_CTRL_C: u8 = 1;

impl NeetanDos {
    /// INT 00h: prints "Divide overflow" and ends the current program like a
    /// Ctrl-C abort.
    pub(crate) fn divide_error(&mut self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        for &byte in DIVIDE_OVERFLOW_MESSAGE {
            self.console_output().write_byte(memory, byte);
        }
        self.terminate_process(cpu, memory, 0, TERMINATION_CTRL_C);
    }
}
