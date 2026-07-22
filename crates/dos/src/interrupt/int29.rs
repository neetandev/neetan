//! INT 29h: DOS Fast Console Output.
//!
//! Outputs the character in AL directly to the console, bypassing normal
//! DOS I/O buffering.

use crate::{
    CpuAccess, MemoryAccess, NeetanDos,
    trace::{DosStdoutEvent, route, source},
};

impl NeetanDos {
    /// INT 29h: Fast console output. Character is in AL.
    pub(crate) fn int29h(&mut self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let al = (cpu.ax() & 0xFF) as u8;
        if self.stdout_trace_enabled() {
            self.record_stdout(
                memory,
                DosStdoutEvent {
                    source: source::INT29,
                    handle: None,
                    buffer_address: None,
                    requested_count: 1,
                    bytes: vec![al],
                    route: route::CONSOLE,
                    suppression_reason: None,
                    int29_segment: 0,
                    int29_offset: 0,
                },
            );
        }
        self.console.process_byte(memory, al);
    }
}
