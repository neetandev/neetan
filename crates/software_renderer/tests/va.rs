//! PC-88VA renderer integration tests: palette conversion, text-layer golden
//! frames in both 24.8 kHz and 15 kHz geometry, the text blink cadence, and the
//! sprite layer composited over text and the backdrop.

use software_renderer::{
    VaRenderer,
    va::{FramebufferVa, HsyncModeVa, RenderInputsVa, va_color_to_rgba},
};

const TEXT_VRAM_BYTES: usize = 0x4_0000;
const FONT_BYTES: usize = 0x5_0000;
const SURFACE_WIDTH: usize = 640;

/// ANK 16-dot glyph base for a character code in the VA font ROM.
fn ank_glyph_offset(code: u8) -> usize {
    0x40000 + (usize::from(code) << 4)
}

fn put_word(buffer: &mut [u8], offset: usize, value: u16) {
    buffer[offset] = value as u8;
    buffer[offset + 1] = (value >> 8) as u8;
}

/// Writes a frame-0 descriptor that covers the whole screen, sourcing character
/// data from `raster_start` with the given attribute mode.
fn write_frame0(text_vram: &mut [u8], raster_start: u16, mode: u16) {
    put_word(text_vram, 0x08, 160); // vw: 80 columns x 2 bytes
    put_word(text_vram, 0x0A, mode); // mode/fg/bg
    text_vram[0x0D] = 0; // raster offset
    put_word(text_vram, 0x10, raster_start); // rsa
    put_word(text_vram, 0x14, 0x01FE); // rh: full height
    put_word(text_vram, 0x16, 640); // rw -> 82 chars
    put_word(text_vram, 0x1A, 0); // rxp
}

/// The fixed palette used by the golden tests.
fn palette() -> [u16; 32] {
    let mut palette = [0u16; 32];
    palette[1] = 0xFC00; // pure green
    palette[2] = 0x03E0; // pure red
    palette
}

/// Builds a renderer with a single ANK glyph defined for `code`: top raster is
/// `0b1000_0000` (leftmost pixel only).
fn renderer_with_glyph(code: u8) -> VaRenderer {
    let mut font = vec![0u8; FONT_BYTES];
    font[ank_glyph_offset(code)] = 0b1000_0000;
    VaRenderer::new(&font)
}

fn base_inputs<'a>(
    text_vram: &'a [u8],
    palette: &'a [u16; 32],
    hsync_mode: HsyncModeVa,
    screen_lines: usize,
) -> RenderInputsVa<'a> {
    RenderInputsVa {
        text_vram,
        text_table: 0,
        attr_offset: 0x2000,
        line_height: 16,
        horizontal_line_position: 0xFF,
        blink_counter2: 0,
        text_magnify: false,
        screen_lines,
        sync_param0: 0,
        hsync_mode,
        sprite_table: 0,
        sprite_enabled: false,
        sprite_count_limit: 0x1F,
        sprite_magnify: false,
        sprite_grouping: false,
        cursor_sprite: 0,
        cursor_blink_enable: false,
        txtmode8: 0xFF, // 80-column
        txtmode: 0,
        graphics_mode: 0x3000, // XVSP | SYNCEN (video output enabled)
        graphics_resolution: 0,
        color_composition: 0x0008, // screen 0 = text layer
        rgb_composition: 0,
        palette_mode: 0,
        page_mask: 0,
        backdrop_color: 0x001F, // pure blue
        transparent_text_sprite: 0x0001,
        transparent_graphic0: 0,
        transparent_graphic1: 0,
        mask_mode: 0,
        mask_left: 0,
        mask_right: 0,
        mask_top: 0,
        mask_bottom: 0,
        palette_blink_counter: 0,
        palette,
        graphics_vram: &[],
        framebuffers: [FramebufferVa::default(); 4],
    }
}

fn pixel(framebuffer: &[u8], x: usize, y: usize) -> u32 {
    let base = (y * SURFACE_WIDTH + x) * 4;
    u32::from(framebuffer[base])
        | (u32::from(framebuffer[base + 1]) << 8)
        | (u32::from(framebuffer[base + 2]) << 16)
        | (u32::from(framebuffer[base + 3]) << 24)
}

#[test]
fn text_layer_composites_glyph_over_backdrop_24khz() {
    let mut text_vram = vec![0u8; TEXT_VRAM_BYTES];
    write_frame0(&mut text_vram, 0x1000, 0x0000); // mode 0
    // Character 'A' (0x41) in cell 0; attribute (mode 0): bg=0, fg=2.
    put_word(&mut text_vram, 0x1000, 0x0041);
    text_vram[0x1000 + 0x2000] = 0x02;

    let palette = palette();
    let mut renderer = renderer_with_glyph(0x41);
    let inputs = base_inputs(&text_vram, &palette, HsyncModeVa::Khz24_8, 400);
    let (width, height) = renderer.render(&inputs);
    assert_eq!((width, height), (640, 400));

    let framebuffer = renderer.framebuffer();
    // Glyph foreground pixel -> palette[2] (red).
    assert_eq!(pixel(framebuffer, 0, 0), va_color_to_rgba(0x03E0));
    // Background pixel inside the cell -> backdrop (blue).
    assert_eq!(pixel(framebuffer, 1, 0), va_color_to_rgba(0x001F));
    // A pixel well outside the glyph -> backdrop.
    assert_eq!(pixel(framebuffer, 300, 200), va_color_to_rgba(0x001F));
}

#[test]
fn text_layer_doubles_lines_in_15khz() {
    let mut text_vram = vec![0u8; TEXT_VRAM_BYTES];
    write_frame0(&mut text_vram, 0x1000, 0x0000);
    put_word(&mut text_vram, 0x1000, 0x0041);
    text_vram[0x1000 + 0x2000] = 0x02;

    let palette = palette();
    let mut renderer = renderer_with_glyph(0x41);
    let inputs = base_inputs(&text_vram, &palette, HsyncModeVa::Khz15_98, 200);
    let (width, height) = renderer.render(&inputs);
    // 200 programmed lines become 400 on the surface.
    assert_eq!((width, height), (640, 400));

    let framebuffer = renderer.framebuffer();
    // Even line carries the glyph; the odd line below it is blanked to backdrop.
    assert_eq!(pixel(framebuffer, 0, 0), va_color_to_rgba(0x03E0));
    assert_eq!(pixel(framebuffer, 0, 1), va_color_to_rgba(0x001F));
}

#[test]
fn blink_attribute_hides_the_cell_on_the_off_phase() {
    let mut text_vram = vec![0u8; TEXT_VRAM_BYTES];
    // Mode 1 carries the effect bits; frame default bg = 0, fg = 0.
    write_frame0(&mut text_vram, 0x1000, 0x0001);
    put_word(&mut text_vram, 0x1000, 0x0041);
    // Mode 1: fg = attr>>4 = 1, effect = attr&0x0f = blink (0x02).
    text_vram[0x1000 + 0x2000] = 0x12;

    let palette = palette();
    let mut renderer = renderer_with_glyph(0x41);

    // Blink "on" phase: foreground visible -> palette[1] (green).
    let mut inputs = base_inputs(&text_vram, &palette, HsyncModeVa::Khz24_8, 400);
    inputs.blink_counter2 = 0;
    renderer.render(&inputs);
    assert_eq!(
        pixel(renderer.framebuffer(), 0, 0),
        va_color_to_rgba(0xFC00)
    );

    // Blink "off" phase: foreground forced to background -> backdrop.
    let mut inputs = base_inputs(&text_vram, &palette, HsyncModeVa::Khz24_8, 400);
    inputs.blink_counter2 = 0x08;
    renderer.render(&inputs);
    assert_eq!(
        pixel(renderer.framebuffer(), 0, 0),
        va_color_to_rgba(0x001F)
    );
}

/// Writes one 8-byte sprite-table entry (four little-endian words).
fn write_sprite_entry(text_vram: &mut [u8], table: usize, index: usize, words: [u16; 4]) {
    let base = table + index * 8;
    for (word_index, word) in words.iter().enumerate() {
        put_word(text_vram, base + word_index * 2, *word);
    }
}

#[test]
fn sprite_layer_composites_over_text_and_backdrop() {
    let table = 0x100;
    let mut text_vram = vec![0u8; TEXT_VRAM_BYTES];
    // Sprite 0: enabled, vlines code 0 (=4), yp 0 | width code 0 (=4 bytes),
    // xp 0, 16-color | spda word 0x100 (-> byte offset 0x200) | fg/bg unused.
    write_sprite_entry(&mut text_vram, table, 0, [0x0200, 0x0000, 0x0100, 0x0000]);
    text_vram[0x200] = 0x12; // x0 -> color 1, x1 -> color 2

    let palette = palette();
    let mut renderer = VaRenderer::new(&vec![0u8; FONT_BYTES]);
    let mut inputs = base_inputs(&text_vram, &palette, HsyncModeVa::Khz24_8, 400);
    inputs.sprite_table = table;
    inputs.sprite_enabled = true;
    inputs.color_composition = 0x0009; // screen 0 = sprite layer
    inputs.page_mask = 0xF000; // text/sprite boundary 15 -> codes route to sprite
    renderer.render(&inputs);

    let framebuffer = renderer.framebuffer();
    // Sprite nibbles -> palette[1] (green) and palette[2] (red).
    assert_eq!(pixel(framebuffer, 0, 0), va_color_to_rgba(0xFC00));
    assert_eq!(pixel(framebuffer, 1, 0), va_color_to_rgba(0x03E0));
    // Transparent sprite pixel falls through to the backdrop (blue).
    assert_eq!(pixel(framebuffer, 2, 0), va_color_to_rgba(0x001F));
}

#[test]
fn video_output_disabled_blacks_the_frame() {
    let mut text_vram = vec![0u8; TEXT_VRAM_BYTES];
    write_frame0(&mut text_vram, 0x1000, 0x0000);
    put_word(&mut text_vram, 0x1000, 0x0041);
    text_vram[0x1000 + 0x2000] = 0x02;

    let palette = palette();
    let mut renderer = renderer_with_glyph(0x41);
    let mut inputs = base_inputs(&text_vram, &palette, HsyncModeVa::Khz24_8, 400);
    inputs.graphics_mode = 0; // XVSP / SYNCEN clear -> no video output
    renderer.render(&inputs);

    // The whole frame is opaque black regardless of text content.
    assert_eq!(pixel(renderer.framebuffer(), 0, 0), va_color_to_rgba(0));
    assert_eq!(pixel(renderer.framebuffer(), 1, 0), va_color_to_rgba(0));
}
