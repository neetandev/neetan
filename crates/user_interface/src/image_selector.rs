use std::path::PathBuf;

use common::{JisChar, StackVec, str_to_jis};
use software_renderer::{
    GdcGraphicsInput, GraphicsInput, RenderInputs, SoftwareRenderer, TEXT_VRAM_BYTES,
};

const VISIBLE_ITEMS: usize = 19;
const COLS: usize = 80;
const FILENAME_MAX_COLS: usize = 74;

const ATTR_WHITE: u8 = (7 << 5) | 0x01;
const ATTR_WHITE_REVERSE: u8 = (7 << 5) | 0x01 | 0x04;
const ATTR_CYAN: u8 = (5 << 5) | 0x01;
const ATTR_CYAN_REVERSE: u8 = (5 << 5) | 0x01 | 0x04;

const BOX_HORIZONTAL: JisChar = JisChar::from_u16(0x2B24);
const BOX_VERTICAL: JisChar = JisChar::from_u16(0x2B26);
const BOX_TOP_LEFT: JisChar = JisChar::from_u16(0x2B30);
const BOX_TOP_RIGHT: JisChar = JisChar::from_u16(0x2B34);
const BOX_BOTTOM_RIGHT: JisChar = JisChar::from_u16(0x2B3C);
const BOX_BOTTOM_LEFT: JisChar = JisChar::from_u16(0x2B38);
const BOX_T_LEFT: JisChar = JisChar::from_u16(0x2B40);
const BOX_T_RIGHT: JisChar = JisChar::from_u16(0x2B48);

#[derive(Clone, PartialEq, Eq)]
pub enum MediaType {
    Floppy(usize),
    CdRom,
}

pub struct ImageEntry {
    pub path: PathBuf,
    pub jis_filename: Vec<JisChar>,
}

impl ImageEntry {
    pub fn new(path: PathBuf) -> Self {
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let jis_filename = str_to_jis(&filename);
        Self { path, jis_filename }
    }
}

pub struct ImageSelector {
    media_type: MediaType,
    cursor: usize,
    scroll_offset: usize,
    renderer: SoftwareRenderer,
    text_vram: Box<TextVram>,
    title_jis: Vec<JisChar>,
    dirty: bool,
}

impl ImageSelector {
    pub fn new(
        media_type: MediaType,
        initial_cursor: usize,
        entry_count: usize,
        font_rom_data: &[u8],
    ) -> Self {
        let cursor = initial_cursor.min(entry_count.saturating_sub(1));
        let scroll_offset = if cursor >= VISIBLE_ITEMS {
            cursor - VISIBLE_ITEMS + 1
        } else {
            0
        };
        let title_jis = str_to_jis(media_title(&media_type));
        let text_vram: Box<TextVram> = vec![0u8; TEXT_VRAM_BYTES]
            .into_boxed_slice()
            .try_into()
            .expect("TEXT_VRAM_BYTES");
        Self {
            media_type,
            cursor,
            scroll_offset,
            renderer: SoftwareRenderer::new(font_rom_data),
            text_vram,
            title_jis,
            dirty: true,
        }
    }

    pub fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn switch_media(
        &mut self,
        media_type: MediaType,
        initial_cursor: usize,
        entry_count: usize,
    ) {
        self.media_type = media_type;
        self.cursor = initial_cursor.min(entry_count.saturating_sub(1));
        self.scroll_offset = if self.cursor >= VISIBLE_ITEMS {
            self.cursor - VISIBLE_ITEMS + 1
        } else {
            0
        };
        self.title_jis = str_to_jis(media_title(&self.media_type));
        self.dirty = true;
    }

    pub fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            if self.cursor < self.scroll_offset {
                self.scroll_offset = self.cursor;
            }
            self.dirty = true;
        }
    }

    pub fn move_down(&mut self, entry_count: usize) {
        if entry_count > 0 && self.cursor < entry_count - 1 {
            self.cursor += 1;
            if self.cursor >= self.scroll_offset + VISIBLE_ITEMS {
                self.scroll_offset = self.cursor - VISIBLE_ITEMS + 1;
            }
            self.dirty = true;
        }
    }

    pub fn ensure_render(&mut self, entries: &[ImageEntry], current_index: Option<usize>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        rebuild_render(
            &mut self.renderer,
            &mut self.text_vram,
            entries,
            &self.title_jis,
            self.cursor,
            self.scroll_offset,
            current_index,
        );
    }

    pub fn framebuffer(&self) -> &[u8] {
        self.renderer.framebuffer()
    }
}

type TextVram = [u8; TEXT_VRAM_BYTES];

/// Writes a single byte into a text VRAM buffer.
fn write_vram_byte(text_vram: &mut TextVram, byte_offset: usize, value: u8) {
    text_vram[byte_offset] = value;
}

/// Writes a single ANK character and its attribute into one VRAM cell.
fn write_ank_cell(text_vram: &mut TextVram, row: usize, col: usize, ch: u8, attr: u8) {
    let offset = (row * COLS + col) * 2;
    write_vram_byte(text_vram, offset, ch);
    write_vram_byte(text_vram, offset + 1, 0x00);
    write_vram_byte(text_vram, 0x2000 + offset, attr);
    write_vram_byte(text_vram, 0x2000 + offset + 1, attr);
}

/// Writes a full-width JIS character into the VRAM at (row, col), occupying two screen columns.
///
/// The first cell stores the JIS code. The second cell (col+1) is written as a dummy ANK (0x00);
/// the normalizer detects that the previous cell was kanji and automatically renders the
/// right half of the glyph there.
fn write_fullwidth_cell(text_vram: &mut TextVram, row: usize, col: usize, jis: JisChar, attr: u8) {
    let offset = (row * COLS + col) * 2;
    let (even, odd) = jis.to_vram_bytes();
    write_vram_byte(text_vram, offset, even);
    write_vram_byte(text_vram, offset + 1, odd);
    write_vram_byte(text_vram, 0x2000 + offset, attr);
    write_vram_byte(text_vram, 0x2000 + offset + 1, attr);

    // Right-half placeholder: dummy ANK with the same attribute.
    let offset2 = (row * COLS + col + 1) * 2;
    write_vram_byte(text_vram, offset2, 0x00);
    write_vram_byte(text_vram, offset2 + 1, 0x00);
    write_vram_byte(text_vram, 0x2000 + offset2, attr);
    write_vram_byte(text_vram, 0x2000 + offset2 + 1, attr);
}

/// Draws a mixed ANK/full-width JIS string, returning the number of columns consumed.
fn draw_jis_string(
    text_vram: &mut TextVram,
    row: usize,
    start_col: usize,
    chars: &[JisChar],
    attr: u8,
    max_cols: usize,
) -> usize {
    let mut col = start_col;
    let limit = start_col + max_cols;
    for &jis in chars {
        if jis.is_ank() {
            if col >= limit {
                break;
            }
            write_ank_cell(text_vram, row, col, jis.as_u16() as u8, attr);
            col += 1;
        } else {
            if col + 2 > limit {
                break;
            }
            write_fullwidth_cell(text_vram, row, col, jis, attr);
            col += 2;
        }
    }
    col - start_col
}

fn draw_ank_str(text_vram: &mut TextVram, row: usize, start_col: usize, text: &str, attr: u8) {
    for (i, byte) in text.bytes().enumerate() {
        write_ank_cell(text_vram, row, start_col + i, byte, attr);
    }
}

/// Writes a half-width JIS character (rows 0x29-0x2B) into a single VRAM column.
fn write_halfwidth_cell(text_vram: &mut TextVram, row: usize, col: usize, jis: JisChar, attr: u8) {
    let offset = (row * COLS + col) * 2;
    let (even, odd) = jis.to_vram_bytes();
    write_vram_byte(text_vram, offset, even);
    write_vram_byte(text_vram, offset + 1, odd);
    write_vram_byte(text_vram, 0x2000 + offset, attr);
    write_vram_byte(text_vram, 0x2000 + offset + 1, attr);
}

/// Draws a horizontal border line using half-width box-drawing characters.
///
/// Layout: left corner (col 0) + 78 horizontal segments (cols 1-78) + right corner (col 79).
fn draw_horizontal_line(
    text_vram: &mut TextVram,
    row: usize,
    left: JisChar,
    right: JisChar,
    attr: u8,
) {
    write_halfwidth_cell(text_vram, row, 0, left, attr);
    for col in 1..79 {
        write_halfwidth_cell(text_vram, row, col, BOX_HORIZONTAL, attr);
    }
    write_halfwidth_cell(text_vram, row, 79, right, attr);
}

/// Draws left and right vertical border characters for a content row.
fn draw_vertical_borders(text_vram: &mut TextVram, row: usize, attr: u8) {
    write_halfwidth_cell(text_vram, row, 0, BOX_VERTICAL, attr);
    write_halfwidth_cell(text_vram, row, 79, BOX_VERTICAL, attr);
}

fn media_title(media_type: &MediaType) -> &'static str {
    match media_type {
        MediaType::Floppy(0) => "FDD1: Select Disk Image",
        MediaType::Floppy(_) => "FDD2: Select Disk Image",
        MediaType::CdRom => "CD-ROM: Select Disc Image",
    }
}

fn rebuild_render(
    renderer: &mut SoftwareRenderer,
    text_vram: &mut TextVram,
    entries: &[ImageEntry],
    title_jis: &[JisChar],
    cursor: usize,
    scroll_offset: usize,
    current_index: Option<usize>,
) {
    text_vram.fill(0);

    let mut palette_rgba = [0u32; 16];
    palette_rgba[0] = 0xFF000000; // black
    palette_rgba[1] = 0xFF0000BB; // blue
    palette_rgba[2] = 0xFF00BB00; // red (PC-98 order)
    palette_rgba[3] = 0xFF00BBBB; // magenta
    palette_rgba[4] = 0xFFBB0000; // green
    palette_rgba[5] = 0xFF777700; // cyan
    palette_rgba[6] = 0xFFBBBB00; // yellow
    palette_rgba[7] = 0xFF777777; // white

    let attr = ATTR_WHITE;

    draw_horizontal_line(text_vram, 0, BOX_TOP_LEFT, BOX_TOP_RIGHT, attr);

    draw_vertical_borders(text_vram, 1, attr);
    draw_jis_string(text_vram, 1, 2, title_jis, attr, 76);

    draw_horizontal_line(text_vram, 2, BOX_T_LEFT, BOX_T_RIGHT, attr);

    // Display list: index 0 = "<Empty>", indices 1..=N = disk image entries.
    let total = entries.len() + 1;
    let can_scroll_up = scroll_offset > 0;
    let can_scroll_down = scroll_offset + VISIBLE_ITEMS < total;

    // The display position of the currently loaded item.
    let loaded_display_index = current_index.map(|n| n + 1);

    for i in 0..VISIBLE_ITEMS {
        let row = 3 + i;
        let display_index = scroll_offset + i;

        draw_vertical_borders(text_vram, row, attr);

        if display_index >= total {
            continue;
        }

        let is_cursor = display_index == cursor;
        let is_loaded = if display_index == 0 {
            current_index.is_none()
        } else {
            loaded_display_index == Some(display_index)
        };

        let line_attr = match (is_cursor, is_loaded) {
            (true, true) => ATTR_CYAN_REVERSE,
            (true, false) => ATTR_WHITE_REVERSE,
            (false, true) => ATTR_CYAN,
            (false, false) => ATTR_WHITE,
        };

        for col in 1..79 {
            write_ank_cell(text_vram, row, col, b' ', line_attr);
        }

        if is_cursor {
            draw_ank_str(text_vram, row, 1, " >", line_attr);
        }

        if display_index == 0 {
            draw_ank_str(text_vram, row, 3, "<Empty>", line_attr);
        } else {
            let entry = &entries[display_index - 1];
            let jis = &entry.jis_filename;
            let cols_needed = jis_display_width(jis);
            if cols_needed > FILENAME_MAX_COLS {
                let truncated = truncate_jis(jis, FILENAME_MAX_COLS - 2);
                draw_jis_string(
                    text_vram,
                    row,
                    3,
                    &truncated,
                    line_attr,
                    FILENAME_MAX_COLS - 2,
                );
                let end_col = 3 + jis_display_width(&truncated);
                draw_ank_str(text_vram, row, end_col, "..", line_attr);
            } else {
                draw_jis_string(text_vram, row, 3, jis, line_attr, FILENAME_MAX_COLS);
            }
        }

        if is_loaded {
            draw_ank_str(text_vram, row, 70, "[loaded]", line_attr);
        }

        if i == 0 && can_scroll_up {
            write_ank_cell(text_vram, row, 78, b'^', line_attr);
        }
        if i == VISIBLE_ITEMS - 1 && can_scroll_down {
            write_ank_cell(text_vram, row, 78, b'v', line_attr);
        }
    }

    draw_horizontal_line(text_vram, 22, BOX_T_LEFT, BOX_T_RIGHT, attr);

    draw_vertical_borders(text_vram, 23, attr);
    draw_ank_str(
        text_vram,
        23,
        2,
        "Up/Down:Move  Enter:Select/Eject  ESC:Cancel",
        attr,
    );

    draw_horizontal_line(text_vram, 24, BOX_BOTTOM_LEFT, BOX_BOTTOM_RIGHT, attr);

    static ZERO_PLANE: [u8; 0x8000] = [0u8; 0x8000];

    let inputs = RenderInputs {
        text_vram,
        gdc_text_pitch: 80,
        gdc_scroll_start_line: [400 << 16, 0, 0, 0],
        video_mode: 0x08, // FONTSEL_8X16
        crtc_pl_bl: 15 << 16,
        crtc_cl_ssl: 16,
        crtc_sur_sdr: 0,
        kanji_high_mask: 0xFF,
        attr_semigraphics_mode: false,
        fontsel_8x16: true,
        blink_visible: true,
        cursor_visible: false,
        cursor_addr: 0,
        cursor_top: 0,
        cursor_bottom: 0,
        gdc_graphics_pitch: 0,
        gdc_graphics_scroll: [0; 4],
        gdc_graphics_display_mode_is_graphics: false,
        gdc_graphics_al: 0,
        crt_31khz_enabled: false,

        palette_rgba,
        global_enabled: true,
        text_enabled: true,
        graphics_enabled: false,

        graphics: GraphicsInput::Gdc(GdcGraphicsInput {
            b_plane: &ZERO_PLANE,
            r_plane: &ZERO_PLANE,
            g_plane: &ZERO_PLANE,
            e_plane: &ZERO_PLANE,
            lines_per_row: 1,
            zoom_display: 0,
            monochrome_mask: 0,
            is_16_color: false,
        }),
    };

    renderer.render(&inputs);
}

fn jis_display_width(chars: &[JisChar]) -> usize {
    chars.iter().map(|c| if c.is_ank() { 1 } else { 2 }).sum()
}

fn truncate_jis(chars: &[JisChar], max_cols: usize) -> StackVec<JisChar, COLS> {
    let mut result = StackVec::new();
    let mut width = 0;
    for &c in chars {
        let w = if c.is_ank() { 1 } else { 2 };
        if width + w > max_cols {
            break;
        }
        result.push(c);
        width += w;
    }
    result
}
