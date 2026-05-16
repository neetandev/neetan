//! AVX2-accelerated cell-row combine inner loops for the PC-98 software renderer.
//!
//! Digital graphics use 16-color palette indices. The hot path expands each
//! 8-pixel cell to byte-lane palette indices via static lookup tables, then
//! resolves RGBA with `vpshufb`. This avoids per-cell `vpgatherdd` from the
//! small 16-entry palette.

use core::arch::x86_64::*;

use super::{
    BLACK, CELL_STRIDE, CELLS_PER_ROW, ComposeScratch, DigitalSimdPalette, FIXED_TEXT_LUT,
    PIXEL_BYTES, PIXELS_PER_CELL, ROW_BYTES,
};

const CHANNEL_COUNT: usize = PIXEL_BYTES;

// One AVX2 register holds one 8-pixel RGBA cell: 8 pixels * 4 bytes = 32
// lanes. The plane tables expand a source byte into palette-index
// contributions in every channel lane so the four planes can be ORed together.
const PLANE_INDEX_B: [[u8; CELL_STRIDE]; 256] = build_plane_index_table(1);
const PLANE_INDEX_R: [[u8; CELL_STRIDE]; 256] = build_plane_index_table(2);
const PLANE_INDEX_G: [[u8; CELL_STRIDE]; 256] = build_plane_index_table(4);
const PLANE_INDEX_E: [[u8; CELL_STRIDE]; 256] = build_plane_index_table(8);
// Byte mask for `vblendv`: 0xFF selects the text pixel, 0 selects graphics.
const TEXT_BLEND_MASK: [[u8; CELL_STRIDE]; 256] = build_text_blend_mask_table();

// `vpshufb` has no concept of RGBA structs. Each control vector keeps only one
// channel live and zeros the other three channels by setting bit 7.
const CHANNEL_CONTROL_R: [u8; CELL_STRIDE] = build_channel_control_mask(0);
const CHANNEL_CONTROL_G: [u8; CELL_STRIDE] = build_channel_control_mask(1);
const CHANNEL_CONTROL_B: [u8; CELL_STRIDE] = build_channel_control_mask(2);
const CHANNEL_CONTROL_A: [u8; CELL_STRIDE] = build_channel_control_mask(3);

const fn build_plane_index_table(contribution: u8) -> [[u8; CELL_STRIDE]; 256] {
    let mut table = [[0u8; CELL_STRIDE]; 256];
    let mut byte = 0usize;
    while byte < 256 {
        let mut pixel = 0usize;
        while pixel < PIXELS_PER_CELL {
            let mask = 1u8 << (7 - pixel);
            let value = if ((byte as u8) & mask) != 0 {
                contribution
            } else {
                0
            };
            let mut channel = 0usize;
            while channel < CHANNEL_COUNT {
                table[byte][pixel * CHANNEL_COUNT + channel] = value;
                channel += 1;
            }
            pixel += 1;
        }
        byte += 1;
    }
    table
}

const fn build_text_blend_mask_table() -> [[u8; CELL_STRIDE]; 256] {
    let mut table = [[0u8; CELL_STRIDE]; 256];
    let mut byte = 0usize;
    while byte < 256 {
        let mut pixel = 0usize;
        while pixel < PIXELS_PER_CELL {
            let mask = 1u8 << (7 - pixel);
            let value = if ((byte as u8) & mask) != 0 { 0xFF } else { 0 };
            let mut channel = 0usize;
            while channel < CHANNEL_COUNT {
                table[byte][pixel * CHANNEL_COUNT + channel] = value;
                channel += 1;
            }
            pixel += 1;
        }
        byte += 1;
    }
    table
}

const fn build_channel_control_mask(channel_index: usize) -> [u8; CELL_STRIDE] {
    let mut table = [0u8; CELL_STRIDE];
    let mut index = 0usize;
    while index < CELL_STRIDE {
        if index % CHANNEL_COUNT != channel_index {
            table[index] = 0x80;
        }
        index += 1;
    }
    table
}

macro_rules! load_bytes_256 {
    ($bytes:expr) => {
        _mm256_loadu_si256(($bytes).as_ptr() as *const __m256i)
    };
}

macro_rules! load_index_bytes {
    ($scratch:expr, $cell:expr) => {{
        let blue =
            load_bytes_256!(PLANE_INDEX_B.get_unchecked($scratch.graphics_b[$cell] as usize));
        let red = load_bytes_256!(PLANE_INDEX_R.get_unchecked($scratch.graphics_r[$cell] as usize));
        let green =
            load_bytes_256!(PLANE_INDEX_G.get_unchecked($scratch.graphics_g[$cell] as usize));
        let extended =
            load_bytes_256!(PLANE_INDEX_E.get_unchecked($scratch.graphics_e[$cell] as usize));
        _mm256_or_si256(_mm256_or_si256(blue, red), _mm256_or_si256(green, extended))
    }};
}

macro_rules! load_text_mask {
    ($scratch:expr, $cell:expr) => {
        load_bytes_256!(TEXT_BLEND_MASK.get_unchecked($scratch.text_byte[$cell] as usize))
    };
}

macro_rules! shuffle_palette {
    (
        $indices:expr,
        $red_table:expr,
        $green_table:expr,
        $blue_table:expr,
        $alpha_table:expr,
        $red_control:expr,
        $green_control:expr,
        $blue_control:expr,
        $alpha_control:expr
    ) => {{
        let red_control = _mm256_or_si256($indices, $red_control);
        let green_control = _mm256_or_si256($indices, $green_control);
        let blue_control = _mm256_or_si256($indices, $blue_control);
        let alpha_control = _mm256_or_si256($indices, $alpha_control);
        let red = _mm256_shuffle_epi8($red_table, red_control);
        let green = _mm256_shuffle_epi8($green_table, green_control);
        let blue = _mm256_shuffle_epi8($blue_table, blue_control);
        let alpha = _mm256_shuffle_epi8($alpha_table, alpha_control);
        _mm256_or_si256(_mm256_or_si256(red, green), _mm256_or_si256(blue, alpha))
    }};
}

macro_rules! text_pixel {
    ($palette:expr, $scratch:expr, $cell:expr) => {{
        let text_color = ($scratch.text_color[$cell] & 0x07) as usize;
        _mm256_set1_epi32(*$palette.text_pixels.get_unchecked(text_color) as i32)
    }};
}

macro_rules! store_cell {
    ($row_ptr:expr, $cell:expr, $value:expr) => {
        _mm256_storeu_si256($row_ptr.add($cell * CELL_STRIDE) as *mut __m256i, $value)
    };
}

#[derive(Clone, Copy)]
struct ChannelVectors {
    red: __m256i,
    green: __m256i,
    blue: __m256i,
    alpha: __m256i,
}

#[target_feature(enable = "avx2")]
pub(super) unsafe fn combine_cells_digital_color_avx2(
    row_buf: &mut [u8],
    scratch: &ComposeScratch,
    palette: &DigitalSimdPalette,
) {
    debug_assert_eq!(row_buf.len(), ROW_BYTES);

    unsafe {
        let graphics = ChannelVectors {
            red: load_bytes_256!(&palette.graphics_red),
            green: load_bytes_256!(&palette.graphics_green),
            blue: load_bytes_256!(&palette.graphics_blue),
            alpha: load_bytes_256!(&palette.graphics_alpha),
        };
        let controls = ChannelVectors {
            red: load_bytes_256!(&CHANNEL_CONTROL_R),
            green: load_bytes_256!(&CHANNEL_CONTROL_G),
            blue: load_bytes_256!(&CHANNEL_CONTROL_B),
            alpha: load_bytes_256!(&CHANNEL_CONTROL_A),
        };
        let row_ptr = row_buf.as_mut_ptr();

        let mut cell = 0usize;
        while cell < CELLS_PER_ROW {
            // Two cells per loop hides some call overhead while keeping each
            // helper focused on exactly one 32-byte output register.
            let first = compose_digital_color_cell(scratch, palette, cell, graphics, controls);
            store_cell!(row_ptr, cell, first);

            let second_cell = cell + 1;
            let second =
                compose_digital_color_cell(scratch, palette, second_cell, graphics, controls);
            store_cell!(row_ptr, second_cell, second);

            cell += 2;
        }
    }
}

#[target_feature(enable = "avx2")]
pub(super) unsafe fn combine_cells_digital_mono_avx2(
    row_buf: &mut [u8],
    scratch: &ComposeScratch,
    palette: &DigitalSimdPalette,
) {
    debug_assert_eq!(row_buf.len(), ROW_BYTES);

    // With text disabled the monochrome path is pure graphics and can use
    // channel shuffle tables. With text enabled, graphics-on pixels borrow
    // the current cell text color, so it is cheaper to build a blend mask.
    unsafe {
        // SAFETY: callers already validated that AVX2 is available before
        // entering this AVX2-dispatched function.
        if palette.text_enabled {
            combine_cells_digital_mono_text_avx2(row_buf, scratch, palette);
        } else {
            combine_cells_digital_mono_graphics_avx2(row_buf, scratch, palette);
        }
    }
}

#[target_feature(enable = "avx2")]
unsafe fn combine_cells_digital_mono_text_avx2(
    row_buf: &mut [u8],
    scratch: &ComposeScratch,
    palette: &DigitalSimdPalette,
) {
    unsafe {
        let mono_mask_table = load_bytes_256!(&palette.mono_mask);
        let palette_zero = _mm256_set1_epi32(palette.palette_zero as i32);
        let row_ptr = row_buf.as_mut_ptr();

        let mut cell = 0usize;
        while cell < CELLS_PER_ROW {
            let first = compose_digital_mono_text_cell(
                scratch,
                palette,
                cell,
                mono_mask_table,
                palette_zero,
            );
            store_cell!(row_ptr, cell, first);

            let second_cell = cell + 1;
            let second = compose_digital_mono_text_cell(
                scratch,
                palette,
                second_cell,
                mono_mask_table,
                palette_zero,
            );
            store_cell!(row_ptr, second_cell, second);

            cell += 2;
        }
    }
}

#[target_feature(enable = "avx2")]
unsafe fn combine_cells_digital_mono_graphics_avx2(
    row_buf: &mut [u8],
    scratch: &ComposeScratch,
    palette: &DigitalSimdPalette,
) {
    unsafe {
        let mono = ChannelVectors {
            red: load_bytes_256!(&palette.mono_red),
            green: load_bytes_256!(&palette.mono_green),
            blue: load_bytes_256!(&palette.mono_blue),
            alpha: load_bytes_256!(&palette.mono_alpha),
        };
        let controls = ChannelVectors {
            red: load_bytes_256!(&CHANNEL_CONTROL_R),
            green: load_bytes_256!(&CHANNEL_CONTROL_G),
            blue: load_bytes_256!(&CHANNEL_CONTROL_B),
            alpha: load_bytes_256!(&CHANNEL_CONTROL_A),
        };
        let row_ptr = row_buf.as_mut_ptr();

        let mut cell = 0usize;
        while cell < CELLS_PER_ROW {
            let first = compose_digital_mono_graphics_cell(scratch, cell, mono, controls);
            store_cell!(row_ptr, cell, first);

            let second_cell = cell + 1;
            let second = compose_digital_mono_graphics_cell(scratch, second_cell, mono, controls);
            store_cell!(row_ptr, second_cell, second);

            cell += 2;
        }
    }
}

#[target_feature(enable = "avx2")]
unsafe fn compose_digital_color_cell(
    scratch: &ComposeScratch,
    palette: &DigitalSimdPalette,
    cell: usize,
    graphics: ChannelVectors,
    controls: ChannelVectors,
) -> __m256i {
    unsafe {
        let index_bytes = load_index_bytes!(scratch, cell);
        let graphics_pixel = shuffle_palette!(
            index_bytes,
            graphics.red,
            graphics.green,
            graphics.blue,
            graphics.alpha,
            controls.red,
            controls.green,
            controls.blue,
            controls.alpha
        );
        let text_mask = load_text_mask!(scratch, cell);
        let text_pixel = text_pixel!(palette, scratch, cell);
        // Text has priority over graphics in color digital modes.
        _mm256_blendv_epi8(graphics_pixel, text_pixel, text_mask)
    }
}

#[target_feature(enable = "avx2")]
unsafe fn compose_digital_mono_text_cell(
    scratch: &ComposeScratch,
    palette: &DigitalSimdPalette,
    cell: usize,
    mono_mask_table: __m256i,
    palette_zero: __m256i,
) -> __m256i {
    unsafe {
        let index_bytes = load_index_bytes!(scratch, cell);
        let mono_mask = _mm256_shuffle_epi8(mono_mask_table, index_bytes);
        let text_mask = load_text_mask!(scratch, cell);
        // Monochrome mixed mode colors both text pixels and graphics-on pixels
        // with the cell text color. Everything else falls back to palette[0].
        let combined_mask = _mm256_or_si256(mono_mask, text_mask);
        let text_pixel = text_pixel!(palette, scratch, cell);
        _mm256_blendv_epi8(palette_zero, text_pixel, combined_mask)
    }
}

#[target_feature(enable = "avx2")]
unsafe fn compose_digital_mono_graphics_cell(
    scratch: &ComposeScratch,
    cell: usize,
    mono: ChannelVectors,
    controls: ChannelVectors,
) -> __m256i {
    unsafe {
        let index_bytes = load_index_bytes!(scratch, cell);
        // Graphics-only monochrome mode maps the selected indices to fixed
        // white and unselected indices to palette[0].
        shuffle_palette!(
            index_bytes,
            mono.red,
            mono.green,
            mono.blue,
            mono.alpha,
            controls.red,
            controls.green,
            controls.blue,
            controls.alpha
        )
    }
}

#[target_feature(enable = "avx2")]
pub(super) unsafe fn combine_cells_pegc_avx2(
    row_buf: &mut [u8],
    scratch: &ComposeScratch,
    palette_256: &[u32; 256],
    graphics_enabled: bool,
) {
    debug_assert_eq!(row_buf.len(), ROW_BYTES);

    unsafe {
        let black_vec = _mm256_set1_epi32(BLACK as i32);
        let row_ptr = row_buf.as_mut_ptr();

        let mut cell = 0usize;
        while cell < CELLS_PER_ROW {
            let first = compose_pegc_cell(scratch, palette_256, cell, black_vec, graphics_enabled);
            store_cell!(row_ptr, cell, first);

            let second_cell = cell + 1;
            let second = compose_pegc_cell(
                scratch,
                palette_256,
                second_cell,
                black_vec,
                graphics_enabled,
            );
            store_cell!(row_ptr, second_cell, second);

            cell += 2;
        }
    }
}

#[target_feature(enable = "avx2")]
unsafe fn compose_pegc_cell(
    scratch: &ComposeScratch,
    palette_256: &[u32; 256],
    cell: usize,
    black_vec: __m256i,
    graphics_enabled: bool,
) -> __m256i {
    unsafe {
        let graphics_pixel = if graphics_enabled {
            // PEGC uses 8-bit palette indices, so the 8 pixels in a cell are
            // widened to u32 indices and gathered from the 256-entry palette.
            let indices = _mm_loadl_epi64(
                scratch.pegc_indices.as_ptr().add(cell * PIXELS_PER_CELL) as *const __m128i,
            );
            let widened = _mm256_cvtepu8_epi32(indices);
            _mm256_i32gather_epi32(palette_256.as_ptr() as *const i32, widened, 4)
        } else {
            black_vec
        };

        let text_mask = load_text_mask!(scratch, cell);
        let text_color = (scratch.text_color[cell] & 0x07) as usize;
        let text_pixel = _mm256_set1_epi32(FIXED_TEXT_LUT[text_color] as i32);
        // Text overlay keeps using fixed 8-color text values on top of PEGC.
        _mm256_blendv_epi8(graphics_pixel, text_pixel, text_mask)
    }
}
