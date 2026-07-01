use std::sync::LazyLock;

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
    #[allow(dead_code)]
    CrtComposite = 4,
}

/// Composite decode FIR parameters. FIR_TAPS/OVERSAMPLE must match the shader.
const COMPOSITE_FIR_TAPS: usize = 25;
const COMPOSITE_OVERSAMPLE: f32 = 2.0;
const COMPOSITE_LUMA_CUTOFF: f32 = 0.4;
const COMPOSITE_CHROMA_BANDWIDTH: f32 = 0.045;
const COMPOSITE_CHROMA_GAIN: f32 = 2.0;
/// Number of `float4`s needed to hold `COMPOSITE_FIR_TAPS` weights, 4 taps each.
const COMPOSITE_FIR_VEC4: usize = COMPOSITE_FIR_TAPS.div_ceil(4);

fn composite_sinc(x: f32) -> f32 {
    if x.abs() < 1e-5 {
        return 1.0;
    }
    let v = std::f32::consts::PI * x;
    v.sin() / v
}

fn composite_blackman(taps: usize, index: usize) -> f32 {
    let phase = std::f32::consts::TAU * index as f32 / (taps - 1) as f32;
    0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos()
}

/// Windowed-sinc low-pass tap weight, matching `lowpass()` in composite.slang.
fn composite_lowpass(cutoff: f32, taps: usize, index: usize) -> f32 {
    let center = index as f32 - (taps / 2) as f32;
    2.0 * cutoff * composite_blackman(taps, index) * composite_sinc(2.0 * cutoff * center)
}

/// The two precomputed, pre-normalized FIR weight tables, packed 4 taps per
/// `float4` so the layout matches the shader's `float4[]` constant-buffer arrays.
struct CompositeFir {
    luma: [[f32; 4]; COMPOSITE_FIR_VEC4],
    chroma: [[f32; 4]; COMPOSITE_FIR_VEC4],
}

/// Computed once: the weights depend only on compile-time constants.
static COMPOSITE_FIR: LazyLock<CompositeFir> = LazyLock::new(|| {
    let mut luma = [0.0f32; COMPOSITE_FIR_TAPS];
    let mut chroma = [0.0f32; COMPOSITE_FIR_TAPS];
    for tap in 0..COMPOSITE_FIR_TAPS {
        luma[tap] = composite_lowpass(
            COMPOSITE_LUMA_CUTOFF / COMPOSITE_OVERSAMPLE,
            COMPOSITE_FIR_TAPS,
            tap,
        );
        chroma[tap] = composite_lowpass(
            COMPOSITE_CHROMA_BANDWIDTH / COMPOSITE_OVERSAMPLE,
            COMPOSITE_FIR_TAPS,
            tap,
        );
    }

    // Pre-normalize so the shader loop needs no post-loop division: luma sums to
    // one; chroma sums to one and carries the quadrature-demod gain of two.
    let luma_sum: f32 = luma.iter().sum();
    let chroma_sum: f32 = chroma.iter().sum();
    for weight in &mut luma {
        *weight /= luma_sum;
    }
    for weight in &mut chroma {
        *weight = *weight / chroma_sum * COMPOSITE_CHROMA_GAIN;
    }

    let pack = |weights: &[f32; COMPOSITE_FIR_TAPS]| {
        let mut packed = [[0.0f32; 4]; COMPOSITE_FIR_VEC4];
        for (tap, &weight) in weights.iter().enumerate() {
            packed[tap / 4][tap % 4] = weight;
        }
        packed
    };

    CompositeFir {
        luma: pack(&luma),
        chroma: pack(&chroma),
    }
});

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
    composite_phase: u32,
    padding: u32,
    luma_weights: [[f32; 4]; COMPOSITE_FIR_VEC4],
    chroma_weights: [[f32; 4]; COMPOSITE_FIR_VEC4],
}

// The weight arrays must stay tightly packed (16-byte scalar block at offset 48,
// then two float4[] arrays at 16-byte stride) so the repr(C) layout matches the
// shader's std140 constant buffer. This trips if repr(C) inserts any padding.
const _: () = assert!(
    std::mem::size_of::<PresentUniforms>() == 48 + 2 * COMPOSITE_FIR_VEC4 * 16,
    "PresentUniforms must match the std140 constant-buffer layout",
);

impl PresentUniforms {
    pub(crate) fn new(
        output_size: (u32, u32),
        source_size: [f32; 2],
        source_used_size: [f32; 2],
        source_max_size: [f32; 2],
        scale_mode: ScaleMode,
        is_srgb_swapchain: bool,
        composite_phase: u32,
    ) -> Self {
        Self {
            output_size: [output_size.0 as f32, output_size.1 as f32],
            source_size,
            source_used_size,
            source_max_size,
            scale_mode: scale_mode as u32,
            is_srgb_swapchain: u32::from(is_srgb_swapchain),
            composite_phase,
            padding: 0,
            luma_weights: COMPOSITE_FIR.luma,
            chroma_weights: COMPOSITE_FIR.chroma,
        }
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        // Safety: PresentUniforms is repr(C) with no padding/uninit fields.
        unsafe { std::slice::from_raw_parts((self as *const Self) as *const u8, size_of::<Self>()) }
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
