//! Disk sub-CPU (PC80S31K) I/O read.

use common::TraceSink;

use super::Pc8801Bus;

/// Open-bus value for unmapped sub-CPU I/O reads.
const SUB_OPEN_BUS: u8 = 0xFF;

impl<T: TraceSink> Pc8801Bus<T> {
    /// Reads a disk sub-CPU port and reports whether it was decoded.
    pub fn sub_io_read(&mut self, port: u16) -> (u8, bool) {
        let value = match port & 0xFF {
            // Interrupt acknowledge: returns 0x00 (no vector latch).
            0xF0 => 0x00,
            // Reading port 0xF8 pulses the FDC terminal count.
            0xF8 => {
                self.assert_fdc_terminal_count();
                0x00
            }
            // uPD765A: 0xFA = main status register, 0xFB = data register.
            0xFA => self.fdc.read_status(),
            0xFB => self.read_fdc_data(),
            // PPI mailbox (disk side): 0xFC=A, 0xFD=B, 0xFE=C, 0xFF=control.
            0xFC..=0xFF => self.ppi_sub.read((port & 0x03) as u8),
            _ => return (SUB_OPEN_BUS, false),
        };
        (value, true)
    }
}
