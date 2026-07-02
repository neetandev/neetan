//! SDL3 rendering backends: GPU API primary, 2D renderer fallback.
#![deny(missing_docs)]

mod device;
mod error;
mod gpu;
mod legacy;
mod pipeline;

pub use error::Error;
pub use gpu::ModernSdlGpuBackend;
pub use legacy::LegacySdlBackend;
use sdl3::video::Window;

/// Native rendering target width in pixels.
pub const PC98_NATIVE_WIDTH: u32 = 640;
/// Native rendering target height in pixels.
pub const PC98_NATIVE_HEIGHT: u32 = 480;
/// Size in bytes of the native framebuffer.
pub const PC98_FRAMEBUFFER_BYTES: u64 = (PC98_NATIVE_WIDTH * PC98_NATIVE_HEIGHT * 4) as u64;
/// Maximum large-target width in pixels (covers the PC-88VA GA and the FM Towns
/// MX 1024x768 high-resolution mode).
pub const GA_MAX_WIDTH: u32 = 1600;
/// Maximum large-target height in pixels.
pub const GA_MAX_HEIGHT: u32 = 1024;

/// Backend result type.
pub type Result<T> = std::result::Result<T, Error>;

/// The instructions to render a frame.
pub struct RenderInstructions<'a> {
    /// Packed `R, G, B, A` sRGB pixels. The slice may be larger than
    /// `width * height * 4`; only the top-left `width * height` region is read.
    pub framebuffer: &'a [u8],
    /// Active output width in pixels.
    pub width: u32,
    /// Active output height in pixels.
    pub height: u32,
    /// Whether the CRT upscale effect is enabled.
    pub crt: bool,
    /// Whether to use the composite (NTSC) CRT variant instead of the sharp
    /// RGB-monitor one. Only meaningful when `crt` is set.
    pub composite: bool,
    /// Composite subcarrier phase select (0..3, in 90-degree steps). Swaps the
    /// complementary artifact-color pair, mimicking the random boot-time phase of
    /// a real MC6847. Only meaningful when `composite` is set.
    pub composite_phase: u32,
}

/// Returns the backing texture dimensions for a backend configuration.
pub fn native_target_size(large_native_target: bool) -> (u32, u32) {
    if large_native_target {
        (GA_MAX_WIDTH, GA_MAX_HEIGHT)
    } else {
        (PC98_NATIVE_WIDTH, PC98_NATIVE_HEIGHT)
    }
}

/// Returns the backing texture byte length for a backend configuration.
pub fn native_target_bytes(large_native_target: bool) -> u64 {
    let (width, height) = native_target_size(large_native_target);
    u64::from(width) * u64::from(height) * 4
}

/// A backend-neutral interface for the graphics engine.
pub trait GraphicsEngine {
    /// Called when the window is resuming.
    fn on_resume(&mut self, window: &mut Window, vsync_enabled: bool) -> Result<()>;

    /// Called when the rendering surface should be torn down (e.g., Android suspend).
    fn on_destroy_surface(&mut self);

    /// Renders the next frame.
    fn render_frame(
        &mut self,
        window: &Window,
        render_instructions: Option<&RenderInstructions>,
    ) -> Result<()>;

    /// Selects the scaling method applied.
    fn set_scaling(&mut self, scaling: Scaling);
}

/// Scaling method used to scale the native image.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Scaling {
    /// Nearest-neighbour sampling: blocky pixels, no blending.
    Nearest,
    /// Hardware bilinear sampling: smooth but blurry.
    Bilinear,
    /// Pixel-art filter: single hardware bilinear sample
    /// produces crisp pixel art at arbitrary (non-integer) scale.
    Pixelart,
}

/// Display aspect mode for scaling and startup dimensions.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DisplayAspectMode {
    /// Pixel aspect correction: 640x400 is presented as 4:3.
    Aspect4By3,
    /// Square pixels: native 640x400 maps to 1:1 pixel aspect.
    Aspect1By1,
}

impl DisplayAspectMode {
    /// Source-size vector passed to the present shader for aspect-ratio fitting.
    ///
    /// For 4:3 mode the display aspect is fixed (the height is derived from
    /// the width times 3/4), so any content shape is stretched to 4:3. For 1:1
    /// (square-pixel) mode the display aspect tracks the live content
    /// dimensions.
    pub fn source_size(self, width: u32, height: u32) -> [f32; 2] {
        let width = width.max(1) as f32;
        let height = height.max(1) as f32;
        match self {
            Self::Aspect4By3 => [width, width * 3.0 / 4.0],
            Self::Aspect1By1 => [width, height],
        }
    }

    /// Returns the displayed aspect ratio for the given content dimensions.
    pub fn display_aspect_ratio(self, width: u32, height: u32) -> f64 {
        let source = self.source_size(width, height);
        f64::from(source[0]) / f64::from(source[1])
    }
}

/// Computes the fitted color-target extent for a given surface size and aspect ratio.
///
/// Picks the largest (width, height) within (surface_width, surface_height) that
/// matches `aspect_ratio` exactly. Used to letterbox/pillarbox the rendered image.
pub fn compute_color_target_extent(
    surface_width: u32,
    surface_height: u32,
    aspect_ratio: f64,
) -> (u32, u32) {
    let surface_aspect = surface_width as f64 / surface_height as f64;
    if surface_aspect > aspect_ratio {
        let height = surface_height;
        let width = (surface_height as f64 * aspect_ratio).round() as u32;
        (width, height)
    } else {
        let width = surface_width;
        let height = (surface_width as f64 / aspect_ratio).round() as u32;
        (width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_by_three_aspect_stretches_to_4_3() {
        assert_eq!(
            DisplayAspectMode::Aspect4By3.source_size(640, 400),
            [640.0, 480.0]
        );
        assert_eq!(
            DisplayAspectMode::Aspect4By3.source_size(1280, 1024),
            [1280.0, 960.0]
        );
        assert_eq!(
            DisplayAspectMode::Aspect4By3.source_size(1600, 1024),
            [1600.0, 1200.0]
        );
    }

    #[test]
    fn one_to_one_aspect_uses_active_dimensions() {
        assert_eq!(
            DisplayAspectMode::Aspect1By1.source_size(1024, 768),
            [1024.0, 768.0]
        );
        assert_eq!(
            DisplayAspectMode::Aspect1By1.source_size(1280, 1024),
            [1280.0, 1024.0]
        );
        assert_eq!(
            DisplayAspectMode::Aspect1By1.source_size(1600, 1024),
            [1600.0, 1024.0]
        );
    }

    #[test]
    fn display_aspect_ratio_uses_corrected_source_size() {
        let ratio = DisplayAspectMode::Aspect4By3.display_aspect_ratio(1600, 1024);
        assert_eq!(ratio, 4.0 / 3.0);
    }
}
