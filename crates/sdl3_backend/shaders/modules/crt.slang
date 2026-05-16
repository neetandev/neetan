#language slang 2026

module crt;

// CRT shader emulating typical NEC monitors (dot pitch, dot-type black matrix).
//
// Single-pass flat-surface CRT with:
//   - Lanczos2 center row / 3-tap Gaussian adjacent rows with analog bandwidth sharpness.
//   - Per-channel brightness-adaptive scanlines.
//   - Luminance-adaptive dot-triad black-matrix phosphor mask.

static const float PI = 3.14159265359;

// Horizontal Lanczos2 sharpness: 0.0 = pure sinc interpolation, 1.0 = hard pixel edges.
// NEC monitors had sharp analog bandwidth at 640 pixels, so moderate values work well.
static const float HORIZONTAL_SHARPNESS = 0.5;
// Scanline Gaussian shape exponent. 2.0 = true Gaussian, higher = flatter top with steeper rolloff.
static const float SHAPE = 2.0;
// Horizontal Gaussian sharpness for off-center scanline rows. More negative = sharper.
static const float HARD_PIX = -3.0;

// Scanline Gaussian sharpness. More negative = harder scanlines.
static const float HARD_SCAN = -4.0;
// Minimum beam width multiplier (dark pixels).
static const float SCAN_BEAM_MIN = 0.7;
// Maximum beam width multiplier (bright pixels).
static const float SCAN_BEAM_MAX = 1.05;

// Mask strength: 0.0 = no mask, 1.0 = full physical mask contrast.
static const float MASK_STRENGTH = 0.15;
// Source pixels per R+G+B triad (physical dot pitch of the simulated monitor).
static const float MASK_TRIAD_WIDTH = 1.5;
// Ratio of lit phosphor area within each cell.
static const float MASK_DOT_RADIUS = 0.75;
// Luminance threshold above which mask fades out (bright phosphors bloom through gaps).
static const float MASK_CUTOFF = 0.3;

// Bright boost for dark pixels (compensates scanline energy loss).
static const float BRIGHT_BOOST_DARK = 1.35;
// Bright boost for bright pixels.
static const float BRIGHT_BOOST_BRIGHT = 1.10;

// Shoulder compress: linear passthrough below knee, exponential
// soft-clip above. Asymptotes to 1.0. Values below knee are untouched.
[ForceInline]
func shoulder_compress(x: float, knee: float) -> float {
    if (x <= knee) { return x; }
    let range = 1.0 - knee;
    return knee + range * (1.0 - exp(-(x - knee) / range));
}

// Hue-preserving phosphor saturation: scales all channels by the
// same ratio so color ratios (hue) are maintained.
[ForceInline]
func phosphor_saturate(color: float3, knee: float) -> float3 {
    let peak = max(color.r, max(color.g, color.b));
    if (peak <= knee) { return color; }
    return color * (shoulder_compress(peak, knee) / peak);
}

[ForceInline]
func source_domain_to_backing_uv(uv: float2, source_size: float2, backing_size: float2) -> float2 {
    return uv * (source_size / backing_size);
}

[ForceInline]
func fetch(
    source: Sampler2D,
    pos: float2,
    offset: float2,
    source_size: float2,
    backing_size: float2
) -> float3 {
    let source_pixel = floor(pos * source_size + offset);
    if (any(source_pixel < float2(0.0)) || any(source_pixel >= source_size)) {
        return float3(0.0);
    }

    let sample_uv = (source_pixel + float2(0.5)) / source_size;
    let backing_uv = source_domain_to_backing_uv(sample_uv, source_size, backing_size);
    return source.SampleLevel(backing_uv, 0.0).xyz;
}

[ForceInline]
func distance_to_nearest_texel(pos: float2, source_size: float2) -> float2 {
    let scaled_pos = pos * source_size;
    return -((scaled_pos - floor(scaled_pos)) - float2(0.5));
}

// Half-circle S-curve applied to fractional texel position. Controls how
// sharply pixels transition: 0.0 = smooth sinc interpolation, 1.0 = hard
// pixel edges that approach nearest-neighbor.
[ForceInline]
func curve_distance(x: float, sharp: float) -> float {
    let x_step = step(0.5, x);
    let curve = 0.5 - sqrt(0.25 - (x - x_step) * (x - x_step)) * sign(0.5 - x);
    return lerp(x, curve, sharp);
}

// 4-tap Lanczos2 filter along a horizontal scanline row. Produces sharper
// pixel transitions than Gaussian with less ringing, matching the high analog
// bandwidth of NEC PC-98 monitors at 640 horizontal pixels.
[ForceInline]
func horizontal_lanczos(
    source: Sampler2D,
    pos: float2,
    offset: float,
    source_size: float2,
    backing_size: float2
) -> float3 {
    // Shift to texel-centered coordinates so the base texel is always to the
    // left of the sample point, giving a stable fractional distance in [0, 1).
    let f = fract(pos.x * source_size.x);
    let shift = (f < 0.5) ? -1.0 : 0.0;
    let d = f - 0.5 - shift;

    let s0 = fetch(source, pos, float2(shift - 1.0, offset), source_size, backing_size);
    let s1 = fetch(source, pos, float2(shift, offset), source_size, backing_size);
    let s2 = fetch(source, pos, float2(shift + 1.0, offset), source_size, backing_size);
    let s3 = fetch(source, pos, float2(shift + 2.0, offset), source_size, backing_size);

    let curve_d = curve_distance(d, HORIZONTAL_SHARPNESS);

    // Lanczos2 kernel: sinc(x) * sinc(x/2), evaluated via the trig identity
    // L(x) = 2 * sin(pi*x) * sin(pi*x/2) / (pi*x)^2.
    var coeffs = float4(1.0 + curve_d, curve_d, 1.0 - curve_d, 2.0 - curve_d) * PI;
    coeffs = max(abs(coeffs), float4(1e-5));
    coeffs = 2.0 * sin(coeffs) * sin(coeffs * 0.5) / (coeffs * coeffs);
    coeffs /= dot(coeffs, float4(1.0));

    let result = s0 * coeffs.x + s1 * coeffs.y + s2 * coeffs.z + s3 * coeffs.w;

    // Anti-ringing clamp: restrict output to the range of the two nearest
    // samples, preventing visible overshoot from Lanczos negative lobes.
    return clamp(result, min(s1, s2), max(s1, s2));
}

// 3-tap Gaussian filter along a horizontal scanline row. Used for adjacent
// rows (offset -1/+1) where scanline weights heavily attenuate the signal,
// making full Lanczos2 quality unnecessary.
[ForceInline]
func horizontal_3tap(
    source: Sampler2D,
    pos: float2,
    offset: float,
    source_size: float2,
    backing_size: float2
) -> float3 {
    let s0 = fetch(source, pos, float2(-1.0, offset), source_size, backing_size);
    let s1 = fetch(source, pos, float2( 0.0, offset), source_size, backing_size);
    let s2 = fetch(source, pos, float2( 1.0, offset), source_size, backing_size);

    let dst = distance_to_nearest_texel(pos, source_size).x;

    let w0 = exp2(HARD_PIX * pow(abs(dst - 1.0), SHAPE));
    let w1 = exp2(HARD_PIX * pow(abs(dst),       SHAPE));
    let w2 = exp2(HARD_PIX * pow(abs(dst + 1.0), SHAPE));

    return (s0 * w0 + s1 * w1 + s2 * w2) / (w0 + w1 + w2);
}

// Per-channel scanline weight: each channel gets its own beam width based on
// that channel's brightness, matching how independent electron guns work on
// real NEC RGB monitors.
[ForceInline]
func scan_weight(pos: float2, offset: float, color: float3, source_size: float2) -> float3 {
    let beam_width = lerp(float3(SCAN_BEAM_MIN), float3(SCAN_BEAM_MAX), color);
    let distance = distance_to_nearest_texel(pos, source_size).y;
    let d = float3(distance + offset);
    return float3(
        exp2(HARD_SCAN * pow(abs(d.x / beam_width.x), SHAPE)),
        exp2(HARD_SCAN * pow(abs(d.y / beam_width.y), SHAPE)),
        exp2(HARD_SCAN * pow(abs(d.z / beam_width.z), SHAPE)),
    );
}

[ForceInline]
func tri(
    source: Sampler2D,
    pos: float2,
    source_size: float2,
    backing_size: float2
) -> float3 {
    let raw_a = horizontal_3tap(source, pos, -1.0, source_size, backing_size);
    let raw_b = horizontal_lanczos(source, pos, 0.0, source_size, backing_size);
    let raw_c = horizontal_3tap(source, pos, 1.0, source_size, backing_size);

    // Luminance-dependent bright boost applied before scanlines: dark pixels
    // get more boost because scanlines remove proportionally more of their energy.
    let row_a = raw_a * lerp(BRIGHT_BOOST_DARK, BRIGHT_BOOST_BRIGHT, max(raw_a.r, max(raw_a.g, raw_a.b)));
    let row_b = raw_b * lerp(BRIGHT_BOOST_DARK, BRIGHT_BOOST_BRIGHT, max(raw_b.r, max(raw_b.g, raw_b.b)));
    let row_c = raw_c * lerp(BRIGHT_BOOST_DARK, BRIGHT_BOOST_BRIGHT, max(raw_c.r, max(raw_c.g, raw_c.b)));

    let weight_a = scan_weight(pos, -1.0, row_a, source_size);
    let weight_b = scan_weight(pos, 0.0, row_b, source_size);
    let weight_c = scan_weight(pos, 1.0, row_c, source_size);

    // Clamp-from-above normalization: only scale down when weights exceed 1.0
    // (preventing clipping), but never scale up (preserving scanline gap darkness).
    var wa = weight_a;
    var wb = weight_b;
    var wc = weight_c;
    let peak_weight = max((wa + wb + wc).r, max((wa + wb + wc).g, (wa + wb + wc).b));
    if (peak_weight > 1.0) {
        let scale = 1.0 / peak_weight;
        wa *= scale;
        wb *= scale;
        wc *= scale;
    }

    return row_a * wa + row_b * wb + row_c * wc;
}

// Luminance-adaptive dot-triad mask: bright content blooms through the mask
// gaps on real CRTs, so we reduce mask contrast for bright pixels.
[ForceInline]
func dot_triad_mask(output_pos: float2, triad_width: float, color: float3) -> float3 {
    let mask_x = output_pos.x / triad_width;
    let mask_y = output_pos.y / triad_width;

    // Stagger every other row by half a triad width for delta arrangement.
    let row = floor(mask_y);
    let stagger = (fract(row * 0.5) > 0.25) ? 0.5 : 0.0;

    let cell_x = fract(mask_x + stagger);
    let cell_y = fract(mask_y);

    // Which phosphor dot: 0=R, 1=G, 2=B
    let phosphor_index = floor(cell_x * 3.0);

    // Distance from center of this phosphor dot.
    let dx = fract(cell_x * 3.0) - 0.5;
    let dy = cell_y - 0.5;
    let dot_dist = length(float2(dx, dy));

    // Dot falloff: 1.0 at center, fading to 0.0 toward edges.
    let dot_value = 1.0 - smoothstep(MASK_DOT_RADIUS * 0.5, MASK_DOT_RADIUS, dot_dist);

    // Effective mask strength reduced for bright content (phosphor bloom through
    // gaps) and for low resolutions (anti-moire).
    let peak = max(color.r, max(color.g, color.b));
    let effective_strength = MASK_STRENGTH * (1.0 - smoothstep(MASK_CUTOFF, 1.0, peak));

    // Per-channel weights: active phosphor is boosted, others are dimmed.
    // Energy-neutral: (active + 2*inactive)/3 = 1.0 per channel across a
    // full triad, so the mask does not steal overall brightness.
    let active_weight = 1.0 + 2.0 * effective_strength;
    let inactive_weight = 1.0 - effective_strength;
    let r_weight = (phosphor_index < 0.5) ? active_weight : inactive_weight;
    let g_weight = (phosphor_index > 0.5 && phosphor_index < 1.5) ? active_weight : inactive_weight;
    let b_weight = (phosphor_index > 1.5) ? active_weight : inactive_weight;

    let channel_mask = float3(r_weight, g_weight, b_weight);

    // Black matrix: attenuate between dots.
    let matrix = lerp(1.0, dot_value, effective_strength);

    return channel_mask * matrix;
}

// Main entry point: samples the native texture at `region_uv` (UV in [0,1]
// over the displayed letterbox area) with the full CRT pipeline. `source_size`
// is the source pixel dims (e.g. 640 x content_height); `output_size` is the
// displayed letterbox area size in swapchain pixels.
[ForceInline]
public func sample_crt_pixel(
    source: Sampler2D,
    region_uv: float2,
    source_size: float2,
    output_size: float2,
    backing_size: float2
) -> float3 {
    // Per-channel beam blur + brightness-adaptive scanlines.
    var color = tri(source, region_uv, source_size, backing_size);

    // Luminance-adaptive dot-triad black-matrix mask in output pixel space.
    // Scale triad width by the output-to-source ratio so the number of triads
    // stays fixed to the source resolution, matching a real shadow mask.
    let triad_width = MASK_TRIAD_WIDTH * (output_size.x / source_size.x);
    color *= dot_triad_mask(region_uv * output_size * 1.000001, triad_width, color);

    // Hue-preserving phosphor saturation limiter.
    color = phosphor_saturate(color, 0.9);

    return color;
}
