//! INT 28h: DOS Idle Interrupt.
//!
//! Called by DOS during character input waits (AH=01h-0Ch). When invoked,
//! SS:SP is on the DOS I/O stack, InDOS=1, and handlers may safely call
//! INT 21h with AH >= 0Ch. TSRs hook this vector to perform background work.
//!
//! The default handler is an IRET (no operation).

use crate::{CpuAccess, MemoryAccess, NeetanDos};

impl NeetanDos {
    /// INT 28h: DOS idle notification. No-op handler (default is IRET per spec).
    pub(crate) fn int28h(&self, _cpu: &mut dyn CpuAccess, _memory: &mut dyn MemoryAccess) {}
}
