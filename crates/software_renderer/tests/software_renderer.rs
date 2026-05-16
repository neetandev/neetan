use std::{env, fs};

use software_renderer::{
    GdcGraphicsInput, GraphicsInput, PegcRenderInputs, RenderInputs, SoftwareRenderer,
    TEXT_VRAM_BYTES,
};

static ZERO_PLANE: [u8; 0x8000] = [0u8; 0x8000];

fn base_inputs<'a>(
    text_vram: &'a [u8; TEXT_VRAM_BYTES],
    graphics_b: &'a [u8],
    graphics_r: &'a [u8],
    graphics_g: &'a [u8],
    graphics_e: &'a [u8],
) -> RenderInputs<'a> {
    RenderInputs {
        text_vram,
        gdc_text_pitch: 0,
        gdc_scroll_start_line: [0; 4],
        video_mode: 0,
        crtc_pl_bl: 0,
        crtc_cl_ssl: 0,
        crtc_sur_sdr: 0,
        kanji_high_mask: 0xFF,
        attr_semigraphics_mode: false,
        fontsel_8x16: true,
        blink_visible: true,
        cursor_visible: false,
        cursor_addr: 0,
        cursor_top: 0,
        cursor_bottom: 0,

        gdc_graphics_pitch: 0,
        gdc_graphics_scroll: [0; 4],
        gdc_graphics_display_mode_is_graphics: false,
        gdc_graphics_al: 0,
        crt_31khz_enabled: false,

        palette_rgba: [0u32; 16],
        global_enabled: false,
        text_enabled: false,
        graphics_enabled: false,

        graphics: GraphicsInput::Gdc(GdcGraphicsInput {
            b_plane: graphics_b,
            r_plane: graphics_r,
            g_plane: graphics_g,
            e_plane: graphics_e,
            lines_per_row: 1,
            zoom_display: 0,
            monochrome_mask: 0,
            is_16_color: false,
        }),
    }
}

fn gdc_mut<'a, 'b>(inputs: &'b mut RenderInputs<'a>) -> &'b mut GdcGraphicsInput<'a> {
    match &mut inputs.graphics {
        GraphicsInput::Gdc(gdc) => gdc,
        _ => panic!("expected GDC graphics variant"),
    }
}

fn pixel_at(framebuffer: &[u8], x: usize, y: usize) -> [u8; 4] {
    let offset = (y * SoftwareRenderer::WIDTH + x) * 4;
    [
        framebuffer[offset],
        framebuffer[offset + 1],
        framebuffer[offset + 2],
        framebuffer[offset + 3],
    ]
}

#[test]
fn renders_text_cell_color_and_writes_ppm() {
    // Set up font ROM so 'A' renders with bit 7 set on its first scanline.
    let mut font_rom = vec![0u8; 0x83000];
    font_rom[0x80000 + (b'A' as usize) * 16] = 0x80;

    // Text VRAM: cell 0 is 'A' with attr bits (color=2, secret=1).
    let mut text_vram = [0u8; TEXT_VRAM_BYTES];
    text_vram[0] = b'A';
    text_vram[0x2000] = (2 << 5) | 0x01;

    let mut inputs = base_inputs(
        &text_vram,
        &ZERO_PLANE,
        &ZERO_PLANE,
        &ZERO_PLANE,
        &ZERO_PLANE,
    );
    inputs.global_enabled = true;
    inputs.text_enabled = true;
    gdc_mut(&mut inputs).is_16_color = true;
    inputs.gdc_text_pitch = 80;
    inputs.gdc_scroll_start_line[0] = 400 << 16;
    inputs.video_mode = 0x08;
    inputs.crtc_pl_bl = 15 << 16;
    inputs.crtc_cl_ssl = 16;
    inputs.palette_rgba[0] = 0xFF00_0000;
    inputs.palette_rgba[2] = 0xFF00_00FF;

    let mut renderer = SoftwareRenderer::new(&font_rom);
    renderer.render(&inputs);

    let fb = renderer.framebuffer();
    assert!(fb.len() >= SoftwareRenderer::FRAMEBUFFER_BYTES);
    assert_eq!(pixel_at(fb, 0, 0), [0xFF, 0x00, 0x00, 0xFF]);
    assert_eq!(pixel_at(fb, 1, 0), [0x00, 0x00, 0x00, 0xFF]);
    assert_eq!(pixel_at(fb, 0, 401), [0x00, 0x00, 0x00, 0xFF]);

    let path = env::temp_dir().join(format!(
        "neetan-software-renderer-{}.ppm",
        std::process::id()
    ));
    SoftwareRenderer::write_ppm(
        &path,
        fb,
        SoftwareRenderer::WIDTH as u32,
        SoftwareRenderer::HEIGHT as u32,
    )
    .unwrap();

    let ppm = fs::read(&path).unwrap();
    let header = b"P6\n640 480\n255\n";
    assert!(ppm.starts_with(header));
    assert_eq!(ppm.len(), header.len() + SoftwareRenderer::PIXEL_COUNT * 3);
    assert_eq!(&ppm[header.len()..header.len() + 3], &[0xFF, 0x00, 0x00]);

    fs::remove_file(path).unwrap();
}

#[test]
fn renders_graphics_only_monochrome_from_mask() {
    let font_rom = vec![0u8; 0x83000];

    let text_vram: [u8; TEXT_VRAM_BYTES] = [0u8; TEXT_VRAM_BYTES];
    let mut graphics_b = [0u8; 0x8000];
    let mut graphics_r = [0u8; 0x8000];
    let mut graphics_g = [0u8; 0x8000];
    graphics_b[0] = 0x80;
    graphics_r[0] = 0x40;
    graphics_g[0] = 0x20;

    let mut inputs = base_inputs(
        &text_vram,
        &graphics_b,
        &graphics_r,
        &graphics_g,
        &ZERO_PLANE,
    );
    inputs.global_enabled = true;
    inputs.graphics_enabled = true;
    inputs.video_mode = 0x02; // monochrome
    inputs.gdc_graphics_pitch = 80;
    inputs.gdc_graphics_scroll[0] = 400 << 16;
    gdc_mut(&mut inputs).monochrome_mask = 0x0000_F0F0;
    inputs.palette_rgba[0] = 0xFF00_0000;
    inputs.palette_rgba[7] = 0xFFFF_FFFF; // monochrome graphics color

    let mut renderer = SoftwareRenderer::new(&font_rom);
    renderer.render(&inputs);
    let fb = renderer.framebuffer();
    assert_eq!(pixel_at(fb, 0, 0), [0x00, 0x00, 0x00, 0xFF]);
    assert_eq!(pixel_at(fb, 1, 0), [0x00, 0x00, 0x00, 0xFF]);
    assert_eq!(pixel_at(fb, 2, 0), [0xFF, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn uses_digital_graphics_palette_offset() {
    let font_rom = vec![0u8; 0x83000];

    let text_vram: [u8; TEXT_VRAM_BYTES] = [0u8; TEXT_VRAM_BYTES];
    let mut graphics_r = [0u8; 0x8000];
    graphics_r[0] = 0x80;

    let mut inputs = base_inputs(
        &text_vram,
        &ZERO_PLANE,
        &graphics_r,
        &ZERO_PLANE,
        &ZERO_PLANE,
    );
    inputs.global_enabled = true;
    inputs.graphics_enabled = true;
    inputs.gdc_graphics_pitch = 80;
    inputs.gdc_graphics_scroll[0] = 400 << 16;
    inputs.palette_rgba[0] = 0xFF00_0000;
    inputs.palette_rgba[2] = 0xFF00_00FF;
    inputs.palette_rgba[8 + 2] = 0xFFFF_FFFF;

    let mut renderer = SoftwareRenderer::new(&font_rom);
    renderer.render(&inputs);
    let fb = renderer.framebuffer();
    assert_eq!(pixel_at(fb, 0, 0), [0xFF, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn text_color_takes_priority_in_monochrome_mixed_mode() {
    let mut font_rom = vec![0u8; 0x83000];
    font_rom[0x80000 + (b'A' as usize) * 16] = 0x80;

    let mut text_vram = [0u8; TEXT_VRAM_BYTES];
    text_vram[0] = b'A';
    text_vram[0x2000] = (2 << 5) | 0x01;

    let mut graphics_b = [0u8; 0x8000];
    graphics_b[0] = 0x40;

    let mut inputs = base_inputs(
        &text_vram,
        &graphics_b,
        &ZERO_PLANE,
        &ZERO_PLANE,
        &ZERO_PLANE,
    );
    inputs.global_enabled = true;
    inputs.graphics_enabled = true;
    inputs.text_enabled = true;
    inputs.video_mode = 0x02 | 0x08;
    inputs.gdc_text_pitch = 80;
    inputs.gdc_scroll_start_line[0] = 400 << 16;
    inputs.crtc_pl_bl = 15 << 16;
    inputs.crtc_cl_ssl = 16;
    inputs.gdc_graphics_pitch = 80;
    inputs.gdc_graphics_scroll[0] = 400 << 16;
    gdc_mut(&mut inputs).monochrome_mask = 0x0000_0002;
    inputs.palette_rgba[0] = 0xFF00_0000;
    inputs.palette_rgba[2] = 0xFF00_00FF;

    let mut renderer = SoftwareRenderer::new(&font_rom);
    renderer.render(&inputs);
    let fb = renderer.framebuffer();
    assert_eq!(
        pixel_at(fb, 0, 0),
        [0xFF, 0x00, 0x00, 0xFF],
        "text 'A' bit 0 lit, color 2 red"
    );
    assert_eq!(pixel_at(fb, 1, 0), [0xFF, 0x00, 0x00, 0xFF]);
    assert_eq!(
        pixel_at(fb, 2, 0),
        [0x00, 0x00, 0x00, 0xFF],
        "no graphics, no text"
    );
}

#[test]
fn renders_analog_16_color_using_extended_plane() {
    let font_rom = vec![0u8; 0x83000];

    let text_vram: [u8; TEXT_VRAM_BYTES] = [0u8; TEXT_VRAM_BYTES];
    let mut graphics_b = [0u8; 0x8000];
    let mut graphics_e = [0u8; 0x8000];
    graphics_b[0] = 0x80;
    graphics_e[0] = 0x80;

    let mut inputs = base_inputs(
        &text_vram,
        &graphics_b,
        &ZERO_PLANE,
        &ZERO_PLANE,
        &graphics_e,
    );
    inputs.global_enabled = true;
    inputs.graphics_enabled = true;
    gdc_mut(&mut inputs).is_16_color = true;
    inputs.gdc_graphics_pitch = 80;
    inputs.gdc_graphics_scroll[0] = 400 << 16;
    inputs.palette_rgba[9] = 0xFF11_2233;

    let mut renderer = SoftwareRenderer::new(&font_rom);
    renderer.render(&inputs);
    let fb = renderer.framebuffer();
    assert_eq!(
        pixel_at(fb, 0, 0),
        [0x33, 0x22, 0x11, 0xFF],
        "B+E -> graphics_color 9, palette[9]"
    );
}

#[test]
fn renders_text_in_width40_mode_doubles_pixels() {
    let mut font_rom = vec![0u8; 0x83000];
    font_rom[0x80000 + (b'A' as usize) * 16] = 0x80;

    let mut text_vram = [0u8; TEXT_VRAM_BYTES];
    text_vram[0] = b'A';
    text_vram[0x2000] = (2 << 5) | 0x01;

    let mut inputs = base_inputs(
        &text_vram,
        &ZERO_PLANE,
        &ZERO_PLANE,
        &ZERO_PLANE,
        &ZERO_PLANE,
    );
    inputs.global_enabled = true;
    inputs.text_enabled = true;
    gdc_mut(&mut inputs).is_16_color = true;
    inputs.gdc_text_pitch = 80;
    inputs.gdc_scroll_start_line[0] = 400 << 16;
    inputs.video_mode = 0x04 | 0x08;
    inputs.crtc_pl_bl = 15 << 16;
    inputs.crtc_cl_ssl = 16;
    inputs.palette_rgba[0] = 0xFF00_0000;
    inputs.palette_rgba[2] = 0xFF00_00FF;

    let mut renderer = SoftwareRenderer::new(&font_rom);
    renderer.render(&inputs);
    let fb = renderer.framebuffer();
    assert_eq!(
        pixel_at(fb, 0, 0),
        [0xFF, 0x00, 0x00, 0xFF],
        "width-40: glyph_x 0 covers pixel 0"
    );
    assert_eq!(
        pixel_at(fb, 1, 0),
        [0xFF, 0x00, 0x00, 0xFF],
        "width-40: glyph_x 0 also covers pixel 1"
    );
    assert_eq!(
        pixel_at(fb, 2, 0),
        [0x00, 0x00, 0x00, 0xFF],
        "width-40: glyph_x 1 (font bit clear) covers pixel 2"
    );
}

#[test]
fn underline_attribute_lights_right_half_of_underline_scanline() {
    let font_rom = vec![0u8; 0x83000];

    let mut text_vram = [0u8; TEXT_VRAM_BYTES];
    text_vram[0] = b' ';
    text_vram[0x2000] = (1 << 5) | 0x09;

    let mut inputs = base_inputs(
        &text_vram,
        &ZERO_PLANE,
        &ZERO_PLANE,
        &ZERO_PLANE,
        &ZERO_PLANE,
    );
    inputs.global_enabled = true;
    inputs.text_enabled = true;
    gdc_mut(&mut inputs).is_16_color = true;
    inputs.gdc_text_pitch = 80;
    inputs.gdc_scroll_start_line[0] = 400 << 16;
    inputs.video_mode = 0x08;
    inputs.crtc_pl_bl = 15 << 16;
    inputs.crtc_cl_ssl = 16;
    inputs.palette_rgba[0] = 0xFF00_0000;
    inputs.palette_rgba[1] = 0xFFFF_0000;

    let mut renderer = SoftwareRenderer::new(&font_rom);
    renderer.render(&inputs);
    let fb = renderer.framebuffer();
    let underline_y = 15;
    assert_eq!(
        pixel_at(fb, 3, underline_y),
        [0x00, 0x00, 0x00, 0xFF],
        "left half of underline scanline stays off"
    );
    assert_eq!(
        pixel_at(fb, 4, underline_y),
        [0x00, 0x00, 0xFF, 0xFF],
        "right half of underline scanline lit in cell color"
    );
    assert_eq!(
        pixel_at(fb, 0, 14),
        [0x00, 0x00, 0x00, 0xFF],
        "non-underline scanline stays off"
    );
}

#[test]
fn pegc_renders_palette_indexed_pixel() {
    let font_rom = vec![0u8; 0x83000];

    let text_vram: [u8; TEXT_VRAM_BYTES] = [0u8; TEXT_VRAM_BYTES];
    let mut pegc_vram = vec![0u8; 0x80000];
    pegc_vram[5] = 0x42;

    let palette_256 = {
        let mut palette = [0u32; 256];
        palette[0] = 0xFF00_0000;
        palette[0x42] = 0xFFAB_CDEF;
        palette
    };

    let pegc = PegcRenderInputs {
        palette_rgba_256: palette_256,
        pegc_flags: 0x02,
        vram: &pegc_vram,
    };

    let mut inputs = base_inputs(
        &text_vram,
        &ZERO_PLANE,
        &ZERO_PLANE,
        &ZERO_PLANE,
        &ZERO_PLANE,
    );
    inputs.global_enabled = true;
    inputs.graphics_enabled = true;
    inputs.gdc_graphics_pitch = 0;
    inputs.gdc_graphics_scroll[0] = 480 << 16;
    inputs.gdc_graphics_al = 480;
    inputs.graphics = GraphicsInput::Pegc(Box::new(pegc));

    let mut renderer = SoftwareRenderer::new(&font_rom);
    renderer.render(&inputs);
    let fb = renderer.framebuffer();
    assert_eq!(
        pixel_at(fb, 5, 0),
        [0xEF, 0xCD, 0xAB, 0xFF],
        "pegc lookup of vram byte 0x42 in palette_256"
    );
    assert_eq!(
        pixel_at(fb, 0, 0),
        [0x00, 0x00, 0x00, 0xFF],
        "pegc vram byte 0 is palette index 0 (default 0xFF000000)"
    );
}

#[test]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn scalar_and_simd_paths_match() {
    #[cfg(target_arch = "x86_64")]
    if !std::is_x86_feature_detected!("avx2") {
        return;
    }

    let font_rom = build_diagonal_font_rom();
    let text_vram = build_dense_text_vram();
    let (graphics_b, graphics_r, graphics_g, graphics_e) = build_dense_graphics_planes();

    let palette_rgba = build_palette_16();
    let palette_rgba_256 = build_palette_256();

    let mut pegc_vram = vec![0u8; 0x80000];
    for (i, b) in pegc_vram.iter_mut().enumerate() {
        *b = ((i * 7) & 0xFF) as u8;
    }

    let cases: [(&str, ParityCase); 8] = [
        (
            "digital_8color_text_graphics",
            ParityCase {
                global_enabled: true,
                text_enabled: true,
                graphics_enabled: true,
                is_16_color: false,
                video_mode: 0x08,
                graphics_monochrome_mask: 0,
                pegc: false,
            },
        ),
        (
            "digital_16color_text_graphics",
            ParityCase {
                global_enabled: true,
                text_enabled: true,
                graphics_enabled: true,
                is_16_color: true,
                video_mode: 0x08,
                graphics_monochrome_mask: 0,
                pegc: false,
            },
        ),
        (
            "monochrome_text_graphics",
            ParityCase {
                global_enabled: true,
                text_enabled: true,
                graphics_enabled: true,
                is_16_color: false,
                video_mode: 0x02 | 0x08,
                graphics_monochrome_mask: 0x0000_F0F0,
                pegc: false,
            },
        ),
        (
            "monochrome_graphics_only",
            ParityCase {
                global_enabled: true,
                text_enabled: false,
                graphics_enabled: true,
                is_16_color: false,
                video_mode: 0x02 | 0x08,
                graphics_monochrome_mask: 0x0000_AAAA,
                pegc: false,
            },
        ),
        (
            "width40_text_graphics",
            ParityCase {
                global_enabled: true,
                text_enabled: true,
                graphics_enabled: true,
                is_16_color: false,
                video_mode: 0x04 | 0x08,
                graphics_monochrome_mask: 0,
                pegc: false,
            },
        ),
        (
            "graphics_only_no_text",
            ParityCase {
                global_enabled: true,
                text_enabled: false,
                graphics_enabled: true,
                is_16_color: false,
                video_mode: 0x08,
                graphics_monochrome_mask: 0,
                pegc: false,
            },
        ),
        (
            "pegc_text_and_graphics",
            ParityCase {
                global_enabled: true,
                text_enabled: true,
                graphics_enabled: true,
                is_16_color: false,
                video_mode: 0x08,
                graphics_monochrome_mask: 0,
                pegc: true,
            },
        ),
        (
            "pegc_graphics_only",
            ParityCase {
                global_enabled: true,
                text_enabled: false,
                graphics_enabled: true,
                is_16_color: false,
                video_mode: 0x08,
                graphics_monochrome_mask: 0,
                pegc: true,
            },
        ),
    ];

    for (name, case) in cases {
        let mut inputs = base_inputs(
            &text_vram,
            &graphics_b,
            &graphics_r,
            &graphics_g,
            &graphics_e,
        );
        inputs.global_enabled = case.global_enabled;
        inputs.text_enabled = case.text_enabled;
        inputs.graphics_enabled = case.graphics_enabled;
        inputs.video_mode = case.video_mode;
        inputs.gdc_text_pitch = 80;
        inputs.gdc_scroll_start_line[0] = 400 << 16;
        inputs.gdc_graphics_pitch = 80;
        inputs.gdc_graphics_scroll[0] = if case.pegc { 480 << 16 } else { 400 << 16 };
        inputs.gdc_graphics_al = if case.pegc { 480 } else { 400 };
        inputs.crtc_pl_bl = 15 << 16;
        inputs.crtc_cl_ssl = 16;
        inputs.cursor_visible = true;
        inputs.cursor_addr = 5;
        inputs.cursor_top = 0;
        inputs.cursor_bottom = 15;
        inputs.palette_rgba = palette_rgba;
        inputs.graphics = match case.pegc {
            false => GraphicsInput::Gdc(GdcGraphicsInput {
                b_plane: &graphics_b,
                r_plane: &graphics_r,
                g_plane: &graphics_g,
                e_plane: &graphics_e,
                lines_per_row: 1,
                zoom_display: 0,
                monochrome_mask: case.graphics_monochrome_mask,
                is_16_color: case.is_16_color,
            }),
            true => GraphicsInput::Pegc(Box::new(PegcRenderInputs {
                palette_rgba_256,
                pegc_flags: 0x02,
                vram: &pegc_vram,
            })),
        };

        let mut renderer_simd = SoftwareRenderer::new(&font_rom);
        renderer_simd.set_simd_enabled(true);
        renderer_simd.render(&inputs);
        let simd_frame: Vec<u8> = renderer_simd.framebuffer().to_vec();

        let mut renderer_scalar = SoftwareRenderer::new(&font_rom);
        renderer_scalar.set_simd_enabled(false);
        renderer_scalar.render(&inputs);
        let scalar_frame: Vec<u8> = renderer_scalar.framebuffer().to_vec();

        if scalar_frame != simd_frame {
            let differing_pixel = (0..SoftwareRenderer::PIXEL_COUNT).find(|i| {
                let off = i * 4;
                scalar_frame[off..off + 4] != simd_frame[off..off + 4]
            });
            if let Some(i) = differing_pixel {
                let off = i * 4;
                let x = i % SoftwareRenderer::WIDTH;
                let y = i / SoftwareRenderer::WIDTH;
                panic!(
                    "{name}: scalar/simd framebuffer mismatch at pixel ({x},{y}): \
                     scalar={:?}, simd={:?}",
                    &scalar_frame[off..off + 4],
                    &simd_frame[off..off + 4],
                );
            } else {
                panic!("{name}: framebuffer length mismatch?");
            }
        }
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
struct ParityCase {
    global_enabled: bool,
    text_enabled: bool,
    graphics_enabled: bool,
    is_16_color: bool,
    video_mode: u32,
    graphics_monochrome_mask: u32,
    pegc: bool,
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn build_diagonal_font_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 0x83000];
    for byte in 0..256 {
        let glyph_offset = 0x80000 + byte * 16;
        for line in 0..16 {
            rom[glyph_offset + line] = ((byte as u32 + line as u32) & 0xFF) as u8;
        }
    }
    for index in 0..(0x80000 / 16) {
        let glyph_offset = index * 16;
        for line in 0..16 {
            rom[glyph_offset + line] = ((index ^ line) & 0xFF) as u8;
        }
    }
    rom
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn build_dense_text_vram() -> [u8; TEXT_VRAM_BYTES] {
    let mut vram = [0u8; TEXT_VRAM_BYTES];
    for cell in 0..(80 * 25) {
        let low = ((cell * 13) & 0xFF) as u8;
        let high = ((cell * 17) & 0x7F) as u8;
        let attr = (((cell as u32) << 4) | (cell as u32 & 0x0F)) as u8;
        vram[cell * 2] = low;
        vram[cell * 2 + 1] = high;
        vram[cell * 2 + 0x2000] = attr;
    }
    vram
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn build_dense_graphics_planes() -> ([u8; 0x8000], [u8; 0x8000], [u8; 0x8000], [u8; 0x8000]) {
    let mut b = [0u8; 0x8000];
    let mut r = [0u8; 0x8000];
    let mut g = [0u8; 0x8000];
    let mut e = [0u8; 0x8000];
    for i in 0..0x8000 {
        b[i] = ((i * 3) & 0xFF) as u8;
        r[i] = ((i * 5) & 0xFF) as u8;
        g[i] = ((i * 7) & 0xFF) as u8;
        e[i] = ((i * 11) & 0xFF) as u8;
    }
    (b, r, g, e)
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn build_palette_16() -> [u32; 16] {
    let mut palette = [0u32; 16];
    for (i, slot) in palette.iter_mut().enumerate() {
        // Force every entry away from `BLACK = 0xFF00_0000`; in particular
        // `palette[0]` must be non-black so the AVX2 monochrome fallback
        // (which uses `palette_rgba[0]` per the scalar `(false, 0)` branch)
        // can be distinguished from a stale BLACK constant.
        let r = ((i as u32 * 17) & 0xFE) | 0x10;
        let g = ((i as u32 * 31) & 0xFE) | 0x20;
        let b = ((i as u32 * 53) & 0xFE) | 0x40;
        *slot = r | (g << 8) | (b << 16) | 0xFF00_0000;
    }
    palette
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn build_palette_256() -> [u32; 256] {
    let mut palette = [0u32; 256];
    for (i, slot) in palette.iter_mut().enumerate() {
        let r = (i as u32 * 11) & 0xFF;
        let g = (i as u32 * 13) & 0xFF;
        let b = (i as u32 * 17) & 0xFF;
        *slot = r | (g << 8) | (b << 16) | 0xFF00_0000;
    }
    palette
}
