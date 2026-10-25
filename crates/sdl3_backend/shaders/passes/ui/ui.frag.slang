#language slang 2026

// Computes the color of each pixel of an item from its exact geometry. The math mirrors the
// CPU rasterizer of the `gui` crate. All coordinates are physical pixels with the y-axis
// pointing down. The output is linear and premultiplied, or sRGB encoded and premultiplied
// when the target has no hardware sRGB encoding.

import sdf;
import shadow;
import slug;
import ui_common;

// Bindings follow the SDL3 GPU binding convention of a fragment shader. Metal numbers the
// uniform buffers and the storage buffers in one range, takes the index of a register and
// ignores its space. The storage buffers follow the one uniform buffer at index 0.
//
// Reference: https://wiki.libsdl.org/SDL3/SDL_CreateGPUShader
[[vk::binding(0, 2)]] Sampler2D<float> blue_noise : register(t0, space2) : register(s0, space2);
#ifdef TARGET_METAL
[[vk::binding(1, 2)]] StructuredBuffer<float4> slug_curves : register(t1);
[[vk::binding(2, 2)]] StructuredBuffer<uint> slug_bands : register(t2);
[[vk::binding(3, 2)]] StructuredBuffer<SlugGlyph> slug_glyphs : register(t3);
#else
[[vk::binding(1, 2)]] StructuredBuffer<float4> slug_curves : register(t1, space2);
[[vk::binding(2, 2)]] StructuredBuffer<uint> slug_bands : register(t2, space2);
[[vk::binding(3, 2)]] StructuredBuffer<SlugGlyph> slug_glyphs : register(t3, space2);
#endif
[[vk::binding(0, 3)]] ConstantBuffer<UiUniforms> uniforms : register(b0, space3);

// The width and the height of the blue noise tile.
static const int BLUE_NOISE_SIZE = 128;

// The luminance weights of the linear red, green and blue channels.
static const float3 LUMINANCE_WEIGHTS = float3(0.2126, 0.7152, 0.0722);

// The sRGB encoding of a linear color.
func linear_to_srgb(value: float3) -> float3 {
    return select(value <= 0.0031308, value * 12.92, 1.055 * pow(value, float3(1.0 / 2.4)) - 0.055);
}

// The linear color of an sRGB encoded color.
func srgb_to_linear(value: float3) -> float3 {
    return select(value <= 0.04045, value / 12.92, pow((value + 0.055) / 1.055, float3(2.4)));
}

// Adds blue noise of up to half an 8 bit step in sRGB to a premultiplied color. A smooth
// gradient over a few steps shows no bands with it.
func dither(color: float4, pixel: float2) -> float4 {
    if (color.a <= 0.0) {
        return color;
    }
    let texel = int2(pixel) % BLUE_NOISE_SIZE;
    let center = (float2(texel) + 0.5) / float(BLUE_NOISE_SIZE);
    let noise = blue_noise.SampleLevel(center, 0.0) + 0.5 / 255.0 - 0.5;
    let srgb = linear_to_srgb(saturate(color.rgb / color.a)) + noise / 255.0;
    return float4(srgb_to_linear(saturate(srgb)) * color.a, color.a);
}

// A rectangle with rounded corners, an inner stroke and a vertical gradient.
func rectangle(input: UiVaryings, pixel: float2) -> float4 {
    let half_size = input.rect.zw * 0.5;
    let distance = sdf_rounded_rectangle(pixel - input.rect.xy - half_size, half_size, input.radii);
    let outer = sdf_coverage(distance);
    if (outer <= 0.0) {
        return float4(0.0);
    }

    var fill = input.color;
    if (input.parameters.y > 0.5) {
        let t = saturate((pixel.y - input.rect.y) / input.rect.w);
        fill = dither(lerp(input.color, input.second_parameters, t), pixel);
    }

    let stroke_width = input.parameters.x;
    if (stroke_width > 0.0) {
        let inner = sdf_coverage(distance + stroke_width);
        return fill * inner + input.second_color * (outer - inner);
    }
    return fill * outer;
}

// The soft shadow of a rectangle with rounded corners.
func shadow(input: UiVaryings, pixel: float2) -> float4 {
    let half_size = input.rect.zw * 0.5;
    let position = pixel - input.rect.xy - half_size;
    let intensity = rounded_rectangle_shadow(position, half_size, input.radii.x, input.parameters.x);
    return input.color * intensity;
}

// A ring segment with round caps.
func arc(input: UiVaryings, pixel: float2) -> float4 {
    let distance = sdf_arc(pixel - input.parameters.xy, input.parameters.z, input.parameters.w, input.second_parameters.x, input.second_parameters.y);
    return input.color * sdf_coverage(distance);
}

// A line with round caps.
func line(input: UiVaryings, pixel: float2) -> float4 {
    let distance = sdf_line(pixel, input.parameters.xy, input.parameters.zw, input.second_parameters.x);
    return input.color * sdf_coverage(distance);
}

// The exponent of the coverage curve of black text. It approximates the sRGB transfer curve.
static const float DARK_TEXT_EXPONENT = 2.2;

// The factor on the luminance of the text color in the coverage curve. Text with a luminance
// of a quarter or more keeps the linear coverage.
static const float TEXT_LUMINANCE_FACTOR = 4.0;

// Blending in linear light thins dark text on a light background. The coverage of dark
// text follows the curve of a blend in sRGB. Light text keeps the linear coverage. The CPU
// rasterizer of the gui crate computes the same curve with the same constants.
func text_coverage(coverage: float, color: float4) -> float {
    let luminance = dot(color.rgb, LUMINANCE_WEIGHTS) / max(color.a, 1.0 / 255.0);
    let exponent = lerp(DARK_TEXT_EXPONENT, 1.0, saturate(luminance * TEXT_LUMINANCE_FACTOR));
    if (exponent == 1.0 || coverage >= 1.0) {
        return coverage;
    }
    return 1.0 - pow(1.0 - coverage, exponent);
}

// A Slug outline of a glyph or a path.
func glyph(input: UiVaryings, pixel: float2) -> float4 {
    if (input.kind.y >= uniforms.glyph_count) {
        return float4(0.0);
    }
    // The em coordinate is a linear function of the pixel. Its derivative is the em size
    // of one pixel.
    let em = input.parameters.xy + (pixel - input.rect.xy) * input.parameters.zw;
    let pixels_per_em = 1.0 / abs(input.parameters.zw);
    let coverage = slug_coverage(slug_curves, slug_bands, slug_glyphs[input.kind.y], em, pixels_per_em);
    if (coverage <= 0.0) {
        return float4(0.0);
    }
    return input.color * text_coverage(coverage, input.color);
}

// The color of the item at the pixel center `pixel`.
func shade(input: UiVaryings, pixel: float2) -> float4 {
    switch (input.kind.x) {
        case KIND_RECTANGLE:
            return rectangle(input, pixel);
        case KIND_SHADOW:
            return shadow(input, pixel);
        case KIND_ARC:
            return arc(input, pixel);
        case KIND_LINE:
            return line(input, pixel);
        case KIND_GLYPH:
            return glyph(input, pixel);
        default:
            return float4(0.0);
    }
}

[[shader("pixel")]]
float4 fs_main(UiVaryings input) : SV_Target {
    let color = shade(input, input.position.xy);
    if (uniforms.encode_srgb == 0 || color.a <= 0.0) {
        return color;
    }
    return float4(linear_to_srgb(saturate(color.rgb / color.a)) * color.a, color.a);
}
