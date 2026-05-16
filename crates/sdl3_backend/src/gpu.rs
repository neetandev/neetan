//! Primary SDL3 rendering backend built on the SDL3 GPU API.

use common::{Context, OptionContext, info};
use sdl3::{
    gpu::{
        ColorTargetDescriptor, GpuGraphicsPipeline, RawWindow, SDL_FColor, SDL_GPU_LOADOP_CLEAR,
        SDL_GPU_STOREOP_STORE, SDL_GPUSwapchainComposition, TextureRegion, TextureSamplerBinding,
        TextureTransferInfo,
    },
    info::version as sdl_version,
    video::Window,
};

use crate::{
    DisplayAspectMode, Error, GraphicsEngine, PC98_NATIVE_HEIGHT, PC98_NATIVE_WIDTH,
    RenderInstructions, Result, Scaling,
    device::{DeviceResources, create as create_device_resources},
    native_target_size, pipeline,
    pipeline::{PresentUniforms, ScaleMode},
};

/// SDL3 GPU API rendering backend.
pub struct ModernSdlGpuBackend {
    aspect_mode: DisplayAspectMode,
    ga_enabled: bool,
    scaling: Scaling,
    state: Option<RenderState>,
}

struct FrameSource<'a> {
    framebuffer: &'a [u8],
    width: u32,
    height: u32,
    source_size: [f32; 2],
    crt: bool,
}

/// Resources owned only between `on_resume` and `on_destroy_surface`.
struct RenderState {
    pipeline: GpuGraphicsPipeline,
    resources: DeviceResources,
    swapchain_is_srgb: bool,
    /// Raw pointer to the `SDL_Window` claimed by `resources.device`. The
    /// application owns the window and guarantees it outlives this state.
    window_ptr: *mut RawWindow,
}

impl Drop for RenderState {
    fn drop(&mut self) {
        // Safety: `window_ptr` was the pointer claimed in `on_resume`, and the
        // application guarantees the window outlives this state. The device
        // is still alive at this point (it drops last among the fields).
        unsafe {
            self.resources.device.release_window_raw(self.window_ptr);
        }
    }
}

impl ModernSdlGpuBackend {
    /// Creates a new SDL3 GPU API backend.
    ///
    /// Fails if the linked SDL3 library is older than 3.2.0, since the GPU
    /// API was only introduced with 3.2.0.
    pub fn new(aspect_mode: DisplayAspectMode, ga_enabled: bool) -> Result<Self> {
        let (major, minor, patch) = sdl_version();
        if (major, minor) < (3, 2) {
            return Err(Error::Message(common::StringError(format!(
                "SDL3 {major}.{minor}.{patch} detected (< 3.2); GPU API not available"
            ))));
        }

        Ok(Self {
            aspect_mode,
            ga_enabled,
            scaling: Scaling::Pixelart,
            state: None,
        })
    }
}

fn scale_mode_for(scaling: Scaling, crt: bool) -> ScaleMode {
    if crt {
        return ScaleMode::Crt;
    }
    match scaling {
        Scaling::Nearest => ScaleMode::Nearest,
        Scaling::Bilinear => ScaleMode::Bilinear,
        Scaling::Pixelart => ScaleMode::Pixelart,
    }
}

impl GraphicsEngine for ModernSdlGpuBackend {
    fn on_resume(&mut self, window: &mut Window, vsync_enabled: bool) -> Result<()> {
        if self.state.is_some() {
            return Ok(());
        }

        let resources = create_device_resources(window, vsync_enabled, self.ga_enabled)?;
        let pipeline = pipeline::build(&resources.device, resources.swapchain_format)?;
        let swapchain_is_srgb =
            resources.swapchain_composition == SDL_GPUSwapchainComposition::SDR_LINEAR;

        info!("SDL3 GPU API backend initialized");

        self.state = Some(RenderState {
            resources,
            pipeline,
            swapchain_is_srgb,
            window_ptr: window.raw(),
        });

        Ok(())
    }

    fn on_destroy_surface(&mut self) {
        // RenderState's Drop impl handles window release and resource teardown
        // in the correct order; assigning None fires it.
        self.state = None;
    }

    fn render_frame(
        &mut self,
        window: &Window,
        render_instructions: Option<&RenderInstructions>,
    ) -> Result<()> {
        let frame_source = render_instructions
            .map(|instructions| self.frame_source(instructions))
            .transpose()?;
        let state = self
            .state
            .as_mut()
            .context("SDL3 GPU API backend not initialized")?;

        let mut command_buffer = state
            .resources
            .device
            .acquire_command_buffer()
            .context("SDL_AcquireGPUCommandBuffer failed")?;

        let acquired = command_buffer
            .wait_and_acquire_swapchain_texture(window)
            .context("SDL_WaitAndAcquireGPUSwapchainTexture failed")?;

        let swapchain_texture = match acquired {
            Some(value) => value,
            None => {
                // Window is not currently presentable (e.g. minimized).
                // Still submit the empty command buffer per SDL3 contract.
                command_buffer
                    .submit()
                    .context("SDL_SubmitGPUCommandBuffer (no swapchain) failed")?;
                return Ok(());
            }
        };

        let output_width = swapchain_texture.width();
        let output_height = swapchain_texture.height();
        if output_width == 0 || output_height == 0 {
            command_buffer
                .submit()
                .context("SDL_SubmitGPUCommandBuffer (zero size) failed")?;
            return Ok(());
        }

        let source_size = frame_source.as_ref().map_or(
            self.aspect_mode
                .source_size(PC98_NATIVE_WIDTH, PC98_NATIVE_HEIGHT),
            |source| source.source_size,
        );
        let source_used_size = frame_source.as_ref().map_or(
            [PC98_NATIVE_WIDTH as f32, PC98_NATIVE_HEIGHT as f32],
            |source| [source.width as f32, source.height as f32],
        );
        let crt_enabled = frame_source.as_ref().is_some_and(|source| source.crt);

        if let Some(source) = frame_source.as_ref() {
            upload_framebuffer(state, &mut command_buffer, source)?;
        }

        let (max_width, max_height) = native_target_size(self.ga_enabled);
        let scale_mode = scale_mode_for(self.scaling, crt_enabled);
        let uniforms = PresentUniforms::new(
            (output_width, output_height),
            source_size,
            source_used_size,
            [max_width as f32, max_height as f32],
            scale_mode,
            state.swapchain_is_srgb,
        );
        // Fragment uniform data is per-command-buffer; push before draw.
        command_buffer.push_fragment_uniform_data(0, uniforms.as_bytes());

        let color_target = ColorTargetDescriptor {
            texture: &swapchain_texture,
            clear_color: SDL_FColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            load_op: SDL_GPU_LOADOP_CLEAR,
            store_op: SDL_GPU_STOREOP_STORE,
        };

        {
            let render_pass = command_buffer.begin_render_pass(&color_target);
            render_pass.bind_graphics_pipeline(&state.pipeline);
            let bindings = [
                TextureSamplerBinding {
                    texture: &state.resources.native_target,
                    sampler: &state.resources.nearest_sampler,
                },
                TextureSamplerBinding {
                    texture: &state.resources.native_target,
                    sampler: &state.resources.linear_sampler,
                },
            ];
            render_pass.bind_fragment_samplers(0, &bindings);
            render_pass.draw_primitives(3, 1, 0, 0);
        }

        command_buffer
            .submit()
            .context("SDL_SubmitGPUCommandBuffer failed")?;

        Ok(())
    }

    fn set_scaling(&mut self, scaling: Scaling) {
        self.scaling = scaling;
    }
}

impl ModernSdlGpuBackend {
    fn frame_source<'a>(&self, instructions: &'a RenderInstructions) -> Result<FrameSource<'a>> {
        let width = instructions.width.max(1);
        let height = instructions.height.max(1);
        let (max_width, max_height) = native_target_size(self.ga_enabled);
        if width > max_width || height > max_height {
            return Err(Error::Message(common::StringError(format!(
                "Frame {width}x{height} exceeds backend target {max_width}x{max_height}"
            ))));
        }
        let source_size = self.aspect_mode.source_size(width, height);
        Ok(FrameSource {
            framebuffer: instructions.framebuffer,
            width,
            height,
            source_size,
            crt: instructions.crt,
        })
    }
}

fn upload_framebuffer(
    state: &RenderState,
    command_buffer: &mut sdl3::gpu::GpuCommandBuffer,
    source_frame: &FrameSource,
) -> Result<()> {
    let expected_bytes = (source_frame.width * source_frame.height * 4) as usize;
    assert!(source_frame.framebuffer.len() >= expected_bytes);

    // Map the staging buffer, copy framebuffer bytes, unmap.
    let ptr = state
        .resources
        .device
        .map_transfer_buffer(&state.resources.transfer_buffer, true)
        .context("SDL_MapGPUTransferBuffer failed")?;
    // Safety: ptr points to at least expected_bytes of writable memory inside
    // a transfer buffer sized for the largest configured target; framebuffer
    // slice covers at least expected_bytes from its start.
    unsafe {
        std::ptr::copy_nonoverlapping(source_frame.framebuffer.as_ptr(), ptr, expected_bytes);
    }
    state
        .resources
        .device
        .unmap_transfer_buffer(&state.resources.transfer_buffer);

    let source = TextureTransferInfo {
        transfer_buffer: &state.resources.transfer_buffer,
        offset: 0,
        pixels_per_row: source_frame.width,
        rows_per_layer: source_frame.height,
    };
    let destination = TextureRegion {
        texture: &state.resources.native_target,
        mip_level: 0,
        layer: 0,
        x: 0,
        y: 0,
        z: 0,
        w: source_frame.width,
        h: source_frame.height,
        d: 1,
    };

    let copy_pass = command_buffer.begin_copy_pass();
    copy_pass.upload_to_texture(&source, &destination, false);
    drop(copy_pass);
    Ok(())
}
