//! Behavioral 8042 keyboard controller and AT keyboard for the PC/AT.
//!
//! The real controller is a UPI-41 microcontroller running firmware; this is
//! a behavioral model of its host-visible protocol (status/data/command ports
//! 0x60/0x64, the command byte, the output port with the A20 and CPU-reset
//! lines, the controller self-test, and set-2-to-set-1 scancode translation)
//! plus an AT keyboard that answers the standard keyboard command set. The
//! output buffer is filled one byte per scheduled delivery so the BIOS sees
//! the input-buffer-full flag drop before the output-buffer-full flag rises.

use std::collections::VecDeque;

/// Status register: output buffer full.
const STATUS_OBF: u8 = 0x01;
/// Status register: system flag (set after a successful self-test).
const STATUS_SYS: u8 = 0x04;
/// Status register: command/data flag (last write was a command).
const STATUS_CMD: u8 = 0x08;
/// Status register: keyboard not inhibited.
const STATUS_INH: u8 = 0x10;

/// Command byte: keyboard interrupt (IRQ1) enable.
const CMD_BYTE_IRQ1: u8 = 0x01;
/// Command byte: system flag mirror.
const CMD_BYTE_SYS: u8 = 0x04;
/// Command byte: keyboard interface disable.
const CMD_BYTE_KBD_DISABLE: u8 = 0x10;
/// Command byte: scancode translation enable.
const CMD_BYTE_TRANSLATE: u8 = 0x40;

/// Output port: CPU reset line (active low: 0 asserts reset).
const OUTPUT_PORT_RESET: u8 = 0x01;
/// Output port: Gate A20 line.
const OUTPUT_PORT_A20: u8 = 0x02;
/// Output port power-on value: reset deasserted, A20 enabled.
const OUTPUT_PORT_RESET_VALUE: u8 = OUTPUT_PORT_RESET | OUTPUT_PORT_A20;

/// Keyboard acknowledge byte.
const KBD_ACK: u8 = 0xFA;
/// Keyboard basic-assurance-test pass byte.
const KBD_BAT_OK: u8 = 0xAA;

/// Set-2 break-code prefix.
const SET2_BREAK_PREFIX: u8 = 0xF0;

save_state::runtime_state! {
/// Authoritative state of the attached AT keyboard.
#[derive(Debug, Clone)]
pub struct AtKeyboardState {
    output: VecDeque<u8>,
    enabled: bool,
    expect_parameter_for: Option<u8>,
    leds: u8,
    typematic: u8,
}}

save_state::runtime_state! {
/// Authoritative 8042 controller and keyboard state.
#[derive(Debug, Clone)]
pub struct I8042KbcState {
    status: u8,
    output_buffer: u8,
    command_byte: u8,
    ram: [u8; 32],
    output_port: u8,
    input_port: u8,
    pending_command: Option<u8>,
    response_queue: VecDeque<(u8, bool)>,
    keyboard: AtKeyboardState,
}}

/// Effects of an 8042 access, for the bus to apply.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KbcEffects {
    /// The output port changed; the bus should refresh Gate A20.
    pub output_port_changed: bool,
    /// A CPU reset pulse was requested via the output port.
    pub reset_pulse: bool,
    /// Output is pending; the bus should schedule a delivery event.
    pub schedule_delivery: bool,
}

/// A behavioral AT keyboard answering the standard keyboard command set.
pub struct AtKeyboard {
    /// Bytes the keyboard is sending to the controller (set-2 scancodes and
    /// command responses), consumed by the controller in order.
    pub output: VecDeque<u8>,
    /// Whether scanning is enabled (keyboard `0xF4`/`0xF5`).
    pub enabled: bool,
    /// A keyboard command awaiting its parameter byte (`0xED`/`0xF3`).
    pub expect_parameter_for: Option<u8>,
    /// LED state set by the `0xED` command.
    pub leds: u8,
    /// Typematic configuration set by the `0xF3` command.
    pub typematic: u8,
}

impl Default for AtKeyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl AtKeyboard {
    /// Creates a keyboard in its power-on state.
    pub fn new() -> Self {
        Self {
            output: VecDeque::new(),
            enabled: true,
            expect_parameter_for: None,
            leds: 0,
            typematic: 0,
        }
    }

    fn capture_state(&self) -> AtKeyboardState {
        AtKeyboardState {
            output: self.output.clone(),
            enabled: self.enabled,
            expect_parameter_for: self.expect_parameter_for,
            leds: self.leds,
            typematic: self.typematic,
        }
    }

    fn restore_state(&mut self, state: AtKeyboardState) {
        self.output = state.output;
        self.enabled = state.enabled;
        self.expect_parameter_for = state.expect_parameter_for;
        self.leds = state.leds;
        self.typematic = state.typematic;
    }

    /// Queues a raw set-2 scancode byte from the host.
    pub fn push_scancode(&mut self, byte: u8) {
        self.output.push_back(byte);
    }

    /// Processes a data byte the controller forwarded to the keyboard.
    pub fn write_data(&mut self, data: u8) {
        if let Some(command) = self.expect_parameter_for.take() {
            match command {
                0xED => self.leds = data & 0x07,
                0xF3 => self.typematic = data,
                _ => {}
            }
            self.output.push_back(KBD_ACK);
            return;
        }

        match data {
            0xFF => {
                // Reset: acknowledge, then pass the basic assurance test.
                self.output.clear();
                self.enabled = true;
                self.output.push_back(KBD_ACK);
                self.output.push_back(KBD_BAT_OK);
            }
            0xEE => self.output.push_back(0xEE), // echo
            0xF2 => {
                // Identify: ACK then the two-byte MF2 keyboard ID.
                self.output.push_back(KBD_ACK);
                self.output.push_back(0xAB);
                self.output.push_back(0x83);
            }
            0xED | 0xF3 => {
                self.expect_parameter_for = Some(data);
                self.output.push_back(KBD_ACK);
            }
            0xF4 => {
                self.enabled = true;
                self.output.push_back(KBD_ACK);
            }
            0xF5 | 0xF6 => {
                self.enabled = data == 0xF6;
                self.output.push_back(KBD_ACK);
            }
            _ => self.output.push_back(KBD_ACK),
        }
    }
}

/// A behavioral 8042 keyboard controller.
pub struct I8042Kbc {
    /// Status register (ports 0x64 read).
    pub status: u8,
    /// Output buffer (port 0x60 read).
    pub output_buffer: u8,
    /// Command byte (controller RAM byte 0).
    pub command_byte: u8,
    /// Controller RAM (32 bytes).
    pub ram: [u8; 32],
    /// Output port (bit 0 CPU reset, bit 1 Gate A20).
    pub output_port: u8,
    /// Input port value returned by command 0xC0.
    pub input_port: u8,
    /// A controller command awaiting its data byte (0x60/0xD1/0xD2 and mouse).
    pub pending_command: Option<u8>,
    /// Controller-originated responses awaiting delivery. The flag records
    /// whether the byte should behave as a keyboard byte (IRQ1-eligible).
    pub response_queue: VecDeque<(u8, bool)>,
    /// The attached keyboard.
    pub keyboard: AtKeyboard,
}

impl Default for I8042Kbc {
    fn default() -> Self {
        Self::new()
    }
}

impl I8042Kbc {
    /// Creates an 8042 in its power-on state.
    pub fn new() -> Self {
        Self {
            status: STATUS_INH,
            output_buffer: 0,
            command_byte: 0,
            ram: [0; 32],
            output_port: OUTPUT_PORT_RESET_VALUE,
            input_port: 0xBF, // AT jumper defaults, color display, key lock off
            pending_command: None,
            response_queue: VecDeque::new(),
            keyboard: AtKeyboard::new(),
        }
    }

    /// Captures the controller, keyboard, and pending output queues.
    pub fn capture_state(&self) -> I8042KbcState {
        I8042KbcState {
            status: self.status,
            output_buffer: self.output_buffer,
            command_byte: self.command_byte,
            ram: self.ram,
            output_port: self.output_port,
            input_port: self.input_port,
            pending_command: self.pending_command,
            response_queue: self.response_queue.clone(),
            keyboard: self.keyboard.capture_state(),
        }
    }

    /// Restores the controller, keyboard, and pending output queues.
    pub fn restore_state(
        &mut self,
        state: I8042KbcState,
    ) -> Result<(), save_state::StateValidationError> {
        if state
            .pending_command
            .is_some_and(|command| !matches!(command, 0x60..=0x7F | 0xD1..=0xD4))
        {
            return Err(save_state::StateValidationError::new(
                "8042 pending command is invalid",
            ));
        }
        self.status = state.status;
        self.output_buffer = state.output_buffer;
        self.command_byte = state.command_byte;
        self.ram = state.ram;
        self.output_port = state.output_port;
        self.input_port = state.input_port;
        self.pending_command = state.pending_command;
        self.response_queue = state.response_queue;
        self.keyboard.restore_state(state.keyboard);
        Ok(())
    }

    /// Reads the status register (port 0x64).
    pub fn read_status(&self) -> u8 {
        self.status
    }

    /// Reads the output buffer (port 0x60), clearing the output-buffer-full
    /// flag. Requests another delivery when more output is pending.
    pub fn read_data(&mut self) -> (u8, KbcEffects) {
        self.status &= !STATUS_OBF;
        let effects = KbcEffects {
            schedule_delivery: self.has_pending_output(),
            ..KbcEffects::default()
        };
        (self.output_buffer, effects)
    }

    /// Writes a controller command (port 0x64).
    pub fn write_command(&mut self, command: u8) -> KbcEffects {
        self.status |= STATUS_CMD;
        let mut effects = KbcEffects::default();

        match command {
            0x20..=0x3F => {
                let value = self.ram[(command & 0x1F) as usize];
                self.response_queue.push_back((value, false));
            }
            0x60..=0x7F => self.pending_command = Some(command),
            0xA4 => self.response_queue.push_back((0xF1, false)), // password not installed
            0xA5 | 0xA6 | 0xA7 | 0xA8 | 0xAC => {}                // password/aux/dump: no-op
            0xA9 => self.response_queue.push_back((0x02, false)), // aux test: no device
            0xAA => {
                self.status |= STATUS_SYS;
                self.command_byte |= CMD_BYTE_SYS;
                self.response_queue.push_back((0x55, false));
            }
            0xAB => self.response_queue.push_back((0x00, false)), // keyboard interface test ok
            0xAD => self.command_byte |= CMD_BYTE_KBD_DISABLE,
            0xAE => self.command_byte &= !CMD_BYTE_KBD_DISABLE,
            0xC0 => self.response_queue.push_back((self.input_port, false)),
            0xD0 => self.response_queue.push_back((self.output_port, false)),
            0xD1..=0xD4 => self.pending_command = Some(command),
            0xE0 => self.response_queue.push_back((0x00, false)),
            0xF0..=0xFF => {
                // Pulse output port: a low bit in the low nibble pulses that
                // line. Bit 0 is the CPU reset.
                effects.reset_pulse = command & OUTPUT_PORT_RESET == 0;
            }
            _ => {}
        }

        effects.schedule_delivery = self.has_pending_output();
        effects
    }

    /// Writes a data byte (port 0x60).
    pub fn write_data(&mut self, data: u8) -> KbcEffects {
        self.status &= !STATUS_CMD;
        let mut effects = KbcEffects::default();

        if let Some(command) = self.pending_command.take() {
            match command {
                0x60..=0x7F => {
                    self.ram[(command & 0x1F) as usize] = data;
                    if command == 0x60 {
                        self.command_byte = data;
                    }
                }
                0xD1 => {
                    let previous = self.output_port;
                    self.output_port = data;
                    effects.output_port_changed = true;
                    if previous & OUTPUT_PORT_RESET != 0 && data & OUTPUT_PORT_RESET == 0 {
                        effects.reset_pulse = true;
                    }
                }
                0xD2 => self.response_queue.push_back((data, true)),
                0xD3 | 0xD4 => {} // no auxiliary (mouse) device on this board
                _ => {}
            }
        } else {
            self.keyboard.write_data(data);
        }

        effects.schedule_delivery = self.has_pending_output();
        effects
    }

    /// Returns whether any output is waiting to be delivered.
    pub fn has_pending_output(&self) -> bool {
        !self.response_queue.is_empty()
            || (self.keyboard_enabled() && !self.keyboard.output.is_empty())
    }

    /// Returns whether the keyboard interface is enabled.
    fn keyboard_enabled(&self) -> bool {
        self.command_byte & CMD_BYTE_KBD_DISABLE == 0
    }

    /// Loads the next pending byte into the output buffer, if the buffer is
    /// free. Returns `Some(raise_irq1)` when a byte was delivered.
    pub fn deliver_next(&mut self) -> Option<bool> {
        if self.status & STATUS_OBF != 0 {
            return None;
        }

        // Controller responses take priority over keyboard scancodes.
        if let Some((byte, from_keyboard)) = self.response_queue.pop_front() {
            self.output_buffer = byte;
            self.status |= STATUS_OBF;
            return Some(from_keyboard && self.command_byte & CMD_BYTE_IRQ1 != 0);
        }

        if self.keyboard_enabled()
            && let Some(byte) = self.next_keyboard_byte()
        {
            self.output_buffer = byte;
            self.status |= STATUS_OBF;
            return Some(self.command_byte & CMD_BYTE_IRQ1 != 0);
        }

        None
    }

    /// Pops the next keyboard byte, applying set-2-to-set-1 translation when
    /// the command byte enables it (folding a break prefix into the high bit).
    fn next_keyboard_byte(&mut self) -> Option<u8> {
        if self.command_byte & CMD_BYTE_TRANSLATE == 0 {
            return self.keyboard.output.pop_front();
        }

        let front = *self.keyboard.output.front()?;
        if front == SET2_BREAK_PREFIX {
            if self.keyboard.output.len() < 2 {
                return None; // wait for the code byte to arrive
            }
            self.keyboard.output.pop_front();
            let code = self.keyboard.output.pop_front()?;
            Some(translate_set2(code) | 0x80)
        } else {
            let code = self.keyboard.output.pop_front()?;
            Some(translate_set2(code))
        }
    }

    /// Returns the Gate A20 level from the output port.
    pub fn a20_enabled(&self) -> bool {
        self.output_port & OUTPUT_PORT_A20 != 0
    }
}

/// Translates a set-2 make code to its set-1 equivalent.
///
/// Codes 0x00-0x7F use the standard AT translation table; the sole set-2 make
/// code above that range is `0x83` (F7). Any other high code passes through.
fn translate_set2(code: u8) -> u8 {
    match code {
        0x83 => 0x41, // F7
        _ if (code as usize) < SET2_TO_SET1.len() => SET2_TO_SET1[code as usize],
        _ => code,
    }
}

/// Standard 8042 set-2-to-set-1 translation table for make codes 0x00-0x7F,
/// matching the documented keyboard controller mask ROM table.
pub const SET2_TO_SET1: [u8; 128] = [
    0xFF, 0x43, 0x41, 0x3F, 0x3D, 0x3B, 0x3C, 0x58, 0x64, 0x44, 0x42, 0x40, 0x3E, 0x0F, 0x29, 0x59,
    0x65, 0x38, 0x2A, 0x70, 0x1D, 0x10, 0x02, 0x5A, 0x66, 0x71, 0x2C, 0x1F, 0x1E, 0x11, 0x03, 0x5B,
    0x67, 0x2E, 0x2D, 0x20, 0x12, 0x05, 0x04, 0x5C, 0x68, 0x39, 0x2F, 0x21, 0x14, 0x13, 0x06, 0x5D,
    0x69, 0x31, 0x30, 0x23, 0x22, 0x15, 0x07, 0x5E, 0x6A, 0x72, 0x32, 0x24, 0x16, 0x08, 0x09, 0x5F,
    0x6B, 0x33, 0x25, 0x17, 0x18, 0x0B, 0x0A, 0x60, 0x6C, 0x34, 0x35, 0x26, 0x27, 0x19, 0x0C, 0x61,
    0x6D, 0x73, 0x28, 0x74, 0x1A, 0x0D, 0x62, 0x6E, 0x3A, 0x36, 0x1C, 0x1B, 0x75, 0x2B, 0x63, 0x76,
    0x55, 0x56, 0x77, 0x78, 0x79, 0x7A, 0x0E, 0x7B, 0x7C, 0x4F, 0x7D, 0x4B, 0x47, 0x7E, 0x7F, 0x6F,
    0x52, 0x53, 0x50, 0x4C, 0x4D, 0x48, 0x01, 0x45, 0x57, 0x4E, 0x51, 0x4A, 0x37, 0x49, 0x46, 0x54,
];
