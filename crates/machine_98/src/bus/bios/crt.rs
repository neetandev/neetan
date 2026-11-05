//! INT 18h dispatcher and the CRT BIOS functions 0Ah to 1Bh of the normal mode.
//!
//! The functions follow undoc98 `int18crt.txt` and the CRT BIOS chapter of the PC-9800 Technical
//! Data Book (BIOS edition). The work area layout comes from undoc98 `memsys.txt`.

use common::{Cpu, MachineModel};

use super::{
    super::Pc9801Bus,
    cgrom_kanji_offset,
    extended_screen::{LINES_400, LINES_480, ROWS_20, ROWS_25, ROWS_30},
    real_mode_address,
};
use crate::TraceSink;

/// Work area byte with the raster count of one text row minus one (CRT_RASTER).
const CRT_RASTER_ADDRESS: usize = 0x053B;
/// Work area byte with the text screen status flags (CRT_STS_FLAG).
pub(super) const CRT_STATUS_ADDRESS: usize = 0x053C;
/// Work area word with the offset of the AH=0Fh display area list (CRT_OFST).
const CRT_LIST_OFFSET_ADDRESS: usize = 0x053E;
/// Work area word with the segment of the AH=0Fh display area list.
const CRT_LIST_SEGMENT_ADDRESS: usize = 0x0540;
/// Work area byte with the font type read by AH=14h (CRT_FONT).
const CRT_FONT_ADDRESS: usize = 0x0546;
/// Work area byte with the number of display areas set by AH=0Fh (CRT_WINDOW_NO).
const CRT_WINDOW_COUNT_ADDRESS: usize = 0x0547;
/// Work area word with the SCROLL start address of AH=0Eh (CRT_W_VRAMADR).
const CRT_SCROLL_ADDRESS_ADDRESS: usize = 0x0548;
/// Work area word with the SCROLL line count of AH=0Eh in GDC parameter form (CRT_W_RASTER).
const CRT_SCROLL_LINES_ADDRESS: usize = 0x054A;

/// CRT_STS_FLAG bit: a dedicated high resolution display is connected.
const STATUS_HIGH_RESOLUTION: u8 = 0x80;
/// CRT_STS_FLAG bit: a CRT BIOS routine waits for VSYNC.
const STATUS_VSYNC_WAIT: u8 = 0x40;
/// CRT_STS_FLAG bit: the KCG is in dot access mode.
const STATUS_KCG_DOT_ACCESS: u8 = 0x08;

/// AH=0Ah mode bit: 20 text rows.
const MODE_20_ROWS: u8 = 0x01;
/// AH=0Ah mode bit: 40 text columns.
const MODE_40_COLUMNS: u8 = 0x02;
/// AH=0Ah mode bit: attribute bit 4 selects simple graphics.
const MODE_SIMPLE_GRAPHICS: u8 = 0x04;
/// AH=0Ah mode bit: the KCG is in dot access mode.
const MODE_KCG_DOT_ACCESS: u8 = 0x08;
/// AH=0Ah mode bit: 30 text rows together with `MODE_20_ROWS` in 640x480.
const MODE_30_ROWS: u8 = 0x10;
/// AH=0Ah mode bits stored in CRT_STS_FLAG on a 640x400 display.
const MODE_STORED_BITS: u8 = 0x0F;

/// Mode F/F 1 command (port 68h): attribute bit 4 draws a vertical line.
const MODE_FF_VERTICAL_LINE: u8 = 0x00;
/// Mode F/F 1 command (port 68h): attribute bit 4 selects simple graphics.
const MODE_FF_SIMPLE_GRAPHICS: u8 = 0x01;
/// Mode F/F 1 command (port 68h): 80 text columns.
const MODE_FF_80_COLUMNS: u8 = 0x04;
/// Mode F/F 1 command (port 68h): 40 text columns.
const MODE_FF_40_COLUMNS: u8 = 0x05;
/// Mode F/F 1 command (port 68h): 6x8 dot font for a 200 line display.
const MODE_FF_FONT_6X8: u8 = 0x06;
/// Mode F/F 1 command (port 68h): 7x13 dot font for a 400 line display.
const MODE_FF_FONT_7X13: u8 = 0x07;
/// Mode F/F 1 command (port 68h): KCG code access.
const MODE_FF_KCG_CODE_ACCESS: u8 = 0x0A;
/// Mode F/F 1 command (port 68h): KCG dot access.
const MODE_FF_KCG_DOT_ACCESS: u8 = 0x0B;

/// Character cell height in rasters on a 640x200 display.
const CELL_RASTERS_200_LINE: u8 = 8;
/// Character cell height in rasters on a 640x400 display.
pub(super) const CELL_RASTERS_400_LINE: u8 = 16;
/// Display lines of a 640x200 display.
const SCREEN_LINES_200_LINE: u16 = 200;
/// Shift of the line count inside the SCROLL parameter word.
const SCROLL_LINE_COUNT_SHIFT: u16 = 4;

/// Line counter register PL (port 70h): two's complement of the first character raster.
const LINE_COUNTER_PL: usize = 0;
/// Line counter register BL (port 72h): body face rasters minus one.
const LINE_COUNTER_BL: usize = 1;
/// Line counter register CL (port 74h): character rasters.
const LINE_COUNTER_CL: usize = 2;
/// Line counter register SSL (port 76h): smooth scroll rasters.
const LINE_COUNTER_SSL: usize = 3;

/// Number of text GDC display areas.
const DISPLAY_AREA_COUNT: usize = 4;
/// Size of one AH=0Fh display area list entry in bytes.
const DISPLAY_AREA_ENTRY_SIZE: u16 = 4;

/// AH=10h value that selects a steady cursor.
const CURSOR_STEADY: u8 = 0x01;

/// AH=14h DH value for the 6x7 dot (8x8 cell) ANK font.
const FONT_ANK_8X8: u8 = 0x00;
/// AH=14h DH value for the 7x13 dot (8x16 cell) ANK font.
const FONT_ANK_8X16: u8 = 0x80;
/// AH=14h DH range of the 8x16 two byte half width characters.
const FONT_HALF_WIDTH_ROWS: std::ops::RangeInclusive<u8> = 0x29..=0x2B;
/// Font pattern buffer header: 8 rasters high and 8 dots wide.
const FONT_HEADER_8X8: u16 = 0x0101;
/// Font pattern buffer header: 16 rasters high and 8 dots wide.
const FONT_HEADER_8X16: u16 = 0x0102;
/// Font pattern buffer header: 16 rasters high and 16 dots wide.
const FONT_HEADER_16X16: u16 = 0x0202;
/// Offset of the pattern data inside the font pattern buffer.
const FONT_PATTERN_OFFSET: u32 = 2;
/// CRT_FONT bit: 16x16 dot character.
const CRT_FONT_FULL_WIDTH: u8 = 0x08;
/// CRT_FONT bit: 8x16 dot two byte half width character.
const CRT_FONT_HALF_WIDTH: u8 = 0x04;
/// CRT_FONT bit: 7x13 dot ANK character.
const CRT_FONT_ANK_8X16: u8 = 0x02;
/// CRT_FONT bit: 6x7 dot ANK character.
const CRT_FONT_ANK_8X8: u8 = 0x01;

/// CG-ROM base of the 8x16 ANK font with 16 bytes per character.
const CGROM_ANK_8X16_BASE: usize = 0x80000;
/// CG-ROM base of the 8x8 ANK font with a 16 byte stride.
const CGROM_ANK_8X8_BASE: usize = 0x82000;
/// CG-ROM stride of one ANK character.
const CGROM_ANK_STRIDE: usize = 16;
/// CG-ROM distance between the left and the right half of a kanji.
const CGROM_KANJI_RIGHT_HALF: usize = 0x800;
/// Rasters of a 16 dot high pattern.
const PATTERN_16_RASTERS: usize = 16;
/// Rasters of an 8 dot high pattern.
const PATTERN_8_RASTERS: usize = 8;

/// First JIS row of the user defined characters.
const USER_CHARACTER_ROW_FIRST: u8 = 0x76;
/// Second JIS row of the user defined characters on models with 188 characters.
const USER_CHARACTER_ROW_SECOND: u8 = 0x77;
/// Columns that accept a user character on models with 63 characters.
const USER_COLUMNS_63_CHARACTERS: std::ops::RangeInclusive<u8> = 0x20..=0x5F;
/// Columns that accept a user character on models with 188 characters.
const USER_COLUMNS_188_CHARACTERS: std::ops::RangeInclusive<u8> = 0x01..=0x80;

/// Text VRAM size of the character plane in bytes.
const TEXT_CHARACTER_PLANE_END: usize = 0x2000;
/// Text VRAM offset of the memory switch area at the end of the attribute plane.
const MEMORY_SWITCH_AREA_START: usize = 0x3FE0;

/// System PPI bit reset command for port C bit 3 (BUZ#, active low): buzzer on.
const PPI_BUZZER_ON: u8 = 0x06;
/// System PPI bit set command for port C bit 3 (BUZ#, active low): buzzer off.
const PPI_BUZZER_OFF: u8 = 0x07;

impl<T: TraceSink> Pc9801Bus<T> {
    /// INT 18h: dispatches the keyboard, CRT and graphics BIOS functions.
    pub(super) fn hle_int18h(&mut self, cpu: &mut impl Cpu) {
        match cpu.ah() {
            0x00 => self.int18h_key_read(cpu),
            0x01 => self.int18h_buffer_sense(cpu),
            0x02 => self.int18h_shift_status(cpu),
            0x03 => self.int18h_kb_init(cpu),
            0x04 => self.int18h_key_state_sense(cpu),
            0x05 => self.int18h_key_code_read(cpu),
            0x0A => self.crt_set_mode(cpu.al()),
            0x0B => cpu.set_al(self.memory.state.ram[CRT_STATUS_ADDRESS]),
            0x0C => self.gdc_master.state.display_enabled = true,
            0x0D => self.gdc_master.state.display_enabled = false,
            0x0E => self.crt_set_single_area(cpu.dx()),
            0x0F => self.crt_set_display_areas(cpu.bx(), cpu.cx(), cpu.dh(), cpu.dl()),
            0x10 => self.crt_set_cursor_form(cpu.al() & CURSOR_STEADY != 0),
            0x11 => self.gdc_master.state.cursor_display = true,
            0x12 => self.gdc_master.state.cursor_display = false,
            0x13 => self.gdc_master.state.ead = u32::from(cpu.dx() >> 1),
            0x14 => self.crt_read_font_pattern(cpu.bx(), cpu.cx(), cpu.dx()),
            0x16 => self.crt_fill_text_vram(cpu.dl(), cpu.dh()),
            0x17 => self.crt_set_buzzer(true),
            0x18 => self.crt_set_buzzer(false),
            0x1A => self.crt_define_user_character(cpu.bx(), cpu.cx(), cpu.dx()),
            0x1B => self.crt_set_kcg_access(cpu.al()),
            0x30 => self.int18h_extended_screen_mode_set(cpu),
            0x31 => self.int18h_extended_screen_mode_get(cpu),
            0x40 => self.int18h_graphics_display_start(),
            0x41 => self.int18h_graphics_display_stop(),
            0x42 => self.int18h_display_area_set(cpu),
            0x43 => self.int18h_palette_set(cpu),
            0x45 => self.int18h_pattern_fill(cpu),
            0x46 => self.int18h_pattern_read(cpu),
            0x47 | 0x48 => self.int18h_vector_draw(cpu),
            0x49 => self.int18h_graphic_char(cpu),
            0x4A => self.int18h_draw_mode_set(cpu),
            0x4D => self.int18h_extended_graphics_select(cpu),
            _ => {}
        }
    }

    /// Returns whether a dedicated high resolution display is connected (system port CRTT).
    fn crt_high_resolution(&self) -> bool {
        self.system_ppi.state.crtt
    }

    /// Returns the rasters of one text row as stored in CRT_RASTER.
    fn crt_row_rasters(&self) -> u8 {
        self.memory.state.ram[CRT_RASTER_ADDRESS].wrapping_add(1)
    }

    /// AH=0Ah: sets the text rows, columns, attribute mode and KCG access mode. In 640x480 it
    /// also accepts 30 rows, rejects bit 4 without bit 0 and hides the text screen.
    fn crt_set_mode(&mut self, mode: u8) {
        let is_480_line = self.crt_480_line_mode();
        if is_480_line && mode & MODE_30_ROWS != 0 && mode & MODE_20_ROWS == 0 {
            return;
        }
        let high_resolution = self.crt_high_resolution();
        let mut status = mode & MODE_STORED_BITS;
        if is_480_line {
            status |= mode & MODE_30_ROWS;
        }
        if high_resolution {
            status |= STATUS_HIGH_RESOLUTION;
        }
        self.memory.state.ram[CRT_STATUS_ADDRESS] = status;

        let mode_commands = [
            select(
                mode & MODE_SIMPLE_GRAPHICS != 0,
                MODE_FF_SIMPLE_GRAPHICS,
                MODE_FF_VERTICAL_LINE,
            ),
            select(
                mode & MODE_40_COLUMNS != 0,
                MODE_FF_40_COLUMNS,
                MODE_FF_80_COLUMNS,
            ),
            select(high_resolution, MODE_FF_FONT_7X13, MODE_FF_FONT_6X8),
            select(
                mode & MODE_KCG_DOT_ACCESS != 0,
                MODE_FF_KCG_DOT_ACCESS,
                MODE_FF_KCG_CODE_ACCESS,
            ),
        ];
        for command in mode_commands {
            self.display_control.write_video_mode(command);
        }

        if is_480_line {
            let rows = if mode & MODE_30_ROWS != 0 {
                ROWS_30
            } else if mode & MODE_20_ROWS != 0 {
                ROWS_20
            } else {
                ROWS_25
            };
            let is_31khz = self.display_control.is_crt_31khz_enabled();
            self.extended_screen_set_text(is_31khz, true, rows);
            self.apply_gdc_dot_clock();
            self.reschedule_gdc_events();
            return;
        }

        let cell_rasters = if high_resolution {
            CELL_RASTERS_400_LINE
        } else {
            CELL_RASTERS_200_LINE
        };
        let row_rasters = if mode & MODE_20_ROWS != 0 {
            cell_rasters + cell_rasters / 4
        } else {
            cell_rasters
        };
        self.crt_set_row_rasters(row_rasters, cell_rasters);
        self.crt_set_cursor_form(false);
    }

    /// Stores `row_rasters` in CRT_RASTER and centers a character cell of `cell_rasters` in the
    /// text row with the line counter registers.
    pub(super) fn crt_set_row_rasters(&mut self, row_rasters: u8, cell_rasters: u8) {
        self.memory.state.ram[CRT_RASTER_ADDRESS] = row_rasters - 1;

        let top_margin = (row_rasters - cell_rasters) / 2;
        self.crtc
            .write_register(LINE_COUNTER_PL, top_margin.wrapping_neg());
        self.crtc
            .write_register(LINE_COUNTER_BL, row_rasters - top_margin - 1);
        self.crtc.write_register(LINE_COUNTER_CL, cell_rasters);
        self.crtc.write_register(LINE_COUNTER_SSL, 0);
    }

    /// AH=10h: issues CSRFORM for a full row cursor and hides the cursor.
    pub(super) fn crt_set_cursor_form(&mut self, steady: bool) {
        self.memory.state.ram[CRT_STATUS_ADDRESS] &= !STATUS_VSYNC_WAIT;
        let last_raster = self.memory.state.ram[CRT_RASTER_ADDRESS];
        let gdc = &mut self.gdc_master.state;
        gdc.lines_per_row = (last_raster & 0x1F) + 1;
        gdc.cursor_display = false;
        gdc.cursor_blink = steady;
        gdc.cursor_top = 0;
        gdc.cursor_bottom = last_raster & 0x1F;
    }

    /// Returns the display lines of the whole text screen.
    fn crt_screen_lines(&self) -> u16 {
        if self.memory.state.ram[CRT_STATUS_ADDRESS] & STATUS_HIGH_RESOLUTION == 0 {
            SCREEN_LINES_200_LINE
        } else if self.crt_480_line_mode() {
            LINES_480
        } else {
            LINES_400
        }
    }

    /// AH=0Eh: shows text VRAM from one offset on the whole screen.
    pub(super) fn crt_set_single_area(&mut self, vram_offset: u16) {
        let gdc_address = vram_offset >> 1;
        let lines = self.crt_screen_lines();
        let area = &mut self.gdc_master.state.scroll[0];
        area.start_address = u32::from(gdc_address);
        area.line_count = lines;
        self.ram_write_u16(CRT_SCROLL_ADDRESS_ADDRESS, gdc_address);
        self.ram_write_u16(CRT_SCROLL_LINES_ADDRESS, lines << SCROLL_LINE_COUNT_SHIFT);
    }

    /// AH=0Fh: loads `count` display areas from the list at `segment:offset`.
    fn crt_set_display_areas(&mut self, segment: u16, offset: u16, first: u8, count: u8) {
        self.ram_write_u16(CRT_LIST_OFFSET_ADDRESS, offset);
        self.ram_write_u16(CRT_LIST_SEGMENT_ADDRESS, segment);
        self.memory.state.ram[CRT_WINDOW_COUNT_ADDRESS] = count;

        let row_rasters = u16::from(self.crt_row_rasters());
        let mut entry_offset = offset;
        for index in 0..usize::from(count) {
            let entry = real_mode_address(segment, entry_offset);
            let vram_offset = self.read_mem_word(entry);
            let rows = self.read_mem_word(entry + 2);
            let area_index = (usize::from(first) + index) % DISPLAY_AREA_COUNT;
            let area = &mut self.gdc_master.state.scroll[area_index];
            area.start_address = u32::from(vram_offset >> 1);
            area.line_count = rows.wrapping_mul(row_rasters);
            entry_offset = entry_offset.wrapping_add(DISPLAY_AREA_ENTRY_SIZE);
        }
    }

    /// AH=14h: copies a font pattern with its size header to `segment:offset`.
    fn crt_read_font_pattern(&mut self, segment: u16, offset: u16, code: u16) {
        let buffer = real_mode_address(segment, offset);
        let pattern = buffer + FONT_PATTERN_OFFSET;
        let [column, row] = code.to_le_bytes();

        match row {
            FONT_ANK_8X8 => {
                self.memory.state.ram[CRT_FONT_ADDRESS] = CRT_FONT_ANK_8X8;
                self.write_mem_word(buffer, FONT_HEADER_8X8);
                let source = CGROM_ANK_8X8_BASE + usize::from(column) * CGROM_ANK_STRIDE;
                self.crt_copy_font_rasters(source, pattern, PATTERN_8_RASTERS);
            }
            FONT_ANK_8X16 => {
                self.memory.state.ram[CRT_FONT_ADDRESS] = CRT_FONT_ANK_8X16;
                self.write_mem_word(buffer, FONT_HEADER_8X16);
                let source = CGROM_ANK_8X16_BASE + usize::from(column) * CGROM_ANK_STRIDE;
                self.crt_copy_font_rasters(source, pattern, PATTERN_16_RASTERS);
            }
            half_width if FONT_HALF_WIDTH_ROWS.contains(&half_width) => {
                self.memory.state.ram[CRT_FONT_ADDRESS] = CRT_FONT_HALF_WIDTH;
                self.write_mem_word(buffer, FONT_HEADER_8X16);
                let source = cgrom_kanji_offset(row, column) as usize;
                self.crt_copy_font_rasters(source, pattern, PATTERN_16_RASTERS);
            }
            _ => {
                self.memory.state.ram[CRT_FONT_ADDRESS] = CRT_FONT_FULL_WIDTH;
                self.write_mem_word(buffer, FONT_HEADER_16X16);
                let left = cgrom_kanji_offset(row, column) as usize;
                for raster in 0..PATTERN_16_RASTERS {
                    let left_byte = self.memory.font_read(left + raster);
                    let right_byte = self
                        .memory
                        .font_read(left + CGROM_KANJI_RIGHT_HALF + raster);
                    let destination = pattern + 2 * raster as u32;
                    self.write_mem_byte(destination, left_byte);
                    self.write_mem_byte(destination + 1, right_byte);
                }
            }
        }
    }

    /// Copies `rasters` consecutive CG-ROM bytes to guest memory.
    fn crt_copy_font_rasters(&mut self, source: usize, destination: u32, rasters: usize) {
        for raster in 0..rasters {
            let byte = self.memory.font_read(source + raster);
            self.write_mem_byte(destination + raster as u32, byte);
        }
    }

    /// AH=16h: fills the character plane with one code and the attribute plane with one value.
    fn crt_fill_text_vram(&mut self, character: u8, attribute: u8) {
        let text_vram = &mut self.memory.state.text_vram;
        for pair in text_vram[..TEXT_CHARACTER_PLANE_END].as_chunks_mut::<2>().0 {
            *pair = [character, 0x00];
        }
        for pair in text_vram[TEXT_CHARACTER_PLANE_END..MEMORY_SWITCH_AREA_START]
            .as_chunks_mut::<2>()
            .0
        {
            pair[0] = attribute;
        }
    }

    /// AH=17h and AH=18h: switches the buzzer through system port C bit 3.
    fn crt_set_buzzer(&mut self, on: bool) {
        let command = if on { PPI_BUZZER_ON } else { PPI_BUZZER_OFF };
        self.system_ppi.write_control(command);
        self.beeper.set_buzzer_enabled(on, self.current_cycle);
    }

    /// AH=1Ah: writes a 16x16 user character pattern into the KCG.
    fn crt_define_user_character(&mut self, segment: u16, offset: u16, code: u16) {
        let [column, row] = code.to_le_bytes();
        let accepted = match self.machine_model {
            MachineModel::PC9801F => {
                row == USER_CHARACTER_ROW_FIRST && USER_COLUMNS_63_CHARACTERS.contains(&column)
            }
            MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => {
                (row == USER_CHARACTER_ROW_FIRST || row == USER_CHARACTER_ROW_SECOND)
                    && USER_COLUMNS_188_CHARACTERS.contains(&column)
            }
        };
        if !accepted {
            return;
        }

        let pattern = real_mode_address(segment, offset) + FONT_PATTERN_OFFSET;
        let left = cgrom_kanji_offset(row, column) as usize;
        for raster in 0..PATTERN_16_RASTERS {
            let source = pattern + 2 * raster as u32;
            let left_byte = self.read_mem_byte(source);
            let right_byte = self.read_mem_byte(source + 1);
            self.memory.font_write(left + raster, left_byte);
            self.memory
                .font_write(left + CGROM_KANJI_RIGHT_HALF + raster, right_byte);
        }
    }

    /// AH=1Bh: selects the code access (AL=0) or the dot access (AL=1) mode of the KCG.
    fn crt_set_kcg_access(&mut self, access: u8) {
        let (dot_access, command) = match access {
            0x00 => (false, MODE_FF_KCG_CODE_ACCESS),
            0x01 => (true, MODE_FF_KCG_DOT_ACCESS),
            _ => return,
        };
        let status = &mut self.memory.state.ram[CRT_STATUS_ADDRESS];
        if dot_access {
            *status |= STATUS_KCG_DOT_ACCESS;
        } else {
            *status &= !STATUS_KCG_DOT_ACCESS;
        }
        self.display_control.write_video_mode(command);
    }
}

/// Returns `when_set` if `condition` holds and `when_clear` otherwise.
fn select(condition: bool, when_set: u8, when_clear: u8) -> u8 {
    if condition { when_set } else { when_clear }
}
