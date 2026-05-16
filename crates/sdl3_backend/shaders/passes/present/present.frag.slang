#language slang 2026

import screen_space;
import filter;
import crt;
import transform;

struct PresentUniforms {
    float2 output_size;
    float2 source_size;
    float2 source_used_size;
    float2 source_max_size;
    uint scale_mode;
    uint is_srgb_swapchain;
    uint2 padding;
}

static const uint SCALE_MODE_NEAREST = 0;
static const uint SCALE_MODE_BILINEAR = 1;
static const uint SCALE_MODE_PIXELART = 2;
static const uint SCALE_MODE_CRT = 3;

// Bindings follows the SDL3 GPU SPIR-V binding convention.
//
// Reference: https://wiki.libsdl.org/SDL3/SDL_CreateGPUShader
[[vk::binding(0, 2)]] var native_nearest: Sampler2D;
[[vk::binding(1, 2)]] var native_linear: Sampler2D;
[[vk::binding(0, 3)]] ConstantBuffer<PresentUniforms> uniforms;

[[shader("pixel")]]
float4 fs_main(FullscreenVertex input) : SV_Target {
    let output_size = max(uniforms.output_size, float2(1.0, 1.0));
    let source_size = max(uniforms.source_size, float2(1.0, 1.0));
    let source_used_size = max(uniforms.source_used_size, float2(1.0, 1.0));
    let source_max_size = max(uniforms.source_max_size, float2(1.0, 1.0));
    let source_max_size_inv = 1.0 / source_max_size;

    // Aspect-correct letterbox/pillarbox inside output_size.
    let source_aspect = source_size.x / source_size.y;
    let output_aspect = output_size.x / output_size.y;

    var rect_size: float2;
    if (output_aspect > source_aspect) {
        rect_size = float2(output_size.y * source_aspect, output_size.y);
    } else {
        rect_size = float2(output_size.x, output_size.x / source_aspect);
    }
    let rect_offset = (output_size - rect_size) * 0.5;

    // Swapchain pixel coord from full-screen-triangle UV.
    let pixel_coord = input.uv * output_size;
    let local_pixel = pixel_coord - rect_offset;

    if (any(local_pixel < float2(0.0)) || any(local_pixel >= rect_size)) {
        return float4(0.0, 0.0, 0.0, 1.0);
    }

    // Region UV is [0, 1] over the visible letterbox area.
    let region_uv = local_pixel / rect_size;

    // Texture UV maps the visible source into the top-left area of the
    // backing texture. Unused rows/columns may contain stale pixels.
    let tex_uv = region_uv * (source_used_size / source_max_size);

    var rgb: float3;

    switch (uniforms.scale_mode) {
    case SCALE_MODE_NEAREST:
        rgb = native_nearest.SampleLevel(tex_uv, 0.0).xyz;
        break;
    case SCALE_MODE_BILINEAR:
        rgb = native_linear.SampleLevel(tex_uv, 0.0).xyz;
        break;
    case SCALE_MODE_PIXELART: {
        let pixelart_uv = compute_pixelart_uv(tex_uv, source_max_size, source_max_size_inv);
        rgb = native_linear.SampleLevel(pixelart_uv, 0.0).xyz;
        break;
    }
    case SCALE_MODE_CRT: {
        // CRT module wants region UV (already 0..1 over the displayed area)
        // plus the source pixel dims and the actual displayed (letterbox) size.
        rgb = sample_crt_pixel(native_linear, region_uv, source_used_size, rect_size, source_max_size);
        break;
    }
    default:
        rgb = float3(0.0, 0.0, 0.0);
        break;
    }

    if (uniforms.is_srgb_swapchain == 0) {
        rgb = color::linear_to_srgb(rgb);
    }

    return float4(rgb, 1.0);
}
