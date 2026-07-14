//! Main-CPU direct uPD765A FDC access (I/O ports 0x1B0-0x1BB).
//!
//! Besides the PC-8801-compatible path through the PC80S31K sub-CPU, the PC-88VA2
//! lets the V30 drive the same uPD765A directly.

use super::Pc88VaBus;

/// uPD765A control-register bit 6: forced ready (FRY).
const FDC_FORCED_READY: u8 = 0x40;
/// uPD765A control-register bit 7: reset (RST), reset on rising edge.
const FDC_RESET: u8 = 0x80;

impl<T: common::TraceSink> Pc88VaBus<T> {
    /// FDC operating-mode register (0x1B0): bit 0 selects DMA (1) or PIO (0).
    /// In DMA mode the uPD765A interrupt is routed to the main 8259 (IRQ 11);
    /// in PIO mode the floppy sub-CPU services it instead.
    pub(crate) fn write_fdc_main_mode(&mut self, value: u8) {
        self.fdc_dma_mode = value & 0x01 != 0;
        self.update_main_fdc_irq();
    }

    /// Reconciles the main-CPU FDC interrupt line (slave IR3 / IRQ 11) with the
    /// uPD765A interrupt output. Only the DMA-mode main-CPU path drives this
    /// line; the sub-CPU PIO path keeps the interrupt on its own core.
    pub(crate) fn update_main_fdc_irq(&mut self) {
        if self.fdc_dma_mode && self.fdc.state.interrupt_pending {
            self.pic.set_irq(11);
        } else {
            self.pic.clear_irq(11);
        }
    }

    /// FDC control port 0 (0x1B2): per-drive density and clock selection.
    /// RV0/RV1 (bits 0-1) select 2HD mode; TD0/TD1 (bits 2-3) select 96 TPI
    /// (2DD) over 48 TPI (2D). This is translated into the same per-drive
    /// density latch the sub-CPU path drives through port 0xF4.
    pub(crate) fn write_fdc_main_control0(&mut self, value: u8) {
        let mut drive_mode = 0u8;
        if value & 0x01 != 0 {
            drive_mode |= 0x01;
        } else if value & 0x04 != 0 {
            drive_mode |= 0x04;
        }
        if value & 0x02 != 0 {
            drive_mode |= 0x02;
        } else if value & 0x08 != 0 {
            drive_mode |= 0x08;
        }
        self.drive_mode = drive_mode;
    }

    /// FDC control port 1 (0x1B4): drive motor control (M0/M1, bits 0-1).
    pub(crate) fn write_fdc_main_control1(&mut self, value: u8) {
        self.motor_on = value & 0x03;
    }

    /// FDC control port 2 (0x1B6): reset, force-ready, and the FDC timer.
    /// FDCRST (bit 7) resets the controller on a rising edge; force ready is
    /// asserted when both FRYCEN (bit 5) and FDCFRY (bit 6) are set.
    pub(crate) fn write_fdc_main_control2(&mut self, value: u8) {
        let mut control = value & FDC_RESET;
        if value & 0x60 == 0x60 {
            control |= FDC_FORCED_READY;
        }
        self.fdc.write_control(control);
        self.update_main_fdc_irq();
    }
}

#[cfg(test)]
mod tests {
    use crate::bus::test_support::test_bus;

    /// FDC interrupt mask on the slave PIC (IR3 = IRQ 11) and its vector.
    const FDC_IRQ_VECTOR: u8 = 0x13;

    /// In DMA mode a pending uPD765A interrupt is routed to the main 8259 as
    /// IRQ 11; clearing DMA mode drops the line again.
    #[test]
    fn dma_mode_routes_fdc_interrupt_to_main_irq_11() {
        let mut bus = test_bus();
        bus.fdc.state.interrupt_pending = true;

        bus.io_write(0x1B0, 0x01); // DMA mode on -> reconciles the IRQ line
        assert!(
            bus.pic.has_pending_irq(),
            "FDC interrupt reaches the main PIC"
        );
        assert_eq!(bus.pic.acknowledge(), FDC_IRQ_VECTOR);

        bus.io_write(0x1B0, 0x00); // back to PIO mode
        assert!(
            !bus.pic.has_pending_irq(),
            "main FDC IRQ released in PIO mode"
        );
    }

    /// In PIO (sub-CPU) mode the FDC interrupt never drives the main IRQ 11.
    #[test]
    fn pio_mode_does_not_raise_main_irq() {
        let mut bus = test_bus();
        bus.fdc.state.interrupt_pending = true;
        bus.update_main_fdc_irq();
        assert!(!bus.pic.has_pending_irq());
    }

    /// Resetting the FDC through control port 2 clears the pending interrupt and
    /// drops the main IRQ line.
    #[test]
    fn fdc_reset_clears_main_irq() {
        let mut bus = test_bus();
        bus.io_write(0x1B0, 0x01);
        bus.fdc.state.interrupt_pending = true;
        bus.update_main_fdc_irq();
        assert!(bus.pic.has_pending_irq());

        bus.io_write(0x1B6, 0x80); // FDCRST rising edge resets the controller
        assert!(!bus.fdc.state.interrupt_pending);
        assert!(!bus.pic.has_pending_irq());
    }
}
