//! Per-frame snapshot resolution for the renderer.

use super::{
    ATC_INDEX_COLOR_SELECT, ATC_INDEX_MODE_CONTROL, ATC_INDEX_OVERSCAN, ATC_INDEX_PEL_PAN,
    ATC_INDEX_PLANE_ENABLE, CRTC_INDEX_CURSOR_END, CRTC_INDEX_CURSOR_START, CRTC_INDEX_EXT_START,
    CRTC_INDEX_OVERFLOW_HIGH, CRTC_INDEX_VRETRACE_END, CRTC_INDEX_VSCONF1, Vga,
    dac::expand_6bit_component,
};

/// Frame phase mask for the hardware cursor blink (on half the period).
const CURSOR_BLINK_PHASE_MASK: u32 = 0x10;
/// Frame phase mask for the text attribute blink.
const TEXT_BLINK_PHASE_MASK: u32 = 0x20;

/// Scan-out interpretation of display memory, decoded from the graphics
/// controller shift register and memory map bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VgaRenderMode {
    /// Alphanumeric mode through the character generator.
    Text,
    /// 16-color planar graphics (EGA and VGA planar modes).
    Planar16,
    /// 256-color packed pixel graphics (mode 13h, Mode X and the SVGA modes).
    Packed256,
    /// CGA compatible 4-color graphics through the interleaved shift register.
    CgaInterleaved,
    /// One bit per pixel graphics from plane zero (CGA mode 06h).
    Mono1bpp,
}

/// Register-derived scan-out state for one frame, in plane address units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedVgaFrame {
    /// How the scan-out interprets display memory.
    pub render_mode: VgaRenderMode,
    /// The screen is blanked (palette address source clear or screen off).
    pub blanked: bool,
    /// Active character columns per row.
    pub columns: u32,
    /// Character cell width in dots (8 or 9).
    pub character_width: u32,
    /// Character cell height in scanlines; in graphics modes the number of
    /// times each memory row is scanned before the row advances.
    pub character_height: u32,
    /// Every row scan is emitted twice (CRTC scan doubling).
    pub scan_doubled: bool,
    /// Active scanlines per frame.
    pub active_scanlines: u32,
    /// Display start address in plane address units (addressing mode applied).
    pub start_address: u32,
    /// Plane address advance per character or memory row.
    pub row_pitch: u32,
    /// Plane address advance per character clock (2 in word mode, 1 in byte
    /// and doubleword modes).
    pub address_step: u32,
    /// Address mask for one plane in the selected IBM or TLI mapping mode.
    pub plane_address_mask: u32,
    /// Address bit 13 is replaced by row scan bit 0 (CGA interleave).
    pub map13_from_row_scan: bool,
    /// Address bit 14 is replaced by row scan bit 1.
    pub map14_from_row_scan: bool,
    /// Scanline at which the fetch address resets to zero (split screen).
    pub line_compare: u32,
    /// Pel panning is forced to zero below the split screen line.
    pub pel_pan_reset_on_split: bool,
    /// Row scan value the first displayed character row starts at.
    pub preset_row_scan: u8,
    /// Hardware cursor location in plane address units.
    pub cursor_address: u32,
    /// First character scanline of the cursor block.
    pub cursor_start_row: u8,
    /// Last character scanline of the cursor block.
    pub cursor_end_row: u8,
    /// The cursor is enabled and in its visible blink phase.
    pub cursor_visible: bool,
    /// Attribute bit 7 selects blink instead of a bright background.
    pub blink_enabled: bool,
    /// Blinking characters are in their visible phase.
    pub blink_visible: bool,
    /// Line graphics characters 0xC0-0xDF extend into the ninth dot.
    pub line_graphics: bool,
    /// Font plane offset used when attribute bit 3 is set.
    pub font_offset_map_a: u32,
    /// Font plane offset used when attribute bit 3 is clear.
    pub font_offset_map_b: u32,
    /// Horizontal pel panning value.
    pub pel_pan: u8,
    /// 256-color pixels are emitted for two dot clocks each (mode 13h rate);
    /// clear for the ET4000 one-pixel-per-dot SVGA modes.
    pub packed_half_rate: bool,
    /// Border color around the active area, packed RGBA.
    pub border_color: u32,
    /// The sixteen attribute colors resolved to packed RGBA.
    pub pens: [u32; 16],
    /// The full DAC palette resolved to packed RGBA (256-color modes).
    pub pens_256: [u32; 256],
}

impl Vga {
    /// Advances the frame counter and interrupt latch at vertical sync start.
    pub fn on_vsync_start(&mut self) {
        self.frame_counter = self.frame_counter.wrapping_add(1);
        if self.crtc[usize::from(CRTC_INDEX_VRETRACE_END)] & 0x20 == 0 {
            self.vretrace_interrupt_latch = true;
        }
    }

    /// Resolves the register file into one frame of scan-out state.
    pub fn resolve(&self) -> ResolvedVgaFrame {
        let attribute_mode = self.atc[usize::from(ATC_INDEX_MODE_CONTROL)];
        let cursor_start = self.crtc[usize::from(CRTC_INDEX_CURSOR_START)];
        let cursor_end = self.crtc[usize::from(CRTC_INDEX_CURSOR_END)];
        let extended_start = self.crtc[usize::from(CRTC_INDEX_EXT_START)];

        // Doubleword mode overrides byte mode; word mode shifts the memory
        // address counter left by one, so start, pitch and step scale by two.
        let dword_mode = self.crtc[0x14] & 0x40 != 0;
        let byte_mode = self.crtc[0x17] & 0x40 != 0;
        let address_shift = if dword_mode || byte_mode { 0 } else { 1 };

        let start_address = ((u32::from(self.crtc[0x0C]) << 8 | u32::from(self.crtc[0x0D]))
            + u32::from((self.crtc[0x08] & 0x60) >> 5)
            + (u32::from(extended_start & 0x03) << 16))
            << address_shift;
        let cursor_address = ((u32::from(self.crtc[0x0E]) << 8 | u32::from(self.crtc[0x0F]))
            + u32::from((cursor_end & 0x60) >> 5)
            + (u32::from(extended_start & 0x0C) << 14))
            << address_shift;
        let row_pitch = (u32::from(self.crtc[0x13]) * 2) << address_shift;

        let line_compare = u32::from(self.crtc[0x18])
            | u32::from(self.crtc[0x07] & 0x10) << 4
            | u32::from(self.crtc[0x09] & 0x40) << 3
            | u32::from(self.crtc[usize::from(CRTC_INDEX_OVERFLOW_HIGH)] & 0x10) << 6;

        let cursor_enabled = cursor_start & 0x20 == 0;
        let cursor_blink_on = self.frame_counter & CURSOR_BLINK_PHASE_MASK != 0;

        ResolvedVgaFrame {
            render_mode: self.classify_render_mode(),
            blanked: self.atc_index & 0x20 == 0 || self.seq[1] & 0x20 != 0,
            columns: u32::from(self.crtc[0x01]) + 1,
            character_width: if self.seq[1] & 0x01 != 0 { 8 } else { 9 },
            character_height: u32::from(self.crtc[0x09] & 0x1F) + 1,
            scan_doubled: self.crtc[0x09] & 0x80 != 0,
            active_scanlines: self.frame_timing().active_scanlines,
            start_address,
            row_pitch,
            address_step: 1 << address_shift,
            plane_address_mask: if self.crtc[usize::from(CRTC_INDEX_VSCONF1)] & 0x20 == 0 {
                0xFFFF
            } else {
                (super::VGA_VRAM_SIZE / 4) as u32 - 1
            },
            map13_from_row_scan: self.crtc[0x17] & 0x01 == 0,
            map14_from_row_scan: self.crtc[0x17] & 0x02 == 0,
            line_compare,
            pel_pan_reset_on_split: attribute_mode & 0x20 != 0,
            preset_row_scan: self.crtc[0x08] & 0x1F,
            cursor_address,
            cursor_start_row: cursor_start & 0x1F,
            cursor_end_row: cursor_end & 0x1F,
            cursor_visible: cursor_enabled && cursor_blink_on,
            blink_enabled: attribute_mode & 0x08 != 0,
            blink_visible: self.frame_counter & TEXT_BLINK_PHASE_MASK != 0,
            line_graphics: attribute_mode & 0x04 != 0,
            font_offset_map_a: font_plane_offset(
                (self.seq[3] >> 2) & 0x03,
                self.seq[3] & 0x20 != 0,
            ),
            font_offset_map_b: font_plane_offset(self.seq[3] & 0x03, self.seq[3] & 0x10 != 0),
            pel_pan: self.atc[usize::from(ATC_INDEX_PEL_PAN)] & 0x0F,
            packed_half_rate: self.crtc[0x17] & 0x08 == 0,
            border_color: self.resolve_dac_entry(self.atc[usize::from(ATC_INDEX_OVERSCAN)]),
            pens: self.resolve_pens(),
            pens_256: self.resolve_pens_256(),
        }
    }

    /// Classifies the scan-out mode from the graphics controller state.
    fn classify_render_mode(&self) -> VgaRenderMode {
        if self.gc[6] & 0x01 == 0 {
            VgaRenderMode::Text
        } else if self.gc[5] & 0x40 != 0 {
            VgaRenderMode::Packed256
        } else if self.gc[5] & 0x20 != 0 {
            VgaRenderMode::CgaInterleaved
        } else if (self.gc[6] >> 2) & 0x03 == 3 {
            VgaRenderMode::Mono1bpp
        } else {
            VgaRenderMode::Planar16
        }
    }

    /// Resolves the sixteen attribute colors through the attribute palette,
    /// color select and DAC into packed RGBA.
    fn resolve_pens(&self) -> [u32; 16] {
        let attribute_mode = self.atc[usize::from(ATC_INDEX_MODE_CONTROL)];
        let plane_enable = self.atc[usize::from(ATC_INDEX_PLANE_ENABLE)] & 0x0F;
        let color_select = self.atc[usize::from(ATC_INDEX_COLOR_SELECT)];
        let mut pens = [0; 16];
        for (attribute, pen) in pens.iter_mut().enumerate() {
            let palette = self.atc[attribute & usize::from(plane_enable)] & 0x3F;
            let mut dac_index = if attribute_mode & 0x80 != 0 {
                (palette & 0x0F) | (color_select & 0x03) << 4
            } else {
                palette
            };
            dac_index |= (color_select & 0x0C) << 4;
            *pen = self.resolve_dac_entry(dac_index);
        }
        pens
    }

    /// Resolves the full DAC palette into packed RGBA for the 256-color modes.
    fn resolve_pens_256(&self) -> [u32; 256] {
        let mut pens = [0; 256];
        for (index, pen) in pens.iter_mut().enumerate() {
            *pen = self.resolve_dac_entry(index as u8);
        }
        pens
    }

    /// Resolves one DAC entry through the pixel mask into packed RGBA.
    fn resolve_dac_entry(&self, index: u8) -> u32 {
        let [red, green, blue] = self.dac[usize::from(index & self.dac_mask)];
        u32::from_le_bytes([
            expand_6bit_component(red),
            expand_6bit_component(green),
            expand_6bit_component(blue),
            0xFF,
        ])
    }
}

/// Plane offset of a font block from its character map select value.
fn font_plane_offset(select_low: u8, select_high: bool) -> u32 {
    (u32::from(select_low) << 14) | (u32::from(select_high) << 13)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addressing_mode_selects_the_plane_address_mask() {
        let mut vga = Vga::new();
        assert_eq!(vga.resolve().plane_address_mask, 0xFFFF);
        vga.crtc[usize::from(CRTC_INDEX_VSCONF1)] = 0x20;
        assert_eq!(vga.resolve().plane_address_mask, 0x3FFFF);
    }
}
