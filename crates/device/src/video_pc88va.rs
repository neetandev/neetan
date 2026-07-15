//! PC-88VA2 video controller register file (VIDEOVA).
//!
//! Holds the display-mode, resolution, composition, mask, palette-mode and
//! backdrop registers (`0x100-0x148`, plus the text-mode port `0x030`) and the
//! 32-entry color palette (`0x300-0x33F`). The 16-bit registers are written a
//! byte at a time through low/high port pairs. Reads are defined only for the
//! handful of ports the hardware exposes; everything else is write-only.

use common::FramebufferVa;

const PALETTE_BASE: u16 = 0x300;
const PALETTE_ENTRIES: usize = 32;

fn set_low(register: &mut u16, value: u8) {
    *register = (*register & 0xFF00) | u16::from(value);
}

fn set_high(register: &mut u16, value: u8) {
    *register = (*register & 0x00FF) | (u16::from(value) << 8);
}

fn adjust_color12(mut color: u16) -> u16 {
    if color & 0xF000 != 0 {
        color |= 0x0C00;
    }
    if color & 0x03C0 != 0 {
        color |= 0x0020;
    }
    if color & 0x001E != 0 {
        color |= 0x0001;
    }
    color
}

/// VIDEOVA register state.
pub struct VideoVa {
    /// Text mode register at port 0x030.
    pub txtmode8: u8,
    /// Text mode register at port 0x148.
    pub txtmode: u8,
    /// Graphics display mode.
    pub grmode: u16,
    /// Graphics resolution.
    pub grres: u16,
    /// Palette screen composition.
    pub colcomp: u16,
    /// Direct-color screen composition.
    pub rgbcomp: u16,
    /// Screen mask mode.
    pub mskmode: u16,
    /// Palette mode.
    pub palmode: u16,
    /// Backdrop color.
    pub dropcol: u16,
    /// Page and plane mask.
    pub pagemsk: u16,
    /// Graphic screen 0 transparent color.
    pub xpar_g0: u16,
    /// Graphic screen 1 transparent color.
    pub xpar_g1: u16,
    /// Text and sprite transparent color.
    pub xpar_txtspr: u16,
    /// Left mask bound.
    pub mskleft: u16,
    /// Right mask bound.
    pub mskrit: u16,
    /// Top mask bound.
    pub msktop: u16,
    /// Bottom mask bound.
    pub mskbot: u16,
    /// Color palette entries.
    pub palette: [u16; PALETTE_ENTRIES],
    /// The four graphics framebuffer descriptors.
    pub framebuffer: [FramebufferVa; 4],
    /// Frame counter for the palette blink machinery.
    pub blinkcnt: u16,
}

impl Default for VideoVa {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoVa {
    /// Creates a reset video register file.
    pub fn new() -> Self {
        Self {
            txtmode8: 0,
            txtmode: 0,
            grmode: 0,
            grres: 0,
            colcomp: 0,
            rgbcomp: 0,
            mskmode: 0,
            palmode: 0,
            dropcol: 0,
            pagemsk: 0,
            xpar_g0: 0,
            xpar_g1: 0,
            xpar_txtspr: 0x0001,
            mskleft: 0,
            mskrit: 0,
            msktop: 0,
            mskbot: 0,
            palette: [0; PALETTE_ENTRIES],
            framebuffer: [
                FramebufferVa::default(),
                FramebufferVa::reset_screen1(),
                FramebufferVa::default(),
                FramebufferVa::default(),
            ],
            blinkcnt: 0,
        }
    }

    /// Advances the palette-blink counter once per frame.
    pub fn tick_blink(&mut self) {
        self.blinkcnt = self.blinkcnt.wrapping_add(1);
    }

    /// Reads a video register. Only the hardware-readable ports return a value.
    pub fn read(&self, port: u16) -> Option<u8> {
        let value = match port {
            0x100 => self.grmode as u8,
            0x101 => (self.grmode >> 8) as u8,
            0x102 => self.grres as u8,
            0x103 => (self.grres >> 8) as u8,
            0x10C => self.palmode as u8,
            0x10D => (self.palmode >> 8) as u8,
            0x200..=0x27F => return self.read_framebuffer(port),
            _ => return None,
        };
        Some(value)
    }

    /// Writes a video register byte. Returns `true` if the port was handled.
    pub fn write(&mut self, port: u16, value: u8) -> bool {
        match port {
            0x030 => self.txtmode8 = value,
            0x100 => set_low(&mut self.grmode, value),
            0x101 => set_high(&mut self.grmode, value),
            0x102 => set_low(&mut self.grres, value),
            0x103 => set_high(&mut self.grres, value),
            0x106 => set_low(&mut self.colcomp, value),
            0x107 => set_high(&mut self.colcomp, value),
            0x108 => set_low(&mut self.rgbcomp, value),
            0x109 => set_high(&mut self.rgbcomp, value),
            0x10A => set_low(&mut self.mskmode, value),
            0x10B => set_high(&mut self.mskmode, value),
            0x10C => set_low(&mut self.palmode, value),
            0x10D => set_high(&mut self.palmode, value),
            0x10E => {
                set_low(&mut self.dropcol, value);
                self.dropcol = adjust_color12(self.dropcol);
            }
            0x10F => {
                set_high(&mut self.dropcol, value);
                self.dropcol = adjust_color12(self.dropcol);
            }
            0x110 => set_low(&mut self.pagemsk, value),
            0x111 => set_high(&mut self.pagemsk, value),
            0x124 => set_low(&mut self.xpar_g0, value),
            0x125 => set_high(&mut self.xpar_g0, value),
            0x126 => set_low(&mut self.xpar_g1, value),
            0x127 => set_high(&mut self.xpar_g1, value),
            0x12E => set_low(&mut self.xpar_txtspr, value | 0x01),
            0x12F => set_high(&mut self.xpar_txtspr, value),
            0x130 => set_low(&mut self.mskleft, value),
            0x131 => set_high(&mut self.mskleft, value & 0x03),
            0x132 => set_low(&mut self.mskrit, value),
            0x133 => set_high(&mut self.mskrit, value & 0x03),
            0x134 => set_low(&mut self.msktop, value),
            0x135 => set_high(&mut self.msktop, value),
            0x136 => set_low(&mut self.mskbot, value),
            0x137 => set_high(&mut self.mskbot, value),
            0x148 => self.txtmode = value | 0x01,
            0x200..=0x27F => self.write_framebuffer(port, value),
            0x300..=0x33F => self.write_palette(port, value),
            _ => return false,
        }
        true
    }

    fn write_framebuffer(&mut self, port: u16, value: u8) {
        let index = ((port >> 5) & 3) as usize;
        let offset = port & 0x1F;
        let frame = &mut self.framebuffer[index];
        // Descriptor 1 carries the no-wrap sentinels; its fsa/fbl/ofx/ofy are
        // read-only (matching the `if (n == 1) return` guards in VIDEOVA.C).
        let screen1 = index == 1;
        match offset {
            0x00 if !screen1 => {
                frame.frame_start = (frame.frame_start & 0xFFFF_FF00) | u32::from(value & 0xFC)
            }
            0x01 if !screen1 => {
                frame.frame_start = (frame.frame_start & 0xFFFF_00FF) | (u32::from(value) << 8)
            }
            0x02 if !screen1 => {
                frame.frame_start =
                    (frame.frame_start & 0xFF00_FFFF) | (u32::from(value & 0x03) << 16)
            }
            0x04 => frame.frame_width = (frame.frame_width & 0xFF00) | u16::from(value & 0xFC),
            0x05 => {
                frame.frame_width = (frame.frame_width & 0x00FF) | (u16::from(value & 0x07) << 8)
            }
            0x06 if !screen1 => frame.frame_lines = (frame.frame_lines & 0xFF00) | u16::from(value),
            0x07 if !screen1 => {
                frame.frame_lines = (frame.frame_lines & 0x00FF) | (u16::from(value & 0x03) << 8)
            }
            0x08 => frame.dot = u16::from(value & 0x1F),
            0x0A if !screen1 => {
                frame.offset_x = (frame.offset_x & 0xFF00) | u16::from(value & 0xFC)
            }
            0x0B if !screen1 => {
                frame.offset_x = (frame.offset_x & 0x00FF) | (u16::from(value & 0x07) << 8)
            }
            0x0C if !screen1 => frame.offset_y = (frame.offset_y & 0xFF00) | u16::from(value),
            0x0D if !screen1 => {
                frame.offset_y = (frame.offset_y & 0x00FF) | (u16::from(value & 0x03) << 8)
            }
            0x0E => {
                frame.display_start = (frame.display_start & 0xFFFF_FF00) | u32::from(value & 0xFC)
            }
            0x0F => {
                frame.display_start = (frame.display_start & 0xFFFF_00FF) | (u32::from(value) << 8)
            }
            0x10 => {
                frame.display_start =
                    (frame.display_start & 0xFF00_FFFF) | (u32::from(value & 0x03) << 16)
            }
            0x12 => frame.display_height = (frame.display_height & 0xFF00) | u16::from(value),
            0x13 => {
                frame.display_height =
                    (frame.display_height & 0x00FF) | (u16::from(value & 0x01) << 8)
            }
            0x16 => frame.display_position = (frame.display_position & 0xFF00) | u16::from(value),
            0x17 => {
                frame.display_position =
                    (frame.display_position & 0x00FF) | (u16::from(value & 0x01) << 8)
            }
            _ => {}
        }
    }

    fn read_framebuffer(&self, port: u16) -> Option<u8> {
        let index = ((port >> 5) & 3) as usize;
        let offset = port & 0x1F;
        let frame = &self.framebuffer[index];
        let value = match offset {
            0x00 => frame.frame_start as u8,
            0x01 => (frame.frame_start >> 8) as u8,
            0x02 => (frame.frame_start >> 16) as u8,
            0x04 => frame.frame_width as u8,
            0x05 => (frame.frame_width >> 8) as u8,
            0x06 => frame.frame_lines as u8,
            0x07 => (frame.frame_lines >> 8) as u8,
            0x08 => frame.dot as u8,
            0x0A => frame.offset_x as u8,
            0x0B => (frame.offset_x >> 8) as u8,
            0x0C => frame.offset_y as u8,
            0x0D => (frame.offset_y >> 8) as u8,
            0x0E => frame.display_start as u8,
            0x0F => (frame.display_start >> 8) as u8,
            0x10 => (frame.display_start >> 16) as u8,
            0x12 => frame.display_height as u8,
            0x13 => (frame.display_height >> 8) as u8,
            0x16 => frame.display_position as u8,
            0x17 => (frame.display_position >> 8) as u8,
            _ => return None,
        };
        Some(value)
    }

    fn write_palette(&mut self, port: u16, value: u8) {
        let entry = ((port - PALETTE_BASE) / 2) as usize;
        if port & 1 == 0 {
            set_low(&mut self.palette[entry], value);
        } else {
            set_high(&mut self.palette[entry], value);
        }
        self.palette[entry] = adjust_color12(self.palette[entry]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grmode_round_trips_through_byte_ports() {
        let mut video = VideoVa::new();
        video.write(0x100, 0x34);
        video.write(0x101, 0x12);
        assert_eq!(video.grmode, 0x1234);
        assert_eq!(video.read(0x100), Some(0x34));
        assert_eq!(video.read(0x101), Some(0x12));
    }

    #[test]
    fn palette_write_applies_adjust_color12() {
        let mut video = VideoVa::new();
        // Entry 5: write 0xf000 (pure top red group) -> adjusted to 0xfc00.
        video.write(0x300 + 5 * 2, 0x00);
        video.write(0x300 + 5 * 2 + 1, 0xF0);
        assert_eq!(video.palette[5], 0xFC00);
    }

    #[test]
    fn text_sprite_transparent_forces_bit0() {
        let mut video = VideoVa::new();
        video.write(0x12E, 0x00);
        assert_eq!(video.xpar_txtspr & 0x01, 0x01);
    }

    #[test]
    fn write_only_ports_have_no_read() {
        let video = VideoVa::new();
        assert_eq!(video.read(0x106), None);
        assert_eq!(video.read(0x300), None);
    }
}
