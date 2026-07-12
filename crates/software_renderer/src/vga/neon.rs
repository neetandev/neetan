//! NEON 256-color packed scan-out inner loop for the VGA renderer.

use core::arch::aarch64::*;

/// Bytes per output pixel (`R, G, B, A`).
const PIXEL_BYTES: usize = 4;

/// Resolves one framebuffer row from packed palette indices in display memory.
///
/// `row_fb` is `dots * 4` bytes. `source` holds the row's palette indices: one
/// per output dot at full rate, or one per pair of dots at half rate. The caller
/// guarantees `source` covers the whole row without wrapping display memory.
pub(super) unsafe fn render_packed_row_neon(
    row_fb: &mut [u8],
    source: &[u8],
    pens_256: &[u32; 256],
    half_rate: bool,
) {
    let dots = row_fb.len() / PIXEL_BYTES;
    debug_assert!(source.len() >= if half_rate { dots.div_ceil(2) } else { dots });

    unsafe {
        let row_ptr = row_fb.as_mut_ptr();

        let mut dot = 0usize;
        if half_rate {
            while dot + 8 <= dots {
                let base = dot >> 1;
                let quad = [
                    pens_256[usize::from(source[base])],
                    pens_256[usize::from(source[base + 1])],
                    pens_256[usize::from(source[base + 2])],
                    pens_256[usize::from(source[base + 3])],
                ];
                let colors = vld1q_u32(quad.as_ptr());
                vst1q_u32(
                    row_ptr.add(dot * PIXEL_BYTES) as *mut u32,
                    vzip1q_u32(colors, colors),
                );
                vst1q_u32(
                    row_ptr.add((dot + 4) * PIXEL_BYTES) as *mut u32,
                    vzip2q_u32(colors, colors),
                );
                dot += 8;
            }
        } else {
            while dot + 4 <= dots {
                let quad = [
                    pens_256[usize::from(source[dot])],
                    pens_256[usize::from(source[dot + 1])],
                    pens_256[usize::from(source[dot + 2])],
                    pens_256[usize::from(source[dot + 3])],
                ];
                vst1q_u32(
                    row_ptr.add(dot * PIXEL_BYTES) as *mut u32,
                    vld1q_u32(quad.as_ptr()),
                );
                dot += 4;
            }
        }

        while dot < dots {
            let index = if half_rate { dot >> 1 } else { dot };
            let pixel = pens_256[usize::from(source[index])];
            let offset = dot * PIXEL_BYTES;
            row_fb[offset..offset + PIXEL_BYTES].copy_from_slice(&pixel.to_le_bytes());
            dot += 1;
        }
    }
}
