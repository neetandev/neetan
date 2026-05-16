use software_renderer::{
    Ga1280aCursorRenderInputs, Ga1280aRenderInputs, Ga1280aRenderMode, compose_ga1280a,
};

const CURSOR_MASK_BYTES: usize = 128;

fn palette_with_identity_rgb() -> Box<[[u8; 3]; 256]> {
    let mut palette = Box::new([[0; 3]; 256]);
    for (index, entry) in palette.iter_mut().enumerate() {
        let value = index as u8;
        *entry = [value, value.wrapping_add(1), value.wrapping_add(2)];
    }
    palette
}

fn transparent_cursor<'a>(
    and_pattern: &'a [u8; CURSOR_MASK_BYTES],
    xor_pattern: &'a [u8; CURSOR_MASK_BYTES],
) -> Ga1280aCursorRenderInputs<'a> {
    Ga1280aCursorRenderInputs {
        visible: false,
        x: 0,
        y: 0,
        colors: [[0; 3]; 2],
        xor_pattern,
        and_pattern,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_inputs<'a>(
    mode: Ga1280aRenderMode,
    width: u32,
    height: u32,
    pixel_map_width: u32,
    pixel_map_height: u32,
    stride_bytes: u32,
    display_offset_pixels: u64,
    visible_mask: u8,
    palette: &'a [[u8; 3]; 256],
    vram: &'a [u8],
    cursor: Ga1280aCursorRenderInputs<'a>,
) -> Ga1280aRenderInputs<'a> {
    Ga1280aRenderInputs {
        mode,
        width,
        height,
        pixel_map_width,
        pixel_map_height,
        stride_bytes,
        display_offset_pixels,
        palette,
        visible_mask,
        vram,
        cursor,
    }
}

fn render_frame(inputs: &Ga1280aRenderInputs<'_>, simd: bool) -> Vec<u8> {
    let mut framebuffer = vec![0u8; (inputs.width as usize) * (inputs.height as usize) * 4];
    compose_ga1280a(&mut framebuffer, inputs, simd);
    framebuffer
}

fn pixel(framebuffer: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * width + x) * 4;
    [
        framebuffer[offset],
        framebuffer[offset + 1],
        framebuffer[offset + 2],
        framebuffer[offset + 3],
    ]
}

#[test]
fn indexed8_non_identity_display_start_wraps() {
    let palette = palette_with_identity_rgb();
    let and_pattern = [0xFF; CURSOR_MASK_BYTES];
    let xor_pattern = [0; CURSOR_MASK_BYTES];
    let mut vram = vec![0u8; 4 * 4];
    for (index, byte) in vram.iter_mut().enumerate() {
        *byte = index as u8;
    }

    let cursor = transparent_cursor(&and_pattern, &xor_pattern);
    let inputs = render_inputs(
        Ga1280aRenderMode::Indexed8,
        3,
        2,
        4,
        4,
        4,
        5,
        0xFF,
        &palette,
        &vram,
        cursor,
    );
    let frame = render_frame(&inputs, false);

    assert_eq!(pixel(&frame, 3, 0, 0), [5, 6, 7, 0xFF]);
    assert_eq!(pixel(&frame, 3, 2, 0), [7, 8, 9, 0xFF]);
    assert_eq!(pixel(&frame, 3, 0, 1), [9, 10, 11, 0xFF]);
}

#[test]
fn direct_color16_scalar_converts_rgb565() {
    let palette = palette_with_identity_rgb();
    let and_pattern = [0xFF; CURSOR_MASK_BYTES];
    let xor_pattern = [0; CURSOR_MASK_BYTES];
    let vram = [0x00, 0xF8, 0xE0, 0x07, 0x1F, 0x00];

    let cursor = transparent_cursor(&and_pattern, &xor_pattern);
    let inputs = render_inputs(
        Ga1280aRenderMode::DirectColor16,
        3,
        1,
        3,
        1,
        6,
        0,
        0xFF,
        &palette,
        &vram,
        cursor,
    );
    let frame = render_frame(&inputs, false);

    assert_eq!(pixel(&frame, 3, 0, 0), [0xFF, 0x00, 0x00, 0xFF]);
    assert_eq!(pixel(&frame, 3, 1, 0), [0x00, 0xFF, 0x00, 0xFF]);
    assert_eq!(pixel(&frame, 3, 2, 0), [0x00, 0x00, 0xFF, 0xFF]);
}

#[test]
fn full_color24_scalar_converts_bgr_to_rgba() {
    let palette = palette_with_identity_rgb();
    let and_pattern = [0xFF; CURSOR_MASK_BYTES];
    let xor_pattern = [0; CURSOR_MASK_BYTES];
    let vram = [0x30, 0x20, 0x10, 0xCC, 0xBB, 0xAA];

    let cursor = transparent_cursor(&and_pattern, &xor_pattern);
    let inputs = render_inputs(
        Ga1280aRenderMode::FullColor24,
        2,
        1,
        2,
        1,
        6,
        0,
        0xFF,
        &palette,
        &vram,
        cursor,
    );
    let frame = render_frame(&inputs, false);

    assert_eq!(pixel(&frame, 2, 0, 0), [0x10, 0x20, 0x30, 0xFF]);
    assert_eq!(pixel(&frame, 2, 1, 0), [0xAA, 0xBB, 0xCC, 0xFF]);
}

#[test]
fn full_color24_non_identity_display_start_converts_bgr_to_rgba() {
    let palette = palette_with_identity_rgb();
    let and_pattern = [0xFF; CURSOR_MASK_BYTES];
    let xor_pattern = [0; CURSOR_MASK_BYTES];
    let vram = [
        0x03, 0x02, 0x01, 0x13, 0x12, 0x11, 0x23, 0x22, 0x21, 0x33, 0x32, 0x31,
    ];

    let cursor = transparent_cursor(&and_pattern, &xor_pattern);
    let inputs = render_inputs(
        Ga1280aRenderMode::FullColor24,
        2,
        1,
        4,
        1,
        12,
        1,
        0xFF,
        &palette,
        &vram,
        cursor,
    );
    let frame = render_frame(&inputs, false);

    assert_eq!(pixel(&frame, 2, 0, 0), [0x11, 0x12, 0x13, 0xFF]);
    assert_eq!(pixel(&frame, 2, 1, 0), [0x21, 0x22, 0x23, 0xFF]);
}

#[test]
fn hardware_cursor_overlays_all_mask_states() {
    let mut palette = palette_with_identity_rgb();
    palette[1] = [0x10, 0x20, 0x30];
    let mut and_pattern = [0xFF; CURSOR_MASK_BYTES];
    let mut xor_pattern = [0; CURSOR_MASK_BYTES];
    and_pattern[0] &= !0x40;
    and_pattern[0] &= !0x20;
    xor_pattern[0] |= 0x20;
    xor_pattern[0] |= 0x10;
    let vram = vec![1u8; 8];

    let cursor = Ga1280aCursorRenderInputs {
        visible: true,
        x: 0x20,
        y: 0x20,
        colors: [[0x80, 0x10, 0x20], [0xF0, 0xE0, 0x10]],
        xor_pattern: &xor_pattern,
        and_pattern: &and_pattern,
    };
    let inputs = render_inputs(
        Ga1280aRenderMode::Indexed8,
        8,
        1,
        8,
        1,
        8,
        0,
        0xFF,
        &palette,
        &vram,
        cursor,
    );
    let frame = render_frame(&inputs, false);

    assert_eq!(pixel(&frame, 8, 0, 0), [0x10, 0x20, 0x30, 0xFF]);
    assert_eq!(pixel(&frame, 8, 1, 0), [0x80, 0x10, 0x20, 0xFF]);
    assert_eq!(pixel(&frame, 8, 2, 0), [0xF0, 0xE0, 0x10, 0xFF]);
    assert_eq!(pixel(&frame, 8, 3, 0), [0xEF, 0xDF, 0xCF, 0xFF]);
}

#[cfg(target_arch = "x86_64")]
fn assert_direct_color16_simd_matches_scalar(
    width: u32,
    height: u32,
    pixel_map_width: u32,
    stride_bytes: u32,
) {
    if !is_x86_feature_detected!("avx2") {
        return;
    }

    let palette = palette_with_identity_rgb();
    let and_pattern = [0xFF; CURSOR_MASK_BYTES];
    let xor_pattern = [0; CURSOR_MASK_BYTES];
    let mut vram = vec![0u8; (stride_bytes * height) as usize];
    for (index, byte) in vram.iter_mut().enumerate() {
        *byte = ((index * 53 + index / 11) & 0xFF) as u8;
    }

    let cursor = transparent_cursor(&and_pattern, &xor_pattern);
    let inputs = render_inputs(
        Ga1280aRenderMode::DirectColor16,
        width,
        height,
        pixel_map_width,
        height,
        stride_bytes,
        0,
        0xFF,
        &palette,
        &vram,
        cursor,
    );

    let scalar = render_frame(&inputs, false);
    let simd = render_frame(&inputs, true);
    assert_eq!(simd, scalar);
}

#[test]
#[cfg(target_arch = "x86_64")]
fn direct_color16_simd_matches_scalar_identity_1024x768() {
    assert_direct_color16_simd_matches_scalar(1024, 768, 1024, 1024 * 2);
}

#[test]
#[cfg(target_arch = "x86_64")]
fn direct_color16_simd_matches_scalar_aligned_narrow() {
    assert_direct_color16_simd_matches_scalar(64, 16, 64, 64 * 2);
}

#[test]
#[cfg(target_arch = "x86_64")]
fn direct_color16_simd_matches_scalar_tail_width() {
    assert_direct_color16_simd_matches_scalar(641, 7, 641, 641 * 2);
}

#[test]
#[cfg(target_arch = "x86_64")]
fn direct_color16_simd_matches_scalar_padded_stride() {
    assert_direct_color16_simd_matches_scalar(128, 4, 128, 512);
}

#[cfg(target_arch = "x86_64")]
fn assert_full_color24_simd_matches_scalar(
    width: u32,
    height: u32,
    pixel_map_width: u32,
    stride_bytes: u32,
) {
    if !is_x86_feature_detected!("avx2") {
        return;
    }

    let palette = palette_with_identity_rgb();
    let and_pattern = [0xFF; CURSOR_MASK_BYTES];
    let xor_pattern = [0; CURSOR_MASK_BYTES];
    let mut vram = vec![0u8; (stride_bytes * height) as usize];
    for (index, byte) in vram.iter_mut().enumerate() {
        *byte = ((index * 71 + index / 13) & 0xFF) as u8;
    }

    let cursor = transparent_cursor(&and_pattern, &xor_pattern);
    let inputs = render_inputs(
        Ga1280aRenderMode::FullColor24,
        width,
        height,
        pixel_map_width,
        height,
        stride_bytes,
        0,
        0xFF,
        &palette,
        &vram,
        cursor,
    );

    let scalar = render_frame(&inputs, false);
    let simd = render_frame(&inputs, true);
    assert_eq!(simd, scalar);
}

#[test]
#[cfg(target_arch = "x86_64")]
fn full_color24_simd_matches_scalar_identity_1024x768() {
    assert_full_color24_simd_matches_scalar(1024, 768, 1024, 1024 * 3);
}

#[test]
#[cfg(target_arch = "x86_64")]
fn full_color24_simd_matches_scalar_aligned_narrow() {
    assert_full_color24_simd_matches_scalar(64, 16, 64, 64 * 3);
}

#[test]
#[cfg(target_arch = "x86_64")]
fn full_color24_simd_matches_scalar_tail_width() {
    assert_full_color24_simd_matches_scalar(643, 5, 643, 643 * 3);
}

#[test]
#[cfg(target_arch = "x86_64")]
fn full_color24_simd_matches_scalar_last_row_boundary() {
    assert_full_color24_simd_matches_scalar(8, 4, 8, 8 * 3);
}

#[test]
#[cfg(target_arch = "x86_64")]
fn full_color24_simd_matches_scalar_padded_stride() {
    assert_full_color24_simd_matches_scalar(128, 4, 128, 512);
}
