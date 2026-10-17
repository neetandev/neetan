//! Table versions of the sRGB transfer functions.
//!
//! The rasterizer evaluates the sRGB curves for millions of pixels in a frame.
//! The tables interpolate linearly between exact values of
//! [`crate::linear_to_srgb`] and [`crate::srgb_to_linear`]. Their error stays
//! below a hundredth of an 8 bit step.

use std::sync::OnceLock;

use super::math::saturate;
use crate::{linear_to_srgb, srgb_to_linear};

/// The number of linear segments of each table.
const SEGMENTS: usize = 4096;

/// A table with the values of a curve at `index / SEGMENTS`.
type Table = Box<[f32; SEGMENTS + 1]>;

/// The tables of both directions.
pub(crate) struct Curves {
    /// The encoded values at the linear values `index / SEGMENTS`.
    encode: Table,
    /// The linear values at the encoded values `index / SEGMENTS`.
    decode: Table,
}

/// The tables, built on first use.
static CURVES: OnceLock<Curves> = OnceLock::new();

/// The shared tables.
pub(crate) fn curves() -> &'static Curves {
    CURVES.get_or_init(|| {
        let table = |function: fn(f32) -> f32| -> Table {
            let mut table = Box::new([0.0; SEGMENTS + 1]);
            for (index, value) in table.iter_mut().enumerate() {
                *value = function(index as f32 / SEGMENTS as f32);
            }
            table
        };
        Curves {
            encode: table(linear_to_srgb),
            decode: table(srgb_to_linear),
        }
    })
}

/// The value of `table` at `value`, clamped to 0 to 1.
#[inline(always)]
fn interpolate(table: &[f32; SEGMENTS + 1], value: f32) -> f32 {
    let position = saturate(value) * SEGMENTS as f32;
    let index = (position as usize).min(SEGMENTS - 1);
    let fraction = position - index as f32;
    let low = table[index];
    let high = table[index + 1];
    low + (high - low) * fraction
}

impl Curves {
    /// The sRGB encoding of the linear value `linear`, clamped to 0 to 1.
    #[inline(always)]
    pub(crate) fn linear_to_srgb(&self, linear: f32) -> f32 {
        interpolate(&self.encode, linear)
    }

    /// The linear value of the sRGB encoded value `srgb`, clamped to 0 to 1.
    #[inline(always)]
    pub(crate) fn srgb_to_linear(&self, srgb: f32) -> f32 {
        interpolate(&self.decode, srgb)
    }

    /// The output bytes of a linear, premultiplied color: sRGB encoded,
    /// premultiplied channels and a linear alpha.
    #[inline(always)]
    pub(crate) fn encode_pixel(&self, color: [f32; 4]) -> [u8; 4] {
        let alpha = saturate(color[3]);
        if alpha <= 0.0 {
            return [0; 4];
        }
        let inverse = 1.0 / alpha;
        let channel =
            |value: f32| (self.linear_to_srgb(value * inverse) * alpha * 255.0 + 0.5) as u8;
        [
            channel(color[0]),
            channel(color[1]),
            channel(color[2]),
            (alpha * 255.0 + 0.5) as u8,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_match_the_exact_curves() {
        let curves = curves();
        for step in 0..=100_000 {
            let value = step as f32 / 100_000.0;
            let encode_error = (curves.linear_to_srgb(value) - linear_to_srgb(value)).abs();
            let decode_error = (curves.srgb_to_linear(value) - srgb_to_linear(value)).abs();
            assert!(encode_error * 255.0 < 0.01, "encode error at {value}");
            assert!(decode_error * 255.0 < 0.01, "decode error at {value}");
        }
    }
}
