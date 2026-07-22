//! Native ESC sequence state machine and processing.

use alloc::{string::ToString, vec::Vec};

use common::{is_shift_jis_lead_byte, is_shift_jis_trail_byte, shift_jis_pair_to_jis};

use crate::{
    MemoryAccess,
    console::Console,
    tables,
    trace::{
        DosConsoleByteEvent, DosConsoleEscapeEvent, DosTraceEvent, character_mode,
        parser_state_symbol,
    },
};

/// Escape-relevant console state captured ahead of a dispatched sequence.
struct EscapeTraceBefore {
    attribute: u8,
    cursor_row: u8,
    cursor_column: u8,
}

/// Console parser and screen state captured before and after one byte.
struct ConsoleByteState {
    parser_state: &'static str,
    character_mode: &'static str,
    pending_lead: Option<u8>,
    cursor_row: u8,
    cursor_column: u8,
    attribute: u8,
}

/// Maps a CSI final byte to its stable command symbol.
fn csi_command_name(final_byte: u8) -> &'static str {
    match final_byte {
        b'H' | b'f' => "cursor-position",
        b'A' => "cursor-up",
        b'B' => "cursor-down",
        b'C' => "cursor-right",
        b'D' => "cursor-left",
        b's' => "save-cursor",
        b'u' => "restore-cursor",
        b'J' => "erase-display",
        b'K' => "erase-line",
        b'L' => "insert-line",
        b'M' => "delete-line",
        b'm' => "sgr",
        _ => "unknown",
    }
}

save_state::runtime_state_enum! {
    /// Current phase of the DOS console escape-sequence parser.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) enum EscState {
        #[default]
        Normal = 0,
        GotEsc = 1,
        GotCsi = 2,
        GotCsiQuestion = 3,
        GotCsiGreater = 4,
        GotEscRightParen = 5,
        GotEscEqual = 6,
        GotEscEqualRow = 7,
    }
}

#[derive(Clone, Debug)]
/// Authoritative parameters and phase of the console escape parser.
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

state_struct_codec!(EscParser {
    state,
    params,
    param_count,
    current_param,
    has_digit,
});

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
    fn shift_jis_mode_enabled(&self, memory: &dyn MemoryAccess) -> bool {
        memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_MODE) != 0
    }

    fn capture_byte_state(&self, memory: &dyn MemoryAccess) -> ConsoleByteState {
        ConsoleByteState {
            parser_state: parser_state_symbol(self.esc_parser.state),
            character_mode: if self.shift_jis_mode_enabled(memory) {
                character_mode::SHIFT_JIS
            } else {
                character_mode::ANK
            },
            pending_lead: self.pending_shift_jis_lead(memory),
            cursor_row: self.cursor_row(memory),
            cursor_column: self.cursor_col(memory),
            attribute: memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_DISPLAY_ATTR),
        }
    }

    /// Main entry point: feed one byte into the console output pipeline.
    /// Handles control characters, ESC sequences, and printable output.
    ///
    /// When the trace log is armed, this records a `byte` event with before and
    /// after parser, cursor, and attribute state around the inner processing.
    pub(crate) fn process_byte(&mut self, memory: &mut dyn MemoryAccess, byte: u8) {
        if !self.dos_trace.console_byte_enabled.get() {
            self.process_byte_inner(memory, byte);
            return;
        }
        let before = self.capture_byte_state(memory);
        let index = {
            let mut events = self.dos_trace.events.borrow_mut();
            let index = events.len();
            events.push(DosTraceEvent::ConsoleByte(DosConsoleByteEvent {
                byte,
                parser_state_before: before.parser_state,
                parser_state_after: before.parser_state,
                character_mode_before: before.character_mode,
                character_mode_after: before.character_mode,
                pending_shift_jis_lead_before: before.pending_lead,
                pending_shift_jis_lead_after: before.pending_lead,
                cursor_row_before: before.cursor_row,
                cursor_column_before: before.cursor_column,
                cursor_row_after: before.cursor_row,
                cursor_column_after: before.cursor_column,
                attribute_before: before.attribute,
                attribute_after: before.attribute,
            }));
            index
        };
        self.process_byte_inner(memory, byte);
        let after = self.capture_byte_state(memory);
        if let Some(DosTraceEvent::ConsoleByte(event)) =
            self.dos_trace.events.borrow_mut().get_mut(index)
        {
            event.parser_state_after = after.parser_state;
            event.character_mode_after = after.character_mode;
            event.pending_shift_jis_lead_after = after.pending_lead;
            event.cursor_row_after = after.cursor_row;
            event.cursor_column_after = after.cursor_column;
            event.attribute_after = after.attribute;
        }
    }

    fn process_byte_inner(&mut self, memory: &mut dyn MemoryAccess, byte: u8) {
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
            self.process_byte_inner(memory, byte);
            return;
        }

        let shift_jis_mode = self.shift_jis_mode_enabled(memory);
        if self.esc_parser.state == EscState::Normal
            && is_shift_jis_lead_byte(byte)
            && (shift_jis_mode || byte == 0x86)
        {
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
                let before = self.escape_trace_begin(memory);
                self.clear_screen(memory);
                self.escape_trace_end(memory, before, &[0x1B, b'*'], "clear-screen", &[]);
                self.esc_parser.reset();
            }
            b'D' => {
                let before = self.escape_trace_begin(memory);
                self.linefeed(memory);
                self.escape_trace_end(memory, before, &[0x1B, b'D'], "line-feed", &[]);
                self.esc_parser.reset();
            }
            b'E' => {
                let before = self.escape_trace_begin(memory);
                self.carriage_return(memory);
                self.linefeed(memory);
                self.escape_trace_end(memory, before, &[0x1B, b'E'], "next-line", &[]);
                self.esc_parser.reset();
            }
            b'M' => {
                let before = self.escape_trace_begin(memory);
                self.reverse_linefeed(memory);
                self.escape_trace_end(memory, before, &[0x1B, b'M'], "reverse-line-feed", &[]);
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
                self.dispatch_csi_traced(memory, byte);
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
                let before = self.escape_trace_begin(memory);
                let prefix: &[u8] = if is_question { b"?" } else { b">" };
                let command = match (is_question, byte) {
                    (true, b'h') => "set-mode",
                    (true, _) => "reset-mode",
                    (false, b'h') => "set-extended-mode",
                    (false, _) => "reset-extended-mode",
                };
                if is_question {
                    self.esc_dispatch_csi_question(memory, byte);
                } else {
                    self.esc_dispatch_csi_greater(memory, byte);
                }
                if before.is_some() {
                    let bytes = self.build_csi_bytes(prefix, byte);
                    let parameters: Vec<u16> =
                        self.esc_parser.params[..self.esc_parser.param_count].to_vec();
                    self.escape_trace_end(memory, before, &bytes, command, &parameters);
                }
                self.esc_parser.reset();
            }
            _ => {
                self.esc_parser.reset();
            }
        }
    }

    fn esc_process_right_paren(&mut self, memory: &mut dyn MemoryAccess, byte: u8) {
        let before = self.escape_trace_begin(memory);
        let command = match byte {
            b'0' => {
                // Set Shift-JIS kanji display mode.
                memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_MODE, 0x01);
                memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_GRAPH_CHAR, 0x20);
                "kanji-mode"
            }
            b'3' => {
                // Set graphic character display mode.
                memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_MODE, 0x00);
                memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_GRAPH_CHAR, 0x67);
                "graphic-mode"
            }
            _ => "unknown",
        };
        self.escape_trace_end(memory, before, &[0x1B, b')', byte], command, &[]);
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
        let before = self.escape_trace_begin(memory);
        self.set_cursor_position(memory, row, col);
        self.escape_trace_end(
            memory,
            before,
            &[0x1B, b'=', raw_row, raw_col],
            "cursor-address",
            &[u16::from(row), u16::from(col)],
        );
        self.esc_parser.reset();
    }

    /// Builds the canonical byte form of the pending CSI sequence.
    ///
    /// `prefix` carries the private-parameter marker for `CSI ?` and `CSI >`
    /// sequences, or is empty for a plain CSI sequence.
    fn build_csi_bytes(&self, prefix: &[u8], final_byte: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(0x1B);
        bytes.push(b'[');
        bytes.extend_from_slice(prefix);
        for index in 0..self.esc_parser.param_count {
            if index > 0 {
                bytes.push(b';');
            }
            bytes.extend_from_slice(self.esc_parser.params[index].to_string().as_bytes());
        }
        bytes.push(final_byte);
        bytes
    }

    /// Captures the escape-relevant state ahead of a dispatch when the escape
    /// action is armed, or returns `None` so tracing costs nothing.
    fn escape_trace_begin(&self, memory: &dyn MemoryAccess) -> Option<EscapeTraceBefore> {
        if !self.dos_trace.console_escape_enabled.get() {
            return None;
        }
        Some(EscapeTraceBefore {
            attribute: memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_DISPLAY_ATTR),
            cursor_row: self.cursor_row(memory),
            cursor_column: self.cursor_col(memory),
        })
    }

    /// Records an `escape` event for a dispatched sequence when armed.
    fn escape_trace_end(
        &self,
        memory: &dyn MemoryAccess,
        before: Option<EscapeTraceBefore>,
        bytes: &[u8],
        command: &'static str,
        parameters: &[u16],
    ) {
        let Some(before) = before else {
            return;
        };
        let event = DosConsoleEscapeEvent {
            bytes: bytes.to_vec(),
            command,
            parameters: parameters.to_vec(),
            attribute_before: before.attribute,
            attribute_after: memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_DISPLAY_ATTR),
            cursor_row_before: before.cursor_row,
            cursor_column_before: before.cursor_column,
            cursor_row_after: self.cursor_row(memory),
            cursor_column_after: self.cursor_col(memory),
        };
        self.dos_trace
            .events
            .borrow_mut()
            .push(DosTraceEvent::ConsoleEscape(event));
    }

    /// Dispatches a CSI final byte, recording an `escape` event when armed.
    fn dispatch_csi_traced(&mut self, memory: &mut dyn MemoryAccess, final_byte: u8) {
        let before = self.escape_trace_begin(memory);
        if before.is_none() {
            self.esc_dispatch_csi(memory, final_byte);
            return;
        }
        let bytes = self.build_csi_bytes(&[], final_byte);
        let parameters: Vec<u16> = self.esc_parser.params[..self.esc_parser.param_count].to_vec();
        self.esc_dispatch_csi(memory, final_byte);
        self.escape_trace_end(
            memory,
            before,
            &bytes,
            csi_command_name(final_byte),
            &parameters,
        );
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
            b'm' => {
                self.set_graphic_rendition(memory);
            }
            _ => {} // Unknown CSI sequence: ignore.
        }
    }

    fn set_graphic_rendition(&self, memory: &mut dyn MemoryAccess) {
        if self.esc_parser.param_count == 0 {
            self.set_attribute(memory, 0xE1);
            return;
        }

        for index in 0..self.esc_parser.param_count {
            let parameter = self.esc_parser.params[index];
            match parameter {
                0 | 39 => self.set_attribute(memory, 0xE1),
                4 => self.update_attribute(memory, 0x08, 0x00),
                5 => self.update_attribute(memory, 0x02, 0x00),
                7 => self.update_attribute(memory, 0x04, 0x00),
                8 => self.update_attribute(memory, 0x00, 0x01),
                24 => self.update_attribute(memory, 0x00, 0x08),
                25 => self.update_attribute(memory, 0x00, 0x02),
                27 => self.update_attribute(memory, 0x00, 0x04),
                28 => self.update_attribute(memory, 0x01, 0x00),
                30..=37 => {
                    let color = ansi_foreground_to_pc98_color(parameter as u8);
                    let lower_bits = memory
                        .read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_DISPLAY_ATTR)
                        & 0x1F;
                    self.set_attribute(memory, (color << 5) | lower_bits);
                }
                40..=47 => {
                    let color = ansi_foreground_to_pc98_color(parameter as u8 - 10);
                    let lower_bits = memory
                        .read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_DISPLAY_ATTR)
                        & 0x1F;
                    self.set_attribute(memory, (color << 5) | lower_bits | 0x04);
                }
                _ => {}
            }
        }
    }

    fn update_attribute(&self, memory: &mut dyn MemoryAccess, set: u8, clear: u8) {
        let attribute = memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_DISPLAY_ATTR);
        self.set_attribute(memory, (attribute | set) & !clear);
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

fn ansi_foreground_to_pc98_color(code: u8) -> u8 {
    match code {
        30 => 0,
        31 => 2,
        32 => 4,
        33 => 6,
        34 => 1,
        35 => 3,
        36 => 5,
        37 => 7,
        _ => 7,
    }
}
