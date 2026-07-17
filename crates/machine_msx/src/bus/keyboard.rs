//! MSX keyboard matrix.

use crate::MsxKeyboardLayout;

/// Number of rows in the MSX keyboard matrix.
const KEYBOARD_ROW_COUNT: usize = 11;

save_state::runtime_state! {
/// Mutable MSX keyboard matrix state.
#[derive(Clone)]
pub(crate) struct MsxKeyboardState {
    rows: [u8; KEYBOARD_ROW_COUNT],
}}

pub(crate) struct MsxKeyboard {
    rows: [u8; KEYBOARD_ROW_COUNT],
    layout: MsxKeyboardLayout,
}

impl MsxKeyboard {
    pub(crate) fn new(layout: MsxKeyboardLayout) -> Self {
        Self {
            rows: [0xFF; KEYBOARD_ROW_COUNT],
            layout,
        }
    }

    pub(crate) fn push_scancode(&mut self, code: u8) {
        let released = code & 0x80 != 0;
        let code = code & 0x7F;
        let Some((row, bit)) = self.matrix_position(code) else {
            return;
        };
        if released {
            self.rows[row] |= 1 << bit;
        } else {
            self.rows[row] &= !(1 << bit);
        }
    }

    pub(crate) fn row(&self, row: u8) -> u8 {
        self.rows.get(usize::from(row)).copied().unwrap_or(0xFF)
    }

    /// Captures every keyboard matrix row.
    pub(crate) const fn capture_state(&self) -> MsxKeyboardState {
        MsxKeyboardState { rows: self.rows }
    }

    /// Restores every keyboard matrix row.
    pub(crate) fn restore_state(&mut self, state: MsxKeyboardState) {
        self.rows = state.rows;
    }

    fn matrix_position(&self, code: u8) -> Option<(usize, u8)> {
        let position = match code {
            0x0A => (0, 0),
            0x01..=0x07 => (0, code),
            0x08 => (1, 0),
            0x09 => (1, 1),
            0x0B => (1, 2),
            0x0C => (1, 3),
            0x0D => (1, 4),
            0x1A => (1, 5),
            0x1B => (1, 6),
            0x26 => (1, 7),
            0x27 => (2, 0),
            0x28 => (2, 1),
            0x30 => (2, 2),
            0x31 => (2, 3),
            0x32 => (2, 4),
            0x33 => (2, 5),
            0x1D => (2, 6),
            0x2D => (2, 7),
            0x2B => (3, 0),
            0x1F => (3, 1),
            0x12 => (3, 2),
            0x20 => (3, 3),
            0x21 => (3, 4),
            0x22 => (3, 5),
            0x17 => (3, 6),
            0x23 => (3, 7),
            0x24 => (4, 0),
            0x25 => (4, 1),
            0x2F => (4, 2),
            0x2E => (4, 3),
            0x18 => (4, 4),
            0x19 => (4, 5),
            0x10 => (4, 6),
            0x13 => (4, 7),
            0x1E => (5, 0),
            0x14 => (5, 1),
            0x16 => (5, 2),
            0x2C => (5, 3),
            0x11 => (5, 4),
            0x2A => (5, 5),
            0x15 => (5, 6),
            0x29 => (5, 7),
            0x70 => (6, 0),
            0x74 => (6, 1),
            0x73 => (6, 2),
            0x71 => (6, 3),
            0x72 => (6, 4),
            0x62..=0x64 => (6, code - 0x5D),
            0x65..=0x66 => (7, code - 0x65),
            0x00 => (7, 2),
            0x0F => (7, 3),
            0x60 => (7, 4),
            0x0E => (7, 5),
            0x3F => (7, 6),
            0x1C => (7, 7),
            0x34 => (8, 0),
            0x3E => (8, 1),
            0x38 => (8, 2),
            0x39 => (8, 3),
            0x3B => (8, 4),
            0x3A => (8, 5),
            0x3D => (8, 6),
            0x3C => (8, 7),
            0x4E => (9, 3),
            0x4A => (9, 4),
            0x4B => (9, 5),
            0x4C => (9, 6),
            0x46 => (9, 7),
            0x47 => (10, 0),
            0x48 => (10, 1),
            0x42 => (10, 2),
            0x43 => (10, 3),
            0x44 => (10, 4),
            0x40 => (10, 5),
            0x4F => (10, 6),
            0x50 => (10, 7),
            _ => return None,
        };
        if position.0 >= 9 && matches!(self.layout, MsxKeyboardLayout::JapaneseAnsi) {
            None
        } else {
            Some(position)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keycodes by matrix row and bit from the Sony keyboard matrix.
    const MATRIX_KEYCODES: [[Option<u8>; 8]; KEYBOARD_ROW_COUNT] = [
        [
            Some(0x0A),
            Some(0x01),
            Some(0x02),
            Some(0x03),
            Some(0x04),
            Some(0x05),
            Some(0x06),
            Some(0x07),
        ],
        [
            Some(0x08),
            Some(0x09),
            Some(0x0B),
            Some(0x0C),
            Some(0x0D),
            Some(0x1A),
            Some(0x1B),
            Some(0x26),
        ],
        [
            Some(0x27),
            Some(0x28),
            Some(0x30),
            Some(0x31),
            Some(0x32),
            Some(0x33),
            Some(0x1D),
            Some(0x2D),
        ],
        [
            Some(0x2B),
            Some(0x1F),
            Some(0x12),
            Some(0x20),
            Some(0x21),
            Some(0x22),
            Some(0x17),
            Some(0x23),
        ],
        [
            Some(0x24),
            Some(0x25),
            Some(0x2F),
            Some(0x2E),
            Some(0x18),
            Some(0x19),
            Some(0x10),
            Some(0x13),
        ],
        [
            Some(0x1E),
            Some(0x14),
            Some(0x16),
            Some(0x2C),
            Some(0x11),
            Some(0x2A),
            Some(0x15),
            Some(0x29),
        ],
        [
            Some(0x70),
            Some(0x74),
            Some(0x73),
            Some(0x71),
            Some(0x72),
            Some(0x62),
            Some(0x63),
            Some(0x64),
        ],
        [
            Some(0x65),
            Some(0x66),
            Some(0x00),
            Some(0x0F),
            Some(0x60),
            Some(0x0E),
            Some(0x3F),
            Some(0x1C),
        ],
        [
            Some(0x34),
            Some(0x3E),
            Some(0x38),
            Some(0x39),
            Some(0x3B),
            Some(0x3A),
            Some(0x3D),
            Some(0x3C),
        ],
        [
            None,
            None,
            None,
            Some(0x4E),
            Some(0x4A),
            Some(0x4B),
            Some(0x4C),
            Some(0x46),
        ],
        [
            Some(0x47),
            Some(0x48),
            Some(0x42),
            Some(0x43),
            Some(0x44),
            Some(0x40),
            Some(0x4F),
            Some(0x50),
        ],
    ];

    #[test]
    fn key_presses_are_active_low() {
        let mut keyboard = MsxKeyboard::new(MsxKeyboardLayout::JapaneseJis);
        keyboard.push_scancode(0x1D);
        assert_eq!(keyboard.row(2), 0xBF);
        keyboard.push_scancode(0x9D);
        assert_eq!(keyboard.row(2), 0xFF);
    }

    #[test]
    fn every_keycode_matches_the_reference_matrix() {
        for (expected_row, row) in MATRIX_KEYCODES.iter().enumerate() {
            for (expected_bit, keycode) in row.iter().enumerate() {
                let Some(keycode) = keycode else {
                    continue;
                };
                let mut keyboard = MsxKeyboard::new(MsxKeyboardLayout::JapaneseJis);
                keyboard.push_scancode(*keycode);

                for actual_row in 0..KEYBOARD_ROW_COUNT {
                    let expected = if actual_row == expected_row {
                        !(1 << expected_bit)
                    } else {
                        0xFF
                    };
                    assert_eq!(
                        keyboard.row(actual_row as u8),
                        expected,
                        "keycode {keycode:#04x}, row {actual_row}"
                    );
                }
            }
        }
    }

    #[test]
    fn ansi_layout_ignores_keypad_scancodes() {
        let mut keyboard = MsxKeyboard::new(MsxKeyboardLayout::JapaneseAnsi);
        keyboard.push_scancode(0x4E);
        assert_eq!(keyboard.row(9), 0xFF);
    }
}
