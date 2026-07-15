//! Generic Intel 8255 PPI (mode 0 + bit set/reset).
//!
//! This is the minimal generic part needed for the PC-8801 main<->sub CPU
//! mailbox: two of these are wired back-to-back (see the machine's `ppi_link`).
//! Port A and B carry data bytes; port C carries the handshake strobes. The
//! device itself is a pure latch + control-register decoder; the cross-connect
//! (which port feeds which on the peer, and the port-C nibble swap) lives in the
//! wiring layer so this stays system-agnostic.

save_state::runtime_state! {
/// Snapshot of the 8255 PPI state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I8255State {
    /// Port A output latch.
    pub port_a: u8,
    /// Port B output latch.
    pub port_b: u8,
    /// Port C output latch.
    pub port_c: u8,
    /// Port A input latch driven by external wiring.
    pub input_a: u8,
    /// Port B input latch driven by external wiring.
    pub input_b: u8,
    /// Port C input latch driven by external wiring.
    pub input_c: u8,
    /// Port A read mask: 1 bits read `input_a`, 0 bits read `port_a`.
    pub read_mask_a: u8,
    /// Port B read mask: 1 bits read `input_b`, 0 bits read `port_b`.
    pub read_mask_b: u8,
    /// Port C read mask: 1 bits read `input_c`, 0 bits read `port_c`.
    pub read_mask_c: u8,
    /// Last mode-set control word.
    pub control: u8,
}}

/// Intel 8255 PPI.
#[derive(Debug, Clone)]
pub struct I8255 {
    /// Embedded state for save/restore.
    pub state: I8255State,
}

/// What a [`I8255::write`] changed, so the wiring layer can propagate it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum I8255Write {
    /// Nothing observable changed.
    None,
    /// Port A was written.
    PortA,
    /// Port B was written.
    PortB,
    /// Port C was written (data write or bit set/reset).
    PortC,
    /// The control register selected a new mode (ports reset to 0).
    Mode,
}

/// Control word bit 7: 1 = mode-set, 0 = port C bit set/reset.
const CONTROL_MODE_SET: u8 = 0x80;

/// Power-on control word: mode 0, all ports input.
const CONTROL_RESET: u8 = 0x9B;

impl Default for I8255 {
    fn default() -> Self {
        Self::new()
    }
}

impl I8255 {
    /// Creates a PPI in the power-on state (mode 0, all input, ports cleared).
    pub fn new() -> Self {
        Self {
            state: I8255State {
                port_a: 0,
                port_b: 0,
                port_c: 0,
                input_a: 0,
                input_b: 0,
                input_c: 0,
                read_mask_a: 0xFF,
                read_mask_b: 0xFF,
                read_mask_c: 0xFF,
                control: CONTROL_RESET,
            },
        }
    }

    /// Resets the PPI to the power-on state.
    pub fn reset(&mut self) {
        self.state = I8255State {
            port_a: 0,
            port_b: 0,
            port_c: 0,
            input_a: 0,
            input_b: 0,
            input_c: 0,
            read_mask_a: 0xFF,
            read_mask_b: 0xFF,
            read_mask_c: 0xFF,
            control: CONTROL_RESET,
        };
    }

    /// Reads a register: `sel` is the port address low 2 bits (0=A, 1=B, 2=C, 3=control).
    pub fn read(&self, sel: u8) -> u8 {
        match sel & 0x03 {
            0 => self.read_port(
                self.state.port_a,
                self.state.input_a,
                self.state.read_mask_a,
            ),
            1 => self.read_port(
                self.state.port_b,
                self.state.input_b,
                self.state.read_mask_b,
            ),
            2 => self.read_port(
                self.state.port_c,
                self.state.input_c,
                self.state.read_mask_c,
            ),
            _ => 0xFF,
        }
    }

    /// Writes a register (0=A, 1=B, 2=C, 3=control). Returns what changed.
    pub fn write(&mut self, sel: u8, value: u8) -> I8255Write {
        match sel & 0x03 {
            0 => {
                self.state.port_a = value;
                I8255Write::PortA
            }
            1 => {
                self.state.port_b = value;
                I8255Write::PortB
            }
            2 => {
                self.state.port_c = value;
                I8255Write::PortC
            }
            _ => self.write_control(value),
        }
    }

    fn write_control(&mut self, value: u8) -> I8255Write {
        if value & CONTROL_MODE_SET != 0 {
            self.state.control = value;
            self.update_read_masks(value);
            self.state.port_a = 0;
            self.state.port_b = 0;
            self.state.port_c = 0;
            I8255Write::Mode
        } else {
            // Bit set/reset on port C: bits 3-1 select the bit, bit 0 sets/resets.
            let bit = (value >> 1) & 0x07;
            if value & 0x01 != 0 {
                self.state.port_c |= 1 << bit;
            } else {
                self.state.port_c &= !(1 << bit);
            }
            I8255Write::PortC
        }
    }

    fn read_port(&self, output: u8, input: u8, read_mask: u8) -> u8 {
        (input & read_mask) | (output & !read_mask)
    }

    fn update_read_masks(&mut self, value: u8) {
        let port_a_mode = if value & 0x40 != 0 {
            2
        } else {
            (value >> 5) & 0x01
        };
        self.state.read_mask_a = if port_a_mode == 2 || value & 0x10 != 0 {
            0xFF
        } else {
            0x00
        };
        self.state.read_mask_b = if value & 0x02 != 0 { 0xFF } else { 0x00 };
        self.state.read_mask_c = (if value & 0x08 != 0 { 0xF0 } else { 0x00 })
            | (if value & 0x01 != 0 { 0x0F } else { 0x00 });
    }

    /// Port A latch.
    pub fn port_a(&self) -> u8 {
        self.state.port_a
    }

    /// Port B latch.
    pub fn port_b(&self) -> u8 {
        self.state.port_b
    }

    /// Port C latch.
    pub fn port_c(&self) -> u8 {
        self.state.port_c
    }

    /// Drives port A from the peer (cross-connect input).
    pub fn set_port_a(&mut self, value: u8) {
        self.state.input_a = value;
    }

    /// Drives port B from the peer (cross-connect input).
    pub fn set_port_b(&mut self, value: u8) {
        self.state.input_b = value;
    }

    /// Drives port C from the peer (cross-connect input).
    pub fn set_port_c(&mut self, value: u8) {
        self.state.input_c = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_set_updates_read_masks_and_clears_outputs() {
        let mut ppi = I8255::new();
        ppi.write(0, 0xAA);
        ppi.write(1, 0xBB);
        ppi.write(2, 0xCC);
        let changed = ppi.write(3, 0x80);
        assert_eq!(changed, I8255Write::Mode);
        assert_eq!(ppi.port_a(), 0);
        assert_eq!(ppi.port_b(), 0);
        assert_eq!(ppi.port_c(), 0);
        assert_eq!(ppi.read(0), 0);
        assert_eq!(ppi.read(1), 0);
        assert_eq!(ppi.read(2), 0);
        assert_eq!(ppi.read(3), 0xFF);
    }

    #[test]
    fn bit_set_reset_touches_only_addressed_bit() {
        let mut ppi = I8255::new();
        // Set bit 4.
        let changed = ppi.write(3, (4 << 1) | 1);
        assert_eq!(changed, I8255Write::PortC);
        assert_eq!(ppi.port_c(), 0x10);
        // Set bit 0.
        ppi.write(3, 0x01);
        assert_eq!(ppi.port_c(), 0x11);
        // Reset bit 4.
        ppi.write(3, 4 << 1);
        assert_eq!(ppi.port_c(), 0x01);
    }

    #[test]
    fn read_back_data_ports() {
        let mut ppi = I8255::new();
        ppi.write(3, 0x80);
        assert_eq!(ppi.write(0, 0x5A), I8255Write::PortA);
        assert_eq!(ppi.write(1, 0xA5), I8255Write::PortB);
        assert_eq!(ppi.read(0), 0x5A);
        assert_eq!(ppi.read(1), 0xA5);
    }

    #[test]
    fn input_masks_select_peer_inputs_for_cpu_reads() {
        let mut ppi = I8255::new();
        ppi.write(0, 0x12);
        ppi.write(1, 0x34);
        ppi.write(2, 0x56);
        ppi.set_port_a(0xA0);
        ppi.set_port_b(0xB1);
        ppi.set_port_c(0xC2);

        assert_eq!(ppi.read(0), 0xA0);
        assert_eq!(ppi.read(1), 0xB1);
        assert_eq!(ppi.read(2), 0xC2);

        // A out, B in, C high in / low out.
        ppi.write(3, 0x8A);
        assert_eq!(ppi.read(0), 0x00);
        assert_eq!(ppi.read(1), 0xB1);
        assert_eq!(ppi.read(2), 0xC0);
    }
}
