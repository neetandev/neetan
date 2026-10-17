#language slang 2026

// Draws every item of a scene with one instanced draw call. Each instance is a quad of six
// vertices that covers one item. All coordinates are physical pixels with the y-axis
// pointing down.

import ui_common;

// Bindings follow the SDL3 GPU binding convention of a vertex shader. Metal numbers the
// uniform buffers and the storage buffers in one range and takes the index of a register.
//
// Reference: https://wiki.libsdl.org/SDL3/SDL_CreateGPUShader
#ifdef TARGET_METAL
[[vk::binding(0, 0)]] StructuredBuffer<Instance> instances : register(t1, space0);
#else
[[vk::binding(0, 0)]] StructuredBuffer<Instance> instances : register(t0, space0);
#endif
[[vk::binding(0, 1)]] ConstantBuffer<UiUniforms> uniforms : register(b0, space1);

// The corners of the two triangles of a quad from 0 at the top left to 1 at the bottom right.
static const float2 CORNERS[6] = {
    float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
    float2(0.0, 1.0), float2(1.0, 0.0), float2(1.0, 1.0),
};

// The vertex `vertex_index` of the quad of the instance `instance_index`.
func build_quad(vertex_index: uint, instance_index: uint) -> UiVaryings {
    let instance = instances[instance_index];

    // The quad grows past the item to give the edge coverage room. A shadow grows by
    // three standard deviations of its blur.
    let margin = instance.kind.x == KIND_SHADOW ? 3.0 * instance.parameters.x + 1.0 : 1.0;
    let minimum = max(instance.rect.xy - margin, instance.clip.xy);
    let maximum = max(min(instance.rect.xy + instance.rect.zw + margin, instance.clip.zw), minimum);

    let pixel = lerp(minimum, maximum, CORNERS[vertex_index]);
    let clip_position = float2(pixel.x / uniforms.size.x * 2.0 - 1.0, 1.0 - pixel.y / uniforms.size.y * 2.0);

    var output: UiVaryings;
    output.position = float4(clip_position, 0.0, 1.0);
    output.rect = instance.rect;
    output.color = instance.color;
    output.second_color = instance.second_color;
    output.radii = instance.radii;
    output.parameters = instance.parameters;
    output.second_parameters = instance.second_parameters;
    output.kind = instance.kind;
    return output;
}

// SV_VertexID and SV_InstanceID on the SPIR-V target need OpCapability DrawParameters.
// The pass draws with a first vertex and a first instance of 0, so the raw Vulkan indices
// are equal.
#ifdef TARGET_SPIRV
[[shader("vertex")]]
func vs_main(vertex_index: uint : SV_VulkanVertexID, instance_index: uint : SV_VulkanInstanceID) -> UiVaryings {
    return build_quad(vertex_index, instance_index);
}
#else
[[shader("vertex")]]
func vs_main(vertex_index: uint : SV_VertexID, instance_index: uint : SV_InstanceID) -> UiVaryings {
    return build_quad(vertex_index, instance_index);
}
#endif
