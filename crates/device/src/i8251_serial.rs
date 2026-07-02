//! i8251A USART serial controller for the PC-98 RS-232C port.
//!
//! The PC-98 RS-232C serial port uses an i8251A USART chip.
//! Port 0x30 is the data register (read/write serial data).
//! Port 0x32 is the control register (read: status, write: mode/command word).
//!
//! TxRDY and TxEMPTY are always set, DSR is always set. Received data can be
//! injected via [`I8251Serial::push_received_byte`].
//!
//! Reference: `undoc98/io_rs232c.txt`.

use std::collections::VecDeque;

/// Status register bit: Transmitter Ready.
const STATUS_TXRDY: u8 = 1 << 0;

/// Status register bit: Receiver Ready.
const STATUS_RXRDY: u8 = 1 << 1;

/// Status register bit: Transmitter Empty.
const STATUS_TXEMPTY: u8 = 1 << 2;

/// Status register bit: Parity Error.
const STATUS_PE: u8 = 1 << 3;

/// Status register bit: Overrun Error.
const STATUS_OE: u8 = 1 << 4;

/// Status register bit: Framing Error.
const STATUS_FE: u8 = 1 << 5;

/// Status register bit: Data Set Ready.
const STATUS_DSR: u8 = 1 << 7;

/// Mask for bits always set when reading the status register: TxRDY | TxEMPTY | DSR.
const STATUS_ALWAYS_ON: u8 = STATUS_TXRDY | STATUS_TXEMPTY | STATUS_DSR;

/// Mask for error bits (PE | OE | FE), cleared by the Error Reset command.
const STATUS_ERROR_MASK: u8 = STATUS_PE | STATUS_OE | STATUS_FE;

/// Command register bit: Error Reset - clears FE, OE, PE in status.
const CMD_ER: u8 = 1 << 4;

/// Command register bit: Internal Reset - next write becomes a mode word.
const CMD_IR: u8 = 1 << 6;

/// Snapshot of the i8251A serial controller state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I8251SerialState {
    /// Mode word (set after reset).
    pub mode: u8,
    /// Command word.
    pub command: u8,
    /// Status register (error bits only; TxRDY/TxEMPTY/DSR are always-on via `STATUS_ALWAYS_ON`).
    pub status: u8,
    /// Last data register value (always 0xFF with no cable).
    pub data: u8,
    /// Next write to port 0x32 is a mode word (true after reset or internal reset).
    pub expect_mode: bool,
}

/// i8251A serial controller (RS-232C).
pub struct I8251Serial {
    /// Embedded state for save/restore.
    pub state: I8251SerialState,
    /// Receive FIFO for injected data (not part of saveable state).
    rx_fifo: VecDeque<u8>,
    /// When true, bytes written to the data register are buffered in `midi_tx`
    /// so a machine can forward them to a MIDI sound module.
    /// Off by default, so the PC-98 RS-232C port is unaffected.
    midi_capture: bool,
    /// Transmitted bytes captured for MIDI output (not part of saveable state).
    midi_tx: Vec<u8>,
}

impl Default for I8251Serial {
    fn default() -> Self {
        Self::new()
    }
}

impl I8251Serial {
    /// Creates a new serial controller in reset state.
    pub fn new() -> Self {
        Self {
            state: I8251SerialState {
                mode: 0x00,
                command: 0x00,
                status: 0x00,
                data: 0xFF,
                expect_mode: true,
            },
            rx_fifo: VecDeque::new(),
            midi_capture: false,
            midi_tx: Vec::new(),
        }
    }

    /// Enables MIDI transmit capture: subsequent writes to the data register
    /// are buffered and can be drained with [`I8251Serial::flush_midi_into`].
    pub fn enable_midi_capture(&mut self) {
        self.midi_capture = true;
    }

    /// Drains captured transmit bytes into `target` and clears the buffer.
    pub fn flush_midi_into(&mut self, target: &mut Vec<u8>) {
        target.extend_from_slice(&self.midi_tx);
        self.midi_tx.clear();
    }

    /// Reads the data register (port 0x30).
    ///
    /// Returns `(data, clear_irq, retrigger_irq)`:
    /// - `data`: the received byte (or last data value when FIFO is empty),
    /// - `clear_irq`: whether IRQ 4 should be cleared,
    /// - `retrigger_irq`: whether IRQ 4 should be re-raised (more buffered data).
    pub fn read_data(&mut self) -> (u8, bool, bool) {
        let Some(byte) = self.rx_fifo.pop_front() else {
            return (self.state.data, false, false);
        };

        self.state.data = byte;
        if self.rx_fifo.is_empty() {
            self.state.status &= !STATUS_RXRDY;
            (byte, true, false)
        } else {
            (byte, false, true)
        }
    }

    /// Pushes a received byte into the FIFO and sets RxRDY.
    pub fn push_received_byte(&mut self, data: u8) {
        self.rx_fifo.push_back(data);
        self.state.data = data;
        self.state.status |= STATUS_RXRDY;
    }

    /// Reads the status register (port 0x32).
    ///
    /// Always sets DSR, TxEMPTY, and TxRDY (`STATUS_ALWAYS_ON`).
    pub fn read_status(&self) -> u8 {
        self.state.status | STATUS_ALWAYS_ON
    }

    /// Writes to the data register (port 0x30).
    ///
    /// No-op with no cable attached (BIOS may send XOFF), unless MIDI capture
    /// is enabled, in which case the byte is buffered for MIDI output.
    pub fn write_data(&mut self, value: u8) {
        if self.midi_capture {
            self.midi_tx.push(value);
        }
    }

    /// Writes to the control register (port 0x32).
    ///
    /// After reset, the first write is a mode word. Subsequent writes
    /// are command words until an internal reset (`CMD_IR`) occurs.
    pub fn write_command(&mut self, value: u8) {
        if self.state.expect_mode {
            self.state.mode = value;
            self.state.expect_mode = false;
            return;
        }

        self.state.command = value;

        if value & CMD_IR != 0 {
            self.state.expect_mode = true;
        }

        if value & CMD_ER != 0 {
            self.state.status &= !STATUS_ERROR_MASK;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_state_returns_correct_status() {
        let serial = I8251Serial::new();
        assert_eq!(serial.read_status(), 0x85);
    }

    #[test]
    fn bios_init_sequence() {
        let mut serial = I8251Serial::new();

        // BIOS sends 3 dummy writes to force the chip into a known state.
        // These go into the mode/command state machine:
        // Write 1: absorbed as mode word (expect_mode was true)
        serial.write_command(0x00);
        assert!(!serial.state.expect_mode);

        // Write 2: absorbed as command word
        serial.write_command(0x00);

        // Write 3: also a command word
        serial.write_command(0x00);

        // Internal reset (CMD_IR = 0x40) - makes next write a mode word
        serial.write_command(0x40);
        assert!(serial.state.expect_mode);

        // Mode word (e.g. async 8-bit, 1 stop, 16x baud = 0x4E)
        serial.write_command(0x4E);
        assert_eq!(serial.state.mode, 0x4E);
        assert!(!serial.state.expect_mode);

        // Command word (e.g. TxEN | DTR | RxEN | RTS = 0x37)
        serial.write_command(0x37);
        assert_eq!(serial.state.command, 0x37);

        // Status should still be 0x85
        assert_eq!(serial.read_status(), 0x85);
    }

    #[test]
    fn midi_capture_off_discards_transmitted_bytes() {
        let mut serial = I8251Serial::new();
        serial.write_data(0x90);
        serial.write_data(0x40);
        let mut drained = Vec::new();
        serial.flush_midi_into(&mut drained);
        assert!(drained.is_empty());
    }

    #[test]
    fn midi_capture_buffers_bytes_in_order() {
        let mut serial = I8251Serial::new();
        serial.enable_midi_capture();
        let bytes = [0x90, 0x40, 0x7F, 0x80, 0x40, 0x00];
        for &byte in &bytes {
            serial.write_data(byte);
        }
        let mut drained = Vec::new();
        serial.flush_midi_into(&mut drained);
        assert_eq!(drained, bytes);

        // A second drain returns nothing.
        let mut again = Vec::new();
        serial.flush_midi_into(&mut again);
        assert!(again.is_empty());
    }

    #[test]
    fn read_data_returns_ff() {
        let mut serial = I8251Serial::new();
        assert_eq!(serial.read_data(), (0xFF, false, false));
    }

    #[test]
    fn push_received_byte_sets_rxrdy() {
        let mut serial = I8251Serial::new();
        serial.push_received_byte(0x41);
        assert_ne!(serial.read_status() & STATUS_RXRDY, 0);
    }

    #[test]
    fn read_data_consumes_fifo() {
        let mut serial = I8251Serial::new();
        serial.push_received_byte(0x41);
        let (data, clear, retrigger) = serial.read_data();
        assert_eq!(data, 0x41);
        assert!(clear);
        assert!(!retrigger);
        assert_eq!(serial.read_status() & STATUS_RXRDY, 0);
    }

    #[test]
    fn read_data_retriggers_with_more_data() {
        let mut serial = I8251Serial::new();
        serial.push_received_byte(0x41);
        serial.push_received_byte(0x42);
        let (data, clear, retrigger) = serial.read_data();
        assert_eq!(data, 0x41);
        assert!(!clear);
        assert!(retrigger);
        assert_ne!(serial.read_status() & STATUS_RXRDY, 0);
    }

    #[test]
    fn error_reset_clears_error_bits() {
        let mut serial = I8251Serial::new();

        // Consume initial mode word expectation
        serial.write_command(0x00);

        // Manually set error bits
        serial.state.status = STATUS_PE | STATUS_OE | STATUS_FE;
        assert_eq!(serial.state.status & STATUS_ERROR_MASK, STATUS_ERROR_MASK);

        // Error reset command
        serial.write_command(CMD_ER);
        assert_eq!(serial.state.status & STATUS_ERROR_MASK, 0);
    }

    #[test]
    fn internal_reset_expects_mode_word() {
        let mut serial = I8251Serial::new();

        // Consume initial mode word expectation
        serial.write_command(0x00);
        assert!(!serial.state.expect_mode);

        // Send a normal command
        serial.write_command(0x37);
        assert!(!serial.state.expect_mode);

        // Internal reset
        serial.write_command(CMD_IR);
        assert!(serial.state.expect_mode);

        // Next write should be treated as mode word
        serial.write_command(0x4E);
        assert_eq!(serial.state.mode, 0x4E);
        assert!(!serial.state.expect_mode);
    }
}
