//! INT 18h graphics service handlers (AH=40..4Ah) and GDC drawing helpers.

use common::Cpu;
use device::upd7220_gdc::GdcScrollPartition;

use super::{
    super::{GRAPHICS_PAGE_SIZE_BYTES, Pc9801Bus},
    reverse_bits,
};
use crate::Tracing;

const GDC_OPE_REPLACE: u8 = 0;
const GDC_OPE_COMPLEMENT: u8 = 1;
const GDC_OPE_CLEAR: u8 = 2;
const GDC_OPE_SET: u8 = 3;

impl<T: Tracing> Pc9801Bus<T> {
    pub(super) fn int18h_graphics_display_start(&mut self) {
        self.gdc_slave.state.display_enabled = true;
        self.memory.state.ram[0x054C] |= 0x80;
    }

    pub(super) fn int18h_graphics_display_stop(&mut self) {
        self.gdc_slave.state.display_enabled = false;
        self.memory.state.ram[0x054C] &= 0x7F;
    }

    pub(super) fn int18h_display_area_set(&mut self, cpu: &mut impl Cpu) {
        const GDC_SLAVE_SYNC: [[u8; 8]; 6] = [
            [0x02, 0x26, 0x03, 0x11, 0x86, 0x0F, 0xC8, 0x94], // 15-L
            [0x02, 0x4E, 0x4B, 0x0C, 0x83, 0x06, 0xE0, 0x95], // 31-H
            [0x02, 0x26, 0x03, 0x11, 0x83, 0x07, 0x90, 0x65], // 24-L
            [0x02, 0x4E, 0x07, 0x25, 0x87, 0x07, 0x90, 0x65], // 24-M
            [0x02, 0x26, 0x41, 0x0C, 0x83, 0x0D, 0x90, 0x89], // 31-L
            [0x02, 0x4E, 0x47, 0x0C, 0x87, 0x0D, 0x90, 0x89], // 31-M
        ];

        let mode = cpu.ch();
        let modenum = [3u8, 1, 0, 2];
        let crtmode = modenum[(mode >> 6) as usize];

        // Zero scroll parameters for slave GDC (first partition).
        // Line count 0 maps to 1024 in the µPD7220 (10-bit wrap).
        self.gdc_slave.state.scroll[0] = GdcScrollPartition {
            start_address: 0x0000,
            line_count: 0x400,
            im: false,
            wd: false,
        };

        let prxdupd = self.memory.state.ram[0x054D];
        if crtmode == 2 {
            // 400-line ALL mode.
            if (prxdupd & 0x24) == 0x20 {
                self.memory.state.ram[0x054D] ^= 4;
                self.gdc_slave.load_sync_params(&GDC_SLAVE_SYNC[3]);
                self.gdc_slave.state.pitch = 80;
                self.memory.state.ram[0x054D] |= 0x08;
                self.display_control.state.mode2 |= 0x0600;
                self.apply_gdc_dot_clock();
            }
        } else {
            if (prxdupd & 0x24) == 0x24 {
                self.memory.state.ram[0x054D] ^= 4;
                // Select sync table based on PRXCRT bit 6.
                let sync_idx = if self.memory.state.ram[0x054C] & 0x40 != 0 {
                    2
                } else {
                    0
                };
                self.gdc_slave.load_sync_params(&GDC_SLAVE_SYNC[sync_idx]);
                self.gdc_slave.state.pitch = 40;
                self.memory.state.ram[0x054D] |= 0x08;
                self.display_control.state.mode2 &= !0x0600;
                self.apply_gdc_dot_clock();
            }
            if crtmode & 1 != 0 {
                // UPPER: set scroll start to page 1.
                self.gdc_slave.state.scroll[0].start_address = 200 * 40;
            }
        }

        if self.memory.state.ram[0x054D] & 4 != 0 {
            self.gdc_slave.state.scroll[0].line_count = 0x400;
        }

        // Determine 400-line vs 200-line display mode.
        let prxcrt = self.memory.state.ram[0x054C];
        if crtmode == 2 || (prxcrt & 0x40) == 0 {
            // 400-line mode: clear hide_odd_rasters, lines_per_row = 1.
            self.display_control.state.video_mode &= !0x10;
            self.gdc_slave.state.lines_per_row = 1;
        } else {
            // 200-line mode: set hide_odd_rasters, lines_per_row = 2.
            self.display_control.state.video_mode |= 0x10;
            self.gdc_slave.state.lines_per_row = 2;
        }

        // Display page selection.
        if crtmode != 3 {
            self.display_control.state.display_page = (mode >> 4) & 1;
        }

        // Graphics mode and EGC extended mode.
        if mode & 0x20 == 0 {
            self.display_control.state.video_mode &= !0x02;
            self.display_control.state.mode2 &= !0x0004;
        } else {
            self.display_control.state.video_mode |= 0x02;
            self.display_control.state.mode2 |= 0x0004;
        }

        // Store crtmode in CRT_BIOS (0x0597).
        self.memory.state.ram[0x0597] = (self.memory.state.ram[0x0597] & 0xFC) | (crtmode & 0x03);
    }

    pub(super) fn int18h_palette_set(&mut self, cpu: &mut impl Cpu) {
        let src_seg = cpu.ds();
        let src_off = cpu.bx();
        let src_base = (u32::from(src_seg) << 4).wrapping_add(u32::from(src_off));

        // Read 4 GBCPC bytes and write to digital palette ports.
        // NP21W computes degpal values then writes to ports. Port 0xA8..0xAE
        // map to digital[0..3], with reversed indexing vs NP21W's degpal.
        let mut col = [0u8; 4];
        for (i, col_byte) in col.iter_mut().enumerate() {
            *col_byte = self.read_mem_byte(src_base + 4 + i as u32);
        }
        self.palette.state.digital[0] = ((col[2] & 0x0F) << 4) | (col[0] & 0x0F);
        self.palette.state.digital[1] = ((col[3] & 0x0F) << 4) | (col[1] & 0x0F);
        self.palette.state.digital[2] = (col[2] & 0xF0) | (col[0] >> 4);
        self.palette.state.digital[3] = (col[3] & 0xF0) | (col[1] >> 4);
    }

    pub(super) fn int18h_pattern_fill(&mut self, cpu: &mut impl Cpu) {
        let src_seg = cpu.ds();
        let src_off = cpu.bx();
        let ucw_base = (u32::from(src_seg) << 4).wrapping_add(u32::from(src_off));
        let ch = cpu.ch();

        let gbon_ptn = self.read_mem_byte(ucw_base); // GBON_PTN
        let gbdotu = self.read_mem_byte(ucw_base + 2); // GBDOTU
        let x = self.read_mem_word(ucw_base + 0x08) as u32; // GBSX1
        let mut y = self.read_mem_word(ucw_base + 0x0A) as u32; // GBSY1
        let length = self.read_mem_word(ucw_base + 0x0C) as u32; // GBLNG1
        let pat_addr = self.read_mem_word(ucw_base + 0x0E); // GBWDPA

        // 200-line page offset.
        if (ch & 0xC0) == 0x40 {
            y += 200;
        }

        let all_planes = (ch & 0x30) == 0x30;
        let ds = u32::from(src_seg);

        let mut i = 0u32;
        loop {
            let pat_byte = self.read_mem_byte((ds << 4).wrapping_add(u32::from(pat_addr) + i));
            let remaining = length - i * 8;
            let bits = if remaining < 8 { remaining } else { 8 };
            let mask = if bits < 8 {
                0xFF_u8 << (8 - bits)
            } else {
                0xFF
            };
            let pat = reverse_bits(pat_byte & mask);

            let px = x + i * 8;
            let word_addr = (y * 40 + (px >> 4)) as usize;
            let bit_offset = px & 0x0F;

            if all_planes {
                for plane in 0..3u8 {
                    let ope = if gbon_ptn & (1 << plane) != 0 {
                        GDC_OPE_SET
                    } else {
                        GDC_OPE_CLEAR
                    };
                    let plane_base = (plane as usize) * 0x4000;
                    self.gdc_pset_byte(plane_base + word_addr, bit_offset, pat, ope);
                }
            } else {
                let ope = gbdotu & 3;
                let plane_sel = ((ch & 0x30) >> 4) as usize;
                let plane_base = plane_sel * 0x4000;
                self.gdc_pset_byte(plane_base + word_addr, bit_offset, pat, ope);
            }

            i += 1;
            if i * 8 >= length {
                break;
            }
        }

        // Save operation mode in PRXDUPD bits 0-1.
        let ope = gbdotu & 3;
        self.memory.state.ram[0x054D] = (self.memory.state.ram[0x054D] & !0x03) | ope;
    }

    fn gdc_pset_byte(&mut self, word_addr: usize, bit_offset: u32, pat: u8, ope: u8) {
        let page_base = self.access_page_index() * GRAPHICS_PAGE_SIZE_BYTES;
        for bit in 0..8u32 {
            let pixel_bit = (pat >> bit) & 1;
            let target_bit = bit_offset + bit;
            let addr = word_addr + (target_bit >> 4) as usize;
            let bit_in_word = (target_bit & 0x0F) as u8;

            if addr >= 0xC000 {
                continue;
            }
            let byte_idx = page_base + addr * 2 + (bit_in_word >> 3) as usize;
            let bit_mask = 0x80 >> (bit_in_word & 7);

            if byte_idx >= self.memory.state.graphics_vram.len() {
                continue;
            }

            Self::apply_gdc_dot(
                &mut self.memory.state.graphics_vram[byte_idx],
                bit_mask,
                ope,
                pixel_bit != 0,
            );
        }
    }

    pub(super) fn int18h_pattern_read(&mut self, cpu: &mut impl Cpu) {
        // Read pattern from graphics VRAM into ES:0 output buffer.
        let ucw_seg = cpu.ds();
        let ucw_off = cpu.bx();
        let ucw_base = (u32::from(ucw_seg) << 4).wrapping_add(u32::from(ucw_off));
        let out_base = u32::from(cpu.es()) << 4;

        let x = self.read_mem_word(ucw_base + 0x08); // GBSX1
        let y = self.read_mem_word(ucw_base + 0x0A); // GBSY1
        let lines = self.read_mem_word(ucw_base + 0x0C); // GBLNG1

        let pitch_bytes = 80u16;
        let word_x = x / 16;
        let mut out_offset = 0u32;

        for dy in 0..lines {
            let row = y + dy;
            if row >= 400 {
                break;
            }
            let byte_offset = u32::from(row) * u32::from(pitch_bytes) + u32::from(word_x) * 2;
            // Read from B-plane (offset 0x0000 in graphics VRAM).
            let b_lo = self.memory.state.graphics_vram[byte_offset as usize];
            let b_hi = self.memory.state.graphics_vram[byte_offset as usize + 1];
            self.write_mem_byte(out_base + out_offset, b_lo);
            self.write_mem_byte(out_base + out_offset + 1, b_hi);
            out_offset += 2;
        }
    }

    pub(super) fn int18h_vector_draw(&mut self, cpu: &mut impl Cpu) {
        let src_seg = cpu.ds();
        let src_off = cpu.bx();
        let ucw_base = (u32::from(src_seg) << 4).wrapping_add(u32::from(src_off));
        let ch = cpu.ch();

        let gbon_ptn = self.read_mem_byte(ucw_base); // GBON_PTN
        let gbdotu = self.read_mem_byte(ucw_base + 2); // GBDOTU
        let x1 = self.read_mem_word(ucw_base + 0x08) as i32; // GBSX1
        let mut y1 = self.read_mem_word(ucw_base + 0x0A) as i32; // GBSY1
        let x2 = self.read_mem_word(ucw_base + 0x16) as i32; // GBSX2
        let mut y2 = self.read_mem_word(ucw_base + 0x18) as i32; // GBSY2
        let gbdtyp = self.read_mem_byte(ucw_base + 0x28); // GBDTYP

        if (ch & 0xC0) == 0x40 {
            y1 += 200;
            y2 += 200;
        }

        // Line style pattern from GBMDOTI (bit-reversed).
        let pat_hi = reverse_bits(self.read_mem_byte(ucw_base + 0x20));
        let pat_lo = reverse_bits(self.read_mem_byte(ucw_base + 0x21));
        let pattern = ((pat_hi as u16) << 8) | pat_lo as u16;

        let ope = gbdotu & 3;

        match gbdtyp {
            0x01 => self.draw_line(x1, y1, x2, y2, pattern, gbon_ptn, ope, ch),
            0x00 | 0x02 => self.draw_rect(x1, y1, x2, y2, pattern, gbon_ptn, ope, ch),
            _ => {
                let radius = self.read_mem_word(ucw_base + 0x1C) as i32; // GBCIR
                self.draw_circle(x1, y1, radius, pattern, gbon_ptn, ope, ch);
            }
        }

        // Save operation mode in PRXDUPD bits 0-1.
        self.memory.state.ram[0x054D] = (self.memory.state.ram[0x054D] & !0x03) | ope;
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_line(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        pattern: u16,
        gbon_ptn: u8,
        ope: u8,
        ch: u8,
    ) {
        let dx = (x2 - x1).abs();
        let dy = -(y2 - y1).abs();
        let sx: i32 = if x1 < x2 { 1 } else { -1 };
        let sy: i32 = if y1 < y2 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut x = x1;
        let mut y = y1;
        let mut pat_idx = 0u32;

        loop {
            if (0..640).contains(&x) && (0..400).contains(&y) {
                let bit_in_pattern = (pattern >> (pat_idx & 15)) & 1;
                self.plot_pixel_ope(x as u16, y as u16, gbon_ptn, ope, ch, bit_in_pattern != 0);
            }
            pat_idx += 1;

            if x == x2 && y == y2 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_rect(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        pattern: u16,
        gbon_ptn: u8,
        ope: u8,
        ch: u8,
    ) {
        self.draw_line(x1, y1, x2, y1, pattern, gbon_ptn, ope, ch);
        self.draw_line(x2, y1, x2, y2, pattern, gbon_ptn, ope, ch);
        self.draw_line(x2, y2, x1, y2, pattern, gbon_ptn, ope, ch);
        self.draw_line(x1, y2, x1, y1, pattern, gbon_ptn, ope, ch);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: i32,
        pattern: u16,
        gbon_ptn: u8,
        ope: u8,
        ch: u8,
    ) {
        let mut x = radius;
        let mut y_pos = 0i32;
        let mut d = 1 - radius;
        let mut pat_idx = 0u32;

        while x >= y_pos {
            let points = [
                (cx + x, cy + y_pos),
                (cx - x, cy + y_pos),
                (cx + x, cy - y_pos),
                (cx - x, cy - y_pos),
                (cx + y_pos, cy + x),
                (cx - y_pos, cy + x),
                (cx + y_pos, cy - x),
                (cx - y_pos, cy - x),
            ];

            for &(px, py) in &points {
                if (0..640).contains(&px) && (0..400).contains(&py) {
                    let bit_in_pattern = (pattern >> (pat_idx & 15)) & 1;
                    self.plot_pixel_ope(
                        px as u16,
                        py as u16,
                        gbon_ptn,
                        ope,
                        ch,
                        bit_in_pattern != 0,
                    );
                }
            }
            pat_idx += 1;

            y_pos += 1;
            if d <= 0 {
                d += 2 * y_pos + 1;
            } else {
                x -= 1;
                d += 2 * (y_pos - x) + 1;
            }
        }
    }

    pub(super) fn int18h_graphic_char(&mut self, cpu: &mut impl Cpu) {
        let src_seg = cpu.ds();
        let src_off = cpu.bx();
        let ucw_base = (u32::from(src_seg) << 4).wrapping_add(u32::from(src_off));
        let ch = cpu.ch();

        let gbon_ptn = self.read_mem_byte(ucw_base); // GBON_PTN
        let gbdotu = self.read_mem_byte(ucw_base + 2); // GBDOTU
        let x = self.read_mem_word(ucw_base + 0x08) as u32; // GBSX1
        let mut y = self.read_mem_word(ucw_base + 0x0A) as u32; // GBSY1
        let gblng1 = self.read_mem_word(ucw_base + 0x0C); // GBLNG1
        let gblng2 = self.read_mem_word(ucw_base + 0x1E); // GBLNG2

        if (ch & 0xC0) == 0x40 {
            y += 200;
        }

        // Read 8 bytes of GBMDOTI pattern and bit-reverse each.
        let mut pat = [0u8; 8];
        for (i, pat_byte) in pat.iter_mut().enumerate() {
            *pat_byte = reverse_bits(self.read_mem_byte(ucw_base + 0x20 + i as u32));
        }

        // Height (DC+1 scan lines) and width from GBLNG1/GBLNG2.
        let height = if gblng1 != 0 { gblng2 } else { 8 } as u32;

        let all_planes = (ch & 0x30) == 0x30;

        for dy in 0..height {
            let row = y + dy;
            if row >= 400 {
                break;
            }
            let pat_byte = pat[(dy & 7) as usize];
            let word_addr = (row * 40 + (x >> 4)) as usize;
            let bit_offset = x & 0x0F;

            if all_planes {
                for plane in 0..3u8 {
                    let ope = if gbon_ptn & (1 << plane) != 0 {
                        GDC_OPE_SET
                    } else {
                        GDC_OPE_CLEAR
                    };
                    let plane_base = (plane as usize) * 0x4000;
                    self.gdc_pset_byte(plane_base + word_addr, bit_offset, pat_byte, ope);
                }
            } else {
                let ope = gbdotu & 3;
                let plane_sel = ((ch & 0x30) >> 4) as usize;
                let plane_base = plane_sel * 0x4000;
                self.gdc_pset_byte(plane_base + word_addr, bit_offset, pat_byte, ope);
            }
        }
    }

    pub(super) fn int18h_draw_mode_set(&mut self, cpu: &mut impl Cpu) {
        if self.memory.state.ram[0x054C] & 0x01 != 0 {
            return;
        }
        let mode = cpu.ch();
        self.gdc_slave.set_sync_mode_byte(mode);
        if mode & 0x10 != 0 {
            self.memory.state.ram[0x054D] &= !0x08;
        } else {
            self.memory.state.ram[0x054D] |= 0x08;
        }
    }

    /// INT 18h AH=4Dh: Select extended (256-color) or standard graphics mode
    /// while in 640x400 mode.
    ///
    /// `CH=00h` selects standard 16/8/2-color graphics, `CH=01h` selects PEGC
    /// 256-color extended graphics. Has no effect when the current graphics
    /// resolution is not 640x400. Mirrors the current selection into BDA
    /// `0000:054Dh` bit 7 so callers that inspect that flag stay in sync.
    pub(super) fn int18h_extended_graphics_select(&mut self, cpu: &mut impl Cpu) {
        if !self.machine_model.has_pegc() {
            return;
        }
        let ch = cpu.ch();
        match ch {
            0x00 => {
                self.pegc.set_256_color_enabled(false);
                self.memory.state.ram[0x054D] &= !0x80;
                self.update_plane_e_mapping();
            }
            0x01 => {
                self.pegc.set_256_color_enabled(true);
                self.memory.state.ram[0x054D] |= 0x80;
                self.memory.set_e_plane_enabled(false);
            }
            _ => {}
        }
    }

    fn apply_gdc_dot(byte: &mut u8, mask: u8, ope: u8, dot: bool) {
        match (ope & 3, dot) {
            (GDC_OPE_REPLACE, true) | (GDC_OPE_SET, true) => *byte |= mask,
            (GDC_OPE_REPLACE, false) | (GDC_OPE_CLEAR, true) => *byte &= !mask,
            (GDC_OPE_COMPLEMENT, true) => *byte ^= mask,
            _ => {}
        }
    }

    fn plot_pixel_ope(&mut self, x: u16, y: u16, gbon_ptn: u8, ope: u8, ch: u8, dot: bool) {
        let byte_offset = (u32::from(y) * 80 + u32::from(x) / 8) as usize;
        let mask = 0x80u8 >> (x & 7);
        let page_base = self.access_page_index() * GRAPHICS_PAGE_SIZE_BYTES;

        let all_planes = (ch & 0x30) == 0x30;

        if all_planes {
            for plane in 0..3u8 {
                let plane_ope = if gbon_ptn & (1 << plane) != 0 {
                    GDC_OPE_SET
                } else {
                    GDC_OPE_CLEAR
                };
                let idx = page_base + (plane as usize) * 0x8000 + byte_offset;
                if idx < self.memory.state.graphics_vram.len() {
                    Self::apply_gdc_dot(
                        &mut self.memory.state.graphics_vram[idx],
                        mask,
                        plane_ope,
                        dot,
                    );
                }
            }
        } else {
            let plane_sel = ((ch & 0x30) >> 4) as usize;
            let idx = page_base + plane_sel * 0x8000 + byte_offset;
            if idx < self.memory.state.graphics_vram.len() {
                Self::apply_gdc_dot(&mut self.memory.state.graphics_vram[idx], mask, ope, dot);
            }
        }
    }
}
