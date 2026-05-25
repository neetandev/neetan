//! Native ESC sequence state machine and processing.

use common::{is_shift_jis_lead_byte, is_shift_jis_trail_byte, shift_jis_pair_to_jis};

use crate::{MemoryAccess, console::Console, tables};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum EscState {
    #[default]
    Normal,
    GotEsc,
    GotCsi,
    GotCsiQuestion,
    GotCsiGreater,
    GotEscRightParen,
    GotEscEqual,
    GotEscEqualRow,
}

#[derive(Clone, Debug)]
pub(crate) struct EscParser {
    pub state: EscState,
    pub params: [u16; 8],
    pub param_count: usize,
    pub current_param: u16,
    pub has_digit: bool,
}

impl Default for EscParser {
    fn default() -> Self {
        Self {
            state: EscState::Normal,
            params: [0; 8],
            param_count: 0,
            current_param: 0,
            has_digit: false,
        }
    }
}

impl EscParser {
    fn reset(&mut self) {
        self.state = EscState::Normal;
        self.param_count = 0;
        self.current_param = 0;
        self.has_digit = false;
    }

    fn push_param(&mut self) {
        if self.param_count < self.params.len() {
            self.params[self.param_count] = self.current_param;
            self.param_count += 1;
        }
        self.current_param = 0;
        self.has_digit = false;
    }

    fn param(&self, index: usize, default: u16) -> u16 {
        if index < self.param_count && self.params[index] != 0 {
            self.params[index]
        } else {
            default
        }
    }
}

impl Console {
    /// Main entry point: feed one byte into the console output pipeline.
    /// Handles control characters, ESC sequences, and printable output.
    pub(crate) fn process_byte(&mut self, memory: &mut dyn MemoryAccess, byte: u8) {
        if let Some(lead) = self.pending_shift_jis_lead(memory) {
            self.clear_pending_shift_jis_lead(memory);
            if let Some(jis) = if is_shift_jis_trail_byte(byte) {
                shift_jis_pair_to_jis(lead, byte)
            } else {
                None
            } {
                self.put_fullwidth_jis_char(memory, jis);
                return;
            }

            self.put_char(memory, lead);
            self.process_byte(memory, byte);
            return;
        }

        if self.esc_parser.state == EscState::Normal && is_shift_jis_lead_byte(byte) {
            self.set_pending_shift_jis_lead(memory, byte);
            return;
        }

        match self.esc_parser.state {
            EscState::Normal => self.esc_process_normal(memory, byte),
            EscState::GotEsc => self.esc_process_got_esc(memory, byte),
            EscState::GotCsi => self.esc_process_csi(memory, byte),
            EscState::GotCsiQuestion => self.esc_process_csi_param(memory, byte, true),
            EscState::GotCsiGreater => self.esc_process_csi_param(memory, byte, false),
            EscState::GotEscRightParen => self.esc_process_right_paren(memory, byte),
            EscState::GotEscEqual => self.esc_process_equal_row(byte),
            EscState::GotEscEqualRow => self.esc_process_equal_col(memory, byte),
        }
    }

    fn esc_process_normal(&mut self, memory: &mut dyn MemoryAccess, byte: u8) {
        match byte {
            0x07 => {} // BEL: no-op
            0x08 => self.backspace(memory),
            0x09 => self.tab(memory),
            0x0A => self.linefeed(memory),
            0x0D => self.carriage_return(memory),
            0x1B => {
                self.esc_parser.state = EscState::GotEsc;
                self.esc_parser.param_count = 0;
                self.esc_parser.current_param = 0;
                self.esc_parser.has_digit = false;
            }
            _ => self.put_char(memory, byte),
        }
    }

    fn esc_process_got_esc(&mut self, memory: &mut dyn MemoryAccess, byte: u8) {
        match byte {
            b'[' => {
                self.esc_parser.state = EscState::GotCsi;
            }
            b'*' => {
                self.clear_screen(memory);
                self.esc_parser.reset();
            }
            b'D' => {
                self.linefeed(memory);
                self.esc_parser.reset();
            }
            b'E' => {
                self.carriage_return(memory);
                self.linefeed(memory);
                self.esc_parser.reset();
            }
            b'M' => {
                self.reverse_linefeed(memory);
                self.esc_parser.reset();
            }
            b')' => {
                self.esc_parser.state = EscState::GotEscRightParen;
            }
            b'=' => {
                self.esc_parser.state = EscState::GotEscEqual;
            }
            _ => {
                // Unknown ESC sequence: output the byte as a literal.
                self.esc_parser.reset();
                self.put_char(memory, byte);
            }
        }
    }

    fn esc_process_csi(&mut self, memory: &mut dyn MemoryAccess, byte: u8) {
        match byte {
            b'?' => {
                self.esc_parser.state = EscState::GotCsiQuestion;
            }
            b'>' => {
                self.esc_parser.state = EscState::GotCsiGreater;
            }
            b'0'..=b'9' => {
                self.esc_parser.current_param =
                    self.esc_parser.current_param * 10 + (byte - b'0') as u16;
                self.esc_parser.has_digit = true;
            }
            b';' => {
                self.esc_parser.push_param();
            }
            b'A'..=b'z' => {
                // Final byte: push pending param and dispatch.
                if self.esc_parser.has_digit || self.esc_parser.param_count > 0 {
                    self.esc_parser.push_param();
                }
                self.esc_dispatch_csi(memory, byte);
                self.esc_parser.reset();
            }
            _ => {
                self.esc_parser.reset();
            }
        }
    }

    /// Shared parameter accumulation for CSI? and CSI> sequences.
    fn esc_process_csi_param(
        &mut self,
        memory: &mut dyn MemoryAccess,
        byte: u8,
        is_question: bool,
    ) {
        match byte {
            b'0'..=b'9' => {
                self.esc_parser.current_param =
                    self.esc_parser.current_param * 10 + (byte - b'0') as u16;
                self.esc_parser.has_digit = true;
            }
            b';' => {
                self.esc_parser.push_param();
            }
            b'h' | b'l' => {
                if self.esc_parser.has_digit || self.esc_parser.param_count > 0 {
                    self.esc_parser.push_param();
                }
                if is_question {
                    self.esc_dispatch_csi_question(memory, byte);
                } else {
                    self.esc_dispatch_csi_greater(memory, byte);
                }
                self.esc_parser.reset();
            }
            _ => {
                self.esc_parser.reset();
            }
        }
    }

    fn esc_process_right_paren(&mut self, memory: &mut dyn MemoryAccess, byte: u8) {
        match byte {
            b'0' => {
                // Set Shift-JIS kanji display mode.
                memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_MODE, 0x01);
                memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_GRAPH_CHAR, 0x20);
            }
            b'3' => {
                // Set graphic character display mode.
                memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_MODE, 0x00);
                memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_GRAPH_CHAR, 0x67);
            }
            _ => {}
        }
        self.esc_parser.reset();
    }

    fn esc_process_equal_row(&mut self, byte: u8) {
        self.esc_parser.params[0] = byte as u16;
        self.esc_parser.state = EscState::GotEscEqualRow;
    }

    fn esc_process_equal_col(&mut self, memory: &mut dyn MemoryAccess, byte: u8) {
        let raw_row = self.esc_parser.params[0] as u8;
        let raw_col = byte;
        let row = raw_row.saturating_sub(0x20);
        let col = raw_col.saturating_sub(0x20);
        self.set_cursor_position(memory, row, col);
        self.esc_parser.reset();
    }

    fn esc_dispatch_csi(&mut self, memory: &mut dyn MemoryAccess, final_byte: u8) {
        match final_byte {
            b'H' | b'f' => {
                // ESC[row;colH - set cursor position (1-based).
                let row = self.esc_parser.param(0, 1).saturating_sub(1) as u8;
                let col = self.esc_parser.param(1, 1).saturating_sub(1) as u8;
                self.set_cursor_position(memory, row, col);
            }
            b'A' => {
                let count = self.esc_parser.param(0, 1).max(1) as u8;
                self.cursor_up(memory, count);
            }
            b'B' => {
                let count = self.esc_parser.param(0, 1).max(1) as u8;
                self.cursor_down(memory, count);
            }
            b'C' => {
                let count = self.esc_parser.param(0, 1).max(1) as u8;
                self.cursor_right(memory, count);
            }
            b'D' => {
                let count = self.esc_parser.param(0, 1).max(1) as u8;
                self.cursor_left(memory, count);
            }
            b's' => {
                self.save_cursor(memory);
            }
            b'u' => {
                self.restore_cursor(memory);
            }
            b'J' => {
                let mode = self.esc_parser.param(0, 0);
                match mode {
                    0 => self.clear_screen_from_cursor(memory),
                    1 => self.clear_screen_to_cursor(memory),
                    2 => self.clear_screen(memory),
                    _ => {}
                }
            }
            b'K' => {
                let mode = self.esc_parser.param(0, 0);
                match mode {
                    0 => self.clear_line_from_cursor(memory),
                    1 => self.clear_line_to_cursor(memory),
                    2 => self.clear_line(memory),
                    _ => {}
                }
            }
            b'L' => {
                let count = self.esc_parser.param(0, 1).max(1) as u8;
                self.scroll_down(memory, count);
            }
            b'M' => {
                let count = self.esc_parser.param(0, 1).max(1) as u8;
                self.scroll_up(memory, count);
            }
            _ => {} // Unknown CSI sequence: ignore.
        }
    }

    fn esc_dispatch_csi_question(&self, memory: &mut dyn MemoryAccess, final_byte: u8) {
        let param = self.esc_parser.param(0, 0);
        let set = final_byte == b'h';
        if param == 7 {
            // ESC[?7h = enable wrap, ESC[?7l = disable wrap.
            // 0x00 = wrap enabled, 0x01 = wrap disabled.
            let value = if set { 0x00 } else { 0x01 };
            memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_LINE_WRAP, value);
        }
    }

    fn esc_dispatch_csi_greater(&self, memory: &mut dyn MemoryAccess, final_byte: u8) {
        let param = self.esc_parser.param(0, 0);
        let set = final_byte == b'h';
        match param {
            1 => {
                // ESC[>1h = hide function key display, ESC[>1l = show.
                let value = if set { 0x00 } else { 0x01 };
                memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_FNKEY_DISPLAY, value);
            }
            3 => {
                // ESC[>3h = 20-line mode, ESC[>3l = 25-line mode.
                let value = if set { 0x00 } else { 0x01 };
                memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCREEN_LINES, value);
                if set {
                    memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_LOWER, 19);
                } else {
                    memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SCROLL_LOWER, 24);
                }
            }
            5 => {
                // ESC[>5h = hide cursor, ESC[>5l = show cursor.
                let value = if set { 0x00 } else { 0x01 };
                memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_VISIBLE, value);
            }
            _ => {}
        }
    }
}
