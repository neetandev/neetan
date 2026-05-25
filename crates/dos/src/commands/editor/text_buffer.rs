use common::{
    JisChar, is_shift_jis_lead_byte, is_shift_jis_trail_byte, jis_to_shift_jis,
    shift_jis_pair_to_jis,
};

pub(crate) const PAGE_ROWS: usize = 19;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextUnit {
    Char(JisChar),
    Raw(u8),
}

impl TextUnit {
    pub(crate) fn display_width(self) -> usize {
        match self {
            Self::Char(ch) => ch.display_width() as usize,
            Self::Raw(_) => 1,
        }
    }

    pub(crate) fn display_char(self) -> JisChar {
        match self {
            Self::Char(ch) if ch.as_u16() >= 0x20 || ch.as_u16() >= 0xA1 => ch,
            Self::Char(ch) => ch,
            Self::Raw(0x1A) => JisChar::from_u16(0x1A),
            Self::Raw(_) => JisChar::from_u16(b'?' as u16),
        }
    }

    pub(crate) fn write_encoded_bytes(self, output: &mut Vec<u8>) {
        match self {
            Self::Char(ch) => {
                if let Some(encoded) = jis_to_shift_jis(ch) {
                    encoded.write_to(output);
                } else {
                    output.push(b'?');
                }
            }
            Self::Raw(byte) => output.push(byte),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Line {
    pub(crate) units: Vec<TextUnit>,
}

#[derive(Debug, Clone)]
pub(crate) struct TextBuffer {
    pub(crate) path: Vec<u8>,
    pub(crate) lines: Vec<Line>,
    pub(crate) cursor_line: usize,
    pub(crate) cursor_index: usize,
    pub(crate) viewport_top: usize,
    pub(crate) horizontal_scroll: usize,
    pub(crate) insert_mode: bool,
    pub(crate) modified: bool,
    pub(crate) read_only: bool,
    pending_lead: Option<u8>,
}

impl TextBuffer {
    pub(crate) fn new(path: Vec<u8>, read_only: bool) -> Self {
        Self {
            path,
            lines: vec![Line { units: Vec::new() }],
            cursor_line: 0,
            cursor_index: 0,
            viewport_top: 0,
            horizontal_scroll: 0,
            insert_mode: true,
            modified: false,
            read_only,
            pending_lead: None,
        }
    }

    pub(crate) fn from_bytes(path: Vec<u8>, bytes: &[u8], read_only: bool) -> Self {
        let mut lines = Vec::new();
        let mut current = Vec::new();
        let mut index = 0usize;
        while index < bytes.len() {
            let byte = bytes[index];
            if byte == b'\r' {
                if bytes.get(index + 1) == Some(&b'\n') {
                    index += 1;
                }
                lines.push(Line { units: current });
                current = Vec::new();
                index += 1;
                continue;
            }
            if byte == b'\n' {
                lines.push(Line { units: current });
                current = Vec::new();
                index += 1;
                continue;
            }
            if is_shift_jis_lead_byte(byte)
                && let Some(&trail) = bytes.get(index + 1)
                && is_shift_jis_trail_byte(trail)
                && let Some(jis) = shift_jis_pair_to_jis(byte, trail)
            {
                current.push(TextUnit::Char(jis));
                index += 2;
                continue;
            }
            if byte >= 0x20 || byte >= 0xA1 {
                current.push(TextUnit::Char(JisChar::from_u16(byte as u16)));
            } else {
                current.push(TextUnit::Raw(byte));
            }
            index += 1;
        }

        lines.push(Line { units: current });

        Self {
            path,
            lines,
            cursor_line: 0,
            cursor_index: 0,
            viewport_top: 0,
            horizontal_scroll: 0,
            insert_mode: true,
            modified: false,
            read_only,
            pending_lead: None,
        }
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            for &unit in &line.units {
                unit.write_encoded_bytes(&mut bytes);
            }
            if index + 1 < self.lines.len() {
                bytes.extend_from_slice(b"\r\n");
            }
        }
        bytes
    }

    pub(crate) fn set_path(&mut self, path: Vec<u8>) {
        self.path = path;
    }

    pub(crate) fn clear_modified(&mut self) {
        self.modified = false;
    }

    pub(crate) fn visual_cursor_col(&self) -> usize {
        self.visual_width_for_index(self.cursor_line, self.cursor_index)
    }

    pub(crate) fn move_left(&mut self) {
        self.pending_lead = None;
        if self.cursor_index > 0 {
            self.cursor_index -= 1;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_index = self.lines[self.cursor_line].units.len();
        }
        self.ensure_cursor_visible();
    }

    pub(crate) fn move_right(&mut self) {
        self.pending_lead = None;
        if self.cursor_index < self.lines[self.cursor_line].units.len() {
            self.cursor_index += 1;
        } else if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_index = 0;
        }
        self.ensure_cursor_visible();
    }

    pub(crate) fn move_up(&mut self) {
        self.pending_lead = None;
        if self.cursor_line > 0 {
            let visual = self.visual_cursor_col();
            self.cursor_line -= 1;
            self.cursor_index = self.index_for_visual_col(self.cursor_line, visual);
        }
        self.ensure_cursor_visible();
    }

    pub(crate) fn move_down(&mut self) {
        self.pending_lead = None;
        if self.cursor_line + 1 < self.lines.len() {
            let visual = self.visual_cursor_col();
            self.cursor_line += 1;
            self.cursor_index = self.index_for_visual_col(self.cursor_line, visual);
        }
        self.ensure_cursor_visible();
    }

    pub(crate) fn page_up(&mut self) {
        let visual = self.visual_cursor_col();
        self.cursor_line = self.cursor_line.saturating_sub(PAGE_ROWS);
        self.cursor_index = self.index_for_visual_col(self.cursor_line, visual);
        self.ensure_cursor_visible();
    }

    pub(crate) fn page_down(&mut self) {
        let visual = self.visual_cursor_col();
        self.cursor_line = (self.cursor_line + PAGE_ROWS).min(self.lines.len().saturating_sub(1));
        self.cursor_index = self.index_for_visual_col(self.cursor_line, visual);
        self.ensure_cursor_visible();
    }

    pub(crate) fn move_home(&mut self) {
        self.pending_lead = None;
        self.cursor_index = 0;
        self.ensure_cursor_visible();
    }

    pub(crate) fn move_end(&mut self) {
        self.pending_lead = None;
        self.cursor_index = self.lines[self.cursor_line].units.len();
        self.ensure_cursor_visible();
    }

    pub(crate) fn toggle_insert_mode(&mut self) {
        self.insert_mode = !self.insert_mode;
    }

    pub(crate) fn insert_tab(&mut self) {
        self.insert_unit(TextUnit::Raw(b'\t'));
    }

    pub(crate) fn insert_input_byte(&mut self, byte: u8) {
        if let Some(lead) = self.pending_lead.take() {
            if is_shift_jis_trail_byte(byte)
                && let Some(jis) = shift_jis_pair_to_jis(lead, byte)
            {
                self.insert_unit(TextUnit::Char(jis));
                return;
            }
            self.insert_unit(TextUnit::Raw(lead));
            self.insert_input_byte(byte);
            return;
        }

        if is_shift_jis_lead_byte(byte) {
            self.pending_lead = Some(byte);
            return;
        }

        if byte >= 0x20 || byte >= 0xA1 {
            self.insert_unit(TextUnit::Char(JisChar::from_u16(byte as u16)));
        } else {
            self.insert_unit(TextUnit::Raw(byte));
        }
    }

    pub(crate) fn flush_pending_input(&mut self) {
        if let Some(lead) = self.pending_lead.take() {
            self.insert_unit(TextUnit::Raw(lead));
        }
    }

    pub(crate) fn backspace(&mut self) {
        if self.read_only {
            return;
        }
        self.flush_pending_input();
        if self.cursor_index > 0 {
            self.cursor_index -= 1;
            self.lines[self.cursor_line].units.remove(self.cursor_index);
            self.modified = true;
        } else if self.cursor_line > 0 {
            let current = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            self.cursor_index = self.lines[self.cursor_line].units.len();
            self.lines[self.cursor_line].units.extend(current.units);
            self.modified = true;
        }
        self.ensure_cursor_visible();
    }

    pub(crate) fn delete(&mut self) {
        if self.read_only {
            return;
        }
        self.flush_pending_input();
        if self.cursor_index < self.lines[self.cursor_line].units.len() {
            self.lines[self.cursor_line].units.remove(self.cursor_index);
            self.modified = true;
        } else if self.cursor_line + 1 < self.lines.len() {
            let next = self.lines.remove(self.cursor_line + 1);
            self.lines[self.cursor_line].units.extend(next.units);
            self.modified = true;
        }
        self.ensure_cursor_visible();
    }

    pub(crate) fn split_line(&mut self) {
        if self.read_only {
            return;
        }
        self.flush_pending_input();
        let trailing = self.lines[self.cursor_line]
            .units
            .split_off(self.cursor_index);
        self.lines
            .insert(self.cursor_line + 1, Line { units: trailing });
        self.cursor_line += 1;
        self.cursor_index = 0;
        self.modified = true;
        self.ensure_cursor_visible();
    }

    pub(crate) fn fill_visible_units(
        &self,
        line_index: usize,
        max_cols: usize,
        output: &mut Vec<JisChar>,
    ) {
        output.clear();
        let Some(line) = self.lines.get(line_index) else {
            return;
        };

        let mut skipped = 0usize;
        let mut used = 0usize;
        for &unit in &line.units {
            let width = unit.display_width();
            if skipped + width <= self.horizontal_scroll {
                skipped += width;
                continue;
            }
            if used + width > max_cols {
                break;
            }
            output.push(unit.display_char());
            used += width;
        }
    }

    fn insert_unit(&mut self, unit: TextUnit) {
        if self.read_only {
            return;
        }
        let line = &mut self.lines[self.cursor_line].units;
        if self.insert_mode || self.cursor_index >= line.len() {
            line.insert(self.cursor_index, unit);
        } else {
            line[self.cursor_index] = unit;
        }
        self.cursor_index += 1;
        self.modified = true;
        self.ensure_cursor_visible();
    }

    fn ensure_cursor_visible(&mut self) {
        if self.cursor_line < self.viewport_top {
            self.viewport_top = self.cursor_line;
        }
        if self.cursor_line >= self.viewport_top + PAGE_ROWS {
            self.viewport_top = self.cursor_line + 1 - PAGE_ROWS;
        }

        let visual_col = self.visual_cursor_col();
        if visual_col < self.horizontal_scroll {
            self.horizontal_scroll = visual_col;
        }
        if visual_col >= self.horizontal_scroll + 80 {
            self.horizontal_scroll = visual_col + 1 - 80;
        }
    }

    fn visual_width_for_index(&self, line_index: usize, index: usize) -> usize {
        self.lines
            .get(line_index)
            .map(|line| {
                line.units[..index.min(line.units.len())]
                    .iter()
                    .map(|unit| unit.display_width())
                    .sum()
            })
            .unwrap_or(0)
    }

    fn index_for_visual_col(&self, line_index: usize, visual_col: usize) -> usize {
        let Some(line) = self.lines.get(line_index) else {
            return 0;
        };

        let mut used = 0usize;
        for (index, unit) in line.units.iter().enumerate() {
            if used >= visual_col {
                return index;
            }
            used += unit.display_width();
        }
        line.units.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_line_endings() {
        let buffer =
            TextBuffer::from_bytes(b"A:\\TEST.TXT".to_vec(), b"ONE\rTWO\nTHREE\r\nFOUR", false);

        assert_eq!(buffer.lines.len(), 4);
    }

    #[test]
    fn encodes_with_crlf() {
        let buffer =
            TextBuffer::from_bytes(b"A:\\TEST.TXT".to_vec(), b"ONE\rTWO\nTHREE\r\nFOUR", false);

        assert_eq!(buffer.encode(), b"ONE\r\nTWO\r\nTHREE\r\nFOUR");
    }

    #[test]
    fn preserves_invalid_bytes() {
        let buffer = TextBuffer::from_bytes(b"A:\\RAW.TXT".to_vec(), b"\x82\x20X", false);
        assert_eq!(buffer.encode(), b"\x82\x20X");
    }

    #[test]
    fn displays_dos_eof_byte_with_native_glyph() {
        assert_eq!(TextUnit::Raw(0x1A).display_char(), JisChar::from_u16(0x1A));
    }
}
