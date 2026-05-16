//! CPU-side compose pass for I-O DATA GA-1280A display output.

#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
mod avx2;

const PIXEL_BYTES: usize = 4;
const CURSOR_WIDTH: i32 = 32;
const CURSOR_HEIGHT: i32 = 32;
const CURSOR_ROW_BYTES: usize = 4;
const CURSOR_POSITION_BIAS: i32 = 0x20;

/// Active GA-1280A framebuffer interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ga1280aRenderMode {
    /// 8-bit indexed color through the RAMDAC palette.
    Indexed8,
    /// 16-bit RGB565 direct color.
    DirectColor16,
    /// 24-bit BGR source pixels converted to RGBA output.
    FullColor24,
}

impl Ga1280aRenderMode {
    fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Indexed8 => 1,
            Self::DirectColor16 => 2,
            Self::FullColor24 => 3,
        }
    }
}

/// RAMDAC hardware cursor inputs for a GA-1280A frame.
pub struct Ga1280aCursorRenderInputs<'a> {
    /// Whether the hardware cursor is visible.
    pub visible: bool,
    /// Raw RAMDAC cursor X position.
    pub x: u16,
    /// Raw RAMDAC cursor Y position.
    pub y: u16,
    /// Cursor background and foreground colors as RGB triples.
    pub colors: [[u8; 3]; 2],
    /// Cursor XOR pattern bytes.
    pub xor_pattern: &'a [u8],
    /// Cursor AND pattern bytes.
    pub and_pattern: &'a [u8],
}

/// Per-frame inputs for GA-1280A framebuffer composition.
pub struct Ga1280aRenderInputs<'a> {
    /// Active framebuffer interpretation.
    pub mode: Ga1280aRenderMode,
    /// Visible output width in pixels.
    pub width: u32,
    /// Visible output height in pixels.
    pub height: u32,
    /// Backing pixel-map width in pixels.
    pub pixel_map_width: u32,
    /// Backing pixel-map height in pixels.
    pub pixel_map_height: u32,
    /// Backing VRAM row stride in bytes.
    pub stride_bytes: u32,
    /// Display-start offset in pixels after mode-specific CRTC unit expansion.
    pub display_offset_pixels: u64,
    /// RAMDAC palette as RGB triples.
    pub palette: &'a [[u8; 3]; 256],
    /// RAMDAC visible palette mask.
    pub visible_mask: u8,
    /// Raw packed-pixel GA VRAM.
    pub vram: &'a [u8],
    /// Hardware cursor state for this frame.
    pub cursor: Ga1280aCursorRenderInputs<'a>,
}

/// Composes one GA-1280A frame into the top-left `width * height` region of
/// `framebuffer`. Returns the `(width, height)` actually written.
///
/// Panics if `framebuffer` is smaller than `inputs.width * inputs.height * 4`
/// bytes.
pub fn compose(
    framebuffer: &mut [u8],
    inputs: &Ga1280aRenderInputs<'_>,
    has_simd: bool,
) -> (u32, u32) {
    let width = inputs.width;
    let height = inputs.height;
    let required_bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(PIXEL_BYTES))
        .expect("GA-1280A frame dimensions overflow");
    assert!(
        framebuffer.len() >= required_bytes,
        "GA-1280A framebuffer capacity {} bytes is smaller than required {} bytes for {}x{}",
        framebuffer.len(),
        required_bytes,
        width,
        height,
    );
    let framebuffer = &mut framebuffer[..required_bytes];

    let palette = build_palette_rgba(inputs.palette);
    match inputs.mode {
        Ga1280aRenderMode::Indexed8 => compose_indexed8(framebuffer, inputs, &palette),
        Ga1280aRenderMode::DirectColor16 => compose_direct_color16(framebuffer, inputs, has_simd),
        Ga1280aRenderMode::FullColor24 => compose_full_color24(framebuffer, inputs, has_simd),
    }
    compose_hardware_cursor(framebuffer, width, height, &inputs.cursor);

    (width, height)
}

/// We tried to accelrate index8 with SIMD, but we are bandwidth limited in this case and it didn't
/// improve with SIMD at all.
fn compose_indexed8(
    framebuffer: &mut [u8],
    inputs: &Ga1280aRenderInputs<'_>,
    palette: &[u32; 256],
) {
    let width = inputs.width as usize;
    let height = inputs.height as usize;
    let stride = inputs.stride_bytes as usize;
    let visible_mask = inputs.visible_mask;

    if is_linear_identity(inputs) {
        for y in 0..height {
            let row_start = y * stride;
            let row = &inputs.vram[row_start..row_start + width];
            let framebuffer_row_start = y * width * PIXEL_BYTES;
            let framebuffer_row = &mut framebuffer
                [framebuffer_row_start..framebuffer_row_start + width * PIXEL_BYTES];
            for (palette_index, pixel) in row
                .iter()
                .zip(framebuffer_row.chunks_exact_mut(PIXEL_BYTES))
            {
                let value = palette[(*palette_index & visible_mask) as usize];
                pixel.copy_from_slice(&value.to_le_bytes());
            }
        }
        return;
    }

    for y in 0..height {
        for x in 0..width {
            let color = read_displayed_pixel(inputs, x as u32, y as u32) as u8;
            let pixel = palette[(color & visible_mask) as usize];
            write_pixel_u32(framebuffer, width, x, y, pixel);
        }
    }
}

fn compose_direct_color16(
    framebuffer: &mut [u8],
    inputs: &Ga1280aRenderInputs<'_>,
    has_simd: bool,
) {
    let width = inputs.width as usize;
    let height = inputs.height as usize;
    let stride = inputs.stride_bytes as usize;

    if is_linear_identity(inputs) {
        #[cfg(target_arch = "x86_64")]
        if has_simd {
            #[allow(unsafe_code)]
            // SAFETY: `has_simd` is true only when AVX2 is available on x86_64.
            unsafe {
                avx2::compose_direct_color16_identity_avx2(
                    framebuffer,
                    inputs.vram,
                    width,
                    height,
                    stride,
                );
            }
            return;
        }
        #[cfg(not(target_arch = "x86_64"))]
        let _ = has_simd;

        compose_direct_color16_identity_scalar(framebuffer, inputs.vram, width, height, stride);
        return;
    }

    for y in 0..height {
        for x in 0..width {
            let color = read_displayed_pixel(inputs, x as u32, y as u32) as u16;
            let pixel = direct_color16_to_rgba(color);
            write_pixel_u32(framebuffer, width, x, y, pixel);
        }
    }
}

fn compose_full_color24(framebuffer: &mut [u8], inputs: &Ga1280aRenderInputs<'_>, has_simd: bool) {
    let width = inputs.width as usize;
    let height = inputs.height as usize;
    let stride = inputs.stride_bytes as usize;

    if is_linear_identity(inputs) {
        #[cfg(target_arch = "x86_64")]
        if has_simd {
            #[allow(unsafe_code)]
            // SAFETY: `has_simd` is true only when AVX2 is available on x86_64.
            unsafe {
                avx2::compose_full_color24_identity_avx2(
                    framebuffer,
                    inputs.vram,
                    width,
                    height,
                    stride,
                );
            }
            return;
        }
        #[cfg(not(target_arch = "x86_64"))]
        let _ = has_simd;

        compose_full_color24_identity_scalar(framebuffer, inputs.vram, width, height, stride);
        return;
    }

    for y in 0..height {
        for x in 0..width {
            let color = read_displayed_pixel(inputs, x as u32, y as u32);
            let pixel = full_color24_to_rgba(color);
            write_pixel_u32(framebuffer, width, x, y, pixel);
        }
    }
}

fn compose_direct_color16_identity_scalar(
    framebuffer: &mut [u8],
    vram: &[u8],
    width: usize,
    height: usize,
    stride: usize,
) {
    for y in 0..height {
        let row_start = y * stride;
        let framebuffer_row_start = y * width * PIXEL_BYTES;
        for x in 0..width {
            let pixel_offset = row_start + x * 2;
            let color = u16::from(vram[pixel_offset]) | (u16::from(vram[pixel_offset + 1]) << 8);
            let pixel = direct_color16_to_rgba(color);
            let fb_offset = framebuffer_row_start + x * PIXEL_BYTES;
            framebuffer[fb_offset..fb_offset + PIXEL_BYTES].copy_from_slice(&pixel.to_le_bytes());
        }
    }
}

fn compose_full_color24_identity_scalar(
    framebuffer: &mut [u8],
    vram: &[u8],
    width: usize,
    height: usize,
    stride: usize,
) {
    for y in 0..height {
        let row_start = y * stride;
        let framebuffer_row_start = y * width * PIXEL_BYTES;
        for x in 0..width {
            let pixel_offset = row_start + x * 3;
            let blue = vram[pixel_offset];
            let green = vram[pixel_offset + 1];
            let red = vram[pixel_offset + 2];
            let fb_offset = framebuffer_row_start + x * PIXEL_BYTES;
            framebuffer[fb_offset] = red;
            framebuffer[fb_offset + 1] = green;
            framebuffer[fb_offset + 2] = blue;
            framebuffer[fb_offset + 3] = 0xFF;
        }
    }
}

fn compose_hardware_cursor(
    framebuffer: &mut [u8],
    framebuffer_width: u32,
    framebuffer_height: u32,
    cursor: &Ga1280aCursorRenderInputs<'_>,
) {
    if !cursor.visible {
        return;
    }

    let left = i32::from(cursor.x) - CURSOR_POSITION_BIAS;
    let top = i32::from(cursor.y) - CURSOR_POSITION_BIAS;
    let width = framebuffer_width as i32;
    let height = framebuffer_height as i32;
    if left >= width || top >= height || left + CURSOR_WIDTH <= 0 || top + CURSOR_HEIGHT <= 0 {
        return;
    }

    let background = cursor.colors[0];
    let foreground = cursor.colors[1];
    for row in 0..CURSOR_HEIGHT {
        let y = top + row;
        if y < 0 || y >= height {
            continue;
        }

        for column in 0..CURSOR_WIDTH {
            let x = left + column;
            if x < 0 || x >= width {
                continue;
            }

            let mask_index = row as usize * CURSOR_ROW_BYTES + column as usize / 8;
            let bit = 0x80 >> (column as usize & 7);
            let and_bit = cursor
                .and_pattern
                .get(mask_index)
                .is_some_and(|value| value & bit != 0);
            let xor_bit = cursor
                .xor_pattern
                .get(mask_index)
                .is_some_and(|value| value & bit != 0);

            match (and_bit, xor_bit) {
                (true, false) => {}
                (false, false) => write_framebuffer_pixel(
                    framebuffer,
                    framebuffer_width,
                    x as u32,
                    y as u32,
                    background[0],
                    background[1],
                    background[2],
                ),
                (false, true) => write_framebuffer_pixel(
                    framebuffer,
                    framebuffer_width,
                    x as u32,
                    y as u32,
                    foreground[0],
                    foreground[1],
                    foreground[2],
                ),
                (true, true) => {
                    invert_framebuffer_pixel(framebuffer, framebuffer_width, x as u32, y as u32)
                }
            }
        }
    }
}

fn invert_framebuffer_pixel(framebuffer: &mut [u8], framebuffer_width: u32, x: u32, y: u32) {
    let offset = ((y * framebuffer_width + x) * PIXEL_BYTES as u32) as usize;
    framebuffer[offset] = 0xFF - framebuffer[offset];
    framebuffer[offset + 1] = 0xFF - framebuffer[offset + 1];
    framebuffer[offset + 2] = 0xFF - framebuffer[offset + 2];
    framebuffer[offset + 3] = 0xFF;
}

fn write_framebuffer_pixel(
    framebuffer: &mut [u8],
    framebuffer_width: u32,
    x: u32,
    y: u32,
    red: u8,
    green: u8,
    blue: u8,
) {
    let offset = ((y * framebuffer_width + x) * PIXEL_BYTES as u32) as usize;
    framebuffer[offset] = red;
    framebuffer[offset + 1] = green;
    framebuffer[offset + 2] = blue;
    framebuffer[offset + 3] = 0xFF;
}

fn is_linear_identity(inputs: &Ga1280aRenderInputs<'_>) -> bool {
    let bytes_per_pixel = inputs.mode.bytes_per_pixel() as u32;
    inputs.display_offset_pixels == 0
        && inputs.pixel_map_width >= inputs.width
        && inputs.pixel_map_height >= inputs.height
        && inputs.stride_bytes >= inputs.pixel_map_width.saturating_mul(bytes_per_pixel)
        && (inputs.height as usize)
            .checked_mul(inputs.stride_bytes as usize)
            .is_some_and(|end| end <= inputs.vram.len())
}

fn read_displayed_pixel(inputs: &Ga1280aRenderInputs<'_>, x: u32, y: u32) -> u32 {
    let pixel_map_width = inputs.pixel_map_width;
    let pixel_map_height = inputs.pixel_map_height;
    if pixel_map_width == 0 || pixel_map_height == 0 {
        return 0;
    }

    let offset =
        inputs.display_offset_pixels + u64::from(y) * u64::from(pixel_map_width) + u64::from(x);
    let source_x = (offset % u64::from(pixel_map_width)) as u32;
    let source_y = ((offset / u64::from(pixel_map_width)) % u64::from(pixel_map_height)) as u32;
    read_packed_pixel(inputs, source_x, source_y)
}

fn read_packed_pixel(inputs: &Ga1280aRenderInputs<'_>, x: u32, y: u32) -> u32 {
    if x >= inputs.pixel_map_width || y >= inputs.pixel_map_height {
        return 0;
    }

    let bytes_per_pixel = inputs.mode.bytes_per_pixel();
    let offset = (y as usize) * (inputs.stride_bytes as usize) + (x as usize) * bytes_per_pixel;
    if offset + bytes_per_pixel > inputs.vram.len() {
        return 0;
    }

    match inputs.mode {
        Ga1280aRenderMode::Indexed8 => u32::from(inputs.vram[offset]),
        Ga1280aRenderMode::DirectColor16 => {
            u32::from(inputs.vram[offset]) | (u32::from(inputs.vram[offset + 1]) << 8)
        }
        Ga1280aRenderMode::FullColor24 => {
            u32::from(inputs.vram[offset])
                | (u32::from(inputs.vram[offset + 1]) << 8)
                | (u32::from(inputs.vram[offset + 2]) << 16)
        }
    }
}

fn build_palette_rgba(palette: &[[u8; 3]; 256]) -> [u32; 256] {
    let mut result = [0u32; 256];
    for (slot, [red, green, blue]) in result.iter_mut().zip(palette.iter()) {
        *slot = u32::from(*red) | (u32::from(*green) << 8) | (u32::from(*blue) << 16) | 0xFF00_0000;
    }
    result
}

const fn expand_5_to_8(value: u8) -> u8 {
    (value << 3) | (value >> 2)
}

const fn expand_6_to_8(value: u8) -> u8 {
    (value << 2) | (value >> 4)
}

const fn direct_color16_to_rgba(color: u16) -> u32 {
    let red = expand_5_to_8(((color >> 11) & 0x1F) as u8);
    let green = expand_6_to_8(((color >> 5) & 0x3F) as u8);
    let blue = expand_5_to_8((color & 0x1F) as u8);
    (red as u32) | ((green as u32) << 8) | ((blue as u32) << 16) | 0xFF00_0000
}

const fn full_color24_to_rgba(color: u32) -> u32 {
    let blue = color & 0x0000_00FF;
    let green = color & 0x0000_FF00;
    let red = color & 0x00FF_0000;
    (red >> 16) | green | (blue << 16) | 0xFF00_0000
}

fn write_pixel_u32(framebuffer: &mut [u8], width: usize, x: usize, y: usize, pixel: u32) {
    let offset = (y * width + x) * PIXEL_BYTES;
    framebuffer[offset..offset + PIXEL_BYTES].copy_from_slice(&pixel.to_le_bytes());
}
