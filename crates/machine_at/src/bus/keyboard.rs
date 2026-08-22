//! Host key injection: set-1 key ids expanded into set-2 scancode sequences.
//!
//! The host forwards one byte per key event: a set-1 make code (or one of the
//! synthetic ids below for the E0-extended keys) with bit 7 set on release.
//! The bus expands it into the set-2 byte sequence the AT keyboard sends, so
//! the 8042's set-2-to-set-1 translation delivers what the BIOS expects.

use common::{
    TraceSink,
    input::{
        AT_KEY_CURSOR_DOWN, AT_KEY_CURSOR_LEFT, AT_KEY_CURSOR_RIGHT, AT_KEY_CURSOR_UP,
        AT_KEY_DELETE, AT_KEY_END, AT_KEY_HOME, AT_KEY_INSERT, AT_KEY_KEYPAD_DIVIDE,
        AT_KEY_KEYPAD_ENTER, AT_KEY_PAGE_DOWN, AT_KEY_PAGE_UP, AT_KEY_RIGHT_ALT, AT_KEY_RIGHT_CTRL,
    },
};
use device::i8042_kbc::SET2_TO_SET1;

use crate::bus::AtBus;

/// Release flag on a forwarded host key id.
const KEY_RELEASE_FLAG: u8 = 0x80;

/// Set-2 prefix marking an extended key.
const SET2_EXTENDED_PREFIX: u8 = 0xE0;
/// Set-2 prefix marking a key release.
const SET2_BREAK_PREFIX: u8 = 0xF0;

/// Inverse of the 8042 translation table: set-1 make code to set-2 make code.
/// Entries without a set-2 counterpart hold zero.
const SET2_FROM_SET1: [u8; 128] = build_set2_from_set1();

const fn build_set2_from_set1() -> [u8; 128] {
    let mut table = [0u8; 128];
    let mut set2 = 0;
    while set2 < SET2_TO_SET1.len() {
        let set1 = SET2_TO_SET1[set2];
        if set1 < 0x80 && table[set1 as usize] == 0 {
            table[set1 as usize] = set2 as u8;
        }
        set2 += 1;
    }
    table
}

/// The set-1 base code whose set-2 sequence an extended synthetic id borrows,
/// or `None` for an ordinary key id.
fn extended_base(id: u8) -> Option<u8> {
    match id {
        AT_KEY_CURSOR_UP => Some(0x48),
        AT_KEY_CURSOR_DOWN => Some(0x50),
        AT_KEY_CURSOR_LEFT => Some(0x4B),
        AT_KEY_CURSOR_RIGHT => Some(0x4D),
        AT_KEY_INSERT => Some(0x52),
        AT_KEY_DELETE => Some(0x53),
        AT_KEY_HOME => Some(0x47),
        AT_KEY_END => Some(0x4F),
        AT_KEY_PAGE_UP => Some(0x49),
        AT_KEY_PAGE_DOWN => Some(0x51),
        AT_KEY_KEYPAD_ENTER => Some(0x1C),
        AT_KEY_KEYPAD_DIVIDE => Some(0x35),
        AT_KEY_RIGHT_CTRL => Some(0x1D),
        AT_KEY_RIGHT_ALT => Some(0x38),
        _ => None,
    }
}

impl<T: TraceSink> AtBus<T> {
    /// Queues a host key event (set-1 id, bit 7 = release) as its set-2
    /// scancode sequence and schedules delivery.
    pub fn push_key_scancode(&mut self, code: u8) {
        let released = code & KEY_RELEASE_FLAG != 0;
        let id = code & !KEY_RELEASE_FLAG;
        let (extended, base) = match extended_base(id) {
            Some(base) => (true, base),
            None => (false, id),
        };
        let set2 = SET2_FROM_SET1[usize::from(base)];
        if set2 == 0 {
            return;
        }
        if extended {
            self.kbc.keyboard.push_scancode(SET2_EXTENDED_PREFIX);
        }
        if released {
            self.kbc.keyboard.push_scancode(SET2_BREAK_PREFIX);
        }
        self.kbc.keyboard.push_scancode(set2);
        self.schedule_kbc_deliver();
    }
}
