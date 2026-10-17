//! The graphics pipeline that draws the user interface overlay.

use common::Context;
use sdl3::gpu::{
    GpuDevice, GpuGraphicsPipeline, GraphicsPipelineDescriptor, SDL_GPU_BLENDFACTOR_ONE,
    SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA, SDL_GPU_BLENDOP_ADD, SDL_GPU_CULLMODE_NONE,
    SDL_GPU_FILLMODE_FILL, SDL_GPU_FRONTFACE_COUNTER_CLOCKWISE, SDL_GPU_PRIMITIVETYPE_TRIANGLELIST,
    SDL_GPU_SAMPLECOUNT_1, SDL_GPU_SHADERSTAGE_FRAGMENT, SDL_GPU_SHADERSTAGE_VERTEX,
    SDL_GPUColorTargetBlendState, SDL_GPUColorTargetDescription, SDL_GPUDepthStencilState,
    SDL_GPUGraphicsPipelineTargetInfo, SDL_GPUMultisampleState, SDL_GPURasterizerState,
    SDL_GPUTextureFormat, SDL_GPUVertexInputState, ShaderDescriptor,
};

use crate::{
    Result,
    pipeline::{PassBinaries, pass_binaries, pick_shader_bytecode},
};

/// The binaries of the user interface pass.
const UI_BINARIES: PassBinaries = pass_binaries!("ui");

/// The number of vertices of the quad of one instance.
pub(crate) const VERTICES_PER_INSTANCE: u32 = 6;

/// The number of storage buffers of the fragment stage: the Slug curves,
/// bands and glyphs.
const FRAGMENT_STORAGE_BUFFERS: u32 = 3;

/// CPU mirror of `UiUniforms` in `shaders/modules/ui_common.slang`. Both stages
/// read it from uniform slot 0. The std140 layout of the four scalars has no
/// padding.
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(C)]
pub(crate) struct UiUniforms {
    /// The width and the height of the target in physical pixels.
    pub(crate) size: [f32; 2],
    /// 1 when the shader encodes its output in sRGB.
    pub(crate) encode_srgb: u32,
    /// The number of records in the glyph buffer.
    pub(crate) glyph_count: u32,
}

const _: () = assert!(size_of::<UiUniforms>() == 16);

impl UiUniforms {
    /// The bytes of the uniforms in native byte order.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        // Safety: UiUniforms is repr(C) with four 4-byte fields and no padding.
        unsafe { std::slice::from_raw_parts((self as *const Self) as *const u8, size_of::<Self>()) }
    }
}

/// Builds the user interface pipeline for the target format `target_format`.
///
/// The pipeline has no vertex input. The vertex stage builds the quads from
/// the instance storage buffer. The color output is premultiplied and blends
/// with `ONE, ONE_MINUS_SRC_ALPHA`.
pub(crate) fn build(
    device: &GpuDevice,
    target_format: SDL_GPUTextureFormat,
) -> Result<GpuGraphicsPipeline> {
    let (format, vertex_code, fragment_code) = pick_shader_bytecode(device, &UI_BINARIES)?;

    let vertex_shader = device
        .create_shader(&ShaderDescriptor {
            code: vertex_code,
            entrypoint: c"vs_main",
            format,
            stage: SDL_GPU_SHADERSTAGE_VERTEX,
            num_samplers: 0,
            num_storage_textures: 0,
            num_storage_buffers: 1,
            num_uniform_buffers: 1,
        })
        .context("SDL_CreateGPUShader failed for the user interface vertex shader")?;
    let fragment_shader = device
        .create_shader(&ShaderDescriptor {
            code: fragment_code,
            entrypoint: c"fs_main",
            format,
            stage: SDL_GPU_SHADERSTAGE_FRAGMENT,
            num_samplers: 1,
            num_storage_textures: 0,
            num_storage_buffers: FRAGMENT_STORAGE_BUFFERS,
            num_uniform_buffers: 1,
        })
        .context("SDL_CreateGPUShader failed for the user interface fragment shader")?;

    let color_targets = [SDL_GPUColorTargetDescription {
        format: target_format,
        blend_state: SDL_GPUColorTargetBlendState {
            src_color_blendfactor: SDL_GPU_BLENDFACTOR_ONE,
            dst_color_blendfactor: SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA,
            color_blend_op: SDL_GPU_BLENDOP_ADD,
            src_alpha_blendfactor: SDL_GPU_BLENDFACTOR_ONE,
            dst_alpha_blendfactor: SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA,
            alpha_blend_op: SDL_GPU_BLENDOP_ADD,
            enable_blend: true,
            ..Default::default()
        },
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
        .context("SDL_CreateGPUGraphicsPipeline failed for the user interface")?;

    Ok(pipeline)
}
