//! X1 video memory and register state.
//!
//! Owns the text VRAM, attribute VRAM, PCG RAM and the three-plane bitmap VRAM,
//! plus the shadow registers (palette gun latches, priority register, column
//! mode). The bitmap planes are accessed through the CPU I/O window
//! `0x4000-0xFFFF`: normally the plane is selected by `addr & 0xC000`, but when
//! the VRAM-mode latch is set (by the falling edge of the PPI I/O switch) writes
//! fan out to several planes at once for fast fills. Any I/O read clears the
//! latch. PCG cells are addressed through the CRTC beam position, computed by the
//! bus and passed in.

/// Text VRAM size (character codes).
pub const TEXT_VRAM_SIZE: usize = 0x800;
/// Attribute VRAM size.
pub const ATTR_VRAM_SIZE: usize = 0x800;
/// Kanji text-VRAM (kvram) size, turbo only.
pub const KVRAM_SIZE: usize = 0x800;
/// PCG RAM size (three planes of 0x800).
pub const PCG_SIZE: usize = 0x1800;
/// Gaiji (16-line PCG) RAM size: three planes of 128 codes x 16 rows.
pub const GAIJI_SIZE: usize = 0x1800;
/// One bitmap page: three planes (blue 0x0000, red 0x4000, green 0x8000).
pub const BITMAP_PAGE_SIZE: usize = 0xC000;
/// Bitmap VRAM size: two pages for the turbo double buffer.
pub const BITMAP_SIZE: usize = BITMAP_PAGE_SIZE * 2;

const VRAM_ADDRESS_MASK: usize = 0x3FFF;
const TEXT_MASK: usize = 0x7FF;
const PCG_PLANE_STRIDE: usize = 0x800;
const GAIJI_PLANE_STRIDE: usize = 0x800;

/// Attribute-VRAM PCG-select bit, matching the renderer's `ATTR_PCG_SELECT`.
const ATTR_PCG_SELECT: u8 = 0x20;

const PLANE_BLUE_OFFSET: usize = 0x0000;
const PLANE_RED_OFFSET: usize = 0x4000;
const PLANE_GREEN_OFFSET: usize = 0x8000;

/// Turbo mode register 1 (`0x1FD0`) bit fields decoded by the bus. The
/// renderer decodes the display-only bits itself: bit 1 disables the hi-res
/// page interleave and bit 3 selects the displayed CG page.
///
/// CRTC character clock x1.5 (and PCG glyph-row halving in text rendering).
pub const MODE1_CHAR_CLOCK_15: u8 = 0x01;
/// CG addressing uses a 0x400-byte line stride (16 rows) instead of 0x800 (8).
pub const MODE1_CG_STRIDE_400: u8 = 0x04;
/// CPU bitmap VRAM window bank: adds 0xC000 to all three plane offsets.
pub const MODE1_VRAM_BANK: u8 = 0x10;
/// PCG/CG-ROM direct access: PCG defines bypass the beam, reads return fonts.
pub const MODE1_PCG_DIRECT: u8 = 0x20;
/// 8x16 ANK font select for the direct-access font reads.
pub const MODE1_ANK16: u8 = 0x40;
/// Kanji-underline (KSEN) mode: extra text rasters and the underline transfer.
pub const MODE1_KANJI_UNDERLINE: u8 = 0x80;

save_state::runtime_state! {
/// X1 video memory and registers.
#[derive(Clone)]
pub struct X1Video {
    text_vram: [u8; TEXT_VRAM_SIZE],
    attr_vram: [u8; ATTR_VRAM_SIZE],
    kvram: [u8; KVRAM_SIZE],
    pcg: [u8; PCG_SIZE],
    /// 16-line PCG mirror: every PCG write also lands here, with the code's
    /// even/odd half selecting the interleaved row.
    pcg_gaiji: [u8; GAIJI_SIZE],
    bitmap: [u8; BITMAP_SIZE],
    palette_blue: u8,
    palette_red: u8,
    palette_green: u8,
    priority: u8,
    /// Turbo mode register 1 (`0x1FD0`), stored raw; see the `MODE1_*` bits.
    mode1: u8,
    /// Turbo mode register 2 (`0x1FE0`), stored raw: bits 0-3 select a text
    /// color forced transparent, bits 4-5 force CG palette entries 0/1 black.
    mode2: u8,
    vram_mode: bool,
}}

impl Default for X1Video {
    fn default() -> Self {
        Self::new()
    }
}

impl X1Video {
    /// Creates empty X1 display memory and reset video registers.
    pub fn new() -> Self {
        Self {
            text_vram: [0; TEXT_VRAM_SIZE],
            attr_vram: [0; ATTR_VRAM_SIZE],
            kvram: [0; KVRAM_SIZE],
            pcg: [0; PCG_SIZE],
            pcg_gaiji: [0; GAIJI_SIZE],
            bitmap: [0; BITMAP_SIZE],
            palette_blue: 0,
            palette_red: 0,
            palette_green: 0,
            priority: 0,
            mode1: 0,
            mode2: 0,
            vram_mode: false,
        }
    }

    /// Captures all X1 video memory, registers, and access latches.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores all X1 video memory, registers, and access latches.
    pub fn restore_state(&mut self, state: Self) {
        *self = state;
    }

    /// Whether an I/O write to `addr` targets the bitmap VRAM window.
    pub fn is_bitmap_write(&self, addr: u16) -> bool {
        (addr & 0xC000) != 0 || self.vram_mode
    }

    /// Whether an I/O read from `addr` targets the bitmap VRAM window.
    pub fn is_bitmap_read(&self, addr: u16) -> bool {
        (addr & 0xC000) != 0
    }

    /// Latches VRAM (multi-plane fill) mode.
    pub fn latch_vram_mode(&mut self) {
        self.vram_mode = true;
    }

    /// Clears the VRAM-mode latch. Called at the start of every I/O read.
    pub fn clear_vram_mode(&mut self) {
        self.vram_mode = false;
    }

    /// The CPU bitmap VRAM window bank offset selected by mode register 1.
    fn cpu_bitmap_page(&self) -> usize {
        if self.mode1 & MODE1_VRAM_BANK != 0 {
            BITMAP_PAGE_SIZE
        } else {
            0
        }
    }

    /// Writes to the bitmap VRAM window, honouring the multi-plane fill mode.
    /// CPU accesses target the bank selected by mode register 1.
    pub fn write_bitmap(&mut self, addr: u16, value: u8) {
        let offset = self.cpu_bitmap_page() + ((addr as usize) & VRAM_ADDRESS_MASK);
        match addr & 0xC000 {
            0x0000 => {
                // Reached only when the VRAM-mode latch is set: fill all planes.
                self.bitmap[PLANE_BLUE_OFFSET + offset] = value;
                self.bitmap[PLANE_RED_OFFSET + offset] = value;
                self.bitmap[PLANE_GREEN_OFFSET + offset] = value;
            }
            0x4000 => {
                if self.vram_mode {
                    self.bitmap[PLANE_RED_OFFSET + offset] = value;
                    self.bitmap[PLANE_GREEN_OFFSET + offset] = value;
                } else {
                    self.bitmap[PLANE_BLUE_OFFSET + offset] = value;
                }
            }
            0x8000 => {
                if self.vram_mode {
                    self.bitmap[PLANE_BLUE_OFFSET + offset] = value;
                    self.bitmap[PLANE_GREEN_OFFSET + offset] = value;
                } else {
                    self.bitmap[PLANE_RED_OFFSET + offset] = value;
                }
            }
            _ => {
                if self.vram_mode {
                    self.bitmap[PLANE_BLUE_OFFSET + offset] = value;
                    self.bitmap[PLANE_RED_OFFSET + offset] = value;
                } else {
                    self.bitmap[PLANE_GREEN_OFFSET + offset] = value;
                }
            }
        }
    }

    /// Reads from the bitmap VRAM window (plane by `addr & 0xC000`) on the bank
    /// selected by mode register 1.
    pub fn read_bitmap(&self, addr: u16) -> u8 {
        let offset = self.cpu_bitmap_page() + ((addr as usize) & VRAM_ADDRESS_MASK);
        match addr & 0xC000 {
            0x4000 => self.bitmap[PLANE_BLUE_OFFSET + offset],
            0x8000 => self.bitmap[PLANE_RED_OFFSET + offset],
            0xC000 => self.bitmap[PLANE_GREEN_OFFSET + offset],
            _ => 0xFF,
        }
    }

    /// Reads one kanji attribute VRAM byte.
    pub fn read_kvram(&self, addr: u16) -> u8 {
        self.kvram[(addr as usize) & TEXT_MASK]
    }

    /// Writes one kanji attribute VRAM byte.
    pub fn write_kvram(&mut self, addr: u16, value: u8) {
        self.kvram[(addr as usize) & TEXT_MASK] = value;
    }

    /// Writes turbo mode register 1 (`0x1FD0`).
    pub fn write_mode1(&mut self, value: u8) {
        self.mode1 = value;
    }

    /// Writes turbo mode register 2 (`0x1FE0`).
    pub fn write_mode2(&mut self, value: u8) {
        self.mode2 = value;
    }

    /// Reads turbo mode register 1.
    pub fn mode1(&self) -> u8 {
        self.mode1
    }

    /// Reads turbo mode register 2.
    pub fn mode2(&self) -> u8 {
        self.mode2
    }

    /// Whether the PCG/CG-ROM direct access mode is active.
    pub fn pcg_direct(&self) -> bool {
        self.mode1 & MODE1_PCG_DIRECT != 0
    }

    /// Borrows kanji attribute VRAM.
    pub fn kvram(&self) -> &[u8] {
        &self.kvram
    }

    /// Finds the character cell the hi-speed kanji read stages its code in: the
    /// first of a fixed set of cells whose attribute clears the PCG-select bit.
    pub fn check_char_address(&self) -> u16 {
        for cell in [0x7FF, 0x3FF, 0x5FF, 0x1FF] {
            if self.attr_vram[cell] & 0x20 == 0 {
                return cell as u16;
            }
        }
        0x3FF
    }

    /// Reads one text VRAM byte.
    pub fn read_text(&self, addr: u16) -> u8 {
        self.text_vram[(addr as usize) & TEXT_MASK]
    }

    /// Writes one text VRAM byte.
    pub fn write_text(&mut self, addr: u16, value: u8) {
        self.text_vram[(addr as usize) & TEXT_MASK] = value;
    }

    /// Reads one text attribute VRAM byte.
    pub fn read_attr(&self, addr: u16) -> u8 {
        self.attr_vram[(addr as usize) & TEXT_MASK]
    }

    /// Writes one text attribute VRAM byte.
    pub fn write_attr(&mut self, addr: u16, value: u8) {
        self.attr_vram[(addr as usize) & TEXT_MASK] = value;
    }

    /// Writes a PCG glyph byte. `plane` is 1..=3 (bit plane B/R/G); plane 0 is
    /// the ANK ROM area and is read-only. `code`/`line` come from the beam.
    pub fn write_pcg(&mut self, code: u8, line: u8, plane: u8, value: u8) {
        if plane == 0 || plane > 3 {
            return;
        }
        self.pcg[pcg_offset(code, line, plane)] = value;
        self.pcg_gaiji[gaiji_offset(code, line, plane)] = value;
    }

    /// The character code and glyph row a direct-access PCG port access
    /// targets: the code comes from the PCG-select staging cell (16-line cells
    /// take the even/odd half from the port's bit 0), the row from the port's
    /// low bits.
    fn hispeed_code_line(&self, port: u16) -> (u8, u8) {
        let cell = self.check_pcg_address();
        let mut code = self.text_vram[cell];
        if self.kvram[cell] & 0x90 != 0 {
            code = (code & 0xFE) | (port as u8 & 1);
        }
        let line = ((port & 0x000E) >> 1) as u8;
        (code, line)
    }

    /// Hi-speed (turbo) PCG glyph define: the character code comes from the
    /// PCG-select staging cell rather than the beam position, and the glyph row
    /// and plane come straight from the I/O port. Lets the turbo define PCG
    /// glyphs without waiting for the CRT beam. `plane` is 1..=3 (bit plane
    /// B/R/G); plane 0 is the read-only ANK area.
    pub fn write_pcg_hispeed(&mut self, port: u16, plane: u8, value: u8) {
        if plane == 0 || plane > 3 {
            return;
        }
        let (code, line) = self.hispeed_code_line(port);
        self.pcg[pcg_offset(code, line, plane)] = value;
        self.pcg_gaiji[gaiji_offset(code, line, plane)] = value;
    }

    /// Hi-speed (turbo) PCG glyph read-back for the colour planes (1..=3),
    /// addressed like [`Self::write_pcg_hispeed`].
    pub fn read_pcg_hispeed(&self, port: u16, plane: u8) -> u8 {
        if plane == 0 || plane > 3 {
            return 0xFF;
        }
        let (code, line) = self.hispeed_code_line(port);
        self.pcg[pcg_offset(code, line, plane)]
    }

    /// Finds the PCG-define staging cell for hi-speed writes: the first of a
    /// fixed set of cells whose attribute has the PCG-select bit set. This is
    /// the complement of [`check_char_address`], which looks for the bit clear.
    fn check_pcg_address(&self) -> usize {
        for cell in [0x7FF, 0x3FF, 0x5FF, 0x1FF] {
            if self.attr_vram[cell] & ATTR_PCG_SELECT != 0 {
                return cell;
            }
        }
        0x3FF
    }

    /// Reads a PCG glyph byte (plane 1..=3), or the CG-ROM for plane 0.
    pub fn read_pcg(&self, code: u8, line: u8, plane: u8, cg_rom: &[u8]) -> u8 {
        if plane == 0 {
            let index = (usize::from(code) * 8 + usize::from(line)) & TEXT_MASK;
            return cg_rom.get(index).copied().unwrap_or(0);
        }
        self.pcg[pcg_offset(code, line, plane)]
    }

    /// Writes the blue palette register.
    pub fn set_palette_blue(&mut self, value: u8) {
        self.palette_blue = value;
    }

    /// Writes the red palette register.
    pub fn set_palette_red(&mut self, value: u8) {
        self.palette_red = value;
    }

    /// Writes the green palette register.
    pub fn set_palette_green(&mut self, value: u8) {
        self.palette_green = value;
    }

    /// Writes the display priority register.
    pub fn set_priority(&mut self, value: u8) {
        self.priority = value;
    }

    /// Borrows text VRAM.
    pub fn text_vram(&self) -> &[u8] {
        &self.text_vram
    }

    /// Borrows text attribute VRAM.
    pub fn attr_vram(&self) -> &[u8] {
        &self.attr_vram
    }

    /// Borrows programmable character generator memory.
    pub fn pcg(&self) -> &[u8] {
        &self.pcg
    }

    /// Borrows the gaiji mirror of programmable character memory.
    pub fn pcg_gaiji(&self) -> &[u8] {
        &self.pcg_gaiji
    }

    /// Borrows bitmap VRAM.
    pub fn bitmap(&self) -> &[u8] {
        &self.bitmap
    }

    /// Returns the blue, red, and green palette registers.
    pub fn palette_guns(&self) -> [u8; 3] {
        [self.palette_blue, self.palette_red, self.palette_green]
    }

    /// Reads the display priority register.
    pub fn priority(&self) -> u8 {
        self.priority
    }
}

fn pcg_offset(code: u8, line: u8, plane: u8) -> usize {
    let base = (usize::from(code) * 8 + usize::from(line)) & TEXT_MASK;
    base + usize::from(plane - 1) * PCG_PLANE_STRIDE
}

/// The gaiji-mirror offset for a PCG write: 16 interleaved rows per code pair,
/// with the code's low bit selecting the odd rows.
fn gaiji_offset(code: u8, line: u8, plane: u8) -> usize {
    let row = ((usize::from(line) & 7) << 1) | (usize::from(code) & 1);
    usize::from(code >> 1) * 16 + row + usize::from(plane - 1) * GAIJI_PLANE_STRIDE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_plane_write_and_read_round_trip() {
        let mut video = X1Video::new();
        video.write_bitmap(0x4000, 0xAA); // blue plane
        video.write_bitmap(0x8000, 0xBB); // red plane
        video.write_bitmap(0xC000, 0xCC); // green plane
        assert_eq!(video.read_bitmap(0x4000), 0xAA);
        assert_eq!(video.read_bitmap(0x8000), 0xBB);
        assert_eq!(video.read_bitmap(0xC000), 0xCC);
    }

    #[test]
    fn vram_mode_fans_writes_across_planes() {
        let mut video = X1Video::new();
        video.latch_vram_mode();
        // In VRAM mode a 0x4000 write hits red and green, not blue.
        video.write_bitmap(0x4000, 0x77);
        assert_eq!(video.read_bitmap(0x4000), 0x00); // blue untouched
        assert_eq!(video.read_bitmap(0x8000), 0x77); // red
        assert_eq!(video.read_bitmap(0xC000), 0x77); // green
    }

    #[test]
    fn io_read_clears_vram_mode() {
        let mut video = X1Video::new();
        video.latch_vram_mode();
        video.clear_vram_mode();
        video.write_bitmap(0x4000, 0x55);
        assert_eq!(video.read_bitmap(0x4000), 0x55); // blue (single-plane)
    }

    #[test]
    fn pcg_planes_are_independent() {
        let mut video = X1Video::new();
        video.write_pcg(2, 3, 1, 0x11); // blue plane
        video.write_pcg(2, 3, 2, 0x22); // red plane
        video.write_pcg(2, 3, 3, 0x33); // green plane
        let cg = [0u8; 0x800];
        assert_eq!(video.read_pcg(2, 3, 1, &cg), 0x11);
        assert_eq!(video.read_pcg(2, 3, 2, &cg), 0x22);
        assert_eq!(video.read_pcg(2, 3, 3, &cg), 0x33);
    }

    #[test]
    fn mode1_vram_bank_selects_the_second_bitmap_page() {
        let mut video = X1Video::new();
        video.write_bitmap(0x4000, 0x11);
        video.write_mode1(MODE1_VRAM_BANK);
        video.write_bitmap(0x4000, 0x22);
        assert_eq!(video.read_bitmap(0x4000), 0x22);
        video.write_mode1(0);
        assert_eq!(video.read_bitmap(0x4000), 0x11);
    }

    #[test]
    fn hispeed_pcg_uses_kvram_to_select_even_and_odd_code_halves() {
        let mut video = X1Video::new();
        video.write_text(0x07FF, 0x42);
        video.write_attr(0x07FF, ATTR_PCG_SELECT);
        video.write_kvram(0x07FF, 0x10);
        video.write_pcg_hispeed(0x1500, 1, 0x11);
        video.write_pcg_hispeed(0x1501, 1, 0x22);

        let cg = [0u8; 0x800];
        assert_eq!(video.read_pcg(0x42, 0, 1, &cg), 0x11);
        assert_eq!(video.read_pcg(0x43, 0, 1, &cg), 0x22);
    }
}
