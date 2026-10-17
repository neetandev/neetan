//! Colors in sRGB.

/// A color with sRGB encoded channels and a linear alpha.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    /// The red channel.
    pub r: u8,
    /// The green channel.
    pub g: u8,
    /// The blue channel.
    pub b: u8,
    /// The opacity.
    pub a: u8,
}

impl Color {
    /// Opaque black.
    pub const BLACK: Self = Self::hex(0x000000);

    /// Opaque white.
    pub const WHITE: Self = Self::hex(0xFFFFFF);

    /// Fully transparent.
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);

    /// A color from its channels.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// An opaque color from `0xRRGGBB`.
    pub const fn hex(rgb: u32) -> Self {
        Self::rgba((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255)
    }

    /// The same color with the opacity `alpha` from 0 to 1.
    pub fn with_alpha(self, alpha: f32) -> Self {
        Self {
            a: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
            ..self
        }
    }

    /// The color between `self` at 0 and `other` at 1, mixed in sRGB.
    pub fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let channel = |from: u8, to: u8| {
            (f32::from(from) + (f32::from(to) - f32::from(from)) * t).round() as u8
        };
        Self {
            r: channel(self.r, other.r),
            g: channel(self.g, other.g),
            b: channel(self.b, other.b),
            a: channel(self.a, other.a),
        }
    }

    /// The color as linear channels premultiplied by the alpha.
    pub fn to_linear(self) -> [f32; 4] {
        let alpha = f32::from(self.a) / 255.0;
        [
            channel_to_linear(self.r) * alpha,
            channel_to_linear(self.g) * alpha,
            channel_to_linear(self.b) * alpha,
            alpha,
        ]
    }
}

/// The linear value of an 8 bit sRGB encoded channel.
fn channel_to_linear(channel: u8) -> f32 {
    srgb_to_linear(f32::from(channel) / 255.0)
}

/// The linear value of an sRGB encoded channel value from 0 to 1.
pub fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// The sRGB encoding of a linear channel value from 0 to 1. Values outside
/// that range are clamped.
pub fn linear_to_srgb(linear: f32) -> f32 {
    let value = linear.clamp(0.0, 1.0);
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_become_linear_and_premultiplied() {
        assert_eq!(Color::WHITE.to_linear(), [1.0, 1.0, 1.0, 1.0]);
        let [r, g, b, a] = Color::hex(0x808080).with_alpha(0.5).to_linear();
        assert!((r - 0.2158 * 0.502).abs() < 1e-3);
        assert_eq!(r, g);
        assert_eq!(g, b);
        assert!((a - 0.502).abs() < 1e-3);
    }

    #[test]
    fn linear_to_srgb_inverts_srgb_to_linear() {
        for channel in 0..=255u8 {
            let encoded = linear_to_srgb(channel_to_linear(channel)) * 255.0;
            assert!((encoded - f32::from(channel)).abs() < 1e-2);
        }
        assert_eq!(linear_to_srgb(-1.0), 0.0);
        assert!((linear_to_srgb(2.0) - 1.0).abs() < 1e-6);
    }
}
