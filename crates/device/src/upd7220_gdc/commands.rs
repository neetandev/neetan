use common::warn;

use super::{
    Gdc, GdcAction, RdatRequest, STATUS_DATA_READY, STATUS_DRAWING, STATUS_FIFO_EMPTY,
    STATUS_LIGHT_PEN,
};

impl Gdc {
    /// Returns the expected parameter byte count for a GDC command.
    pub(crate) const fn command_param_count(&self, cmd: u8) -> u8 {
        // Exact-match commands first.
        match cmd {
            0x00 | 0x01 | 0x09 => return 8, // RESET (0 or 8 params; accept 8, handle 0-param case elsewhere)
            0x0E | 0x0F => return 8,        // SYNC
            0x6E | 0x6F => return 0,        // VSYNC
            0x4B => return 3,               // CCHAR / CSRFORM
            0x6B => return 0,               // START
            0x0C | 0x0D => return 0,        // BLANK
            0x05 => return 0,               // BLANK2 (µPD7220A)
            0x46 => return 1,               // ZOOM
            0x49 => return 3,               // CURS / CSRW (3 base, optional 4th handled separately)
            0x47 => return 1,               // PITCH
            0x4A => return 2,               // MASK
            0x4C => return 11,              // FIGS / VECTW (up to 11)
            0x6C => return 0,               // FIGD / VECTE
            0x68 => return 0,               // GCHRD / TEXTE
            0xE0 => return 0,               // CURD / CSRR
            0xC0 => return 0,               // LPRD / LPEN
            0x5A => return 3,               // Undocumented 5A
            _ => {}
        }

        // Masked commands.
        if cmd & 0xF0 == 0x70 {
            return 16; // PRAM / SCROLL+TEXTW (variable, accept up to 16)
        }
        if cmd & 0xE4 == 0x20 {
            // WDAT: type determines param count.
            let transfer_type = (cmd >> 3) & 3;
            return if transfer_type == 0 { 2 } else { 1 };
        }
        if cmd & 0xE4 == 0xA0 {
            return 0; // RDAT
        }
        if cmd & 0xE4 == 0x24 {
            return 0; // DMAW
        }
        if cmd & 0xE4 == 0xA4 {
            return 0; // DMAR
        }

        0
    }

    /// Dispatches and executes zero-parameter commands.
    pub(crate) fn apply_command(&mut self) -> GdcAction {
        let cmd = self.current_command;

        match cmd {
            0x6E | 0x6F => self.apply_vsync(cmd),
            0x6B => self.apply_start(),
            0x0C | 0x0D => self.apply_blank(cmd),
            0x05 => self.apply_blank2(),
            0x6C => self.apply_figd(),
            0x68 => self.apply_gchrd(),
            0xE0 => self.apply_curd(),
            0xC0 => self.apply_lprd(),
            0x5A => GdcAction::None,
            _ if cmd & 0xE4 == 0xA0 => self.apply_rdat(cmd),
            _ if cmd & 0xE4 == 0x24 => self.apply_dmaw(cmd),
            _ if cmd & 0xE4 == 0xA4 => self.apply_dmar(cmd),
            _ => GdcAction::None,
        }
    }

    /// Called from `write_command()` when RESET command byte is received.
    /// Applies immediate effects; SYNC params are parsed incrementally.
    pub(crate) fn apply_reset_immediate(&mut self, cmd: u8) -> GdcAction {
        if cmd != 0x09 {
            self.display_enabled = false;
        }

        self.ead = 0;
        self.mask = 0;
        self.status &= !STATUS_DRAWING;
        self.fifo.clear();
        self.status =
            (self.status & !(STATUS_DATA_READY | STATUS_FIFO_EMPTY)) | self.fifo.status_bits();
        self.rdat_pending = false;
        self.stop_dma();

        GdcAction::None
    }

    /// Called from `write_data()` after every parameter byte. Dispatches to
    /// per-command incremental handlers based on `current_command`.
    pub(crate) fn apply_incremental(&mut self) -> GdcAction {
        let cmd = self.current_command;

        match cmd {
            0x00 | 0x01 | 0x09 => self.apply_reset_incremental(),
            0x0E | 0x0F => self.apply_sync_incremental(cmd),
            0x4B => self.apply_cchar_incremental(),
            0x46 => self.apply_zoom_incremental(),
            0x49 => self.apply_curs_incremental(),
            0x47 => self.apply_pitch_incremental(),
            0x4A => self.apply_mask_incremental(),
            0x4C => self.apply_figs_incremental(),
            _ if cmd & 0xF0 == 0x70 => self.apply_pram_incremental(cmd),
            _ if cmd & 0xE4 == 0x20 => self.apply_wdat_incremental(cmd),
            _ => GdcAction::None,
        }
    }

    fn apply_reset_incremental(&mut self) -> GdcAction {
        if self.param_index >= 8 {
            self.parse_sync_params();
            if self.master_mode {
                return GdcAction::TimingChanged;
            }
        }
        GdcAction::None
    }

    fn apply_sync_incremental(&mut self, cmd: u8) -> GdcAction {
        if self.param_index >= 8 {
            self.display_enabled = cmd & 1 != 0;
            self.parse_sync_params();
            if self.master_mode {
                return GdcAction::TimingChanged;
            }
        }
        GdcAction::None
    }

    pub(crate) fn parse_sync_params(&mut self) {
        if self.param_index < 8 {
            return;
        }

        let p: [u8; 16] = self.param_buffer;

        // P1: mode byte.
        let mode = p[0];
        self.display_mode = mode & super::DISPLAY_MODE_MASK;
        self.interlace_mode = (mode & 0x01) | (mode & 0x08);
        self.draw_on_retrace = mode & 0x10 != 0;

        // P2: AW - 2.
        self.aw = u16::from(p[1]) + 2;

        // P3: HS | VS_low.
        self.hs = u16::from(p[2] & 0x1F) + 1;
        let vs_low = u16::from(p[2] >> 5);

        // P4: VS_high | HFP.
        let vs_high = u16::from(p[3] & 0x03);
        self.vs = (vs_high << 3) | vs_low;
        self.hfp = u16::from(p[3] >> 2) + 1;

        // P5: HBP | pitch_ext.
        self.hbp = u16::from(p[4] & 0x3F) + 1;
        let pitch_ext = u16::from(p[4] & 0x40) << 2; // Bit 8 of pitch.

        // P6: VFP | AL_ext.
        self.vfp = u16::from(p[5] & 0x3F);

        // P7: AL_low.
        let al_low = u16::from(p[6]);

        // P8: AL_high | VBP.
        let al_high = u16::from(p[7] & 0x03);
        self.al = (al_high << 8) | al_low;
        self.vbp = u16::from(p[7] >> 2);

        // Set pitch from AW (with optional bit 8 extension).
        self.pitch = self.aw | pitch_ext;

        self.recompute_timing();
    }

    fn apply_cchar_incremental(&mut self) -> GdcAction {
        let p = self.param_buffer;
        let count = self.param_index as usize;

        if count >= 1 {
            self.lines_per_row = (p[0] & 0x1F) + 1;
            self.cursor_display = p[0] & 0x80 != 0;
        }
        if count >= 2 {
            self.cursor_top = p[1] & 0x1F;
            self.cursor_blink = p[1] & 0x20 != 0;
        }
        if count >= 3 {
            let br_low = (p[1] >> 6) & 0x03;
            let br_high = p[2] & 0x07;
            self.cursor_blink_rate = (br_high << 2) | br_low;
            self.cursor_bottom = (p[2] >> 3) & 0x1F;
        }

        GdcAction::None
    }

    fn apply_zoom_incremental(&mut self) -> GdcAction {
        if self.param_index >= 1 {
            let p = self.param_buffer[0];
            self.zoom_gchr = p & 0x0F;
            self.zoom_display = (p >> 4) & 0x0F;
        }
        GdcAction::None
    }

    fn apply_curs_incremental(&mut self) -> GdcAction {
        let p = self.param_buffer;
        let count = self.param_index as usize;

        if count >= 2 {
            self.ead = u32::from(p[0]) | (u32::from(p[1]) << 8);
        }
        if count >= 3 {
            self.ead = (self.ead & 0xFFFF) | (u32::from(p[2] & 0x03) << 16);
            self.dad = (p[2] >> 4) & 0x0F;
            self.mask = 1u16 << self.dad;
        }

        GdcAction::None
    }

    fn apply_pitch_incremental(&mut self) -> GdcAction {
        if self.param_index >= 1 {
            self.pitch = (self.pitch & 0x100) | u16::from(self.param_buffer[0]);
            if self.pitch < 2 {
                self.pitch = 2;
            }
        }
        GdcAction::None
    }

    fn apply_mask_incremental(&mut self) -> GdcAction {
        if self.param_index >= 2 {
            let p = self.param_buffer;
            self.mask = u16::from(p[0]) | (u16::from(p[1]) << 8);
        }
        GdcAction::None
    }

    fn apply_figs_incremental(&mut self) -> GdcAction {
        let count = self.param_index as usize;
        let p = self.param_buffer;

        if count >= 1 {
            self.drawing_dir = p[0] & 0x07;
            self.figure_type = (p[0] >> 3) & 0x1F;
        }

        if count >= 2 {
            self.drawing_dc = u16::from(p[1]);
        }
        if count >= 3 {
            self.drawing_dc = u16::from(p[1]) | (u16::from(p[2] & 0x3F) << 8);
            self.drawing_gd = p[2] & 0x40 != 0;
        }

        if count >= 5 {
            self.drawing_d = u16::from(p[3]) | (u16::from(p[4] & 0x3F) << 8);
        } else if count >= 4 {
            self.drawing_d = u16::from(p[3]);
        }

        if count >= 7 {
            self.drawing_d2 = u16::from(p[5]) | (u16::from(p[6] & 0x3F) << 8);
        } else if count >= 6 {
            self.drawing_d2 = u16::from(p[5]);
        }

        if count >= 9 {
            self.drawing_d1 = u16::from(p[7]) | (u16::from(p[8] & 0x3F) << 8);
        } else if count >= 8 {
            self.drawing_d1 = u16::from(p[7]);
        }

        if count >= 11 {
            self.drawing_dm = u16::from(p[9]) | (u16::from(p[10] & 0x3F) << 8);
        } else if count >= 10 {
            self.drawing_dm = u16::from(p[9]);
        }

        GdcAction::None
    }

    fn apply_pram_incremental(&mut self, cmd: u8) -> GdcAction {
        let start_index = (cmd & 0x0F) as usize;
        let byte_offset = (self.param_index as usize).saturating_sub(1);
        let ra_index = start_index + byte_offset;

        if ra_index < 16 {
            self.ra[ra_index] = self.param_buffer[byte_offset];

            if ra_index == 8 || ra_index == 9 {
                self.pattern = u16::from(self.ra[8]) | (u16::from(self.ra[9]) << 8);
            }

            self.update_scroll_partitions(self.is_slave);

            // Accept more bytes until RA is full.
            if ra_index + 1 < 16 {
                self.params_remaining = 1;
            }
        }

        GdcAction::None
    }

    fn apply_wdat_incremental(&mut self, cmd: u8) -> GdcAction {
        let transfer_type = (cmd >> 3) & 3;
        let needed = if transfer_type == 0 { 2u8 } else { 1u8 };

        if self.param_index < needed {
            return GdcAction::None;
        }

        let transfer_mode = cmd & 3;
        self.bitmap_mod = transfer_mode;
        let p = self.param_buffer;

        let data = match transfer_type {
            0 => u16::from(p[0]) | (u16::from(p[1]) << 8),
            2 => u16::from(p[0]),
            3 => u16::from(p[0]) << 8,
            _ => {
                warn!("GDC WDAT invalid transfer type: {transfer_type}");
                return GdcAction::None;
            }
        };

        if self.figure_type != 0 {
            match transfer_type {
                0 => self.pattern = data,
                2 => self.pattern = (self.pattern & 0xFF00) | (data & 0x00FF),
                3 => self.pattern = (self.pattern & 0x00FF) | (data & 0xFF00),
                _ => {}
            }
            return GdcAction::None;
        }

        let result = self.execute_wdat(data, transfer_type, transfer_mode);
        self.reset_figs_params();

        // Reset for continuous mode: accept the next word of data.
        self.param_index = 0;
        self.params_remaining = needed;

        GdcAction::Draw(result)
    }

    fn apply_vsync(&mut self, cmd: u8) -> GdcAction {
        self.master_mode = cmd & 1 != 0;
        self.recompute_timing();
        if self.master_mode {
            GdcAction::TimingChanged
        } else {
            GdcAction::None
        }
    }

    fn apply_start(&mut self) -> GdcAction {
        self.display_enabled = true;
        GdcAction::None
    }

    fn apply_blank(&mut self, cmd: u8) -> GdcAction {
        self.display_enabled = cmd & 1 != 0;
        GdcAction::None
    }

    fn apply_blank2(&mut self) -> GdcAction {
        self.display_enabled = false;
        GdcAction::None
    }

    fn apply_figd(&mut self) -> GdcAction {
        self.status |= STATUS_DRAWING;
        let result = self.execute_drawing();
        self.reset_figs_params();
        GdcAction::Draw(result)
    }

    fn apply_gchrd(&mut self) -> GdcAction {
        self.status |= STATUS_DRAWING;
        let result = self.execute_gchrd();
        self.reset_figs_params();
        GdcAction::Draw(result)
    }

    fn apply_curd(&mut self) -> GdcAction {
        let ead = self.ead;
        let mask = self.mask;
        self.fifo.clear();
        self.fifo.queue_byte(ead as u8);
        self.fifo.queue_byte((ead >> 8) as u8);
        self.fifo.queue_byte((ead >> 16) as u8 & 0x03);
        self.fifo.queue_byte(mask as u8);
        self.fifo.queue_byte((mask >> 8) as u8);
        self.status |= STATUS_DATA_READY;
        self.status &= !STATUS_FIFO_EMPTY;
        GdcAction::None
    }

    fn apply_lprd(&mut self) -> GdcAction {
        let lad = self.lad;
        self.fifo.clear();
        self.fifo.queue_byte(lad as u8);
        self.fifo.queue_byte((lad >> 8) as u8);
        self.fifo.queue_byte((lad >> 16) as u8);
        self.status |= STATUS_DATA_READY;
        self.status &= !STATUS_FIFO_EMPTY;
        self.status &= !STATUS_LIGHT_PEN;
        GdcAction::None
    }

    fn apply_rdat(&mut self, cmd: u8) -> GdcAction {
        let transfer_type = (cmd >> 3) & 3;

        if transfer_type == 1 {
            warn!("GDC RDAT invalid transfer type 1");
            return GdcAction::None;
        }

        self.rdat_pending = true;
        self.rdat_remaining = self.drawing_dc;
        self.rdat_type = transfer_type;

        self.fifo.clear();
        self.status |= STATUS_DATA_READY;
        self.status &= !STATUS_FIFO_EMPTY;

        GdcAction::ReadVram(RdatRequest { transfer_type })
    }

    fn apply_dmaw(&mut self, cmd: u8) -> GdcAction {
        let transfer_type = (cmd >> 3) & 3;
        let transfer_mode = cmd & 3;

        self.dma_type = transfer_type;
        self.dma_mod = transfer_mode;
        self.dma_is_write = true;
        self.dma_transfer_length =
            (u32::from(self.drawing_dc) + 1) * (u32::from(self.drawing_d) + 1);
        self.dma_data = 0;
        self.start_dma();

        GdcAction::None
    }

    fn apply_dmar(&mut self, cmd: u8) -> GdcAction {
        let transfer_type = (cmd >> 3) & 3;

        self.dma_type = transfer_type;
        self.dma_mod = 0;
        self.dma_is_write = false;
        self.dma_transfer_length =
            (u32::from(self.drawing_dc) + 1) * (u32::from(self.drawing_d) + 2);
        self.dma_data = 0;
        self.start_dma();

        GdcAction::None
    }
}
