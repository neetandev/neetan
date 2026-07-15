//! Main i8255 PPI wiring for the X1 (ports 0x1A00-0x1A03).
//!
//! Port A is unused (reads 0xFF). Port B is assembled dynamically by the bus:
//! the sub-CPU handshake (IBF/OBF/break), the beam-derived V-DISP and V-SYNC
//! flags, the RAM-bank flag, and the cassette read bit. Port C is the I/O system
//! port: bit 6 selects 320-column (hi-speed pixel clock), bit 5 is the I/O-bus
//! mode switch (whose falling edge latches VRAM access mode), and bit 0 is the
//! cassette output. Only port C carries side effects the bus must apply.

use device::i8255::I8255;

/// Side effect of a PPI write for the bus to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PpiEffect {
    /// Nothing for the bus to do.
    None,
    /// A port C write changed the I/O system port.
    PortC {
        /// 320-column / hi-speed mode (port C bit 6).
        column40: bool,
        /// The I/O-bus mode switch fell from high to low; latch VRAM mode.
        vram_mode_latch: bool,
        /// Cassette output level (port C bit 0).
        cassette_out: bool,
    },
}

const PORT_C_COLUMN40: u8 = 0x40;
const PORT_C_IO_SWITCH: u8 = 0x20;
const PORT_C_CASSETTE_OUT: u8 = 0x01;

/// Main PPI plus the shadow state port C reads combine.
pub(crate) struct PpiLink {
    ppi: I8255,
    io_sys: u8,
    io_switch_high: bool,
}

save_state::runtime_state! {
/// Authoritative state of the linked PPI handshake lines.
#[derive(Clone)]
pub(crate) struct PpiLinkState {
    ppi: device::i8255::I8255State,
    io_system: u8,
    io_switch_high: bool,
}}

impl PpiLink {
    /// Creates a PPI in the power-on state.
    pub(crate) fn new() -> Self {
        Self {
            ppi: I8255::new(),
            io_sys: 0,
            io_switch_high: false,
        }
    }

    pub(crate) fn capture_state(&self) -> PpiLinkState {
        PpiLinkState {
            ppi: self.ppi.state.clone(),
            io_system: self.io_sys,
            io_switch_high: self.io_switch_high,
        }
    }

    pub(crate) fn restore_state(&mut self, state: PpiLinkState) {
        self.ppi.state = state.ppi;
        self.io_sys = state.io_system;
        self.io_switch_high = state.io_switch_high;
    }

    /// Reads a PPI register. `port_b` is the bus-assembled port B value.
    pub(crate) fn read(&self, offset: u8, port_b: u8) -> u8 {
        match offset & 0x03 {
            0 => 0xFF,
            1 => port_b,
            2 => self.read_port_c(),
            _ => self.ppi.read(3),
        }
    }

    /// The computed port C read value (the I/O switch bit reads inverted, as on
    /// hardware where it reflects the physical mode switch).
    fn read_port_c(&self) -> u8 {
        (self.io_sys & 0x9F)
            | (self.io_sys & PORT_C_COLUMN40)
            | (0xFF ^ (self.io_sys & PORT_C_IO_SWITCH))
    }

    /// Writes a PPI register, returning any side effect for the bus.
    pub(crate) fn write(&mut self, offset: u8, value: u8) -> PpiEffect {
        self.ppi.write(offset & 0x03, value);
        match offset & 0x03 {
            2 | 3 => self.update_port_c(),
            _ => PpiEffect::None,
        }
    }

    fn update_port_c(&mut self) -> PpiEffect {
        let port_c = self.ppi.port_c();
        self.io_sys = port_c;
        let io_switch_high = (port_c & PORT_C_IO_SWITCH) != 0;
        let vram_mode_latch = self.io_switch_high && !io_switch_high;
        self.io_switch_high = io_switch_high;
        PpiEffect::PortC {
            column40: (port_c & PORT_C_COLUMN40) != 0,
            vram_mode_latch,
            cassette_out: (port_c & PORT_C_CASSETTE_OUT) != 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Control word: mode 0, all ports output, so port C latches writes.
    const MODE_ALL_OUTPUT: u8 = 0x80;

    #[test]
    fn port_a_reads_open_bus_and_port_b_is_passed_through() {
        let ppi = PpiLink::new();
        assert_eq!(ppi.read(0, 0x00), 0xFF);
        assert_eq!(ppi.read(1, 0xA5), 0xA5);
    }

    #[test]
    fn port_c_write_reports_column_and_cassette_bits() {
        let mut ppi = PpiLink::new();
        ppi.write(3, MODE_ALL_OUTPUT);
        let effect = ppi.write(2, PORT_C_COLUMN40 | PORT_C_CASSETTE_OUT);
        assert_eq!(
            effect,
            PpiEffect::PortC {
                column40: true,
                vram_mode_latch: false,
                cassette_out: true,
            }
        );
    }

    #[test]
    fn io_switch_falling_edge_latches_vram_mode() {
        let mut ppi = PpiLink::new();
        ppi.write(3, MODE_ALL_OUTPUT);
        // Raise the I/O switch, then drop it: the falling edge latches.
        let raised = ppi.write(2, PORT_C_IO_SWITCH);
        assert!(matches!(
            raised,
            PpiEffect::PortC {
                vram_mode_latch: false,
                ..
            }
        ));
        let dropped = ppi.write(2, 0x00);
        assert!(matches!(
            dropped,
            PpiEffect::PortC {
                vram_mode_latch: true,
                ..
            }
        ));
    }
}
