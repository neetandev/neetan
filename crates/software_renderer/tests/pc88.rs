//! PC-88 renderer tests: graphics-mode rasterization and text/graphics compositing.

use software_renderer::{GraphicsMode88, Pc88Renderer, RenderInputs88, pc88::PC88_WIDTH};

const PLANE_BYTES: usize = 0x4000;
const ANK_FONT_OFFSET: usize = 0x1000;

/// Returns the RGB triple at pixel (x, y) in the packed RGBA framebuffer.
fn pixel(framebuffer: &[u8], x: usize, y: usize) -> [u8; 3] {
    let index = (y * PC88_WIDTH + x) * 4;
    [
        framebuffer[index],
        framebuffer[index + 1],
        framebuffer[index + 2],
    ]
}

/// A small graphics palette where pen N maps to a distinct, recognizable color.
fn graphics_palette() -> [[u8; 3]; 8] {
    [
        [0, 0, 0],       // 0
        [0, 0, 255],     // 1 blue
        [255, 0, 0],     // 2 red
        [255, 0, 255],   // 3 magenta
        [0, 255, 0],     // 4 green
        [0, 255, 255],   // 5 cyan
        [255, 255, 0],   // 6 yellow
        [255, 255, 255], // 7 white
    ]
}

#[test]
fn color8_combines_three_planes_into_pen_index() {
    let mut renderer = Pc88Renderer::new(&[]);
    let mut blue = vec![0u8; PLANE_BYTES];
    let mut red = vec![0u8; PLANE_BYTES];
    let mut green = vec![0u8; PLANE_BYTES];

    // Leftmost pixel: blue only -> pen 1. Second pixel: blue+red+green -> pen 7.
    blue[0] = 0b1100_0000;
    red[0] = 0b0100_0000;
    green[0] = 0b0100_0000;

    let inputs = RenderInputs88 {
        text_codes: &[],
        text_attrib: &[],
        columns: 80,
        rows: 25,
        char_height: 8,
        width_40col: false,
        color_mode: true,
        text_enabled: false,
        background_rgb: [10, 20, 30],
        graphics_enabled: true,
        graphics_mode: GraphicsMode88::Color8,
        line_400: false,
        gvram_blue: &blue,
        gvram_red: &red,
        gvram_green: &green,
        graphics_palette: graphics_palette(),
        palette_mode: false,
        plane_disable: 0,
        width: 640,
        height: 200,
    };
    renderer.render(&inputs);
    let framebuffer = renderer.framebuffer();

    assert_eq!(pixel(framebuffer, 0, 0), [0, 0, 255], "pen 1 (blue)");
    assert_eq!(pixel(framebuffer, 1, 0), [255, 255, 255], "pen 7 (white)");
    assert_eq!(
        pixel(framebuffer, 2, 0),
        [0, 0, 0],
        "pen 0 (black) in 8-color"
    );
}

#[test]
fn attrib200_colors_mask_by_text_attribute_and_honors_reverse() {
    let mut renderer = Pc88Renderer::new(&[]);
    let mut blue = vec![0u8; PLANE_BYTES];
    let red = vec![0u8; PLANE_BYTES];
    let green = vec![0u8; PLANE_BYTES];

    // Row 0: top two pixels set in the mask.
    blue[0] = 0b1100_0000;

    let mut attrib = vec![0u8; 25 * 80];
    attrib[0] = 3 << 5; // cell (0,0): color 3 (magenta), no reverse
    attrib[80] = (5 << 5) | 0x01; // cell (1,0): color 5 (cyan), reverse

    // Row for char row 1 starts at scanline char_height (8); set its mask bit 0.
    blue[8 * 10] = 0b0000_0000; // leave row 8 mask empty so reverse fills it

    let inputs = RenderInputs88 {
        text_codes: &[],
        text_attrib: &attrib,
        columns: 80,
        rows: 25,
        char_height: 8,
        width_40col: false,
        color_mode: true,
        text_enabled: false,
        background_rgb: [10, 20, 30],
        graphics_enabled: true,
        graphics_mode: GraphicsMode88::Attrib200,
        line_400: false,
        gvram_blue: &blue,
        gvram_red: &red,
        gvram_green: &green,
        graphics_palette: graphics_palette(),
        palette_mode: false,
        plane_disable: 0,
        width: 640,
        height: 200,
    };
    renderer.render(&inputs);
    let framebuffer = renderer.framebuffer();

    // Mask set -> text color 3 (magenta); mask clear -> background (pen 0).
    assert_eq!(pixel(framebuffer, 0, 0), [255, 0, 255], "mask set, color 3");
    assert_eq!(
        pixel(framebuffer, 2, 0),
        [10, 20, 30],
        "mask clear -> background"
    );

    // Char row 1 (scanline 8): mask byte is 0, reverse inverts it to all-set, so
    // every pixel takes the cell color 5 (cyan).
    assert_eq!(
        pixel(framebuffer, 0, 8),
        [0, 255, 255],
        "reverse fills with color 5"
    );
    assert_eq!(
        pixel(framebuffer, 7, 8),
        [0, 255, 255],
        "reverse fills with color 5"
    );
}

#[test]
fn attrib400_switches_plane_at_half_screen() {
    let mut renderer = Pc88Renderer::new(&[]);
    let mut blue = vec![0u8; PLANE_BYTES];
    let mut red = vec![0u8; PLANE_BYTES];
    let green = vec![0u8; PLANE_BYTES];

    blue[0] = 0b1000_0000; // upper half (blue), line 0 pixel 0
    red[0] = 0b1000_0000; // lower half (red), line 200 pixel 0

    let attrib = vec![0xE0u8; 25 * 80]; // all cells color 7 (white)

    let inputs = RenderInputs88 {
        text_codes: &[],
        text_attrib: &attrib,
        columns: 80,
        rows: 25,
        char_height: 8,
        width_40col: false,
        color_mode: true,
        text_enabled: false,
        background_rgb: [10, 20, 30],
        graphics_enabled: true,
        graphics_mode: GraphicsMode88::Attrib400,
        line_400: true,
        gvram_blue: &blue,
        gvram_red: &red,
        gvram_green: &green,
        graphics_palette: graphics_palette(),
        palette_mode: false,
        plane_disable: 0,
        width: 640,
        height: 400,
    };
    renderer.render(&inputs);
    let framebuffer = renderer.framebuffer();

    assert_eq!(
        pixel(framebuffer, 0, 0),
        [255, 255, 255],
        "upper half reads blue"
    );
    assert_eq!(
        pixel(framebuffer, 0, 200),
        [255, 255, 255],
        "lower half reads red"
    );
    // A pixel where the relevant plane is empty stays background.
    assert_eq!(
        pixel(framebuffer, 0, 8),
        [10, 20, 30],
        "blue empty -> background"
    );
}

#[test]
fn plane_disable_applies_in_attrib_mode_but_not_color8() {
    let mut blue = vec![0u8; PLANE_BYTES];
    blue[0] = 0b1000_0000; // line 0, pixel 0 set in the blue plane
    let red = vec![0u8; PLANE_BYTES];
    let green = vec![0u8; PLANE_BYTES];
    let attrib = vec![0xE0u8; 25 * 80]; // color 7 (white)

    let make = |mode, plane_disable| RenderInputs88 {
        text_codes: &[],
        text_attrib: &attrib,
        columns: 80,
        rows: 25,
        char_height: 8,
        width_40col: false,
        color_mode: true,
        text_enabled: false,
        background_rgb: [10, 20, 30],
        graphics_enabled: true,
        graphics_mode: mode,
        line_400: false,
        gvram_blue: &blue,
        gvram_red: &red,
        gvram_green: &green,
        graphics_palette: graphics_palette(),
        palette_mode: false,
        plane_disable,
        width: 640,
        height: 200,
    };

    // Attribute mode: disabling the blue plane (bit 0) drops the masked pixel to
    // the background color.
    let mut renderer = Pc88Renderer::new(&[]);
    renderer.render(&make(GraphicsMode88::Attrib200, 0x00));
    assert_eq!(
        pixel(renderer.framebuffer(), 0, 0),
        [255, 255, 255],
        "blue plane on"
    );
    renderer.render(&make(GraphicsMode88::Attrib200, 0x01));
    assert_eq!(
        pixel(renderer.framebuffer(), 0, 0),
        [10, 20, 30],
        "blue plane disabled"
    );

    // 8-color mode ignores the plane-disable flags.
    renderer.render(&make(GraphicsMode88::Color8, 0x01));
    assert_eq!(
        pixel(renderer.framebuffer(), 0, 0),
        [0, 0, 255],
        "Color8 ignores disable"
    );
}

#[test]
fn text_has_priority_over_graphics_and_graphics_shows_through() {
    // A font where glyph 0x41 ('A') lights only the top-left pixel of row 0.
    let mut font = vec![0u8; ANK_FONT_OFFSET + 0x800];
    font[ANK_FONT_OFFSET + 0x41 * 8] = 0b1000_0000;
    let mut renderer = Pc88Renderer::new(&font);

    // Graphics: every pixel pen 1 (blue) in 8-color mode.
    let blue = vec![0xFFu8; PLANE_BYTES];
    let red = vec![0u8; PLANE_BYTES];
    let green = vec![0u8; PLANE_BYTES];

    let mut codes = vec![0u8; 25 * 80];
    let mut attrib = vec![0u8; 25 * 80];
    codes[0] = 0x41;
    attrib[0] = 0xE0; // color 7 (white), not semigraphics

    let inputs = RenderInputs88 {
        text_codes: &codes,
        text_attrib: &attrib,
        columns: 80,
        rows: 25,
        char_height: 8,
        width_40col: false,
        color_mode: true,
        text_enabled: true,
        background_rgb: [10, 20, 30],
        graphics_enabled: true,
        graphics_mode: GraphicsMode88::Color8,
        line_400: false,
        gvram_blue: &blue,
        gvram_red: &red,
        gvram_green: &green,
        graphics_palette: graphics_palette(),
        palette_mode: false,
        plane_disable: 0,
        width: 640,
        height: 200,
    };
    renderer.render(&inputs);
    let framebuffer = renderer.framebuffer();

    // Top-left pixel: text dot wins (white).
    assert_eq!(
        pixel(framebuffer, 0, 0),
        [255, 255, 255],
        "text over graphics"
    );
    // Adjacent pixel has no text dot: graphics (blue) shows through.
    assert_eq!(
        pixel(framebuffer, 1, 0),
        [0, 0, 255],
        "graphics through text 0"
    );
}
