//! Sharp X68000 video-controller register and palette state.

/// Number of entries in each X68000 palette.
pub const X68K_PALETTE_ENTRIES: usize = 256;

/// X68000 graphics or text/sprite palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteX68k {
    /// Graphics palette at `0xE82000`.
    Graphics,
    /// Text, sprite, and background palette at `0xE82200`.
    Text,
}

/// Graphics screen color mode selected by video-controller R0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicModeX68k {
    /// 512x512 16-color screen with four planes.
    Colors16,
    /// 512x512 256-color screen with two planes.
    Colors256,
    /// 512x512 65536-color screen with one plane.
    Colors65536,
    /// 1024x1024 16-color screen with one plane.
    Colors16Virtual1024,
}

save_state::runtime_state! {
/// X68000 video-controller registers and palettes.
#[derive(Debug, Clone)]
pub struct VideoControllerX68k {
    graphics_palette: [u16; X68K_PALETTE_ENTRIES],
    text_palette: [u16; X68K_PALETTE_ENTRIES],
    registers: [u16; 3],
}}

impl VideoControllerX68k {
    /// Captures complete video controller and palette state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores complete video controller and palette state.
    pub fn restore_state(&mut self, state: Self) {
        *self = state;
    }
}

impl Default for VideoControllerX68k {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoControllerX68k {
    /// Creates cleared video-controller state.
    pub const fn new() -> Self {
        Self {
            graphics_palette: [0; X68K_PALETTE_ENTRIES],
            text_palette: [0; X68K_PALETTE_ENTRIES],
            registers: [0; 3],
        }
    }

    /// Resets all registers and palette entries.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Reads one palette entry.
    pub fn read_palette(&self, palette: PaletteX68k, index: usize) -> u16 {
        self.palette(palette)[index & 0xFF]
    }

    /// Writes one palette entry.
    pub fn write_palette(&mut self, palette: PaletteX68k, index: usize, value: u16) {
        self.palette_mut(palette)[index & 0xFF] = value;
    }

    /// Reads one of video registers R0-R2.
    pub fn read_register(&self, index: usize) -> u16 {
        self.registers[index % 3]
    }

    /// Writes one of video registers R0-R2.
    pub fn write_register(&mut self, index: usize, value: u16) {
        let index = index % 3;
        self.registers[index] = value & REGISTER_MASKS[index];
    }

    /// Returns the graphics screen color mode from R0.
    pub const fn memory_mode(&self) -> GraphicModeX68k {
        match self.registers[0] & 7 {
            0 => GraphicModeX68k::Colors16,
            1 => GraphicModeX68k::Colors256,
            2 | 3 => GraphicModeX68k::Colors65536,
            _ => GraphicModeX68k::Colors16Virtual1024,
        }
    }

    /// Returns the sprite screen priority from R1.
    pub const fn sprite_priority(&self) -> u16 {
        (self.registers[1] >> 12) & 3
    }

    /// Returns the text screen priority from R1.
    pub const fn text_priority(&self) -> u16 {
        (self.registers[1] >> 10) & 3
    }

    /// Returns the graphics screen priority from R1.
    pub const fn graphics_priority(&self) -> u16 {
        (self.registers[1] >> 8) & 3
    }

    /// Returns the graphics page displayed at one rank, front first.
    pub const fn graphic_page_rank(&self, rank: usize) -> usize {
        ((self.registers[1] >> ((rank & 3) * 2)) & 3) as usize
    }

    /// Returns whether the sprite screen is enabled.
    pub const fn sprite_screen_enabled(&self) -> bool {
        self.registers[2] & 0x0040 != 0
    }

    /// Returns the graphics plane enable bits from R2.
    pub const fn graphic_plane_enables(&self) -> u16 {
        self.registers[2] & 0x001F
    }

    /// Returns whether the special priority extension is selected.
    pub const fn special_priority_enabled(&self) -> bool {
        self.registers[2] & 0x1800 == 0x1000
    }

    /// Returns whether the translucency extension is selected.
    pub const fn half_brightness_enabled(&self) -> bool {
        self.registers[2] & 0x1800 == 0x1800
    }

    /// Returns whether the extension area is selected by GVRAM contents.
    pub const fn least_significant_bit_translucent(&self) -> bool {
        self.registers[2] & 0x0400 != 0
    }

    /// Returns whether the first graphics plane blends with the second.
    pub const fn graphic_translucent_graphic(&self) -> bool {
        self.registers[2] & 0x0200 != 0
    }

    /// Returns whether the first graphics plane blends with sprite and text.
    pub const fn graphic_translucent_sprite_text(&self) -> bool {
        self.registers[2] & 0x0100 != 0
    }

    /// Returns whether text palette 0 modulates the graphics color.
    pub const fn brightness_modulation_enabled(&self) -> bool {
        self.registers[2] & 0x4000 != 0
    }

    /// Returns whether the text layer is enabled.
    pub const fn text_enabled(&self) -> bool {
        self.registers[2] & 0x0020 != 0
    }

    /// Returns the graphics palette.
    pub const fn graphics_palette(&self) -> &[u16; X68K_PALETTE_ENTRIES] {
        &self.graphics_palette
    }

    /// Returns the text palette.
    pub const fn text_palette(&self) -> &[u16; X68K_PALETTE_ENTRIES] {
        &self.text_palette
    }

    fn palette(&self, palette: PaletteX68k) -> &[u16; X68K_PALETTE_ENTRIES] {
        match palette {
            PaletteX68k::Graphics => &self.graphics_palette,
            PaletteX68k::Text => &self.text_palette,
        }
    }

    fn palette_mut(&mut self, palette: PaletteX68k) -> &mut [u16; X68K_PALETTE_ENTRIES] {
        match palette {
            PaletteX68k::Graphics => &mut self.graphics_palette,
            PaletteX68k::Text => &mut self.text_palette,
        }
    }
}

/// Writable-bit masks for video registers R0-R2.
const REGISTER_MASKS: [u16; 3] = [0x0007, 0x3FFF, 0xFFFF];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_and_registers_are_independent() {
        let mut video = VideoControllerX68k::new();
        video.write_palette(PaletteX68k::Graphics, 1, 0x1234);
        video.write_palette(PaletteX68k::Text, 1, 0x5678);
        video.write_register(2, 0x20);
        assert_eq!(video.read_palette(PaletteX68k::Graphics, 1), 0x1234);
        assert_eq!(video.read_palette(PaletteX68k::Text, 1), 0x5678);
        assert!(video.text_enabled());
    }

    #[test]
    fn registers_keep_only_their_writable_bits() {
        let mut video = VideoControllerX68k::new();
        video.write_register(0, 0xFFFF);
        video.write_register(1, 0xFFFF);
        video.write_register(2, 0xFFFF);
        assert_eq!(video.read_register(0), 0x0007);
        assert_eq!(video.read_register(1), 0x3FFF);
        assert_eq!(video.read_register(2), 0xFFFF);
    }

    #[test]
    fn memory_mode_folds_undefined_values_onto_real_modes() {
        let mut video = VideoControllerX68k::new();
        let expected = [
            GraphicModeX68k::Colors16,
            GraphicModeX68k::Colors256,
            GraphicModeX68k::Colors65536,
            GraphicModeX68k::Colors65536,
            GraphicModeX68k::Colors16Virtual1024,
            GraphicModeX68k::Colors16Virtual1024,
            GraphicModeX68k::Colors16Virtual1024,
            GraphicModeX68k::Colors16Virtual1024,
        ];
        for (value, mode) in expected.into_iter().enumerate() {
            video.write_register(0, value as u16);
            assert_eq!(video.memory_mode(), mode);
        }
    }

    #[test]
    fn priority_and_page_ranks_decode_register_one() {
        let mut video = VideoControllerX68k::new();
        video.write_register(1, 0x25B4);
        assert_eq!(video.sprite_priority(), 2);
        assert_eq!(video.text_priority(), 1);
        assert_eq!(video.graphics_priority(), 1);
        assert_eq!(video.graphic_page_rank(0), 0);
        assert_eq!(video.graphic_page_rank(1), 1);
        assert_eq!(video.graphic_page_rank(2), 3);
        assert_eq!(video.graphic_page_rank(3), 2);
    }

    #[test]
    fn mixing_predicates_decode_register_two() {
        let mut video = VideoControllerX68k::new();
        video.write_register(2, 0x1000);
        assert!(video.special_priority_enabled());
        assert!(!video.half_brightness_enabled());
        video.write_register(2, 0x1F5F);
        assert!(!video.special_priority_enabled());
        assert!(video.half_brightness_enabled());
        assert!(video.least_significant_bit_translucent());
        assert!(video.graphic_translucent_graphic());
        assert!(video.graphic_translucent_sprite_text());
        assert!(!video.brightness_modulation_enabled());
        assert!(video.sprite_screen_enabled());
        assert!(video.graphic_plane_enables() == 0x001F);
        video.write_register(2, 0x4800);
        assert!(video.brightness_modulation_enabled());
        assert!(!video.half_brightness_enabled());
    }
}
