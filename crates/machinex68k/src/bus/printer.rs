//! Printer port latches with no printer attached.
//!
//! The data and strobe latches are readable and writable, but with no
//! printer connected the IOC READY input stays busy and the printer
//! interrupt never fires.

use common::Tracing;

use super::X68kBus;

/// Offset of the printer data latch within the printer window.
const PRINTER_DATA_OFFSET: u32 = 1;
/// Offset of the printer strobe latch within the printer window.
const PRINTER_STROBE_OFFSET: u32 = 3;
/// Writable bit of the strobe latch.
const PRINTER_STROBE_MASK: u8 = 0x01;

impl<T: Tracing> X68kBus<T> {
    /// Reads a printer register byte at an odd address.
    pub(super) fn read_printer_register(&self, address: u32) -> u8 {
        match address & 3 {
            PRINTER_DATA_OFFSET => self.printer_data,
            PRINTER_STROBE_OFFSET => self.printer_strobe,
            other => unreachable!("even printer offset {other} is bus-error checked"),
        }
    }

    /// Writes a printer register byte at an odd address.
    pub(super) fn write_printer_register(&mut self, address: u32, value: u8) {
        match address & 3 {
            PRINTER_DATA_OFFSET => self.printer_data = value,
            PRINTER_STROBE_OFFSET => self.printer_strobe = value & PRINTER_STROBE_MASK,
            other => unreachable!("even printer offset {other} is bus-error checked"),
        }
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, M68000AccessSize, M68000FunctionCode};

    use crate::{
        X68kModel,
        bus::test_support::{access, bus},
    };

    #[test]
    fn printer_latches_read_back_and_mirror_across_the_window() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;

        // Reset state: data 0, strobe 1.
        assert_eq!(
            bus.m68000_read(access(0xE8C001, M68000AccessSize::Byte, supervisor)),
            Ok(0x00)
        );
        assert_eq!(
            bus.m68000_read(access(0xE8C003, M68000AccessSize::Byte, supervisor)),
            Ok(0x01)
        );

        bus.m68000_write(access(0xE8C001, M68000AccessSize::Byte, supervisor), 0xA5)
            .unwrap();
        bus.m68000_write(access(0xE8C003, M68000AccessSize::Byte, supervisor), 0xFE)
            .unwrap();
        assert_eq!(
            bus.m68000_read(access(0xE8C001, M68000AccessSize::Byte, supervisor)),
            Ok(0xA5)
        );
        // Only bit 0 of the strobe latch is writable.
        assert_eq!(
            bus.m68000_read(access(0xE8C003, M68000AccessSize::Byte, supervisor)),
            Ok(0x00)
        );

        // The two latches mirror every 4 bytes across the window.
        assert_eq!(
            bus.m68000_read(access(0xE8C005, M68000AccessSize::Byte, supervisor)),
            Ok(0xA5)
        );
        assert_eq!(
            bus.m68000_read(access(0xE8C004, M68000AccessSize::Word, supervisor)),
            Ok(0xFFA5)
        );

        // The strobe pulse raises no printer interrupt without a printer.
        bus.m68000_write(access(0xE8C003, M68000AccessSize::Byte, supervisor), 0x01)
            .unwrap();
        assert_eq!(bus.m68000_interrupt_level(), 0);
    }

    #[test]
    fn even_printer_addresses_raise_bus_errors() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        assert!(
            bus.m68000_read(access(0xE8C000, M68000AccessSize::Byte, supervisor))
                .is_err()
        );
        assert!(
            bus.m68000_write(access(0xE8C002, M68000AccessSize::Byte, supervisor), 0)
                .is_err()
        );
    }
}
