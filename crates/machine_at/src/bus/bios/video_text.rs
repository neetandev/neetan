//! INT 10h text services: window scroll, character and attribute access,
//! teletype output and write string.
//!
//! Text cells go through the paging-aware guest memory accessors, so the
//! regen traffic reaches the VGA device exactly like CPU stores. Graphics
//! mode operations delegate to the codecs in `video_graphics.rs`.

use common::{Cpu, TraceSink};

use super::{
    super::AtBus,
    video::BDA_ACTIVE_PAGE,
    video_modes::{ModeFamily, VideoModeEntry},
};

/// PIT control port.
const PIT_CONTROL_PORT: u16 = 0x0043;
/// PIT channel 2 data port.
const PIT_CHANNEL_2_PORT: u16 = 0x0042;
/// System control port B (PIT gate 2 and speaker data bits).
const SYSTEM_CONTROL_PORT_B: u16 = 0x0061;
/// PIT channel 2 divisor for the teletype BEL tone (about 896 Hz).
const BEL_TONE_DIVISOR: u16 = 0x0533;
/// Timer ticks the BEL tone stays on (about 110 ms).
const BEL_TONE_TICKS: u8 = 2;

impl<T: TraceSink> AtBus<T> {
    /// Linear address of a text cell on the given page.
    fn text_cell_address(
        &mut self,
        entry: &'static VideoModeEntry,
        page: u8,
        row: u8,
        column: u8,
    ) -> u32 {
        entry.regen_base
            + u32::from(page & 0x07) * u32::from(entry.page_size)
            + (u32::from(row) * u32::from(entry.columns) + u32::from(column)) * 2
    }

    /// AH=06h/07h: scrolls a window up or down, blanking with attribute BH.
    pub(super) fn int10h_scroll(&mut self, cpu: &mut impl Cpu, up: bool) {
        let Some(entry) = self.active_mode_entry() else {
            return;
        };
        self.scroll_window(
            entry,
            cpu.ch(),
            cpu.cl(),
            cpu.dh(),
            cpu.dl(),
            cpu.al(),
            cpu.bh(),
            up,
        );
    }

    /// Scrolls the window (top/left)-(bottom/right) by the given number of
    /// text lines (0 clears the window) on the active page.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn scroll_window(
        &mut self,
        entry: &'static VideoModeEntry,
        top: u8,
        left: u8,
        bottom: u8,
        right: u8,
        lines: u8,
        fill_attribute: u8,
        up: bool,
    ) {
        let bottom = bottom.min(entry.rows_minus_one);
        let right = right.min((entry.columns - 1) as u8);
        if top > bottom || left > right {
            return;
        }
        let window_rows = bottom - top + 1;
        let lines = if lines == 0 || lines >= window_rows {
            window_rows
        } else {
            lines
        };

        if entry.family != ModeFamily::Text {
            self.graphics_scroll(entry, top, left, bottom, right, lines, fill_attribute, up);
            return;
        }

        let page = self.read_mem_byte(BDA_ACTIVE_PAGE);
        let move_rows = window_rows - lines;
        for step in 0..move_rows {
            let (destination_row, source_row) = if up {
                (top + step, top + lines + step)
            } else {
                (bottom - step, bottom - lines - step)
            };
            for column in left..=right {
                let source = self.text_cell_address(entry, page, source_row, column);
                let cell = self.read_mem_word(source);
                let destination = self.text_cell_address(entry, page, destination_row, column);
                self.write_mem_word(destination, cell);
            }
        }

        let fill = (u16::from(fill_attribute) << 8) | 0x20;
        for step in 0..lines {
            let row = if up { bottom - step } else { top + step };
            for column in left..=right {
                let address = self.text_cell_address(entry, page, row, column);
                self.write_mem_word(address, fill);
            }
        }
    }

    /// AH=08h: reads the character and attribute at the cursor of page BH.
    pub(super) fn int10h_read_char_attr(&mut self, cpu: &mut impl Cpu) {
        let Some(entry) = self.active_mode_entry() else {
            return;
        };
        let page = cpu.bh();
        let (row, column) = self.cursor_position(page);
        if entry.family == ModeFamily::Text {
            let address = self.text_cell_address(entry, page, row, column);
            let cell = self.read_mem_word(address);
            cpu.set_al(cell as u8);
            cpu.set_ah((cell >> 8) as u8);
        } else {
            let code = self.graphics_glyph_read(entry, page, row, column);
            cpu.set_al(code);
            cpu.set_ah(0x00);
        }
    }

    /// AH=09h/0Ah: writes character AL (with attribute BL for AH=09h) CX
    /// times at the cursor of page BH, without moving the cursor.
    pub(super) fn int10h_write_char_attr(&mut self, cpu: &mut impl Cpu, with_attribute: bool) {
        let Some(entry) = self.active_mode_entry() else {
            return;
        };
        let page = cpu.bh();
        let character = cpu.al();
        let (row, column) = self.cursor_position(page);
        let count = cpu.cx();

        if entry.family == ModeFamily::Text {
            let attribute = cpu.bl();
            let start = self.text_cell_address(entry, page, row, column);
            let page_end = entry.regen_base
                + u32::from(page & 0x07) * u32::from(entry.page_size)
                + u32::from(entry.columns) * (u32::from(entry.rows_minus_one) + 1) * 2;
            for index in 0..u32::from(count) {
                let address = start + index * 2;
                if address >= page_end {
                    break;
                }
                self.write_mem_byte(address, character);
                if with_attribute {
                    self.write_mem_byte(address + 1, attribute);
                }
            }
        } else {
            // Graphics glyphs stay on the current row (bit 7 of the color
            // XORs the glyph into the frame).
            let color = cpu.bl();
            let columns = entry.columns as u8;
            for index in 0..count {
                let target_column = column + index as u8;
                if target_column >= columns {
                    break;
                }
                self.graphics_glyph_write(entry, page, row, target_column, character, color);
            }
        }
    }

    /// AH=0Eh: teletype output of AL on page BH (BL is the foreground color
    /// in graphics modes).
    pub(super) fn int10h_teletype(&mut self, cpu: &mut impl Cpu) {
        let Some(entry) = self.active_mode_entry() else {
            return;
        };
        self.teletype_character(entry, cpu.bh(), cpu.al(), None, cpu.bl());
    }

    /// Writes one teletype character: printable output advances and wraps
    /// the cursor, CR/LF/BS/BEL act as control codes, the bottom line
    /// scrolls. `attribute` writes the attribute byte too (the write-string
    /// sub-modes); `None` keeps the attribute in place like plain teletype.
    fn teletype_character(
        &mut self,
        entry: &'static VideoModeEntry,
        page: u8,
        character: u8,
        attribute: Option<u8>,
        graphics_color: u8,
    ) {
        let (mut row, mut column) = self.cursor_position(page);
        match character {
            0x07 => {
                self.start_teletype_beep();
                return;
            }
            0x08 => {
                column = column.saturating_sub(1);
            }
            0x0D => {
                column = 0;
            }
            0x0A => {
                row += 1;
            }
            _ => {
                if entry.family == ModeFamily::Text {
                    let address = self.text_cell_address(entry, page, row, column);
                    self.write_mem_byte(address, character);
                    if let Some(attribute) = attribute {
                        self.write_mem_byte(address + 1, attribute);
                    }
                } else {
                    let color = attribute.unwrap_or(graphics_color);
                    self.graphics_glyph_write(entry, page, row, column, character, color);
                }
                column += 1;
                if column >= entry.columns as u8 {
                    column = 0;
                    row += 1;
                }
            }
        }

        if row > entry.rows_minus_one {
            row = entry.rows_minus_one;
            let fill_attribute = if entry.family == ModeFamily::Text {
                // The real BIOS blanks with the attribute under the cursor.
                let address = self.text_cell_address(entry, page, row, column);
                self.read_mem_byte(address + 1)
            } else {
                0x00
            };
            self.scroll_window(
                entry,
                0,
                0,
                entry.rows_minus_one,
                (entry.columns - 1) as u8,
                1,
                fill_attribute,
                true,
            );
        }
        self.set_cursor_position(entry, page, row, column);
    }

    /// AH=13h: writes a string from ES:BP at DH/DL on page BH. AL bit 0
    /// moves the cursor to the string end, bit 1 selects the
    /// character/attribute pair format.
    pub(super) fn int10h_write_string(&mut self, cpu: &mut impl Cpu) {
        let Some(entry) = self.active_mode_entry() else {
            return;
        };
        let move_cursor = cpu.al() & 0x01 != 0;
        let with_attributes = cpu.al() & 0x02 != 0;
        let page = cpu.bh();
        let attribute = cpu.bl();
        let count = cpu.cx();
        let source = (u32::from(cpu.es()) << 4).wrapping_add(u32::from(cpu.bp()));

        let (saved_row, saved_column) = self.cursor_position(page);
        self.set_cursor_position(entry, page, cpu.dh(), cpu.dl());

        let mut offset = 0u32;
        for _ in 0..count {
            let character = self.read_mem_byte(source.wrapping_add(offset));
            offset += 1;
            let attribute = if with_attributes {
                let value = self.read_mem_byte(source.wrapping_add(offset));
                offset += 1;
                value
            } else {
                attribute
            };
            self.teletype_character(entry, page, character, Some(attribute), attribute);
        }

        if !move_cursor {
            self.set_cursor_position(entry, page, saved_row, saved_column);
        }
    }

    /// Starts the teletype BEL tone: PIT channel 2 square wave with the
    /// speaker gate open. The INT 08h tick handler stops it.
    fn start_teletype_beep(&mut self) {
        self.io_write(PIT_CONTROL_PORT, 0xB6);
        self.io_write(PIT_CHANNEL_2_PORT, BEL_TONE_DIVISOR as u8);
        self.io_write(PIT_CHANNEL_2_PORT, (BEL_TONE_DIVISOR >> 8) as u8);
        let port_b = self.io_read(SYSTEM_CONTROL_PORT_B).0;
        self.io_write(SYSTEM_CONTROL_PORT_B, port_b | 0x03);
        self.bel_ticks_remaining = BEL_TONE_TICKS;
    }

    /// Stops the BEL tone when its tick countdown expires. Called from the
    /// INT 08h handler.
    pub(super) fn tick_teletype_beep(&mut self) {
        if self.bel_ticks_remaining == 0 {
            return;
        }
        self.bel_ticks_remaining -= 1;
        if self.bel_ticks_remaining == 0 {
            let port_b = self.io_read(SYSTEM_CONTROL_PORT_B).0;
            self.io_write(SYSTEM_CONTROL_PORT_B, port_b & !0x03);
        }
    }
}
