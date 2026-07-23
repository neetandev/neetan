//! INT 0Eh diskette hardware interrupt (IRQ 6) bookkeeping.

use common::TraceSink;

use super::AtBus;

/// BIOS data area: diskette recalibrate and interrupt status.
const BDA_FLOPPY_RECALIBRATE: u32 = 0x43E;
/// BDA 40:3E bit 7: a diskette interrupt occurred.
const FLOPPY_INTERRUPT_FLAG: u8 = 0x80;

impl<T: TraceSink> AtBus<T> {
    /// INT 0Eh: records the diskette interrupt in the BDA completion flag.
    /// The ROM stub acknowledges the IRQ itself after the trap; the handler
    /// never touches CPU registers or the IRET frame.
    pub(super) fn hle_int0eh(&mut self) {
        let recalibrate = self.read_mem_byte(BDA_FLOPPY_RECALIBRATE);
        self.write_mem_byte(BDA_FLOPPY_RECALIBRATE, recalibrate | FLOPPY_INTERRUPT_FLAG);
    }
}
