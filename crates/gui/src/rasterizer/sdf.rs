//! Signed distance functions in pixels. A negative distance lies inside the
//! shape.
//!
//! Every shape splits its distance into the constants of the instance, the
//! constants of one pixel row and the part that depends on the column.

use std::f32::consts::PI;

use super::math::{length, saturate, sign};

/// The coverage of a pixel by a shape whose edge has the distance `distance`
/// from the pixel center. The edge fades over one pixel.
#[inline(always)]
pub(crate) fn sdf_coverage(distance: f32) -> f32 {
    saturate(0.5 - distance)
}

/// One pixel row of a rectangle with rounded corners.
pub(crate) struct RoundedRectangleRow {
    /// Half the width of the rectangle.
    half_width: f32,
    /// The radius of the left and the right corner of the row.
    radii: [f32; 2],
    /// The vertical distance into the left and the right corner.
    corners_y: [f32; 2],
}

impl RoundedRectangleRow {
    /// The row at `position_y` relative to the center of the rectangle.
    /// `radii` holds the radius of the corners top left, top right, bottom
    /// right and bottom left.
    #[inline(always)]
    pub(crate) fn new(position_y: f32, half_size: [f32; 2], radii: [f32; 4]) -> Self {
        let radius_pair = if position_y < 0.0 {
            [radii[0], radii[1]]
        } else {
            [radii[3], radii[2]]
        };
        let limit = half_size[0].min(half_size[1]);
        let radii = radius_pair.map(|radius| radius.min(limit));
        Self {
            half_width: half_size[0],
            radii,
            corners_y: radii.map(|radius| position_y.abs() - half_size[1] + radius),
        }
    }

    /// The distance at `position_x` relative to the center of the rectangle.
    #[inline(always)]
    pub(crate) fn distance(&self, position_x: f32) -> f32 {
        let side = if position_x < 0.0 { 0 } else { 1 };
        let radius = self.radii[side];
        let corner_y = self.corners_y[side];
        let corner_x = position_x.abs() - self.half_width + radius;
        corner_x.max(corner_y).min(0.0) + length(corner_x.max(0.0), corner_y.max(0.0)) - radius
    }
}

/// A ring segment with round caps. The arc starts at the angle `start` and
/// sweeps clockwise on screen by `sweep`. Angles start at the positive x-axis.
pub(crate) struct Arc {
    /// The radius of the middle of the ring.
    radius: f32,
    /// Half the width of the ring.
    half_width: f32,
    /// Half the sweep, limited to a full turn.
    half_sweep: f32,
    /// The sine of the angle of the middle of the arc.
    sin_middle: f32,
    /// The cosine of the angle of the middle of the arc.
    cos_middle: f32,
    /// The x coordinate of the cap centers on the rotated arc.
    end_x: f32,
    /// The sine of half the sweep.
    sin_half_sweep: f32,
}

/// One pixel row of an [`Arc`].
pub(crate) struct ArcRow {
    /// The row offset from the center times the sine of the middle angle.
    y_sin_middle: f32,
    /// The row offset from the center times the cosine of the middle angle.
    y_cos_middle: f32,
}

impl Arc {
    /// The arc with the radius `radius`, the width `width`, the start angle
    /// `start` and the sweep `sweep`.
    pub(crate) fn new(radius: f32, width: f32, start: f32, sweep: f32) -> Self {
        let half_sweep = (sweep.abs() * 0.5).clamp(0.0, PI);
        let middle = start + sign(sweep) * half_sweep;
        let (sin_middle, cos_middle) = middle.sin_cos();
        Self {
            radius,
            half_width: width * 0.5,
            half_sweep,
            sin_middle,
            cos_middle,
            end_x: radius * half_sweep.cos(),
            sin_half_sweep: half_sweep.sin(),
        }
    }

    /// The row at `position_y` relative to the center.
    #[inline(always)]
    pub(crate) fn row(&self, position_y: f32) -> ArcRow {
        ArcRow {
            y_sin_middle: position_y * self.sin_middle,
            y_cos_middle: position_y * self.cos_middle,
        }
    }

    /// The distance at `position_x` relative to the center in the row `row`.
    /// The arc is rotated so that its middle lies on the positive x-axis.
    #[inline(always)]
    pub(crate) fn distance(&self, row: &ArcRow, position_x: f32) -> f32 {
        let rotated_x = position_x * self.cos_middle + row.y_sin_middle;
        let rotated_y = -position_x * self.sin_middle + row.y_cos_middle;

        let angle = rotated_y.atan2(rotated_x);
        if angle.abs() <= self.half_sweep {
            return (length(rotated_x, rotated_y) - self.radius).abs() - self.half_width;
        }

        let end_y = self.radius * sign(rotated_y) * self.sin_half_sweep;
        length(rotated_x - self.end_x, rotated_y - end_y) - self.half_width
    }
}

/// A line segment with round caps.
pub(crate) struct Line {
    /// The start point.
    from: [f32; 2],
    /// The vector from the start to the end point.
    direction: [f32; 2],
    /// The squared length of the segment.
    length_squared: f32,
    /// Half the width of the line.
    half_width: f32,
}

/// One pixel row of a [`Line`].
pub(crate) struct LineRow {
    /// The vertical offset of the row from the start point.
    offset_y: f32,
    /// The vertical offset times the vertical component of the direction.
    offset_y_direction: f32,
}

impl Line {
    /// The line from `from` to `to` with the width `width`.
    pub(crate) fn new(from: [f32; 2], to: [f32; 2], width: f32) -> Self {
        let direction = [to[0] - from[0], to[1] - from[1]];
        Self {
            from,
            direction,
            length_squared: direction[0] * direction[0] + direction[1] * direction[1],
            half_width: width * 0.5,
        }
    }

    /// The row with the pixel center height `pixel_y`.
    #[inline(always)]
    pub(crate) fn row(&self, pixel_y: f32) -> LineRow {
        let offset_y = pixel_y - self.from[1];
        LineRow {
            offset_y,
            offset_y_direction: offset_y * self.direction[1],
        }
    }

    /// The distance at the pixel center column `pixel_x` in the row `row`.
    #[inline(always)]
    pub(crate) fn distance(&self, row: &LineRow, pixel_x: f32) -> f32 {
        let offset_x = pixel_x - self.from[0];
        let t = if self.length_squared > 0.0 {
            saturate((offset_x * self.direction[0] + row.offset_y_direction) / self.length_squared)
        } else {
            0.0
        };
        length(
            offset_x - self.direction[0] * t,
            row.offset_y - self.direction[1] * t,
        ) - self.half_width
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The distance to a rectangle with rounded corners evaluated for one
    /// pixel. `position` is relative to the center of the rectangle.
    pub(crate) fn sdf_rounded_rectangle(
        position: [f32; 2],
        half_size: [f32; 2],
        radii: [f32; 4],
    ) -> f32 {
        let top = position[1] < 0.0;
        let left = position[0] < 0.0;
        let radius_pair = if top {
            [radii[0], radii[1]]
        } else {
            [radii[3], radii[2]]
        };
        let corner_radius = if left { radius_pair[0] } else { radius_pair[1] };
        let radius = corner_radius.min(half_size[0].min(half_size[1]));

        let corner_x = position[0].abs() - half_size[0] + radius;
        let corner_y = position[1].abs() - half_size[1] + radius;
        corner_x.max(corner_y).min(0.0) + length(corner_x.max(0.0), corner_y.max(0.0)) - radius
    }

    /// The distance to a ring segment with round caps evaluated for one pixel.
    pub(crate) fn sdf_arc(
        position: [f32; 2],
        radius: f32,
        width: f32,
        start: f32,
        sweep: f32,
    ) -> f32 {
        let half_sweep = (sweep.abs() * 0.5).clamp(0.0, PI);
        let middle = start + sign(sweep) * half_sweep;

        let (sin_middle, cos_middle) = middle.sin_cos();
        let rotated_x = position[0] * cos_middle + position[1] * sin_middle;
        let rotated_y = -position[0] * sin_middle + position[1] * cos_middle;

        let angle = rotated_y.atan2(rotated_x);
        if angle.abs() <= half_sweep {
            return (length(rotated_x, rotated_y) - radius).abs() - width * 0.5;
        }

        let end_x = radius * half_sweep.cos();
        let end_y = radius * sign(rotated_y) * half_sweep.sin();
        length(rotated_x - end_x, rotated_y - end_y) - width * 0.5
    }

    /// The distance to a line segment with round caps evaluated for one pixel.
    pub(crate) fn sdf_line(position: [f32; 2], from: [f32; 2], to: [f32; 2], width: f32) -> f32 {
        let direction = [to[0] - from[0], to[1] - from[1]];
        let offset = [position[0] - from[0], position[1] - from[1]];
        let length_squared = direction[0] * direction[0] + direction[1] * direction[1];
        let t = if length_squared > 0.0 {
            saturate((offset[0] * direction[0] + offset[1] * direction[1]) / length_squared)
        } else {
            0.0
        };
        length(offset[0] - direction[0] * t, offset[1] - direction[1] * t) - width * 0.5
    }

    /// The pixel center coordinates of a grid around the origin with a
    /// fractional offset.
    fn grid() -> impl Iterator<Item = f32> {
        (-60..60).map(|step| step as f32 * 0.37 + 0.5)
    }

    #[test]
    fn distances_are_negative_inside() {
        let half_size = [10.0, 5.0];
        assert_eq!(sdf_rounded_rectangle([0.0, 0.0], half_size, [0.0; 4]), -5.0);
        assert_eq!(sdf_rounded_rectangle([12.0, 0.0], half_size, [0.0; 4]), 2.0);
        let corner = sdf_rounded_rectangle([10.0, 5.0], half_size, [0.0, 0.0, 2.0, 0.0]);
        assert!((corner - (8.0f32.sqrt() - 2.0)).abs() < 1e-5);
        assert_eq!(sdf_line([5.0, 3.0], [0.0, 0.0], [10.0, 0.0], 2.0), 2.0);
        assert_eq!(sdf_line([-3.0, 0.0], [0.0, 0.0], [10.0, 0.0], 2.0), 2.0);
        assert!((sdf_arc([10.0, 0.0], 10.0, 2.0, 0.0, 1.0) + 1.0).abs() < 1e-5);
        assert!(sdf_arc([-10.0, 0.0], 10.0, 2.0, 0.0, 1.0) > 10.0);
        assert_eq!(sdf_coverage(0.0), 0.5);
    }

    #[test]
    fn row_distances_equal_the_pixel_distances() {
        for (half_size, radii) in [
            ([10.0, 5.0], [0.0; 4]),
            ([10.25, 7.5], [1.0, 3.0, 6.5, 12.0]),
            ([4.0, 9.0], [f32::NAN, 2.0, -1.0, 20.0]),
        ] {
            for position_y in grid() {
                let row = RoundedRectangleRow::new(position_y, half_size, radii);
                for position_x in grid() {
                    let expected =
                        sdf_rounded_rectangle([position_x, position_y], half_size, radii);
                    assert_eq!(
                        row.distance(position_x).to_bits(),
                        expected.to_bits(),
                        "rectangle at {position_x}, {position_y}"
                    );
                }
            }
        }

        for (radius, width, start, sweep) in [
            (12.0, 3.0, 0.4, 2.0),
            (8.5, 5.0, -1.0, -4.5),
            (15.0, 2.0, 2.0, 7.0),
            (10.0, 4.0, 0.0, 0.0),
        ] {
            let arc = Arc::new(radius, width, start, sweep);
            for position_y in grid() {
                let row = arc.row(position_y);
                for position_x in grid() {
                    let expected = sdf_arc([position_x, position_y], radius, width, start, sweep);
                    assert_eq!(
                        arc.distance(&row, position_x).to_bits(),
                        expected.to_bits(),
                        "arc at {position_x}, {position_y}"
                    );
                }
            }
        }

        for (from, to, width) in [
            ([1.5, -3.25], [17.0, 9.0], 2.5),
            ([4.0, 4.0], [4.0, 4.0], 3.0),
            ([-10.0, 2.0], [12.0, 2.0], 1.0),
        ] {
            let line = Line::new(from, to, width);
            for pixel_y in grid() {
                let row = line.row(pixel_y);
                for pixel_x in grid() {
                    let expected = sdf_line([pixel_x, pixel_y], from, to, width);
                    assert_eq!(
                        line.distance(&row, pixel_x).to_bits(),
                        expected.to_bits(),
                        "line at {pixel_x}, {pixel_y}"
                    );
                }
            }
        }
    }
}
