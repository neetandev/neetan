//! Pure display helpers for the automation screen operations.
//!
//! These functions operate on tightly packed RGBA8 buffers (`width * height * 4`
//! bytes, rows of `width` pixels). They hold no session or machine state so they
//! can be unit tested directly. The session layer reads the machine framebuffer,
//! decodes expected PNGs, and writes artifacts around them.

use blake3::Hasher;

/// Versioned ASCII tag that prefixes the screen-hash input.
///
/// Bumping this string changes every hash, so it is part of the hash contract.
const SCREEN_HASH_TAG: &[u8] = b"neetan-screen-rgba8-v1";

/// Returns the 64-character lowercase hex BLAKE3 hash of a screen.
///
/// The input is the versioned tag, then little-endian `width` and `height`, then
/// the tightly packed valid RGBA bytes, so dimensions participate in the hash.
#[must_use]
pub fn screen_hash_hex(width: u32, height: u32, rgba: &[u8]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(SCREEN_HASH_TAG);
    hasher.update(&width.to_le_bytes());
    hasher.update(&height.to_le_bytes());
    hasher.update(rgba);
    let mut digest = [0u8; 32];
    hasher.finalize(&mut digest);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push(nibble_to_hex(byte >> 4));
        hex.push(nibble_to_hex(byte & 0x0F));
    }
    hex
}

/// Maps a 0..=15 nibble to its lowercase hex character.
fn nibble_to_hex(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'a' + (nibble - 10)) as char,
    }
}

/// Computes the normalized RGB RMSE between two RGBA8 buffers of equal size.
///
/// Alpha is ignored. The result is
/// `sqrt(sum((actual - expected)^2) / (width * height * 3)) / 255`, in `[0, 1]`.
#[must_use]
pub fn rgb_rmse(actual: &[u8], expected: &[u8], width: u32, height: u32) -> f64 {
    let pixels = width as u64 * height as u64;
    if pixels == 0 {
        return 0.0;
    }
    let mut sum_squares: f64 = 0.0;
    for pixel in 0..pixels as usize {
        let base = pixel * 4;
        for channel in 0..3 {
            let a = f64::from(actual[base + channel]);
            let e = f64::from(expected[base + channel]);
            let difference = a - e;
            sum_squares += difference * difference;
        }
    }
    (sum_squares / (pixels as f64 * 3.0)).sqrt() / 255.0
}

/// Reports whether two equal-sized RGBA8 images match within an RGB RMSE limit.
///
/// Alpha is ignored. Exact comparisons avoid floating-point work, while
/// tolerant comparisons stop once the accumulated error exceeds the limit.
#[must_use]
pub fn rgb_matches(
    actual: &[u8],
    expected: &[u8],
    width: u32,
    height: u32,
    tolerance: f64,
) -> bool {
    let pixels = width as usize * height as usize;
    if tolerance == 0.0 {
        return actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .take(pixels)
            .all(|(actual, expected)| actual[..3] == expected[..3]);
    }

    let channel_count = pixels as f64 * 3.0;
    let maximum_error = tolerance * tolerance * 255.0 * 255.0 * channel_count;
    let mut sum_squares = 0u128;
    for (actual, expected) in actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .take(pixels)
    {
        for channel in 0..3 {
            let difference = i32::from(actual[channel]) - i32::from(expected[channel]);
            sum_squares += u128::from(difference.unsigned_abs()).pow(2);
        }
        if sum_squares as f64 > maximum_error {
            return false;
        }
    }
    true
}

/// Extracts a `w` by `h` RGBA8 region at `(x, y)` from a `full_width` by
/// `full_height` buffer, or `None` when the region falls outside the source.
#[must_use]
pub fn extract_region(
    source: &[u8],
    full_width: u32,
    full_height: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Option<Vec<u8>> {
    if w == 0 || h == 0 {
        return None;
    }
    if x.checked_add(w)? > full_width || y.checked_add(h)? > full_height {
        return None;
    }
    let full_row = full_width as usize * 4;
    let region_row = w as usize * 4;
    let mut out = vec![0u8; region_row * h as usize];
    for row in 0..h as usize {
        let source_start = (y as usize + row) * full_row + x as usize * 4;
        out[row * region_row..(row + 1) * region_row]
            .copy_from_slice(&source[source_start..source_start + region_row]);
    }
    Some(out)
}

/// Builds a side-by-side RGBA8 image, expected on the left and actual on the
/// right, each `width` by `height`, so the result is `2 * width` by `height`.
///
/// Alpha is forced opaque so the comparison image renders cleanly in a viewer.
#[must_use]
pub fn side_by_side(expected: &[u8], actual: &[u8], width: u32, height: u32) -> Vec<u8> {
    let half_row = width as usize * 4;
    let full_row = half_row * 2;
    let mut out = vec![0u8; full_row * height as usize];
    for row in 0..height as usize {
        let target = row * full_row;
        let source = row * half_row;
        copy_opaque(
            &mut out[target..target + half_row],
            &expected[source..source + half_row],
        );
        copy_opaque(
            &mut out[target + half_row..target + full_row],
            &actual[source..source + half_row],
        );
    }
    out
}

/// Places differently sized images side by side on an opaque black canvas.
#[must_use]
pub fn side_by_side_native_size(
    expected: &[u8],
    expected_width: u32,
    expected_height: u32,
    actual: &[u8],
    actual_width: u32,
    actual_height: u32,
) -> Option<(u32, u32, Vec<u8>)> {
    let width = expected_width.checked_add(actual_width)?;
    let height = expected_height.max(actual_height);
    let row_bytes = width as usize * 4;
    let mut output = vec![0u8; row_bytes * height as usize];
    for pixel in output.chunks_exact_mut(4) {
        pixel[3] = 0xFF;
    }

    copy_image(
        &mut output,
        width,
        0,
        expected,
        expected_width,
        expected_height,
    );
    copy_image(
        &mut output,
        width,
        expected_width,
        actual,
        actual_width,
        actual_height,
    );
    Some((width, height, output))
}

/// Copies one image into an opaque destination canvas at the given x offset.
fn copy_image(
    target: &mut [u8],
    target_width: u32,
    target_x: u32,
    source: &[u8],
    source_width: u32,
    source_height: u32,
) {
    let target_row = target_width as usize * 4;
    let source_row = source_width as usize * 4;
    for row in 0..source_height as usize {
        let target_start = row * target_row + target_x as usize * 4;
        let source_start = row * source_row;
        copy_opaque(
            &mut target[target_start..target_start + source_row],
            &source[source_start..source_start + source_row],
        );
    }
}

/// Copies one RGBA8 row, forcing every alpha byte to fully opaque.
fn copy_opaque(target: &mut [u8], source: &[u8]) {
    target.copy_from_slice(source);
    for pixel in target.chunks_exact_mut(4) {
        pixel[3] = 0xFF;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        extract_region, rgb_matches, rgb_rmse, screen_hash_hex, side_by_side,
        side_by_side_native_size,
    };

    fn solid(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        color
            .iter()
            .copied()
            .cycle()
            .take((width * height * 4) as usize)
            .collect()
    }

    #[test]
    fn identical_images_have_zero_rmse() {
        let image = solid(4, 4, [10, 20, 30, 255]);
        assert_eq!(rgb_rmse(&image, &image, 4, 4), 0.0);
    }

    #[test]
    fn constant_offset_matches_hand_computed_rmse() {
        // Every RGB channel differs by exactly 51, so RMSE = 51 / 255 = 0.2.
        let actual = solid(2, 2, [51, 51, 51, 255]);
        let expected = solid(2, 2, [0, 0, 0, 0]);
        let metric = rgb_rmse(&actual, &expected, 2, 2);
        assert!((metric - 0.2).abs() < 1e-12, "metric was {metric}");
    }

    #[test]
    fn alpha_is_ignored_by_rmse() {
        let actual = solid(2, 2, [0, 0, 0, 0]);
        let expected = solid(2, 2, [0, 0, 0, 255]);
        assert_eq!(rgb_rmse(&actual, &expected, 2, 2), 0.0);
    }

    #[test]
    fn exact_match_ignores_alpha() {
        let actual = solid(2, 2, [10, 20, 30, 0]);
        let expected = solid(2, 2, [10, 20, 30, 255]);
        assert!(rgb_matches(&actual, &expected, 2, 2, 0.0));
    }

    #[test]
    fn tolerant_match_accepts_only_values_within_the_rmse_limit() {
        let actual = solid(2, 2, [51, 51, 51, 255]);
        let expected = solid(2, 2, [0, 0, 0, 255]);
        assert!(rgb_matches(&actual, &expected, 2, 2, 0.2));
        assert!(!rgb_matches(&actual, &expected, 2, 2, 0.19));
    }

    #[test]
    fn hash_is_stable_and_dimension_sensitive() {
        let image = solid(2, 3, [1, 2, 3, 255]);
        let hash = screen_hash_hex(2, 3, &image);
        assert_eq!(hash.len(), 64);
        assert!(
            hash.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_eq!(hash, screen_hash_hex(2, 3, &image));
        assert_ne!(hash, screen_hash_hex(3, 2, &image));
    }

    #[test]
    fn side_by_side_places_halves_and_forces_opaque() {
        let expected = solid(1, 1, [1, 2, 3, 0]);
        let actual = solid(1, 1, [4, 5, 6, 0]);
        let combined = side_by_side(&expected, &actual, 1, 1);
        assert_eq!(combined.len(), 8);
        assert_eq!(&combined[0..4], &[1, 2, 3, 255]);
        assert_eq!(&combined[4..8], &[4, 5, 6, 255]);
    }

    #[test]
    fn native_size_comparison_preserves_dimensions_and_pads_black() {
        let expected = solid(1, 2, [1, 2, 3, 0]);
        let actual = solid(2, 1, [4, 5, 6, 0]);
        let (width, height, combined) =
            side_by_side_native_size(&expected, 1, 2, &actual, 2, 1).expect("dimensions");
        assert_eq!((width, height), (3, 2));
        assert_eq!(&combined[0..4], &[1, 2, 3, 255]);
        assert_eq!(&combined[4..8], &[4, 5, 6, 255]);
        assert_eq!(&combined[8..12], &[4, 5, 6, 255]);
        assert_eq!(&combined[16..20], &[0, 0, 0, 255]);
    }

    #[test]
    fn extract_region_copies_the_window() {
        // 3x2 image with a unique red value per pixel column.
        let mut source = vec![0u8; 3 * 2 * 4];
        for (index, pixel) in source.chunks_exact_mut(4).enumerate() {
            pixel[0] = index as u8;
            pixel[3] = 255;
        }
        let region = extract_region(&source, 3, 2, 1, 0, 2, 2).expect("in range");
        assert_eq!(region.len(), 2 * 2 * 4);
        assert_eq!(region[0], 1);
        assert_eq!(region[4], 2);
        assert_eq!(region[8], 4);
        assert!(extract_region(&source, 3, 2, 2, 0, 2, 2).is_none());
    }
}
