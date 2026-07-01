#language slang 2026

module composite;

import transform;

// Composite/RF CRT shader.
//
// Models the PC-6001 video path: the M5C6847P (MC6847) VDG emits luma plus two
// color-difference signals, the M51342P encoder QAM-modulates them onto the
// 3.58 MHz subcarrier (R-Y / B-Y axes, in quadrature), and a period NTSC set
// decodes the single composite wire back to RGB with a 1D notch/bandpass. Luma
// and chroma share one bandwidth-limited wire, so a hi-res monochrome dot
// pattern lands on the subcarrier band and the receiver turns it into color: on
// SCREEN 4 two dots make one subcarrier cycle, so an even-column dot decodes
// green and an odd-column dot decodes red/pink.

static const float PI = 3.14159265359;
static const float TAU = 6.28318530718;
static const float HALF_PI = 1.57079632679;

// Color subcarrier frequency in cycles per source pixel. This is the key knob:
// it sets how many source dots make one subcarrier cycle and therefore which
// artifact colors emerge.
static const float F_COL = 0.5;
// Signal samples per source pixel. At one sample per pixel with F_COL = 0.5 the
// quadrature sine is sin(PI * column), identically zero at every integer column,
// which collapses chroma onto a single axis and drops each source color's Q
// component at encode time. Oversampling restores a non-zero sine and the full
// chroma plane. Must match COMPOSITE_OVERSAMPLE in pipeline.rs.
static const int OVERSAMPLE = 2;
// Number of FIR taps in signal-sample space (not source pixels). The window
// spans FIR_TAPS / OVERSAMPLE source pixels. Must match COMPOSITE_FIR_TAPS in
// pipeline.rs.
static const int FIR_TAPS = 25;
// Number of float4s that hold the FIR weights, four taps each.
static const int FIR_TAPS_VEC4 = (FIR_TAPS + 3) / 4;
// The windowed-sinc luma low-pass and chroma bandpass weights are computed on the
// CPU (pipeline.rs: LUMA_CUTOFF, CHROMA_BANDWIDTH, CHROMA_GAIN).
static const float SATURATION = 1.0;
// Luma gain.
static const float BRIGHTNESS = 1.0;
// Fixed subcarrier/burst phase in turns (1.0 = full circle) added on top of the
// runtime composite_phase select. At 0 the R-Y/B-Y axes already land the two
// SCREEN 4 dot phases on green (even column) and red/pink (odd column) for our
// palette.
static const float HUE = 0.0;

// Scanline shape: Gaussian sharpness (more negative = harder) and the minimum
// brightness in the gap between scanlines.
static const float SCAN_HARDNESS = -2.5;
static const float SCAN_MIN = 0.55;

// Fetches one source column at gamma-encoded RGB. The sampled texture is sRGB,
// so the hardware decodes it to linear; NTSC modulation belongs in gamma space,
// so re-encode. X snaps to the texel center so the linear sampler stays
// effectively nearest horizontally (preserving the dot pattern that carries the
// artifact color), while `sample_y` is passed through continuous so the same
// sampler interpolates vertically between rows for free: a one-source-row stroke
// then fades smoothly across output rows instead of aliasing razor thin. Y is
// clamped to the first/last row centers so the interpolation never pulls in stale
// backing texels past the visible area. Columns outside the source read black to
// keep the horizontal border.
[ForceInline]
func fetch_gamma(
    source: Sampler2D,
    column: int,
    sample_y: float,
    source_size: float2,
    backing_size: float2
) -> float3 {
    if (column < 0 || float(column) >= source_size.x) {
        return float3(0.0);
    }
    let clamped_y = clamp(sample_y, 0.5, source_size.y - 0.5);
    let backing_uv = float2(float(column) + 0.5, clamped_y) / backing_size;
    let linear = source.SampleLevel(backing_uv, 0.0).xyz;
    return color::linear_to_srgb(linear);
}

// Luma of a gamma-encoded pixel.
[ForceInline]
func luma_of(rgb: float3) -> float {
    return dot(rgb, float3(0.299, 0.587, 0.114));
}

// Composite scalar for a gamma-encoded pixel at subcarrier phase `theta`: luma
// plus the two color-difference components on the B-Y / R-Y axes riding the
// oscillator in quadrature, matching the M51342P encoder.
[ForceInline]
func modulate(rgb: float3, cosine: float, sine: float) -> float {
    let y = luma_of(rgb);
    let u = dot(rgb, float3(-0.14713, -0.28886, 0.436));
    let v = dot(rgb, float3(0.615, -0.51499, -0.10001));
    return y + u * cosine + v * sine;
}

[ForceInline]
func yuv_to_rgb(yuv: float3) -> float3 {
    return float3(
        yuv.x + 1.13983 * yuv.z,
        yuv.x - 0.39465 * yuv.y - 0.58060 * yuv.z,
        yuv.x + 2.03211 * yuv.y,
    );
}

// Vertical scanline weight from the fractional position within the source row.
[ForceInline]
func scanline_weight(source_row_fraction: float) -> float {
    let d = source_row_fraction - 0.5;
    let gaussian = exp2(SCAN_HARDNESS * (d * d) * 4.0);
    return lerp(SCAN_MIN, 1.0, gaussian);
}

[ForceInline]
public func sample_composite_pixel(
    source: Sampler2D,
    region_uv: float2,
    source_size: float2,
    output_size: float2,
    backing_size: float2,
    composite_phase: uint,
    luma_weights: float4[FIR_TAPS_VEC4],
    chroma_weights: float4[FIR_TAPS_VEC4]
) -> float3 {
    let continuous_x = region_uv.x * source_size.x;
    let continuous_y = region_uv.y * source_size.y;

    // Every scanline carries the subcarrier at the same phase reference. The
    // PC-6001 artifact color is stable down a column (a green vertical line stays
    // green top to bottom), so there is no per-line phase inversion and Y/C are
    // separated horizontally, the way a period 1D-notch receiver did, rather than
    // by a vertical comb. A comb would cancel any dither that varies row-to-row,
    // draining sustained fills to gray while leaving only edges colored.

    var luma = 0.0;
    var chroma_u = 0.0;
    var chroma_v = 0.0;

    // Incremental carrier: the subcarrier phase advances by a constant step per
    // sub-sample, so seed cos/sin once at the window start and rotate them each
    // tap.
    let inv_oversample = 1.0 / float(OVERSAMPLE);
    let carrier_step = TAU * F_COL * inv_oversample;
    let step_cos = cos(carrier_step);
    let step_sin = sin(carrier_step);
    var sample_x = continuous_x - float(FIR_TAPS / 2) * inv_oversample;
    let start_phase = TAU * F_COL * sample_x;
    var cosine = cos(start_phase);
    var sine = sin(start_phase);

    for (int tap = 0; tap < FIR_TAPS; tap++) {
        // Nearest-neighbour source column: each dot acts as a box pulse spanning
        // OVERSAMPLE samples, which is what the real dot clock puts on the wire.
        let column = int(floor(sample_x));
        let composite = modulate(
            fetch_gamma(source, column, continuous_y, source_size, backing_size), cosine, sine);

        // Precomputed, pre-normalized FIR weights (packed four taps per float4):
        // luma is a low-pass below the subcarrier, chroma a narrow bandpass around
        // it, and the quadrature product then shifts the chroma sidebands to DC.
        let luma_weight = luma_weights[tap >> 2][tap & 3];
        let chroma_weight = chroma_weights[tap >> 2][tap & 3];
        luma += composite * luma_weight;
        chroma_u += composite * cosine * chroma_weight;
        chroma_v += composite * sine * chroma_weight;

        // Advance the sample position and rotate the carrier one step.
        sample_x += inv_oversample;
        let next_cos = cosine * step_cos - sine * step_sin;
        let next_sin = sine * step_cos + cosine * step_sin;
        cosine = next_cos;
        sine = next_sin;
    }

    // Weights are already normalized (luma to unit sum, chroma to unit sum with
    // the quadrature-demod gain folded in).
    luma *= BRIGHTNESS;
    let u_component = chroma_u;
    let v_component = chroma_v;

    // HUE is the fixed burst reference that lands even/odd SCREEN 4 dots on
    // green/pink; composite_phase then rotates in quarter turns like the PC-6601
    // color DIP switch, cycling the artifact palette.
    let hue_angle = TAU * HUE + HALF_PI * float(composite_phase);
    let hue_cos = cos(hue_angle);
    let hue_sin = sin(hue_angle);
    let rotated_u = u_component * hue_cos - v_component * hue_sin;
    let rotated_v = u_component * hue_sin + v_component * hue_cos;

    let decoded = yuv_to_rgb(float3(luma, rotated_u * SATURATION, rotated_v * SATURATION));

    let scanline = scanline_weight(frac(continuous_y));
    let gamma_rgb = max(decoded, float3(0.0)) * scanline;

    return color::srgb_to_linear(gamma_rgb);
}
