//! i8251A USART keyboard controller for the PC-98.
//!
//! The PC-98 keyboard connects to the system board via an i8251A USART chip.
//! Port 0x41 is the data register (read: scan codes, write: keyboard commands).
//! Port 0x43 is the control register (read: status, write: mode/command word).
//!
//! Reference: `undoc98/io_kb.txt`.

use std::{
    collections::VecDeque,
    ops::{Deref, DerefMut},
};

/// Status register bit: Transmitter Ready.
/// Ref: undoc98 `io_kb.txt` port 0x43 bit 0
const STATUS_TXRDY: u8 = 1 << 0;

/// Status register bit: Receiver Ready (data available from keyboard).
/// Ref: undoc98 `io_kb.txt` port 0x43 bit 1
const STATUS_RXRDY: u8 = 1 << 1;

/// Status register bit: Transmitter Empty.
/// Ref: undoc98 `io_kb.txt` port 0x43 bit 2
const STATUS_TXEMPTY: u8 = 1 << 2;

/// Status register bit: Parity Error.
/// Ref: undoc98 `io_kb.txt` port 0x43 bit 3
const STATUS_PE: u8 = 1 << 3;

/// Status register bit: Overrun Error.
/// Ref: undoc98 `io_kb.txt` port 0x43 bit 4
const STATUS_OE: u8 = 1 << 4;

/// Status register bit: Framing Error.
/// Ref: undoc98 `io_kb.txt` port 0x43 bit 5
const STATUS_FE: u8 = 1 << 5;

/// Status register bit: Data Set Ready.
/// Ref: undoc98 `io_kb.txt` port 0x43 bit 7
const STATUS_DSR: u8 = 1 << 7;

/// Mask for bits always set when reading the status register: TxRDY | TxEMPTY | DSR.
const STATUS_ALWAYS_ON: u8 = STATUS_TXRDY | STATUS_TXEMPTY | STATUS_DSR;

/// Mask for error bits (PE | OE | FE), cleared by the Error Reset command.
/// Ref: undoc98 `io_kb.txt` port 0x43 command bit 4
const STATUS_ERROR_MASK: u8 = STATUS_PE | STATUS_OE | STATUS_FE;

/// Command register bit: Transmit Enable - enables sending commands to the keyboard.
/// Ref: undoc98 `io_kb.txt` port 0x43 command bit 0
const CMD_TXEN: u8 = 1 << 0;

/// Command register bit: Send Break - controls RST# signal to keyboard.
/// Falling edge (1->0) triggers keyboard reset.
/// Ref: undoc98 `io_kb.txt` port 0x43 command bit 3
const CMD_SBRK: u8 = 1 << 3;

/// Command register bit: Error Reset - clears FE, OE, PE in status.
/// Ref: undoc98 `io_kb.txt` port 0x43 command bit 4
const CMD_ER: u8 = 1 << 4;

/// Command register bit: Internal Reset - next write becomes a mode word.
/// Ref: undoc98 `io_kb.txt` port 0x43 command bit 6
const CMD_IR: u8 = 1 << 6;

/// Default mode word: async 8-bit odd parity, 1 stop bit, 16x baud factor (0x5E).
/// Ref: undoc98 `io_kb.txt` port 0x43 mode word
const DEFAULT_MODE: u8 = 0x5E;

/// Default data register value on reset (no key pressed).
const DEFAULT_DATA: u8 = 0xFF;
/// Maximum number of buffered keyboard scan codes.
const RX_FIFO_CAPACITY: usize = 16;

save_state::runtime_state! {
/// Authoritative i8251A keyboard controller state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I8251KeyboardState {
    /// Mode word (set after reset, default `DEFAULT_MODE` = 0x5E for async 8-bit odd parity
    /// 1 stop 16x baud).
    pub mode: u8,
    /// Command word.
    pub command: u8,
    /// Status register (error bits only; TxRDY/TxEMPTY/DSR are always-on via `STATUS_ALWAYS_ON`).
    pub status: u8,
    /// Last received scan code.
    pub data: u8,
    /// Data available from keyboard (drives `STATUS_RXRDY` bit).
    pub rx_ready: bool,
    /// Buffered scan codes from keyboard (oldest first).
    pub rx_fifo: VecDeque<u8>,
    /// Next write to port 0x43 is a mode word (true after reset).
    pub expect_mode: bool,
}}

/// i8251A keyboard controller.
pub struct I8251Keyboard {
    /// Embedded state for save/restore.
    pub state: I8251KeyboardState,
}

impl_simple_state_accessors!(I8251Keyboard, I8251KeyboardState);

impl Deref for I8251Keyboard {
    type Target = I8251KeyboardState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for I8251Keyboard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Default for I8251Keyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl I8251Keyboard {
    /// Creates a new keyboard controller in reset state.
    pub fn new() -> Self {
        Self {
            state: I8251KeyboardState {
                mode: DEFAULT_MODE,
                command: 0x00,
                status: 0x00,
                data: DEFAULT_DATA,
                rx_ready: false,
                rx_fifo: VecDeque::new(),
                expect_mode: true,
            },
        }
    }

    /// Reads the data register (port 0x41).
    ///
    /// Returns the oldest buffered scan code and updates RxRDY.
    ///
    /// Returned tuple:
    /// - `u8`: scan code data (or last data value when no data is buffered),
    /// - `bool`: whether IRQ 1 should be cleared,
    /// - `bool`: whether IRQ 1 should be re-raised (more buffered data remains).
    pub fn read_data(&mut self) -> (u8, bool, bool) {
        let Some(code) = self.rx_fifo.pop_front() else {
            self.rx_ready = false;
            self.status &= !STATUS_RXRDY;
            return (self.data, false, false);
        };

        self.data = code;
        if self.rx_fifo.is_empty() {
            self.rx_ready = false;
            self.status &= !STATUS_RXRDY;
            (code, true, false)
        } else {
            self.rx_ready = true;
            self.status |= STATUS_RXRDY;
            (code, false, true)
        }
    }

    /// Reads the status register (port 0x43).
    ///
    /// Always sets DSR, TxEMPTY, and TxRDY (`STATUS_ALWAYS_ON`).
    /// Sets RxRDY when data is available.
    pub fn read_status(&self) -> u8 {
        let mut result = self.status | STATUS_ALWAYS_ON;
        if self.rx_ready {
            result |= STATUS_RXRDY;
        }
        result
    }

    /// Writes to the data register (port 0x41).
    ///
    /// If `CMD_TXEN` is set, the value is a keyboard command.
    /// Otherwise, it is stored as a mode word.
    pub fn write_data(&mut self, value: u8) {
        if self.command & CMD_TXEN != 0 {
            common::trace!("Keyboard command sent: {value:#04X}");
        } else {
            self.mode = value;
        }
    }

    /// Writes to the control register (port 0x43).
    ///
    /// After reset, the first write is a mode word. Subsequent writes
    /// are command words until an internal reset (`CMD_IR`) occurs.
    pub fn write_command(&mut self, value: u8) {
        if self.expect_mode {
            self.mode = value;
            self.expect_mode = false;
            return;
        }

        let prev_command = self.command;
        self.command = value;

        if value & CMD_IR != 0 {
            self.expect_mode = true;
        }

        if value & CMD_ER != 0 {
            self.status &= !STATUS_ERROR_MASK;
        }

        // SBRK falling edge (1->0) triggers keyboard reset.
        if prev_command & CMD_SBRK != 0 && value & CMD_SBRK == 0 {
            common::trace!("Keyboard reset signal (SBRK 1->0)");
            self.command = 0x00;
            self.status = 0x00;
            self.rx_ready = false;
            self.rx_fifo.clear();
        }
    }

    /// Pushes a scan code from the keyboard (for future input support).
    pub fn push_scancode(&mut self, code: u8) {
        if self.rx_fifo.len() >= RX_FIFO_CAPACITY {
            common::warn!("Keyboard RX FIFO overflow, dropping scan code {code:#04X}");
            return;
        }
        self.rx_fifo.push_back(code);
        self.data = code;
        self.rx_ready = true;
        self.status |= STATUS_RXRDY;
    }

    /// Returns whether RxRDY is set (data available from keyboard).
    pub fn has_rx_ready(&self) -> bool {
        self.rx_ready
    }
}
