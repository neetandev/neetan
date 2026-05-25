//! Console I/O and text VRAM interaction.

use common::JisChar;

use crate::{MemoryAccess, console_esc::EscParser, tables};

const TEXT_CHAR_BASE: u32 = 0xA0000;
const TEXT_ATTR_BASE: u32 = 0xA2000;
const COLUMNS: u8 = 80;

fn vram_char_addr(row: u8, col: u8) -> u32 {
    TEXT_CHAR_BASE + (row as u32 * COLUMNS as u32 + col as u32) * 2
}

fn vram_attr_addr(row: u8, col: u8) -> u32 {
    TEXT_ATTR_BASE + (row as u32 * COLUMNS as u32 + col as u32) * 2
}

#[derive(Default)]
pub(crate) struct Console {
    pub(crate) esc_parser: EscParser,
}

impl Console {
    pub(crate) fn cursor_row(&self, memory: &dyn MemoryAccess) -> u8 {
        memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_Y)
    }

    pub(crate) fn cursor_col(&self, memory: &dyn MemoryAccess) -> u8 {
        memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_X)
    }

    pub(crate) fn set_cursor(&self, memory: &mut dyn MemoryAccess, row: u8, col: u8) {
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_Y, row);
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_X, col);
    }

    fn display_attr(&self, memory: &dyn MemoryAccess) -> u8 {
        memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_DISPLAY_ATTR)
    }

    fn clear_attr(&self, memory: &dyn MemoryAccess) -> u8 {
        memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CLEAR_ATTR)
    }

    fn clear_char(&self, memory: &dyn MemoryAccess) -> u8 {
        memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CLEAR_CHAR)
    }

    pub(crate) fn scroll_upper(&self, memory: &dyn MemoryAccess) -> u8 {
        memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_UPPER)
    }

    pub(crate) fn scroll_lower(&self, memory: &dyn MemoryAccess) -> u8 {
        memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_LOWER)
    }

    fn line_wrap_enabled(&self, memory: &dyn MemoryAccess) -> bool {
        memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_LINE_WRAP) == 0x00
    }

    pub(crate) fn pending_shift_jis_lead(&self, memory: &dyn MemoryAccess) -> Option<u8> {
        if memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_HI_FLAG) == 0 {
            None
        } else {
            Some(memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_HI_BYTE))
        }
    }

    pub(crate) fn set_pending_shift_jis_lead(&self, memory: &mut dyn MemoryAccess, byte: u8) {
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_HI_FLAG, 0x01);
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_HI_BYTE, byte);
    }

    pub(crate) fn clear_pending_shift_jis_lead(&self, memory: &mut dyn MemoryAccess) {
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_HI_FLAG, 0x00);
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_HI_BYTE, 0x00);
    }

    pub(crate) fn write_cell(
        &self,
        memory: &mut dyn MemoryAccess,
        row: u8,
        col: u8,
        jis: JisChar,
        attr: u8,
    ) {
        let (even, odd) = jis.to_vram_bytes();
        let char_addr = vram_char_addr(row, col);
        memory.write_byte(char_addr, even);
        memory.write_byte(char_addr + 1, odd);
        memory.write_word(vram_attr_addr(row, col), attr as u16);
    }

    pub(crate) fn put_char(&self, memory: &mut dyn MemoryAccess, ch: u8) {
        let row = self.cursor_row(memory);
        let col = self.cursor_col(memory);
        let attr = self.display_attr(memory);
        self.write_cell(memory, row, col, JisChar::from_u16(ch as u16), attr);
        self.advance_cursor(memory);
    }

    pub(crate) fn put_fullwidth_jis_char(&self, memory: &mut dyn MemoryAccess, jis: JisChar) {
        let mut row = self.cursor_row(memory);
        let mut col = self.cursor_col(memory);
        if col == COLUMNS - 1 {
            if !self.wrap_to_next_line(memory) {
                return;
            }
            row = self.cursor_row(memory);
            col = self.cursor_col(memory);
        }

        let attr = self.display_attr(memory);
        self.write_cell(memory, row, col, jis, attr);
        self.write_cell(memory, row, col + 1, JisChar::from_u16(0x0000), attr);
        self.advance_cursor(memory);
        self.advance_cursor(memory);
    }

    fn advance_cursor(&self, memory: &mut dyn MemoryAccess) {
        let col = self.cursor_col(memory) + 1;
        let row = self.cursor_row(memory);
        if col >= COLUMNS {
            if !self.wrap_to_next_line(memory) {
                self.set_cursor(memory, row, COLUMNS - 1);
            }
        } else {
            self.set_cursor(memory, row, col);
        }
    }

    fn wrap_to_next_line(&self, memory: &mut dyn MemoryAccess) -> bool {
        if !self.line_wrap_enabled(memory) {
            return false;
        }

        let row = self.cursor_row(memory);
        let scroll_lower = self.scroll_lower(memory);
        if row >= scroll_lower {
            self.scroll_up(memory, 1);
            self.set_cursor(memory, scroll_lower, 0);
        } else {
            self.set_cursor(memory, row + 1, 0);
        }
        true
    }

    pub(crate) fn carriage_return(&self, memory: &mut dyn MemoryAccess) {
        let row = self.cursor_row(memory);
        self.set_cursor(memory, row, 0);
    }

    pub(crate) fn linefeed(&self, memory: &mut dyn MemoryAccess) {
        let row = self.cursor_row(memory);
        let col = self.cursor_col(memory);
        let scroll_lower = self.scroll_lower(memory);
        if row >= scroll_lower {
            self.scroll_up(memory, 1);
        } else {
            self.set_cursor(memory, row + 1, col);
        }
    }

    pub(crate) fn reverse_linefeed(&self, memory: &mut dyn MemoryAccess) {
        let row = self.cursor_row(memory);
        let col = self.cursor_col(memory);
        let scroll_upper = self.scroll_upper(memory);
        if row <= scroll_upper {
            self.scroll_down(memory, 1);
        } else {
            self.set_cursor(memory, row - 1, col);
        }
    }

    pub(crate) fn backspace(&self, memory: &mut dyn MemoryAccess) {
        let row = self.cursor_row(memory);
        let col = self.cursor_col(memory);
        if col > 0 {
            self.set_cursor(memory, row, col - 1);
        }
    }

    pub(crate) fn tab(&self, memory: &mut dyn MemoryAccess) {
        let col = self.cursor_col(memory);
        let next_tab = (col + 8) & !7;
        let target = next_tab.min(COLUMNS - 1);
        for _ in col..target {
            self.put_char(memory, b' ');
        }
    }

    pub(crate) fn scroll_up(&self, memory: &mut dyn MemoryAccess, count: u8) {
        let upper = self.scroll_upper(memory);
        let lower = self.scroll_lower(memory);
        let count = count.min(lower - upper + 1);

        // Copy rows upward.
        for row in upper..=lower.saturating_sub(count) {
            let src_row = row + count;
            if src_row > lower {
                break;
            }
            for col in 0..COLUMNS {
                let src_char = vram_char_addr(src_row, col);
                let dst_char = vram_char_addr(row, col);
                let ch = memory.read_word(src_char);
                memory.write_word(dst_char, ch);
                let src_attr = vram_attr_addr(src_row, col);
                let dst_attr = vram_attr_addr(row, col);
                let attr = memory.read_word(src_attr);
                memory.write_word(dst_attr, attr);
            }
        }

        // Clear the bottom rows.
        let clear_ch = self.clear_char(memory);
        let clear_at = self.clear_attr(memory);
        let jis = JisChar::from_u16(clear_ch as u16);
        let (even, odd) = jis.to_vram_bytes();
        for row in (lower + 1 - count)..=lower {
            for col in 0..COLUMNS {
                let char_addr = vram_char_addr(row, col);
                memory.write_byte(char_addr, even);
                memory.write_byte(char_addr + 1, odd);
                memory.write_word(vram_attr_addr(row, col), clear_at as u16);
            }
        }
    }

    pub(crate) fn scroll_down(&self, memory: &mut dyn MemoryAccess, count: u8) {
        let upper = self.scroll_upper(memory);
        let lower = self.scroll_lower(memory);
        let count = count.min(lower - upper + 1);

        // Copy rows downward (bottom-to-top to avoid overwrite).
        for row in (upper + count..=lower).rev() {
            let src_row = row - count;
            for col in 0..COLUMNS {
                let src_char = vram_char_addr(src_row, col);
                let dst_char = vram_char_addr(row, col);
                let ch = memory.read_word(src_char);
                memory.write_word(dst_char, ch);
                let src_attr = vram_attr_addr(src_row, col);
                let dst_attr = vram_attr_addr(row, col);
                let attr = memory.read_word(src_attr);
                memory.write_word(dst_attr, attr);
            }
        }

        // Clear the top rows.
        let clear_ch = self.clear_char(memory);
        let clear_at = self.clear_attr(memory);
        let jis = JisChar::from_u16(clear_ch as u16);
        let (even, odd) = jis.to_vram_bytes();
        for row in upper..upper + count {
            for col in 0..COLUMNS {
                let char_addr = vram_char_addr(row, col);
                memory.write_byte(char_addr, even);
                memory.write_byte(char_addr + 1, odd);
                memory.write_word(vram_attr_addr(row, col), clear_at as u16);
            }
        }
    }

    pub(crate) fn clear_screen(&self, memory: &mut dyn MemoryAccess) {
        let clear_ch = self.clear_char(memory);
        let clear_at = self.clear_attr(memory);
        let jis = JisChar::from_u16(clear_ch as u16);
        let (even, odd) = jis.to_vram_bytes();
        let lower = self.scroll_lower(memory);
        for row in 0..=lower {
            for col in 0..COLUMNS {
                let char_addr = vram_char_addr(row, col);
                memory.write_byte(char_addr, even);
                memory.write_byte(char_addr + 1, odd);
                memory.write_word(vram_attr_addr(row, col), clear_at as u16);
            }
        }
        self.set_cursor(memory, 0, 0);
    }

    pub(crate) fn clear_screen_from_cursor(&self, memory: &mut dyn MemoryAccess) {
        let row = self.cursor_row(memory);
        let col = self.cursor_col(memory);
        let lower = self.scroll_lower(memory);
        let clear_ch = self.clear_char(memory);
        let clear_at = self.clear_attr(memory);
        let jis = JisChar::from_u16(clear_ch as u16);
        let (even, odd) = jis.to_vram_bytes();
        for c in col..COLUMNS {
            let addr = vram_char_addr(row, c);
            memory.write_byte(addr, even);
            memory.write_byte(addr + 1, odd);
            memory.write_word(vram_attr_addr(row, c), clear_at as u16);
        }
        for r in (row + 1)..=lower {
            for c in 0..COLUMNS {
                let addr = vram_char_addr(r, c);
                memory.write_byte(addr, even);
                memory.write_byte(addr + 1, odd);
                memory.write_word(vram_attr_addr(r, c), clear_at as u16);
            }
        }
    }

    pub(crate) fn clear_screen_to_cursor(&self, memory: &mut dyn MemoryAccess) {
        let row = self.cursor_row(memory);
        let col = self.cursor_col(memory);
        let clear_ch = self.clear_char(memory);
        let clear_at = self.clear_attr(memory);
        let jis = JisChar::from_u16(clear_ch as u16);
        let (even, odd) = jis.to_vram_bytes();
        for r in 0..row {
            for c in 0..COLUMNS {
                let addr = vram_char_addr(r, c);
                memory.write_byte(addr, even);
                memory.write_byte(addr + 1, odd);
                memory.write_word(vram_attr_addr(r, c), clear_at as u16);
            }
        }
        for c in 0..=col {
            let addr = vram_char_addr(row, c);
            memory.write_byte(addr, even);
            memory.write_byte(addr + 1, odd);
            memory.write_word(vram_attr_addr(row, c), clear_at as u16);
        }
    }

    pub(crate) fn clear_line_from_cursor(&self, memory: &mut dyn MemoryAccess) {
        let row = self.cursor_row(memory);
        let col = self.cursor_col(memory);
        let clear_ch = self.clear_char(memory);
        let clear_at = self.clear_attr(memory);
        let jis = JisChar::from_u16(clear_ch as u16);
        let (even, odd) = jis.to_vram_bytes();
        for c in col..COLUMNS {
            let char_addr = vram_char_addr(row, c);
            memory.write_byte(char_addr, even);
            memory.write_byte(char_addr + 1, odd);
            memory.write_word(vram_attr_addr(row, c), clear_at as u16);
        }
    }

    pub(crate) fn clear_line_to_cursor(&self, memory: &mut dyn MemoryAccess) {
        let row = self.cursor_row(memory);
        let col = self.cursor_col(memory);
        let clear_ch = self.clear_char(memory);
        let clear_at = self.clear_attr(memory);
        let jis = JisChar::from_u16(clear_ch as u16);
        let (even, odd) = jis.to_vram_bytes();
        for c in 0..=col {
            let char_addr = vram_char_addr(row, c);
            memory.write_byte(char_addr, even);
            memory.write_byte(char_addr + 1, odd);
            memory.write_word(vram_attr_addr(row, c), clear_at as u16);
        }
    }

    pub(crate) fn clear_line(&self, memory: &mut dyn MemoryAccess) {
        let row = self.cursor_row(memory);
        let clear_ch = self.clear_char(memory);
        let clear_at = self.clear_attr(memory);
        let jis = JisChar::from_u16(clear_ch as u16);
        let (even, odd) = jis.to_vram_bytes();
        for c in 0..COLUMNS {
            let char_addr = vram_char_addr(row, c);
            memory.write_byte(char_addr, even);
            memory.write_byte(char_addr + 1, odd);
            memory.write_word(vram_attr_addr(row, c), clear_at as u16);
        }
    }

    pub(crate) fn set_cursor_position(&self, memory: &mut dyn MemoryAccess, row: u8, col: u8) {
        let lower = self.scroll_lower(memory);
        let clamped_row = row.min(lower);
        let clamped_col = col.min(COLUMNS - 1);
        self.set_cursor(memory, clamped_row, clamped_col);
    }

    pub(crate) fn save_cursor(&self, memory: &mut dyn MemoryAccess) {
        let row = self.cursor_row(memory);
        let col = self.cursor_col(memory);
        let attr = self.display_attr(memory);
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SAVED_CURSOR_Y, row);
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SAVED_CURSOR_X, col);
        memory.write_byte(
            tables::IOSYS_BASE + tables::IOSYS_OFF_SAVED_CURSOR_ATTR,
            attr,
        );
    }

    pub(crate) fn restore_cursor(&self, memory: &mut dyn MemoryAccess) {
        let row = memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SAVED_CURSOR_Y);
        let col = memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SAVED_CURSOR_X);
        let attr = memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SAVED_CURSOR_ATTR);
        self.set_cursor(memory, row, col);
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_DISPLAY_ATTR, attr);
    }

    pub(crate) fn set_attribute(&self, memory: &mut dyn MemoryAccess, attr: u8) {
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_DISPLAY_ATTR, attr);
    }

    pub(crate) fn set_cursor_visible(&self, memory: &mut dyn MemoryAccess, visible: bool) {
        memory.write_byte(
            tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_VISIBLE,
            u8::from(visible),
        );
    }

    pub(crate) fn screen_size(&self, memory: &dyn MemoryAccess) -> (u8, u8) {
        let rows = if memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCREEN_LINES) == 0 {
            20
        } else {
            25
        };
        (COLUMNS, rows)
    }

    pub(crate) fn cursor_up(&self, memory: &mut dyn MemoryAccess, count: u8) {
        let row = self.cursor_row(memory);
        let col = self.cursor_col(memory);
        let upper = self.scroll_upper(memory);
        let new_row = row.saturating_sub(count).max(upper);
        self.set_cursor(memory, new_row, col);
    }

    pub(crate) fn cursor_down(&self, memory: &mut dyn MemoryAccess, count: u8) {
        let row = self.cursor_row(memory);
        let col = self.cursor_col(memory);
        let lower = self.scroll_lower(memory);
        let new_row = row.saturating_add(count).min(lower);
        self.set_cursor(memory, new_row, col);
    }

    pub(crate) fn cursor_right(&self, memory: &mut dyn MemoryAccess, count: u8) {
        let row = self.cursor_row(memory);
        let col = self.cursor_col(memory);
        let new_col = col.saturating_add(count).min(COLUMNS - 1);
        self.set_cursor(memory, row, new_col);
    }

    pub(crate) fn cursor_left(&self, memory: &mut dyn MemoryAccess, count: u8) {
        let row = self.cursor_row(memory);
        let col = self.cursor_col(memory);
        let new_col = col.saturating_sub(count);
        self.set_cursor(memory, row, new_col);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::tables;

    struct MockMemory {
        data: HashMap<u32, u8>,
    }

    impl MockMemory {
        fn with_default_iosys() -> Self {
            let mut memory = Self {
                data: HashMap::new(),
            };
            let base = tables::IOSYS_BASE;
            memory.write_byte(base + tables::IOSYS_OFF_KANJI_MODE, 0x01);
            memory.write_byte(base + tables::IOSYS_OFF_GRAPH_CHAR, 0x20);
            memory.write_byte(base + tables::IOSYS_OFF_SCROLL_LOWER, 24);
            memory.write_byte(base + tables::IOSYS_OFF_SCREEN_LINES, 0x01);
            memory.write_byte(base + tables::IOSYS_OFF_CLEAR_ATTR, 0xE1);
            memory.write_byte(base + tables::IOSYS_OFF_LINE_WRAP, 0x00);
            memory.write_byte(base + tables::IOSYS_OFF_CLEAR_CHAR, 0x20);
            memory.write_byte(base + tables::IOSYS_OFF_CURSOR_VISIBLE, 0x01);
            memory.write_byte(base + tables::IOSYS_OFF_DISPLAY_ATTR, 0xE1);
            memory.write_byte(base + tables::IOSYS_OFF_SCROLL_UPPER, 0x00);
            memory
        }
    }

    impl MemoryAccess for MockMemory {
        fn read_byte(&self, address: u32) -> u8 {
            self.data.get(&address).copied().unwrap_or(0x00)
        }

        fn write_byte(&mut self, address: u32, value: u8) {
            self.data.insert(address, value);
        }

        fn read_word(&self, address: u32) -> u16 {
            let lo = self.read_byte(address) as u16;
            let hi = self.read_byte(address + 1) as u16;
            (hi << 8) | lo
        }

        fn write_word(&mut self, address: u32, value: u16) {
            self.write_byte(address, value as u8);
            self.write_byte(address + 1, (value >> 8) as u8);
        }

        fn read_block(&self, address: u32, buf: &mut [u8]) {
            for (i, byte) in buf.iter_mut().enumerate() {
                *byte = self.read_byte(address + i as u32);
            }
        }

        fn write_block(&mut self, address: u32, data: &[u8]) {
            for (i, &byte) in data.iter().enumerate() {
                self.write_byte(address + i as u32, byte);
            }
        }
    }

    fn make_console() -> Console {
        Console::default()
    }

    fn make_memory() -> MockMemory {
        MockMemory::with_default_iosys()
    }

    fn read_vram_char(memory: &MockMemory, row: u8, col: u8) -> (u8, u8) {
        let addr = vram_char_addr(row, col);
        (memory.read_byte(addr), memory.read_byte(addr + 1))
    }

    fn read_vram_attr(memory: &MockMemory, row: u8, col: u8) -> u16 {
        memory.read_word(vram_attr_addr(row, col))
    }

    fn assert_cursor(console: &Console, memory: &MockMemory, row: u8, col: u8) {
        assert_eq!(
            (console.cursor_row(memory), console.cursor_col(memory)),
            (row, col),
            "expected cursor at ({}, {}), got ({}, {})",
            row,
            col,
            console.cursor_row(memory),
            console.cursor_col(memory),
        );
    }

    fn assert_vram_char(
        memory: &MockMemory,
        row: u8,
        col: u8,
        expected_even: u8,
        expected_odd: u8,
    ) {
        let (even, odd) = read_vram_char(memory, row, col);
        assert_eq!(
            (even, odd),
            (expected_even, expected_odd),
            "VRAM char at ({}, {}): expected ({:#04X}, {:#04X}), got ({:#04X}, {:#04X})",
            row,
            col,
            expected_even,
            expected_odd,
            even,
            odd,
        );
    }

    fn assert_vram_jis(memory: &MockMemory, row: u8, col: u8, expected: JisChar) {
        let (even, odd) = read_vram_char(memory, row, col);
        assert_eq!(
            JisChar::from_vram_bytes(even, odd),
            expected,
            "VRAM JIS char at ({}, {}) did not match",
            row,
            col,
        );
    }

    fn assert_vram_clear(memory: &MockMemory, row: u8, col_start: u8, col_end: u8) {
        for col in col_start..=col_end {
            assert_vram_char(memory, row, col, 0x20, 0x00);
            assert_eq!(
                read_vram_attr(memory, row, col),
                0x00E1,
                "VRAM attr at ({}, {}) should be 0x00E1",
                row,
                col,
            );
        }
    }

    fn write_vram_char(memory: &mut MockMemory, row: u8, col: u8, ch: u8) {
        let addr = vram_char_addr(row, col);
        memory.write_byte(addr, ch);
        memory.write_byte(addr + 1, 0x00);
        memory.write_word(vram_attr_addr(row, col), 0x00E1);
    }

    fn fill_row(memory: &mut MockMemory, row: u8, ch: u8) {
        for col in 0..COLUMNS {
            write_vram_char(memory, row, col, ch);
        }
    }

    fn feed_str(console: &mut Console, memory: &mut MockMemory, s: &str) {
        for byte in s.bytes() {
            console.process_byte(memory, byte);
        }
    }

    #[test]
    fn cursor_initial_position() {
        let console = make_console();
        let memory = make_memory();
        assert_cursor(&console, &memory, 0, 0);
    }

    #[test]
    fn set_cursor_and_read_back() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        assert_cursor(&console, &memory, 5, 10);
    }

    #[test]
    fn set_cursor_overwrites_previous() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 3, 7);
        console.set_cursor(&mut memory, 10, 20);
        assert_cursor(&console, &memory, 10, 20);
    }

    #[test]
    fn display_attr_default() {
        let console = make_console();
        let memory = make_memory();
        assert_eq!(console.display_attr(&memory), 0xE1);
    }

    #[test]
    fn scroll_bounds_default() {
        let console = make_console();
        let memory = make_memory();
        assert_eq!(console.scroll_upper(&memory), 0);
        assert_eq!(console.scroll_lower(&memory), 24);
    }

    #[test]
    fn put_char_writes_to_vram() {
        let console = make_console();
        let mut memory = make_memory();
        console.put_char(&mut memory, b'A');
        assert_vram_char(&memory, 0, 0, 0x41, 0x00);
        assert_eq!(read_vram_attr(&memory, 0, 0), 0x00E1);
    }

    #[test]
    fn put_char_advances_cursor() {
        let console = make_console();
        let mut memory = make_memory();
        console.put_char(&mut memory, b'A');
        assert_cursor(&console, &memory, 0, 1);
    }

    #[test]
    fn put_char_uses_display_attr() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_attribute(&mut memory, 0x45);
        console.put_char(&mut memory, b'X');
        assert_eq!(read_vram_attr(&memory, 0, 0), 0x0045);
    }

    #[test]
    fn put_char_sequence() {
        let console = make_console();
        let mut memory = make_memory();
        console.put_char(&mut memory, b'A');
        console.put_char(&mut memory, b'B');
        console.put_char(&mut memory, b'C');
        assert_vram_char(&memory, 0, 0, 0x41, 0x00);
        assert_vram_char(&memory, 0, 1, 0x42, 0x00);
        assert_vram_char(&memory, 0, 2, 0x43, 0x00);
        assert_cursor(&console, &memory, 0, 3);
    }

    #[test]
    fn process_byte_shift_jis_pair_writes_fullwidth_char() {
        let mut console = make_console();
        let mut memory = make_memory();

        console.process_byte(&mut memory, 0x82);
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_HI_FLAG),
            0x01
        );
        assert_cursor(&console, &memory, 0, 0);

        console.process_byte(&mut memory, 0xA0);

        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_HI_FLAG),
            0x00
        );
        assert_vram_jis(&memory, 0, 0, JisChar::from_u16(0x2422));
        assert_vram_char(&memory, 0, 1, 0x00, 0x00);
        assert_cursor(&console, &memory, 0, 2);
    }

    #[test]
    fn process_byte_halfwidth_kana_stays_single_cell() {
        let mut console = make_console();
        let mut memory = make_memory();

        console.process_byte(&mut memory, 0xA6);

        assert_vram_char(&memory, 0, 0, 0xA6, 0x00);
        assert_cursor(&console, &memory, 0, 1);
    }

    #[test]
    fn process_byte_invalid_shift_jis_trail_falls_back_to_single_byte() {
        let mut console = make_console();
        let mut memory = make_memory();

        console.process_byte(&mut memory, 0x82);
        console.process_byte(&mut memory, 0x0D);

        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_HI_FLAG),
            0x00
        );
        assert_vram_char(&memory, 0, 0, 0x82, 0x00);
        assert_cursor(&console, &memory, 0, 0);
    }

    #[test]
    fn process_byte_fullwidth_wraps_before_last_column() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 0, 79);

        console.process_byte(&mut memory, 0x82);
        console.process_byte(&mut memory, 0xA0);

        assert_vram_jis(&memory, 1, 0, JisChar::from_u16(0x2422));
        assert_vram_char(&memory, 1, 1, 0x00, 0x00);
        assert_cursor(&console, &memory, 1, 2);
    }

    #[test]
    fn process_byte_fullwidth_at_last_column_no_wrap_is_dropped() {
        let mut console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_LINE_WRAP, 0x01);
        console.set_cursor(&mut memory, 0, 79);

        console.process_byte(&mut memory, 0x82);
        console.process_byte(&mut memory, 0xA0);

        assert_vram_char(&memory, 0, 79, 0x00, 0x00);
        assert_cursor(&console, &memory, 0, 79);
    }

    #[test]
    fn put_char_at_end_of_row_wraps() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 0, 79);
        console.put_char(&mut memory, b'Z');
        assert_vram_char(&memory, 0, 79, b'Z', 0x00);
        assert_cursor(&console, &memory, 1, 0);
    }

    #[test]
    fn put_char_at_end_of_row_no_wrap() {
        let console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_LINE_WRAP, 0x01);
        console.set_cursor(&mut memory, 0, 79);
        console.put_char(&mut memory, b'Z');
        assert_vram_char(&memory, 0, 79, b'Z', 0x00);
        assert_cursor(&console, &memory, 0, 79);
    }

    #[test]
    fn advance_wraps_to_next_row() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 0, 79);
        console.put_char(&mut memory, b'X');
        assert_cursor(&console, &memory, 1, 0);
    }

    #[test]
    fn advance_at_bottom_scrolls() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        console.set_cursor(&mut memory, 24, 79);
        console.put_char(&mut memory, b'X');
        assert_cursor(&console, &memory, 24, 0);
        // Row 0 should now have what was row 1 (empty/zero).
        let (even, _) = read_vram_char(&memory, 0, 0);
        assert_eq!(
            even, 0x00,
            "row 0 should have been scrolled away (row 1 content)"
        );
    }

    #[test]
    fn advance_no_wrap_stays_at_79() {
        let console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_LINE_WRAP, 0x01);
        console.set_cursor(&mut memory, 5, 79);
        console.put_char(&mut memory, b'X');
        assert_cursor(&console, &memory, 5, 79);
    }

    #[test]
    fn advance_wraps_mid_screen() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 10, 79);
        console.put_char(&mut memory, b'X');
        assert_cursor(&console, &memory, 11, 0);
    }

    #[test]
    fn advance_at_custom_scroll_lower() {
        let console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_LOWER, 10);
        fill_row(&mut memory, 0, b'A');
        console.set_cursor(&mut memory, 10, 79);
        console.put_char(&mut memory, b'X');
        assert_cursor(&console, &memory, 10, 0);
    }

    #[test]
    fn carriage_return_sets_col_zero() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 30);
        console.carriage_return(&mut memory);
        assert_cursor(&console, &memory, 5, 0);
    }

    #[test]
    fn carriage_return_at_col_zero() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 0);
        console.carriage_return(&mut memory);
        assert_cursor(&console, &memory, 5, 0);
    }

    #[test]
    fn carriage_return_preserves_row() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 20, 50);
        console.carriage_return(&mut memory);
        assert_eq!(console.cursor_row(&memory), 20);
    }

    #[test]
    fn linefeed_moves_down() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        console.linefeed(&mut memory);
        assert_cursor(&console, &memory, 6, 10);
    }

    #[test]
    fn linefeed_at_scroll_lower_scrolls() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        fill_row(&mut memory, 24, b'Z');
        console.set_cursor(&mut memory, 24, 10);
        console.linefeed(&mut memory);
        // Cursor should stay on row 24 (scroll happened).
        assert_cursor(&console, &memory, 24, 10);
        // Row 0 should now have row 1 content (was empty).
        let (even, _) = read_vram_char(&memory, 0, 0);
        assert_eq!(even, 0x00);
    }

    #[test]
    fn linefeed_preserves_col() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 3, 42);
        console.linefeed(&mut memory);
        assert_eq!(console.cursor_col(&memory), 42);
    }

    #[test]
    fn linefeed_custom_scroll_lower() {
        let console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_LOWER, 15);
        console.set_cursor(&mut memory, 15, 0);
        console.linefeed(&mut memory);
        assert_cursor(&console, &memory, 15, 0);
    }

    #[test]
    fn reverse_linefeed_moves_up() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        console.reverse_linefeed(&mut memory);
        assert_cursor(&console, &memory, 4, 10);
    }

    #[test]
    fn reverse_linefeed_at_scroll_upper_scrolls() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        fill_row(&mut memory, 24, b'Z');
        console.set_cursor(&mut memory, 0, 10);
        console.reverse_linefeed(&mut memory);
        assert_cursor(&console, &memory, 0, 10);
        // Row 1 should now have the old row 0 content ('A').
        let (even, _) = read_vram_char(&memory, 1, 0);
        assert_eq!(even, b'A');
    }

    #[test]
    fn reverse_linefeed_preserves_col() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 3, 42);
        console.reverse_linefeed(&mut memory);
        assert_eq!(console.cursor_col(&memory), 42);
    }

    #[test]
    fn reverse_linefeed_custom_scroll_upper() {
        let console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_UPPER, 5);
        console.set_cursor(&mut memory, 5, 0);
        console.reverse_linefeed(&mut memory);
        assert_cursor(&console, &memory, 5, 0);
    }

    #[test]
    fn backspace_moves_left() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        console.backspace(&mut memory);
        assert_cursor(&console, &memory, 5, 9);
    }

    #[test]
    fn backspace_at_col_zero_no_op() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 0);
        console.backspace(&mut memory);
        assert_cursor(&console, &memory, 5, 0);
    }

    #[test]
    fn backspace_does_not_erase() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        console.put_char(&mut memory, b'A');
        console.backspace(&mut memory);
        assert_cursor(&console, &memory, 5, 10);
        assert_vram_char(&memory, 5, 10, 0x41, 0x00);
    }

    #[test]
    fn tab_from_col_zero() {
        let console = make_console();
        let mut memory = make_memory();
        console.tab(&mut memory);
        assert_cursor(&console, &memory, 0, 8);
    }

    #[test]
    fn tab_from_col_five() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 0, 5);
        console.tab(&mut memory);
        assert_cursor(&console, &memory, 0, 8);
    }

    #[test]
    fn tab_from_col_eight() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 0, 8);
        console.tab(&mut memory);
        assert_cursor(&console, &memory, 0, 16);
    }

    #[test]
    fn tab_near_end_of_row() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 0, 75);
        console.tab(&mut memory);
        // (75 + 8) & !7 = 80, clamped to 79.
        assert_cursor(&console, &memory, 0, 79);
    }

    #[test]
    fn scroll_up_one_row() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        fill_row(&mut memory, 1, b'B');
        fill_row(&mut memory, 2, b'C');
        console.scroll_up(&mut memory, 1);
        // Row 0 should now have 'B', row 1 should have 'C'.
        assert_vram_char(&memory, 0, 0, b'B', 0x00);
        assert_vram_char(&memory, 1, 0, b'C', 0x00);
        // Row 24 (bottom) should be cleared.
        assert_vram_clear(&memory, 24, 0, 79);
    }

    #[test]
    fn scroll_up_multiple_rows() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        fill_row(&mut memory, 1, b'B');
        fill_row(&mut memory, 2, b'C');
        fill_row(&mut memory, 3, b'D');
        fill_row(&mut memory, 4, b'E');
        console.scroll_up(&mut memory, 3);
        // Row 0 should now have 'D', row 1 should have 'E'.
        assert_vram_char(&memory, 0, 0, b'D', 0x00);
        assert_vram_char(&memory, 1, 0, b'E', 0x00);
        // Bottom 3 rows should be cleared.
        assert_vram_clear(&memory, 22, 0, 79);
        assert_vram_clear(&memory, 23, 0, 79);
        assert_vram_clear(&memory, 24, 0, 79);
    }

    #[test]
    fn scroll_up_uses_clear_char() {
        let console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CLEAR_CHAR, 0x2E);
        fill_row(&mut memory, 0, b'A');
        console.scroll_up(&mut memory, 1);
        // Bottom row should have clear char 0x2E ('.').
        assert_vram_char(&memory, 24, 0, 0x2E, 0x00);
    }

    #[test]
    fn scroll_up_uses_clear_attr() {
        let console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CLEAR_ATTR, 0x45);
        fill_row(&mut memory, 0, b'A');
        console.scroll_up(&mut memory, 1);
        assert_eq!(read_vram_attr(&memory, 24, 0), 0x0045);
    }

    #[test]
    fn scroll_up_respects_scroll_region() {
        let console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_UPPER, 5);
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_LOWER, 10);
        fill_row(&mut memory, 0, b'X');
        fill_row(&mut memory, 5, b'A');
        fill_row(&mut memory, 6, b'B');
        fill_row(&mut memory, 7, b'C');
        console.scroll_up(&mut memory, 1);
        // Row 0 should be untouched (outside scroll region).
        assert_vram_char(&memory, 0, 0, b'X', 0x00);
        // Row 5 should now have 'B'.
        assert_vram_char(&memory, 5, 0, b'B', 0x00);
        assert_vram_char(&memory, 6, 0, b'C', 0x00);
        // Row 10 should be cleared.
        assert_vram_clear(&memory, 10, 0, 79);
    }

    #[test]
    fn scroll_up_count_exceeds_region() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        fill_row(&mut memory, 12, b'M');
        fill_row(&mut memory, 24, b'Z');
        console.scroll_up(&mut memory, 30);
        // All rows should be cleared.
        assert_vram_clear(&memory, 0, 0, 79);
        assert_vram_clear(&memory, 12, 0, 79);
        assert_vram_clear(&memory, 24, 0, 79);
    }

    #[test]
    fn scroll_down_one_row() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        fill_row(&mut memory, 1, b'B');
        fill_row(&mut memory, 2, b'C');
        console.scroll_down(&mut memory, 1);
        // Row 0 should be cleared, row 1 should have 'A', row 2 should have 'B'.
        assert_vram_clear(&memory, 0, 0, 79);
        assert_vram_char(&memory, 1, 0, b'A', 0x00);
        assert_vram_char(&memory, 2, 0, b'B', 0x00);
    }

    #[test]
    fn scroll_down_multiple_rows() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        fill_row(&mut memory, 1, b'B');
        fill_row(&mut memory, 2, b'C');
        console.scroll_down(&mut memory, 3);
        // Top 3 rows cleared.
        assert_vram_clear(&memory, 0, 0, 79);
        assert_vram_clear(&memory, 1, 0, 79);
        assert_vram_clear(&memory, 2, 0, 79);
        // Row 3 should have 'A'.
        assert_vram_char(&memory, 3, 0, b'A', 0x00);
    }

    #[test]
    fn scroll_down_uses_clear_char() {
        let console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CLEAR_CHAR, 0x2E);
        fill_row(&mut memory, 0, b'A');
        console.scroll_down(&mut memory, 1);
        assert_vram_char(&memory, 0, 0, 0x2E, 0x00);
    }

    #[test]
    fn scroll_down_respects_scroll_region() {
        let console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_UPPER, 5);
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_LOWER, 10);
        fill_row(&mut memory, 0, b'X');
        fill_row(&mut memory, 5, b'A');
        fill_row(&mut memory, 6, b'B');
        console.scroll_down(&mut memory, 1);
        // Row 0 untouched.
        assert_vram_char(&memory, 0, 0, b'X', 0x00);
        // Row 5 cleared (top of region).
        assert_vram_clear(&memory, 5, 0, 79);
        // Row 6 should have 'A'.
        assert_vram_char(&memory, 6, 0, b'A', 0x00);
    }

    #[test]
    fn scroll_down_count_exceeds_region() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        fill_row(&mut memory, 12, b'M');
        fill_row(&mut memory, 24, b'Z');
        console.scroll_down(&mut memory, 30);
        assert_vram_clear(&memory, 0, 0, 79);
        assert_vram_clear(&memory, 12, 0, 79);
        assert_vram_clear(&memory, 24, 0, 79);
    }

    #[test]
    fn clear_screen_fills_vram() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        fill_row(&mut memory, 12, b'M');
        fill_row(&mut memory, 24, b'Z');
        console.clear_screen(&mut memory);
        assert_vram_clear(&memory, 0, 0, 79);
        assert_vram_clear(&memory, 12, 0, 79);
        assert_vram_clear(&memory, 24, 0, 79);
    }

    #[test]
    fn clear_screen_resets_cursor() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 10, 30);
        console.clear_screen(&mut memory);
        assert_cursor(&console, &memory, 0, 0);
    }

    #[test]
    fn clear_screen_respects_scroll_lower() {
        let console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_LOWER, 10);
        fill_row(&mut memory, 5, b'A');
        fill_row(&mut memory, 15, b'X');
        console.clear_screen(&mut memory);
        // Row 5 should be cleared (within 0..=10).
        assert_vram_clear(&memory, 5, 0, 79);
        // Row 15 should be untouched (beyond scroll_lower).
        assert_vram_char(&memory, 15, 0, b'X', 0x00);
    }

    #[test]
    fn clear_line_from_cursor_clears_right() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 5, b'X');
        console.set_cursor(&mut memory, 5, 10);
        console.clear_line_from_cursor(&mut memory);
        // Cols 0-9 should still have 'X'.
        assert_vram_char(&memory, 5, 0, b'X', 0x00);
        assert_vram_char(&memory, 5, 9, b'X', 0x00);
        // Cols 10-79 should be cleared.
        assert_vram_clear(&memory, 5, 10, 79);
    }

    #[test]
    fn clear_line_from_cursor_at_col_zero() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 5, b'X');
        console.set_cursor(&mut memory, 5, 0);
        console.clear_line_from_cursor(&mut memory);
        assert_vram_clear(&memory, 5, 0, 79);
    }

    #[test]
    fn clear_line_to_cursor_clears_left() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 5, b'X');
        console.set_cursor(&mut memory, 5, 10);
        console.clear_line_to_cursor(&mut memory);
        // Cols 0-10 should be cleared.
        assert_vram_clear(&memory, 5, 0, 10);
        // Cols 11-79 should still have 'X'.
        assert_vram_char(&memory, 5, 11, b'X', 0x00);
        assert_vram_char(&memory, 5, 79, b'X', 0x00);
    }

    #[test]
    fn clear_line_to_cursor_at_last_col() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 5, b'X');
        console.set_cursor(&mut memory, 5, 79);
        console.clear_line_to_cursor(&mut memory);
        assert_vram_clear(&memory, 5, 0, 79);
    }

    #[test]
    fn clear_line_clears_full_row() {
        let console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 5, b'X');
        console.set_cursor(&mut memory, 5, 30);
        console.clear_line(&mut memory);
        assert_vram_clear(&memory, 5, 0, 79);
    }

    #[test]
    fn clear_line_preserves_cursor() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 30);
        console.clear_line(&mut memory);
        assert_cursor(&console, &memory, 5, 30);
    }

    #[test]
    fn clear_line_uses_clear_char_and_attr() {
        let console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CLEAR_CHAR, 0x2E);
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CLEAR_ATTR, 0x45);
        fill_row(&mut memory, 5, b'X');
        console.set_cursor(&mut memory, 5, 0);
        console.clear_line(&mut memory);
        assert_vram_char(&memory, 5, 0, 0x2E, 0x00);
        assert_eq!(read_vram_attr(&memory, 5, 0), 0x0045);
        assert_vram_char(&memory, 5, 79, 0x2E, 0x00);
        assert_eq!(read_vram_attr(&memory, 5, 79), 0x0045);
    }

    #[test]
    fn set_cursor_position_normal() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor_position(&mut memory, 5, 10);
        assert_cursor(&console, &memory, 5, 10);
    }

    #[test]
    fn set_cursor_position_clamps_row() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor_position(&mut memory, 30, 10);
        assert_cursor(&console, &memory, 24, 10);
    }

    #[test]
    fn set_cursor_position_clamps_col() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor_position(&mut memory, 5, 100);
        assert_cursor(&console, &memory, 5, 79);
    }

    #[test]
    fn set_cursor_position_clamps_both() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor_position(&mut memory, 255, 255);
        assert_cursor(&console, &memory, 24, 79);
    }

    #[test]
    fn save_and_restore_cursor() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 10, 30);
        console.set_attribute(&mut memory, 0x45);
        console.save_cursor(&mut memory);
        console.set_cursor(&mut memory, 0, 0);
        console.set_attribute(&mut memory, 0xE1);
        console.restore_cursor(&mut memory);
        assert_cursor(&console, &memory, 10, 30);
        assert_eq!(console.display_attr(&memory), 0x45);
    }

    #[test]
    fn save_cursor_stores_attr() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_attribute(&mut memory, 0x77);
        console.save_cursor(&mut memory);
        console.set_attribute(&mut memory, 0x22);
        assert_eq!(console.display_attr(&memory), 0x22);
        console.restore_cursor(&mut memory);
        assert_eq!(console.display_attr(&memory), 0x77);
    }

    #[test]
    fn restore_reads_saved_values() {
        let console = make_console();
        let mut memory = make_memory();
        let base = tables::IOSYS_BASE;
        memory.write_byte(base + tables::IOSYS_OFF_SAVED_CURSOR_Y, 15);
        memory.write_byte(base + tables::IOSYS_OFF_SAVED_CURSOR_X, 40);
        memory.write_byte(base + tables::IOSYS_OFF_SAVED_CURSOR_ATTR, 0x33);
        console.restore_cursor(&mut memory);
        assert_cursor(&console, &memory, 15, 40);
        assert_eq!(console.display_attr(&memory), 0x33);
    }

    #[test]
    fn double_save_overwrites() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        console.save_cursor(&mut memory);
        console.set_cursor(&mut memory, 20, 60);
        console.save_cursor(&mut memory);
        console.set_cursor(&mut memory, 0, 0);
        console.restore_cursor(&mut memory);
        assert_cursor(&console, &memory, 20, 60);
    }

    #[test]
    fn set_attribute_changes_display_attr() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_attribute(&mut memory, 0x45);
        assert_eq!(console.display_attr(&memory), 0x45);
    }

    #[test]
    fn set_attribute_affects_put_char() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_attribute(&mut memory, 0x45);
        console.put_char(&mut memory, b'X');
        assert_eq!(read_vram_attr(&memory, 0, 0), 0x0045);
    }

    #[test]
    fn cursor_up_basic() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 10, 5);
        console.cursor_up(&mut memory, 3);
        assert_cursor(&console, &memory, 7, 5);
    }

    #[test]
    fn cursor_up_clamped() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 2, 5);
        console.cursor_up(&mut memory, 10);
        assert_cursor(&console, &memory, 0, 5);
    }

    #[test]
    fn cursor_up_custom_upper() {
        let console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_UPPER, 5);
        console.set_cursor(&mut memory, 7, 5);
        console.cursor_up(&mut memory, 10);
        assert_cursor(&console, &memory, 5, 5);
    }

    #[test]
    fn cursor_down_basic() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 10, 5);
        console.cursor_down(&mut memory, 3);
        assert_cursor(&console, &memory, 13, 5);
    }

    #[test]
    fn cursor_down_clamped() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 22, 5);
        console.cursor_down(&mut memory, 10);
        assert_cursor(&console, &memory, 24, 5);
    }

    #[test]
    fn cursor_down_custom_lower() {
        let console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_LOWER, 15);
        console.set_cursor(&mut memory, 14, 5);
        console.cursor_down(&mut memory, 10);
        assert_cursor(&console, &memory, 15, 5);
    }

    #[test]
    fn cursor_right_basic() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        console.cursor_right(&mut memory, 5);
        assert_cursor(&console, &memory, 5, 15);
    }

    #[test]
    fn cursor_right_clamped() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 70);
        console.cursor_right(&mut memory, 20);
        assert_cursor(&console, &memory, 5, 79);
    }

    #[test]
    fn cursor_left_basic() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        console.cursor_left(&mut memory, 5);
        assert_cursor(&console, &memory, 5, 5);
    }

    #[test]
    fn cursor_left_clamped() {
        let console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 3);
        console.cursor_left(&mut memory, 10);
        assert_cursor(&console, &memory, 5, 0);
    }

    #[test]
    fn esc_bel_no_op() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.process_byte(&mut memory, 0x07);
        assert_cursor(&console, &memory, 0, 0);
    }

    #[test]
    fn esc_bs_calls_backspace() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        console.process_byte(&mut memory, 0x08);
        assert_cursor(&console, &memory, 5, 9);
    }

    #[test]
    fn esc_tab_calls_tab() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.process_byte(&mut memory, 0x09);
        assert_cursor(&console, &memory, 0, 8);
    }

    #[test]
    fn esc_lf_calls_linefeed() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        console.process_byte(&mut memory, 0x0A);
        assert_cursor(&console, &memory, 6, 10);
    }

    #[test]
    fn esc_cr_calls_carriage_return() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        console.process_byte(&mut memory, 0x0D);
        assert_cursor(&console, &memory, 5, 0);
    }

    #[test]
    fn esc_plain_d_calls_linefeed() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        feed_str(&mut console, &mut memory, "\x1bD");
        assert_cursor(&console, &memory, 6, 10);
    }

    #[test]
    fn esc_plain_e_calls_carriage_return_then_linefeed() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        feed_str(&mut console, &mut memory, "\x1bE");
        assert_cursor(&console, &memory, 6, 0);
    }

    #[test]
    fn esc_plain_m_calls_reverse_linefeed() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        feed_str(&mut console, &mut memory, "\x1bM");
        assert_cursor(&console, &memory, 4, 10);
    }

    #[test]
    fn esc_printable_single() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.process_byte(&mut memory, b'A');
        assert_vram_char(&memory, 0, 0, 0x41, 0x00);
        assert_cursor(&console, &memory, 0, 1);
    }

    #[test]
    fn esc_printable_string() {
        let mut console = make_console();
        let mut memory = make_memory();
        feed_str(&mut console, &mut memory, "Hello");
        assert_vram_char(&memory, 0, 0, b'H', 0x00);
        assert_vram_char(&memory, 0, 1, b'e', 0x00);
        assert_vram_char(&memory, 0, 2, b'l', 0x00);
        assert_vram_char(&memory, 0, 3, b'l', 0x00);
        assert_vram_char(&memory, 0, 4, b'o', 0x00);
        assert_cursor(&console, &memory, 0, 5);
    }

    #[test]
    fn esc_printable_after_cr_lf() {
        let mut console = make_console();
        let mut memory = make_memory();
        feed_str(&mut console, &mut memory, "A\r\nB");
        assert_vram_char(&memory, 0, 0, b'A', 0x00);
        assert_vram_char(&memory, 1, 0, b'B', 0x00);
        assert_cursor(&console, &memory, 1, 1);
    }

    #[test]
    fn esc_csi_h_sets_cursor() {
        let mut console = make_console();
        let mut memory = make_memory();
        feed_str(&mut console, &mut memory, "\x1b[5;10H");
        assert_cursor(&console, &memory, 4, 9);
    }

    #[test]
    fn esc_csi_f_sets_cursor() {
        let mut console = make_console();
        let mut memory = make_memory();
        feed_str(&mut console, &mut memory, "\x1b[5;10f");
        assert_cursor(&console, &memory, 4, 9);
    }

    #[test]
    fn esc_csi_h_defaults_to_origin() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 10, 20);
        feed_str(&mut console, &mut memory, "\x1b[H");
        assert_cursor(&console, &memory, 0, 0);
    }

    #[test]
    fn esc_csi_h_row_only() {
        let mut console = make_console();
        let mut memory = make_memory();
        feed_str(&mut console, &mut memory, "\x1b[5H");
        assert_cursor(&console, &memory, 4, 0);
    }

    #[test]
    fn esc_csi_a_cursor_up() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 10, 5);
        feed_str(&mut console, &mut memory, "\x1b[3A");
        assert_cursor(&console, &memory, 7, 5);
    }

    #[test]
    fn esc_csi_b_cursor_down() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 10, 5);
        feed_str(&mut console, &mut memory, "\x1b[3B");
        assert_cursor(&console, &memory, 13, 5);
    }

    #[test]
    fn esc_csi_c_cursor_right() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        feed_str(&mut console, &mut memory, "\x1b[5C");
        assert_cursor(&console, &memory, 5, 15);
    }

    #[test]
    fn esc_csi_d_cursor_left() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        feed_str(&mut console, &mut memory, "\x1b[5D");
        assert_cursor(&console, &memory, 5, 5);
    }

    #[test]
    fn esc_csi_a_default_count() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 10, 5);
        feed_str(&mut console, &mut memory, "\x1b[A");
        assert_cursor(&console, &memory, 9, 5);
    }

    #[test]
    fn esc_csi_b_default_count() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 10, 5);
        feed_str(&mut console, &mut memory, "\x1b[B");
        assert_cursor(&console, &memory, 11, 5);
    }

    #[test]
    fn esc_csi_c_default_count() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        feed_str(&mut console, &mut memory, "\x1b[C");
        assert_cursor(&console, &memory, 5, 11);
    }

    #[test]
    fn esc_csi_d_default_count() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        feed_str(&mut console, &mut memory, "\x1b[D");
        assert_cursor(&console, &memory, 5, 9);
    }

    #[test]
    fn esc_csi_s_saves_cursor() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        console.set_attribute(&mut memory, 0x45);
        feed_str(&mut console, &mut memory, "\x1b[s");
        let base = tables::IOSYS_BASE;
        assert_eq!(memory.read_byte(base + tables::IOSYS_OFF_SAVED_CURSOR_Y), 5);
        assert_eq!(
            memory.read_byte(base + tables::IOSYS_OFF_SAVED_CURSOR_X),
            10
        );
        assert_eq!(
            memory.read_byte(base + tables::IOSYS_OFF_SAVED_CURSOR_ATTR),
            0x45
        );
    }

    #[test]
    fn esc_csi_u_restores_cursor() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        console.set_attribute(&mut memory, 0x45);
        feed_str(&mut console, &mut memory, "\x1b[s");
        console.set_cursor(&mut memory, 20, 60);
        console.set_attribute(&mut memory, 0xE1);
        feed_str(&mut console, &mut memory, "\x1b[u");
        assert_cursor(&console, &memory, 5, 10);
        assert_eq!(console.display_attr(&memory), 0x45);
    }

    #[test]
    fn esc_csi_2j_clears_screen() {
        let mut console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        fill_row(&mut memory, 12, b'M');
        feed_str(&mut console, &mut memory, "\x1b[2J");
        assert_vram_clear(&memory, 0, 0, 79);
        assert_vram_clear(&memory, 12, 0, 79);
        assert_cursor(&console, &memory, 0, 0);
    }

    #[test]
    fn esc_csi_0j_clears_from_cursor() {
        let mut console = make_console();
        let mut memory = make_memory();
        for r in 0..25 {
            fill_row(&mut memory, r, b'X');
        }
        console.set_cursor(&mut memory, 5, 10);
        feed_str(&mut console, &mut memory, "\x1b[0J");
        // Rows 0-4 preserved.
        assert_vram_char(&memory, 0, 0, b'X', 0x00);
        assert_vram_char(&memory, 4, 79, b'X', 0x00);
        // Row 5 cols 0-9 preserved, 10-79 cleared.
        assert_vram_char(&memory, 5, 9, b'X', 0x00);
        assert_vram_clear(&memory, 5, 10, 79);
        // Rows 6-24 cleared.
        assert_vram_clear(&memory, 6, 0, 79);
        assert_vram_clear(&memory, 24, 0, 79);
        // Cursor unchanged.
        assert_cursor(&console, &memory, 5, 10);
    }

    #[test]
    fn esc_csi_j_default_clears_from_cursor() {
        let mut console = make_console();
        let mut memory = make_memory();
        for r in 0..25 {
            fill_row(&mut memory, r, b'X');
        }
        console.set_cursor(&mut memory, 3, 0);
        // ESC[J with no parameter defaults to mode 0.
        feed_str(&mut console, &mut memory, "\x1b[J");
        assert_vram_char(&memory, 2, 79, b'X', 0x00);
        assert_vram_clear(&memory, 3, 0, 79);
        assert_vram_clear(&memory, 24, 0, 79);
        assert_cursor(&console, &memory, 3, 0);
    }

    #[test]
    fn esc_csi_1j_clears_to_cursor() {
        let mut console = make_console();
        let mut memory = make_memory();
        for r in 0..25 {
            fill_row(&mut memory, r, b'X');
        }
        console.set_cursor(&mut memory, 5, 10);
        feed_str(&mut console, &mut memory, "\x1b[1J");
        // Rows 0-4 cleared.
        assert_vram_clear(&memory, 0, 0, 79);
        assert_vram_clear(&memory, 4, 0, 79);
        // Row 5 cols 0-10 cleared, 11-79 preserved.
        assert_vram_clear(&memory, 5, 0, 10);
        assert_vram_char(&memory, 5, 11, b'X', 0x00);
        // Rows 6-24 preserved.
        assert_vram_char(&memory, 6, 0, b'X', 0x00);
        assert_vram_char(&memory, 24, 79, b'X', 0x00);
        assert_cursor(&console, &memory, 5, 10);
    }

    #[test]
    fn esc_csi_k_clears_from_cursor() {
        let mut console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 5, b'X');
        console.set_cursor(&mut memory, 5, 10);
        feed_str(&mut console, &mut memory, "\x1b[K");
        assert_vram_char(&memory, 5, 9, b'X', 0x00);
        assert_vram_clear(&memory, 5, 10, 79);
    }

    #[test]
    fn esc_csi_1k_clears_to_cursor() {
        let mut console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 5, b'X');
        console.set_cursor(&mut memory, 5, 10);
        feed_str(&mut console, &mut memory, "\x1b[1K");
        assert_vram_clear(&memory, 5, 0, 10);
        assert_vram_char(&memory, 5, 11, b'X', 0x00);
    }

    #[test]
    fn esc_csi_2k_clears_line() {
        let mut console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 5, b'X');
        console.set_cursor(&mut memory, 5, 30);
        feed_str(&mut console, &mut memory, "\x1b[2K");
        assert_vram_clear(&memory, 5, 0, 79);
    }

    #[test]
    fn esc_csi_3k_ignored() {
        let mut console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 5, b'X');
        console.set_cursor(&mut memory, 5, 10);
        feed_str(&mut console, &mut memory, "\x1b[3K");
        // Unknown mode, nothing should change.
        assert_vram_char(&memory, 5, 0, b'X', 0x00);
        assert_vram_char(&memory, 5, 79, b'X', 0x00);
    }

    #[test]
    fn esc_csi_l_scrolls_down() {
        let mut console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        fill_row(&mut memory, 1, b'B');
        feed_str(&mut console, &mut memory, "\x1b[2L");
        // Top 2 rows cleared, original rows shifted down.
        assert_vram_clear(&memory, 0, 0, 79);
        assert_vram_clear(&memory, 1, 0, 79);
        assert_vram_char(&memory, 2, 0, b'A', 0x00);
        assert_vram_char(&memory, 3, 0, b'B', 0x00);
    }

    #[test]
    fn esc_csi_l_default_one() {
        let mut console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        feed_str(&mut console, &mut memory, "\x1b[L");
        assert_vram_clear(&memory, 0, 0, 79);
        assert_vram_char(&memory, 1, 0, b'A', 0x00);
    }

    #[test]
    fn esc_csi_m_scrolls_up() {
        let mut console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        fill_row(&mut memory, 1, b'B');
        fill_row(&mut memory, 2, b'C');
        feed_str(&mut console, &mut memory, "\x1b[2M");
        assert_vram_char(&memory, 0, 0, b'C', 0x00);
        assert_vram_clear(&memory, 23, 0, 79);
        assert_vram_clear(&memory, 24, 0, 79);
    }

    #[test]
    fn esc_csi_m_default_one() {
        let mut console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        fill_row(&mut memory, 1, b'B');
        feed_str(&mut console, &mut memory, "\x1b[M");
        assert_vram_char(&memory, 0, 0, b'B', 0x00);
        assert_vram_clear(&memory, 24, 0, 79);
    }

    #[test]
    fn esc_csi_question_7h_enables_wrap() {
        let mut console = make_console();
        let mut memory = make_memory();
        // Disable first, then re-enable.
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_LINE_WRAP, 0x01);
        feed_str(&mut console, &mut memory, "\x1b[?7h");
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_LINE_WRAP),
            0x00
        );
    }

    #[test]
    fn esc_csi_question_7l_disables_wrap() {
        let mut console = make_console();
        let mut memory = make_memory();
        feed_str(&mut console, &mut memory, "\x1b[?7l");
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_LINE_WRAP),
            0x01
        );
    }

    #[test]
    fn esc_csi_question_7h_then_wrap_works() {
        let mut console = make_console();
        let mut memory = make_memory();
        feed_str(&mut console, &mut memory, "\x1b[?7h");
        console.set_cursor(&mut memory, 0, 79);
        console.put_char(&mut memory, b'X');
        assert_cursor(&console, &memory, 1, 0);
    }

    #[test]
    fn esc_csi_question_7l_then_wrap_blocked() {
        let mut console = make_console();
        let mut memory = make_memory();
        feed_str(&mut console, &mut memory, "\x1b[?7l");
        console.set_cursor(&mut memory, 0, 79);
        console.put_char(&mut memory, b'X');
        assert_cursor(&console, &memory, 0, 79);
    }

    #[test]
    fn esc_csi_greater_1h_hides_fnkey() {
        let mut console = make_console();
        let mut memory = make_memory();
        feed_str(&mut console, &mut memory, "\x1b[>1h");
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_FNKEY_DISPLAY),
            0x00
        );
    }

    #[test]
    fn esc_csi_greater_1l_shows_fnkey() {
        let mut console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_FNKEY_DISPLAY, 0x00);
        feed_str(&mut console, &mut memory, "\x1b[>1l");
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_FNKEY_DISPLAY),
            0x01
        );
    }

    #[test]
    fn esc_csi_greater_3h_20_line_mode() {
        let mut console = make_console();
        let mut memory = make_memory();
        feed_str(&mut console, &mut memory, "\x1b[>3h");
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCREEN_LINES),
            0x00
        );
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_LOWER),
            19
        );
    }

    #[test]
    fn esc_csi_greater_3l_25_line_mode() {
        let mut console = make_console();
        let mut memory = make_memory();
        // Switch to 20-line first, then back.
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCREEN_LINES, 0x00);
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_LOWER, 19);
        feed_str(&mut console, &mut memory, "\x1b[>3l");
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCREEN_LINES),
            0x01
        );
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_LOWER),
            24
        );
    }

    #[test]
    fn esc_csi_greater_5h_hides_cursor() {
        let mut console = make_console();
        let mut memory = make_memory();
        feed_str(&mut console, &mut memory, "\x1b[>5h");
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_VISIBLE),
            0x00
        );
    }

    #[test]
    fn esc_csi_greater_5l_shows_cursor() {
        let mut console = make_console();
        let mut memory = make_memory();
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_VISIBLE, 0x00);
        feed_str(&mut console, &mut memory, "\x1b[>5l");
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_VISIBLE),
            0x01
        );
    }

    #[test]
    fn esc_right_paren_0_kanji_mode() {
        let mut console = make_console();
        let mut memory = make_memory();
        // Set to graphic mode first.
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_MODE, 0x00);
        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_GRAPH_CHAR, 0x67);
        feed_str(&mut console, &mut memory, "\x1b)0");
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_MODE),
            0x01
        );
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_GRAPH_CHAR),
            0x20
        );
    }

    #[test]
    fn esc_right_paren_3_graphic_mode() {
        let mut console = make_console();
        let mut memory = make_memory();
        feed_str(&mut console, &mut memory, "\x1b)3");
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_MODE),
            0x00
        );
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_GRAPH_CHAR),
            0x67
        );
    }

    #[test]
    fn esc_right_paren_unknown_resets() {
        let mut console = make_console();
        let mut memory = make_memory();
        let kanji_before = memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_MODE);
        feed_str(&mut console, &mut memory, "\x1b)X");
        // Kanji mode should be unchanged.
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_MODE),
            kanji_before
        );
        // Parser should be back to Normal.
        assert_eq!(
            console.esc_parser.state,
            crate::console_esc::EscState::Normal
        );
    }

    #[test]
    fn parser_resets_on_unknown_after_esc() {
        let mut console = make_console();
        let mut memory = make_memory();
        // ESC followed by 'X' (not '[' or ')') should reset and output 'X'.
        feed_str(&mut console, &mut memory, "\x1bX");
        assert_vram_char(&memory, 0, 0, b'X', 0x00);
        assert_eq!(
            console.esc_parser.state,
            crate::console_esc::EscState::Normal
        );
    }

    #[test]
    fn esc_star_clears_screen() {
        let mut console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'X');
        console.set_cursor(&mut memory, 5, 10);
        feed_str(&mut console, &mut memory, "\x1b*");
        assert_vram_clear(&memory, 0, 0, 79);
        assert_cursor(&console, &memory, 0, 0);
        assert_eq!(
            console.esc_parser.state,
            crate::console_esc::EscState::Normal
        );
    }

    #[test]
    fn parser_resets_on_unknown_csi() {
        let mut console = make_console();
        let mut memory = make_memory();
        // ESC[ followed by '!' (not digit, ?, >, ;, or letter) should reset.
        feed_str(&mut console, &mut memory, "\x1b[!");
        assert_eq!(
            console.esc_parser.state,
            crate::console_esc::EscState::Normal
        );
    }

    #[test]
    fn parser_multi_param() {
        let mut console = make_console();
        let mut memory = make_memory();
        // ESC[10;20H should set cursor to (9, 19).
        feed_str(&mut console, &mut memory, "\x1b[10;20H");
        assert_cursor(&console, &memory, 9, 19);
    }

    #[test]
    fn csi_question_unknown_param_ignored() {
        let mut console = make_console();
        let mut memory = make_memory();
        let wrap_before = memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_LINE_WRAP);
        feed_str(&mut console, &mut memory, "\x1b[?99h");
        // Unknown param 99, nothing should change.
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_LINE_WRAP),
            wrap_before
        );
    }

    #[test]
    fn csi_greater_unknown_param_ignored() {
        let mut console = make_console();
        let mut memory = make_memory();
        let fnkey_before = memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_FNKEY_DISPLAY);
        feed_str(&mut console, &mut memory, "\x1b[>99h");
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_FNKEY_DISPLAY),
            fnkey_before
        );
    }

    #[test]
    fn hello_world() {
        let mut console = make_console();
        let mut memory = make_memory();
        feed_str(&mut console, &mut memory, "Hello, World!\r\n");
        assert_vram_char(&memory, 0, 0, b'H', 0x00);
        assert_vram_char(&memory, 0, 1, b'e', 0x00);
        assert_vram_char(&memory, 0, 4, b'o', 0x00);
        assert_vram_char(&memory, 0, 5, b',', 0x00);
        assert_vram_char(&memory, 0, 7, b'W', 0x00);
        assert_vram_char(&memory, 0, 12, b'!', 0x00);
        assert_cursor(&console, &memory, 1, 0);
    }

    #[test]
    fn clear_and_rewrite() {
        let mut console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'X');
        feed_str(&mut console, &mut memory, "\x1b[2JTest");
        assert_vram_char(&memory, 0, 0, b'T', 0x00);
        assert_vram_char(&memory, 0, 1, b'e', 0x00);
        assert_vram_char(&memory, 0, 2, b's', 0x00);
        assert_vram_char(&memory, 0, 3, b't', 0x00);
        // Rest of row should be cleared.
        assert_vram_clear(&memory, 0, 4, 79);
    }

    #[test]
    fn save_restore_round_trip() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 5, 10);
        feed_str(&mut console, &mut memory, "\x1b[s");
        feed_str(&mut console, &mut memory, "\x1b[20;60H");
        assert_cursor(&console, &memory, 19, 59);
        feed_str(&mut console, &mut memory, "\x1b[u");
        assert_cursor(&console, &memory, 5, 10);
    }

    #[test]
    fn set_cursor_and_write() {
        let mut console = make_console();
        let mut memory = make_memory();
        feed_str(&mut console, &mut memory, "\x1b[10;40HX");
        assert_vram_char(&memory, 9, 39, b'X', 0x00);
        assert_cursor(&console, &memory, 9, 40);
    }

    #[test]
    fn wrap_and_scroll() {
        let mut console = make_console();
        let mut memory = make_memory();
        fill_row(&mut memory, 0, b'A');
        // Position cursor at end of last row.
        console.set_cursor(&mut memory, 24, 79);
        // Write a character - should trigger wrap + scroll.
        console.process_byte(&mut memory, b'Z');
        assert_cursor(&console, &memory, 24, 0);
        // Row 0 should have been scrolled away (was 'A', now has what was row 1).
        let (even, _) = read_vram_char(&memory, 0, 0);
        assert_ne!(even, b'A', "row 0 should have been scrolled away");
    }

    #[test]
    fn esc_equal_cursor_position() {
        let mut console = make_console();
        let mut memory = make_memory();
        // ESC = 0x25 0x2A -> row 5, col 10
        feed_str(&mut console, &mut memory, "\x1b=\x25\x2A");
        assert_cursor(&console, &memory, 5, 10);
    }

    #[test]
    fn esc_equal_origin() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 10, 20);
        // ESC = 0x20 0x20 -> row 0, col 0
        feed_str(&mut console, &mut memory, "\x1b=\x20\x20");
        assert_cursor(&console, &memory, 0, 0);
    }

    #[test]
    fn esc_equal_clamp_below_0x20() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.set_cursor(&mut memory, 10, 20);
        // Both bytes < 0x20: should clamp to (0, 0).
        feed_str(&mut console, &mut memory, "\x1b=\x10\x10");
        assert_cursor(&console, &memory, 0, 0);
    }

    #[test]
    fn esc_equal_clamp_to_screen() {
        let mut console = make_console();
        let mut memory = make_memory();
        // ESC = 0x7F 0x7F -> row 95, col 95: clamped to (24, 79).
        feed_str(&mut console, &mut memory, "\x1b=\x7F\x7F");
        assert_cursor(&console, &memory, 24, 79);
    }

    #[test]
    fn esc_equal_preserves_content() {
        let mut console = make_console();
        let mut memory = make_memory();
        console.process_byte(&mut memory, b'A');
        // ESC = to row 2 col 5.
        feed_str(&mut console, &mut memory, "\x1b=\x22\x25");
        assert_cursor(&console, &memory, 2, 5);
        let (even, _) = read_vram_char(&memory, 0, 0);
        assert_eq!(even, b'A', "'A' at (0,0) should be preserved");
    }
}
