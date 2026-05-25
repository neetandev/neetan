use common::{
    ConsoleIo, JisChar, is_shift_jis_lead_byte, is_shift_jis_trail_byte, shift_jis_pair_to_jis,
};

use super::{
    AppMode, MessageLine, MessageStyle,
    dialog::{DeleteConfirm, FileMenuItem, Overlay},
    file_picker::{FilePickerEntryKind, FilePickerState, PickerFocus},
    text_buffer::PAGE_ROWS,
};

const BOX_HORIZONTAL: JisChar = JisChar::from_u16(0x2B24);
const BOX_VERTICAL: JisChar = JisChar::from_u16(0x2B26);
const BOX_TOP_LEFT: JisChar = JisChar::from_u16(0x2B30);
const BOX_TOP_RIGHT: JisChar = JisChar::from_u16(0x2B34);
const BOX_BOTTOM_RIGHT: JisChar = JisChar::from_u16(0x2B3C);
const BOX_BOTTOM_LEFT: JisChar = JisChar::from_u16(0x2B38);

const ATTR_TEXT: u8 = 0xE1;
const ATTR_CHROME: u8 = 0xA1;
const ATTR_MESSAGE_SUCCESS: u8 = 0x41;
const ATTR_MESSAGE_WARNING: u8 = 0xF1;
const ATTR_MESSAGE_ERROR: u8 = 0x81;
const ATTR_SELECTION: u8 = 0x65;
const ATTR_DIALOG: u8 = 0xA1;
const ATTR_DIALOG_TEXT: u8 = 0xA1;

#[derive(Default)]
pub(crate) struct DisplayBuffer {
    bytes: Vec<u8>,
    jis: Vec<JisChar>,
    line: Vec<JisChar>,
}

impl DisplayBuffer {
    pub(crate) fn new() -> Self {
        Self {
            bytes: Vec::with_capacity(160),
            jis: Vec::with_capacity(160),
            line: Vec::with_capacity(80),
        }
    }
}

pub(crate) fn render(
    mode: &AppMode,
    overlay: Option<&Overlay>,
    message: &MessageLine,
    display: &mut DisplayBuffer,
    console: &mut dyn ConsoleIo,
) {
    console.set_cursor_visible(false);
    console.fill_region(0, 0, 25, 80, JisChar::SPACE, ATTR_TEXT);
    match mode {
        AppMode::FilePicker(picker) => render_picker(picker, message, display, console),
        AppMode::Editor(editor) => render_editor(editor, message, display, console),
    }
    if let Some(overlay) = overlay {
        render_overlay(overlay, display, console);
        set_overlay_cursor(overlay, console);
    } else {
        set_mode_cursor(mode, console);
    }
}

fn render_picker(
    picker: &FilePickerState,
    message: &MessageLine,
    display: &mut DisplayBuffer,
    console: &mut dyn ConsoleIo,
) {
    fill_line(console, 0, ATTR_CHROME);
    console.write_ank_at(0, 0, b"edit - Open File", ATTR_CHROME);
    display.bytes.clear();
    append_ascii(&mut display.bytes, b"Dir: ");
    append_ank_lossy_shift_jis(&mut display.bytes, &picker.directory_path);
    right_align_bytes(console, 0, &display.bytes, ATTR_CHROME);

    fill_line(console, 1, ATTR_CHROME);
    console.write_ank_at(1, 0, b"Path: ", ATTR_CHROME);
    write_shift_jis(
        display,
        console,
        1,
        6,
        picker.path_field.bytes(),
        ATTR_CHROME,
    );

    draw_separator(console, 2);
    for row in 0..18u8 {
        let screen_row = 3 + row;
        let entry_index = picker.scroll + row as usize;
        let attr = if entry_index == picker.selected {
            ATTR_SELECTION
        } else {
            ATTR_TEXT
        };
        fill_line(console, screen_row, attr);
        if let Some(entry) = picker.entries.get(entry_index) {
            match entry.kind {
                FilePickerEntryKind::Parent => {
                    console.write_ank_at(screen_row, 0, b"> ", attr);
                    write_shift_jis(display, console, screen_row, 2, &entry.display_name, attr);
                }
                FilePickerEntryKind::Directory => {
                    let next_col =
                        write_shift_jis(display, console, screen_row, 2, &entry.display_name, attr);
                    if next_col < 80 {
                        console.write_ank_at(screen_row, next_col, b"\\", attr);
                    }
                }
                FilePickerEntryKind::File => {
                    write_shift_jis(display, console, screen_row, 2, &entry.display_name, attr);
                    display.bytes.clear();
                    append_size_kb(&mut display.bytes, entry.size);
                    right_align_bytes(console, screen_row, &display.bytes, attr);
                }
            }
        }
    }

    draw_separator(console, 21);
    fill_line(console, 22, ATTR_CHROME);
    if let Some(entry) = picker.selected_entry() {
        console.write_ank_at(22, 0, b"Sel:", ATTR_CHROME);
        write_shift_jis(display, console, 22, 4, &entry.display_name, ATTR_CHROME);
        console.write_ank_at(22, 68, b"Focus:", ATTR_CHROME);
        console.write_ank_at(
            22,
            74,
            if picker.focus == PickerFocus::Path {
                b"Path"
            } else {
                b"List"
            },
            ATTR_CHROME,
        );
    } else {
        console.write_ank_at(22, 0, b"Sel:- Focus:Path", ATTR_CHROME);
    }

    render_message_line(
        23,
        message,
        display,
        console,
        b"Enter opens item or path. Tab switches Path/List.",
    );
    render_picker_function_bar(console);
}

fn render_editor(
    editor: &super::EditorState,
    message: &MessageLine,
    display: &mut DisplayBuffer,
    console: &mut dyn ConsoleIo,
) {
    fill_line(console, 0, ATTR_CHROME);
    let mut next_col = console.write_ank_at(0, 0, b"edit - ", ATTR_CHROME);
    next_col = write_shift_jis(
        display,
        console,
        0,
        next_col,
        &editor.buffer.path,
        ATTR_CHROME,
    );
    if editor.buffer.modified && next_col < 79 {
        console.write_ank_at(0, next_col, b" *", ATTR_CHROME);
    }

    display.bytes.clear();
    append_ascii(
        &mut display.bytes,
        if editor.buffer.insert_mode {
            b"INS  "
        } else {
            b"OVR  "
        },
    );
    append_ascii(&mut display.bytes, b"Ln ");
    append_decimal_min_width(&mut display.bytes, editor.buffer.cursor_line + 1, 2);
    append_ascii(&mut display.bytes, b" Col ");
    append_decimal_min_width(&mut display.bytes, editor.buffer.visual_cursor_col() + 1, 2);
    right_align_bytes(console, 0, &display.bytes, ATTR_CHROME);

    draw_separator(console, 1);
    for row in 0..PAGE_ROWS as u8 {
        let screen_row = 2 + row;
        fill_line(console, screen_row, ATTR_TEXT);
        let line_index = editor.buffer.viewport_top + row as usize;
        editor
            .buffer
            .fill_visible_units(line_index, 80, &mut display.line);
        console.write_jis(screen_row, 0, &display.line, ATTR_TEXT);
    }

    draw_separator(console, 21);
    fill_line(console, 22, ATTR_CHROME);
    console.write_ank_at(22, 0, b"File:", ATTR_CHROME);
    next_col = write_shift_jis(display, console, 22, 5, &editor.buffer.path, ATTR_CHROME);
    if next_col < 80 {
        console.write_ank_at(22, next_col.min(79), b" ", ATTR_CHROME);
    }
    console.write_ank_at(22, 50, b"Mod:", ATTR_CHROME);
    console.write_ank_at(
        22,
        54,
        if editor.buffer.modified {
            b"Yes"
        } else {
            b"No"
        },
        ATTR_CHROME,
    );
    console.write_ank_at(22, 58, b"Enc:SJIS", ATTR_CHROME);
    console.write_ank_at(22, 67, b"Wrap:Off", ATTR_CHROME);
    if editor.buffer.read_only {
        console.write_ank_at(22, 76, b"RO", ATTR_CHROME);
    }

    render_message_line(23, message, display, console, b"");
    render_editor_function_bar(console);
}

fn render_overlay(overlay: &Overlay, display: &mut DisplayBuffer, console: &mut dyn ConsoleIo) {
    match overlay {
        Overlay::DriveSelect { drives, dialog } => {
            draw_box(console, 7, 27, 11, 28, ATTR_DIALOG);
            console.write_ank_at(7, 29, b"Select Drive", ATTR_DIALOG_TEXT);
            for (index, drive) in drives.iter().enumerate() {
                let attr = if index == dialog.selected {
                    ATTR_SELECTION
                } else {
                    ATTR_DIALOG_TEXT
                };
                fill_region(console, 8 + index as u8, 28, 1, 26, attr);
                let row = 8 + index as u8;
                let text = [b'A' + *drive, b':', b'\\'];
                console.write_ank_at(row, 30, &text, attr);
                if index == dialog.selected {
                    console.write_ank_at(row, 28, b">", attr);
                }
            }
            console.write_ank_at(16, 29, b"Enter: Choose  Esc: Back", ATTR_DIALOG_TEXT);
        }
        Overlay::CreateDirectory(prompt) => render_text_prompt(
            prompt.title,
            &prompt.field,
            b"Enter: Create  Esc: Back",
            display,
            console,
        ),
        Overlay::DeleteConfirm(confirm) => render_delete_confirm(confirm, display, console),
        Overlay::NewFile(prompt) => render_text_prompt(
            prompt.title,
            &prompt.field,
            b"Enter: Create  Esc: Back",
            display,
            console,
        ),
        Overlay::SaveAs(prompt) => render_text_prompt(
            prompt.title,
            &prompt.field,
            b"Enter: Save  Esc: Back",
            display,
            console,
        ),
        Overlay::FileMenu(dialog) => {
            draw_box(console, 8, 27, 8, 28, ATTR_DIALOG);
            console.write_ank_at(8, 29, b"File", ATTR_DIALOG_TEXT);
            let items = FileMenuItem::all();
            for (index, item) in items.iter().enumerate() {
                let row = 9 + index as u8;
                let attr = if index == dialog.selected {
                    ATTR_SELECTION
                } else {
                    ATTR_DIALOG_TEXT
                };
                fill_region(console, row, 28, 1, 26, attr);
                console.write_ank_at(row, 30, item.label(), attr);
            }
            console.write_ank_at(13, 29, b"Enter: Choose  Esc: Back", ATTR_DIALOG_TEXT);
        }
        Overlay::UnsavedChanges(_) => {
            draw_box(console, 9, 23, 6, 34, ATTR_DIALOG);
            console.write_ank_at(10, 25, b"Unsaved Changes", ATTR_DIALOG_TEXT);
            console.write_ank_at(12, 25, b"F2 Save   F3 NoSave   Esc", ATTR_DIALOG_TEXT);
        }
    }
}

fn render_delete_confirm(
    confirm: &DeleteConfirm,
    display: &mut DisplayBuffer,
    console: &mut dyn ConsoleIo,
) {
    draw_box(console, 9, 16, 6, 48, ATTR_DIALOG);
    console.write_ank_at(9, 18, b"Delete", ATTR_DIALOG_TEXT);
    console.write_ank_at(
        10,
        18,
        if confirm.is_directory {
            b"Delete directory?"
        } else {
            b"Delete file?"
        },
        ATTR_DIALOG_TEXT,
    );
    write_shift_jis(
        display,
        console,
        11,
        18,
        &confirm.display_name,
        ATTR_DIALOG_TEXT,
    );
    console.write_ank_at(13, 18, b"Enter: Delete  Esc: Back", ATTR_DIALOG_TEXT);
}

fn render_text_prompt(
    title: &[u8],
    field: &super::input::ByteField,
    footer: &[u8],
    display: &mut DisplayBuffer,
    console: &mut dyn ConsoleIo,
) {
    draw_box(console, 9, 10, 5, 60, ATTR_DIALOG);
    console.write_ank_at(9, 12, title, ATTR_DIALOG_TEXT);
    console.write_ank_at(10, 12, b"Path: ", ATTR_DIALOG_TEXT);
    write_shift_jis(display, console, 10, 18, field.bytes(), ATTR_DIALOG_TEXT);
    console.write_ank_at(12, 12, footer, ATTR_DIALOG_TEXT);
}

fn set_mode_cursor(mode: &AppMode, console: &mut dyn ConsoleIo) {
    match mode {
        AppMode::FilePicker(picker) => {
            if picker.focus == PickerFocus::Path {
                let col = 6 + display_width_for_prefix(
                    picker.path_field.bytes(),
                    picker.path_field.cursor,
                );
                console.set_cursor_position(1, col.min(79) as u8);
                console.set_cursor_visible(true);
            }
        }
        AppMode::Editor(editor) => {
            let row = editor
                .buffer
                .cursor_line
                .saturating_sub(editor.buffer.viewport_top);
            if row < PAGE_ROWS {
                let col = editor
                    .buffer
                    .visual_cursor_col()
                    .saturating_sub(editor.buffer.horizontal_scroll)
                    .min(79);
                console.set_cursor_position((2 + row) as u8, col as u8);
                console.set_cursor_visible(true);
            }
        }
    }
}

fn set_overlay_cursor(overlay: &Overlay, console: &mut dyn ConsoleIo) {
    match overlay {
        Overlay::CreateDirectory(prompt) | Overlay::NewFile(prompt) | Overlay::SaveAs(prompt) => {
            let col = 18 + display_width_for_prefix(prompt.field.bytes(), prompt.field.cursor);
            console.set_cursor_position(10, col.min(67) as u8);
            console.set_cursor_visible(true);
        }
        _ => {}
    }
}

fn draw_separator(console: &mut dyn ConsoleIo, row: u8) {
    fill_line(console, row, ATTR_TEXT);
    for col in 0..80u8 {
        console.write_jis_char(row, col, BOX_HORIZONTAL, ATTR_TEXT);
    }
}

fn draw_box(console: &mut dyn ConsoleIo, top: u8, left: u8, height: u8, width: u8, attr: u8) {
    let bottom = top + height - 1;
    let right = left + width - 1;
    console.write_jis_char(top, left, BOX_TOP_LEFT, attr);
    console.write_jis_char(top, right, BOX_TOP_RIGHT, attr);
    console.write_jis_char(bottom, left, BOX_BOTTOM_LEFT, attr);
    console.write_jis_char(bottom, right, BOX_BOTTOM_RIGHT, attr);
    for col in left + 1..right {
        console.write_jis_char(top, col, BOX_HORIZONTAL, attr);
        console.write_jis_char(bottom, col, BOX_HORIZONTAL, attr);
    }
    for row in top + 1..bottom {
        console.write_jis_char(row, left, BOX_VERTICAL, attr);
        console.write_jis_char(row, right, BOX_VERTICAL, attr);
        fill_region(console, row, left + 1, 1, width - 2, attr);
    }
}

fn fill_region(console: &mut dyn ConsoleIo, row: u8, col: u8, height: u8, width: u8, attr: u8) {
    console.fill_region(row, col, height, width, JisChar::SPACE, attr);
}

fn fill_line(console: &mut dyn ConsoleIo, row: u8, attr: u8) {
    console.fill_region(row, 0, 1, 80, JisChar::SPACE, attr);
}

fn render_message_line(
    row: u8,
    message: &MessageLine,
    display: &mut DisplayBuffer,
    console: &mut dyn ConsoleIo,
    fallback: &[u8],
) {
    let (text, attr) = if message.text.is_empty() {
        (fallback, ATTR_CHROME)
    } else {
        (
            message.text.as_slice(),
            match message.style {
                MessageStyle::Neutral => ATTR_CHROME,
                MessageStyle::Success => ATTR_MESSAGE_SUCCESS,
                MessageStyle::Warning => ATTR_MESSAGE_WARNING,
                MessageStyle::Error => ATTR_MESSAGE_ERROR,
            },
        )
    };
    fill_line(console, row, attr);
    write_shift_jis(display, console, row, 0, text, attr);
}

fn render_picker_function_bar(console: &mut dyn ConsoleIo) {
    fill_line(console, 24, ATTR_CHROME);
    let labels: [(u8, &[u8]); 7] = [
        (0, b"F1 Drv"),
        (8, b"F2 Open"),
        (18, b"F3 New"),
        (28, b"F4 Refr"),
        (39, b"F5 Del"),
        (51, b"F6 MkDr"),
        (62, b"F7 Up"),
    ];
    for (col, label) in labels {
        console.write_ank_at(24, col, label, ATTR_CHROME);
    }
}

fn render_editor_function_bar(console: &mut dyn ConsoleIo) {
    fill_line(console, 24, ATTR_CHROME);
    let labels: [(u8, &[u8]); 3] = [(0, b"F1 File"), (10, b"F2 Save"), (21, b"F3 SaveAs")];
    for (col, label) in labels {
        console.write_ank_at(24, col, label, ATTR_CHROME);
    }
}

fn write_shift_jis(
    display: &mut DisplayBuffer,
    console: &mut dyn ConsoleIo,
    row: u8,
    col: u8,
    bytes: &[u8],
    attr: u8,
) -> u8 {
    display.jis.clear();
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if is_shift_jis_lead_byte(byte)
            && let Some(&trail) = bytes.get(index + 1)
            && is_shift_jis_trail_byte(trail)
            && let Some(jis) = shift_jis_pair_to_jis(byte, trail)
        {
            display.jis.push(jis);
            index += 2;
            continue;
        }
        display
            .jis
            .push(JisChar::from_u16(if byte >= 0x20 || byte >= 0xA1 {
                byte as u16
            } else {
                b'?' as u16
            }));
        index += 1;
    }
    console.write_jis(row, col, &display.jis, attr)
}

fn right_align_bytes(console: &mut dyn ConsoleIo, row: u8, text: &[u8], attr: u8) {
    let col = 80usize.saturating_sub(text.len());
    console.write_ank_at(row, col as u8, text, attr);
}

fn display_width_for_prefix(bytes: &[u8], byte_count: usize) -> usize {
    let mut width = 0usize;
    let mut index = 0usize;
    let limit = byte_count.min(bytes.len());
    while index < limit {
        let byte = bytes[index];
        if is_shift_jis_lead_byte(byte)
            && index + 1 < limit
            && is_shift_jis_trail_byte(bytes[index + 1])
            && shift_jis_pair_to_jis(byte, bytes[index + 1]).is_some()
        {
            width += 2;
            index += 2;
        } else {
            width += 1;
            index += 1;
        }
    }
    width
}

fn append_ascii(buffer: &mut Vec<u8>, bytes: &[u8]) {
    buffer.extend_from_slice(bytes);
}

fn append_ank_lossy_shift_jis(buffer: &mut Vec<u8>, bytes: &[u8]) {
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if is_shift_jis_lead_byte(byte)
            && let Some(&trail) = bytes.get(index + 1)
            && is_shift_jis_trail_byte(trail)
            && shift_jis_pair_to_jis(byte, trail).is_some()
        {
            buffer.push(b'?');
            index += 2;
        } else {
            buffer.push(if byte >= 0x20 || byte >= 0xA1 {
                byte
            } else {
                b'?'
            });
            index += 1;
        }
    }
}

fn append_decimal(buffer: &mut Vec<u8>, mut value: usize) {
    let start = buffer.len();
    if value == 0 {
        buffer.push(b'0');
        return;
    }
    while value > 0 {
        buffer.push(b'0' + (value % 10) as u8);
        value /= 10;
    }
    buffer[start..].reverse();
}

fn append_decimal_min_width(buffer: &mut Vec<u8>, value: usize, width: usize) {
    let start = buffer.len();
    append_decimal(buffer, value);
    let digits = buffer.len() - start;
    if digits < width {
        let pad = width - digits;
        buffer.resize(buffer.len() + pad, b'0');
        buffer[start..].rotate_right(pad);
    }
}

fn append_size_kb(buffer: &mut Vec<u8>, file_size: u32) {
    append_decimal(buffer, file_size.div_ceil(1024).max(1) as usize);
    append_ascii(buffer, b" KB");
}

#[cfg(test)]
mod tests {
    use common::ConsoleIo;

    use super::*;
    use crate::commands::editor::{
        EditorState,
        dialog::ListDialog,
        file_picker::{FilePickerEntry, PickerFocus},
        input::ByteField,
        text_buffer::TextBuffer,
    };

    #[derive(Clone, Copy)]
    struct Cell {
        ch: JisChar,
        attr: u8,
    }

    impl Default for Cell {
        fn default() -> Self {
            Self {
                ch: JisChar::SPACE,
                attr: 0,
            }
        }
    }

    struct TestConsole {
        cells: [[Cell; 80]; 25],
        cursor_row: u8,
        cursor_col: u8,
        cursor_visible: bool,
    }

    impl TestConsole {
        fn new() -> Self {
            Self {
                cells: [[Cell::default(); 80]; 25],
                cursor_row: 0,
                cursor_col: 0,
                cursor_visible: false,
            }
        }

        fn row_ascii(&self, row: u8) -> String {
            self.cells[row as usize]
                .iter()
                .map(|cell| {
                    if cell.ch.is_ank() {
                        cell.ch.as_u16() as u8 as char
                    } else {
                        ' '
                    }
                })
                .collect()
        }

        fn attr_at(&self, row: u8, col: u8) -> u8 {
            self.cells[row as usize][col as usize].attr
        }
    }

    impl ConsoleIo for TestConsole {
        fn write_char(&mut self, ch: u8) {
            self.write_jis_char(
                self.cursor_row,
                self.cursor_col,
                JisChar::from_u16(ch as u16),
                ATTR_TEXT,
            );
            self.cursor_col = self.cursor_col.saturating_add(1).min(79);
        }

        fn write_str(&mut self, s: &[u8]) {
            for &byte in s {
                self.write_char(byte);
            }
        }

        fn write_jis_char(&mut self, row: u8, col: u8, ch: JisChar, attr: u8) {
            self.cells[row as usize][col as usize] = Cell { ch, attr };
            if ch.display_width() == 2 && col < 79 {
                self.cells[row as usize][col as usize + 1] = Cell {
                    ch: JisChar::from_u16(0x0000),
                    attr,
                };
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
                } else if current_col >= 79 {
                    break;
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
            0
        }

        fn char_available(&self) -> bool {
            false
        }

        fn read_key(&mut self) -> (u8, u8) {
            (0, 0)
        }

        fn cursor_position(&self) -> (u8, u8) {
            (self.cursor_row, self.cursor_col)
        }

        fn set_cursor_position(&mut self, row: u8, col: u8) {
            self.cursor_row = row;
            self.cursor_col = col;
        }

        fn scroll_up(&mut self) {}

        fn clear_screen(&mut self) {
            self.fill_region(0, 0, 25, 80, JisChar::SPACE, ATTR_TEXT);
        }

        fn set_cursor_visible(&mut self, visible: bool) {
            self.cursor_visible = visible;
        }

        fn screen_size(&self) -> (u8, u8) {
            (80, 25)
        }
    }

    fn empty_message() -> MessageLine {
        MessageLine {
            text: Vec::new(),
            style: MessageStyle::Neutral,
        }
    }

    #[test]
    fn picker_uses_unified_chrome_and_plain_directory_rows() {
        let mut picker = FilePickerState {
            directory_path: b"A:\\".to_vec(),
            path_field: ByteField::new(b"A:\\".to_vec()),
            focus: PickerFocus::List,
            entries: vec![
                FilePickerEntry {
                    kind: FilePickerEntryKind::Directory,
                    display_name: b"FOLDER".to_vec(),
                    full_path: b"A:\\FOLDER".to_vec(),
                    size: 0,
                },
                FilePickerEntry {
                    kind: FilePickerEntryKind::File,
                    display_name: b"FILE.TXT".to_vec(),
                    full_path: b"A:\\FILE.TXT".to_vec(),
                    size: 1234,
                },
            ],
            selected: 1,
            scroll: 0,
        };
        picker.sync_path_with_selection();

        let mut display = DisplayBuffer::new();
        let mut console = TestConsole::new();

        render(
            &AppMode::FilePicker(picker),
            None,
            &empty_message(),
            &mut display,
            &mut console,
        );

        let directory_row = console.row_ascii(3);
        assert!(directory_row.contains("FOLDER\\"));
        assert!(!directory_row.contains("[DIR]"));

        assert_eq!(console.attr_at(0, 0), ATTR_CHROME);
        assert_eq!(console.attr_at(1, 0), ATTR_CHROME);
        assert_eq!(console.attr_at(22, 0), ATTR_CHROME);
        assert_eq!(console.attr_at(23, 0), ATTR_CHROME);
        assert_eq!(console.attr_at(24, 0), ATTR_CHROME);
        assert_eq!(console.attr_at(24, 8), ATTR_CHROME);
        assert_eq!(console.attr_at(24, 18), ATTR_CHROME);

        let footer = console.row_ascii(22);
        assert!(!footer.contains("Type:"));
        assert!(!footer.contains("Size:"));
        assert!(!footer.contains("Action:Open"));
        assert!(console.row_ascii(24).contains("F5 Del"));
    }

    #[test]
    fn file_menu_dialog_uses_dialog_text_for_unselected_items() {
        let mut display = DisplayBuffer::new();
        let mut console = TestConsole::new();
        let editor = EditorState {
            buffer: TextBuffer::new(b"A:\\TEST.TXT".to_vec(), false),
            require_create_on_first_save: false,
        };

        render(
            &AppMode::Editor(editor),
            Some(&Overlay::FileMenu(ListDialog::new())),
            &empty_message(),
            &mut display,
            &mut console,
        );

        assert_eq!(console.attr_at(8, 27), ATTR_DIALOG);
        assert_eq!(console.attr_at(8, 29), ATTR_DIALOG_TEXT);
        assert_eq!(console.attr_at(9, 30), ATTR_SELECTION);
        assert_eq!(console.attr_at(10, 30), ATTR_DIALOG_TEXT);
        assert_eq!(console.attr_at(13, 29), ATTR_DIALOG_TEXT);
    }
}
