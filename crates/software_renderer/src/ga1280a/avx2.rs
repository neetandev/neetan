//! AVX2 GA-1280A compose helpers.

use core::arch::x86_64::*;

use super::{PIXEL_BYTES, direct_color16_to_rgba};

const PIXELS_PER_DIRECT_COLOR16_ITER: usize = 16;
const SOURCE_BYTES_PER_DIRECT_COLOR16_ITER: usize = PIXELS_PER_DIRECT_COLOR16_ITER * 2;
const OUTPUT_BYTES_PER_DIRECT_COLOR16_ITER: usize = PIXELS_PER_DIRECT_COLOR16_ITER * PIXEL_BYTES;

#[target_feature(enable = "avx2")]
pub(super) unsafe fn compose_direct_color16_identity_avx2(
    framebuffer: &mut [u8],
    vram: &[u8],
    width: usize,
    height: usize,
    stride: usize,
) {
    unsafe {
        let red_mask = _mm256_set1_epi16(0x001F);
        let green_mask = _mm256_set1_epi16(0x003F);
        let blue_mask = _mm256_set1_epi16(0x001F);
        let alpha_high_byte = _mm256_set1_epi16(0xFF00u16 as i16);

        for y in 0..height {
            let row_start = y * stride;
            let framebuffer_row_start = y * width * PIXEL_BYTES;

            let mut x = 0usize;
            while x + PIXELS_PER_DIRECT_COLOR16_ITER <= width {
                let source_ptr = vram.as_ptr().add(row_start + x * 2);
                let destination_ptr = framebuffer
                    .as_mut_ptr()
                    .add(framebuffer_row_start + x * PIXEL_BYTES);

                let data = _mm256_loadu_si256(source_ptr as *const __m256i);

                let red5 = _mm256_and_si256(_mm256_srli_epi16(data, 11), red_mask);
                let red8 = _mm256_or_si256(_mm256_slli_epi16(red5, 3), _mm256_srli_epi16(red5, 2));

                let green6 = _mm256_and_si256(_mm256_srli_epi16(data, 5), green_mask);
                let green8 =
                    _mm256_or_si256(_mm256_slli_epi16(green6, 2), _mm256_srli_epi16(green6, 4));

                let blue5 = _mm256_and_si256(data, blue_mask);
                let blue8 =
                    _mm256_or_si256(_mm256_slli_epi16(blue5, 3), _mm256_srli_epi16(blue5, 2));

                let red_green = _mm256_or_si256(red8, _mm256_slli_epi16(green8, 8));
                let blue_alpha = _mm256_or_si256(blue8, alpha_high_byte);

                let lo = _mm256_unpacklo_epi16(red_green, blue_alpha);
                let hi = _mm256_unpackhi_epi16(red_green, blue_alpha);

                let pixels_0_7 = _mm256_permute2x128_si256(lo, hi, 0x20);
                let pixels_8_15 = _mm256_permute2x128_si256(lo, hi, 0x31);

                _mm256_storeu_si256(destination_ptr as *mut __m256i, pixels_0_7);
                _mm256_storeu_si256(destination_ptr.add(32) as *mut __m256i, pixels_8_15);

                x += PIXELS_PER_DIRECT_COLOR16_ITER;
            }

            while x < width {
                let pixel_offset = row_start + x * 2;
                let color =
                    u16::from(vram[pixel_offset]) | (u16::from(vram[pixel_offset + 1]) << 8);
                let pixel = direct_color16_to_rgba(color);
                let fb_offset = framebuffer_row_start + x * PIXEL_BYTES;
                framebuffer[fb_offset..fb_offset + PIXEL_BYTES]
                    .copy_from_slice(&pixel.to_le_bytes());
                x += 1;
            }
        }
    }
}

const PIXELS_PER_FULL_COLOR24_ITER: usize = 8;
const SOURCE_BYTES_PER_FULL_COLOR24_ITER: usize = PIXELS_PER_FULL_COLOR24_ITER * 3;
const FULL_COLOR24_LAST_LOAD_END_OFFSET: usize = 28;
const FULL_COLOR24_SHUFFLE_LANE: [u8; 16] =
    [2, 1, 0, 0x80, 5, 4, 3, 0x80, 8, 7, 6, 0x80, 11, 10, 9, 0x80];

#[target_feature(enable = "avx2")]
pub(super) unsafe fn compose_full_color24_identity_avx2(
    framebuffer: &mut [u8],
    vram: &[u8],
    width: usize,
    height: usize,
    stride: usize,
) {
    unsafe {
        let shuffle_lane = _mm_loadu_si128(FULL_COLOR24_SHUFFLE_LANE.as_ptr() as *const __m128i);
        let shuffle = _mm256_set_m128i(shuffle_lane, shuffle_lane);
        let alpha_mask = _mm256_set1_epi32(0xFF000000u32 as i32);

        for y in 0..height {
            let row_start = y * stride;
            let framebuffer_row_start = y * width * PIXEL_BYTES;
            let row_capacity = vram.len() - row_start;
            let simd_iters_max_capacity = if row_capacity >= FULL_COLOR24_LAST_LOAD_END_OFFSET {
                (row_capacity - FULL_COLOR24_LAST_LOAD_END_OFFSET)
                    / SOURCE_BYTES_PER_FULL_COLOR24_ITER
                    + 1
            } else {
                0
            };
            let simd_iters = (width / PIXELS_PER_FULL_COLOR24_ITER).min(simd_iters_max_capacity);

            let mut iter = 0usize;
            while iter < simd_iters {
                let pixel_index = iter * PIXELS_PER_FULL_COLOR24_ITER;
                let source_ptr = vram.as_ptr().add(row_start + pixel_index * 3);
                let destination_ptr = framebuffer
                    .as_mut_ptr()
                    .add(framebuffer_row_start + pixel_index * PIXEL_BYTES);

                let lo = _mm_loadu_si128(source_ptr as *const __m128i);
                let hi = _mm_loadu_si128(source_ptr.add(12) as *const __m128i);
                let data = _mm256_set_m128i(hi, lo);

                let shuffled = _mm256_shuffle_epi8(data, shuffle);
                let result = _mm256_or_si256(shuffled, alpha_mask);

                _mm256_storeu_si256(destination_ptr as *mut __m256i, result);

                iter += 1;
            }

            let mut x = simd_iters * PIXELS_PER_FULL_COLOR24_ITER;
            while x < width {
                let pixel_offset = row_start + x * 3;
                let blue = vram[pixel_offset];
                let green = vram[pixel_offset + 1];
                let red = vram[pixel_offset + 2];
                let fb_offset = framebuffer_row_start + x * PIXEL_BYTES;
                framebuffer[fb_offset] = red;
                framebuffer[fb_offset + 1] = green;
                framebuffer[fb_offset + 2] = blue;
                framebuffer[fb_offset + 3] = 0xFF;
                x += 1;
            }
        }
    }
}

const _: () = {
    assert!(OUTPUT_BYTES_PER_DIRECT_COLOR16_ITER == 64);
    assert!(SOURCE_BYTES_PER_DIRECT_COLOR16_ITER == 32);
    assert!(SOURCE_BYTES_PER_FULL_COLOR24_ITER == 24);
};
