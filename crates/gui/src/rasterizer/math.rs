//! Scalar helpers with the semantics of the shader intrinsics.

/// Clamps `value` to the range from 0 to 1. NaN becomes 0, as on the GPU.
#[inline(always)]
pub(crate) fn saturate(value: f32) -> f32 {
    if value > 0.0 { value.min(1.0) } else { 0.0 }
}

/// The sign of `value` as -1, 0 or 1. Zero and NaN give 0, as on the GPU.
#[inline(always)]
pub(crate) fn sign(value: f32) -> f32 {
    if value > 0.0 {
        1.0
    } else if value < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// The length of the vector `(x, y)`.
#[inline(always)]
pub(crate) fn length(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// The value between `from` at 0 and `to` at 1.
#[inline(always)]
pub(crate) fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}
