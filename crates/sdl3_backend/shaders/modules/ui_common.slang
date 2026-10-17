#language slang 2026

// The records the vertex and the fragment stage of the user interface pass share.

module ui_common;

public static const uint KIND_RECTANGLE = 0;
public static const uint KIND_SHADOW = 1;
public static const uint KIND_ARC = 2;
public static const uint KIND_LINE = 3;
public static const uint KIND_GLYPH = 4;

// The uniforms of both stages. Must match `UiUniforms` in `ui_pipeline.rs`.
public struct UiUniforms {
    // The width and the height of the target in physical pixels.
    public float2 size;
    // 1 when the target stores the bytes of the color without an sRGB encoding by the hardware.
    public uint encode_srgb;
    // The number of records in the glyph buffer.
    public uint glyph_count;
};

// One draw item. Must match `Instance` in `crates/gui/src/scene.rs`.
public struct Instance {
    // The item rectangle as left, top, width and height.
    public float4 rect;
    // The clip rectangle as left, top, right and bottom.
    public float4 clip;
    // The main color, linear and premultiplied.
    public float4 color;
    // The stroke color of a rectangle.
    public float4 second_color;
    // The corner radii top left, top right, bottom right and bottom left.
    public float4 radii;
    // The parameters of the kind.
    public float4 parameters;
    // More parameters of the kind.
    public float4 second_parameters;
    // The kind and, for a glyph, the index of the glyph in the glyph buffer.
    public uint4 kind;
};

// The output of the vertex stage. The instance fields stay constant over the quad.
public struct UiVaryings {
    public float4 position : SV_Position;
    [[vk::location(0)]] public nointerpolation float4 rect : TEXCOORD0;
    [[vk::location(1)]] public nointerpolation float4 color : TEXCOORD1;
    [[vk::location(2)]] public nointerpolation float4 second_color : TEXCOORD2;
    [[vk::location(3)]] public nointerpolation float4 radii : TEXCOORD3;
    [[vk::location(4)]] public nointerpolation float4 parameters : TEXCOORD4;
    [[vk::location(5)]] public nointerpolation float4 second_parameters : TEXCOORD5;
    [[vk::location(6)]] public nointerpolation uint4 kind : TEXCOORD6;
};
