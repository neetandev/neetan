//! INT 18h graphics service handlers (AH=40..4Ah) and GDC drawing helpers.

use common::{Cpu, MachineModel};
use device::grcg;

use super::{
    super::{GRAPHICS_PAGE_SIZE_BYTES, Pc9801Bus},
    reverse_bits,
};
use crate::TraceSink;

const GDC_OPE_REPLACE: u8 = 0;
const GDC_OPE_COMPLEMENT: u8 = 1;
const GDC_OPE_CLEAR: u8 = 2;
const GDC_OPE_SET: u8 = 3;

/// AH=42h CH shift of the VRAM area field (bits 7-6).
pub(super) const SETTING_AREA_SHIFT: u8 = 6;
/// AH=42h VRAM area: the whole 32 KB of each plane (640x400).
pub(super) const SETTING_AREA_ALL: u8 = 0b11;
/// AH=42h VRAM area: the lower 16 KB of each plane (640x200 LOWER).
pub(super) const SETTING_AREA_LOWER: u8 = 0b10;
/// AH=42h VRAM area: the upper 16 KB of each plane (640x200 UPPER).
pub(super) const SETTING_AREA_UPPER: u8 = 0b01;
/// AH=42h CH bit: monochrome graphics.
pub(super) const SETTING_MONOCHROME: u8 = 0x20;
/// AH=42h CH bit: display bank 1.
pub(super) const SETTING_DISPLAY_BANK: u8 = 0x10;
/// Shift of the display bank bit in CH.
const SETTING_DISPLAY_BANK_SHIFT: u8 = 4;

/// Mode F/F 1 command (port 68h): color graphics.
const MODE_FF_COLOR: u8 = 0x02;
/// Mode F/F 1 command (port 68h): monochrome graphics.
const MODE_FF_MONOCHROME: u8 = 0x03;
/// Mode F/F 1 command (port 68h): odd graphics rasters are shown.
const MODE_FF_SHOW_ODD_RASTERS: u8 = 0x08;
/// Mode F/F 1 command (port 68h): odd graphics rasters are blank.
const MODE_FF_HIDE_ODD_RASTERS: u8 = 0x09;
/// Mode F/F 2 command (port 6Ah): GDC CLOCK-1 at 2.5 MHz.
const MODE_FF2_GDC_CLOCK1_2_5MHZ: u8 = 0x82;
/// Mode F/F 2 command (port 6Ah): GDC CLOCK-1 at 5 MHz.
const MODE_FF2_GDC_CLOCK1_5MHZ: u8 = 0x83;
/// Mode F/F 2 command (port 6Ah): GDC CLOCK-2 at 2.5 MHz.
const MODE_FF2_GDC_CLOCK2_2_5MHZ: u8 = 0x84;
/// Mode F/F 2 command (port 6Ah): GDC CLOCK-2 at 5 MHz.
const MODE_FF2_GDC_CLOCK2_5MHZ: u8 = 0x85;

/// Work area byte with the graphics status flags (PRXCRT).
pub(super) const PRXCRT_ADDRESS: usize = 0x054C;
/// PRXCRT bit 5: the display runs at 31.47 kHz.
pub(super) const PRXCRT_31KHZ: u8 = 0x20;
/// PRXCRT bit: DIP SW 1-8 was on at boot.
pub(super) const PRXCRT_DIP_SWITCH_1_8: u8 = 0x01;
/// Work area byte with the graphics mode flags (PRXDUPD).
pub(super) const PRXDUPD_ADDRESS: usize = 0x054D;
/// PRXDUPD bit: extended graphics mode (PEGC) on the PC-9821.
pub(super) const PRXDUPD_EXTENDED_GRAPHICS: u8 = 0x80;
/// PRXDUPD bit: DIP SW 2-8 selects 5 MHz for 640x400 graphics.
pub(super) const PRXDUPD_5MHZ_ALLOWED: u8 = 0x20;
/// PRXDUPD bit: flash drawing.
pub(super) const PRXDUPD_FLASH_DRAW: u8 = 0x08;
/// PRXDUPD bit: the GDC runs at 5 MHz.
pub(super) const PRXDUPD_5MHZ_ACTIVE: u8 = 0x04;
/// Work area byte whose bits 1-0 hold the graphics resolution on the PC-9821.
pub(super) const GRAPHICS_RESOLUTION_ADDRESS: usize = 0x0597;
/// Graphics resolution field of `GRAPHICS_RESOLUTION_ADDRESS`.
pub(super) const RESOLUTION_MASK: u8 = 0x03;
/// Saved graphics resolution: 640x200 UPPER.
pub(super) const RESOLUTION_UPPER: u8 = 0b00;
/// Saved graphics resolution: 640x200 LOWER.
pub(super) const RESOLUTION_LOWER: u8 = 0b01;
/// Saved graphics resolution: 640x400.
pub(super) const RESOLUTION_ALL: u8 = 0b10;
/// Saved graphics resolution: 640x480.
pub(super) const RESOLUTION_480_LINE: u8 = 0b11;

/// GDC word address of the upper area: 200 lines of 40 words.
const UPPER_AREA_START_ADDRESS: u32 = 8000;
/// SCROLL line count that covers the whole screen (the SL field value 0 means 1024 lines).
pub(super) const FULL_SCREEN_LINE_COUNT: u16 = 0x400;
/// Graphics GDC words per line at 2.5 MHz (16 dots per word).
const WORDS_PER_LINE_2_5MHZ: u16 = 40;
/// Graphics GDC words per line at 5 MHz.
const WORDS_PER_LINE_5MHZ: u16 = 80;

/// VRAM area selected by AH=42h.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayArea {
    /// 640x400 over the whole plane.
    All,
    /// 640x200 in the lower half of the plane.
    Lower,
    /// 640x200 in the upper half of the plane.
    Upper,
}

/// SYNC mode byte of the graphics GDC: graphics display mode, drawing at any time.
pub(super) const SYNC_MODE_GRAPHICS: u8 = 0x02;

/// GDC SYNC parameters in the field encoding of the uPD7220 SYNC command.
pub(super) struct SyncTiming {
    /// C/R: active words per line minus 2.
    pub(super) words_per_line: u8,
    /// HS: horizontal sync width minus 1.
    pub(super) horizontal_sync: u8,
    /// HFP: horizontal front porch minus 1.
    pub(super) horizontal_front_porch: u8,
    /// HBP: horizontal back porch minus 1.
    pub(super) horizontal_back_porch: u8,
    /// VS: vertical sync width in lines.
    pub(super) vertical_sync: u8,
    /// VFP: vertical front porch in lines.
    pub(super) vertical_front_porch: u8,
    /// VBP: vertical back porch in lines.
    pub(super) vertical_back_porch: u8,
    /// L/F: active lines per frame.
    pub(super) active_lines: u16,
}

impl SyncTiming {
    /// Packs the timing into the eight SYNC parameter bytes after the mode byte `mode`.
    pub(super) const fn sync_parameters(&self, mode: u8) -> [u8; 8] {
        [
            mode,
            self.words_per_line,
            (self.vertical_sync << 5) | self.horizontal_sync,
            (self.horizontal_front_porch << 2) | (self.vertical_sync >> 3),
            self.horizontal_back_porch,
            self.vertical_front_porch,
            self.active_lines as u8,
            (self.vertical_back_porch << 2) | (self.active_lines >> 8) as u8,
        ]
    }
}

/// Standard graphics GDC timing at 2.5 MHz on a 24.83 kHz display.
pub(super) const SYNC_TIMING_2_5MHZ: SyncTiming = SyncTiming {
    words_per_line: 0x26,
    horizontal_sync: 0x03,
    horizontal_front_porch: 0x04,
    horizontal_back_porch: 0x03,
    vertical_sync: 0x08,
    vertical_front_porch: 0x07,
    vertical_back_porch: 0x19,
    active_lines: 0x190,
};

/// Standard graphics GDC timing at 5 MHz on a 24.83 kHz display.
pub(super) const SYNC_TIMING_5MHZ: SyncTiming = SyncTiming {
    words_per_line: 0x4E,
    horizontal_sync: 0x07,
    horizontal_front_porch: 0x09,
    horizontal_back_porch: 0x07,
    vertical_sync: 0x08,
    vertical_front_porch: 0x07,
    vertical_back_porch: 0x19,
    active_lines: 0x190,
};

impl<T: TraceSink> Pc9801Bus<T> {
    pub(super) fn int18h_graphics_display_start(&mut self) {
        self.gdc_slave.state.display_enabled = true;
        self.memory.state.ram[0x054C] |= 0x80;
    }

    pub(super) fn int18h_graphics_display_stop(&mut self) {
        self.gdc_slave.state.display_enabled = false;
        self.memory.state.ram[0x054C] &= 0x7F;
    }

    /// INT 18h AH=42h: selects the graphics VRAM area, the color mode and the display bank.
    pub(super) fn int18h_display_area_set(&mut self, cpu: &mut impl Cpu) {
        let setting = cpu.ch();
        if self.crt_480_line_mode() {
            self.extended_screen_leave_480_line(setting);
        } else {
            self.graphics_set_display_area(setting);
        }
    }

    /// Applies the AH=42h `setting` byte: VRAM area (bits 7-6), monochrome (bit 5) and display
    /// bank (bit 4).
    pub(super) fn graphics_set_display_area(&mut self, setting: u8) {
        let area = match setting >> SETTING_AREA_SHIFT {
            SETTING_AREA_ALL => DisplayArea::All,
            SETTING_AREA_LOWER => DisplayArea::Lower,
            SETTING_AREA_UPPER => DisplayArea::Upper,
            _ => return,
        };
        let high_resolution_display = self.system_ppi.state.crtt;
        let half_height = area != DisplayArea::All;
        let doubled_lines = half_height && high_resolution_display;

        let color_command = if setting & SETTING_MONOCHROME != 0 {
            MODE_FF_MONOCHROME
        } else {
            MODE_FF_COLOR
        };
        let raster_command = if doubled_lines {
            MODE_FF_HIDE_ODD_RASTERS
        } else {
            MODE_FF_SHOW_ODD_RASTERS
        };
        self.display_control.write_video_mode(color_command);
        self.display_control.write_video_mode(raster_command);

        if high_resolution_display {
            self.graphics_select_gdc_clock(!half_height);
        }
        if self.memory.state.ram[PRXCRT_ADDRESS] & PRXCRT_DIP_SWITCH_1_8 != 0 {
            self.memory.state.ram[PRXDUPD_ADDRESS] |= PRXDUPD_FLASH_DRAW;
        }

        self.gdc_slave.state.lines_per_row = if doubled_lines { 2 } else { 1 };
        let start_address = if area == DisplayArea::Upper {
            UPPER_AREA_START_ADDRESS
        } else {
            0
        };
        let scroll = &mut self.gdc_slave.state.scroll[0];
        scroll.start_address = start_address;
        scroll.line_count = FULL_SCREEN_LINE_COUNT;
        scroll.im = false;
        scroll.wd = false;

        self.display_control
            .write_display_page((setting & SETTING_DISPLAY_BANK) >> SETTING_DISPLAY_BANK_SHIFT);

        let saves_resolution = match self.machine_model {
            MachineModel::PC9801F
            | MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA => false,
            MachineModel::PC9821AS | MachineModel::PC9821AP => true,
        };
        if saves_resolution {
            let resolution = match area {
                DisplayArea::Upper => RESOLUTION_UPPER,
                DisplayArea::Lower => RESOLUTION_LOWER,
                DisplayArea::All => RESOLUTION_ALL,
            };
            let saved = &mut self.memory.state.ram[GRAPHICS_RESOLUTION_ADDRESS];
            *saved = (*saved & !RESOLUTION_MASK) | resolution;
        }
    }

    /// Switches the graphics GDC between 2.5 MHz (640x200) and 5 MHz (640x400) when DIP SW 2-8
    /// allows 5 MHz, and reloads the SYNC parameters of the new clock in graphics display mode.
    fn graphics_select_gdc_clock(&mut self, full_height: bool) {
        let prxdupd = self.memory.state.ram[PRXDUPD_ADDRESS];
        if prxdupd & PRXDUPD_5MHZ_ALLOWED == 0 {
            return;
        }
        let running_5mhz = prxdupd & PRXDUPD_5MHZ_ACTIVE != 0;
        if full_height == running_5mhz {
            return;
        }

        let (timing, words_per_line) = if full_height {
            (&SYNC_TIMING_5MHZ, WORDS_PER_LINE_5MHZ)
        } else {
            (&SYNC_TIMING_2_5MHZ, WORDS_PER_LINE_2_5MHZ)
        };
        self.gdc_slave
            .load_sync_params(&timing.sync_parameters(SYNC_MODE_GRAPHICS));
        self.gdc_slave.state.pitch = words_per_line;
        self.graphics_set_gdc_clock(full_height);
    }

    /// Selects 5 MHz or 2.5 MHz on GDC CLOCK-1 and CLOCK-2 (port 6Ah 82h-85h) and records the
    /// clock in PRXDUPD bit 2.
    pub(super) fn graphics_set_gdc_clock(&mut self, five_mhz: bool) {
        let clock_commands = if five_mhz {
            [MODE_FF2_GDC_CLOCK1_5MHZ, MODE_FF2_GDC_CLOCK2_5MHZ]
        } else {
            [MODE_FF2_GDC_CLOCK1_2_5MHZ, MODE_FF2_GDC_CLOCK2_2_5MHZ]
        };
        let prxdupd = &mut self.memory.state.ram[PRXDUPD_ADDRESS];
        if five_mhz {
            *prxdupd |= PRXDUPD_5MHZ_ACTIVE;
        } else {
            *prxdupd &= !PRXDUPD_5MHZ_ACTIVE;
        }
        let has_egc = self.grcg.state.chip == grcg::GRCG_CHIP_EGC;
        for command in clock_commands {
            self.display_control.write_mode2(command, has_egc);
        }
        self.apply_gdc_dot_clock();
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
