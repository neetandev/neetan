//! PC-88VA sprite layer: the sprite-definition table walk and per-raster sprite
//! rendering into palette-index scanlines.
//!
//! The sprite table holds 32 eight-byte entries inside text VRAM at `sprtable`.
//! Each entry carries the sprite's vertical size and position, horizontal size,
//! position and color mode, a data pointer, and the foreground/background color.
//! The sprite raster shares the text coordinate system (1024 dots wide; the
//! visible surface samples the left 640) and the compositor splits the combined
//! text/sprite raster by the text/sprite boundary color.

use alloc::{boxed::Box, vec};

/// Number of sprite-table entries.
const SPRITE_COUNT: usize = 0x20;
/// Sprite raster width in dots (the sprite plane is 1024 wide).
pub(super) const SPRITE_RASTER_WIDTH: usize = 0x400;
/// Horizontal wrap mask for the sprite plane.
const SPRITE_X_MASK: usize = SPRITE_RASTER_WIDTH - 1;
/// Palette index written for opaque-background monochrome pixels.
const MONOCHROME_BACKGROUND: u8 = 8;

/// A parsed sprite-table entry (`_SPRVA` plus the cached attribute fields).
#[derive(Clone, Copy, Default)]
struct SpriteEntry {
    /// Display switch (`sw`).
    enabled: bool,
    /// Vertical size in lines (`vlines`).
    vertical_lines: u16,
    /// Vertical display position (`yp`).
    y_position: u16,
    /// Horizontal display position (`xp`).
    x_position: u16,
    /// Sprite width in bytes per line (`xbytes`).
    width_bytes: u16,
    /// Monochrome (1 bpp) mode when set; 16-color (4 bpp) otherwise (`md`).
    monochrome: bool,
    /// Foreground palette index for monochrome mode (`fg`).
    foreground: u8,
    /// Opaque background for monochrome mode (`bg`).
    background_opaque: bool,
    /// Sprite data base offset within text VRAM (`spda`).
    data_offset: u32,
}

/// Scratch state and output buffer for the sprite raster walk (`_SPRVAWORK`).
pub(super) struct SpriteWork {
    /// Current scanline's palette indices across the sprite plane.
    pub(super) sprraster: Box<[u8]>,
    sprites: [SpriteEntry; SPRITE_COUNT],
    /// Current raster in the screen coordinate system (`screeny`).
    screen_y: u32,
    /// Current raster in the sprite coordinate system (`y`).
    sprite_y: u32,
}

fn read_word(text_vram: &[u8], index: usize) -> u16 {
    let low = text_vram.get(index).copied().unwrap_or(0);
    let high = text_vram.get(index + 1).copied().unwrap_or(0);
    u16::from(low) | (u16::from(high) << 8)
}

impl SpriteWork {
    pub(super) fn new() -> Self {
        Self {
            sprraster: vec![0u8; SPRITE_RASTER_WIDTH].into_boxed_slice(),
            sprites: [SpriteEntry::default(); SPRITE_COUNT],
            screen_y: 0,
            sprite_y: 0,
        }
    }

    /// Parses the 32 sprite-table entries and resets the per-frame walk
    /// (`makesprva_begin`). The cursor sprite is hidden during the blink-off
    /// phase, and every sprite is hidden while sprite display is off (SPROFF).
    pub(super) fn begin(
        &mut self,
        text_vram: &[u8],
        sprite_table: usize,
        sprite_enabled: bool,
        cursor_sprite: u8,
        cursor_blink_enable: bool,
        blink_counter2: u8,
    ) {
        self.screen_y = 0;
        self.sprite_y = 0;

        for (index, sprite) in self.sprites.iter_mut().enumerate() {
            let base = sprite_table + index * 8;
            let word0 = read_word(text_vram, base);
            let word1 = read_word(text_vram, base + 2);
            let word2 = read_word(text_vram, base + 4);
            let word3 = read_word(text_vram, base + 6);

            let mut enabled = word0 & 0x0200 != 0;
            // The sprite data pointer is a 16-bit word address: the doubling and
            // the high-bank bias both wrap within 16 bits before becoming a byte
            // offset, so compute them as u16 and only then widen.
            let data_offset = if word2 & 0x8000 != 0 {
                u32::from((word2 << 1).wrapping_add(0x8000))
            } else {
                u32::from(word2 << 1)
            };

            let is_cursor = index == usize::from(cursor_sprite)
                && cursor_blink_enable
                && (blink_counter2 & 0x08) != 0;
            if is_cursor || !sprite_enabled {
                enabled = false;
            }

            *sprite = SpriteEntry {
                enabled,
                vertical_lines: ((word0 >> 10) + 1) * 4,
                y_position: word0 & 0x01FF,
                x_position: word1 & 0x03FF,
                width_bytes: ((word1 >> 11) + 1) * 4,
                monochrome: word1 & 0x0400 != 0,
                foreground: ((word3 & 0x00F0) >> 4) as u8,
                background_opaque: word1 & 0x0400 != 0 && word3 & 0x0008 != 0,
                data_offset,
            };
        }
    }

    /// Clears the output raster (no sprite line).
    pub(super) fn blank_raster(&mut self) {
        for value in self.sprraster.iter_mut() {
            *value = 0;
        }
    }

    /// Produces the next output scanline's sprite palette indices
    /// (`makesprva_raster`). `magnify` doubles each sprite line vertically
    /// (suppressed in 200-line mode); `count_limit` is `hspn`, so at most
    /// `count_limit + 1` sprites draw on one raster (the lowest-indexed win).
    pub(super) fn raster(
        &mut self,
        text_vram: &[u8],
        magnify: bool,
        two_hundred_line: bool,
        count_limit: u8,
    ) {
        let magnify = magnify && !two_hundred_line;
        if !magnify || self.screen_y & 1 == 0 {
            self.blank_raster();

            let limit = usize::from(count_limit) + 1;
            let mut kept = [0usize; SPRITE_COUNT];
            let mut count = 0;
            for index in 0..SPRITE_COUNT {
                let sprite = &self.sprites[index];
                if sprite.enabled
                    && (self.sprite_y.wrapping_sub(u32::from(sprite.y_position)) & 0x01FF)
                        < u32::from(sprite.vertical_lines)
                {
                    if count >= limit {
                        break;
                    }
                    kept[count] = index;
                    count += 1;
                }
            }

            // Draw the kept sprites in descending index order so the lowest
            // index ends up on top.
            for &index in kept[..count].iter().rev() {
                self.draw_raster(text_vram, index);
            }

            self.sprite_y += 1;
        }
        self.screen_y += 1;
    }

    fn draw_raster(&mut self, text_vram: &[u8], index: usize) {
        let sprite = self.sprites[index];
        let line = self.sprite_y.wrapping_sub(u32::from(sprite.y_position)) & 0x01FF;
        let mut address = sprite.data_offset + line * u32::from(sprite.width_bytes);
        let mut x = usize::from(sprite.x_position);

        if sprite.monochrome {
            for _ in 0..sprite.width_bytes {
                let mut data = text_vram.get(address as usize).copied().unwrap_or(0);
                address += 1;
                for _ in 0..8 {
                    if data & 0x80 != 0 {
                        self.sprraster[x] = sprite.foreground;
                    } else if sprite.background_opaque {
                        self.sprraster[x] = MONOCHROME_BACKGROUND;
                    }
                    data <<= 1;
                    x = (x + 1) & SPRITE_X_MASK;
                }
            }
        } else {
            for _ in 0..sprite.width_bytes {
                let data = text_vram.get(address as usize).copied().unwrap_or(0);
                address += 1;
                let high = data >> 4;
                if high != 0 {
                    self.sprraster[x] = high;
                }
                x = (x + 1) & SPRITE_X_MASK;
                let low = data & 0x0F;
                if low != 0 {
                    self.sprraster[x] = low;
                }
                x = (x + 1) & SPRITE_X_MASK;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    /// Builds a text-VRAM image with one sprite-table entry written at slot
    /// `index` of the table at `sprite_table`.
    fn write_entry(vram: &mut [u8], sprite_table: usize, index: usize, words: [u16; 4]) {
        let base = sprite_table + index * 8;
        for (word_index, word) in words.iter().enumerate() {
            vram[base + word_index * 2] = (*word & 0xFF) as u8;
            vram[base + word_index * 2 + 1] = (*word >> 8) as u8;
        }
    }

    const TABLE: usize = 0x100;

    #[test]
    fn sixteen_color_sprite_rasterizes_nibbles() {
        let mut vram = vec![0u8; 0x1000];
        // enable, vlines code 0 (=4), yp 0 | width code 0 (=4 bytes), xp 0, md 0
        // | spda word 0x100 (-> byte offset 0x200) | fg/bg unused.
        write_entry(&mut vram, TABLE, 0, [0x0200, 0x0000, 0x0100, 0x0000]);
        vram[0x200] = 0x12;
        vram[0x201] = 0x34;
        vram[0x202] = 0x00;
        vram[0x203] = 0x56;

        let mut work = SpriteWork::new();
        work.begin(&vram, TABLE, true, 0, false, 0);
        work.raster(&vram, false, false, 31);

        assert_eq!(&work.sprraster[0..8], &[1, 2, 3, 4, 0, 0, 5, 6]);
    }

    #[test]
    fn monochrome_transparent_background_skips_clear_bits() {
        let mut vram = vec![0u8; 0x1000];
        // md set (0x0400), width code 0 | spda 0x100 | fg=5, bg flag clear.
        write_entry(&mut vram, TABLE, 0, [0x0200, 0x0400, 0x0100, 0x0050]);
        vram[0x200] = 0b1010_0001;

        let mut work = SpriteWork::new();
        work.begin(&vram, TABLE, true, 0, false, 0);
        work.raster(&vram, false, false, 31);

        assert_eq!(&work.sprraster[0..8], &[5, 0, 5, 0, 0, 0, 0, 5]);
    }

    #[test]
    fn monochrome_opaque_background_fills_clear_bits() {
        let mut vram = vec![0u8; 0x1000];
        // md set, bg flag set (0x0008) -> clear bits become palette index 8.
        write_entry(&mut vram, TABLE, 0, [0x0200, 0x0400, 0x0100, 0x0058]);
        vram[0x200] = 0b1010_0001;

        let mut work = SpriteWork::new();
        work.begin(&vram, TABLE, true, 0, false, 0);
        work.raster(&vram, false, false, 31);

        assert_eq!(&work.sprraster[0..8], &[5, 8, 5, 8, 8, 8, 8, 5]);
    }

    #[test]
    fn high_bank_data_pointer_wraps_in_16_bits() {
        // A data pointer word with bit 15 set is doubled and biased by 0x8000
        // in 16-bit arithmetic before becoming a byte offset, so word 0xBF80
        // resolves to byte offset 0xFF00, not 0x1FF00.
        let mut vram = vec![0u8; 0x1_0000];
        write_entry(&mut vram, TABLE, 0, [0x0200, 0x0400, 0xBF80, 0x0050]);
        vram[0xFF00] = 0b1000_0000;

        let mut work = SpriteWork::new();
        work.begin(&vram, TABLE, true, 0, false, 0);
        work.raster(&vram, false, false, 31);

        assert_eq!(work.sprraster[0], 5);
    }

    #[test]
    fn lower_index_sprite_wins_overlap() {
        let mut vram = vec![0u8; 0x1000];
        // Sprite 0: nibble 1 at x0. Sprite 1: nibble 2 at x0.
        write_entry(&mut vram, TABLE, 0, [0x0200, 0x0000, 0x0100, 0x0000]);
        write_entry(&mut vram, TABLE, 1, [0x0200, 0x0000, 0x0180, 0x0000]);
        vram[0x200] = 0x10;
        vram[0x300] = 0x20;

        let mut work = SpriteWork::new();
        work.begin(&vram, TABLE, true, 0, false, 0);
        work.raster(&vram, false, false, 31);

        assert_eq!(work.sprraster[0], 1);
    }

    #[test]
    fn per_line_limit_drops_high_index_sprites() {
        let mut vram = vec![0u8; 0x1000];
        // Three sprites on line 0 at xp 0, 2, 4, each with its own data.
        write_entry(&mut vram, TABLE, 0, [0x0200, 0x0000, 0x0100, 0x0000]);
        write_entry(&mut vram, TABLE, 1, [0x0200, 0x0002, 0x0102, 0x0000]);
        write_entry(&mut vram, TABLE, 2, [0x0200, 0x0004, 0x0104, 0x0000]);
        vram[0x200] = 0x10; // sprite 0 -> color 1 at x0
        vram[0x204] = 0x20; // sprite 1 -> color 2 at x2
        vram[0x208] = 0x30; // sprite 2 -> color 3 at x4

        let mut work = SpriteWork::new();
        work.begin(&vram, TABLE, true, 0, false, 0);
        // hspn = 1 -> keep at most 2 sprites; sprite 2 is dropped.
        work.raster(&vram, false, false, 1);

        assert_eq!(work.sprraster[0], 1);
        assert_eq!(work.sprraster[2], 2);
        assert_eq!(work.sprraster[4], 0);
    }

    #[test]
    fn magnification_duplicates_lines() {
        let mut vram = vec![0u8; 0x1000];
        // vlines code 1 (=8 lines), width 4 bytes, spda 0x100.
        write_entry(&mut vram, TABLE, 0, [0x0600, 0x0000, 0x0100, 0x0000]);
        vram[0x200] = 0x10; // line 0 -> color 1
        vram[0x204] = 0x20; // line 1 -> color 2

        let mut work = SpriteWork::new();
        work.begin(&vram, TABLE, true, 0, false, 0);
        work.raster(&vram, true, false, 31);
        assert_eq!(work.sprraster[0], 1);
        // Magnified odd raster reuses the previous sprite line.
        work.raster(&vram, true, false, 31);
        assert_eq!(work.sprraster[0], 1);
        // Next even raster advances to the second sprite line.
        work.raster(&vram, true, false, 31);
        assert_eq!(work.sprraster[0], 2);
    }

    #[test]
    fn two_hundred_line_suppresses_magnification() {
        let mut vram = vec![0u8; 0x1000];
        write_entry(&mut vram, TABLE, 0, [0x0600, 0x0000, 0x0100, 0x0000]);
        vram[0x200] = 0x10;
        vram[0x204] = 0x20;

        let mut work = SpriteWork::new();
        work.begin(&vram, TABLE, true, 0, false, 0);
        work.raster(&vram, true, true, 31);
        assert_eq!(work.sprraster[0], 1);
        work.raster(&vram, true, true, 31);
        assert_eq!(work.sprraster[0], 2);
    }

    #[test]
    fn cursor_sprite_blinks() {
        let mut vram = vec![0u8; 0x1000];
        write_entry(&mut vram, TABLE, 0, [0x0200, 0x0000, 0x0100, 0x0000]);
        vram[0x200] = 0x10;

        // Blink-off phase (bit 3 set): the cursor sprite is hidden.
        let mut work = SpriteWork::new();
        work.begin(&vram, TABLE, true, 0, true, 0x08);
        work.raster(&vram, false, false, 31);
        assert_eq!(work.sprraster[0], 0);

        // Blink-on phase: the cursor sprite renders.
        work.begin(&vram, TABLE, true, 0, true, 0x00);
        work.raster(&vram, false, false, 31);
        assert_eq!(work.sprraster[0], 1);
    }

    #[test]
    fn sprite_off_hides_all_sprites() {
        let mut vram = vec![0u8; 0x1000];
        write_entry(&mut vram, TABLE, 0, [0x0200, 0x0000, 0x0100, 0x0000]);
        vram[0x200] = 0x10;

        let mut work = SpriteWork::new();
        work.begin(&vram, TABLE, false, 0, false, 0);
        work.raster(&vram, false, false, 31);
        assert_eq!(work.sprraster[0], 0);
    }
}
