//! High-level emulation of the keyboard/cassette sub-controller.
//!
//! The real machine uses a micro-controller to scan the keyboard and signal
//! the main CPU through a vectored interrupt. We model the keyboard directly.
//!
//! Host events arrive as a single byte: bit 7 marks a key release, and the low
//! seven bits are a key id. Ids 0x00-0x5F are normal keycodes; ids 0x60-0x69
//! are function keys F1-F10, which the firmware expects as keycodes 0xF0-0xF9.
//! A single key is latched at a time; a release returns to the no-key state.

/// Vector for a normal-key change.
const NORMAL_KEY_VECTOR: u8 = 0x02;
/// Vector for a function-key change.
const FUNCTION_KEY_VECTOR: u8 = 0x14;
/// Release marker bit on a host event.
const RELEASE_FLAG: u8 = 0x80;
/// First function-key id on the wire.
const FUNCTION_KEY_ID_BASE: u8 = 0x60;
/// Number of function keys.
const FUNCTION_KEY_COUNT: u8 = 10;
/// First function keycode the firmware expects.
const FUNCTION_KEYCODE_BASE: u8 = 0xF0;

/// Translates a host key id (low seven bits) into a firmware keycode.
fn keycode_for(id: u8) -> u8 {
    if (FUNCTION_KEY_ID_BASE..FUNCTION_KEY_ID_BASE + FUNCTION_KEY_COUNT).contains(&id) {
        FUNCTION_KEYCODE_BASE + (id - FUNCTION_KEY_ID_BASE)
    } else {
        id
    }
}

/// Keyboard sub-controller state.
#[derive(Debug, Clone, Default)]
pub(crate) struct SubHle {
    current_keycode: u8,
    pending_keycode: u8,
    last_scanned: u8,
}

impl SubHle {
    /// Creates an idle sub-controller.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Records a host key event.
    pub(crate) fn push_scancode(&mut self, code: u8) {
        self.pending_keycode = if code & RELEASE_FLAG != 0 {
            0
        } else {
            keycode_for(code & !RELEASE_FLAG)
        };
    }

    /// Runs one keyboard scan. If the held key changed, latches the new code
    /// and returns the interrupt vector to raise.
    pub(crate) fn scan(&mut self) -> Option<u8> {
        if self.pending_keycode == self.last_scanned {
            return None;
        }
        self.last_scanned = self.pending_keycode;
        self.current_keycode = self.pending_keycode;
        let vector = if self.current_keycode & 0xF0 == 0xF0 {
            FUNCTION_KEY_VECTOR
        } else {
            NORMAL_KEY_VECTOR
        };
        Some(vector)
    }

    /// Latches a demodulated cassette byte into the shared port A latch. The
    /// keyboard transition state is left untouched: cassette and keyboard share
    /// the latch, and the host scans only while the tape is stopped.
    pub(crate) fn set_cassette_byte(&mut self, byte: u8) {
        self.current_keycode = byte;
    }

    /// The currently latched keycode (read through PPI port A).
    pub(crate) fn current_keycode(&self) -> u8 {
        self.current_keycode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_key_press_raises_vector_two() {
        let mut sub = SubHle::new();
        sub.push_scancode(0x41);
        assert_eq!(sub.scan(), Some(NORMAL_KEY_VECTOR));
        assert_eq!(sub.current_keycode(), 0x41);
        // No change on the next scan.
        assert_eq!(sub.scan(), None);
    }

    #[test]
    fn function_key_press_raises_vector_fourteen() {
        let mut sub = SubHle::new();
        sub.push_scancode(FUNCTION_KEY_ID_BASE);
        assert_eq!(sub.scan(), Some(FUNCTION_KEY_VECTOR));
        assert_eq!(sub.current_keycode(), 0xF0);
    }

    #[test]
    fn release_scans_back_to_no_key() {
        let mut sub = SubHle::new();
        sub.push_scancode(0x41);
        sub.scan();
        sub.push_scancode(0x41 | RELEASE_FLAG);
        assert_eq!(sub.scan(), Some(NORMAL_KEY_VECTOR));
        assert_eq!(sub.current_keycode(), 0x00);
    }
}
