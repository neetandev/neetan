//! PC-88VA graphic bitmap layers: per-raster rasterization of the two graphic
//! screens into palette-index / color-code scanlines.
//!
//! Each graphic screen walks one or more framebuffer descriptors over graphics
//! VRAM (256 KiB), producing one `u16` per visible dot: a palette index in palette
//! mode or a VA color code in direct-color mode. Single-plane modes pack pixels
//! linearly (1/4/8/16 bpp); multi-plane 4 bpp combines four 1-bpp planes spaced
//! 64 KiB apart.

use alloc::{boxed::Box, vec};

use super::VA_SURFACE_WIDTH;

/// Number of graphic screens.
pub(super) const GRAPHIC_SCREENS: usize = 2;
/// Raster scratch length: the visible width plus the overshoot a partial first
/// word and 320-dot doubling can produce.
const RASTER_LEN: usize = VA_SURFACE_WIDTH + 64;

/// A graphics framebuffer descriptor (`_FRAMEBUFFER`).
#[derive(Clone, Copy, Default)]
pub struct FramebufferVa {
    /// Frame start address in GVRAM.
    pub frame_start: u32,
    /// Frame buffer width in bytes.
    pub frame_width: u16,
    /// Frame buffer line count (`0xFFFF` marks the no-wrap screen 1).
    pub frame_lines: u16,
    /// Dot address (bit/byte offset within the first word).
    pub dot: u16,
    /// Horizontal offset (`0xFFFF` marks the no-wrap screen 1).
    pub offset_x: u16,
    /// Vertical offset.
    pub offset_y: u16,
    /// Display start address in GVRAM.
    pub display_start: u32,
    /// Sub-screen height in scanlines.
    pub display_height: u16,
    /// Sub-screen position (first scanline).
    pub display_position: u16,
}

impl FramebufferVa {
    /// The reset state for framebuffer 1 (the no-wrap screen-1 sentinels).
    pub fn reset_screen1() -> Self {
        Self {
            frame_start: 0xFFFF_FFFF,
            frame_lines: 0xFFFF,
            offset_x: 0xFFFF,
            offset_y: 0xFFFF,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ScreenWalk {
    y: u16,
    r320dots: bool,
    pixelmode: u8,
    addrmask: u32,
    addrofs: u32,
    lineaddr: u32,
    wrappedaddr: u32,
    vwrapcount: u16,
    current_fb: Option<usize>,
    next_fb: i32,
}

/// Scratch state and output rasters for the graphic-screen walk.
pub(super) struct GraphicsWork {
    rasters: [Box<[u16]>; GRAPHIC_SCREENS],
    noraster: [bool; GRAPHIC_SCREENS],
    screens: [ScreenWalk; GRAPHIC_SCREENS],
}

fn gvram(grphmem: &[u8], addr: u32) -> u8 {
    grphmem.get(addr as usize).copied().unwrap_or(0)
}

fn addr18(value: u32, mask: u32, ofs: u32) -> u32 {
    (value & mask) | ofs
}

impl GraphicsWork {
    pub(super) fn new() -> Self {
        Self {
            rasters: [
                vec![0u16; RASTER_LEN].into_boxed_slice(),
                vec![0u16; RASTER_LEN].into_boxed_slice(),
            ],
            noraster: [true; GRAPHIC_SCREENS],
            screens: [ScreenWalk::default(); GRAPHIC_SCREENS],
        }
    }

    pub(super) fn raster_for(&self, screen: usize) -> &[u16] {
        &self.rasters[screen]
    }

    pub(super) fn has_raster(&self, screen: usize) -> bool {
        !self.noraster[screen]
    }

    fn single_plane(grmode: u16) -> bool {
        grmode & 0x0400 != 0
    }

    /// Initializes the per-screen walk for a frame (`makegrphva_begin`). Returns
    /// the 200-line graphics flag (`grph200`).
    pub(super) fn begin(
        &mut self,
        grmode: u16,
        grres: u16,
        framebuffers: &[FramebufferVa; 4],
    ) -> bool {
        let single = Self::single_plane(grmode);

        self.screens[0].pixelmode = (grres & 0x0003) as u8;
        self.screens[1].pixelmode = ((grres >> 8) & 0x0003) as u8;
        self.screens[0].r320dots = grres & 0x0010 != 0;
        self.screens[1].r320dots = grres & 0x1000 != 0;

        if single {
            self.screens[0].addrmask = if grmode & 0x0800 != 0 {
                0x0001_FFFF
            } else {
                0x0003_FFFF
            };
            self.screens[0].addrofs = 0;
        } else {
            self.screens[0].addrmask = 0x0001_FFFF / 4;
            self.screens[0].addrofs = 0;
        }
        self.screens[1].addrmask = 0x0001_FFFF;
        self.screens[1].addrofs = 0x0002_0000;

        self.screens[0].next_fb = 0;
        self.screens[1].next_fb = 1;
        self.screens[0].y = 0;
        self.screens[1].y = 0;
        self.select_frame(0, -1, framebuffers, single);
        self.select_frame(1, -1, framebuffers, single);

        grmode & 0x0002 != 0
    }

    pub(super) fn blank_raster(&mut self) {
        self.noraster = [true; GRAPHIC_SCREENS];
    }

    /// Produces the next scanline for the active graphic screens
    /// (`makegrphva_raster`).
    pub(super) fn raster(
        &mut self,
        grphmem: &[u8],
        grmode: u16,
        framebuffers: &[FramebufferVa; 4],
        page_mask: u16,
    ) {
        if grmode & 0x8000 == 0 {
            self.blank_raster();
            return;
        }
        let single = Self::single_plane(grmode);
        if single {
            self.draw_raster(0, grphmem, framebuffers, single, page_mask);
            self.draw_raster(1, grphmem, framebuffers, single, page_mask);
        } else {
            self.draw_raster(0, grphmem, framebuffers, single, page_mask);
            self.noraster[1] = true;
        }
    }

    fn select_frame(
        &mut self,
        screen: usize,
        no: i32,
        framebuffers: &[FramebufferVa; 4],
        single: bool,
    ) {
        if !(0..4).contains(&no) {
            self.screens[screen].current_fb = None;
            return;
        }
        let index = no as usize;
        let frame = framebuffers[index];
        self.screens[screen].current_fb = Some(index);
        let (mask, mut ofs) = (self.screens[screen].addrmask, self.screens[screen].addrofs);
        if single {
            self.screens[screen].lineaddr = addr18(frame.display_start, mask, ofs);
            self.screens[screen].wrappedaddr = addr18(
                frame.display_start.wrapping_sub(u32::from(frame.offset_x)),
                mask,
                ofs,
            );
        } else {
            ofs = if frame.frame_start & 0x0002_0000 != 0 {
                0x0002_0000 / 4
            } else {
                0
            };
            self.screens[screen].addrofs = ofs;
            self.screens[screen].lineaddr = addr18(frame.display_start / 4, mask, ofs);
            self.screens[screen].wrappedaddr = addr18(
                (frame.display_start / 4).wrapping_sub(u32::from(frame.offset_x) / 4),
                mask,
                ofs,
            );
        }
        self.screens[screen].vwrapcount = if frame.frame_lines == 0xFFFF {
            0
        } else {
            (i32::from(frame.frame_lines) + 1 - i32::from(frame.offset_y)) as u16
        };

        self.screens[screen].next_fb = match index {
            0 => 2,
            2 => 3,
            _ => -1,
        };
    }

    fn select_next_frame(
        &mut self,
        screen: usize,
        framebuffers: &[FramebufferVa; 4],
        single: bool,
    ) {
        self.screens[screen].current_fb = None;
        let next = self.screens[screen].next_fb;
        if next < 0 {
            return;
        }
        let y = self.screens[screen].y;
        let dsp = framebuffers[next as usize].display_position;
        if dsp < y {
            self.screens[screen].next_fb = -1;
        } else if dsp == y {
            self.select_frame(screen, next, framebuffers, single);
        }
    }

    fn end_raster(&mut self, screen: usize, framebuffers: &[FramebufferVa; 4], single: bool) {
        let s = &mut self.screens[screen];
        let mask = s.addrmask;
        let ofs = s.addrofs;
        let Some(fb) = s.current_fb else {
            return;
        };
        let frame = framebuffers[fb];
        let divisor = if single { 1 } else { 4 };
        s.vwrapcount = s.vwrapcount.wrapping_sub(1);
        if s.vwrapcount == 0 {
            s.wrappedaddr = addr18(frame.frame_start / divisor, mask, ofs);
            s.lineaddr = addr18(
                s.wrappedaddr + u32::from(frame.offset_x) / divisor,
                mask,
                ofs,
            );
        } else {
            let step = u32::from(frame.frame_width) / divisor;
            s.lineaddr = addr18(s.lineaddr + step, mask, ofs);
            s.wrappedaddr = addr18(s.wrappedaddr + step, mask, ofs);
        }
    }

    fn draw_raster(
        &mut self,
        screen: usize,
        grphmem: &[u8],
        framebuffers: &[FramebufferVa; 4],
        single: bool,
        page_mask: u16,
    ) {
        if let Some(fb) = self.screens[screen].current_fb {
            let frame = framebuffers[fb];
            if u32::from(frame.display_position) + u32::from(frame.display_height)
                == u32::from(self.screens[screen].y)
            {
                self.screens[screen].current_fb = None;
            }
        }
        if self.screens[screen].current_fb.is_none() {
            loop {
                self.select_next_frame(screen, framebuffers, single);
                let done = match self.screens[screen].current_fb {
                    None => true,
                    Some(fb) => framebuffers[fb].display_height > 0,
                };
                if done {
                    break;
                }
            }
        }

        self.noraster[screen] = false;
        match self.screens[screen].current_fb {
            None => self.noraster[screen] = true,
            Some(_) if single => match self.screens[screen].pixelmode {
                0 => self.draw_s1(screen, grphmem, framebuffers, page_mask),
                1 => self.draw_s4(screen, grphmem, framebuffers),
                2 => self.draw_s8(screen, grphmem, framebuffers),
                _ => self.draw_s16(screen, grphmem, framebuffers),
            },
            Some(_) => match self.screens[screen].pixelmode {
                1 => self.draw_m4(screen, grphmem, framebuffers),
                _ => {
                    for value in self.rasters[screen].iter_mut() {
                        *value = 0;
                    }
                }
            },
        }
        self.screens[screen].y = self.screens[screen].y.wrapping_add(1);
    }

    /// Reads the big-endian 32-bit word at `addr` and advances `addr` by 4 through `addr18`.
    fn read_dword_be(grphmem: &[u8], addr: u32) -> u32 {
        (u32::from(gvram(grphmem, addr)) << 24)
            | (u32::from(gvram(grphmem, addr + 1)) << 16)
            | (u32::from(gvram(grphmem, addr + 2)) << 8)
            | u32::from(gvram(grphmem, addr + 3))
    }

    fn read_word_le(grphmem: &[u8], addr: u32) -> u16 {
        u16::from(gvram(grphmem, addr)) | (u16::from(gvram(grphmem, addr + 1)) << 8)
    }

    fn draw_s1(
        &mut self,
        screen: usize,
        grphmem: &[u8],
        framebuffers: &[FramebufferVa; 4],
        page_mask: u16,
    ) {
        let s = self.screens[screen];
        let frame = framebuffers[s.current_fb.unwrap()];
        let mut addr = s.lineaddr;
        let mut b = 0usize;
        let mut wrapcount = if frame.offset_x == 0xFFFF {
            0u16
        } else {
            frame.frame_width.wrapping_sub(frame.offset_x)
        };
        let fg = (page_mask & 0x0F00) >> 8;
        let r320 = s.r320dots;
        let buffer = &mut self.rasters[screen];

        let mut dd = Self::read_dword_be(grphmem, addr);
        addr = addr18(addr + 4, s.addrmask, s.addrofs);
        let start = (frame.dot & 0x1F) as u32;
        dd <<= start;
        for _ in start..32 {
            let pixel = if dd & 0x8000_0000 != 0 { fg } else { 0 };
            emit(buffer, &mut b, pixel, r320);
            dd <<= 1;
        }
        let words = if r320 { 320 / 32 } else { 640 / 32 };
        for _ in 0..words {
            wrapcount = wrapcount.wrapping_sub(4);
            if wrapcount == 0 {
                addr = s.wrappedaddr;
            }
            dd = Self::read_dword_be(grphmem, addr);
            addr = addr18(addr + 4, s.addrmask, s.addrofs);
            for _ in 0..32 {
                let pixel = if dd & 0x8000_0000 != 0 { fg } else { 0 };
                emit(buffer, &mut b, pixel, r320);
                dd <<= 1;
            }
        }
        self.end_raster(screen, framebuffers, true);
    }

    fn draw_s4(&mut self, screen: usize, grphmem: &[u8], framebuffers: &[FramebufferVa; 4]) {
        let s = self.screens[screen];
        let frame = framebuffers[s.current_fb.unwrap()];
        let mut addr = s.lineaddr;
        let mut b = 0usize;
        let mut wrapcount = if frame.offset_x == 0xFFFF {
            0u16
        } else {
            frame.frame_width.wrapping_sub(frame.offset_x)
        };
        let r320 = s.r320dots;
        let buffer = &mut self.rasters[screen];

        let mut d = Self::read_word_le(grphmem, addr);
        let mut d2 = Self::read_word_le(grphmem, addr + 2);
        addr = addr18(addr + 4, s.addrmask, s.addrofs);
        let nibbles = [
            (d >> 4) & 0x0F,
            d & 0x0F,
            (d >> 12) & 0x0F,
            (d >> 8) & 0x0F,
            (d2 >> 4) & 0x0F,
            d2 & 0x0F,
            (d2 >> 12) & 0x0F,
            (d2 >> 8) & 0x0F,
        ];
        let start = match frame.dot & 0x13 {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 3,
            0x10 => 4,
            0x11 => 5,
            0x12 => 6,
            _ => 7,
        };
        for &nibble in &nibbles[start..] {
            emit(buffer, &mut b, nibble, r320);
        }
        let groups = if r320 { 320 / 8 } else { 640 / 8 };
        for _ in 0..groups {
            wrapcount = wrapcount.wrapping_sub(4);
            if wrapcount == 0 {
                addr = s.wrappedaddr;
            }
            d = Self::read_word_le(grphmem, addr);
            d2 = Self::read_word_le(grphmem, addr + 2);
            addr = addr18(addr + 4, s.addrmask, s.addrofs);
            for nibble in [
                (d >> 4) & 0x0F,
                d & 0x0F,
                (d >> 12) & 0x0F,
                (d >> 8) & 0x0F,
                (d2 >> 4) & 0x0F,
                d2 & 0x0F,
                (d2 >> 12) & 0x0F,
                (d2 >> 8) & 0x0F,
            ] {
                emit(buffer, &mut b, nibble, r320);
            }
        }
        self.end_raster(screen, framebuffers, true);
    }

    fn draw_s8(&mut self, screen: usize, grphmem: &[u8], framebuffers: &[FramebufferVa; 4]) {
        let s = self.screens[screen];
        let frame = framebuffers[s.current_fb.unwrap()];
        let mut addr = s.lineaddr;
        let mut b = 0usize;
        let mut wrapcount = if frame.offset_x == 0xFFFF {
            0u16
        } else {
            frame.frame_width.wrapping_sub(frame.offset_x)
        };
        let r320 = s.r320dots;
        let buffer = &mut self.rasters[screen];

        let mut d = Self::read_word_le(grphmem, addr);
        let mut d2 = Self::read_word_le(grphmem, addr + 2);
        addr = addr18(addr + 4, s.addrmask, s.addrofs);
        let bytes = [d & 0xFF, (d >> 8) & 0xFF, d2 & 0xFF, (d2 >> 8) & 0xFF];
        let start = match frame.dot & 0x11 {
            0 => 0,
            1 => 1,
            0x10 => 2,
            _ => 3,
        };
        for &byte in &bytes[start..] {
            emit(buffer, &mut b, byte, r320);
        }
        let groups = if r320 { 320 / 4 } else { 640 / 4 };
        for _ in 0..groups {
            wrapcount = wrapcount.wrapping_sub(4);
            if wrapcount == 0 {
                addr = s.wrappedaddr;
            }
            d = Self::read_word_le(grphmem, addr);
            d2 = Self::read_word_le(grphmem, addr + 2);
            addr = addr18(addr + 4, s.addrmask, s.addrofs);
            for byte in [d & 0xFF, (d >> 8) & 0xFF, d2 & 0xFF, (d2 >> 8) & 0xFF] {
                emit(buffer, &mut b, byte, r320);
            }
        }
        self.end_raster(screen, framebuffers, true);
    }

    fn draw_s16(&mut self, screen: usize, grphmem: &[u8], framebuffers: &[FramebufferVa; 4]) {
        let s = self.screens[screen];
        let frame = framebuffers[s.current_fb.unwrap()];
        let mut addr = s.lineaddr;
        let mut b = 0usize;
        let mut wrapcount = if frame.offset_x == 0xFFFF {
            0u16
        } else {
            frame.frame_width.wrapping_sub(frame.offset_x)
        };
        let r320 = s.r320dots;
        let buffer = &mut self.rasters[screen];

        let mut d = Self::read_word_le(grphmem, addr);
        let mut d2 = Self::read_word_le(grphmem, addr + 2);
        addr = addr18(addr + 4, s.addrmask, s.addrofs);
        if frame.dot & 0x10 == 0 {
            emit(buffer, &mut b, d, r320);
        }
        emit(buffer, &mut b, d2, r320);
        let groups = if r320 { 320 / 2 } else { 640 / 2 };
        for _ in 0..groups {
            wrapcount = wrapcount.wrapping_sub(4);
            if wrapcount == 0 {
                addr = s.wrappedaddr;
            }
            d = Self::read_word_le(grphmem, addr);
            d2 = Self::read_word_le(grphmem, addr + 2);
            addr = addr18(addr + 4, s.addrmask, s.addrofs);
            emit(buffer, &mut b, d, r320);
            emit(buffer, &mut b, d2, r320);
        }
        self.end_raster(screen, framebuffers, true);
    }

    fn draw_m4(&mut self, screen: usize, grphmem: &[u8], framebuffers: &[FramebufferVa; 4]) {
        let s = self.screens[screen];
        let frame = framebuffers[s.current_fb.unwrap()];
        let mut addr = s.lineaddr;
        let mut b = 0usize;
        let mut wrapcount = if frame.offset_x == 0xFFFF {
            0u16
        } else {
            (frame.frame_width / 4).wrapping_sub(frame.offset_x / 4)
        };
        let r320 = s.r320dots;
        let buffer = &mut self.rasters[screen];
        for value in buffer.iter_mut() {
            *value = 0;
        }

        let combine = |grphmem: &[u8], addr: u32| -> [u16; 8] {
            let d0 = gvram(grphmem, addr);
            let d1 = gvram(grphmem, addr + 0x10000);
            let d2 = gvram(grphmem, addr + 0x20000);
            let d3 = gvram(grphmem, addr + 0x30000);
            let mut pixels = [0u16; 8];
            for (bit, pixel) in pixels.iter_mut().enumerate() {
                let shift = 7 - bit;
                *pixel = u16::from((d0 >> shift) & 1)
                    | (u16::from((d1 >> shift) & 1) << 1)
                    | (u16::from((d2 >> shift) & 1) << 2)
                    | (u16::from((d3 >> shift) & 1) << 3);
            }
            pixels
        };

        let start = (frame.dot & 0x07) as usize;
        if start > 0 {
            let pixels = combine(grphmem, addr);
            addr = addr18(addr + 1, s.addrmask, s.addrofs);
            for &pixel in &pixels[start..] {
                emit(buffer, &mut b, pixel, r320);
            }
            wrapcount = wrapcount.wrapping_sub(1);
        }
        let bytes = if r320 { 320 / 8 } else { 640 / 8 };
        for _ in 0..bytes {
            if wrapcount == 0 {
                addr = s.wrappedaddr;
            }
            wrapcount = wrapcount.wrapping_sub(1);
            let pixels = combine(grphmem, addr);
            addr = addr18(addr + 1, s.addrmask, s.addrofs);
            for &pixel in &pixels {
                emit(buffer, &mut b, pixel, r320);
            }
        }
        self.end_raster(screen, framebuffers, false);
    }
}

/// Writes one source pixel, doubled for 320-dot mode, advancing the cursor.
fn emit(buffer: &mut [u16], b: &mut usize, value: u16, r320: bool) {
    if *b < buffer.len() {
        buffer[*b] = value;
    }
    *b += 1;
    if r320 {
        if *b < buffer.len() {
            buffer[*b] = value;
        }
        *b += 1;
    }
}
