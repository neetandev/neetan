//! Legacy SDL3 rendering backend built on the 2D `SDL_Renderer`.
//!
//! Kept as a fallback for hosts where the SDL3 GPU API is unavailable or
//! the [`SdlGpuBackend`](crate::ModernSdlGpuBackend) fails to initialize. Functionally
//! a simple streaming-texture blit with letterboxing. The overlay is drawn by
//! the CPU rasterizer of the `gui` crate into a second streaming texture.

use common::{Context, OptionContext, bail, error, info, warn};
use gui::Rasterizer;
use sdl3::{
    info::version as sdl_version,
    render::{BlendMode, PixelFormat, Rect, Renderer, ScaleMode, Texture},
    video::Window,
};

use crate::{
    DisplayAspectMode, GraphicsEngine, Overlay, RenderInstructions, Result, Scaling,
    compute_color_target_extent, native_target_size,
};

/// Renderer state that exists only between `on_resume` and `on_destroy_surface`.
struct SdlState {
    /// The streaming texture of the emulator frame.
    texture: Texture,
    /// The streaming texture of the overlay.
    overlay: OverlaySlot,
    /// The renderer. It drops after the textures.
    renderer: Renderer,
}

/// The overlay texture of the renderer.
enum OverlaySlot {
    /// No overlay texture exists.
    Empty,
    /// A texture in the size of the render output.
    Ready(OverlayTexture),
    /// The overlay failed at this output size. It stays off until the size
    /// changes.
    Disabled {
        /// The output width in pixels.
        width: u32,
        /// The output height in pixels.
        height: u32,
    },
}

/// A streaming texture that holds the rasterized overlay.
struct OverlayTexture {
    /// The texture in the size of the render output.
    texture: Texture,
    /// The width in pixels.
    width: u32,
    /// The height in pixels.
    height: u32,
}

/// SDL 2D rendering backend.
pub struct LegacySdlBackend {
    aspect_mode: DisplayAspectMode,
    large_native_target: bool,
    state: Option<SdlState>,
    scaling: Scaling,
    /// `SDL_SCALEMODE_PIXELART` was introduced in SDL 3.4.
    pixelart_supported: bool,
    /// Draws the overlay scene on the CPU.
    overlay_rasterizer: Rasterizer,
    /// The overlay pixels of the last rasterized frame.
    overlay_pixels: Vec<u8>,
}

impl LegacySdlBackend {
    /// Creates a new SDL graphics engine. The renderer itself is created in `on_resume`.
    pub fn new(aspect_mode: DisplayAspectMode, large_native_target: bool) -> Self {
        let (major, minor, patch) = sdl_version();
        let pixelart_supported = (major, minor) >= (3, 4);
        if !pixelart_supported {
            warn!(
                "SDL {major}.{minor}.{patch} detected (< 3.4); Scaling::Pixelart will use Nearest fallback"
            );
        }

        Self {
            aspect_mode,
            large_native_target,
            state: None,
            scaling: Scaling::Pixelart,
            pixelart_supported,
            overlay_rasterizer: Rasterizer::new(),
            overlay_pixels: Vec::new(),
        }
    }

    /// Returns the SDL scale mode of the emulator frame texture.
    fn sdl_scale_mode(&self, scaling: Scaling) -> ScaleMode {
        match scaling {
            Scaling::Nearest => ScaleMode::Nearest,
            Scaling::Bilinear => ScaleMode::Linear,
            Scaling::Pixelart => {
                if self.pixelart_supported {
                    ScaleMode::Pixelart
                } else {
                    ScaleMode::Nearest
                }
            }
        }
    }
}

impl GraphicsEngine for LegacySdlBackend {
    fn on_resume(&mut self, window: &mut Window, vsync_enabled: bool) -> Result<()> {
        if self.state.is_some() {
            return Ok(());
        }

        let renderer = window
            .create_renderer()
            .context("SDL_CreateRenderer failed")?;
        renderer
            .set_vsync(vsync_enabled)
            .context("SDL_SetRenderVSync failed")?;

        let (target_width, target_height) = native_target_size(self.large_native_target);
        let texture = renderer
            .create_streaming_texture(PixelFormat::Rgba32, target_width, target_height)
            .context("SDL_CreateTexture failed")?;
        texture
            .set_scale_mode(self.sdl_scale_mode(self.scaling))
            .context("SDL_SetTextureScaleMode failed")?;

        info!("SDL Renderer backend initialized");

        self.state = Some(SdlState {
            texture,
            overlay: OverlaySlot::Empty,
            renderer,
        });

        Ok(())
    }

    fn on_destroy_surface(&mut self) {
        self.state = None;
    }

    fn render_frame(
        &mut self,
        window: &Window,
        render_instructions: Option<&RenderInstructions>,
        overlay: Option<Overlay<'_>>,
    ) -> Result<bool> {
        let state = self
            .state
            .as_mut()
            .context("SDL renderer not initialized")?;

        let (output_width, output_height) = state
            .renderer
            .output_size()
            .context("SDL_GetRenderOutputSize failed")?;

        if output_width == 0 || output_height == 0 {
            return Ok(true);
        }
        let stale = (output_width, output_height) != window.size_in_pixels();

        state
            .renderer
            .clear_with_color(0, 0, 0, 255)
            .context("SDL_RenderClear failed")?;

        if let Some(instructions) = render_instructions {
            let width = instructions.width.max(1);
            let height = instructions.height.max(1);
            let (max_width, max_height) = native_target_size(self.large_native_target);
            if width > max_width || height > max_height {
                panic!("Frame {width}x{height} exceeds backend target {max_width}x{max_height}");
            }
            let aspect_ratio = self.aspect_mode.display_aspect_ratio(width, height);

            let expected_bytes = (width * height * 4) as usize;
            if instructions.framebuffer.len() < expected_bytes {
                return Ok(true);
            }

            let update_rect = Rect {
                x: 0,
                y: 0,
                w: width as i32,
                h: height as i32,
            };
            state
                .texture
                .update(
                    Some(update_rect),
                    &instructions.framebuffer[..expected_bytes],
                    width * 4,
                )
                .context("SDL_UpdateTexture failed")?;

            let src = Rect {
                x: 0,
                y: 0,
                w: width as i32,
                h: height as i32,
            };
            let dst = fitted_destination_rect(output_width, output_height, aspect_ratio);

            state
                .renderer
                .render_texture(&state.texture, Some(src), Some(dst))
                .context("SDL_RenderTexture failed")?;
        }

        match overlay {
            Some(overlay) => {
                if let Err(error) = draw_overlay(
                    state,
                    &mut self.overlay_rasterizer,
                    &mut self.overlay_pixels,
                    overlay,
                    output_width,
                    output_height,
                ) {
                    error!("Overlay disabled at {output_width}x{output_height}: {error:#}");
                    state.overlay = OverlaySlot::Disabled {
                        width: output_width,
                        height: output_height,
                    };
                    self.overlay_pixels = Vec::new();
                    self.overlay_rasterizer.invalidate();
                }
            }
            None => {
                state.overlay = OverlaySlot::Empty;
                self.overlay_pixels = Vec::new();
                self.overlay_rasterizer.invalidate();
            }
        }

        state
            .renderer
            .present()
            .context("SDL_RenderPresent failed")?;

        Ok(stale)
    }

    fn set_scaling(&mut self, scaling: Scaling) {
        self.scaling = scaling;
        let mode = self.sdl_scale_mode(scaling);
        if let Some(state) = self.state.as_ref()
            && let Err(error) = state.texture.set_scale_mode(mode)
        {
            error!("Failed to set SDL texture scale mode: {error}");
        }
    }

    fn set_display_aspect_mode(&mut self, aspect_mode: DisplayAspectMode) {
        self.aspect_mode = aspect_mode;
    }

    fn set_large_native_target(&mut self, large_native_target: bool) -> Result<()> {
        if large_native_target == self.large_native_target {
            return Ok(());
        }
        let scale_mode = self.sdl_scale_mode(self.scaling);
        if let Some(state) = self.state.as_mut() {
            let (width, height) = native_target_size(large_native_target);
            let texture = state
                .renderer
                .create_streaming_texture(PixelFormat::Rgba32, width, height)
                .context("SDL_CreateTexture failed")?;
            texture
                .set_scale_mode(scale_mode)
                .context("SDL_SetTextureScaleMode failed")?;
            state.texture = texture;
        }
        self.large_native_target = large_native_target;
        let (width, height) = native_target_size(large_native_target);
        info!("Native target set to {width}x{height}");
        Ok(())
    }
}

/// Rasterizes the overlay and draws it 1:1 over the whole render output.
///
/// The renderer uses no logical presentation, viewport or render scale. Its
/// render coordinates are the pixels of the output. The texture is uploaded
/// only when the rasterized pixels changed or the texture was recreated.
fn draw_overlay(
    state: &mut SdlState,
    rasterizer: &mut Rasterizer,
    pixels: &mut Vec<u8>,
    overlay: Overlay<'_>,
    output_width: u32,
    output_height: u32,
) -> Result<()> {
    if overlay.scene.is_empty() && overlay.scene.clear_color().a == 0 {
        return Ok(());
    }

    match &state.overlay {
        OverlaySlot::Ready(existing)
            if existing.width == output_width && existing.height == output_height => {}
        OverlaySlot::Disabled { width, height }
            if *width == output_width && *height == output_height =>
        {
            return Ok(());
        }
        _ => {
            state.overlay = OverlaySlot::Empty;
            let texture = create_overlay_texture(&state.renderer, output_width, output_height)?;
            rasterizer.invalidate();
            state.overlay = OverlaySlot::Ready(texture);
        }
    }
    let OverlaySlot::Ready(overlay_texture) = &mut state.overlay else {
        return Ok(());
    };

    pixels.resize(output_width as usize * output_height as usize * 4, 0);
    let changed = rasterizer.rasterize(
        overlay.scene,
        overlay.slug,
        output_width,
        output_height,
        pixels,
    );
    if changed {
        overlay_texture
            .texture
            .update(None, pixels, output_width * 4)
            .context("SDL_UpdateTexture failed for the overlay")?;
    }

    state
        .renderer
        .render_texture(&overlay_texture.texture, None, None)
        .context("SDL_RenderTexture failed for the overlay")?;
    Ok(())
}

/// Creates the overlay texture with premultiplied blending in the size of the
/// render output. Fails when the output exceeds the maximum texture size.
fn create_overlay_texture(renderer: &Renderer, width: u32, height: u32) -> Result<OverlayTexture> {
    if let Some(maximum) = renderer.max_texture_size()
        && (width > maximum || height > maximum)
    {
        bail!("the output exceeds the maximum texture size of {maximum} pixels");
    }
    let texture = renderer
        .create_streaming_texture(PixelFormat::Rgba32, width, height)
        .context("SDL_CreateTexture failed for the overlay")?;
    texture
        .set_blend_mode(BlendMode::BlendPremultiplied)
        .context("SDL_SetTextureBlendMode failed")?;
    texture
        .set_scale_mode(ScaleMode::Nearest)
        .context("SDL_SetTextureScaleMode failed")?;
    Ok(OverlayTexture {
        texture,
        width,
        height,
    })
}

/// Returns the letterboxed destination of the emulator frame in output pixels.
fn fitted_destination_rect(output_width: u32, output_height: u32, aspect_ratio: f64) -> Rect {
    let (fitted_width, fitted_height) =
        compute_color_target_extent(output_width, output_height, aspect_ratio);
    let offset_x = ((output_width - fitted_width) / 2) as i32;
    let offset_y = ((output_height - fitted_height) / 2) as i32;
    Rect {
        x: offset_x,
        y: offset_y,
        w: fitted_width as i32,
        h: fitted_height as i32,
    }
}
