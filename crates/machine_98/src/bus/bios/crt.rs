//! INT 18h CRT/text/cursor/font/beep service handlers (AH=0A..1Bh)
//! plus the INT 18h dispatcher itself.

use common::Cpu;

use super::{super::Pc9801Bus, cgrom_kanji_offset};
use crate::Tracing;

impl<T: Tracing> Pc9801Bus<T> {
    pub(super) fn hle_int18h(&mut self, cpu: &mut impl Cpu) {
        match cpu.ah() {
            0x00 => self.int18h_key_read(cpu),
            0x01 => self.int18h_buffer_sense(cpu),
            0x02 => self.int18h_shift_status(cpu),
            0x03 => self.int18h_kb_init(cpu),
            0x04 => self.int18h_key_state_sense(cpu),
            0x05 => self.int18h_key_code_read(cpu),
            0x0A => self.int18h_crt_mode_set(cpu),
            0x0B => self.int18h_crt_mode_sense(cpu),
            0x0C => self.int18h_text_display_start(),
            0x0D => self.int18h_text_display_stop(),
            0x0E => self.int18h_single_display_area(cpu),
            0x0F => self.int18h_multi_display_area(cpu),
            0x10 => self.int18h_cursor_blink(cpu),
            0x11 => self.int18h_cursor_display_start(),
            0x12 => self.int18h_cursor_display_stop(),
            0x13 => self.int18h_cursor_position_set(cpu),
            0x14 => self.int18h_font_pattern_read(cpu),
            0x16 => self.int18h_text_vram_init(cpu),
            0x17 => self.int18h_beep_on(),
            0x18 => self.int18h_beep_off(),
            0x1A => self.int18h_user_char_define(cpu),
            0x1B => self.int18h_kcg_access_mode(cpu),
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

    fn int18h_crt_mode_set(&mut self, cpu: &mut impl Cpu) {
        let mode = cpu.al();

        // Clear mode1 bits: atr_sel(0x01), column_width(0x04), font_sel(0x08), KAC(0x20).
        self.display_control.state.video_mode &= !0x2D;

        // Store mode in CRT_STS_FLAG.
        self.memory.state.ram[0x053C] = mode;

        // 400-line text mode when CRTT is set (DIP SW 1-1 = normal mode).
        // Raster/line parameters: 200-line uses index 0/1, 400-line uses index 2/3.
        //                  raster, pl,   bl,   cl
        // 200-20:          0x09,   0x1F, 0x08, 0x08
        // 200-25:          0x07,   0x00, 0x07, 0x08
        // 400-20:          0x13,   0x1E, 0x11, 0x10
        // 400-25:          0x0F,   0x00, 0x0F, 0x10
        let is_hires = self.system_ppi.state.crtt;
        if is_hires {
            self.memory.state.ram[0x053C] |= 0x80;
            self.display_control.state.video_mode |= 0x08; // font_sel
        }

        if mode & 0x02 != 0 {
            self.display_control.state.video_mode |= 0x04; // column_width (40 columns)
        }
        if mode & 0x04 != 0 {
            self.display_control.state.video_mode |= 0x01; // atr_sel
        }
        if mode & 0x08 != 0 {
            self.display_control.state.video_mode |= 0x20; // KAC dot access mode
        }

        // CRTC parameters: (raster, pl, bl, cl)
        //   200-25: (0x07, 0x00, 0x07, 0x08)
        //   200-20: (0x09, 0x1F, 0x08, 0x08)
        //   400-25: (0x0F, 0x00, 0x0F, 0x10)
        //   400-20: (0x13, 0x1E, 0x11, 0x10)
        let is_20_line = mode & 0x01 != 0;
        let (raster, pl, bl, cl) = match (is_hires, is_20_line) {
            (false, false) => (0x07u8, 0x00u8, 0x07u8, 0x08u8),
            (false, true) => (0x09, 0x1F, 0x08, 0x08),
            (true, false) => (0x0F, 0x00, 0x0F, 0x10),
            (true, true) => (0x13, 0x1E, 0x11, 0x10),
        };
        self.memory.state.ram[0x053B] = raster;

        // Update master GDC text lines_per_row from raster count.
        self.gdc_master.state.lines_per_row = (raster & 0x1F) + 1;

        // Update CRTC registers.
        self.crtc.state.regs[0] = pl;
        self.crtc.state.regs[1] = bl;
        self.crtc.state.regs[2] = cl;
        self.crtc.state.regs[3] = 0; // SSL = 0

        // Reset cursor blink after mode change.
        self.int18h_cursor_blink_inner(0);
    }

    fn int18h_crt_mode_sense(&mut self, cpu: &mut impl Cpu) {
        let mode = self.memory.state.ram[0x053C];
        cpu.set_al(mode);
    }

    fn int18h_text_display_start(&mut self) {
        self.gdc_master.state.display_enabled = true;
    }

    fn int18h_text_display_stop(&mut self) {
        self.gdc_master.state.display_enabled = false;
    }

    fn int18h_single_display_area(&mut self, cpu: &mut impl Cpu) {
        let vram_word_addr = cpu.dx() / 2;
        self.gdc_master.state.scroll[0].start_address = u32::from(vram_word_addr);

        // Set line count: 200 lines * raster, doubled for hi-res.
        let crt_sts_flag = self.memory.state.ram[0x053C];
        let mut raster: u16 = 200 << 4;
        if crt_sts_flag & 0x80 != 0 {
            raster <<= 1;
        }
        self.gdc_master.state.scroll[0].line_count = raster;

        // Update BDA fields.
        self.ram_write_u16(0x0548, vram_word_addr);
        self.ram_write_u16(0x054A, raster);
    }

    fn int18h_multi_display_area(&mut self, cpu: &mut impl Cpu) {
        let table_seg = cpu.bx();
        let mut table_off = cpu.cx();
        let start_area = cpu.dh() as usize;
        let count = cpu.dl() as usize;

        // Store table pointer and area info in BDA.
        self.ram_write_u16(0x053E, table_off);
        self.ram_write_u16(0x0540, table_seg);
        self.memory.state.ram[0x0547] = cpu.dh();
        self.memory.state.ram[0x053D] = cpu.dl();

        let crt_sts_flag = self.memory.state.ram[0x053C];
        let raster: u32 = if crt_sts_flag & 0x01 == 0 {
            // 25-line mode.
            8 << 4
        } else {
            // 20-line mode.
            16 << 4
        };
        let raster = if crt_sts_flag & 0x80 != 0 {
            raster << 1
        } else {
            raster
        };

        let seg = u32::from(table_seg);

        for i in 0..count {
            let area_index = start_area + i;
            if area_index >= 4 {
                break;
            }
            let entry_addr = (seg << 4).wrapping_add(u32::from(table_off));
            let addr_word = self.read_mem_word(entry_addr) >> 1;
            let lines = self.read_mem_word(entry_addr + 2) as u32 * raster;

            self.gdc_master.state.scroll[area_index].start_address = u32::from(addr_word);
            self.gdc_master.state.scroll[area_index].line_count = lines as u16;
            table_off = table_off.wrapping_add(4);
        }
    }

    fn int18h_cursor_blink(&mut self, cpu: &mut impl Cpu) {
        self.int18h_cursor_blink_inner(cpu.al() & 1);
    }

    pub(super) fn int18h_cursor_blink_inner(&mut self, curdel: u8) {
        let sts = self.memory.state.ram[0x053C];
        self.memory.state.ram[0x053C] = sts & !0x40;

        // Determine cursor form index from mode.
        // 200-25=0, 200-20=1, 400-25=2, 400-20=3
        let mut pos = sts & 0x01;
        if sts & 0x80 != 0 {
            pos += 2;
        }

        self.memory.state.ram[0x053D] = curdel << 5;
        self.gdc_master.state.cursor_blink_rate = curdel << 5;
        self.gdc_master.state.cursor_blink = curdel != 0;

        // Set cursor form: raster and cursor lines per mode.
        let raster = self.memory.state.ram[0x053B];
        self.gdc_master.state.lines_per_row = (raster & 0x1F) + 1;
        let cursor_bottom = [0x07u8, 0x09, 0x0F, 0x13][pos as usize];
        self.gdc_master.state.cursor_top = 0;
        self.gdc_master.state.cursor_bottom = cursor_bottom;
        self.gdc_master.state.cursor_display = false;
    }

    fn int18h_cursor_display_start(&mut self) {
        self.gdc_master.state.cursor_display = true;
    }

    fn int18h_cursor_display_stop(&mut self) {
        self.gdc_master.state.cursor_display = false;
    }

    fn int18h_cursor_position_set(&mut self, cpu: &mut impl Cpu) {
        let word_addr = cpu.dx() / 2;
        self.gdc_master.state.ead = u32::from(word_addr);
    }

    fn int18h_font_pattern_read(&mut self, cpu: &mut impl Cpu) {
        let char_code = cpu.dx();
        let dest_seg = cpu.bx();
        let dest_off = cpu.cx();
        let dest_base = (u32::from(dest_seg) << 4).wrapping_add(u32::from(dest_off));

        let high_byte = (char_code >> 8) as u8;
        match high_byte {
            0x00 => {
                // 8x8 font, header 0x0101, 8 bytes from fontrom + 0x82000.
                self.write_mem_word(dest_base, 0x0101);
                let font_base = 0x82000 + (char_code as u8 as usize) * 16;
                for i in 0..8 {
                    let byte = self.memory.font_read(font_base + i);
                    self.write_mem_byte(dest_base + 2 + i as u32, byte);
                }
            }
            0x29..=0x2B => {
                // 8x16 half-width kanji subset, header 0x0102.
                self.write_mem_word(dest_base, 0x0102);
                let font_offset = cgrom_kanji_offset(high_byte, char_code as u8) as usize;
                for i in 0..16 {
                    let byte = self.memory.font_read(font_offset + i);
                    self.write_mem_byte(dest_base + 2 + i as u32, byte);
                }
            }
            0x80 => {
                // 8x16 ANK, header 0x0102, 16 bytes from fontrom + 0x80000.
                self.write_mem_word(dest_base, 0x0102);
                let font_base = 0x80000 + (char_code as u8 as usize) * 16;
                for i in 0..16 {
                    let byte = self.memory.font_read(font_base + i);
                    self.write_mem_byte(dest_base + 2 + i as u32, byte);
                }
            }
            _ => {
                // 16x16 kanji, header 0x0202.
                self.write_mem_word(dest_base, 0x0202);
                let jis_col = char_code as u8;
                let font_offset = cgrom_kanji_offset(high_byte, jis_col) as usize;
                for i in 0..16 {
                    let left = self.memory.font_read(font_offset + i);
                    let right = self.memory.font_read(font_offset + 0x800 + i);
                    self.write_mem_byte(dest_base + 2 + (i as u32) * 2, left);
                    self.write_mem_byte(dest_base + 2 + (i as u32) * 2 + 1, right);
                }
            }
        }
    }

    fn int18h_text_vram_init(&mut self, cpu: &mut impl Cpu) {
        let char_byte = cpu.dl();
        let attr_byte = cpu.dh();

        // Fill character plane with char_byte at even offsets.
        for i in (0..0x2000).step_by(2) {
            self.memory.state.text_vram[i] = char_byte;
            self.memory.state.text_vram[i + 1] = 0x00;
        }
        // Fill attribute plane with attr_byte at even offsets (up to 0x3FE0).
        for i in (0x2000..0x3FE0).step_by(2) {
            self.memory.state.text_vram[i] = attr_byte;
        }
    }

    fn int18h_beep_on(&mut self) {
        self.beeper.state.buzzer_enabled = true;
    }

    fn int18h_beep_off(&mut self) {
        self.beeper.state.buzzer_enabled = false;
    }

    fn int18h_user_char_define(&mut self, cpu: &mut impl Cpu) {
        let char_code = cpu.dx();
        let jis_row = (char_code >> 8) as u8;
        let jis_col = char_code as u8;

        // Only rows 0x76/0x77 are user-definable.
        if (jis_row & 0x7E) != 0x76 {
            return;
        }

        let src_seg = cpu.bx();
        let src_off = cpu.cx();
        let src_base = (u32::from(src_seg) << 4).wrapping_add(u32::from(src_off));

        // Skip 2-byte size header, read 32 bytes of interleaved font data.
        // Input format: [left0, right0, left1, right1, ..., left15, right15].
        let font_offset = cgrom_kanji_offset(jis_row, jis_col) as usize;

        for i in 0..16 {
            let left = self.read_mem_byte(src_base + 2 + (i as u32) * 2);
            let right = self.read_mem_byte(src_base + 2 + (i as u32) * 2 + 1);
            self.memory.font_write(font_offset + i, left);
            self.memory.font_write(font_offset + 0x800 + i, right);
        }
    }

    fn int18h_kcg_access_mode(&mut self, cpu: &mut impl Cpu) {
        match cpu.al() {
            0 => {
                self.memory.state.ram[0x053C] &= !0x08;
                self.display_control.state.video_mode &= !0x20; // code access mode
            }
            1 => {
                self.memory.state.ram[0x053C] |= 0x08;
                self.display_control.state.video_mode |= 0x20; // dot access mode
            }
            _ => {}
        }
    }
}
