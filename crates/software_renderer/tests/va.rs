//! PC-88VA renderer integration tests: palette conversion, text-layer golden
//! frames in both 24.8 kHz and 15 kHz geometry, the text blink cadence, the
//! sprite layer composited over text and the backdrop, and the gap rasters of
//! a 200-line picture under each line doubling value.

use common::LineDoubling;
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
        line_doubling: LineDoubling::Auto,
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

/// Bytes per graphics VRAM line in the single-plane 4 bpp gap test scene.
const GRAPHICS_LINE_BYTES: usize = 320;
/// Size of the graphics VRAM image.
const GRAPHICS_VRAM_BYTES: usize = 0x4_0000;
/// Text foreground color of the gap test scene (palette entry 1).
const TEXT_COLOR: u16 = 0xFC00;
/// Graphics color of picture line 0 (palette entry 2).
const GRAPHICS_LINE0_COLOR: u16 = 0x03E0;
/// Graphics color of picture line 1 (palette entry 3).
const GRAPHICS_LINE1_COLOR: u16 = 0x7FFF;
/// Backdrop color of the gap test scene.
const BACKDROP_COLOR: u16 = 0x001F;
/// Graphics mode bits shared by the gap test scenes: graphics on, video
/// output on, single plane.
const GAP_SCENE_GRAPHICS_MODE: u16 = 0x8000 | 0x3000 | 0x0400;
/// Graphics mode bit of the 200-line graphics picture.
const GRAPHICS_200_LINE: u16 = 0x0002;

/// Text VRAM, graphics VRAM, palette, and font of the gap test scene: a glyph
/// with dot 0 on glyph row 0 and dot 1 on glyph row 1 in cell 0, graphics
/// line 0 in palette color 2 and graphics line 1 in palette color 3.
struct GapScene {
    /// Text VRAM image with the frame descriptor, the glyph cell and its
    /// attribute.
    text_vram: Vec<u8>,
    /// Graphics VRAM image with the two picture lines.
    graphics_vram: Vec<u8>,
    /// Palette with the text and the two graphics line colors.
    palette: [u16; 32],
    /// Font ROM image with the test glyph.
    font: Vec<u8>,
}

impl GapScene {
    /// Builds the gap test scene.
    fn new() -> Self {
        let mut text_vram = vec![0u8; TEXT_VRAM_BYTES];
        write_frame0(&mut text_vram, 0x1000, 0x0000);
        put_word(&mut text_vram, 0x1000, 0x0041);
        text_vram[0x1000 + 0x2000] = 0x01;

        let mut graphics_vram = vec![0u8; GRAPHICS_VRAM_BYTES];
        graphics_vram[..GRAPHICS_LINE_BYTES].fill(0x22);
        graphics_vram[GRAPHICS_LINE_BYTES..2 * GRAPHICS_LINE_BYTES].fill(0x33);

        let mut palette = [0u16; 32];
        palette[1] = TEXT_COLOR;
        palette[2] = GRAPHICS_LINE0_COLOR;
        palette[3] = GRAPHICS_LINE1_COLOR;

        let mut font = vec![0u8; FONT_BYTES];
        font[ank_glyph_offset(0x41)] = 0b1000_0000;
        font[ank_glyph_offset(0x41) + 1] = 0b0100_0000;

        Self {
            text_vram,
            graphics_vram,
            palette,
            font,
        }
    }

    /// Renders the scene and returns the framebuffer.
    fn render(
        &self,
        hsync_mode: HsyncModeVa,
        screen_lines: usize,
        graphics_mode: u16,
        line_doubling: LineDoubling,
    ) -> Vec<u8> {
        let mut renderer = VaRenderer::new(&self.font);
        let mut inputs = base_inputs(&self.text_vram, &self.palette, hsync_mode, screen_lines);
        inputs.line_doubling = line_doubling;
        inputs.graphics_mode = graphics_mode;
        inputs.graphics_resolution = 0x0001; // screen 0: 4 bpp
        inputs.color_composition = 0x00A8; // screen 0 = text, screen 1 = graphic 0
        inputs.transparent_graphic0 = 0x0001;
        inputs.graphics_vram = &self.graphics_vram;
        inputs.framebuffers[0] = FramebufferVa {
            frame_start: 0,
            frame_width: GRAPHICS_LINE_BYTES as u16,
            frame_lines: 0xFFFF,
            dot: 0,
            offset_x: 0,
            offset_y: 0,
            display_start: 0,
            display_height: 400,
            display_position: 0,
        };
        renderer.render(&inputs);
        renderer.framebuffer().to_vec()
    }
}

/// Text dot on glyph row 0.
const TEXT_ROW0_X: usize = 0;
/// Text dot on glyph row 1.
const TEXT_ROW1_X: usize = 1;
/// A dot without text that shows the graphics layer.
const GRAPHICS_X: usize = 8;

#[test]
fn gap_raster_15khz_non_interlace_follows_line_doubling() {
    let scene = GapScene::new();
    let text = va_color_to_rgba(TEXT_COLOR);
    let line0 = va_color_to_rgba(GRAPHICS_LINE0_COLOR);
    let line1 = va_color_to_rgba(GRAPHICS_LINE1_COLOR);
    let backdrop = va_color_to_rgba(BACKDROP_COLOR);

    for (line_doubling, repeats) in [
        (LineDoubling::Auto, false),
        (LineDoubling::On, true),
        (LineDoubling::Off, false),
    ] {
        let framebuffer = scene.render(
            HsyncModeVa::Khz15_98,
            200,
            GAP_SCENE_GRAPHICS_MODE | GRAPHICS_200_LINE,
            line_doubling,
        );
        let (gap_text, gap_graphics) = if repeats {
            (text, line0)
        } else {
            (backdrop, backdrop)
        };
        assert_eq!(pixel(&framebuffer, TEXT_ROW0_X, 0), text, "{line_doubling}");
        assert_eq!(pixel(&framebuffer, GRAPHICS_X, 0), line0, "{line_doubling}");
        assert_eq!(
            pixel(&framebuffer, TEXT_ROW0_X, 1),
            gap_text,
            "{line_doubling}"
        );
        assert_eq!(
            pixel(&framebuffer, GRAPHICS_X, 1),
            gap_graphics,
            "{line_doubling}"
        );
        assert_eq!(pixel(&framebuffer, TEXT_ROW1_X, 2), text, "{line_doubling}");
        assert_eq!(
            pixel(&framebuffer, TEXT_ROW0_X, 2),
            line1,
            "{line_doubling}"
        );
        assert_eq!(pixel(&framebuffer, GRAPHICS_X, 2), line1, "{line_doubling}");
        if repeats {
            assert_eq!(
                framebuffer[..SURFACE_WIDTH * 4],
                framebuffer[SURFACE_WIDTH * 4..SURFACE_WIDTH * 8],
                "the gap raster repeats the picture raster"
            );
        }
    }
}

/// Checks the 24 kHz 200-line graphics gap raster of one interlace mode.
fn check_24khz_graphics_gap_raster(
    interlace_mode: u16,
    line_doubling: LineDoubling,
    repeats: bool,
) {
    let scene = GapScene::new();
    let text = va_color_to_rgba(TEXT_COLOR);
    let line0 = va_color_to_rgba(GRAPHICS_LINE0_COLOR);
    let line1 = va_color_to_rgba(GRAPHICS_LINE1_COLOR);
    let backdrop = va_color_to_rgba(BACKDROP_COLOR);

    let framebuffer = scene.render(
        HsyncModeVa::Khz24_8,
        400,
        GAP_SCENE_GRAPHICS_MODE | GRAPHICS_200_LINE | interlace_mode,
        line_doubling,
    );
    let gap_graphics = if repeats { line0 } else { backdrop };
    let context = format!("mode {interlace_mode:#04X} {line_doubling}");
    assert_eq!(pixel(&framebuffer, TEXT_ROW0_X, 0), text, "{context}");
    assert_eq!(pixel(&framebuffer, GRAPHICS_X, 0), line0, "{context}");
    assert_eq!(pixel(&framebuffer, TEXT_ROW1_X, 1), text, "{context}");
    assert_eq!(
        pixel(&framebuffer, TEXT_ROW0_X, 1),
        gap_graphics,
        "{context}"
    );
    assert_eq!(
        pixel(&framebuffer, GRAPHICS_X, 1),
        gap_graphics,
        "{context}"
    );
    assert_eq!(pixel(&framebuffer, GRAPHICS_X, 2), line1, "{context}");
    assert_eq!(
        pixel(&framebuffer, GRAPHICS_X, 3),
        if repeats { line1 } else { backdrop },
        "{context}"
    );
}

#[test]
fn gap_raster_24khz_mode0_follows_line_doubling() {
    check_24khz_graphics_gap_raster(0x00, LineDoubling::Auto, false);
    check_24khz_graphics_gap_raster(0x00, LineDoubling::On, true);
    check_24khz_graphics_gap_raster(0x00, LineDoubling::Off, false);
}

#[test]
fn gap_raster_24khz_mode1_follows_line_doubling() {
    check_24khz_graphics_gap_raster(0x40, LineDoubling::Auto, true);
    check_24khz_graphics_gap_raster(0x40, LineDoubling::On, true);
    check_24khz_graphics_gap_raster(0x40, LineDoubling::Off, false);
}

#[test]
fn interlace_modes_ignore_line_doubling() {
    let scene = GapScene::new();
    for (hsync_mode, screen_lines, interlace_mode) in [
        (HsyncModeVa::Khz15_73, 200, 0x80),
        (HsyncModeVa::Khz15_73, 200, 0xC0),
        (HsyncModeVa::Khz24_8, 400, 0x80),
        (HsyncModeVa::Khz24_8, 400, 0xC0),
    ] {
        for graphics_200_line in [0, GRAPHICS_200_LINE] {
            let graphics_mode = GAP_SCENE_GRAPHICS_MODE | graphics_200_line | interlace_mode;
            let auto = scene.render(hsync_mode, screen_lines, graphics_mode, LineDoubling::Auto);
            for line_doubling in [LineDoubling::On, LineDoubling::Off] {
                let framebuffer =
                    scene.render(hsync_mode, screen_lines, graphics_mode, line_doubling);
                assert!(
                    framebuffer == auto,
                    "{hsync_mode:?} mode {graphics_mode:#06X} differs with {line_doubling}"
                );
            }
        }
    }
}

#[test]
fn gap_raster_24khz_400_line_graphics_ignore_line_doubling() {
    let scene = GapScene::new();
    let line1 = va_color_to_rgba(GRAPHICS_LINE1_COLOR);
    for interlace_mode in [0x00, 0x40] {
        let graphics_mode = GAP_SCENE_GRAPHICS_MODE | interlace_mode;
        let auto = scene.render(HsyncModeVa::Khz24_8, 400, graphics_mode, LineDoubling::Auto);
        assert_eq!(
            pixel(&auto, GRAPHICS_X, 1),
            line1,
            "mode {interlace_mode:#04X}"
        );
        for line_doubling in [LineDoubling::On, LineDoubling::Off] {
            let framebuffer = scene.render(HsyncModeVa::Khz24_8, 400, graphics_mode, line_doubling);
            assert!(
                framebuffer == auto,
                "mode {interlace_mode:#04X} differs with {line_doubling}"
            );
        }
    }
}
