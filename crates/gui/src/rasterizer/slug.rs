//! Glyph coverage with the Slug algorithm by Eric Lengyel.
//!
//! Reference: <https://github.com/EricLengyel/Slug/tree/main>
//!
//! SPDX-License-Identifier: MIT OR Apache-2.0
//!
//! The patent has been dedicated to the public domain in 2026.

use super::math::saturate;
use crate::SlugGlyph;

/// The magnitude below which a polynomial coefficient counts as zero.
const NEARLY_ZERO: f32 = 1.0 / 65536.0;

/// Calculates the root eligibility code for a sample relative quadratic Bezier
/// curve from the signs of the y coordinates of its three control points. Bit 0
/// tells if the first root contributes and bit 8 tells if the second root
/// contributes.
#[inline(always)]
fn calculate_root_code(y1: f32, y2: f32, y3: f32) -> u32 {
    let first = y1.to_bits() >> 31;
    let second = y2.to_bits() >> 30;
    let third = y3.to_bits() >> 29;

    let shift = (second & 2) | (first & !2);
    let shift = (third & 4) | (shift & !4);

    (0x2E74 >> shift) & 0x0101
}

/// Solves for the x coordinates where the curve crosses y = 0. The polynomial
/// in t is a t^2 - 2b t + c with a = p1.y - 2 p2.y + p3.y, b = p1.y - p2.y and
/// c = p1.y. Imaginary roots are treated as a double root at the minimum where
/// t = b / a.
#[inline(always)]
fn solve_horizontal_polynomial(points12: [f32; 4], point3: [f32; 2]) -> [f32; 2] {
    let a = [
        points12[0] - points12[2] * 2.0 + point3[0],
        points12[1] - points12[3] * 2.0 + point3[1],
    ];
    let b = [points12[0] - points12[2], points12[1] - points12[3]];
    let reciprocal_a = 1.0 / a[1];
    let half_reciprocal_b = 0.5 / b[1];

    let root = (b[1] * b[1] - a[1] * points12[1]).max(0.0).sqrt();
    let mut t1 = (b[1] - root) * reciprocal_a;
    let mut t2 = (b[1] + root) * reciprocal_a;

    // A nearly linear polynomial is solved as -2b t + c = 0.
    if a[1].abs() < NEARLY_ZERO {
        t1 = points12[1] * half_reciprocal_b;
        t2 = t1;
    }

    [
        (a[0] * t1 - b[0] * 2.0) * t1 + points12[0],
        (a[0] * t2 - b[0] * 2.0) * t2 + points12[0],
    ]
}

/// Solves for the y coordinates where the curve crosses x = 0.
#[inline(always)]
fn solve_vertical_polynomial(points12: [f32; 4], point3: [f32; 2]) -> [f32; 2] {
    let a = [
        points12[0] - points12[2] * 2.0 + point3[0],
        points12[1] - points12[3] * 2.0 + point3[1],
    ];
    let b = [points12[0] - points12[2], points12[1] - points12[3]];
    let reciprocal_a = 1.0 / a[0];
    let half_reciprocal_b = 0.5 / b[0];

    let root = (b[0] * b[0] - a[0] * points12[0]).max(0.0).sqrt();
    let mut t1 = (b[0] - root) * reciprocal_a;
    let mut t2 = (b[0] + root) * reciprocal_a;

    if a[0].abs() < NEARLY_ZERO {
        t1 = points12[0] * half_reciprocal_b;
        t2 = t1;
    }

    [
        (a[1] * t1 - b[1] * 2.0) * t1 + points12[1],
        (a[1] * t2 - b[1] * 2.0) * t2 + points12[1],
    ]
}

/// The control points of the curve at `location` relative to the sample. The
/// first entry holds p1 and p2, the next entry holds point3 in its first two
/// components.
#[inline(always)]
fn relative_curve(
    curves: &[[f32; 4]],
    location: u32,
    render_coordinate: [f32; 2],
) -> ([f32; 4], [f32; 2]) {
    let [x, y] = render_coordinate;
    let entry = curves[location as usize];
    let next = curves[location as usize + 1];
    (
        [entry[0] - x, entry[1] - y, entry[2] - x, entry[3] - y],
        [next[0] - x, next[1] - y],
    )
}

/// Calculates the coverage of the pixel at the em space coordinate by the
/// glyph, using the nonzero fill rule.
pub(crate) fn slug_coverage(
    curves: &[[f32; 4]],
    bands: &[u32],
    glyph: &SlugGlyph,
    render_coordinate: [f32; 2],
    pixels_per_em: [f32; 2],
) -> f32 {
    let transform = glyph.band_transform;
    let band_max = [glyph.band_max_x as i32, glyph.band_max_y as i32];
    let band_index = [
        ((render_coordinate[0] * transform[0] + transform[2]) as i32).clamp(0, band_max[0]),
        ((render_coordinate[1] * transform[1] + transform[3]) as i32).clamp(0, band_max[1]),
    ];
    let base = glyph.band_base as usize;

    let mut x_coverage = 0.0f32;
    let mut x_weight = 0.0f32;

    // The header of a band holds the number of curves in the band and the
    // offset of its curve list relative to the first header of the glyph.
    let horizontal_header = base + band_index[1] as usize * 2;
    let horizontal_count = bands[horizontal_header] as usize;
    let horizontal_list = base + bands[horizontal_header + 1] as usize;

    for &location in &bands[horizontal_list..horizontal_list + horizontal_count] {
        let (points12, point3) = relative_curve(curves, location, render_coordinate);

        // The curves are sorted in descending order of their maximum x
        // coordinate. Once a curve lies left of the pixel, no further curve can
        // contribute.
        if points12[0].max(points12[2]).max(point3[0]) * pixels_per_em[0] < -0.5 {
            break;
        }

        let code = calculate_root_code(points12[1], points12[3], point3[1]);

        if code != 0 {
            let [first, second] =
                solve_horizontal_polynomial(points12, point3).map(|root| root * pixels_per_em[0]);

            if code & 1 != 0 {
                x_coverage += saturate(first + 0.5);
                x_weight = x_weight.max(saturate(1.0 - first.abs() * 2.0));
            }

            if code > 1 {
                x_coverage -= saturate(second + 0.5);
                x_weight = x_weight.max(saturate(1.0 - second.abs() * 2.0));
            }
        }
    }

    let mut y_coverage = 0.0f32;
    let mut y_weight = 0.0f32;

    // The vertical band headers follow the horizontal band headers.
    let vertical_header = base + (band_max[1] + 1 + band_index[0]) as usize * 2;
    let vertical_count = bands[vertical_header] as usize;
    let vertical_list = base + bands[vertical_header + 1] as usize;

    for &location in &bands[vertical_list..vertical_list + vertical_count] {
        let (points12, point3) = relative_curve(curves, location, render_coordinate);

        // The curves are sorted in descending order of their maximum y
        // coordinate.
        if points12[1].max(points12[3]).max(point3[1]) * pixels_per_em[1] < -0.5 {
            break;
        }

        let code = calculate_root_code(points12[0], points12[2], point3[0]);

        if code != 0 {
            let [first, second] =
                solve_vertical_polynomial(points12, point3).map(|root| root * pixels_per_em[1]);

            if code & 1 != 0 {
                y_coverage -= saturate(first + 0.5);
                y_weight = y_weight.max(saturate(1.0 - first.abs() * 2.0));
            }

            if code > 1 {
                y_coverage += saturate(second + 0.5);
                y_weight = y_weight.max(saturate(1.0 - second.abs() * 2.0));
            }
        }
    }

    // Combine the coverage of both rays by their weights. The absolute values
    // make both winding directions work.
    let coverage = ((x_coverage * x_weight + y_coverage * y_weight).abs()
        / (x_weight + y_weight).max(NEARLY_ZERO))
    .max(x_coverage.abs().min(y_coverage.abs()));

    saturate(coverage)
}
