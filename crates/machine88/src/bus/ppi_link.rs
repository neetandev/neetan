//! PC-8801 main<->sub CPU mailbox: two i8255 PPIs wired back-to-back.
//!
//! Port A of each side feeds port B of the other (data bytes), and port C is
//! cross-connected with the nibbles swapped (`(C & 0x0f) << 4 | (C & 0xf0) >> 4`),
//! so each side's driven strobe nibble lands in the other side's read nibble.
//! A control-register write that changes a side's configuration updates its read
//! masks, and any port-C change arms the interleave resync window so the peer CPU
//! runs promptly enough to advance the handshake.

use common::Tracing;
use device::i8255::I8255Write;

use super::{Pc8801Bus, SYNC_SLICE};

/// Swaps the high and low nibbles for the port-C cross-wiring.
fn swap_nibbles(value: u8) -> u8 {
    value.rotate_left(4)
}

impl<T: Tracing> Pc8801Bus<T> {
    /// Reacts to a host-side (main I/O 0xFE/0xFF) PPI register change.
    pub(crate) fn on_ppi_main_change(&mut self, changed: I8255Write) {
        match changed {
            I8255Write::PortC => {
                self.ppi_sub
                    .set_port_c(swap_nibbles(self.ppi_main.port_c()));
                self.arm_resync();
            }
            I8255Write::Mode => {
                self.ppi_sub.set_port_b(self.ppi_main.port_a());
                self.ppi_sub.set_port_a(self.ppi_main.port_b());
                self.ppi_sub
                    .set_port_c(swap_nibbles(self.ppi_main.port_c()));
                self.arm_resync();
            }
            I8255Write::None | I8255Write::PortA | I8255Write::PortB => {}
        }
    }

    /// Reacts to a disk-side (sub I/O 0xFE/0xFF) PPI register change.
    pub(crate) fn on_ppi_sub_change(&mut self, changed: I8255Write) {
        match changed {
            I8255Write::PortC => {
                self.ppi_main
                    .set_port_c(swap_nibbles(self.ppi_sub.port_c()));
                self.arm_resync();
            }
            I8255Write::Mode => {
                self.ppi_main.set_port_b(self.ppi_sub.port_a());
                self.ppi_main.set_port_a(self.ppi_sub.port_b());
                self.ppi_main
                    .set_port_c(swap_nibbles(self.ppi_sub.port_c()));
                self.arm_resync();
            }
            I8255Write::None | I8255Write::PortA | I8255Write::PortB => {}
        }
    }

    /// Arms the tight-interleave resync window: a handshake strobe means the peer
    /// CPU must make prompt progress (single-step the peer while a strobe is live).
    fn arm_resync(&mut self) {
        self.resync_until = self.current_cycle + SYNC_SLICE;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ClockSelect, Pc8801Model};

    fn bus() -> Pc8801Bus {
        Pc8801Bus::new(Pc8801Model::PC8801MC, ClockSelect::FourMhz, 48_000)
    }

    #[test]
    fn nibble_swap_is_symmetric() {
        assert_eq!(swap_nibbles(0x12), 0x21);
        assert_eq!(swap_nibbles(0x80), 0x08);
        assert_eq!(swap_nibbles(0x0F), 0xF0);
    }

    #[test]
    fn host_data_byte_reaches_sub_port_b() {
        let mut bus = bus();
        bus.io_write(0xFC, 0x5A); // host writes port A
        assert_eq!(bus.ppi_sub.read(1), 0x5A, "sub reads host's data on port B");
    }

    #[test]
    fn sub_data_byte_reaches_host_port_b() {
        let mut bus = bus();
        bus.sub_io_write(0xFC, 0xA5); // sub writes port A
        assert_eq!(
            bus.ppi_main.read(1),
            0xA5,
            "host reads the sub's data on port B"
        );
    }

    #[test]
    fn host_port_c_strobe_appears_in_sub_read_nibble() {
        let mut bus = bus();
        // Host bit-sets DAV (port C bit 4) via the control register BSR.
        bus.io_write(0xFF, (4 << 1) | 1);
        // Host's high nibble bit 4 (DAV) lands in the sub's low nibble bit 0.
        assert_eq!(bus.ppi_sub.read(2) & 0x0F, 0x01);
        // And the strobe arms the resync window.
        assert!(bus.resync_until > 0);
    }

    #[test]
    fn sub_port_c_strobe_appears_in_host_read_nibble() {
        let mut bus = bus();
        // Sub bit-sets its DAV (port C bit 0).
        bus.sub_io_write(0xFF, 0x01);
        // Sub's low nibble bit 0 lands in the host's high nibble bit 4.
        assert_eq!(bus.ppi_main.read(2) & 0xF0, 0x10);
    }
}
