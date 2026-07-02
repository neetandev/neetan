//! FM Towns per-layer decode and blit.
//!
//! Each layer is sampled from native packed VRAM in its color depth (4-bit
//! packed, 8-bit palette-indexed, or 16-bit direct color), scaled by the CRTC
//! zoom factor, and placed at its on-monitor origin. The scroll offset is
//! split into a horizontal part that wraps within one scanline and a vertical
//! part that wraps within the layer's VRAM page; fetching stops at the end of
//! the programmed line length. Single-page mode reads VRAM through the
//! interleaved bank transform.

use super::{RenderInputsTowns, TOWNS_PIXEL_BYTES, TownsLayer, palette};

/// The two 256 KiB VRAM banks are interleaved at 4-byte granularity in
/// single-page mode; this matches the CPU-side single-page window transform.
fn interleaved_offset(offset: usize) -> usize {
    ((offset & 4) << 16) | ((offset & 0x7_FFF8) >> 1) | (offset & 3)
}

/// The high-res single-page interleave: the same 4-byte swizzle as the low-res
/// path plus the 0x80000 page-select bit, matching the CPU-side 0x83000000
/// window transform.
fn high_res_interleaved_offset(offset: usize) -> usize {
    (offset & 0x8_0000) | interleaved_offset(offset)
}

/// Reads one native VRAM byte, applying the single-page interleave when active
/// (the high-res variant when the high-res CRTC is driving the frame).
fn fetch(vram: &[u8], offset: usize, single_page: bool, high_res: bool) -> u8 {
    let index = if single_page {
        if high_res {
            high_res_interleaved_offset(offset)
        } else {
            interleaved_offset(offset)
        }
    } else {
        offset
    };
    let mask = vram.len().wrapping_sub(1);
    vram[index & mask]
}

/// Draws one layer into the framebuffer. When `transparent` is set (the priority
/// page over a lower page), transparent pixels are skipped; otherwise every pixel
/// is written opaquely.
pub(super) fn draw_layer(
    framebuffer: &mut [u8],
    inputs: &RenderInputsTowns<'_>,
    layer: &TownsLayer,
    frame_width: usize,
    frame_height: usize,
    transparent: bool,
) {
    if !layer.shown || layer.width == 0 || layer.height == 0 {
        return;
    }

    let vram = inputs.vram;
    let palette16 = &inputs.palette_16[usize::from(layer.palette_bank) & 1];
    let palette256 = &inputs.palette_256;
    let row_bytes = frame_width * TOWNS_PIXEL_BYTES;

    let height = layer
        .height
        .min(frame_height.saturating_sub(layer.origin_y));
    let width = layer.width.min(frame_width.saturating_sub(layer.origin_x));
    let zoom_x = usize::from(layer.zoom_x).max(2);
    let zoom_y = usize::from(layer.zoom_y.max(1));
    let zoom_repeat = [zoom_x / 2, zoom_x.div_ceil(2)];
    let offset_vertical = layer.scroll_offset & !layer.h_scroll_mask;
    let offset_horizontal = layer.scroll_offset & layer.h_scroll_mask;
    let line_limit = match layer.bits_per_pixel {
        8 => layer.bytes_per_line,
        16 => layer.bytes_per_line + layer.origin_x * 2,
        24 => layer.bytes_per_line + layer.origin_x * 3,
        _ => usize::MAX,
    };

    for monitor_y in 0..height {
        let source_y = (monitor_y * 2) / zoom_y;
        let line_vram_offset = source_y * layer.bytes_per_line;
        // The 4 bpp path masks the whole line-start offset vertically and
        // reads the line linearly; 8/16 bpp wrap each fetch horizontally
        // within the line and vertically within the page.
        let line_start_4bpp = layer.vram_addr
            + layer.vram_h_skip_bytes
            + ((layer.scroll_offset + line_vram_offset) & layer.v_scroll_mask);
        let out_row = &mut framebuffer[(layer.origin_y + monitor_y) * row_bytes..][..row_bytes];
        let mut in_line_offset = if layer.bits_per_pixel == 4 {
            0
        } else {
            layer.vram_h_skip_bytes
        };
        let mut high_nibble = false;
        let mut zoom_phase = 0;
        let mut zoom_remaining = zoom_repeat[0];
        for monitor_x in 0..width {
            if in_line_offset >= line_limit {
                break;
            }
            let address = match layer.bits_per_pixel {
                4 => line_start_4bpp + in_line_offset,
                _ => {
                    let line_address = line_vram_offset
                        + ((in_line_offset + offset_horizontal) & layer.h_scroll_mask);
                    layer.vram_addr + ((line_address + offset_vertical) & layer.v_scroll_mask)
                }
            };
            let color = sample_pixel(
                vram,
                layer,
                palette16,
                palette256,
                inputs.single_page,
                inputs.high_res,
                address,
                high_nibble,
                transparent,
            );
            if let Some(color) = color {
                let base = (layer.origin_x + monitor_x) * TOWNS_PIXEL_BYTES;
                out_row[base] = color as u8;
                out_row[base + 1] = (color >> 8) as u8;
                out_row[base + 2] = (color >> 16) as u8;
                out_row[base + 3] = (color >> 24) as u8;
            }
            zoom_remaining -= 1;
            if zoom_remaining == 0 {
                zoom_phase ^= 1;
                zoom_remaining = zoom_repeat[zoom_phase];
                match layer.bits_per_pixel {
                    4 => {
                        if high_nibble {
                            in_line_offset += 1;
                        }
                        high_nibble = !high_nibble;
                    }
                    8 => in_line_offset += 1,
                    16 => in_line_offset += 2,
                    24 => in_line_offset += 3,
                    _ => {}
                }
            }
        }
    }
}

/// Samples the source pixel at `address`, returning its RGBA color or `None`
/// when the pixel is transparent and `transparent` sampling is requested.
#[allow(clippy::too_many_arguments)]
fn sample_pixel(
    vram: &[u8],
    layer: &TownsLayer,
    palette16: &[u32; 16],
    palette256: &[u32; 256],
    single_page: bool,
    high_res: bool,
    address: usize,
    high_nibble: bool,
    transparent: bool,
) -> Option<u32> {
    match layer.bits_per_pixel {
        4 => {
            let byte = fetch(vram, address, single_page, high_res);
            let nibble = if high_nibble { byte >> 4 } else { byte & 0x0F };
            let index = nibble & layer.plane_mask & 0x0F;
            if transparent && index == 0 {
                return None;
            }
            Some(palette16[usize::from(index)])
        }
        8 => {
            let byte = fetch(vram, address, single_page, high_res);
            if transparent && byte == 0 {
                return None;
            }
            Some(palette256[usize::from(byte)])
        }
        16 => {
            let low = fetch(vram, address, single_page, high_res);
            let high = fetch(vram, address + 1, single_page, high_res);
            let color = u16::from(low) | (u16::from(high) << 8);
            if transparent && color & 0x8000 != 0 {
                return None;
            }
            Some(palette::towns_color15_to_rgba(color))
        }
        24 => Some(sample_24bpp(vram, layer, single_page, high_res, address)),
        _ => None,
    }
}

/// The high-res 24bpp direct-color value that means "no reorder": R/G/B come
/// from source bytes 0/1/2 in order.
const RGB_SWAP_IDENTITY: u8 = 0x06;

/// Decodes one 24bpp (16M-color) pixel: three VRAM bytes reordered by the
/// layer's `high_res_rgb_swap` (each 2-bit field picks the source byte for the
/// R/G/B channel). Opaque; 24bpp mode has no transparency key.
fn sample_24bpp(
    vram: &[u8],
    layer: &TownsLayer,
    single_page: bool,
    high_res: bool,
    address: usize,
) -> u32 {
    let bytes = [
        fetch(vram, address, single_page, high_res),
        fetch(vram, address + 1, single_page, high_res),
        fetch(vram, address + 2, single_page, high_res),
    ];
    let (red, green, blue) = if layer.high_res_rgb_swap == RGB_SWAP_IDENTITY {
        (bytes[0], bytes[1], bytes[2])
    } else {
        let swap = layer.high_res_rgb_swap;
        let red_index = usize::from((swap >> 4) & 3);
        let green_index = usize::from((swap >> 2) & 3);
        let blue_index = usize::from(swap & 3);
        (
            bytes[red_index.min(2)],
            bytes[green_index.min(2)],
            bytes[blue_index.min(2)],
        )
    };
    palette::towns_color_to_rgba(red, green, blue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_res_interleave_adds_the_page_select_bit() {
        // Same 4-byte swizzle as the low-res single-page transform...
        assert_eq!(high_res_interleaved_offset(0), 0);
        assert_eq!(high_res_interleaved_offset(4), 4 << 16);
        assert_eq!(high_res_interleaved_offset(8), 8 >> 1);
        assert_eq!(high_res_interleaved_offset(3), 3);
        // ...plus the 0x80000 page-select bit carried straight through.
        assert_eq!(high_res_interleaved_offset(0x8_0000), 0x8_0000);
        assert_eq!(high_res_interleaved_offset(0x8_0004), 0x8_0000 | (4 << 16));
    }
}
