//! AVX2 256-color packed scan-out inner loop for the VGA renderer.
//!
//! Each output dot is an 8-bit palette index. The indices are read straight from
//! display memory and gathered from the 256-entry RGBA palette eight at a time
//! into the framebuffer row, avoiding the scalar line buffer and its second copy
//! pass. The half rate mode 13h path gathers each source pixel once and
//! duplicates the resolved color, halving the gather traffic that dominates the
//! loop.

use core::arch::x86_64::*;

/// Bytes per output pixel (`R, G, B, A`).
const PIXEL_BYTES: usize = 4;

/// Resolves one framebuffer row from packed palette indices in display memory.
///
/// `row_fb` is `dots * 4` bytes. `source` holds the row's palette indices: one
/// per output dot at full rate, or one per pair of dots at half rate. The caller
/// guarantees `source` covers the whole row without wrapping display memory.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn render_packed_row_avx2(
    row_fb: &mut [u8],
    source: &[u8],
    pens_256: &[u32; 256],
    half_rate: bool,
) {
    let dots = row_fb.len() / PIXEL_BYTES;
    debug_assert!(source.len() >= if half_rate { dots.div_ceil(2) } else { dots });

    unsafe {
        let palette = pens_256.as_ptr() as *const i32;
        let row_ptr = row_fb.as_mut_ptr();
        let source_ptr = source.as_ptr();

        let mut dot = 0usize;
        if half_rate {
            // Duplicate each of the eight gathered colors into an adjacent pair,
            // producing sixteen output dots from a single gather.
            let low_pairs = _mm256_setr_epi32(0, 0, 1, 1, 2, 2, 3, 3);
            let high_pairs = _mm256_setr_epi32(4, 4, 5, 5, 6, 6, 7, 7);
            while dot + 16 <= dots {
                let packed = _mm_loadl_epi64(source_ptr.add(dot >> 1) as *const __m128i);
                let widened = _mm256_cvtepu8_epi32(packed);
                let pixels = _mm256_i32gather_epi32(palette, widened, 4);
                let low = _mm256_permutevar8x32_epi32(pixels, low_pairs);
                let high = _mm256_permutevar8x32_epi32(pixels, high_pairs);
                _mm256_storeu_si256(row_ptr.add(dot * PIXEL_BYTES) as *mut __m256i, low);
                _mm256_storeu_si256(row_ptr.add((dot + 8) * PIXEL_BYTES) as *mut __m256i, high);
                dot += 16;
            }
        } else {
            while dot + 8 <= dots {
                let packed = _mm_loadl_epi64(source_ptr.add(dot) as *const __m128i);
                let widened = _mm256_cvtepu8_epi32(packed);
                let pixels = _mm256_i32gather_epi32(palette, widened, 4);
                _mm256_storeu_si256(row_ptr.add(dot * PIXEL_BYTES) as *mut __m256i, pixels);
                dot += 8;
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
