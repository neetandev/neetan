//! FM Towns serial keyboard controller (I/O 0x0600-0x0604, IRQ 1).
//!
//! The host forwards a single byte per key event (bit 7 = release, bits 0-6 =
//! the FM Towns JIS scancode). The controller expands each event into the real
//! two-byte serial packet: a flag byte (bit 7 set, bit 4 = release, bit 3 =
//! Ctrl, bit 2 = Shift, JIS type in bits 5-6) followed by the scancode byte.
//! Bytes queue in a receive FIFO. The controller raises IRQ 1 while data is
//! available and interrupts are enabled.

use std::collections::VecDeque;

/// Receive FIFO capacity in bytes (holds paired make/break packets).
const FIFO_CAPACITY: usize = 32;

/// Flag-byte base for a JIS key press (bit 7 set, JIS type in bits 5-6).
const FLAG_JIS_PRESS: u8 = 0xA0;
/// Flag-byte base for a JIS key release (adds the release bit).
const FLAG_JIS_RELEASE: u8 = 0xB0;
/// Shift-held bit in the flag byte.
const FLAG_SHIFT: u8 = 0x04;
/// Ctrl-held bit in the flag byte.
const FLAG_CTRL: u8 = 0x08;

/// FM Towns JIS scancodes for the modifier keys, tracked to fill the flag byte.
const SCANCODE_CTRL: u8 = 0x52;
const SCANCODE_SHIFT: u8 = 0x53;

/// Keyboard identity reply bytes returned after a reset/identify command.
const KEYBOARD_ID: [u8; 2] = [0xB0, 0x7F];

save_state::runtime_state! {
/// Authoritative FM Towns keyboard FIFO and modifier state.
#[derive(Clone)]
pub struct TownsKeyboardState {
    fifo: VecDeque<u8>,
    irq_enabled: bool,
    interrupt_pending: bool,
    shift_held: bool,
    ctrl_held: bool,
}}

/// Serial keyboard controller.
pub struct TownsKeyboard {
    /// Serial bytes waiting to be read by the guest.
    fifo: VecDeque<u8>,
    /// Interrupt-enable flag (I/O 0x0604 bit 0).
    irq_enabled: bool,
    /// Interrupt-pending latch, set while data is available.
    interrupt_pending: bool,
    /// Live Shift-held state, tracked from forwarded modifier events.
    shift_held: bool,
    /// Live Ctrl-held state, tracked from forwarded modifier events.
    ctrl_held: bool,
}

impl Default for TownsKeyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl TownsKeyboard {
    /// Creates a keyboard controller with an empty FIFO and interrupts disabled.
    pub fn new() -> Self {
        Self {
            fifo: VecDeque::with_capacity(FIFO_CAPACITY),
            irq_enabled: false,
            interrupt_pending: false,
            shift_held: false,
            ctrl_held: false,
        }
    }

    /// Captures queued serial bytes and modifier state.
    pub fn capture_state(&self) -> TownsKeyboardState {
        TownsKeyboardState {
            fifo: self.fifo.clone(),
            irq_enabled: self.irq_enabled,
            interrupt_pending: self.interrupt_pending,
            shift_held: self.shift_held,
            ctrl_held: self.ctrl_held,
        }
    }

    /// Restores queued serial bytes and modifier state.
    pub fn restore_state(
        &mut self,
        state: TownsKeyboardState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.fifo.len() > FIFO_CAPACITY {
            return Err(save_state::StateValidationError::new(
                "FM Towns keyboard FIFO is too large",
            ));
        }
        self.fifo = state.fifo;
        self.irq_enabled = state.irq_enabled;
        self.interrupt_pending = state.interrupt_pending;
        self.shift_held = state.shift_held;
        self.ctrl_held = state.ctrl_held;
        Ok(())
    }

    /// Queues a key event forwarded from the host, expanding it into the two-byte
    /// serial packet. NULL (scancode 0) events are ignored.
    pub fn push_scancode(&mut self, code: u8) {
        let release = code & 0x80 != 0;
        let scancode = code & 0x7F;
        if scancode == 0 {
            return;
        }

        match scancode {
            SCANCODE_SHIFT => self.shift_held = !release,
            SCANCODE_CTRL => self.ctrl_held = !release,
            _ => {}
        }

        let mut flag = if release {
            FLAG_JIS_RELEASE
        } else {
            FLAG_JIS_PRESS
        };
        if self.shift_held {
            flag |= FLAG_SHIFT;
        }
        if self.ctrl_held {
            flag |= FLAG_CTRL;
        }
        self.push_byte(flag);
        self.push_byte(scancode);
        self.interrupt_pending = true;
    }

    /// Pushes one serial byte, dropping the oldest when the FIFO is full.
    fn push_byte(&mut self, byte: u8) {
        if self.fifo.len() >= FIFO_CAPACITY {
            self.fifo.pop_front();
        }
        self.fifo.push_back(byte);
    }

    /// Reads the data port (0x0600): pops one byte and keeps the interrupt
    /// pending while more bytes remain.
    pub fn read_data(&mut self) -> u8 {
        let byte = self.fifo.pop_front().unwrap_or(0);
        self.interrupt_pending = !self.fifo.is_empty();
        byte
    }

    /// Reads the status port (0x0602): bit 0 set means a byte is available.
    pub fn read_status(&self) -> u8 {
        if self.fifo.is_empty() { 0 } else { 1 }
    }

    /// Reads the IRQ-status port (0x0604): bit 0 reflects the pending interrupt.
    pub fn read_irq(&self) -> u8 {
        if self.interrupt_pending { 1 } else { 0 }
    }

    /// Writes the IRQ-control port (0x0604): bit 0 enables the interrupt.
    pub fn write_irq(&mut self, value: u8) {
        self.irq_enabled = value & 0x01 != 0;
        if self.irq_enabled && !self.fifo.is_empty() {
            self.interrupt_pending = true;
        }
    }

    /// Accepts a command byte on the data or status/command port. A reset /
    /// identify command (0xA0-0xA2) queues the keyboard-ID reply the SYSROM
    /// expects; other commands (LED, key-repeat rate) are not modeled.
    pub fn write_command(&mut self, value: u8) {
        if (0xA0..=0xA2).contains(&value) {
            for byte in KEYBOARD_ID {
                self.push_byte(byte);
            }
            self.interrupt_pending = true;
        }
    }

    /// The current IRQ 1 line level (pending and enabled).
    pub fn irq_line(&self) -> bool {
        self.irq_enabled && self.interrupt_pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_event_expands_into_two_byte_packet() {
        let mut keyboard = TownsKeyboard::new();
        keyboard.write_irq(0x01);
        assert!(!keyboard.irq_line());

        // Press the 'A' key (JIS scancode 0x1E).
        keyboard.push_scancode(0x1E);
        assert!(keyboard.irq_line());
        assert_eq!(keyboard.read_data(), FLAG_JIS_PRESS);
        // Interrupt stays pending for the second byte.
        assert!(keyboard.irq_line());
        assert_eq!(keyboard.read_data(), 0x1E);
        assert!(!keyboard.irq_line());
    }

    #[test]
    fn release_sets_the_release_bit() {
        let mut keyboard = TownsKeyboard::new();
        keyboard.push_scancode(0x1E | 0x80);
        assert_eq!(keyboard.read_data(), FLAG_JIS_RELEASE);
        assert_eq!(keyboard.read_data(), 0x1E);
    }

    #[test]
    fn held_modifiers_set_flag_bits() {
        let mut keyboard = TownsKeyboard::new();
        keyboard.push_scancode(SCANCODE_SHIFT);
        // The Shift packet itself carries the Shift bit.
        assert_eq!(keyboard.read_data(), FLAG_JIS_PRESS | FLAG_SHIFT);
        assert_eq!(keyboard.read_data(), SCANCODE_SHIFT);
        // A subsequent key carries Shift while held.
        keyboard.push_scancode(0x1E);
        assert_eq!(keyboard.read_data(), FLAG_JIS_PRESS | FLAG_SHIFT);
        assert_eq!(keyboard.read_data(), 0x1E);
    }

    #[test]
    fn null_scancode_is_ignored() {
        let mut keyboard = TownsKeyboard::new();
        keyboard.push_scancode(0x00);
        assert_eq!(keyboard.read_status(), 0);
    }

    #[test]
    fn reset_command_queues_keyboard_id() {
        let mut keyboard = TownsKeyboard::new();
        keyboard.write_command(0xA0);
        assert_eq!(keyboard.read_data(), KEYBOARD_ID[0]);
        assert_eq!(keyboard.read_data(), KEYBOARD_ID[1]);
    }
}
