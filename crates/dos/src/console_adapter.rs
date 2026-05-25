use common::{ConsoleIo, JisChar};

use crate::{MemoryAccess, console::Console, tables};

pub(crate) struct ConsoleAdapter<'a> {
    console: &'a mut Console,
    memory: &'a mut dyn MemoryAccess,
}

impl<'a> ConsoleAdapter<'a> {
    pub(crate) fn new(console: &'a mut Console, memory: &'a mut dyn MemoryAccess) -> Self {
        Self { console, memory }
    }
}

impl ConsoleIo for ConsoleAdapter<'_> {
    fn write_char(&mut self, ch: u8) {
        self.console.put_char(self.memory, ch);
    }

    fn write_str(&mut self, s: &[u8]) {
        for &byte in s {
            self.console.process_byte(self.memory, byte);
        }
    }

    fn write_jis_char(&mut self, row: u8, col: u8, ch: JisChar, attr: u8) {
        self.console.write_cell(self.memory, row, col, ch, attr);
        if ch.display_width() == 2 && col < 79 {
            self.console
                .write_cell(self.memory, row, col + 1, JisChar::from_u16(0x0000), attr);
        }
    }

    fn write_jis(&mut self, row: u8, col: u8, s: &[JisChar], attr: u8) -> u8 {
        let mut current_col = col;
        for &ch in s {
            let width = ch.display_width();
            if width == 1 {
                if current_col >= 80 {
                    break;
                }
            } else {
                if current_col >= 79 {
                    break;
                }
            }
            self.write_jis_char(row, current_col, ch, attr);
            current_col += width;
        }
        current_col
    }

    fn write_ank_at(&mut self, row: u8, col: u8, s: &[u8], attr: u8) -> u8 {
        let mut current_col = col;
        for &byte in s {
            if current_col >= 80 {
                break;
            }
            self.write_jis_char(row, current_col, JisChar::from_u16(byte as u16), attr);
            current_col += 1;
        }
        current_col
    }

    fn fill_region(&mut self, top: u8, left: u8, height: u8, width: u8, ch: JisChar, attr: u8) {
        let row_end = top.saturating_add(height).min(25);
        let col_end = left.saturating_add(width).min(80);
        for row in top..row_end {
            let mut col = left;
            while col < col_end {
                self.write_jis_char(row, col, ch, attr);
                col += ch.display_width();
            }
        }
    }

    fn read_char(&mut self) -> u8 {
        if tables::key_available(self.memory) {
            let (_, ch) = tables::read_key(self.memory);
            ch
        } else {
            0
        }
    }

    fn char_available(&self) -> bool {
        tables::key_available(self.memory)
    }

    fn read_key(&mut self) -> (u8, u8) {
        tables::read_key(self.memory)
    }

    fn cursor_position(&self) -> (u8, u8) {
        (
            self.console.cursor_row(self.memory),
            self.console.cursor_col(self.memory),
        )
    }

    fn set_cursor_position(&mut self, row: u8, col: u8) {
        self.console.set_cursor_position(self.memory, row, col);
    }

    fn scroll_up(&mut self) {
        self.console.scroll_up(self.memory, 1);
    }

    fn clear_screen(&mut self) {
        self.console.clear_screen(self.memory);
    }

    fn set_cursor_visible(&mut self, visible: bool) {
        self.console.set_cursor_visible(self.memory, visible);
    }

    fn screen_size(&self) -> (u8, u8) {
        self.console.screen_size(self.memory)
    }
}
