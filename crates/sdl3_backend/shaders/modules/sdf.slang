#language slang 2026

// Signed distance functions in pixels. A negative distance lies inside the shape.

module sdf;

static const float PI = 3.14159265358979;

// The coverage of a pixel by a shape whose edge has the distance `distance` from the
// pixel center. The edge fades over one pixel.
[ForceInline]
public func sdf_coverage(distance: float) -> float {
    return saturate(0.5 - distance);
}

// The distance to a rectangle with rounded corners. `position` is relative to the center
// of the rectangle. `radii` holds the radius of the corners top left, top right, bottom
// right and bottom left.
public func sdf_rounded_rectangle(position: float2, half_size: float2, radii: float4) -> float {
    let top = position.y < 0.0;
    let left = position.x < 0.0;
    let radius_pair = top ? radii.xy : radii.wz;
    let radius = min(left ? radius_pair.x : radius_pair.y, min(half_size.x, half_size.y));

    let corner = abs(position) - half_size + radius;
    return min(max(corner.x, corner.y), 0.0) + length(max(corner, float2(0.0))) - radius;
}

// The distance to a ring segment with round caps. The arc starts at the angle `start`
// and sweeps clockwise on screen by `sweep`. Angles start at the positive x-axis.
public func sdf_arc(position: float2, radius: float, width: float, start: float, sweep: float) -> float {
    let half_sweep = clamp(abs(sweep) * 0.5, 0.0, PI);
    let middle = start + sign(sweep) * half_sweep;

    // Rotate the arc so that its middle lies on the positive x-axis.
    let sin_middle = sin(middle);
    let cos_middle = cos(middle);
    let rotated = float2(
        position.x * cos_middle + position.y * sin_middle,
        -position.x * sin_middle + position.y * cos_middle
    );

    let angle = atan2(rotated.y, rotated.x);
    if (abs(angle) <= half_sweep) {
        return abs(length(rotated) - radius) - width * 0.5;
    }

    let end = radius * float2(cos(half_sweep), sign(rotated.y) * sin(half_sweep));
    return length(rotated - end) - width * 0.5;
}

// The distance to a line segment from `from` to `to` with round caps.
public func sdf_line(position: float2, from: float2, to: float2, width: float) -> float {
    let direction = to - from;
    let length_squared = dot(direction, direction);
    let t = length_squared > 0.0 ? saturate(dot(position - from, direction) / length_squared) : 0.0;
    return length(position - from - direction * t) - width * 0.5;
}
