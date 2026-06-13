//! Disk sub-CPU (PC80S31K) I/O writes.

use common::Tracing;

use super::Pc8801Bus;

/// Forced-ready control bit on the uPD765A control register (the PC-88 disk unit
/// always asserts drive ready when the motor is driven).
const FDC_FORCED_READY: u8 = 0x40;

impl<T: Tracing> Pc8801Bus<T> {
    /// Writes a disk sub-CPU I/O port (`port & 0xFF`). Public for tests and tooling.
    pub fn sub_io_write(&mut self, port: u16, value: u8) {
        match port & 0xFF {
            // Interrupt acknowledge latch: no-op (no vector latch).
            0xF0 => {}
            // Drive mode select (per-drive 2D/2DD/2HD).
            0xF4 => self.drive_mode = value,
            // Write precompensation: no-op.
            0xF7 => {}
            // Motor control: the disk unit always forces drive ready.
            0xF8 => {
                self.motor_on = value;
                self.fdc.state.control |= FDC_FORCED_READY;
            }
            // uPD765A data register: command/parameter bytes or PIO data bytes.
            0xFB => self.write_fdc_data(value),
            // PPI mailbox (disk side): A->peer B, B->peer A, C/control -> resync.
            0xFC => {
                self.ppi_sub.write(0, value);
                self.ppi_main.set_port_b(value);
            }
            0xFD => {
                self.ppi_sub.write(1, value);
                self.ppi_main.set_port_a(value);
            }
            0xFE | 0xFF => {
                let changed = self.ppi_sub.write((port & 0x03) as u8, value);
                self.on_ppi_sub_change(changed);
            }
            _ => {}
        }
    }
}
