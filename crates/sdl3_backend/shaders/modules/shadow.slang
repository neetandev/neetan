#language slang 2026

// The shadow of a rectangle with rounded corners under a Gaussian blur.
//
// The blur of a straight edge has a closed form with the error function. The shadow
// integrates the blurred rows of the rectangle along the y-axis with four samples. Each
// row knows the width of the rounded rectangle at its height. This follows the approach
// of Evan Wallace: https://madebyevan.com/shaders/fast-rounded-rectangle-shadows/

module shadow;

static const float PI = 3.14159265358979;

// The density of the normal distribution with the deviation `sigma` at `x`.
[ForceInline]
func gaussian(x: float, sigma: float) -> float {
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (sqrt(2.0 * PI) * sigma);
}

// An approximation of the error function with a maximum error of 5e-4.
[ForceInline]
func erf2(x: float2) -> float2 {
    let sign_x = sign(x);
    let magnitude = abs(x);
    var t = 1.0 + (0.278393 + (0.230389 + 0.078108 * (magnitude * magnitude)) * magnitude) * magnitude;
    t *= t;
    return sign_x - sign_x / (t * t);
}

// The blurred coverage of one row of the rectangle at the height `y`.
[ForceInline]
func row(x: float, y: float, sigma: float, radius: float, half_size: float2) -> float {
    let delta = min(half_size.y - radius - abs(y), 0.0);
    let curved = half_size.x - radius + sqrt(max(0.0, radius * radius - delta * delta));
    let integral = 0.5 + 0.5 * erf2((x + float2(-curved, curved)) * (sqrt(0.5) / sigma));
    return integral.y - integral.x;
}

// The shadow intensity at `position` relative to the center of the rectangle.
public func rounded_rectangle_shadow(position: float2, half_size: float2, radius: float, sigma: float) -> float {
    let radius = min(radius, min(half_size.x, half_size.y));
    let low = position.y - half_size.y;
    let high = position.y + half_size.y;
    let start = clamp(-3.0 * sigma, low, high);
    let end = clamp(3.0 * sigma, low, high);

    let step = (end - start) / 4.0;
    var y = start + step * 0.5;
    var value = 0.0;
    for (var index = 0; index < 4; index++) {
        value += row(position.x, position.y - y, sigma, radius, half_size) * gaussian(y, sigma) * step;
        y += step;
    }
    return value;
}
