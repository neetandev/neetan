//! PC-88VA2 HLE keyboard.
//!
//! The real VA keyboard is an undumped serial MCU. The interface sub-CPU exposes
//! two ways to read it (technical manual 5.6): the always-on 88-compatible scan
//! matrix (rows read at ports 0x00-0x0E) and the keycode FIFO (read at port 0x1C1,
//! raising master IRQ1) used by V3-mode software such as the DOS shell.
//!
//! The host reports a VA keycode (the value the keycode interface returns) with
//! bit 7 set for a release. That keycode is queued for the FIFO as-is, and is also
//! translated to its cell in the 88-compatible matrix so both interfaces stay
//! consistent.

use std::collections::VecDeque;

/// Number of 88-compatible matrix rows, read at ports 0x00-0x0E.
const MATRIX_ROWS: usize = 16;

save_state::runtime_state! {
/// HLE keyboard: the always-on 88-compatible scan matrix plus the keycode FIFO.
#[derive(Clone)]
pub struct KeyboardVa {
    fifo: VecDeque<u8>,
    /// Active-low key matrix, read row by row at ports 0x00-0x0E. A pressed key
    /// clears its column bit; an idle matrix reads all ones.
    matrix: [u8; MATRIX_ROWS],
}}

impl Default for KeyboardVa {
    fn default() -> Self {
        Self {
            fifo: VecDeque::new(),
            matrix: [0xFF; MATRIX_ROWS],
        }
    }
}

impl KeyboardVa {
    /// Queues a scancode reported by the host.
    pub fn push_scancode(&mut self, code: u8) {
        let pressed = code & 0x80 == 0;
        let keycode = code & 0x7F;
        if let Some((row, column)) = va_keycode_matrix_cell(keycode) {
            self.set_key(row, column, pressed);
        }
        self.fifo.push_back(code);
    }

    /// Pops the next scancode (0x00 when empty).
    pub fn read_data(&mut self) -> u8 {
        self.fifo.pop_front().unwrap_or(0x00)
    }

    /// Whether a scancode is waiting (drives the keyboard IRQ).
    pub fn has_data(&self) -> bool {
        !self.fifo.is_empty()
    }

    /// Sets a matrix key. The matrix is active low, so a pressed key clears its
    /// column bit in the row read at ports 0x00-0x0E.
    pub fn set_key(&mut self, row: usize, column: usize, pressed: bool) {
        if row >= self.matrix.len() || column >= 8 {
            return;
        }
        let mask = 1u8 << column;
        if pressed {
            self.matrix[row] &= !mask;
        } else {
            self.matrix[row] |= mask;
        }
    }

    /// Reads a matrix row (ports 0x00-0x0E); unknown rows read all ones.
    pub fn read_row(&self, row: usize) -> u8 {
        self.matrix.get(row).copied().unwrap_or(0xFF)
    }
}

/// Translates a VA keycode (the value the keycode interface returns at port 0x1C1)
/// into its `(row, column)` cell in the 88-compatible scan matrix. Returns `None`
/// for VA-only keys that have no cell in that matrix. The matrix layout follows the
/// PC-8801 keyboard, extended with rows 13-14 for the VA-specific keys.
const fn va_keycode_matrix_cell(keycode: u8) -> Option<(usize, usize)> {
    Some(match keycode {
        0x00 => (9, 7),  // ESC
        0x01 => (6, 1),  // 1
        0x02 => (6, 2),  // 2
        0x03 => (6, 3),  // 3
        0x04 => (6, 4),  // 4
        0x05 => (6, 5),  // 5
        0x06 => (6, 6),  // 6
        0x07 => (6, 7),  // 7
        0x08 => (7, 0),  // 8
        0x09 => (7, 1),  // 9
        0x0A => (6, 0),  // 0
        0x0B => (5, 7),  // -
        0x0C => (5, 6),  // ^
        0x0D => (5, 4),  // YEN
        0x0E => (8, 3),  // BS (shared cell with INS/DEL)
        0x0F => (10, 0), // TAB
        0x10 => (4, 1),  // Q
        0x11 => (4, 7),  // W
        0x12 => (2, 5),  // E
        0x13 => (4, 2),  // R
        0x14 => (4, 4),  // T
        0x15 => (5, 1),  // Y
        0x16 => (4, 5),  // U
        0x17 => (3, 1),  // I
        0x18 => (3, 7),  // O
        0x19 => (4, 0),  // P
        0x1A => (2, 0),  // @
        0x1B => (5, 3),  // [
        0x1C => (1, 7),  // RETURN
        0x1D => (2, 1),  // A
        0x1E => (4, 3),  // S
        0x1F => (2, 4),  // D
        0x20 => (2, 6),  // F
        0x21 => (2, 7),  // G
        0x22 => (3, 0),  // H
        0x23 => (3, 2),  // J
        0x24 => (3, 3),  // K
        0x25 => (3, 4),  // L
        0x26 => (7, 3),  // ;
        0x27 => (7, 2),  // :
        0x28 => (5, 5),  // ]
        0x29 => (5, 2),  // Z
        0x2A => (5, 0),  // X
        0x2B => (2, 3),  // C
        0x2C => (4, 6),  // V
        0x2D => (2, 2),  // B
        0x2E => (3, 6),  // N
        0x2F => (3, 5),  // M
        0x30 => (7, 4),  // ,
        0x31 => (7, 5),  // .
        0x32 => (7, 6),  // /
        0x33 => (7, 7),  // _
        0x34 => (9, 6),  // SPACE
        0x35 => (13, 5), // 変換 (henkan)
        0x36 => (11, 0), // ROLL UP
        0x37 => (11, 1), // ROLL DOWN
        0x38 => (8, 3),  // INS (shared cell with BS/DEL)
        0x39 => (8, 3),  // DEL (shared cell with BS/INS)
        0x3A => (8, 1),  // UP
        0x3B => (10, 2), // LEFT
        0x3C => (8, 2),  // RIGHT
        0x3D => (10, 1), // DOWN
        0x3E => (8, 0),  // HOME / CLR
        0x3F => (10, 3), // HELP
        0x40 => (10, 5), // KP -
        0x41 => (10, 6), // KP /
        0x42 => (0, 7),  // KP 7
        0x43 => (1, 0),  // KP 8
        0x44 => (1, 1),  // KP 9
        0x45 => (1, 2),  // KP *
        0x46 => (0, 4),  // KP 4
        0x47 => (0, 5),  // KP 5
        0x48 => (0, 6),  // KP 6
        0x49 => (1, 3),  // KP +
        0x4A => (0, 1),  // KP 1
        0x4B => (0, 2),  // KP 2
        0x4C => (0, 3),  // KP 3
        0x4D => (1, 4),  // KP =
        0x4E => (0, 0),  // KP 0
        0x4F => (1, 5),  // KP ,
        0x50 => (1, 6),  // KP .
        0x51 => (13, 6), // 決定 (kettei)
        0x79 => (1, 7),  // KP ENTER (shares RETURN's cell)
        0x60 => (9, 0),  // STOP
        0x61 => (10, 4), // COPY
        0x62 => (9, 1),  // F1
        0x63 => (9, 2),  // F2
        0x64 => (9, 3),  // F3
        0x65 => (9, 4),  // F4
        0x66 => (9, 5),  // F5
        0x67 => (13, 0), // F6
        0x68 => (13, 1), // F7
        0x69 => (13, 2), // F8 (the boot setup key, read at port 0x0D bit 2)
        0x6A => (13, 3), // F9
        0x6B => (13, 4), // F10
        0x70 => (8, 6),  // SHIFT
        0x71 => (10, 7), // CAPS
        0x72 => (8, 5),  // KANA
        0x73 => (8, 4),  // GRPH
        0x74 => (8, 7),  // CTRL
        0x7A => (14, 5), // PC
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{KeyboardVa, va_keycode_matrix_cell};

    #[test]
    fn fifo_pops_in_order_and_reports_pending() {
        let mut keyboard = KeyboardVa::default();
        assert!(!keyboard.has_data());
        keyboard.push_scancode(0x1D);
        keyboard.push_scancode(0x9D);
        assert!(keyboard.has_data());
        assert_eq!(keyboard.read_data(), 0x1D);
        assert_eq!(keyboard.read_data(), 0x9D);
        assert!(!keyboard.has_data());
        assert_eq!(keyboard.read_data(), 0x00);
    }

    #[test]
    fn matrix_cell_is_active_low() {
        let mut keyboard = KeyboardVa::default();
        // 'A' is keycode 0x1D, matrix cell (row 2, column 1).
        let (row, column) = va_keycode_matrix_cell(0x1D).expect("A has a matrix cell");
        assert_eq!((row, column), (2, 1));
        keyboard.set_key(row, column, true);
        assert_eq!(keyboard.read_row(row), !(1u8 << column));
        keyboard.set_key(row, column, false);
        assert_eq!(keyboard.read_row(row), 0xFF);
    }

    #[test]
    fn backspace_and_delete_keep_distinct_keycodes_but_share_a_matrix_cell() {
        // The matrix can only carry one bit for the combined "Del Ins" key, but the
        // keycode FIFO must distinguish them: BS is 0x0E, INS 0x38, DEL 0x39.
        assert_eq!(va_keycode_matrix_cell(0x0E), Some((8, 3)));
        assert_eq!(va_keycode_matrix_cell(0x38), Some((8, 3)));
        assert_eq!(va_keycode_matrix_cell(0x39), Some((8, 3)));
    }

    #[test]
    fn matrix_cells_are_unique_per_physical_key() {
        // Every keycode that maps into the matrix must land in a valid cell, and
        // the only cells shared by more than one keycode are the BS/INS/DEL trio.
        let mut owners: std::collections::HashMap<(usize, usize), Vec<u8>> =
            std::collections::HashMap::new();
        for keycode in 0u8..=0x7F {
            if let Some((row, column)) = va_keycode_matrix_cell(keycode) {
                assert!(row < 16, "keycode {keycode:#04x} row {row} out of range");
                assert!(
                    column < 8,
                    "keycode {keycode:#04x} column {column} out of range"
                );
                owners.entry((row, column)).or_default().push(keycode);
            }
        }
        for (cell, keycodes) in &owners {
            match *cell {
                // The combined "Del Ins" key carries BS/INS/DEL.
                (8, 3) => assert_eq!(keycodes.as_slice(), [0x0E, 0x38, 0x39]),
                // RETURN and the keypad ENTER share a cell.
                (1, 7) => assert_eq!(keycodes.as_slice(), [0x1C, 0x79]),
                _ => assert_eq!(
                    keycodes.len(),
                    1,
                    "cell {cell:?} shared by keycodes {keycodes:02x?}"
                ),
            }
        }
    }
}
