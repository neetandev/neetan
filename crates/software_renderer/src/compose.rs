//! Per-frame compose loop for the PC-98 software renderer.
//!
//! The compose pass walks the output framebuffer scanline-by-scanline. Per
//! scanline we resolve scroll regions and font geometry once, then decode the
//! 80 text cell columns and copy the four graphics plane rows into small
//! scratch arrays. The inner loop only walks 80 cells x 8 pixels using
//! indexed reads against the scratch, which is the shape the auto-vectorizer
//! might turn into SIMD.
//!
//! On x86_64 with AVX2 and aarch64 we additionally dispatch the per-scanline cell combine
//! into a hand-written SIMD path; pre-passes stay scalar.
//!
//! The digital path renders the standard B/R/G/E planar graphics modes. The
//! PEGC path renders the 256-color byte-per-pixel framebuffer. Both paths use
//! the same text decode so cursor, underline, reverse, and 40-column behavior
//! remain identical across graphics modes.

#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
mod avx2;

#[cfg(target_arch = "aarch64")]
#[allow(unsafe_code)]
mod neon;

use super::{
    GdcGraphicsInput, GraphicsInput, PegcRenderInputs, RenderInputs, SoftwareRenderer,
    TEXT_CELL_COUNT,
};

const BLACK: u32 = 0xFF00_0000;
const BLACK_PALETTE: [u32; 16] = [BLACK; 16];

const VIDEO_MODE_MONOCHROME: u32 = 0x02;
const VIDEO_MODE_WIDTH_40: u32 = 0x04;
const VIDEO_MODE_FONTSEL_8X16: u32 = 0x08;

const DIGITAL_GRAPHICS_PALETTE_BASE: u32 = 8;
const MONOCHROME_GRAPHICS_COLOR: u32 = 7;

const TEXT_CELL_FONT_OFFSET_MASK: u32 = 0x000F_FFFF;
const TEXT_CELL_COLOR_SHIFT: u32 = 20;
const TEXT_CELL_FLAG_BLANK: u32 = 1 << 23;
const TEXT_CELL_FLAG_REVERSE: u32 = 1 << 24;
const TEXT_CELL_FLAG_UNDERLINE_THIS: u32 = 1 << 25;
const TEXT_CELL_FLAG_UNDERLINE_LEFT: u32 = 1 << 26;
const TEXT_CELL_FLAG_VERTICAL_LINE: u32 = 1 << 27;
const TEXT_CELL_FLAG_CURSOR: u32 = 1 << 28;

const TEXT_CELL_INDEX_MASK: u32 = 0xFFF;

const PIXELS_PER_CELL: usize = 8;
const CELLS_PER_ROW: usize = SoftwareRenderer::WIDTH / PIXELS_PER_CELL;
const PIXEL_BYTES: usize = SoftwareRenderer::PIXEL_BYTES;
const ROW_BYTES: usize = SoftwareRenderer::WIDTH * PIXEL_BYTES;

const GRAPHICS_PLANE_SIZE: usize = 0x8000;
const GRAPHICS_PLANE_MASK: u32 = (GRAPHICS_PLANE_SIZE as u32) - 1;

/// Per-row scratch shared between scanlines. Lives on the renderer so the
/// hot loop allocates nothing per frame.
pub(super) struct ComposeScratch {
    /// 8-bit "is text on" mask for each cell column. After `decode_text_row`
    /// the byte already encodes blank / reverse / cursor / underline /
    /// vertical-line, so the inner loop is a plain bit test.
    pub text_byte: [u8; CELLS_PER_ROW],
    /// 3-bit text foreground palette index for each cell column.
    pub text_color: [u8; CELLS_PER_ROW],
    /// One graphics byte per 8-pixel cell for each standard digital plane.
    /// Plane bits are combined as B,R,G,E to form the hardware color index.
    pub graphics_b: [u8; CELLS_PER_ROW],
    pub graphics_r: [u8; CELLS_PER_ROW],
    pub graphics_g: [u8; CELLS_PER_ROW],
    pub graphics_e: [u8; CELLS_PER_ROW],
    /// PEGC is byte-per-pixel, so the row scratch stores palette indices for
    /// the full 640-pixel scanline instead of one byte per text cell.
    pub pegc_indices: [u8; SoftwareRenderer::WIDTH],
}

impl ComposeScratch {
    pub(super) fn new() -> Box<Self> {
        Box::new(Self {
            text_byte: [0; CELLS_PER_ROW],
            text_color: [0; CELLS_PER_ROW],
            graphics_b: [0; CELLS_PER_ROW],
            graphics_r: [0; CELLS_PER_ROW],
            graphics_g: [0; CELLS_PER_ROW],
            graphics_e: [0; CELLS_PER_ROW],
            pegc_indices: [0; SoftwareRenderer::WIDTH],
        })
    }
}

/// Width-40 bit-doubling: high half = bit-doubled high nibble of input,
/// low half = bit-doubled low nibble.
const BIT_DOUBLE_HIGH: [u8; 256] = build_bit_double(true);
const BIT_DOUBLE_LOW: [u8; 256] = build_bit_double(false);

const fn build_bit_double(high_nibble: bool) -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut byte = 0usize;
    while byte < 256 {
        let nibble = if high_nibble {
            (byte >> 4) & 0xF
        } else {
            byte & 0xF
        };
        let mut out = 0u8;
        if (nibble & 0b1000) != 0 {
            out |= 0b1100_0000;
        }
        if (nibble & 0b0100) != 0 {
            out |= 0b0011_0000;
        }
        if (nibble & 0b0010) != 0 {
            out |= 0b0000_1100;
        }
        if (nibble & 0b0001) != 0 {
            out |= 0b0000_0011;
        }
        table[byte] = out;
        byte += 1;
    }
    table
}

const FIXED_TEXT_LUT: [u32; 8] = build_fixed_text_lut();

/// Hardware fixed text colors use BRG bit order. These colors bypass the
/// analog palette in 16-color and PEGC modes.
const fn build_fixed_text_lut() -> [u32; 8] {
    let mut table = [0u32; 8];
    let mut index = 0u32;
    while index < 8 {
        let blue: u32 = if (index & 1) != 0 { 0xFF } else { 0 };
        let red: u32 = if ((index >> 1) & 1) != 0 { 0xFF } else { 0 };
        let green: u32 = if ((index >> 2) & 1) != 0 { 0xFF } else { 0 };
        table[index as usize] = red | (green << 8) | (blue << 16) | 0xFF00_0000;
        index += 1;
    }
    table
}

/// Stride of one composed cell in bytes (8 pixels x 4 channels).
const CELL_STRIDE: usize = PIXELS_PER_CELL * PIXEL_BYTES;

/// SIMD palette used by both the AVX2 and NEON compose paths.
///
/// The four-channel shuffle tables hold one byte per palette entry per
/// channel. The first 16 bytes carry palette entries 0..16; the second
/// 16 bytes mirror the first half so AVX2's lane-local `vpshufb` can
/// resolve both halves of a cell against the same 16-entry palette. NEON
/// reads only the first 16 bytes from each table.
pub(super) struct DigitalSimdPalette {
    /// Shuffle tables for color digital and 16-color graphics paths.
    pub graphics_red: [u8; CELL_STRIDE],
    pub graphics_green: [u8; CELL_STRIDE],
    pub graphics_blue: [u8; CELL_STRIDE],
    pub graphics_alpha: [u8; CELL_STRIDE],
    /// Shuffle tables for monochrome graphics-only output.
    pub mono_red: [u8; CELL_STRIDE],
    pub mono_green: [u8; CELL_STRIDE],
    pub mono_blue: [u8; CELL_STRIDE],
    pub mono_alpha: [u8; CELL_STRIDE],
    /// Per-palette-index on/off mask for monochrome mixed text/graphics mode.
    pub mono_mask: [u8; CELL_STRIDE],
    /// Text colors are broadcast as full pixels, not shuffled from the graphics
    /// palette, because analog and PEGC text use fixed BRG colors.
    pub text_pixels: [u32; 8],
    pub palette_zero: u32,
    pub text_enabled: bool,
}

impl DigitalSimdPalette {
    pub(super) fn new(
        graphics_palette_rgba: &[u32; 16],
        text_palette_rgba: &[u32; 16],
        mono_lookup: &[u8; 16],
        mode: CombineMode,
    ) -> Self {
        let mut palette = Self {
            graphics_red: [0; CELL_STRIDE],
            graphics_green: [0; CELL_STRIDE],
            graphics_blue: [0; CELL_STRIDE],
            graphics_alpha: [0; CELL_STRIDE],
            mono_red: [0; CELL_STRIDE],
            mono_green: [0; CELL_STRIDE],
            mono_blue: [0; CELL_STRIDE],
            mono_alpha: [0; CELL_STRIDE],
            mono_mask: [0; CELL_STRIDE],
            text_pixels: [0; 8],
            palette_zero: graphics_palette_rgba[0],
            text_enabled: mode.text_enabled,
        };

        for (index, mono_lookup_entry) in mono_lookup.iter().enumerate() {
            // In 8-color digital graphics mode, graphics colors live at
            // palette indices 8-15. In 16-color analog mode they use 0-15.
            let graphics_index = if mode.graphics_enabled && !mode.is_16_color {
                DIGITAL_GRAPHICS_PALETTE_BASE as usize + (index & 0x07)
            } else {
                index
            };
            let graphics_pixel = graphics_palette_rgba[graphics_index];
            write_shuffle_pixel(
                &mut palette.graphics_red,
                &mut palette.graphics_green,
                &mut palette.graphics_blue,
                &mut palette.graphics_alpha,
                index,
                graphics_pixel,
            );

            let mono_pixel = if *mono_lookup_entry != 0 {
                FIXED_TEXT_LUT[MONOCHROME_GRAPHICS_COLOR as usize]
            } else {
                graphics_palette_rgba[0]
            };
            write_shuffle_pixel(
                &mut palette.mono_red,
                &mut palette.mono_green,
                &mut palette.mono_blue,
                &mut palette.mono_alpha,
                index,
                mono_pixel,
            );
            let mono_mask = if *mono_lookup_entry != 0 { 0xFF } else { 0 };
            // Duplicate into both 128-bit lanes. `vpshufb` control bytes are
            // lane-local, so each half needs its own copy of entries 0-15.
            palette.mono_mask[index] = mono_mask;
            palette.mono_mask[index + 16] = mono_mask;
        }

        for index in 0..8 {
            palette.text_pixels[index] = if mode.is_16_color {
                FIXED_TEXT_LUT[index]
            } else {
                text_palette_rgba[index]
            };
        }

        palette
    }
}

fn write_shuffle_pixel(
    red_table: &mut [u8; CELL_STRIDE],
    green_table: &mut [u8; CELL_STRIDE],
    blue_table: &mut [u8; CELL_STRIDE],
    alpha_table: &mut [u8; CELL_STRIDE],
    index: usize,
    pixel: u32,
) {
    let red = pixel as u8;
    let green = (pixel >> 8) as u8;
    let blue = (pixel >> 16) as u8;
    let alpha = (pixel >> 24) as u8;

    red_table[index] = red;
    green_table[index] = green;
    blue_table[index] = blue;
    alpha_table[index] = alpha;

    // `vpshufb` operates independently on the low and high 128-bit lanes of
    // the AVX2 register, so the 16-entry palette has to appear in both lanes.
    // NEON ignores bytes 16..32; this duplication is harmless there.
    red_table[index + 16] = red;
    green_table[index + 16] = green;
    blue_table[index + 16] = blue;
    alpha_table[index + 16] = alpha;
}

pub(super) fn compose(
    font_rom: &[u8],
    text_cells: &[u32; TEXT_CELL_COUNT],
    inputs: &RenderInputs<'_>,
    framebuffer: &mut [u8],
    scratch: &mut ComposeScratch,
    has_simd: bool,
) {
    assert!(framebuffer.len() >= SoftwareRenderer::FRAMEBUFFER_BYTES);
    let framebuffer = &mut framebuffer[..SoftwareRenderer::FRAMEBUFFER_BYTES];

    let max_y = compute_max_y(inputs);

    if !inputs.global_enabled {
        fill_black(framebuffer);
        return;
    }

    match &inputs.graphics {
        GraphicsInput::Gdc(gdc) => compose_digital(
            font_rom,
            text_cells,
            inputs,
            gdc,
            max_y,
            framebuffer,
            scratch,
            has_simd,
        ),
        GraphicsInput::Pegc(pegc) => compose_pegc(
            font_rom,
            text_cells,
            inputs,
            pegc,
            max_y,
            framebuffer,
            scratch,
            has_simd,
        ),
    }

    let inactive_start = (max_y as usize) * ROW_BYTES;
    if inactive_start < framebuffer.len() {
        // Native PC-98 output is 400 active lines except PEGC 480-line modes.
        // The renderer always owns a 640x480 target, so clear the border below
        // the active area explicitly.
        fill_black(&mut framebuffer[inactive_start..]);
    }
}

#[allow(clippy::too_many_arguments)]
fn compose_digital(
    font_rom: &[u8],
    text_cells: &[u32; TEXT_CELL_COUNT],
    inputs: &RenderInputs<'_>,
    gdc: &GdcGraphicsInput<'_>,
    max_y: u32,
    framebuffer: &mut [u8],
    scratch: &mut ComposeScratch,
    has_simd: bool,
) {
    let video_mode = inputs.video_mode;
    let width40_mode = (video_mode & VIDEO_MODE_WIDTH_40) != 0;
    let fontsel_8x16 = (video_mode & VIDEO_MODE_FONTSEL_8X16) != 0;
    let is_monochrome = (video_mode & VIDEO_MODE_MONOCHROME) != 0;
    let is_16_color = gdc.is_16_color;
    let text_enabled = inputs.text_enabled;
    let graphics_enabled = inputs.graphics_enabled;

    // CRTC values are packed as low/high 16-bit fields by the bus snapshot,
    // but the hardware registers themselves are 5-bit quantities.
    let crtc_pl_bl = inputs.crtc_pl_bl;
    let pl = crtc_pl_bl & 0x1F;
    let bl = (crtc_pl_bl >> 16) & 0x1F;
    let crtc_cl_ssl = inputs.crtc_cl_ssl;
    let cl = (crtc_cl_ssl & 0x1F).min(16) as i32;
    let ssl = (crtc_cl_ssl >> 16) & 0x1F;
    let crtc_sur_sdr = inputs.crtc_sur_sdr;
    let sur = crtc_sur_sdr & 0x1F;

    // PL/BL describe which glyph scanlines are visible in a character row.
    // `topline` may be negative when PL encodes a wrapped row shape.
    let (topline, lines) = compute_text_line_shape(pl, bl);
    let scanlines_per_row = (lines - topline).max(1) as u32;

    let cursor_top = inputs.cursor_top as i32;
    let cursor_bottom = inputs.cursor_bottom as i32;

    let lines_per_row = gdc.lines_per_row.max(1);
    let zoom = gdc.zoom_display + 1;
    let graphics_y_divisor = lines_per_row * zoom;
    let graphics_pitch = inputs.gdc_graphics_pitch;
    let text_pitch = inputs.gdc_text_pitch;

    let mono_lookup = compute_mono_lookup(gdc.monochrome_mask);
    // Monochrome graphics reduce a 4-bit graphics color to an on/off mask.
    // The final color still depends on whether text is enabled for the scanline.
    let mode = CombineMode {
        text_enabled,
        graphics_enabled,
        is_monochrome,
        is_16_color,
    };

    // Skipline is gated on the GDC line-repeat factor only.
    let skipline_enabled = gdc.lines_per_row > 1;

    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    let digital_simd_palette = if has_simd {
        Some(DigitalSimdPalette::new(
            &inputs.palette_rgba,
            &inputs.palette_rgba,
            &mono_lookup,
            mode,
        ))
    } else {
        None
    };
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    let digital_simd_palette_dim = if has_simd && skipline_enabled {
        Some(DigitalSimdPalette::new(
            &BLACK_PALETTE,
            &inputs.palette_rgba,
            &mono_lookup,
            mode,
        ))
    } else {
        None
    };

    let mut text_scroll_iter = ScrollLineIter::new(inputs.gdc_scroll_start_line);
    let mut graphics_byte_iter = GraphicsByteBaseIter::new(
        inputs.gdc_graphics_scroll,
        graphics_pitch,
        graphics_y_divisor,
    );

    for y in 0..max_y {
        // Resolve the scroll partition and glyph geometry once per scanline,
        // then keep the hot combine loop to dense scratch reads.
        let (text_scroll_start, text_scanline) = text_scroll_iter.next();
        let (row, glyph_y) =
            compute_text_row_and_glyph_y(text_scanline, scanlines_per_row, ssl, sur);
        let font_line = topline + glyph_y as i32;
        let font_rom_line = (font_line as u32) & 0x0F;
        let font_visible = font_line >= 0 && font_line < cl;
        let at_underline_line = font_line + 1 == lines;
        let cursor_active_for_scanline = font_line >= cursor_top && font_line <= cursor_bottom;
        let text_cell_base = text_scroll_start.wrapping_add(row.wrapping_mul(text_pitch));

        let (graphics_byte_base, is_first_raster) = graphics_byte_iter.next();
        let is_skipline = skipline_enabled && !is_first_raster;

        if text_enabled {
            decode_text_row(
                font_rom,
                text_cells,
                TextRowState {
                    cell_base: text_cell_base,
                    font_rom_line,
                    font_visible,
                    fontsel_8x16,
                    at_underline_line,
                    cursor_active_for_scanline,
                    width40_mode,
                },
                scratch,
            );
        } else {
            scratch.text_byte.fill(0);
            scratch.text_color.fill(0);
        }

        if graphics_enabled {
            load_graphics_row(gdc.b_plane, graphics_byte_base, &mut scratch.graphics_b);
            load_graphics_row(gdc.r_plane, graphics_byte_base, &mut scratch.graphics_r);
            load_graphics_row(gdc.g_plane, graphics_byte_base, &mut scratch.graphics_g);
            if is_16_color {
                load_graphics_row(gdc.e_plane, graphics_byte_base, &mut scratch.graphics_e);
            } else {
                scratch.graphics_e.fill(0);
            }
        } else {
            scratch.graphics_b.fill(0);
            scratch.graphics_r.fill(0);
            scratch.graphics_g.fill(0);
            scratch.graphics_e.fill(0);
        }

        let row_offset = (y as usize) * ROW_BYTES;
        let row_buf = &mut framebuffer[row_offset..row_offset + ROW_BYTES];

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        let _ = has_simd;

        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        if let Some(palette) = digital_simd_palette.as_ref() {
            let active_palette = if is_skipline {
                digital_simd_palette_dim.as_ref().unwrap_or(palette)
            } else {
                palette
            };
            #[allow(unsafe_code)]
            // SAFETY: `has_simd` was validated at renderer construction:
            // `is_x86_feature_detected!("avx2")` on x86_64, unconditional on
            // aarch64 where NEON is in the baseline ISA.
            unsafe {
                #[cfg(target_arch = "x86_64")]
                if mode.is_monochrome && mode.graphics_enabled {
                    avx2::combine_cells_digital_mono_avx2(row_buf, scratch, active_palette);
                } else {
                    avx2::combine_cells_digital_color_avx2(row_buf, scratch, active_palette);
                }
                #[cfg(target_arch = "aarch64")]
                if mode.is_monochrome && mode.graphics_enabled {
                    neon::combine_cells_digital_mono_neon(row_buf, scratch, active_palette);
                } else {
                    neon::combine_cells_digital_color_neon(row_buf, scratch, active_palette);
                }
            }
            continue;
        }

        let active_graphics_palette = if is_skipline {
            &BLACK_PALETTE
        } else {
            &inputs.palette_rgba
        };
        combine_cells_digital(
            row_buf,
            scratch,
            active_graphics_palette,
            &inputs.palette_rgba,
            &mono_lookup,
            mode,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn compose_pegc(
    font_rom: &[u8],
    text_cells: &[u32; TEXT_CELL_COUNT],
    inputs: &RenderInputs<'_>,
    pegc: &PegcRenderInputs<'_>,
    max_y: u32,
    framebuffer: &mut [u8],
    scratch: &mut ComposeScratch,
    has_simd: bool,
) {
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let _ = has_simd;
    let video_mode = inputs.video_mode;
    let width40_mode = (video_mode & VIDEO_MODE_WIDTH_40) != 0;
    let fontsel_8x16 = (video_mode & VIDEO_MODE_FONTSEL_8X16) != 0;
    let text_enabled = inputs.text_enabled;
    let graphics_enabled = inputs.graphics_enabled;

    let crtc_pl_bl = inputs.crtc_pl_bl;
    let pl = crtc_pl_bl & 0x1F;
    let bl = (crtc_pl_bl >> 16) & 0x1F;
    let crtc_cl_ssl = inputs.crtc_cl_ssl;
    let cl = (crtc_cl_ssl & 0x1F).min(16) as i32;
    let ssl = (crtc_cl_ssl >> 16) & 0x1F;
    let crtc_sur_sdr = inputs.crtc_sur_sdr;
    let sur = crtc_sur_sdr & 0x1F;

    let (topline, lines) = compute_text_line_shape(pl, bl);
    let scanlines_per_row = (lines - topline).max(1) as u32;

    let cursor_top = inputs.cursor_top as i32;
    let cursor_bottom = inputs.cursor_bottom as i32;
    let text_pitch = inputs.gdc_text_pitch;
    let graphics_pitch = inputs.gdc_graphics_pitch;

    let pegc_flags = pegc.pegc_flags;
    let is_one_screen = (pegc_flags & 0x02) != 0;
    let display_page = (pegc_flags >> 2) & 1;
    // PEGC has either one 512 KB screen or two 256 KB pages. Address masking
    // is kept separate from `page_base` so short test VRAM slices can still be
    // handled by the loader fallback.
    let vram_size: u32 = if is_one_screen { 0x80000 } else { 0x40000 };
    let vram_mask = vram_size - 1;
    let page_base: u32 = if !is_one_screen && display_page != 0 {
        vram_size
    } else {
        0
    };
    let mut text_scroll_iter = ScrollLineIter::new(inputs.gdc_scroll_start_line);
    let mut pegc_scanline_iter = PegcScanlineBaseIter::new(
        inputs.gdc_graphics_scroll,
        graphics_pitch,
        page_base,
        inputs.gdc_graphics_display_mode_is_graphics,
    );

    for y in 0..max_y {
        // Text overlay rules are shared with digital modes; only the graphics
        // source changes from bitplanes to 8-bit PEGC palette indices.
        let (text_scroll_start, text_scanline) = text_scroll_iter.next();
        let (row, glyph_y) =
            compute_text_row_and_glyph_y(text_scanline, scanlines_per_row, ssl, sur);
        let font_line = topline + glyph_y as i32;
        let font_rom_line = (font_line as u32) & 0x0F;
        let font_visible = font_line >= 0 && font_line < cl;
        let at_underline_line = font_line + 1 == lines;
        let cursor_active_for_scanline = font_line >= cursor_top && font_line <= cursor_bottom;
        let text_cell_base = text_scroll_start.wrapping_add(row.wrapping_mul(text_pitch));

        let pegc_scanline_base = pegc_scanline_iter.next();

        if graphics_enabled {
            load_pegc_indices(
                pegc.vram,
                pegc_scanline_base,
                vram_mask,
                &mut scratch.pegc_indices,
            );
        } else {
            scratch.pegc_indices.fill(0);
        }

        if text_enabled {
            decode_text_row(
                font_rom,
                text_cells,
                TextRowState {
                    cell_base: text_cell_base,
                    font_rom_line,
                    font_visible,
                    fontsel_8x16,
                    at_underline_line,
                    cursor_active_for_scanline,
                    width40_mode,
                },
                scratch,
            );
        } else {
            scratch.text_byte.fill(0);
            scratch.text_color.fill(0);
        }

        let row_offset = (y as usize) * ROW_BYTES;
        let row_buf = &mut framebuffer[row_offset..row_offset + ROW_BYTES];

        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        if has_simd {
            #[allow(unsafe_code)]
            // SAFETY: `has_simd` was validated at renderer construction:
            // `is_x86_feature_detected!("avx2")` on x86_64, unconditional on
            // aarch64 where NEON is in the baseline ISA.
            unsafe {
                #[cfg(target_arch = "x86_64")]
                avx2::combine_cells_pegc_avx2(
                    row_buf,
                    scratch,
                    &pegc.palette_rgba_256,
                    graphics_enabled,
                );
                #[cfg(target_arch = "aarch64")]
                neon::combine_cells_pegc_neon(
                    row_buf,
                    scratch,
                    &pegc.palette_rgba_256,
                    graphics_enabled,
                );
            }
            continue;
        }

        combine_cells_pegc(row_buf, scratch, &pegc.palette_rgba_256, graphics_enabled);
    }
}

#[derive(Clone, Copy)]
pub(super) struct CombineMode {
    /// Text and graphics enable bits are kept here so the scalar and AVX2
    /// combine paths make the same palette choices.
    pub text_enabled: bool,
    pub graphics_enabled: bool,
    pub is_monochrome: bool,
    pub is_16_color: bool,
}

#[derive(Clone, Copy)]
struct TextRowState {
    /// Per-scanline text state after scroll and CRTC row-shape resolution.
    /// This lets cell decode avoid re-reading hardware registers.
    cell_base: u32,
    font_rom_line: u32,
    font_visible: bool,
    fontsel_8x16: bool,
    at_underline_line: bool,
    cursor_active_for_scanline: bool,
    width40_mode: bool,
}

/// Iterator over packed GDC scroll areas. Low 16 bits are the start address;
/// high 16 bits are the line count in scanlines.
struct ScrollLineIter {
    scrolls: [u32; 4],
    index: usize,
    start_address: u32,
    line_count: u32,
    scanline_in_area: u32,
    fallback_scanline: u32,
}

impl ScrollLineIter {
    fn new(scrolls: [u32; 4]) -> Self {
        let mut iter = Self {
            scrolls,
            index: 0,
            start_address: 0,
            line_count: 0,
            scanline_in_area: 0,
            fallback_scanline: 0,
        };
        iter.advance_to_next_area();
        iter
    }

    fn next(&mut self) -> (u32, u32) {
        if self.line_count == 0 {
            // If the GDC snapshot has no active scroll area, keep advancing a
            // synthetic scanline so row math still behaves deterministically.
            let scanline = self.fallback_scanline;
            self.fallback_scanline = self.fallback_scanline.wrapping_add(1);
            return (0, scanline);
        }

        let result = (self.start_address, self.scanline_in_area);
        self.scanline_in_area = self.scanline_in_area.wrapping_add(1);
        if self.scanline_in_area >= self.line_count {
            self.index += 1;
            self.scanline_in_area = 0;
            self.advance_to_next_area();
        }
        result
    }

    fn advance_to_next_area(&mut self) {
        while self.index < self.scrolls.len() {
            let scroll = self.scrolls[self.index];
            let line_count = (scroll >> 16) & 0xFFFF;
            if line_count != 0 {
                self.start_address = scroll & 0xFFFF;
                self.line_count = line_count;
                return;
            }
            self.index += 1;
        }

        self.start_address = 0;
        self.line_count = 0;
        self.scanline_in_area = 0;
        self.fallback_scanline = 0;
    }
}

/// Digital graphics address iterator. Standard PC-98 graphics store 8 pixels
/// per byte; GDC start addresses are word-based, hence the `* 2`.
struct GraphicsByteBaseIter {
    scroll_iter: ScrollLineIter,
    pitch: u32,
    repeat: u32,
    repeat_remaining: u32,
    current_byte_base: u32,
}

impl GraphicsByteBaseIter {
    fn new(scrolls: [u32; 4], pitch: u32, repeat: u32) -> Self {
        Self {
            scroll_iter: ScrollLineIter::new(scrolls),
            pitch,
            repeat: repeat.max(1),
            repeat_remaining: 0,
            current_byte_base: 0,
        }
    }

    /// Returns the byte base for the current raster and whether this raster is
    /// the first of a fresh VRAM row cycle (i.e. not a repeat).
    fn next(&mut self) -> (u32, bool) {
        let is_first = self.repeat_remaining == 0;
        if is_first {
            let (start_address, scanline) = self.scroll_iter.next();
            // `repeat` folds together GDC lines-per-row and zoom so each VRAM
            // scanline can be reused for multiple output scanlines.
            self.current_byte_base = start_address
                .wrapping_mul(2)
                .wrapping_add(scanline.wrapping_mul(self.pitch));
            self.repeat_remaining = self.repeat;
        }

        self.repeat_remaining = self.repeat_remaining.wrapping_sub(1);
        (self.current_byte_base, is_first)
    }
}

/// PEGC scanline address iterator. PEGC is byte-per-pixel, so the GDC word
/// address is expanded to pixels here and `load_pegc_indices` adds X.
struct PegcScanlineBaseIter {
    scroll_iter: ScrollLineIter,
    pitch: u32,
    page_base: u32,
    graphics_display_mode: bool,
}

impl PegcScanlineBaseIter {
    fn new(scrolls: [u32; 4], pitch: u32, page_base: u32, graphics_display_mode: bool) -> Self {
        Self {
            scroll_iter: ScrollLineIter::new(scrolls),
            pitch,
            page_base,
            graphics_display_mode,
        }
    }

    fn next(&mut self) -> u32 {
        let (start_address, scanline) = self.scroll_iter.next();
        let line_pitch = if self.graphics_display_mode {
            self.pitch
        } else {
            self.pitch.wrapping_mul(2)
        };
        let inner = start_address
            .wrapping_mul(2)
            .wrapping_add(scanline.wrapping_mul(line_pitch))
            .wrapping_mul(8);
        self.page_base.wrapping_add(inner)
    }
}

fn combine_cells_digital(
    row_buf: &mut [u8],
    scratch: &ComposeScratch,
    graphics_palette_rgba: &[u32; 16],
    text_palette_rgba: &[u32; 16],
    mono_lookup: &[u8; 16],
    mode: CombineMode,
) {
    debug_assert_eq!(row_buf.len(), ROW_BYTES);

    for cell in 0..CELLS_PER_ROW {
        let text_byte = scratch.text_byte[cell];
        let text_color = scratch.text_color[cell] as u32;
        let b_byte = scratch.graphics_b[cell];
        let r_byte = scratch.graphics_r[cell];
        let g_byte = scratch.graphics_g[cell];
        let e_byte = scratch.graphics_e[cell];

        let cell_offset = cell * PIXELS_PER_CELL * PIXEL_BYTES;
        for bit in 0..PIXELS_PER_CELL {
            let mask = 1u8 << (7 - bit);
            let text_on = (text_byte & mask) != 0;
            // Plane bit order follows PC-98 palette index order: B, R, G,
            // then E for 16-color analog graphics.
            let blue = ((b_byte & mask) != 0) as u32;
            let red = ((r_byte & mask) != 0) as u32;
            let green = ((g_byte & mask) != 0) as u32;
            let extended = ((e_byte & mask) != 0) as u32;
            let graphics_color = blue | (red << 1) | (green << 2) | (extended << 3);

            let pixel = combine_pixel_digital(
                text_on,
                text_color,
                graphics_color,
                graphics_palette_rgba,
                text_palette_rgba,
                mono_lookup,
                mode,
            );

            let offset = cell_offset + bit * PIXEL_BYTES;
            row_buf[offset..offset + PIXEL_BYTES].copy_from_slice(&pixel.to_le_bytes());
        }
    }
}

#[inline]
fn combine_pixel_digital(
    text_on: bool,
    text_color: u32,
    graphics_color: u32,
    graphics_palette_rgba: &[u32; 16],
    text_palette_rgba: &[u32; 16],
    mono_lookup: &[u8; 16],
    mode: CombineMode,
) -> u32 {
    // `is_text_pixel` carries whether the resolved color comes from the text
    // plane. On skip rasters the graphics palette is dimmed but the text
    // palette stays at full brightness.
    let (use_fixed_color, is_text_pixel, color_index) =
        if mode.is_monochrome && mode.graphics_enabled {
            let graphics_on = mono_lookup[graphics_color as usize] != 0;
            // Mixed monochrome mode treats graphics as a mask. When text is also
            // enabled, graphics-on pixels inherit the current text cell color so
            // text and graphics share the foreground color for that cell.
            if text_on || (graphics_on && mode.text_enabled) {
                (mode.is_16_color, true, text_color)
            } else if graphics_on {
                (true, false, MONOCHROME_GRAPHICS_COLOR)
            } else {
                (false, false, 0)
            }
        } else if text_on {
            (mode.is_16_color, true, text_color)
        } else {
            let palette_index = if mode.graphics_enabled && !mode.is_16_color {
                graphics_color + DIGITAL_GRAPHICS_PALETTE_BASE
            } else {
                graphics_color
            };
            (false, false, palette_index)
        };

    if use_fixed_color {
        // Fixed text colors are used for text in 16-color analog mode and for
        // monochrome graphics, independent of palette register contents.
        FIXED_TEXT_LUT[(color_index & 0x07) as usize]
    } else if is_text_pixel {
        text_palette_rgba[(color_index & 0x0F) as usize]
    } else {
        graphics_palette_rgba[(color_index & 0x0F) as usize]
    }
}

fn combine_cells_pegc(
    row_buf: &mut [u8],
    scratch: &ComposeScratch,
    palette_256: &[u32; 256],
    graphics_enabled: bool,
) {
    debug_assert_eq!(row_buf.len(), ROW_BYTES);

    for cell in 0..CELLS_PER_ROW {
        let text_byte = scratch.text_byte[cell];
        let text_color = scratch.text_color[cell] as u32;

        let cell_offset = cell * PIXELS_PER_CELL * PIXEL_BYTES;
        for bit in 0..PIXELS_PER_CELL {
            let mask = 1u8 << (7 - bit);
            let text_on = (text_byte & mask) != 0;
            // PEGC graphics use the 256-entry palette, but text overlay still
            // uses the fixed 8 hardware text colors.
            let pixel = if text_on {
                FIXED_TEXT_LUT[(text_color & 0x07) as usize]
            } else if graphics_enabled {
                let pixel_x = cell * PIXELS_PER_CELL + bit;
                palette_256[scratch.pegc_indices[pixel_x] as usize]
            } else {
                BLACK
            };

            let offset = cell_offset + bit * PIXEL_BYTES;
            row_buf[offset..offset + PIXEL_BYTES].copy_from_slice(&pixel.to_le_bytes());
        }
    }
}

fn decode_text_row(
    font_rom: &[u8],
    text_cells: &[u32; TEXT_CELL_COUNT],
    state: TextRowState,
    scratch: &mut ComposeScratch,
) {
    if state.width40_mode {
        for k in 0..(CELLS_PER_ROW / 2) {
            // In 40-column mode each text cell is 16 output pixels wide. Only
            // every other memory column is displayed, matching the GDC layout.
            let memory_column = (k as u32) * 2;
            let cell_index = state.cell_base.wrapping_add(memory_column) & TEXT_CELL_INDEX_MASK;
            let cell = text_cells[cell_index as usize];
            let (font_byte_8, color) = decode_cell_to_byte(cell, font_rom, state);
            scratch.text_byte[2 * k] = BIT_DOUBLE_HIGH[font_byte_8 as usize];
            scratch.text_byte[2 * k + 1] = BIT_DOUBLE_LOW[font_byte_8 as usize];
            scratch.text_color[2 * k] = color;
            scratch.text_color[2 * k + 1] = color;
        }
    } else {
        for k in 0..CELLS_PER_ROW {
            let memory_column = k as u32;
            let cell_index = state.cell_base.wrapping_add(memory_column) & TEXT_CELL_INDEX_MASK;
            let cell = text_cells[cell_index as usize];
            let (font_byte_8, color) = decode_cell_to_byte(cell, font_rom, state);
            scratch.text_byte[k] = font_byte_8;
            scratch.text_color[k] = color;
        }
    }
}

fn decode_cell_to_byte(cell: u32, font_rom: &[u8], state: TextRowState) -> (u8, u8) {
    let font_offset = cell & TEXT_CELL_FONT_OFFSET_MASK;
    let color = ((cell >> TEXT_CELL_COLOR_SHIFT) & 0x07) as u8;
    let blank = (cell & TEXT_CELL_FLAG_BLANK) != 0;
    let is_reverse = (cell & TEXT_CELL_FLAG_REVERSE) != 0;
    let is_underline = (cell & TEXT_CELL_FLAG_UNDERLINE_THIS) != 0;
    let underline_left = (cell & TEXT_CELL_FLAG_UNDERLINE_LEFT) != 0;
    let is_vertical_line = (cell & TEXT_CELL_FLAG_VERTICAL_LINE) != 0;
    let cursor_cell = (cell & TEXT_CELL_FLAG_CURSOR) != 0;

    let mut byte: u8 = if !state.font_visible || blank {
        0
    } else {
        // In 6x8 mode the same font byte covers two output scanlines.
        let font_byte_offset = if state.fontsel_8x16 {
            font_offset.wrapping_add(state.font_rom_line)
        } else {
            font_offset.wrapping_add(state.font_rom_line / 2)
        };
        font_rom
            .get(font_byte_offset as usize)
            .copied()
            .unwrap_or(0)
    };

    if is_reverse {
        byte = !byte;
    }
    if cursor_cell && state.cursor_active_for_scanline {
        // Cursor is an XOR overlay; the normalizer pre-marks cells affected by
        // wide characters, so only the vertical range is handled here.
        byte = !byte;
    }
    if state.at_underline_line {
        // Underline covers the right half of this cell and the left half of
        // the next cell; the normalizer precomputes the left-half flag.
        if is_underline {
            byte |= 0x0F;
        }
        if underline_left {
            byte |= 0xF0;
        }
    }
    if is_vertical_line {
        // PC-98 vertical-line attribute draws at pixel 4, represented by bit 3.
        byte |= 0x08;
    }

    (byte, color)
}

fn load_graphics_row(plane: &[u8], byte_base: u32, dst: &mut [u8; CELLS_PER_ROW]) {
    if plane.len() < GRAPHICS_PLANE_SIZE {
        for (i, slot) in dst.iter_mut().enumerate() {
            let idx = (byte_base.wrapping_add(i as u32) & GRAPHICS_PLANE_MASK) as usize;
            *slot = plane.get(idx).copied().unwrap_or(0);
        }
        return;
    }

    let base = (byte_base & GRAPHICS_PLANE_MASK) as usize;
    let bytes_until_wrap = GRAPHICS_PLANE_SIZE - base;
    if bytes_until_wrap >= CELLS_PER_ROW {
        // Full-size VRAM snapshots can copy the 80 bytes for a row directly;
        // the fallback above keeps small test buffers bounds-safe.
        dst.copy_from_slice(&plane[base..base + CELLS_PER_ROW]);
    } else {
        dst[..bytes_until_wrap].copy_from_slice(&plane[base..GRAPHICS_PLANE_SIZE]);
        let remaining = CELLS_PER_ROW - bytes_until_wrap;
        dst[bytes_until_wrap..].copy_from_slice(&plane[..remaining]);
    }
}

fn load_pegc_indices(vram: &[u8], scanline_base: u32, vram_mask: u32, dst: &mut [u8]) {
    debug_assert_eq!(dst.len(), SoftwareRenderer::WIDTH);

    let vram_size = (vram_mask as usize).wrapping_add(1);
    if vram.len() >= vram_size {
        let base = (scanline_base & vram_mask) as usize;
        let pixels_until_wrap = vram_size - base;
        if pixels_until_wrap >= SoftwareRenderer::WIDTH {
            // PEGC stores one palette index per pixel, so a whole scanline is
            // copied into scratch before text overlay.
            dst.copy_from_slice(&vram[base..base + SoftwareRenderer::WIDTH]);
        } else {
            dst[..pixels_until_wrap].copy_from_slice(&vram[base..vram_size]);
            let remaining = SoftwareRenderer::WIDTH - pixels_until_wrap;
            dst[pixels_until_wrap..].copy_from_slice(&vram[..remaining]);
        }
        return;
    }

    for (i, slot) in dst.iter_mut().enumerate() {
        let addr = (scanline_base.wrapping_add(i as u32) & vram_mask) as usize;
        *slot = vram.get(addr).copied().unwrap_or(0);
    }
}

fn compute_max_y(inputs: &RenderInputs<'_>) -> u32 {
    let is_pegc = matches!(inputs.graphics, GraphicsInput::Pegc(_));
    // PEGC 480-line output requires BOTH a GDC active-line count above 400
    // AND the 31.778 kHz CRT scan-frequency flag (port 09A8h).
    // Standard PC-98 modes always cap at 400 active lines.
    if is_pegc && inputs.gdc_graphics_al > 400 && inputs.crt_31khz_enabled {
        inputs.gdc_graphics_al.min(SoftwareRenderer::HEIGHT as u32)
    } else {
        400
    }
}

fn compute_mono_lookup(mask: u32) -> [u8; 16] {
    let mut table = [0u8; 16];
    for (i, slot) in table.iter_mut().enumerate() {
        *slot = ((mask >> i) & 1) as u8;
    }
    table
}

fn compute_text_line_shape(pl: u32, bl: u32) -> (i32, i32) {
    let topline;
    let mut lines;
    let bl_plus_1 = bl as i32 + 1;
    // PL values 16-31 encode negative top lines through wraparound. This is
    // why `topline` is signed even though the raw CRTC register is 5-bit.
    if pl >= 16 {
        topline = pl as i32 - 32;
        lines = bl_plus_1;
    } else {
        topline = pl as i32;
        lines = bl_plus_1 - topline;
        if lines <= 0 {
            lines += 32;
        }
    }
    (topline, lines)
}

fn compute_text_row_and_glyph_y(
    scanline: u32,
    scanlines_per_row: u32,
    ssl: u32,
    sur: u32,
) -> (u32, u32) {
    if sur == 0 && ssl > 0 {
        // Smooth scroll with no upper fixed region: the first visible row
        // starts partway through its glyph.
        let first_row_height = scanlines_per_row.wrapping_sub(ssl);
        if scanline < first_row_height {
            (0, scanline.wrapping_add(ssl))
        } else {
            let adjusted = scanline.wrapping_sub(first_row_height);
            (
                adjusted / scanlines_per_row + 1,
                adjusted % scanlines_per_row,
            )
        }
    } else if sur != 0 {
        // SUR reserves an upper fixed text region. SSL then applies only to
        // the scrolling region below it.
        let upper_rows = 32u32 - sur;
        let upper_zone_scanlines = upper_rows.wrapping_mul(scanlines_per_row);
        if scanline < upper_zone_scanlines {
            (scanline / scanlines_per_row, scanline % scanlines_per_row)
        } else {
            let zone_scanline = scanline.wrapping_sub(upper_zone_scanlines);
            if ssl > 0 {
                let first_scroll_row_height = scanlines_per_row.wrapping_sub(ssl);
                if zone_scanline < first_scroll_row_height {
                    (upper_rows, zone_scanline.wrapping_add(ssl))
                } else {
                    let adjusted = zone_scanline.wrapping_sub(first_scroll_row_height);
                    (
                        upper_rows + adjusted / scanlines_per_row + 1,
                        adjusted % scanlines_per_row,
                    )
                }
            } else {
                (
                    upper_rows + zone_scanline / scanlines_per_row,
                    zone_scanline % scanlines_per_row,
                )
            }
        }
    } else {
        (scanline / scanlines_per_row, scanline % scanlines_per_row)
    }
}

fn fill_black(buffer: &mut [u8]) {
    debug_assert_eq!(buffer.len() % PIXEL_BYTES, 0);
    for chunk in buffer.chunks_exact_mut(PIXEL_BYTES) {
        chunk.copy_from_slice(&BLACK.to_le_bytes());
    }
}
