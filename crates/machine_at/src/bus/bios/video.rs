//! INT 10h video services: dispatch, mode set, cursor, pages, font services,
//! alternate select and the display combination code.
//!
//! Scroll, character, teletype and write-string services live in
//! `video_text.rs`, the pixel and glyph codecs in `video_graphics.rs`, the
//! palette services in `video_palette.rs` and the captured mode tables in
//! `video_modes.rs`. Behavior was matched against the real AMI + ET4000AX
//! BIOS probed in the emulator.

use common::{Cpu, TraceSink};
use device::vga::{
    VGA_PORT_ATC_WRITE, VGA_PORT_CRTC_DATA_COLOR, VGA_PORT_CRTC_DATA_MONO,
    VGA_PORT_CRTC_INDEX_COLOR, VGA_PORT_CRTC_INDEX_MONO, VGA_PORT_DAC_DATA, VGA_PORT_DAC_MASK,
    VGA_PORT_DAC_WRITE_INDEX, VGA_PORT_GC_DATA, VGA_PORT_GC_INDEX, VGA_PORT_HERCULES_COMPAT,
    VGA_PORT_MODE_CONTROL_COLOR, VGA_PORT_MODE_CONTROL_MONO, VGA_PORT_SEGMENT_SELECT,
    VGA_PORT_SEQ_DATA, VGA_PORT_SEQ_INDEX, VGA_PORT_STATUS_COLOR, VGA_PORT_STATUS_MONO,
    VGA_PORT_STATUS0_MISC_WRITE,
};

use super::{
    super::AtBus,
    video_modes::{
        EXTENDED_CRTC, ModeFamily, ModeFont, VGA_BIOS_SEGMENT, VGA_METADATA_FONT_8X8_UPPER,
        VGA_METADATA_FONT_8X16, VGA_METADATA_FUNCTIONALITY, VGA_METADATA_SAVE_POINTER_TABLE,
        VgaModeRegisters, VideoModeEntry, mode_entry,
    },
    video_palette::DAC_ENTRIES,
};

/// BIOS data area: current video mode.
pub(super) const BDA_VIDEO_MODE: u32 = 0x449;
/// BIOS data area: text columns (word).
pub(super) const BDA_VIDEO_COLUMNS: u32 = 0x44A;
/// BIOS data area: display page size in bytes (word).
pub(super) const BDA_VIDEO_PAGE_SIZE: u32 = 0x44C;
/// BIOS data area: active page regen start offset (word).
pub(super) const BDA_VIDEO_PAGE_START: u32 = 0x44E;
/// BIOS data area: cursor positions of the eight pages (words, col/row).
pub(super) const BDA_CURSOR_POSITIONS: u32 = 0x450;
/// BIOS data area: cursor shape (word, start/end scan line).
pub(super) const BDA_CURSOR_SHAPE: u32 = 0x460;
/// BIOS data area: active display page.
pub(super) const BDA_ACTIVE_PAGE: u32 = 0x462;
/// BIOS data area: CRTC index port base (word).
pub(super) const BDA_CRTC_BASE: u32 = 0x463;
/// BIOS data area: CGA mode select register image.
pub(super) const BDA_MODE_SELECT: u32 = 0x465;
/// BIOS data area: CGA palette register image.
pub(super) const BDA_CGA_PALETTE: u32 = 0x466;
/// BIOS data area: text rows minus one.
pub(super) const BDA_VIDEO_ROWS: u32 = 0x484;
/// BIOS data area: character cell height (word).
pub(super) const BDA_CHAR_HEIGHT: u32 = 0x485;
/// BIOS data area: video control bits (bit 7 = mode set does not clear).
pub(super) const BDA_VIDEO_CONTROL: u32 = 0x487;
/// BIOS data area: video feature switches.
pub(super) const BDA_VIDEO_SWITCHES: u32 = 0x488;
/// BIOS data area: video mode set control.
pub(super) const BDA_MODESET_CONTROL: u32 = 0x489;
/// BIOS data area: display combination code table index.
pub(super) const BDA_DCC_INDEX: u32 = 0x48A;
/// BIOS data area: SAVE_PTR, the far pointer to the video save pointer table.
pub(super) const BDA_SAVE_POINTER: u32 = 0x4A8;

/// Value the POST writes to BDA 40:89 (VGA active, 400 line default state).
const MODESET_CONTROL_DEFAULT: u8 = 0x51;
/// BDA 40:89 bit that asks the mode set to leave the DAC palette alone.
const MODESET_CONTROL_NO_PALETTE_LOAD: u8 = 0x08;
/// BDA 40:89 bit that asks the mode set to gray-scale sum the palette.
const MODESET_CONTROL_GRAY_SUM: u8 = 0x02;
/// BDA 40:89 bit pair holding the requested scan line count.
const MODESET_CONTROL_SCAN_LINES: u8 = 0x90;
/// BDA 40:89 scan line request bit for 200 lines.
const MODESET_CONTROL_200_LINES: u8 = 0x80;
/// BDA 40:89 scan line request bit for 400 lines.
const MODESET_CONTROL_400_LINES: u8 = 0x10;
/// Value the mode set writes to BDA 40:8A (DCC table index).
const DCC_INDEX_DEFAULT: u8 = 0x0B;
/// Cursor shape the mode set programs in text modes.
const TEXT_CURSOR_SHAPE: u16 = 0x0D0E;
/// Blank text cell (space, gray on black) used for the regen clear.
const TEXT_FILL: u16 = 0x0720;
/// Display combination code of the active display: VGA with color monitor.
const DCC_VGA_COLOR: u8 = 0x08;
/// AH=1Bh video memory size code: 256 KiB or more.
const VIDEO_MEMORY_256K: u8 = 0x03;
/// Save pointer table offset of the alphanumeric character set override.
const SAVE_POINTER_ALPHA_OVERRIDE: u32 = 8;
/// Save pointer table offset of the graphics character set override.
const SAVE_POINTER_GRAPHICS_OVERRIDE: u32 = 12;
/// Largest character cell height a character set override may install.
const OVERRIDE_MAX_HEIGHT: u32 = 32;
/// Largest number of glyphs a character set override may install.
const OVERRIDE_MAX_GLYPHS: u32 = 256;
/// Bytes scanned for the 0xFF terminator of an applicable mode list.
const OVERRIDE_MODE_LIST_LIMIT: u32 = 32;

impl<T: TraceSink> AtBus<T> {
    /// INT 10h video services dispatch.
    pub(super) fn hle_int10h(&mut self, cpu: &mut impl Cpu) {
        match cpu.ah() {
            0x00 => self.int10h_set_mode(cpu),
            0x01 => self.int10h_set_cursor_shape(cpu),
            0x02 => self.int10h_set_cursor_position(cpu),
            0x03 => self.int10h_read_cursor(cpu),
            0x04 => self.int10h_read_light_pen(cpu),
            0x05 => self.int10h_set_active_page(cpu),
            0x06 => self.int10h_scroll(cpu, true),
            0x07 => self.int10h_scroll(cpu, false),
            0x08 => self.int10h_read_char_attr(cpu),
            0x09 => self.int10h_write_char_attr(cpu, true),
            0x0A => self.int10h_write_char_attr(cpu, false),
            0x0B => self.int10h_cga_palette(cpu),
            0x0C => self.int10h_write_pixel(cpu),
            0x0D => self.int10h_read_pixel(cpu),
            0x0E => self.int10h_teletype(cpu),
            0x0F => self.int10h_get_mode(cpu),
            0x10 => self.int10h_palette_services(cpu),
            0x11 => self.int10h_font_services(cpu),
            0x12 => self.int10h_alternate_select(cpu),
            0x13 => self.int10h_write_string(cpu),
            0x1A => self.int10h_display_combination(cpu),
            0x1B => self.int10h_functionality_state(cpu),
            // Unknown functions (including AH=1Ch save/restore, which is
            // unsupported by design) return with all registers and flags
            // untouched, matching the probed real BIOS behavior.
            _ => {}
        }
    }

    /// Table entry of the mode currently stored in the BDA.
    pub(super) fn active_mode_entry(&mut self) -> Option<&'static VideoModeEntry> {
        let mode = self.read_mem_byte(BDA_VIDEO_MODE);
        mode_entry(mode)
    }

    /// AH=00h: sets the video mode in AL (bit 7 keeps the regen content).
    /// Unknown modes are ignored without any state change, matching the real
    /// BIOS probe.
    fn int10h_set_mode(&mut self, cpu: &mut impl Cpu) {
        self.bios_set_video_mode(cpu.al());
    }

    /// Programs a video mode: register file, font upload, INT 1Fh/43h font
    /// vectors, regen clear and the BDA video block. Also used by the POST
    /// to enter mode 03h.
    pub(crate) fn bios_set_video_mode(&mut self, requested: u8) {
        let no_clear = requested & 0x80 != 0;
        let Some(entry) = mode_entry(requested & 0x7F) else {
            return;
        };

        let modeset_control = self.read_mem_byte(BDA_MODESET_CONTROL);
        let load_palette = modeset_control & MODESET_CONTROL_NO_PALETTE_LOAD == 0;
        let gray_sum = modeset_control & MODESET_CONTROL_GRAY_SUM != 0;
        self.apply_vga_mode_registers(entry.registers, load_palette, gray_sum);

        if entry.family == ModeFamily::Text {
            self.upload_rom_font_to_plane_2(entry.registers);
        }
        self.install_font_vectors(entry.font);

        if !no_clear {
            self.clear_regen_buffer(entry);
        }

        self.write_mem_byte(BDA_VIDEO_MODE, entry.mode);
        self.write_mem_word(BDA_VIDEO_COLUMNS, entry.columns);
        self.write_mem_word(BDA_VIDEO_PAGE_SIZE, entry.page_size);
        self.write_mem_word(BDA_VIDEO_PAGE_START, 0);
        for page in 0..8u32 {
            self.write_mem_word(BDA_CURSOR_POSITIONS + page * 2, 0);
        }
        let cursor_shape = if entry.family == ModeFamily::Text {
            TEXT_CURSOR_SHAPE
        } else {
            0x0000
        };
        self.write_mem_word(BDA_CURSOR_SHAPE, cursor_shape);
        self.write_mem_byte(BDA_ACTIVE_PAGE, 0);
        self.write_mem_word(BDA_CRTC_BASE, entry.crtc_base());
        // Only the CGA generation modes update the mode select and palette
        // register images; the real BIOS leaves both bytes alone for the
        // EGA/VGA modes 0Dh and up.
        if entry.mode <= 0x07 {
            self.write_mem_byte(BDA_MODE_SELECT, entry.mode_select);
            self.write_mem_byte(BDA_CGA_PALETTE, entry.cga_palette);
        }
        self.write_mem_byte(BDA_VIDEO_ROWS, entry.rows_minus_one);
        self.write_mem_word(BDA_CHAR_HEIGHT, entry.char_height);
        let video_control = entry.video_control | if no_clear { 0x80 } else { 0x00 };
        self.write_mem_byte(BDA_VIDEO_CONTROL, video_control);
        self.write_mem_byte(BDA_VIDEO_SWITCHES, entry.switches);
        // 40:89 is deliberately not written: the probed real BIOS leaves the
        // byte alone across a mode set, so a palette load, gray-scale summing
        // or scan line request stays in effect for the next mode set too.
        self.write_mem_byte(BDA_DCC_INDEX, DCC_INDEX_DEFAULT);

        self.apply_character_set_overrides(entry);
        self.program_display_start(entry, 0);
        self.program_cursor_location(entry, 0, 0, 0);
    }

    /// Applies the character set overrides a guest published through its own
    /// copy of the save pointer table, the documented way to install a font
    /// that survives a mode set. Runs after the BDA block, whose row and cell
    /// height fields an override replaces.
    fn apply_character_set_overrides(&mut self, entry: &'static VideoModeEntry) {
        let save_pointer = self.read_mem_dword(BDA_SAVE_POINTER);
        if save_pointer == 0 {
            return;
        }
        let table = far_pointer_address(save_pointer);
        if entry.family == ModeFamily::Text {
            let pointer = self.read_mem_dword(table + SAVE_POINTER_ALPHA_OVERRIDE);
            self.apply_alpha_character_set_override(entry, pointer);
        } else {
            let pointer = self.read_mem_dword(table + SAVE_POINTER_GRAPHICS_OVERRIDE);
            self.apply_graphics_character_set_override(entry, pointer);
        }
    }

    /// Loads an alphanumeric character set override into plane 2 and adopts
    /// its cell height and row count.
    fn apply_alpha_character_set_override(&mut self, entry: &'static VideoModeEntry, pointer: u32) {
        if pointer == 0 {
            return;
        }
        let table = far_pointer_address(pointer);
        let height = u32::from(self.read_mem_byte(table));
        let block = u32::from(self.read_mem_byte(table + 1) & 0x07);
        let count = u32::from(self.read_mem_word(table + 2));
        let first_code = u32::from(self.read_mem_word(table + 4));
        let font = far_pointer_address(self.read_mem_dword(table + 6));
        let rows = self.read_mem_byte(table + 10);

        if height == 0
            || height > OVERRIDE_MAX_HEIGHT
            || count == 0
            || count > OVERRIDE_MAX_GLYPHS
            || first_code + count > OVERRIDE_MAX_GLYPHS
            || !self.override_applies_to_mode(table + 11, entry.mode)
        {
            return;
        }

        let mut bytes = vec![0u8; (count * height) as usize];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = self.read_mem_byte(font.wrapping_add(index as u32));
        }
        self.write_plane_2_glyphs(
            block * 0x2000 / 32 + first_code,
            count,
            height,
            &bytes,
            entry.registers,
        );
        self.reprogram_character_height(entry, height as u8);
        // 0xFF asks for the row count the cell height works out to, which
        // `reprogram_character_height` already stored.
        if rows != 0xFF {
            self.write_mem_byte(BDA_VIDEO_ROWS, rows.saturating_sub(1));
        }
    }

    /// Points IVT 43h at a graphics character set override and adopts its cell
    /// height and row count.
    fn apply_graphics_character_set_override(
        &mut self,
        entry: &'static VideoModeEntry,
        pointer: u32,
    ) {
        if pointer == 0 {
            return;
        }
        let table = far_pointer_address(pointer);
        let rows = self.read_mem_byte(table);
        let height = self.read_mem_word(table + 1);
        let font = self.read_mem_dword(table + 3);

        if rows == 0
            || height == 0
            || u32::from(height) > OVERRIDE_MAX_HEIGHT
            || !self.override_applies_to_mode(table + 7, entry.mode)
        {
            return;
        }

        self.write_mem_dword(0x43 * 4, font);
        self.set_graphics_font_rows(rows, height);
    }

    /// Whether a mode appears in an override's 0xFF-terminated mode list. A
    /// list without its terminator counts as no match.
    fn override_applies_to_mode(&mut self, list: u32, mode: u8) -> bool {
        for index in 0..OVERRIDE_MODE_LIST_LIMIT {
            match self.read_mem_byte(list + index) {
                0xFF => return false,
                listed if listed == mode => return true,
                _ => {}
            }
        }
        false
    }

    /// Writes one CRTC register through the mode's index/data port pair.
    pub(super) fn crtc_register_write(&mut self, crtc_base: u16, index: u8, value: u8) {
        self.io_write(crtc_base, index);
        self.io_write(crtc_base + 1, value);
    }

    /// Initializes the two BDA video fields the POST owns and the mode set
    /// never rewrites: the mode set control byte 40:89 and the video save
    /// pointer in 40:A8.
    ///
    /// Afterwards both belong to the guest. It requests scan line counts,
    /// palette loading and gray-scale summing through 40:89, and may redirect
    /// 40:A8 at its own table copy to install character set overrides.
    ///
    /// The real AMI + ET4000AX pair leaves INT 1Dh on the system BIOS dummy
    /// handler and advertises the video parameter table only through this
    /// pointer, so the HLE POST does the same.
    pub(crate) fn initialize_video_bda_state(&mut self) {
        self.write_mem_byte(BDA_MODESET_CONTROL, MODESET_CONTROL_DEFAULT);
        let offset = self.vga_rom_metadata_word(VGA_METADATA_SAVE_POINTER_TABLE);
        if offset == 0 {
            return;
        }
        self.write_mem_dword(
            BDA_SAVE_POINTER,
            (u32::from(VGA_BIOS_SEGMENT) << 16) | u32::from(offset),
        );
    }

    /// Points IVT 43h at the given ROM font and IVT 1Fh at the 8x8 upper
    /// half, both in the VGA BIOS segment.
    fn install_font_vectors(&mut self, font: ModeFont) {
        let font_offset = self.vga_rom_metadata_word(font.metadata_offset());
        self.write_mem_dword(
            0x43 * 4,
            (u32::from(VGA_BIOS_SEGMENT) << 16) | u32::from(font_offset),
        );
        let upper_offset = self.vga_rom_metadata_word(VGA_METADATA_FONT_8X8_UPPER);
        self.write_mem_dword(
            0x1F * 4,
            (u32::from(VGA_BIOS_SEGMENT) << 16) | u32::from(upper_offset),
        );
    }

    /// Uploads the ROM 8x16 font into plane 2, the character generator the
    /// text modes scan glyphs from.
    fn upload_rom_font_to_plane_2(&mut self, registers: &VgaModeRegisters) {
        let font_offset = self.vga_rom_metadata_word(VGA_METADATA_FONT_8X16) as usize;
        let mut glyphs = vec![0u8; 256 * 16];
        for (index, byte) in glyphs.iter_mut().enumerate() {
            *byte = self.memory.vga_bios_byte(font_offset + index);
        }
        self.write_plane_2_glyphs(0, 256, 16, &glyphs, registers);
    }

    /// Writes glyph bitmaps into plane 2 at 32-byte slots, then restores the
    /// sequencer and graphics controller state from the mode register file.
    pub(super) fn write_plane_2_glyphs(
        &mut self,
        first_code: u32,
        count: u32,
        height: u32,
        bytes: &[u8],
        registers: &VgaModeRegisters,
    ) {
        self.open_plane_2();
        for glyph in 0..count {
            let code = first_code + glyph;
            for row in 0..height {
                let byte = bytes[(glyph * height + row) as usize];
                self.vga.mem_write(code * 32 + row, byte);
            }
        }
        self.restore_vram_access_registers(registers);
    }

    /// Programs the sequencer and graphics controller for linear writes to
    /// plane 2 through the 64 KiB window at 0xA0000.
    fn open_plane_2(&mut self) {
        self.io_write(VGA_PORT_SEQ_INDEX, 0x02);
        self.io_write(VGA_PORT_SEQ_DATA, 0x04);
        self.io_write(VGA_PORT_SEQ_INDEX, 0x04);
        self.io_write(VGA_PORT_SEQ_DATA, 0x06);
        self.io_write(VGA_PORT_GC_INDEX, 0x05);
        self.io_write(VGA_PORT_GC_DATA, 0x00);
        self.io_write(VGA_PORT_GC_INDEX, 0x06);
        self.io_write(VGA_PORT_GC_DATA, 0x04);
        self.io_write(VGA_PORT_GC_INDEX, 0x08);
        self.io_write(VGA_PORT_GC_DATA, 0xFF);
    }

    /// Restores the VRAM access registers a transient reprogram touched from
    /// the mode register file.
    pub(super) fn restore_vram_access_registers(&mut self, registers: &VgaModeRegisters) {
        self.io_write(VGA_PORT_SEQ_INDEX, 0x02);
        self.io_write(VGA_PORT_SEQ_DATA, registers.seq[2]);
        self.io_write(VGA_PORT_SEQ_INDEX, 0x04);
        self.io_write(VGA_PORT_SEQ_DATA, registers.seq[4]);
        for index in [0x03u8, 0x04, 0x05, 0x06, 0x08] {
            self.io_write(VGA_PORT_GC_INDEX, index);
            self.io_write(VGA_PORT_GC_DATA, registers.gc[index as usize]);
        }
    }

    /// Clears the regen buffer of a freshly set mode: text pages get blank
    /// cells, graphics modes get zero fill through the device write pipeline.
    fn clear_regen_buffer(&mut self, entry: &'static VideoModeEntry) {
        match entry.family {
            ModeFamily::Text => {
                let cells = u32::from(entry.page_count) * u32::from(entry.page_size) / 2;
                for cell in 0..cells {
                    let address = entry.regen_base + cell * 2;
                    self.write_mem_word(address, TEXT_FILL);
                }
            }
            ModeFamily::Cga2 | ModeFamily::Cga4 => {
                for offset in 0..0x4000u32 {
                    self.write_mem_byte(entry.regen_base + offset, 0x00);
                }
            }
            ModeFamily::Planar => {
                // The map mask already enables all planes after the mode set.
                let plane_bytes = u32::from(entry.width / 8) * u32::from(entry.height);
                for offset in 0..plane_bytes {
                    self.write_mem_byte(entry.regen_base + offset, 0x00);
                }
            }
            ModeFamily::Packed => {
                let pixels = u32::from(entry.width) * u32::from(entry.height);
                let mut cleared = 0u32;
                while cleared < pixels {
                    let bank = cleared >> 16;
                    self.select_svga_bank(bank as u8);
                    let bank_bytes = (pixels - cleared).min(0x1_0000);
                    for offset in 0..bank_bytes {
                        self.write_mem_byte(entry.regen_base + offset, 0x00);
                    }
                    cleared += bank_bytes;
                }
                self.select_svga_bank(0);
            }
        }
    }

    /// Selects the ET4000 64 KiB read and write bank of the 0xA0000 window.
    pub(super) fn select_svga_bank(&mut self, bank: u8) {
        self.io_write(VGA_PORT_SEGMENT_SELECT, (bank << 4) | (bank & 0x0F));
    }

    /// Programs the CRTC start address for the given page. The CRTC counts
    /// character cells in text modes and plane bytes in the planar modes.
    pub(super) fn program_display_start(&mut self, entry: &'static VideoModeEntry, page: u8) {
        let page_start = u32::from(page) * u32::from(entry.page_size);
        let start = match entry.family {
            ModeFamily::Text => page_start / 2,
            _ => page_start,
        };
        let crtc_base = entry.crtc_base();
        self.crtc_register_write(crtc_base, 0x0C, (start >> 8) as u8);
        self.crtc_register_write(crtc_base, 0x0D, start as u8);
    }

    /// Programs the CRTC cursor location for a text cell on the given page.
    pub(super) fn program_cursor_location(
        &mut self,
        entry: &'static VideoModeEntry,
        page: u8,
        row: u8,
        column: u8,
    ) {
        let cell = u32::from(page) * u32::from(entry.page_size) / 2
            + u32::from(row) * u32::from(entry.columns)
            + u32::from(column);
        let crtc_base = entry.crtc_base();
        self.crtc_register_write(crtc_base, 0x0E, (cell >> 8) as u8);
        self.crtc_register_write(crtc_base, 0x0F, cell as u8);
    }

    /// Reads the cursor position of a page from the BDA as (row, column).
    pub(super) fn cursor_position(&mut self, page: u8) -> (u8, u8) {
        let word = self.read_mem_word(BDA_CURSOR_POSITIONS + u32::from(page & 0x07) * 2);
        ((word >> 8) as u8, word as u8)
    }

    /// Stores the cursor position of a page in the BDA and reprograms the
    /// CRTC when the page is the active one.
    pub(super) fn set_cursor_position(
        &mut self,
        entry: &'static VideoModeEntry,
        page: u8,
        row: u8,
        column: u8,
    ) {
        let word = (u16::from(row) << 8) | u16::from(column);
        self.write_mem_word(BDA_CURSOR_POSITIONS + u32::from(page & 0x07) * 2, word);
        if page == self.read_mem_byte(BDA_ACTIVE_PAGE) && entry.family == ModeFamily::Text {
            self.program_cursor_location(entry, page, row, column);
        }
    }

    /// AH=01h: sets the cursor shape from CX (bit 5 of CH hides the cursor).
    fn int10h_set_cursor_shape(&mut self, cpu: &mut impl Cpu) {
        let shape = cpu.cx();
        self.write_mem_word(BDA_CURSOR_SHAPE, shape);
        let crtc_base = self.read_mem_word(BDA_CRTC_BASE);
        self.crtc_register_write(crtc_base, 0x0A, cpu.ch() & 0x3F);
        self.crtc_register_write(crtc_base, 0x0B, cpu.cl() & 0x1F);
    }

    /// AH=02h: sets the cursor position of page BH to DH/DL.
    fn int10h_set_cursor_position(&mut self, cpu: &mut impl Cpu) {
        let Some(entry) = self.active_mode_entry() else {
            return;
        };
        self.set_cursor_position(entry, cpu.bh(), cpu.dh(), cpu.dl());
    }

    /// AH=03h: returns the cursor position of page BH in DX and the cursor
    /// shape in CX.
    fn int10h_read_cursor(&mut self, cpu: &mut impl Cpu) {
        let (row, column) = self.cursor_position(cpu.bh());
        cpu.set_dh(row);
        cpu.set_dl(column);
        let shape = self.read_mem_word(BDA_CURSOR_SHAPE);
        cpu.set_ch((shape >> 8) as u8);
        cpu.set_cl(shape as u8);
    }

    /// AH=04h: reads the light pen position. No light pen is attached, so
    /// AH returns zero (not triggered).
    fn int10h_read_light_pen(&mut self, cpu: &mut impl Cpu) {
        cpu.set_ah(0x00);
    }

    /// AH=05h: selects the active display page.
    fn int10h_set_active_page(&mut self, cpu: &mut impl Cpu) {
        let Some(entry) = self.active_mode_entry() else {
            return;
        };
        let page = cpu.al();
        if page >= entry.page_count {
            return;
        }
        self.write_mem_byte(BDA_ACTIVE_PAGE, page);
        let page_start = u16::from(page) * entry.page_size;
        self.write_mem_word(BDA_VIDEO_PAGE_START, page_start);
        self.program_display_start(entry, page);
        if entry.family == ModeFamily::Text {
            let (row, column) = self.cursor_position(page);
            self.program_cursor_location(entry, page, row, column);
        }
    }

    /// AH=0Fh: returns the mode (with the no-clear bit from 40:87), the
    /// column count and the active page.
    fn int10h_get_mode(&mut self, cpu: &mut impl Cpu) {
        let mode = self.read_mem_byte(BDA_VIDEO_MODE);
        let no_clear = self.read_mem_byte(BDA_VIDEO_CONTROL) & 0x80;
        cpu.set_al(mode | no_clear);
        cpu.set_ah(self.read_mem_byte(BDA_VIDEO_COLUMNS));
        let page = self.read_mem_byte(BDA_ACTIVE_PAGE);
        cpu.set_bh(page);
    }

    /// AH=11h: font services.
    fn int10h_font_services(&mut self, cpu: &mut impl Cpu) {
        match cpu.al() {
            0x00 | 0x10 => self.int10h_load_user_font(cpu, cpu.al() == 0x10),
            0x01 | 0x11 => self.int10h_load_rom_font(cpu, ModeFont::Font8x14, cpu.al() == 0x11),
            0x02 | 0x12 => self.int10h_load_rom_font(cpu, ModeFont::Font8x8, cpu.al() == 0x12),
            0x04 | 0x14 => self.int10h_load_rom_font(cpu, ModeFont::Font8x16, cpu.al() == 0x14),
            0x03 => self.int10h_set_block_specifier(cpu),
            0x20 => self.int10h_set_int1fh_font(cpu),
            0x21 => self.int10h_set_int43h_user_font(cpu),
            0x22 => self.int10h_set_int43h_rom_font(cpu, ModeFont::Font8x14),
            0x23 => self.int10h_set_int43h_rom_font(cpu, ModeFont::Font8x8),
            0x24 => self.int10h_set_int43h_rom_font(cpu, ModeFont::Font8x16),
            0x30 => self.int10h_get_font_information(cpu),
            _ => self.set_iret_cf(cpu, true),
        }
    }

    /// AH=11h AL=00h/10h: loads a user font from ES:BP into plane 2. The
    /// AL=10h variant reprograms the character height afterwards.
    fn int10h_load_user_font(&mut self, cpu: &mut impl Cpu, reprogram: bool) {
        let Some(entry) = self.active_mode_entry() else {
            return;
        };
        let height = u32::from(cpu.bh());
        let count = u32::from(cpu.cx());
        let first_code = u32::from(cpu.dx());
        let block = u32::from(cpu.bl() & 0x07);
        let source = (u32::from(cpu.es()) << 4).wrapping_add(u32::from(cpu.bp()));

        let mut bytes = vec![0u8; (count * height) as usize];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = self.read_mem_byte(source.wrapping_add(index as u32));
        }
        self.write_plane_2_glyphs(
            block * 0x2000 / 32 + first_code,
            count,
            height,
            &bytes,
            entry.registers,
        );
        if reprogram {
            self.reprogram_character_height(entry, cpu.bh());
        }
    }

    /// AH=11h AL=01h/02h/04h and 11h/12h/14h: loads a ROM font into plane 2.
    fn int10h_load_rom_font(&mut self, cpu: &mut impl Cpu, font: ModeFont, reprogram: bool) {
        let Some(entry) = self.active_mode_entry() else {
            return;
        };
        let block = u32::from(cpu.bl() & 0x07);
        let height = font.height();
        let font_offset = self.vga_rom_metadata_word(font.metadata_offset()) as usize;
        let mut bytes = vec![0u8; (256 * height) as usize];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = self.memory.vga_bios_byte(font_offset + index);
        }
        self.write_plane_2_glyphs(block * 0x2000 / 32, 256, height, &bytes, entry.registers);
        if reprogram {
            self.reprogram_character_height(entry, height as u8);
        }
    }

    /// Reprograms the CRTC maximum scan line and cursor shape for a new
    /// character height and updates the BDA rows and height fields.
    fn reprogram_character_height(&mut self, entry: &'static VideoModeEntry, height: u8) {
        let height = height.max(1);
        let crtc_base = entry.crtc_base();

        self.io_write(crtc_base, 0x09);
        let max_scan_line = self.io_read(crtc_base + 1).0;
        self.crtc_register_write(
            crtc_base,
            0x09,
            (max_scan_line & 0xE0) | ((height - 1) & 0x1F),
        );
        self.crtc_register_write(crtc_base, 0x0A, height.saturating_sub(2));
        self.crtc_register_write(crtc_base, 0x0B, height - 1);
        let shape = (u16::from(height.saturating_sub(2)) << 8) | u16::from(height - 1);
        self.write_mem_word(BDA_CURSOR_SHAPE, shape);

        // 400 scan lines of a VGA text mode divided by the new cell height.
        let rows = (400 / u16::from(height)).max(1) as u8;
        self.write_mem_byte(BDA_VIDEO_ROWS, rows - 1);
        self.write_mem_word(BDA_CHAR_HEIGHT, u16::from(height));
    }

    /// AH=11h AL=03h: selects the character generator blocks (SEQ 03h).
    fn int10h_set_block_specifier(&mut self, cpu: &mut impl Cpu) {
        self.io_write(VGA_PORT_SEQ_INDEX, 0x03);
        self.io_write(VGA_PORT_SEQ_DATA, cpu.bl());
    }

    /// AH=11h AL=20h: points IVT 1Fh at the user graphics font in ES:BP.
    fn int10h_set_int1fh_font(&mut self, cpu: &mut impl Cpu) {
        let pointer = (u32::from(cpu.es()) << 16) | u32::from(cpu.bp());
        self.write_mem_dword(0x1F * 4, pointer);
    }

    /// AH=11h AL=21h: points IVT 43h at the user graphics font in ES:BP and
    /// updates the BDA rows and height.
    fn int10h_set_int43h_user_font(&mut self, cpu: &mut impl Cpu) {
        let pointer = (u32::from(cpu.es()) << 16) | u32::from(cpu.bp());
        self.write_mem_dword(0x43 * 4, pointer);
        self.apply_graphics_font_rows(cpu, cpu.cx());
    }

    /// AH=11h AL=22h/23h/24h: points IVT 43h at a ROM font and updates the
    /// BDA rows and height.
    fn int10h_set_int43h_rom_font(&mut self, cpu: &mut impl Cpu, font: ModeFont) {
        let font_offset = self.vga_rom_metadata_word(font.metadata_offset());
        self.write_mem_dword(
            0x43 * 4,
            (u32::from(VGA_BIOS_SEGMENT) << 16) | u32::from(font_offset),
        );
        self.apply_graphics_font_rows(cpu, font.height() as u16);
    }

    /// Applies the BL row specifier and character height of the AH=11h
    /// AL=20h-24h graphics font services to the BDA.
    fn apply_graphics_font_rows(&mut self, cpu: &impl Cpu, height: u16) {
        let rows = match cpu.bl() {
            0x00 => cpu.dl(),
            0x01 => 14,
            0x03 => 43,
            _ => 25,
        };
        self.set_graphics_font_rows(rows, height);
    }

    /// Stores the displayed row count and character cell height of a graphics
    /// mode font in the BDA.
    fn set_graphics_font_rows(&mut self, rows: u8, height: u16) {
        self.write_mem_byte(BDA_VIDEO_ROWS, rows.saturating_sub(1));
        self.write_mem_word(BDA_CHAR_HEIGHT, height);
    }

    /// AH=11h AL=30h: returns font information: ES:BP pointer, CX height,
    /// DL rows.
    fn int10h_get_font_information(&mut self, cpu: &mut impl Cpu) {
        let pointer = match cpu.bh() {
            0x00 => self.read_mem_dword(0x1F * 4),
            0x01 => self.read_mem_dword(0x43 * 4),
            0x02 | 0x05 => self.rom_font_pointer(ModeFont::Font8x14),
            0x03 => self.rom_font_pointer(ModeFont::Font8x8),
            0x04 => {
                let offset = self.vga_rom_metadata_word(VGA_METADATA_FONT_8X8_UPPER);
                (u32::from(VGA_BIOS_SEGMENT) << 16) | u32::from(offset)
            }
            0x06 | 0x07 => self.rom_font_pointer(ModeFont::Font8x16),
            _ => {
                self.set_iret_cf(cpu, true);
                return;
            }
        };
        cpu.set_es((pointer >> 16) as u16);
        cpu.set_bp(pointer as u16);
        let height = self.read_mem_word(BDA_CHAR_HEIGHT);
        cpu.set_cx(height);
        let rows = self.read_mem_byte(BDA_VIDEO_ROWS);
        cpu.set_dl(rows);
    }

    /// Far pointer to a ROM font in the VGA BIOS segment.
    fn rom_font_pointer(&mut self, font: ModeFont) -> u32 {
        let offset = self.vga_rom_metadata_word(font.metadata_offset());
        (u32::from(VGA_BIOS_SEGMENT) << 16) | u32::from(offset)
    }

    /// AH=12h: alternate select services, dispatched on BL.
    fn int10h_alternate_select(&mut self, cpu: &mut impl Cpu) {
        match cpu.bl() {
            0x10 => {
                // EGA information: color/mono from misc output bit 0, 256 KiB
                // of video memory, feature bits clear, switches from the BDA.
                let color = self.vga.misc_output & 0x01 != 0;
                cpu.set_bh(if color { 0x00 } else { 0x01 });
                cpu.set_bl(0x03);
                cpu.set_ch(0x00);
                let switches = self.read_mem_byte(BDA_VIDEO_SWITCHES) & 0x0F;
                cpu.set_cl(switches);
            }
            0x20 => {
                // Alternate print screen: accepted as a no-op.
            }
            0x30 => {
                // Scan line request for the next mode set: 0=200, 1=350,
                // 2=400 lines, stored in the modeset control bits. Every
                // captured register file is 400 line, so the request is
                // recorded but not honored.
                let lines = cpu.al();
                if lines > 0x02 {
                    return;
                }
                let control = self.read_mem_byte(BDA_MODESET_CONTROL) & !MODESET_CONTROL_SCAN_LINES;
                let bits = match lines {
                    0x00 => MODESET_CONTROL_200_LINES,
                    0x01 => 0x00,
                    _ => MODESET_CONTROL_400_LINES,
                };
                self.write_mem_byte(BDA_MODESET_CONTROL, control | bits);
                cpu.set_al(0x12);
            }
            0x31 => {
                // Palette load enable/disable on mode set.
                let control =
                    self.read_mem_byte(BDA_MODESET_CONTROL) & !MODESET_CONTROL_NO_PALETTE_LOAD;
                let bit = if cpu.al() & 0x01 != 0 {
                    MODESET_CONTROL_NO_PALETTE_LOAD
                } else {
                    0x00
                };
                self.write_mem_byte(BDA_MODESET_CONTROL, control | bit);
                cpu.set_al(0x12);
            }
            0x32 => {
                // Video enable/disable through the ATC palette address source.
                let status_port = if self.vga.misc_output & 0x01 != 0 {
                    VGA_PORT_STATUS_COLOR
                } else {
                    VGA_PORT_STATUS_MONO
                };
                let _ = self.io_read(status_port);
                let index = if cpu.al() & 0x01 != 0 { 0x00 } else { 0x20 };
                self.io_write(VGA_PORT_ATC_WRITE, index);
                cpu.set_al(0x12);
            }
            0x33 => {
                // Gray-scale summing enable (AL=0) or disable (AL=1) for the
                // palette every following mode set loads.
                let control = self.read_mem_byte(BDA_MODESET_CONTROL) & !MODESET_CONTROL_GRAY_SUM;
                let bit = if cpu.al() & 0x01 == 0 {
                    MODESET_CONTROL_GRAY_SUM
                } else {
                    0x00
                };
                self.write_mem_byte(BDA_MODESET_CONTROL, control | bit);
                cpu.set_al(0x12);
            }
            0x34 => {
                // Cursor emulation flag.
                let control = self.read_mem_byte(BDA_VIDEO_CONTROL) & !0x01;
                let bit = if cpu.al() & 0x01 != 0 { 0x01 } else { 0x00 };
                self.write_mem_byte(BDA_VIDEO_CONTROL, control | bit);
                cpu.set_al(0x12);
            }
            0x36 => {
                // Screen off/on through the sequencer clocking mode bit.
                self.io_write(VGA_PORT_SEQ_INDEX, 0x01);
                let clocking = self.io_read(VGA_PORT_SEQ_DATA).0 & !0x20;
                let bit = if cpu.al() & 0x01 != 0 { 0x20 } else { 0x00 };
                self.io_write(VGA_PORT_SEQ_INDEX, 0x01);
                self.io_write(VGA_PORT_SEQ_DATA, clocking | bit);
                cpu.set_al(0x12);
            }
            _ => self.set_iret_cf(cpu, true),
        }
    }

    /// AH=1Ah: display combination code. Read returns VGA with color
    /// monitor; write accepts and stores the requested code.
    fn int10h_display_combination(&mut self, cpu: &mut impl Cpu) {
        match cpu.al() {
            0x00 => {
                cpu.set_al(0x1A);
                cpu.set_bl(DCC_VGA_COLOR);
                cpu.set_bh(0x00);
            }
            0x01 => {
                cpu.set_al(0x1A);
            }
            _ => self.set_iret_cf(cpu, true),
        }
    }

    /// AH=1Bh: functionality/state information. Fills the 64-byte buffer at
    /// ES:DI with the static table pointer, the BDA video state and the
    /// per-mode capabilities, byte matching the captured real BIOS output.
    fn int10h_functionality_state(&mut self, cpu: &mut impl Cpu) {
        let Some(entry) = self.active_mode_entry() else {
            return;
        };
        let buffer = (u32::from(cpu.es()) << 4).wrapping_add(u32::from(cpu.di()));
        for offset in 0..64u32 {
            self.write_mem_byte(buffer + offset, 0x00);
        }

        let table_offset = self.vga_rom_metadata_word(VGA_METADATA_FUNCTIONALITY);
        self.write_mem_word(buffer, table_offset);
        self.write_mem_word(buffer + 2, VGA_BIOS_SEGMENT);

        // A verbatim copy of BDA 40:49-66: mode, columns, page size and
        // start, the cursor positions and shape, active page, CRTC base and
        // the mode select and palette register images.
        for offset in 0..0x1E_u32 {
            let value = self.read_mem_byte(BDA_VIDEO_MODE + offset);
            self.write_mem_byte(buffer + 4 + offset, value);
        }

        let rows = self.read_mem_byte(BDA_VIDEO_ROWS) + 1;
        self.write_mem_byte(buffer + 0x22, rows);
        let char_height = self.read_mem_word(BDA_CHAR_HEIGHT);
        self.write_mem_word(buffer + 0x23, char_height);
        self.write_mem_byte(buffer + 0x25, DCC_VGA_COLOR);
        self.write_mem_word(buffer + 0x27, entry.color_count);
        self.write_mem_byte(buffer + 0x29, entry.page_count);
        self.write_mem_byte(buffer + 0x2A, entry.scan_line_code);
        let blink = self.vga.atc[0x10] & 0x08 != 0;
        let misc_flags = if blink { 0x31 } else { 0x11 };
        self.write_mem_byte(buffer + 0x2D, misc_flags);
        self.write_mem_byte(buffer + 0x31, VIDEO_MEMORY_256K);

        cpu.set_al(0x1B);
    }

    /// Applies a VGA register file through the internal I/O dispatch, the same
    /// port sequence the real BIOS mode set uses (KEY unlock included),
    /// leaving the adapter ready to scan out the mode.
    ///
    /// `load_palette` and `gray_sum` come from the BDA 40:89 request bits. A
    /// guest that disabled palette loading keeps the DAC contents it had, and
    /// no palette load means nothing to sum either.
    pub(crate) fn apply_vga_mode_registers(
        &mut self,
        mode: &VgaModeRegisters,
        load_palette: bool,
        gray_sum: bool,
    ) {
        self.io_write(VGA_PORT_STATUS0_MISC_WRITE, mode.misc);
        let color = mode.misc & 0x01 != 0;
        let (crtc_index_port, crtc_data_port, mode_control_port, status_port) = if color {
            (
                VGA_PORT_CRTC_INDEX_COLOR,
                VGA_PORT_CRTC_DATA_COLOR,
                VGA_PORT_MODE_CONTROL_COLOR,
                VGA_PORT_STATUS_COLOR,
            )
        } else {
            (
                VGA_PORT_CRTC_INDEX_MONO,
                VGA_PORT_CRTC_DATA_MONO,
                VGA_PORT_MODE_CONTROL_MONO,
                VGA_PORT_STATUS_MONO,
            )
        };

        self.io_write(VGA_PORT_HERCULES_COMPAT, 0x03);
        self.io_write(mode_control_port, 0xA0);

        for (index, value) in mode.seq.iter().enumerate() {
            self.io_write(VGA_PORT_SEQ_INDEX, index as u8);
            self.io_write(VGA_PORT_SEQ_DATA, *value);
        }

        // Lift the CRTC write protection before loading 0x00-0x07, then
        // restore the captured vertical retrace end value last.
        self.io_write(crtc_index_port, 0x11);
        self.io_write(crtc_data_port, mode.crtc[0x11] & 0x7F);
        for (index, value) in mode.crtc.iter().enumerate() {
            if index == 0x11 {
                continue;
            }
            self.io_write(crtc_index_port, index as u8);
            self.io_write(crtc_data_port, *value);
        }
        for (index, value) in EXTENDED_CRTC {
            self.io_write(crtc_index_port, index);
            self.io_write(crtc_data_port, value);
        }
        self.io_write(crtc_index_port, 0x11);
        self.io_write(crtc_data_port, mode.crtc[0x11]);

        for (index, value) in mode.gc.iter().enumerate() {
            self.io_write(VGA_PORT_GC_INDEX, index as u8);
            self.io_write(VGA_PORT_GC_DATA, *value);
        }

        // Reset the attribute flip-flop to index phase, load the registers
        // with the palette address source clear, then re-enable the display.
        let _ = self.io_read(status_port);
        for (index, value) in mode.atc.iter().enumerate() {
            self.io_write(VGA_PORT_ATC_WRITE, index as u8);
            self.io_write(VGA_PORT_ATC_WRITE, *value);
        }
        self.io_write(VGA_PORT_ATC_WRITE, 0x20);

        if load_palette {
            self.io_write(VGA_PORT_DAC_MASK, 0xFF);
            self.io_write(VGA_PORT_DAC_WRITE_INDEX, 0x00);
            for &component in mode.palette.iter() {
                self.io_write(VGA_PORT_DAC_DATA, component);
            }
            if gray_sum {
                self.gray_scale_sum_dac(0, DAC_ENTRIES);
            }
        }

        self.io_write(VGA_PORT_SEGMENT_SELECT, mode.segment_select);
    }
}

/// Linear address a far pointer stored as a dword (segment high, offset low)
/// resolves to in real mode.
fn far_pointer_address(pointer: u32) -> u32 {
    ((pointer >> 16) << 4).wrapping_add(pointer & 0xFFFF)
}
