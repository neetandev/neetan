//! X68000 sprite and background line rasterization.
//!
//! One visible line is rasterized at a time into per-plane 4-bit code and
//! palette-block buffers ordered SP3 > BG0 > SP2 > BG1 > SP1 front to back,
//! then folded into 8-bit sprite-screen codes. Sprites are evaluated in
//! ascending number; at most 32 sprites take part in one raster and a
//! sprite crossing the raster consumes a slot even when it is fully
//! outside the screen horizontally. Within one plane the lower sprite
//! number wins; a background pixel with code zero but a non-zero palette
//! block is opaque, and among those the deepest background shows.

use alloc::vec::Vec;

use super::RenderInputsX68k;

/// Number of sprites evaluated per raster before the hardware stops drawing.
const SPRITES_PER_RASTER_LIMIT: u32 = 32;
/// Margin of the line buffers left and right of the visible pixels.
const LINE_BUFFER_MARGIN: usize = 16;
/// Length of the plane line buffers (margins around the widest screen).
const LINE_BUFFER_SIZE: usize = 1056;
/// Sprite coordinate value that lands on the screen's left or top edge.
const SPRITE_ORIGIN_OFFSET: i32 = 16;
/// Background control bit enabling the whole sprite screen.
const CONTROL_DISPLAY_ENABLE: u16 = 0x0200;
/// Background control bit selecting the absent chip for BG1's tile map.
const CONTROL_BACKGROUND_1_CHIP_SELECT: u16 = 0x0020;
/// Background control bit selecting BG1's tile map.
const CONTROL_BACKGROUND_1_MAP_SELECT: u16 = 0x0010;
/// Background control bit enabling BG1.
const CONTROL_BACKGROUND_1_ENABLE: u16 = 0x0008;
/// Background control bit selecting the absent chip for BG0's tile map.
const CONTROL_BACKGROUND_0_CHIP_SELECT: u16 = 0x0004;
/// Background control bit selecting BG0's tile map.
const CONTROL_BACKGROUND_0_MAP_SELECT: u16 = 0x0002;
/// Background control bit enabling BG0.
const CONTROL_BACKGROUND_0_ENABLE: u16 = 0x0001;
/// Resolution register bits selecting 16x16 background tiles.
const RESOLUTION_TILE_SIZE_MASK: u16 = 0x0003;
/// Word offset of the first background tile map in the pattern RAM.
const TILE_MAP_0_WORD_OFFSET: usize = 0x2000;
/// Word offset of the second background tile map in the pattern RAM.
const TILE_MAP_1_WORD_OFFSET: usize = 0x3000;
/// Sprite priority-word bit selecting the absent second pattern chip.
const SPRITE_CHIP_SELECT: u16 = 0x0004;
/// Pattern-word and map-entry bit flipping the pattern vertically.
const PATTERN_VERTICAL_FLIP: u16 = 0x8000;
/// Pattern-word and map-entry bit flipping the pattern horizontally.
const PATTERN_HORIZONTAL_FLIP: u16 = 0x4000;
/// Shift of the palette block in a pattern word or map entry.
const PATTERN_BLOCK_SHIFT: u32 = 8;
/// Mask of the pattern number in a pattern word or map entry.
const PATTERN_NUMBER_MASK: u16 = 0x00FF;
/// Nibble shift of the BG0 plane in the line buffers.
const BACKGROUND_0_SHIFT: u32 = 20;
/// Nibble shift of the BG1 plane in the line buffers.
const BACKGROUND_1_SHIFT: u32 = 12;
/// Line-buffer mask covering the two background plane nibbles.
const BACKGROUND_BLOCK_MASK: u32 = 0x00F0_F000;

/// Per-plane 4-bit code and palette-block line buffers.
struct LineBuffers {
    codes: [u32; LINE_BUFFER_SIZE],
    blocks: [u32; LINE_BUFFER_SIZE],
}

/// Rasterizes the sprite and background screens into 8-bit codes for one line.
pub(super) fn rasterize_sprite_line(
    inputs: &RenderInputsX68k,
    screen_y: usize,
    line: &mut Vec<u8>,
) {
    line.clear();
    line.resize(inputs.width as usize, 0);
    if !inputs.sprite_area_accessible || inputs.background_control & CONTROL_DISPLAY_ENABLE == 0 {
        return;
    }
    let raster = screen_y as i32 + i32::from(inputs.sprite_vertical_back_end)
        - i32::from(inputs.crtc_vertical_back_end);
    if !(0..=1023).contains(&raster) {
        return;
    }
    let horizontal_start = (i32::from(inputs.sprite_horizontal_back_end)
        - i32::from(inputs.crtc_horizontal_back_end)
        - 4)
        * 8;
    let width = (inputs.width as usize).min(LINE_BUFFER_SIZE - 2 * LINE_BUFFER_MARGIN);
    let visible_end = LINE_BUFFER_MARGIN + width;
    let mut buffers = LineBuffers {
        codes: [0; LINE_BUFFER_SIZE],
        blocks: [0; LINE_BUFFER_SIZE],
    };
    place_sprites(inputs, raster, horizontal_start, visible_end, &mut buffers);
    let control = inputs.background_control;
    if inputs.sprite_resolution & RESOLUTION_TILE_SIZE_MASK == 0 {
        if control & CONTROL_BACKGROUND_0_ENABLE != 0 {
            place_background_8(
                inputs,
                raster,
                horizontal_start,
                visible_end,
                0,
                &mut buffers,
            );
        }
        if control & CONTROL_BACKGROUND_1_ENABLE != 0 {
            place_background_8(
                inputs,
                raster,
                horizontal_start,
                visible_end,
                1,
                &mut buffers,
            );
        }
    } else if control & CONTROL_BACKGROUND_0_ENABLE != 0 {
        place_background_16(inputs, raster, horizontal_start, visible_end, &mut buffers);
    }
    for (screen_x, output) in line.iter_mut().take(width).enumerate() {
        *output = fold_pixel(
            buffers.codes[LINE_BUFFER_MARGIN + screen_x],
            buffers.blocks[LINE_BUFFER_MARGIN + screen_x],
        );
    }
}

/// Places the sprites crossing one raster into the plane buffers.
fn place_sprites(
    inputs: &RenderInputsX68k,
    raster: i32,
    horizontal_start: i32,
    visible_end: usize,
    buffers: &mut LineBuffers,
) {
    let mut slots = SPRITES_PER_RASTER_LIMIT;
    for entry in inputs.sprite_scroll.iter() {
        let priority = entry[3] & 3;
        if priority == 0 {
            continue;
        }
        let row = raster - i32::from(entry[1]) + SPRITE_ORIGIN_OFFSET;
        if !(0..16).contains(&row) {
            continue;
        }
        let position = horizontal_start + i32::from(entry[0]);
        // A set chip-select bit fetches every dot from the absent second
        // pattern chip: the sprite stays invisible but keeps its slot.
        let chip_selected = entry[3] & SPRITE_CHIP_SELECT != 0;
        if !chip_selected && position > 0 && position < visible_end as i32 {
            let pattern_word = entry[2];
            let base = usize::from(pattern_word & PATTERN_NUMBER_MASK) * 64;
            let block = u32::from(pattern_word >> PATTERN_BLOCK_SHIFT & 0x000F);
            let fetch_row = if pattern_word & PATTERN_VERTICAL_FLIP != 0 {
                15 - row as usize
            } else {
                row as usize
            };
            let shift = u32::from(priority) * 8;
            for column in 0..16 {
                let source_column = if pattern_word & PATTERN_HORIZONTAL_FLIP != 0 {
                    15 - column
                } else {
                    column
                };
                let code = u32::from(pattern_pixel_16(
                    inputs.sprite_pattern,
                    base,
                    source_column,
                    fetch_row,
                ));
                if code == 0 {
                    continue;
                }
                let index = position as usize + column;
                if buffers.codes[index] >> shift != 0 {
                    continue;
                }
                buffers.codes[index] |= code << shift;
                buffers.blocks[index] |= block << shift;
            }
        }
        slots -= 1;
        if slots == 0 {
            return;
        }
    }
}

/// Places one 8x8-tile background layer into the plane buffers.
fn place_background_8(
    inputs: &RenderInputsX68k,
    raster: i32,
    horizontal_start: i32,
    visible_end: usize,
    layer: usize,
    buffers: &mut LineBuffers,
) {
    let shift = if layer == 0 {
        BACKGROUND_0_SHIFT
    } else {
        BACKGROUND_1_SHIFT
    };
    let map_offset = background_map_offset(inputs.background_control, layer);
    let vertical = ((raster + i32::from(inputs.background_scroll[layer * 2 + 1])) & 511) as usize;
    let map_row = (vertical >> 3) * 64;
    let row = vertical & 7;
    let start = LINE_BUFFER_MARGIN as i32 + horizontal_start
        - i32::from(inputs.background_scroll[layer * 2]);
    let mut map_column = ((((start & 7) - start) >> 3) & 63) as usize;
    let mut position = (start & 7) as usize;
    while position < visible_end {
        let entry = match map_offset {
            Some(offset) => inputs.sprite_pattern[offset + map_row + map_column],
            None => 0,
        };
        let base = usize::from(entry & PATTERN_NUMBER_MASK) * 16;
        let block = u32::from(entry >> PATTERN_BLOCK_SHIFT & 0x000F);
        let fetch_row = if entry & PATTERN_VERTICAL_FLIP != 0 {
            7 - row
        } else {
            row
        };
        for column in 0..8 {
            let source_column = if entry & PATTERN_HORIZONTAL_FLIP != 0 {
                7 - column
            } else {
                column
            };
            let code = u32::from(pattern_pixel_8(
                inputs.sprite_pattern,
                base,
                source_column,
                fetch_row,
            ));
            buffers.codes[position + column] |= code << shift;
            if block != 0 {
                buffers.blocks[position + column] |= block << shift;
            }
        }
        position += 8;
        map_column = (map_column + 1) & 63;
    }
}

/// Places the 16x16-tile BG0 layer into the plane buffers.
fn place_background_16(
    inputs: &RenderInputsX68k,
    raster: i32,
    horizontal_start: i32,
    visible_end: usize,
    buffers: &mut LineBuffers,
) {
    let map_offset = background_map_offset(inputs.background_control, 0);
    let vertical = ((raster + i32::from(inputs.background_scroll[1])) & 1023) as usize;
    let map_row = (vertical >> 4) * 64;
    let row = vertical & 15;
    let start =
        LINE_BUFFER_MARGIN as i32 + horizontal_start - i32::from(inputs.background_scroll[0]);
    let mut map_column = ((((start & 15) - start) >> 4) & 63) as usize;
    let mut position = (start & 15) as usize;
    while position < visible_end {
        let entry = match map_offset {
            Some(offset) => inputs.sprite_pattern[offset + map_row + map_column],
            None => 0,
        };
        let base = usize::from(entry & PATTERN_NUMBER_MASK) * 64;
        let block = u32::from(entry >> PATTERN_BLOCK_SHIFT & 0x000F);
        let fetch_row = if entry & PATTERN_VERTICAL_FLIP != 0 {
            15 - row
        } else {
            row
        };
        for column in 0..16 {
            let source_column = if entry & PATTERN_HORIZONTAL_FLIP != 0 {
                15 - column
            } else {
                column
            };
            let code = u32::from(pattern_pixel_16(
                inputs.sprite_pattern,
                base,
                source_column,
                fetch_row,
            ));
            buffers.codes[position + column] |= code << BACKGROUND_0_SHIFT;
            if block != 0 {
                buffers.blocks[position + column] |= block << BACKGROUND_0_SHIFT;
            }
        }
        position += 16;
        map_column = (map_column + 1) & 63;
    }
}

/// Returns one background layer's tile-map word offset in the pattern RAM.
/// A layer whose chip-select bit targets the absent second chip has no map:
/// every entry reads zero.
fn background_map_offset(control: u16, layer: usize) -> Option<usize> {
    let (chip_select, map_select) = if layer == 0 {
        (
            control & CONTROL_BACKGROUND_0_CHIP_SELECT != 0,
            control & CONTROL_BACKGROUND_0_MAP_SELECT != 0,
        )
    } else {
        (
            control & CONTROL_BACKGROUND_1_CHIP_SELECT != 0,
            control & CONTROL_BACKGROUND_1_MAP_SELECT != 0,
        )
    };
    if chip_select {
        None
    } else if map_select {
        Some(TILE_MAP_1_WORD_OFFSET)
    } else {
        Some(TILE_MAP_0_WORD_OFFSET)
    }
}

/// Reads one pixel of a 16x16 pattern (four 8x8 cells, quadrant order).
fn pattern_pixel_16(pattern: &[u16], base: usize, column: usize, row: usize) -> u16 {
    let half_offset = column / 8 * 32;
    let pixel = column % 8;
    let word = pattern[base + half_offset + row * 2 + pixel / 4];
    word >> (12 - pixel % 4 * 4) & 0x000F
}

/// Reads one pixel of an 8x8 pattern (two words per row, left pixel high).
fn pattern_pixel_8(pattern: &[u16], base: usize, column: usize, row: usize) -> u16 {
    let word = pattern[base + row * 2 + column / 4];
    word >> (12 - column % 4 * 4) & 0x000F
}

/// Folds the plane buffers of one pixel into an 8-bit sprite-screen code.
fn fold_pixel(code_bits: u32, block_bits: u32) -> u8 {
    if code_bits != 0 {
        let shift = (31 - code_bits.leading_zeros()) & !3;
        return ((block_bits >> shift & 15) << 4 | code_bits >> shift & 15) as u8;
    }
    let blocks = block_bits & BACKGROUND_BLOCK_MASK;
    if blocks != 0 {
        let shift = blocks.trailing_zeros() & !3;
        return ((blocks >> shift & 15) << 4) as u8;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::x68k::FixtureX68k;

    fn line_codes(fixture: &FixtureX68k, screen_y: usize) -> Vec<u8> {
        let mut line = Vec::new();
        rasterize_sprite_line(&fixture.inputs(), screen_y, &mut line);
        line
    }

    fn sprite_fixture(width: u32, height: u32) -> FixtureX68k {
        let mut fixture = FixtureX68k::new(width, height);
        fixture.background_control = CONTROL_DISPLAY_ENABLE;
        fixture
    }

    #[test]
    fn sprites_render_at_their_offset_position() {
        let mut fixture = sprite_fixture(32, 32);
        fixture.fill_sprite_pattern(1, 5);
        fixture.set_sprite(0, 20, 18, 0x0201, 3);
        let line = line_codes(&fixture, 2);
        assert_eq!(line[3], 0);
        assert_eq!(line[4], 0x25);
        assert_eq!(line[19], 0x25);
        assert_eq!(line[20], 0);
        assert!(line_codes(&fixture, 1).iter().all(|&code| code == 0));
        assert_eq!(line_codes(&fixture, 17)[4], 0x25);
        assert!(line_codes(&fixture, 18).iter().all(|&code| code == 0));
    }

    #[test]
    fn sprites_clip_at_all_screen_edges() {
        let mut fixture = sprite_fixture(32, 32);
        fixture.fill_sprite_pattern(1, 5);
        fixture.set_sprite(0, 8, 16, 0x0001, 3);
        let line = line_codes(&fixture, 0);
        assert_eq!(&line[..8], &[5; 8]);
        assert_eq!(line[8], 0);
        fixture.set_sprite(0, 40, 16, 0x0001, 3);
        let line = line_codes(&fixture, 0);
        assert_eq!(line[23], 0);
        assert_eq!(&line[24..32], &[5; 8]);
        fixture.set_sprite(0, 16, 8, 0x0001, 3);
        assert_eq!(line_codes(&fixture, 0)[0], 5);
        assert!(line_codes(&fixture, 8).iter().all(|&code| code == 0));
        fixture.set_sprite(0, 16, 40, 0x0001, 3);
        assert!(line_codes(&fixture, 23).iter().all(|&code| code == 0));
        assert_eq!(line_codes(&fixture, 24)[0], 5);
        assert_eq!(line_codes(&fixture, 31)[0], 5);
    }

    #[test]
    fn flips_mirror_the_sprite_pattern() {
        let mut fixture = sprite_fixture(16, 16);
        fixture.sprite_pattern[2 * 64] = 0x5000;
        fixture.set_sprite(0, 16, 16, 0x0002, 3);
        assert_eq!(line_codes(&fixture, 0)[0], 5);
        fixture.set_sprite(0, 16, 16, 0x4002, 3);
        let line = line_codes(&fixture, 0);
        assert_eq!(line[0], 0);
        assert_eq!(line[15], 5);
        fixture.set_sprite(0, 16, 16, 0x8002, 3);
        assert_eq!(line_codes(&fixture, 0)[0], 0);
        assert_eq!(line_codes(&fixture, 15)[0], 5);
        fixture.set_sprite(0, 16, 16, 0xC002, 3);
        assert_eq!(line_codes(&fixture, 15)[15], 5);
    }

    #[test]
    fn palette_blocks_shift_sprites_and_zero_dots_stay_transparent() {
        let mut fixture = sprite_fixture(16, 16);
        fixture.sprite_pattern[64] = 0x0500;
        fixture.set_sprite(0, 16, 16, 0x0701, 3);
        let line = line_codes(&fixture, 0);
        assert_eq!(line[0], 0);
        assert_eq!(line[1], 0x75);
    }

    #[test]
    fn lower_sprite_numbers_win_within_a_plane() {
        let mut fixture = sprite_fixture(16, 16);
        fixture.fill_sprite_pattern(1, 1);
        fixture.fill_sprite_pattern(2, 2);
        fixture.set_sprite(0, 16, 16, 0x0001, 3);
        fixture.set_sprite(1, 16, 16, 0x0002, 3);
        assert_eq!(line_codes(&fixture, 0)[0], 1);
        fixture.set_sprite(1, 16, 16, 0x0002, 1);
        assert_eq!(line_codes(&fixture, 0)[0], 1);
    }

    #[test]
    fn higher_priority_planes_show_in_front_across_sprite_numbers() {
        let mut fixture = sprite_fixture(16, 16);
        fixture.fill_sprite_pattern(1, 1);
        fixture.fill_sprite_pattern(2, 2);
        fixture.set_sprite(0, 16, 16, 0x0001, 1);
        fixture.set_sprite(1, 16, 16, 0x0002, 3);
        assert_eq!(line_codes(&fixture, 0)[0], 2);
    }

    #[test]
    fn sprite_priorities_interleave_with_the_backgrounds() {
        let mut fixture = sprite_fixture(8, 8);
        fixture.background_control = CONTROL_DISPLAY_ENABLE
            | CONTROL_BACKGROUND_0_ENABLE
            | CONTROL_BACKGROUND_1_ENABLE
            | CONTROL_BACKGROUND_1_MAP_SELECT;
        fixture.fill_background_pattern(2, 3);
        fixture.fill_background_pattern(3, 4);
        fixture.set_background_tile(0, 0, 0, 0x0002);
        fixture.set_background_tile(1, 0, 0, 0x0003);
        fixture.fill_sprite_pattern(1, 1);
        fixture.set_sprite(0, 16, 16, 0x0001, 3);
        assert_eq!(line_codes(&fixture, 0)[0], 1);
        fixture.set_sprite(0, 16, 16, 0x0001, 2);
        assert_eq!(line_codes(&fixture, 0)[0], 3);
        fixture.set_background_tile(0, 0, 0, 0x0000);
        assert_eq!(line_codes(&fixture, 0)[0], 1);
        fixture.set_sprite(0, 16, 16, 0x0001, 1);
        assert_eq!(line_codes(&fixture, 0)[0], 4);
        fixture.set_background_tile(1, 0, 0, 0x0000);
        assert_eq!(line_codes(&fixture, 0)[0], 1);
    }

    #[test]
    fn only_the_first_32_sprites_on_a_raster_are_drawn() {
        let mut fixture = sprite_fixture(544, 16);
        fixture.fill_sprite_pattern(1, 5);
        for index in 0..33 {
            fixture.set_sprite(index, 16 + 16 * index as u16, 16, 0x0001, 3);
        }
        let line = line_codes(&fixture, 0);
        assert_eq!(line[31 * 16], 0x05);
        assert_eq!(line[32 * 16], 0);
        fixture.set_sprite(0, 16, 16, 0x0001, 0);
        let line = line_codes(&fixture, 0);
        assert_eq!(line[0], 0);
        assert_eq!(line[32 * 16], 0x05);
    }

    #[test]
    fn off_screen_sprites_consume_raster_slots() {
        let mut fixture = sprite_fixture(64, 16);
        fixture.fill_sprite_pattern(1, 5);
        for index in 0..32 {
            fixture.set_sprite(index, 0, 16, 0x0001, 3);
        }
        fixture.set_sprite(32, 16, 16, 0x0001, 3);
        assert!(line_codes(&fixture, 0).iter().all(|&code| code == 0));
        for index in 0..32 {
            fixture.set_sprite(index, 0, 16, 0x0001, 0);
        }
        assert_eq!(line_codes(&fixture, 0)[0], 0x05);
    }

    #[test]
    fn chip_selected_sprites_are_invisible_but_consume_their_slot() {
        let mut fixture = sprite_fixture(64, 16);
        fixture.fill_sprite_pattern(1, 5);
        fixture.set_sprite(0, 16, 16, 0x0001, 0x0007);
        assert!(line_codes(&fixture, 0).iter().all(|&code| code == 0));
        for index in 1..33 {
            fixture.set_sprite(index, 0, 16, 0x0001, 3);
        }
        fixture.set_sprite(33, 32, 16, 0x0001, 3);
        assert!(line_codes(&fixture, 0).iter().all(|&code| code == 0));
        fixture.set_sprite(0, 16, 16, 0x0001, 0x0003);
        assert_eq!(line_codes(&fixture, 0)[0], 0x05);
    }

    #[test]
    fn porch_registers_offset_the_sprite_coordinates() {
        let mut fixture = sprite_fixture(32, 32);
        fixture.fill_sprite_pattern(1, 5);
        fixture.set_sprite(0, 16, 16, 0x0001, 3);
        fixture.sprite_back_ends = (5, 2);
        let line = line_codes(&fixture, 0);
        assert_eq!(line[7], 0);
        assert_eq!(line[8], 0x05);
        assert_eq!(line_codes(&fixture, 13)[8], 0x05);
        assert!(line_codes(&fixture, 14).iter().all(|&code| code == 0));
    }

    #[test]
    fn background_tiles_render_at_their_map_coordinates() {
        let mut fixture = sprite_fixture(64, 64);
        fixture.background_control = CONTROL_DISPLAY_ENABLE | CONTROL_BACKGROUND_0_ENABLE;
        fixture.fill_background_pattern(7, 9);
        fixture.set_background_tile(0, 2, 1, 0x0007);
        let line = line_codes(&fixture, 8);
        assert_eq!(line[15], 0);
        assert_eq!(line[16], 0x09);
        assert_eq!(line[23], 0x09);
        assert_eq!(line[24], 0);
        assert!(line_codes(&fixture, 7).iter().all(|&code| code == 0));
        assert!(line_codes(&fixture, 16).iter().all(|&code| code == 0));
    }

    #[test]
    fn chip_selected_background_reads_an_all_zero_map() {
        let mut fixture = sprite_fixture(64, 64);
        fixture.background_control =
            CONTROL_DISPLAY_ENABLE | CONTROL_BACKGROUND_0_ENABLE | CONTROL_BACKGROUND_0_CHIP_SELECT;
        fixture.fill_background_pattern(7, 9);
        fixture.set_background_tile(0, 2, 1, 0x0007);
        assert!(line_codes(&fixture, 8).iter().all(|&code| code == 0));
        // The absent map still names pattern 0: its dots tile the layer.
        fixture.fill_background_pattern(0, 3);
        let line = line_codes(&fixture, 8);
        assert_eq!(line[0], 0x03);
        assert_eq!(line[63], 0x03);
    }

    #[test]
    fn chip_selected_background_one_reads_an_all_zero_map() {
        let mut fixture = sprite_fixture(64, 64);
        fixture.background_control =
            CONTROL_DISPLAY_ENABLE | CONTROL_BACKGROUND_1_ENABLE | CONTROL_BACKGROUND_1_CHIP_SELECT;
        fixture.fill_background_pattern(7, 9);
        fixture.set_background_tile(0, 2, 1, 0x0007);
        assert!(line_codes(&fixture, 8).iter().all(|&code| code == 0));
        fixture.background_control = CONTROL_DISPLAY_ENABLE | CONTROL_BACKGROUND_1_ENABLE;
        assert_eq!(line_codes(&fixture, 8)[16], 0x09);
    }

    #[test]
    fn background_scroll_wraps_at_the_virtual_screen_size() {
        let mut fixture = sprite_fixture(32, 32);
        fixture.background_control = CONTROL_DISPLAY_ENABLE | CONTROL_BACKGROUND_0_ENABLE;
        fixture.fill_background_pattern(7, 9);
        fixture.set_background_tile(0, 0, 0, 0x0007);
        fixture.background_scroll = [504, 504, 0, 0];
        let line = line_codes(&fixture, 8);
        assert_eq!(line[7], 0);
        assert_eq!(line[8], 0x09);
        assert_eq!(line[15], 0x09);
        assert!(line_codes(&fixture, 7).iter().all(|&code| code == 0));
    }

    #[test]
    fn background_tiles_apply_flips_and_palette_blocks() {
        let mut fixture = sprite_fixture(16, 16);
        fixture.background_control = CONTROL_DISPLAY_ENABLE | CONTROL_BACKGROUND_0_ENABLE;
        fixture.sprite_pattern[7 * 16] = 0x5000;
        fixture.set_background_tile(0, 0, 0, 0x0307);
        let line = line_codes(&fixture, 0);
        assert_eq!(line[0], 0x35);
        assert_eq!(line[1], 0x30);
        fixture.set_background_tile(0, 0, 0, 0x4307);
        assert_eq!(line_codes(&fixture, 0)[7], 0x35);
        fixture.set_background_tile(0, 0, 0, 0x8307);
        assert_eq!(line_codes(&fixture, 7)[0], 0x35);
        assert_eq!(line_codes(&fixture, 0)[0], 0x30);
    }

    #[test]
    fn block_only_pixels_show_the_deepest_background_block() {
        let mut fixture = sprite_fixture(16, 16);
        fixture.background_control = CONTROL_DISPLAY_ENABLE
            | CONTROL_BACKGROUND_0_ENABLE
            | CONTROL_BACKGROUND_1_ENABLE
            | CONTROL_BACKGROUND_1_MAP_SELECT;
        fixture.set_background_tile(0, 0, 0, 0x0300);
        fixture.set_background_tile(1, 0, 0, 0x0500);
        assert_eq!(line_codes(&fixture, 0)[0], 0x50);
    }

    #[test]
    fn sixteen_pixel_tiles_wrap_at_1024_and_disable_background_1() {
        let mut fixture = sprite_fixture(32, 32);
        fixture.sprite_resolution = 1;
        fixture.background_control = CONTROL_DISPLAY_ENABLE
            | CONTROL_BACKGROUND_0_ENABLE
            | CONTROL_BACKGROUND_1_ENABLE
            | CONTROL_BACKGROUND_1_MAP_SELECT;
        fixture.fill_sprite_pattern(2, 6);
        fixture.fill_sprite_pattern(3, 4);
        fixture.set_background_tile(0, 0, 0, 0x0002);
        fixture.set_background_tile(1, 0, 0, 0x0003);
        fixture.background_scroll = [1016, 1016, 0, 0];
        let line = line_codes(&fixture, 8);
        assert_eq!(line[0], 0);
        assert_eq!(line[7], 0);
        assert_eq!(line[8], 0x06);
        assert_eq!(line[23], 0x06);
        assert!(line_codes(&fixture, 7).iter().all(|&code| code == 0));
    }

    #[test]
    fn both_backgrounds_can_share_one_tile_map() {
        let mut fixture = sprite_fixture(16, 16);
        fixture.background_control =
            CONTROL_DISPLAY_ENABLE | CONTROL_BACKGROUND_0_ENABLE | CONTROL_BACKGROUND_1_ENABLE;
        fixture.fill_background_pattern(3, 9);
        fixture.set_background_tile(0, 0, 0, 0x0203);
        let shared = line_codes(&fixture, 0);
        fixture.background_control = CONTROL_DISPLAY_ENABLE | CONTROL_BACKGROUND_0_ENABLE;
        assert_eq!(shared, line_codes(&fixture, 0));
        assert_eq!(shared[0], 0x29);
    }

    #[test]
    fn display_and_access_gates_blank_the_sprite_screen() {
        let mut fixture = sprite_fixture(16, 16);
        fixture.background_control = CONTROL_DISPLAY_ENABLE | CONTROL_BACKGROUND_0_ENABLE;
        fixture.fill_background_pattern(3, 9);
        fixture.set_background_tile(0, 0, 0, 0x0003);
        fixture.fill_sprite_pattern(1, 5);
        fixture.set_sprite(0, 16, 16, 0x0001, 3);
        assert_eq!(line_codes(&fixture, 0)[0], 5);
        fixture.background_control &= !CONTROL_DISPLAY_ENABLE;
        assert!(line_codes(&fixture, 0).iter().all(|&code| code == 0));
        fixture.background_control |= CONTROL_DISPLAY_ENABLE;
        fixture.sprite_area_accessible = false;
        assert!(line_codes(&fixture, 0).iter().all(|&code| code == 0));
    }
}
