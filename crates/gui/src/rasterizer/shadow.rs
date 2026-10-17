//! The shadow of a rectangle with rounded corners under a Gaussian blur.
//!
//! The blur of a straight edge has a closed form with the error function. The
//! shadow integrates the blurred rows of the rectangle along the y-axis with
//! four samples. Each row knows the width of the rounded rectangle at its
//! height. This follows the approach of Evan Wallace:
//! <https://madebyevan.com/shaders/fast-rounded-rectangle-shadows/>

use std::f32::consts::{FRAC_1_SQRT_2, PI};

use super::math::sign;

/// The number of rows the shadow integrates.
const ROW_SAMPLES: usize = 4;

/// The density of the normal distribution with the deviation `sigma` at `x`.
#[inline(always)]
fn gaussian(x: f32, sigma: f32) -> f32 {
    (-(x * x) / (2.0 * sigma * sigma)).exp() / ((2.0 * PI).sqrt() * sigma)
}

/// An approximation of the error function with a maximum error of 5e-4.
#[inline(always)]
fn erf(x: f32) -> f32 {
    let sign_x = sign(x);
    let magnitude = x.abs();
    let mut t = 1.0
        + (0.278_393 + (0.230_389 + 0.078_108 * (magnitude * magnitude)) * magnitude) * magnitude;
    t *= t;
    sign_x - sign_x / (t * t)
}

/// The shadow of one rounded rectangle.
pub(crate) struct Shadow {
    /// Half the width and the height of the rectangle.
    half_size: [f32; 2],
    /// The corner radius, limited to half the shorter side.
    radius: f32,
    /// The standard deviation of the blur.
    sigma: f32,
}

/// One pixel row of a [`Shadow`].
pub(crate) struct ShadowRow {
    /// The half width of the rectangle at every integrated row.
    curved: [f32; ROW_SAMPLES],
    /// The Gaussian weight of every integrated row.
    weights: [f32; ROW_SAMPLES],
    /// The distance between two integrated rows.
    step: f32,
    /// The factor that turns a distance into the argument of the error
    /// function.
    scale: f32,
}

impl Shadow {
    /// The shadow of a rectangle of `half_size` with the corner radius
    /// `radius` under a blur with the deviation `sigma`.
    pub(crate) fn new(half_size: [f32; 2], radius: f32, sigma: f32) -> Self {
        Self {
            half_size,
            radius: radius.min(half_size[0].min(half_size[1])),
            sigma,
        }
    }

    /// The row at `position_y` relative to the center of the rectangle.
    pub(crate) fn row(&self, position_y: f32) -> ShadowRow {
        let Self {
            half_size,
            radius,
            sigma,
        } = *self;
        let low = position_y - half_size[1];
        let high = position_y + half_size[1];
        let start = (-3.0 * sigma).max(low).min(high);
        let end = (3.0 * sigma).max(low).min(high);

        let step = (end - start) / ROW_SAMPLES as f32;
        let mut y = start + step * 0.5;
        let mut curved = [0.0; ROW_SAMPLES];
        let mut weights = [0.0; ROW_SAMPLES];
        for (curved, weight) in curved.iter_mut().zip(&mut weights) {
            let delta = (half_size[1] - radius - (position_y - y).abs()).min(0.0);
            *curved = half_size[0] - radius + (radius * radius - delta * delta).max(0.0).sqrt();
            *weight = gaussian(y, sigma);
            y += step;
        }
        ShadowRow {
            curved,
            weights,
            step,
            scale: FRAC_1_SQRT_2 / sigma,
        }
    }
}

impl ShadowRow {
    /// The shadow intensity at `position_x` relative to the center of the
    /// rectangle.
    #[inline(always)]
    pub(crate) fn intensity(&self, position_x: f32) -> f32 {
        let mut value = 0.0;
        for (curved, weight) in self.curved.iter().zip(&self.weights) {
            let left = 0.5 + 0.5 * erf((position_x - curved) * self.scale);
            let right = 0.5 + 0.5 * erf((position_x + curved) * self.scale);
            value += (right - left) * weight * self.step;
        }
        value
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The blurred coverage of one row of the rectangle at the height `y`.
    fn row(x: f32, y: f32, sigma: f32, radius: f32, half_size: [f32; 2]) -> f32 {
        let delta = (half_size[1] - radius - y.abs()).min(0.0);
        let curved = half_size[0] - radius + (radius * radius - delta * delta).max(0.0).sqrt();
        let scale = FRAC_1_SQRT_2 / sigma;
        let left = 0.5 + 0.5 * erf((x - curved) * scale);
        let right = 0.5 + 0.5 * erf((x + curved) * scale);
        right - left
    }

    /// The shadow intensity at `position` relative to the center of the
    /// rectangle evaluated for one pixel.
    pub(crate) fn rounded_rectangle_shadow(
        position: [f32; 2],
        half_size: [f32; 2],
        radius: f32,
        sigma: f32,
    ) -> f32 {
        let radius = radius.min(half_size[0].min(half_size[1]));
        let low = position[1] - half_size[1];
        let high = position[1] + half_size[1];
        let start = (-3.0 * sigma).max(low).min(high);
        let end = (3.0 * sigma).max(low).min(high);

        let step = (end - start) / ROW_SAMPLES as f32;
        let mut y = start + step * 0.5;
        let mut value = 0.0;
        for _ in 0..ROW_SAMPLES {
            value += row(position[0], position[1] - y, sigma, radius, half_size)
                * gaussian(y, sigma)
                * step;
            y += step;
        }
        value
    }

    #[test]
    fn the_shadow_is_dense_inside_and_fades_outside() {
        let half_size = [50.0, 50.0];
        let center = rounded_rectangle_shadow([0.0, 0.0], half_size, 4.0, 3.0);
        assert!((center - 1.0).abs() < 0.02, "center {center}");
        let edge = rounded_rectangle_shadow([50.0, 0.0], half_size, 4.0, 3.0);
        assert!((edge - 0.5).abs() < 0.02, "edge {edge}");
        let outside = rounded_rectangle_shadow([60.0, 0.0], half_size, 4.0, 3.0);
        assert!(outside < 0.01, "outside {outside}");
        assert!(erf(0.0).abs() < 1e-6);
        assert!((erf(1.0) - 0.842_7).abs() < 5e-4);
    }

    #[test]
    fn row_intensities_equal_the_pixel_intensities() {
        for (half_size, radius, sigma) in [
            ([50.0, 30.0], 4.0, 3.0),
            ([12.5, 40.25], 20.0, 0.5),
            ([200.0, 100.0], 0.0, 24.0),
        ] {
            let shadow = Shadow::new(half_size, radius, sigma);
            for step_y in -120..120 {
                let position_y = step_y as f32 * 1.13 + 0.5;
                let row = shadow.row(position_y);
                for step_x in -150..150 {
                    let position_x = step_x as f32 * 1.71 + 0.5;
                    let expected = rounded_rectangle_shadow(
                        [position_x, position_y],
                        half_size,
                        radius,
                        sigma,
                    );
                    assert_eq!(
                        row.intensity(position_x).to_bits(),
                        expected.to_bits(),
                        "shadow at {position_x}, {position_y}"
                    );
                }
            }
        }
    }
}
