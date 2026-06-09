use common::Context;
use sdl3::gpu::{
    GpuDevice, GpuGraphicsPipeline, GraphicsPipelineDescriptor, SDL_GPU_CULLMODE_NONE,
    SDL_GPU_FILLMODE_FILL, SDL_GPU_FRONTFACE_COUNTER_CLOCKWISE, SDL_GPU_PRIMITIVETYPE_TRIANGLELIST,
    SDL_GPU_SAMPLECOUNT_1, SDL_GPU_SHADERSTAGE_FRAGMENT, SDL_GPU_SHADERSTAGE_VERTEX,
    SDL_GPUColorTargetBlendState, SDL_GPUColorTargetDescription, SDL_GPUDepthStencilState,
    SDL_GPUGraphicsPipelineTargetInfo, SDL_GPUMultisampleState, SDL_GPURasterizerState,
    SDL_GPUTextureFormat, SDL_GPUVertexInputState, ShaderDescriptor,
};

use crate::{Error, Result};

/// Scaling mode value pushed via the fragment uniform. Must match
/// `SCALE_MODE_*` constants in `shaders/passes/present/present.frag.slang`.
#[derive(Copy, Clone, Debug)]
#[repr(u32)]
pub(crate) enum ScaleMode {
    Nearest = 0,
    Bilinear = 1,
    Pixelart = 2,
    #[allow(dead_code)]
    Crt = 3,
}

/// CPU mirror of the fragment `ConstantBuffer<PresentUniforms>` at set 3, binding 0.
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub(crate) struct PresentUniforms {
    output_size: [f32; 2],
    source_size: [f32; 2],
    source_used_size: [f32; 2],
    source_max_size: [f32; 2],
    scale_mode: u32,
    is_srgb_swapchain: u32,
    padding: [u32; 2],
}

impl PresentUniforms {
    pub(crate) fn new(
        output_size: (u32, u32),
        source_size: [f32; 2],
        source_used_size: [f32; 2],
        source_max_size: [f32; 2],
        scale_mode: ScaleMode,
        is_srgb_swapchain: bool,
    ) -> Self {
        Self {
            output_size: [output_size.0 as f32, output_size.1 as f32],
            source_size,
            source_used_size,
            source_max_size,
            scale_mode: scale_mode as u32,
            is_srgb_swapchain: u32::from(is_srgb_swapchain),
            padding: [0; 2],
        }
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        // Safety: PresentUniforms is repr(C) with no padding/uninit fields.
        unsafe {
            std::slice::from_raw_parts(
                (self as *const Self) as *const u8,
                std::mem::size_of::<Self>(),
            )
        }
    }
}

// Per-target shader artifacts committed under shaders/, produced by the
// `compile_shaders` developer tool (cargo run -p sdl3_backend --bin compile_shaders).
#[cfg(target_os = "windows")]
mod shader_bytes {
    pub(super) const VERT_DXIL: &[u8] = include_bytes!("../shaders/present.vert.dxil");
    pub(super) const FRAG_DXIL: &[u8] = include_bytes!("../shaders/present.frag.dxil");
    pub(super) const VERT_SPIRV: &[u8] = include_bytes!("../shaders/present.vert.spv");
    pub(super) const FRAG_SPIRV: &[u8] = include_bytes!("../shaders/present.frag.spv");
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod shader_bytes {
    pub(super) const VERT_METALLIB: &[u8] = include_bytes!("../shaders/present.vert.metallib");
    pub(super) const FRAG_METALLIB: &[u8] = include_bytes!("../shaders/present.frag.metallib");
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "ios")))]
mod shader_bytes {
    pub(super) const VERT_SPIRV: &[u8] = include_bytes!("../shaders/present.vert.spv");
    pub(super) const FRAG_SPIRV: &[u8] = include_bytes!("../shaders/present.frag.spv");
}

/// Selects the shader format that the device accepts and returns the matching bytecode.
fn pick_shader_bytecode(
    device: &GpuDevice,
) -> Result<(sdl3::gpu::SDL_GPUShaderFormat, &'static [u8], &'static [u8])> {
    let formats = device.shader_formats();

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    if (formats.0 & sdl3::gpu::SDL_GPU_SHADERFORMAT_METALLIB.0) != 0 {
        return Ok((
            sdl3::gpu::SDL_GPU_SHADERFORMAT_METALLIB,
            shader_bytes::VERT_METALLIB,
            shader_bytes::FRAG_METALLIB,
        ));
    }

    #[cfg(target_os = "windows")]
    if (formats.0 & sdl3::gpu::SDL_GPU_SHADERFORMAT_DXIL.0) != 0 {
        return Ok((
            sdl3::gpu::SDL_GPU_SHADERFORMAT_DXIL,
            shader_bytes::VERT_DXIL,
            shader_bytes::FRAG_DXIL,
        ));
    }

    #[cfg(any(
        target_os = "windows",
        not(any(target_os = "macos", target_os = "ios"))
    ))]
    if (formats.0 & sdl3::gpu::SDL_GPU_SHADERFORMAT_SPIRV.0) != 0 {
        return Ok((
            sdl3::gpu::SDL_GPU_SHADERFORMAT_SPIRV,
            shader_bytes::VERT_SPIRV,
            shader_bytes::FRAG_SPIRV,
        ));
    }

    let _ = formats;
    Err(Error::Message(common::StringError(
        "SDL3 GPU device does not accept any shader format produced by compile_shaders".to_string(),
    )))
}

/// Builds the fullscreen present pipeline against the swapchain texture format.
pub(crate) fn build(
    device: &GpuDevice,
    swapchain_format: SDL_GPUTextureFormat,
) -> Result<GpuGraphicsPipeline> {
    let (format, vert_code, frag_code) = pick_shader_bytecode(device)?;

    let vertex_shader = device
        .create_shader(&ShaderDescriptor {
            code: vert_code,
            entrypoint: c"vs_main",
            format,
            stage: SDL_GPU_SHADERSTAGE_VERTEX,
            num_samplers: 0,
            num_storage_textures: 0,
            num_storage_buffers: 0,
            num_uniform_buffers: 0,
        })
        .context("SDL_CreateGPUShader vertex failed")?;
    let fragment_shader = device
        .create_shader(&ShaderDescriptor {
            code: frag_code,
            entrypoint: c"fs_main",
            format,
            stage: SDL_GPU_SHADERSTAGE_FRAGMENT,
            num_samplers: 2,
            num_storage_textures: 0,
            num_storage_buffers: 0,
            num_uniform_buffers: 1,
        })
        .context("SDL_CreateGPUShader fragment failed")?;

    let color_targets = [SDL_GPUColorTargetDescription {
        format: swapchain_format,
        blend_state: SDL_GPUColorTargetBlendState::default(),
    }];

    let pipeline = device
        .create_graphics_pipeline(&GraphicsPipelineDescriptor {
            vertex_shader: &vertex_shader,
            fragment_shader: &fragment_shader,
            vertex_input_state: SDL_GPUVertexInputState::default(),
            primitive_type: SDL_GPU_PRIMITIVETYPE_TRIANGLELIST,
            rasterizer_state: SDL_GPURasterizerState {
                fill_mode: SDL_GPU_FILLMODE_FILL,
                cull_mode: SDL_GPU_CULLMODE_NONE,
                front_face: SDL_GPU_FRONTFACE_COUNTER_CLOCKWISE,
                enable_depth_bias: false,
                enable_depth_clip: true,
                ..Default::default()
            },
            multisample_state: SDL_GPUMultisampleState {
                sample_count: SDL_GPU_SAMPLECOUNT_1,
                ..Default::default()
            },
            depth_stencil_state: SDL_GPUDepthStencilState::default(),
            target_info: SDL_GPUGraphicsPipelineTargetInfo {
                color_target_descriptions: color_targets.as_ptr(),
                num_color_targets: color_targets.len() as u32,
                has_depth_stencil_target: false,
                ..Default::default()
            },
        })
        .context("SDL_CreateGPUGraphicsPipeline failed")?;

    Ok(pipeline)
}
