//! FM Towns sprite engine rasterizer.
//!
//! The sprite hardware is a VRAM blitter: it walks the attribute table in sprite
//! RAM and paints 16x16 patterns as 16-bit direct-color pixels into VRAM layer 1
//! (the second 256 KiB VRAM region). The CRTC then displays that region as an
//! ordinary 16 bpp layer, converting the packed 5-5-5 words to RGBA. Layer 1 is
//! double-buffered across two 128 KiB pages; the engine renders into one page
//! while the CRTC displays the other.

/// Byte offset of VRAM layer 1 (the sprite region) within the 1 MiB VRAM.
pub const TOWNS_SPRITE_LAYER_VRAM_OFFSET: usize = 0x0004_0000;

/// One sprite page: 256 lines of 512 bytes = 128 KiB.
const SPRITE_PAGE_SIZE: usize = 128 * 1024;
/// Bytes per line in the sprite VRAM page (256 pixels x 2 bytes).
const SPRITE_VRAM_BYTES_PER_LINE: usize = 512;
/// The top two lines hold the screen-clear fill pattern that is tiled downward.
const SPRITE_CLEAR_BLOCK: usize = SPRITE_VRAM_BYTES_PER_LINE * 2;

/// Sprites are 16x16.
const SPRITE_DIMENSION: usize = 16;
/// Number of attribute entries in the table.
const MAX_NUM_SPRITE_INDEX: usize = 1024;
/// Each attribute entry is four little-endian 16-bit words.
const ATTRIBUTE_ENTRY_BYTES: usize = 8;

/// A 16-color pattern is 128 bytes: 16 lines of 8 bytes (4 bits per pixel).
const PATTERN16_BYTES_PER_LINE: usize = 8;
/// A 32768-color pattern line is 32 bytes (16 pixels x 2 bytes).
const PATTERN32K_BYTES_PER_LINE: usize = 32;
/// Every pattern index step covers one 128-byte pattern slot.
const PATTERN_SLOT_SHIFT: u32 = 7;
/// Every palette index step covers one 32-byte 16-color palette.
const PALETTE_SLOT_SHIFT: u32 = 5;

/// Attribute word bit fields.
const ATTR_PATTERN_MASK: u16 = 0x03FF;
const ATTR_SHRINK_X: u16 = 0x0400;
const ATTR_SHRINK_Y: u16 = 0x0800;
const ATTR_ROTATION_MASK: u16 = 0x7000;
const ATTR_ROTATION_SHIFT: u16 = 12;
const ATTR_USE_OFFSET: u16 = 0x8000;

/// Palette/control word bit fields.
const PALETTE_INDEX_MASK: u16 = 0x0FFF;
const PALETTE_HIDE: u16 = 0x2000;
const PALETTE_SPYS: u16 = 0x4000;
const PALETTE_COLOR16: u16 = 0x8000;

/// Position field mask (9 bits).
const POSITION_MASK: u32 = 0x01FF;
/// High byte of a written pixel keeps 7 bits of color; bit 7 is the SPYS flag.
const PIXEL_COLOR_HIGH_MASK: u8 = 0x7F;
/// SPYS bit forced into the high byte of every written pixel.
const PIXEL_SPYS_BIT: u8 = 0x80;
/// Transparency flag in a 32768-color source pixel's high byte.
const PIXEL_TRANSPARENT_BIT: u8 = 0x80;

/// Parameters captured from the sprite register file for one render pass.
#[derive(Clone, Copy, Debug, Default)]
pub struct SpriteRenderParams {
    /// Internal render page (0 or 1) selecting which 128 KiB half to paint.
    pub page: usize,
    /// First attribute index to draw; entries below it are skipped.
    pub first_index: usize,
    /// Horizontal offset added to sprites with the OFFS attribute.
    pub h_offset: u32,
    /// Vertical offset added to sprites with the OFFS attribute.
    pub v_offset: u32,
}

/// Maps a pattern coordinate to a draw coordinate under one of the eight
/// rotation/flip codes, then applies the optional half-size shrink.
fn transform(
    rotation: u16,
    shrink_x: bool,
    shrink_y: bool,
    px: usize,
    py: usize,
) -> (usize, usize) {
    let last = SPRITE_DIMENSION - 1;
    let (mut dx, mut dy) = match rotation {
        1 => (px, last - py),
        2 => (last - px, py),
        3 => (last - px, last - py),
        4 => (py, px),
        5 => (py, last - px),
        6 => (last - py, px),
        7 => (last - py, last - px),
        _ => (px, py),
    };
    if shrink_x {
        dx >>= 1;
    }
    if shrink_y {
        dy >>= 1;
    }
    (dx, dy)
}

/// Reads a little-endian 16-bit word from sprite RAM, wrapping at its length.
fn read_word(sprite_ram: &[u8], offset: usize) -> u16 {
    let mask = sprite_ram.len().wrapping_sub(1);
    u16::from(sprite_ram[offset & mask]) | (u16::from(sprite_ram[(offset + 1) & mask]) << 8)
}

/// Renders all enabled sprites into the selected VRAM page.
pub fn render_sprites(vram: &mut [u8], sprite_ram: &[u8], params: &SpriteRenderParams) {
    let page_base = TOWNS_SPRITE_LAYER_VRAM_OFFSET + SPRITE_PAGE_SIZE * (params.page & 1);
    if page_base + SPRITE_PAGE_SIZE > vram.len() {
        return;
    }
    clear_page(&mut vram[page_base..page_base + SPRITE_PAGE_SIZE]);

    let sprite_ram_mask = sprite_ram.len().wrapping_sub(1);
    for index in params.first_index..MAX_NUM_SPRITE_INDEX {
        let entry = index * ATTRIBUTE_ENTRY_BYTES;
        let mut dst_x = u32::from(read_word(sprite_ram, entry)) & POSITION_MASK;
        let mut dst_y = u32::from(read_word(sprite_ram, entry + 2)) & POSITION_MASK;
        let attribute = read_word(sprite_ram, entry + 4);
        let palette_info = read_word(sprite_ram, entry + 6);

        if palette_info & PALETTE_HIDE != 0 {
            continue;
        }
        if attribute & ATTR_USE_OFFSET != 0 {
            dst_x += params.h_offset;
            dst_y += params.v_offset;
        }

        let rotation = (attribute & ATTR_ROTATION_MASK) >> ATTR_ROTATION_SHIFT;
        let shrink_x = attribute & ATTR_SHRINK_X != 0;
        let shrink_y = attribute & ATTR_SHRINK_Y != 0;
        let spys = if palette_info & PALETTE_SPYS != 0 {
            PIXEL_SPYS_BIT
        } else {
            0
        };

        if palette_info & PALETTE_COLOR16 != 0 {
            let pattern_base = (usize::from(attribute & ATTR_PATTERN_MASK) << PATTERN_SLOT_SHIFT)
                & sprite_ram_mask;
            let palette_base = (usize::from(palette_info & PALETTE_INDEX_MASK)
                << PALETTE_SLOT_SHIFT)
                & sprite_ram_mask;
            draw_pattern_16color(
                vram,
                sprite_ram,
                sprite_ram_mask,
                page_base,
                pattern_base,
                palette_base,
                dst_x,
                dst_y,
                rotation,
                shrink_x,
                shrink_y,
                spys,
            );
        } else {
            // A 32768-color pattern spans four 128-byte slots.
            let pattern_index = usize::from(attribute & ATTR_PATTERN_MASK) & !3;
            let pattern_base = (pattern_index << PATTERN_SLOT_SHIFT) & sprite_ram_mask;
            draw_pattern_32k(
                vram,
                sprite_ram,
                sprite_ram_mask,
                page_base,
                pattern_base,
                dst_x,
                dst_y,
                rotation,
                shrink_x,
                shrink_y,
                spys,
            );
        }
    }
}

/// Clears a sprite page by tiling its top two lines across the whole page. The
/// top two lines hold the fill pattern, so software can clear to any value.
fn clear_page(page: &mut [u8]) {
    let mut fill = [0u8; SPRITE_CLEAR_BLOCK];
    fill.copy_from_slice(&page[..SPRITE_CLEAR_BLOCK]);
    let mut offset = SPRITE_CLEAR_BLOCK;
    while offset < page.len() {
        page[offset..offset + SPRITE_CLEAR_BLOCK].copy_from_slice(&fill);
        offset += SPRITE_CLEAR_BLOCK;
    }
}

/// Computes the destination VRAM offset for a sprite pixel, or `None` when the
/// pixel falls outside the drawable region (`sx < 256 && 2 <= sy < 256`).
fn pixel_destination(
    page_base: usize,
    dst_x: u32,
    dst_y: u32,
    dx: usize,
    dy: usize,
) -> Option<usize> {
    let sx = (dst_x + dx as u32) & POSITION_MASK;
    let sy = (dst_y + dy as u32) & POSITION_MASK;
    if sx < 256 && (2..256).contains(&sy) {
        Some(page_base + SPRITE_VRAM_BYTES_PER_LINE * sy as usize + 2 * sx as usize)
    } else {
        None
    }
}

/// Rasterizes one 16-color sprite pattern. Nibble value 0 is transparent.
#[allow(clippy::too_many_arguments)]
fn draw_pattern_16color(
    vram: &mut [u8],
    sprite_ram: &[u8],
    sprite_ram_mask: usize,
    page_base: usize,
    pattern_base: usize,
    palette_base: usize,
    dst_x: u32,
    dst_y: u32,
    rotation: u16,
    shrink_x: bool,
    shrink_y: bool,
    spys: u8,
) {
    for pattern_y in 0..SPRITE_DIMENSION {
        for pattern_x in 0..SPRITE_DIMENSION {
            let (dx, dy) = transform(rotation, shrink_x, shrink_y, pattern_x, pattern_y);
            let Some(destination) = pixel_destination(page_base, dst_x, dst_y, dx, dy) else {
                continue;
            };
            let source = (pattern_base + PATTERN16_BYTES_PER_LINE * pattern_y + (pattern_x >> 1))
                & sprite_ram_mask;
            let byte = sprite_ram[source];
            let nibble = if pattern_x & 1 == 0 {
                byte & 0x0F
            } else {
                byte >> 4
            };
            if nibble == 0 {
                continue;
            }
            let color = (palette_base + usize::from(nibble) * 2) & sprite_ram_mask;
            vram[destination] = sprite_ram[color];
            vram[destination + 1] =
                (sprite_ram[(color + 1) & sprite_ram_mask] & PIXEL_COLOR_HIGH_MASK) | spys;
        }
    }
}

/// Rasterizes one 32768-color sprite pattern. A source pixel with bit 15 set is
/// transparent.
#[allow(clippy::too_many_arguments)]
fn draw_pattern_32k(
    vram: &mut [u8],
    sprite_ram: &[u8],
    sprite_ram_mask: usize,
    page_base: usize,
    pattern_base: usize,
    dst_x: u32,
    dst_y: u32,
    rotation: u16,
    shrink_x: bool,
    shrink_y: bool,
    spys: u8,
) {
    for pattern_y in 0..SPRITE_DIMENSION {
        for pattern_x in 0..SPRITE_DIMENSION {
            let (dx, dy) = transform(rotation, shrink_x, shrink_y, pattern_x, pattern_y);
            let Some(destination) = pixel_destination(page_base, dst_x, dst_y, dx, dy) else {
                continue;
            };
            let source = (pattern_base + PATTERN32K_BYTES_PER_LINE * pattern_y + 2 * pattern_x)
                & sprite_ram_mask;
            let low = sprite_ram[source];
            let high = sprite_ram[(source + 1) & sprite_ram_mask];
            if high & PIXEL_TRANSPARENT_BIT == 0 {
                vram[destination] = low;
                vram[destination + 1] = high | spys;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    const VRAM_SIZE: usize = 0x10_0000;
    const SPRITE_RAM_SIZE: usize = 0x2_0000;

    /// Sprite RAM with every attribute entry hidden, so a test sees only the
    /// sprites it explicitly writes (an all-zero entry is a valid opaque
    /// 32768-color sprite at the origin, which would clobber isolated pixels).
    fn hidden_sprite_ram() -> Vec<u8> {
        let mut sprite_ram = vec![0u8; SPRITE_RAM_SIZE];
        for index in 0..MAX_NUM_SPRITE_INDEX {
            write_attribute(&mut sprite_ram, index, [0, 0, 0, PALETTE_HIDE]);
        }
        sprite_ram
    }

    fn write_attribute(sprite_ram: &mut [u8], index: usize, words: [u16; 4]) {
        let base = index * ATTRIBUTE_ENTRY_BYTES;
        for (word, chunk) in words.iter().zip(sprite_ram[base..].chunks_mut(2)) {
            chunk[0] = *word as u8;
            chunk[1] = (*word >> 8) as u8;
        }
    }

    fn read_pixel(vram: &[u8], page: usize, x: usize, y: usize) -> u16 {
        let base = TOWNS_SPRITE_LAYER_VRAM_OFFSET
            + SPRITE_PAGE_SIZE * page
            + SPRITE_VRAM_BYTES_PER_LINE * y
            + 2 * x;
        u16::from(vram[base]) | (u16::from(vram[base + 1]) << 8)
    }

    #[test]
    fn transform_covers_all_rotations() {
        assert_eq!(transform(0, false, false, 1, 2), (1, 2));
        assert_eq!(transform(1, false, false, 1, 2), (1, 13));
        assert_eq!(transform(2, false, false, 1, 2), (14, 2));
        assert_eq!(transform(3, false, false, 1, 2), (14, 13));
        assert_eq!(transform(4, false, false, 1, 2), (2, 1));
        assert_eq!(transform(5, false, false, 1, 2), (2, 14));
        assert_eq!(transform(6, false, false, 1, 2), (13, 1));
        assert_eq!(transform(7, false, false, 1, 2), (13, 14));
    }

    #[test]
    fn transform_shrink_halves_axes() {
        assert_eq!(transform(0, true, false, 15, 15), (7, 15));
        assert_eq!(transform(0, false, true, 15, 15), (15, 7));
        assert_eq!(transform(0, true, true, 15, 15), (7, 7));
    }

    #[test]
    fn draws_16color_pixel_with_palette_and_skips_nibble_zero() {
        let mut vram = vec![0u8; VRAM_SIZE];
        let mut sprite_ram = hidden_sprite_ram();

        // Pattern 64, palette 256, 16-color, placed at (10, 10). The indices are
        // chosen so the pattern/palette bytes sit above the 8 KiB attribute
        // table and do not overlap any entry.
        let pattern_index = 64;
        let palette_index = 256;
        let pattern_base = pattern_index << PATTERN_SLOT_SHIFT;
        let palette_base = palette_index << PALETTE_SLOT_SHIFT;
        // Pattern pixel (0,0) = nibble 3, pixel (1,0) = nibble 0 (transparent).
        sprite_ram[pattern_base] = 0x03;
        // Palette entry 3 = 0x1234.
        sprite_ram[palette_base + 3 * 2] = 0x34;
        sprite_ram[palette_base + 3 * 2 + 1] = 0x12;

        write_attribute(
            &mut sprite_ram,
            0,
            [
                10,
                10,
                pattern_index as u16,
                PALETTE_COLOR16 | palette_index as u16,
            ],
        );
        let params = SpriteRenderParams {
            page: 0,
            first_index: 0,
            h_offset: 0,
            v_offset: 0,
        };
        render_sprites(&mut vram, &sprite_ram, &params);

        // High byte keeps 7 bits: 0x12 & 0x7F = 0x12.
        assert_eq!(read_pixel(&vram, 0, 10, 10), 0x1234);
        // Nibble-0 pixel stays cleared.
        assert_eq!(read_pixel(&vram, 0, 11, 10), 0x0000);
    }

    #[test]
    fn draws_32k_pixel_and_skips_transparent() {
        let mut vram = vec![0u8; VRAM_SIZE];
        let mut sprite_ram = hidden_sprite_ram();

        let pattern_base = 4 << PATTERN_SLOT_SHIFT;
        // Pixel (0,0) opaque 0x1234, pixel (1,0) transparent (bit 15 set).
        sprite_ram[pattern_base] = 0x34;
        sprite_ram[pattern_base + 1] = 0x12;
        sprite_ram[pattern_base + 2] = 0xFF;
        sprite_ram[pattern_base + 3] = 0x80;

        // 32K mode: CTEN clear. Pattern index 4.
        write_attribute(&mut sprite_ram, 0, [20, 20, 4, 0]);
        let params = SpriteRenderParams {
            page: 0,
            first_index: 0,
            h_offset: 0,
            v_offset: 0,
        };
        render_sprites(&mut vram, &sprite_ram, &params);

        assert_eq!(read_pixel(&vram, 0, 20, 20), 0x1234);
        assert_eq!(read_pixel(&vram, 0, 21, 20), 0x0000);
    }

    #[test]
    fn spys_forces_high_bit() {
        let mut vram = vec![0u8; VRAM_SIZE];
        let mut sprite_ram = hidden_sprite_ram();
        let pattern_base = 4 << PATTERN_SLOT_SHIFT;
        sprite_ram[pattern_base] = 0x34;
        sprite_ram[pattern_base + 1] = 0x12;
        write_attribute(&mut sprite_ram, 0, [30, 30, 4, PALETTE_SPYS]);
        let params = SpriteRenderParams {
            page: 0,
            first_index: 0,
            h_offset: 0,
            v_offset: 0,
        };
        render_sprites(&mut vram, &sprite_ram, &params);
        assert_eq!(read_pixel(&vram, 0, 30, 30), 0x9234);
    }

    #[test]
    fn hidden_sprite_is_skipped() {
        let mut vram = vec![0u8; VRAM_SIZE];
        let mut sprite_ram = hidden_sprite_ram();
        let pattern_base = 4 << PATTERN_SLOT_SHIFT;
        sprite_ram[pattern_base] = 0x34;
        sprite_ram[pattern_base + 1] = 0x12;
        write_attribute(&mut sprite_ram, 0, [30, 30, 4, PALETTE_HIDE]);
        let params = SpriteRenderParams {
            page: 0,
            first_index: 0,
            h_offset: 0,
            v_offset: 0,
        };
        render_sprites(&mut vram, &sprite_ram, &params);
        assert_eq!(read_pixel(&vram, 0, 30, 30), 0x0000);
    }

    #[test]
    fn offset_attribute_shifts_position() {
        let mut vram = vec![0u8; VRAM_SIZE];
        let mut sprite_ram = hidden_sprite_ram();
        let pattern_base = 4 << PATTERN_SLOT_SHIFT;
        sprite_ram[pattern_base] = 0x34;
        sprite_ram[pattern_base + 1] = 0x12;
        write_attribute(&mut sprite_ram, 0, [10, 10, 4 | ATTR_USE_OFFSET, 0]);
        let params = SpriteRenderParams {
            page: 0,
            first_index: 0,
            h_offset: 5,
            v_offset: 7,
        };
        render_sprites(&mut vram, &sprite_ram, &params);
        assert_eq!(read_pixel(&vram, 0, 15, 17), 0x1234);
    }

    #[test]
    fn first_index_skips_lower_sprites() {
        let mut vram = vec![0u8; VRAM_SIZE];
        let mut sprite_ram = hidden_sprite_ram();
        let pattern_base = 4 << PATTERN_SLOT_SHIFT;
        sprite_ram[pattern_base] = 0x34;
        sprite_ram[pattern_base + 1] = 0x12;
        // Sprite 0 would draw at (40,40); it must be skipped by first_index=1.
        write_attribute(&mut sprite_ram, 0, [40, 40, 4, 0]);
        let params = SpriteRenderParams {
            page: 0,
            first_index: 1,
            h_offset: 0,
            v_offset: 0,
        };
        render_sprites(&mut vram, &sprite_ram, &params);
        assert_eq!(read_pixel(&vram, 0, 40, 40), 0x0000);
    }

    #[test]
    fn screen_clear_tiles_top_two_lines() {
        let mut vram = vec![0u8; VRAM_SIZE];
        let sprite_ram = hidden_sprite_ram();
        // Seed the page's first pixel with a fill value; it should tile down.
        let page_base = TOWNS_SPRITE_LAYER_VRAM_OFFSET;
        vram[page_base] = 0xCD;
        vram[page_base + 1] = 0xAB;
        let params = SpriteRenderParams::default();
        render_sprites(&mut vram, &sprite_ram, &params);
        // The same fill appears two lines down (the clear block boundary).
        assert_eq!(read_pixel(&vram, 0, 0, 2), 0xABCD);
    }

    #[test]
    fn renders_into_second_page() {
        let mut vram = vec![0u8; VRAM_SIZE];
        let mut sprite_ram = hidden_sprite_ram();
        let pattern_base = 4 << PATTERN_SLOT_SHIFT;
        sprite_ram[pattern_base] = 0x34;
        sprite_ram[pattern_base + 1] = 0x12;
        write_attribute(&mut sprite_ram, 0, [50, 50, 4, 0]);
        let params = SpriteRenderParams {
            page: 1,
            first_index: 0,
            h_offset: 0,
            v_offset: 0,
        };
        render_sprites(&mut vram, &sprite_ram, &params);
        assert_eq!(read_pixel(&vram, 1, 50, 50), 0x1234);
        // Page 0 untouched.
        assert_eq!(read_pixel(&vram, 0, 50, 50), 0x0000);
    }
}
