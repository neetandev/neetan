//! Console device of DOS: the backends of CON output and keyboard input.
//!
//! `ConsoleOutput` takes the bytes that `INT 21h` and `INT 29h` write to the
//! console. `KeyboardInput` delivers the keys that the console input
//! functions read. The PC-98 backends are the text VRAM console and the
//! keyboard ring buffer of the BIOS data area. The guest backend sends the
//! work to the CON driver of a guest device chain.

use std::collections::VecDeque;

use crate::{MemoryAccess, guest_call::GuestStep, guest_character::GuestConsole};

/// Start of the keyboard ring buffer in the PC-98 BIOS data area.
const BDA_KEYBOARD_BUFFER_START: u32 = 0x0502;
/// End of the keyboard ring buffer (exclusive).
const BDA_KEYBOARD_BUFFER_END: u32 = 0x0522;
/// Word with the address of the next key to read.
const BDA_KEYBOARD_BUFFER_HEAD: u32 = 0x0524;
/// Word with the address of the next free slot.
const BDA_KEYBOARD_BUFFER_TAIL: u32 = 0x0526;
/// Byte with the number of waiting keys.
const BDA_KEYBOARD_BUFFER_COUNT: u32 = 0x0528;
/// Size of one key entry: the character byte and the scan code.
const BDA_KEYBOARD_ENTRY_SIZE: u32 = 2;

/// Sink of console output.
pub(crate) trait ConsoleOutput {
    /// Writes one byte to the console.
    fn write_byte(&mut self, memory: &mut dyn MemoryAccess, byte: u8);
}

/// One key from the keyboard backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KeyStroke {
    /// Scan code of the key. Backends that deliver bytes have none.
    pub(crate) scan_code: Option<u8>,
    /// Character byte. Zero marks an extended key that the scan code names.
    pub(crate) character: u8,
}

impl KeyStroke {
    /// Returns the scan code, zero for backends without scan codes.
    pub(crate) fn scan_code_or_zero(&self) -> u8 {
        self.scan_code.unwrap_or(0)
    }
}

/// Source of keyboard input.
pub(crate) trait KeyboardInput {
    /// Returns true when a key waits.
    fn key_available(&mut self, memory: &dyn MemoryAccess) -> bool;
    /// Takes the next key. Returns `None` when no key waits.
    fn read_key(&mut self, memory: &mut dyn MemoryAccess) -> Option<KeyStroke>;
    /// Discards every waiting key.
    fn flush(&mut self, memory: &mut dyn MemoryAccess);
    /// Returns true when `key_available` reflects the device. A guest device
    /// needs a status call in the current entry first.
    fn status_is_current(&self) -> bool;
}

/// Keyboard ring buffer of the PC-98 BIOS data area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BdaKeyboard;

impl KeyboardInput for BdaKeyboard {
    fn key_available(&mut self, memory: &dyn MemoryAccess) -> bool {
        memory.read_byte(BDA_KEYBOARD_BUFFER_COUNT) > 0
    }

    fn read_key(&mut self, memory: &mut dyn MemoryAccess) -> Option<KeyStroke> {
        if memory.read_byte(BDA_KEYBOARD_BUFFER_COUNT) == 0 {
            return None;
        }
        let head = u32::from(memory.read_word(BDA_KEYBOARD_BUFFER_HEAD));
        let character = memory.read_byte(head);
        let scan_code = memory.read_byte(head + 1);

        let mut new_head = head + BDA_KEYBOARD_ENTRY_SIZE;
        if new_head >= BDA_KEYBOARD_BUFFER_END {
            new_head = BDA_KEYBOARD_BUFFER_START;
        }
        memory.write_word(BDA_KEYBOARD_BUFFER_HEAD, new_head as u16);
        let count = memory.read_byte(BDA_KEYBOARD_BUFFER_COUNT);
        memory.write_byte(BDA_KEYBOARD_BUFFER_COUNT, count - 1);

        Some(KeyStroke {
            scan_code: Some(scan_code),
            character,
        })
    }

    fn flush(&mut self, memory: &mut dyn MemoryAccess) {
        memory.write_byte(BDA_KEYBOARD_BUFFER_COUNT, 0);
        let head = memory.read_word(BDA_KEYBOARD_BUFFER_HEAD);
        memory.write_word(BDA_KEYBOARD_BUFFER_TAIL, head);
    }

    fn status_is_current(&self) -> bool {
        true
    }
}

/// Backend that serves the DOS console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConsoleDevice {
    /// PC-98 text VRAM console and the BIOS data area keyboard buffer.
    TextVram(BdaKeyboard),
    /// CON driver of a guest device chain.
    GuestDriver(GuestConsole),
}

/// Save state tag of `ConsoleDevice::TextVram`.
const CONSOLE_TAG_TEXT_VRAM: u8 = 0;
/// Save state tag of `ConsoleDevice::GuestDriver`.
const CONSOLE_TAG_GUEST_DRIVER: u8 = 1;

impl save_state::StateEncode for ConsoleDevice {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::TextVram(_) => {
                save_state::StateEncode::encode_state(&CONSOLE_TAG_TEXT_VRAM, output)
            }
            Self::GuestDriver(console) => {
                save_state::StateEncode::encode_state(&CONSOLE_TAG_GUEST_DRIVER, output);
                save_state::StateEncode::encode_state(console, output);
            }
        }
    }
}

impl save_state::StateDecode for ConsoleDevice {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            CONSOLE_TAG_TEXT_VRAM => Ok(Self::TextVram(BdaKeyboard)),
            CONSOLE_TAG_GUEST_DRIVER => Ok(Self::GuestDriver(
                save_state::StateDecode::decode_state(decoder)?,
            )),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

impl ConsoleDevice {
    /// Returns the output backend. The text VRAM backend is `text_vram`.
    pub(crate) fn output<'a>(
        &'a mut self,
        text_vram: &'a mut crate::console::TextVramConsole,
    ) -> &'a mut dyn ConsoleOutput {
        match self {
            Self::TextVram(_) => text_vram,
            Self::GuestDriver(console) => console,
        }
    }

    /// Returns true for the PC-98 text VRAM console.
    pub(crate) fn is_text_vram(&self) -> bool {
        match self {
            Self::TextVram(_) => true,
            Self::GuestDriver(_) => false,
        }
    }

    /// Returns the keyboard backend.
    pub(crate) fn keyboard(&mut self) -> &mut dyn KeyboardInput {
        match self {
            Self::TextVram(keyboard) => keyboard,
            Self::GuestDriver(console) => console,
        }
    }

    /// Forgets the device status of the previous entry.
    pub(crate) fn begin_entry(&mut self) {
        match self {
            Self::TextVram(_) => {}
            Self::GuestDriver(console) => console.begin_entry(),
        }
    }

    /// Asks for new input before the entry runs again. Returns false for a
    /// backend that waits by a repeat of the guest `INT` instruction.
    pub(crate) fn request_input_wait(&mut self) -> bool {
        match self {
            Self::TextVram(_) => false,
            Self::GuestDriver(console) => {
                console.request_input_wait();
                true
            }
        }
    }

    /// Appends the queued output and flush work to `steps` and clears it.
    pub(crate) fn take_queued_steps(&mut self, steps: &mut VecDeque<GuestStep>) {
        match self {
            Self::TextVram(_) => {}
            Self::GuestDriver(console) => console.take_queued_steps(steps),
        }
    }

    /// Appends the queued idle call and status poll to `steps` and clears them.
    pub(crate) fn take_queued_poll(&mut self, steps: &mut VecDeque<GuestStep>) {
        match self {
            Self::TextVram(_) => {}
            Self::GuestDriver(console) => console.take_queued_poll(steps),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MockMemory;

    fn memory_with_keys(keys: &[(u8, u8)]) -> MockMemory {
        let mut memory = MockMemory::with_extended_memory(0x10000, 0);
        memory.write_word(BDA_KEYBOARD_BUFFER_HEAD, BDA_KEYBOARD_BUFFER_START as u16);
        let mut tail = BDA_KEYBOARD_BUFFER_START;
        for &(character, scan_code) in keys {
            memory.write_byte(tail, character);
            memory.write_byte(tail + 1, scan_code);
            tail += BDA_KEYBOARD_ENTRY_SIZE;
        }
        memory.write_word(BDA_KEYBOARD_BUFFER_TAIL, tail as u16);
        memory.write_byte(BDA_KEYBOARD_BUFFER_COUNT, keys.len() as u8);
        memory
    }

    #[test]
    fn bda_keyboard_reads_keys_in_order() {
        let mut memory = memory_with_keys(&[(b'A', 0x1D), (0x00, 0x62)]);
        let mut keyboard = BdaKeyboard;
        assert!(keyboard.key_available(&memory));
        assert_eq!(
            keyboard.read_key(&mut memory),
            Some(KeyStroke {
                scan_code: Some(0x1D),
                character: b'A'
            })
        );
        assert_eq!(
            keyboard.read_key(&mut memory),
            Some(KeyStroke {
                scan_code: Some(0x62),
                character: 0x00
            })
        );
        assert!(!keyboard.key_available(&memory));
        assert_eq!(keyboard.read_key(&mut memory), None);
        assert!(keyboard.status_is_current());
    }

    #[test]
    fn bda_keyboard_head_wraps_at_the_buffer_end() {
        let mut memory = memory_with_keys(&[]);
        let last = BDA_KEYBOARD_BUFFER_END - BDA_KEYBOARD_ENTRY_SIZE;
        memory.write_word(BDA_KEYBOARD_BUFFER_HEAD, last as u16);
        memory.write_byte(last, b'Z');
        memory.write_byte(BDA_KEYBOARD_BUFFER_COUNT, 1);
        let mut keyboard = BdaKeyboard;
        assert_eq!(keyboard.read_key(&mut memory).unwrap().character, b'Z');
        assert_eq!(
            u32::from(memory.read_word(BDA_KEYBOARD_BUFFER_HEAD)),
            BDA_KEYBOARD_BUFFER_START
        );
    }

    #[test]
    fn bda_keyboard_flush_empties_the_ring() {
        let mut memory = memory_with_keys(&[(b'A', 0x1D), (b'B', 0x2D)]);
        let mut keyboard = BdaKeyboard;
        keyboard.flush(&mut memory);
        assert!(!keyboard.key_available(&memory));
        assert_eq!(
            memory.read_word(BDA_KEYBOARD_BUFFER_TAIL),
            memory.read_word(BDA_KEYBOARD_BUFFER_HEAD)
        );
    }

    #[test]
    fn text_vram_device_waits_by_repeating_the_instruction() {
        let mut device = ConsoleDevice::TextVram(BdaKeyboard);
        assert!(!device.request_input_wait());
        let mut steps = VecDeque::new();
        device.take_queued_steps(&mut steps);
        device.take_queued_poll(&mut steps);
        assert!(steps.is_empty());
    }
}
