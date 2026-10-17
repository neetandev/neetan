//! Builds the GPU data for the Slug rendering algorithm by Eric Lengyel from
//! glyph outlines and paths.
//!
//! The data is split into three arrays:
//!
//! - The curve array holds the control points of quadratic Bezier curves. One
//!   entry holds two points `(x1, y1, x2, y2)`. A curve starts at entry `i` and
//!   reads its third point from the first two components of entry `i + 1`.
//!   Connected curves of a contour share that entry.
//! - The band array holds the band headers and the curve lists of the bands.
//!   Every glyph starts with one header for each horizontal band, followed by
//!   one header for each vertical band. A header holds the curve count of the
//!   band and the offset of its curve list relative to the first header of the
//!   glyph. A curve list holds the curve array indices of all curves that
//!   intersect the band.
//! - The glyph array holds one [`SlugGlyph`] for each glyph.
//!
//! All coordinates are in em space, with the y-axis pointing up. The arrays
//! only grow. A renderer uploads the entries past the lengths it has seen.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::{OutlineSink, bytes::as_bytes};

/// The maximum number of bands in each direction.
const MAX_BAND_COUNT: usize = 16;

/// The bands overlap by this distance in em space to avoid gaps from rounding.
const BAND_EPSILON: f32 = 1.0 / 1024.0;

/// The number of quadratic curves that approximate one cubic curve.
const CUBIC_SUBDIVISIONS: usize = 4;

/// The GPU record of one glyph.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SlugGlyph {
    /// Scale (xy) and offset (zw) that map an em space coordinate to a band
    /// index.
    pub band_transform: [f32; 4],
    /// The index of the first band header of the glyph in the band array.
    pub band_base: u32,
    /// The index of the last vertical band.
    pub band_max_x: u32,
    /// The index of the last horizontal band.
    pub band_max_y: u32,
    /// Keeps the record at 32 bytes.
    pub padding: u32,
}

/// An em space bounding box with the y-axis pointing up.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    /// The left edge.
    pub x_min: f32,
    /// The bottom edge.
    pub y_min: f32,
    /// The right edge.
    pub x_max: f32,
    /// The top edge.
    pub y_max: f32,
}

impl Bounds {
    /// The width of the box.
    pub fn width(&self) -> f32 {
        self.x_max - self.x_min
    }

    /// The height of the box.
    pub fn height(&self) -> f32 {
        self.y_max - self.y_min
    }
}

/// A glyph in the Slug arrays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlugShape {
    /// The index of the glyph in the glyph array.
    pub index: u32,
    /// The em space bounds of the outline.
    pub bounds: Bounds,
}

/// The source of the generations of all Slug data.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(0);

/// A generation that no Slug data has used before.
fn next_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// The source of the identities of all Slug data.
static NEXT_IDENTITY: AtomicU64 = AtomicU64::new(0);

/// An identity that no Slug data has used before.
fn next_identity() -> u64 {
    NEXT_IDENTITY.fetch_add(1, Ordering::Relaxed)
}

/// The CPU side Slug data of all converted glyphs and paths.
#[derive(Debug)]
pub struct SlugData {
    /// The curve array.
    curves: Vec<[f32; 4]>,
    /// The band array.
    bands: Vec<u32>,
    /// The glyph array.
    glyphs: Vec<SlugGlyph>,
    /// The value of [`SlugData::generation`].
    generation: u64,
    /// The value of [`SlugData::identity`].
    identity: u64,
}

impl Default for SlugData {
    fn default() -> Self {
        Self {
            curves: Vec::new(),
            bands: Vec::new(),
            glyphs: Vec::new(),
            generation: next_generation(),
            identity: next_identity(),
        }
    }
}

impl Clone for SlugData {
    /// A copy with the same content and generation and a new identity. Both
    /// stores grow apart from here on.
    fn clone(&self) -> Self {
        Self {
            curves: self.curves.clone(),
            bands: self.bands.clone(),
            glyphs: self.glyphs.clone(),
            generation: self.generation,
            identity: next_identity(),
        }
    }
}

impl SlugData {
    /// Identifies the content. Two values are equal only when the data is
    /// the same, because every creation and every change draws a new value.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Identifies the store. It stays the same while [`SlugData::add`]
    /// appends to the arrays. A new store and a clone draw a new value. A
    /// renderer that saw shorter arrays of the same identity uploads only
    /// the appended entries.
    pub fn identity(&self) -> u64 {
        self.identity
    }

    /// The curve array.
    pub fn curves(&self) -> &[[f32; 4]] {
        &self.curves
    }

    /// The band array.
    pub fn bands(&self) -> &[u32] {
        &self.bands
    }

    /// The glyph array.
    pub fn glyphs(&self) -> &[SlugGlyph] {
        &self.glyphs
    }

    /// The curve array as bytes for a GPU upload.
    pub fn curve_bytes(&self) -> &[u8] {
        as_bytes(&self.curves)
    }

    /// The band array as bytes for a GPU upload.
    pub fn band_bytes(&self) -> &[u8] {
        as_bytes(&self.bands)
    }

    /// The glyph array as bytes for a GPU upload.
    pub fn glyph_bytes(&self) -> &[u8] {
        as_bytes(&self.glyphs)
    }

    /// Adds the contours collected by `builder`. An empty outline adds
    /// nothing.
    pub(crate) fn add(&mut self, builder: ContourBuilder) -> Option<SlugShape> {
        let build = build_glyph(builder.finish())?;

        let curve_base = self.curves.len() as u32;
        let band_base = self.bands.len();

        self.curves.extend_from_slice(&build.curves);
        self.bands.extend_from_slice(&build.bands);

        for &reference in &build.curve_references {
            self.bands[band_base + reference] += curve_base;
        }

        self.generation = next_generation();
        let index = self.glyphs.len() as u32;
        self.glyphs.push(SlugGlyph {
            band_transform: build.band_transform,
            band_base: band_base as u32,
            band_max_x: build.vertical_band_count as u32 - 1,
            band_max_y: build.horizontal_band_count as u32 - 1,
            padding: 0,
        });

        Some(SlugShape {
            index,
            bounds: build.bounds,
        })
    }
}

/// A quadratic Bezier curve in em space.
#[derive(Clone, Copy)]
struct Curve {
    /// The start point.
    p1: [f32; 2],
    /// The control point.
    p2: [f32; 2],
    /// The end point.
    p3: [f32; 2],
}

impl Curve {
    /// The smallest coordinate of the control points along `axis`.
    fn min(&self, axis: usize) -> f32 {
        self.p1[axis].min(self.p2[axis]).min(self.p3[axis])
    }

    /// The largest coordinate of the control points along `axis`.
    fn max(&self, axis: usize) -> f32 {
        self.p1[axis].max(self.p2[axis]).max(self.p3[axis])
    }

    /// Whether the curve is a straight line parallel to `axis`. Such a line
    /// never crosses a ray that runs along that axis.
    fn is_parallel_to(&self, axis: usize) -> bool {
        let other = 1 - axis;
        self.p1[other] == self.p2[other] && self.p2[other] == self.p3[other]
    }
}

/// Collects the contours of an outline as quadratic curves in em space.
pub(crate) struct ContourBuilder {
    /// The scale applied to every input point.
    scale: [f32; 2],
    /// The offset added to every scaled input point.
    offset: [f32; 2],
    /// The finished contours.
    contours: Vec<Vec<Curve>>,
    /// The curves of the open contour.
    current: Vec<Curve>,
    /// The start point of the open contour.
    start: [f32; 2],
    /// The end point of the last curve.
    position: [f32; 2],
}

impl ContourBuilder {
    /// A builder that maps a point `p` to `p * scale + offset`.
    pub(crate) fn new(scale: [f32; 2], offset: [f32; 2]) -> Self {
        Self {
            scale,
            offset,
            contours: Vec::new(),
            current: Vec::new(),
            start: [0.0, 0.0],
            position: [0.0, 0.0],
        }
    }

    /// The em space point of the input point `(x, y)`.
    fn point(&self, x: f32, y: f32) -> [f32; 2] {
        [
            x * self.scale[0] + self.offset[0],
            y * self.scale[1] + self.offset[1],
        ]
    }

    /// Appends a line to `to` unless it has no length.
    fn push_line(&mut self, to: [f32; 2]) {
        if to != self.position {
            // A line is a quadratic curve with the control point duplicated at
            // the end point. This is the most robust encoding for the root
            // solver.
            self.current.push(Curve {
                p1: self.position,
                p2: to,
                p3: to,
            });
            self.position = to;
        }
    }

    /// Appends a quadratic curve. A degenerate curve becomes a line.
    fn push_quadratic(&mut self, control: [f32; 2], to: [f32; 2]) {
        if control == self.position || control == to {
            self.push_line(to);
        } else {
            self.current.push(Curve {
                p1: self.position,
                p2: control,
                p3: to,
            });
            self.position = to;
        }
    }

    /// Closes the open contour and keeps it if it has curves.
    fn finish_contour(&mut self) {
        let start = self.start;
        self.push_line(start);
        if !self.current.is_empty() {
            self.contours.push(std::mem::take(&mut self.current));
        }
    }

    /// Closes the open contour and returns all contours.
    fn finish(mut self) -> Vec<Vec<Curve>> {
        self.finish_contour();
        self.contours
    }
}

impl OutlineSink for ContourBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish_contour();
        self.start = self.point(x, y);
        self.position = self.start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let to = self.point(x, y);
        self.push_line(to);
    }

    fn quad_to(&mut self, control_x: f32, control_y: f32, x: f32, y: f32) {
        let control = self.point(control_x, control_y);
        let to = self.point(x, y);
        self.push_quadratic(control, to);
    }

    fn curve_to(
        &mut self,
        first_control_x: f32,
        first_control_y: f32,
        second_control_x: f32,
        second_control_y: f32,
        x: f32,
        y: f32,
    ) {
        let start = self.position;
        let first_control = self.point(first_control_x, first_control_y);
        let second_control = self.point(second_control_x, second_control_y);
        let end = self.point(x, y);

        let cubic = |t: f32| -> [f32; 2] {
            let u = 1.0 - t;
            let a = u * u * u;
            let b = 3.0 * u * u * t;
            let c = 3.0 * u * t * t;
            let d = t * t * t;
            [
                a * start[0] + b * first_control[0] + c * second_control[0] + d * end[0],
                a * start[1] + b * first_control[1] + c * second_control[1] + d * end[1],
            ]
        };

        let derivative = |t: f32| -> [f32; 2] {
            let u = 1.0 - t;
            let a = 3.0 * u * u;
            let b = 6.0 * u * t;
            let c = 3.0 * t * t;
            [
                a * (first_control[0] - start[0])
                    + b * (second_control[0] - first_control[0])
                    + c * (end[0] - second_control[0]),
                a * (first_control[1] - start[1])
                    + b * (second_control[1] - first_control[1])
                    + c * (end[1] - second_control[1]),
            ]
        };

        // Each segment of the cubic gets approximated by a quadratic curve with
        // the same end points. The control point is the average of the two
        // points that the end tangents of the segment point to.
        for segment in 0..CUBIC_SUBDIVISIONS {
            let segment_start = segment as f32 / CUBIC_SUBDIVISIONS as f32;
            let segment_end = (segment + 1) as f32 / CUBIC_SUBDIVISIONS as f32;
            let step = (segment_end - segment_start) / 3.0;

            let from = cubic(segment_start);
            let to = if segment + 1 == CUBIC_SUBDIVISIONS {
                end
            } else {
                cubic(segment_end)
            };
            let start_tangent = derivative(segment_start);
            let end_tangent = derivative(segment_end);

            let control = [
                ((from[0] + start_tangent[0] * step * 1.5) + (to[0] - end_tangent[0] * step * 1.5))
                    * 0.5,
                ((from[1] + start_tangent[1] * step * 1.5) + (to[1] - end_tangent[1] * step * 1.5))
                    * 0.5,
            ];

            self.push_quadratic(control, to);
        }
    }

    fn close(&mut self) {
        self.finish_contour();
    }
}

/// The Slug data of a single glyph with indices local to the glyph.
struct GlyphBuild {
    /// The em space bounds of the outline.
    bounds: Bounds,
    /// The curve entries of the glyph.
    curves: Vec<[f32; 4]>,
    /// The band headers and curve lists of the glyph.
    bands: Vec<u32>,
    /// Positions in `bands` that hold curve indices.
    curve_references: Vec<usize>,
    /// The band transform of the [`SlugGlyph`] record.
    band_transform: [f32; 4],
    /// The number of bands along the y-axis.
    horizontal_band_count: usize,
    /// The number of bands along the x-axis.
    vertical_band_count: usize,
}

/// Builds the curve entries and the bands of one outline. An outline without
/// contours returns `None`.
fn build_glyph(contours: Vec<Vec<Curve>>) -> Option<GlyphBuild> {
    if contours.is_empty() {
        return None;
    }

    // Curve `k` of a contour starts at entry `start + k`, and the contour ends
    // with one entry for the last point.
    let curve_count: usize = contours.iter().map(Vec::len).sum();
    let mut curves = Vec::with_capacity(curve_count + contours.len());
    let mut indexed_curves = Vec::with_capacity(curve_count);

    for contour in &contours {
        for curve in contour {
            indexed_curves.push((curves.len() as u32, *curve));
            curves.push([curve.p1[0], curve.p1[1], curve.p2[0], curve.p2[1]]);
        }
        let last = contour.last()?;
        curves.push([last.p3[0], last.p3[1], 0.0, 0.0]);
    }

    let mut bounds = Bounds {
        x_min: f32::MAX,
        y_min: f32::MAX,
        x_max: f32::MIN,
        y_max: f32::MIN,
    };
    for (_, curve) in &indexed_curves {
        bounds.x_min = bounds.x_min.min(curve.min(0));
        bounds.y_min = bounds.y_min.min(curve.min(1));
        bounds.x_max = bounds.x_max.max(curve.max(0));
        bounds.y_max = bounds.y_max.max(curve.max(1));
    }

    // Horizontal bands slice the glyph along the y-axis and vertical bands
    // along the x-axis.
    let (horizontal_band_count, horizontal_scale, horizontal_offset) =
        band_layout(&indexed_curves, 1, bounds.y_min, bounds.y_max);
    let (vertical_band_count, vertical_scale, vertical_offset) =
        band_layout(&indexed_curves, 0, bounds.x_min, bounds.x_max);

    let horizontal_bands = collect_bands(
        &indexed_curves,
        1,
        horizontal_band_count,
        horizontal_scale,
        horizontal_offset,
    );
    let vertical_bands = collect_bands(
        &indexed_curves,
        0,
        vertical_band_count,
        vertical_scale,
        vertical_offset,
    );

    let header_count = horizontal_band_count + vertical_band_count;
    let mut bands = vec![0; header_count * 2];
    let mut curve_references = Vec::new();

    let mut previous: Option<(&Vec<u32>, u32)> = None;

    for (band_index, band) in horizontal_bands
        .iter()
        .chain(vertical_bands.iter())
        .enumerate()
    {
        // The first vertical band never shares the list of the last horizontal
        // band. The lists are sorted along different axes.
        if band_index == horizontal_band_count {
            previous = None;
        }

        let offset = match previous {
            Some((previous_band, previous_offset)) if previous_band == band => previous_offset,
            _ => {
                let offset = bands.len() as u32;
                curve_references.extend(bands.len()..bands.len() + band.len());
                bands.extend_from_slice(band);
                offset
            }
        };

        bands[band_index * 2] = band.len() as u32;
        bands[band_index * 2 + 1] = offset;
        previous = Some((band, offset));
    }

    Some(GlyphBuild {
        bounds,
        curves,
        bands,
        curve_references,
        band_transform: [
            vertical_scale,
            horizontal_scale,
            vertical_offset,
            horizontal_offset,
        ],
        horizontal_band_count,
        vertical_band_count,
    })
}

/// The range of band indices that a curve covers along the axis.
fn band_range(
    curve: &Curve,
    axis: usize,
    band_count: usize,
    scale: f32,
    offset: f32,
) -> (usize, usize) {
    let last = band_count as i32 - 1;
    let low = (((curve.min(axis) - BAND_EPSILON) * scale + offset).floor() as i32).clamp(0, last);
    let high = (((curve.max(axis) + BAND_EPSILON) * scale + offset).floor() as i32).clamp(0, last);
    (low as usize, high as usize)
}

/// Picks the band count that minimizes the largest number of curves in any
/// band. Ties go to the smaller total number of band entries.
fn band_layout(
    curves: &[(u32, Curve)],
    axis: usize,
    minimum: f32,
    maximum: f32,
) -> (usize, f32, f32) {
    let extent = maximum - minimum;

    if extent <= 0.0 {
        return (1, 0.0, 0.0);
    }

    let ray_axis = 1 - axis;
    let maximum_band_count = curves.len().clamp(1, MAX_BAND_COUNT);
    let mut best = (i32::MAX, i32::MAX, 1);
    let mut counts = [0i32; MAX_BAND_COUNT + 1];

    for band_count in 1..=maximum_band_count {
        let scale = band_count as f32 / extent;
        let offset = -minimum * scale;

        counts.fill(0);

        for (_, curve) in curves
            .iter()
            .filter(|(_, curve)| !curve.is_parallel_to(ray_axis))
        {
            let (low, high) = band_range(curve, axis, band_count, scale, offset);
            counts[low] += 1;
            counts[high + 1] -= 1;
        }

        let mut running = 0;
        let mut largest = 0;
        let mut total = 0;

        for count in counts.iter().take(band_count) {
            running += count;
            largest = largest.max(running);
            total += running;
        }

        if (largest, total) < (best.0, best.1) {
            best = (largest, total, band_count);
        }
    }

    let band_count = best.2;
    let scale = band_count as f32 / extent;
    (band_count, scale, -minimum * scale)
}

/// Collects the curves of every band. A band along the y-axis is crossed by
/// rays along the x-axis and the other way around. The curves of a band are
/// sorted in descending order of their maximum coordinate along the ray axis.
/// This lets the coverage evaluation stop early.
fn collect_bands(
    curves: &[(u32, Curve)],
    axis: usize,
    band_count: usize,
    scale: f32,
    offset: f32,
) -> Vec<Vec<u32>> {
    let ray_axis = 1 - axis;
    let mut bands: Vec<Vec<(u32, f32)>> = vec![Vec::new(); band_count];

    for (index, curve) in curves
        .iter()
        .filter(|(_, curve)| !curve.is_parallel_to(ray_axis))
    {
        let (low, high) = band_range(curve, axis, band_count, scale, offset);
        for band in &mut bands[low..=high] {
            band.push((*index, curve.max(ray_axis)));
        }
    }

    bands
        .into_iter()
        .map(|mut band| {
            band.sort_by(|first, second| second.1.total_cmp(&first.1));
            band.into_iter().map(|(index, _)| index).collect()
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use std::f32::consts::TAU;

    use super::*;
    use crate::{Path, ShapeStore, rasterizer::slug::slug_coverage};

    /// The scale of the coverage evaluation. It makes the edges almost hard.
    const PIXELS_PER_EM: f32 = 1.0e5;

    /// The number of sample rows and columns over the bounds of each shape.
    const GRID_SIZE: usize = 64;

    /// A small deterministic pseudo random number generator.
    struct Random(u32);

    impl Random {
        /// The next value from 0 to 1.
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            (self.0 >> 8) as f32 / (1u32 << 24) as f32
        }
    }

    /// A closed polygon with one vertex for each entry of `radii`. The vertices
    /// lie at equal angles around the center `(0.5, 0.5)`, each at the distance
    /// of its radius. `reverse` flips the winding direction.
    fn radial_polygon(path: Path, radii: &[f32], reverse: bool) -> Path {
        let count = radii.len();
        let points: Vec<[f32; 2]> = (0..count)
            .map(|index| {
                let index = if reverse { count - 1 - index } else { index };
                let angle = TAU * index as f32 / count as f32 + 0.1;
                [
                    0.5 + radii[index] * angle.cos(),
                    0.5 + radii[index] * angle.sin(),
                ]
            })
            .collect();
        path.polygon(&points)
    }

    /// The handle length of a cubic curve that approximates a quarter circle of
    /// radius 1.
    const QUARTER_CIRCLE_HANDLE: f32 = 0.552_284_8;

    /// A circle of four cubic curves around `(x, y)`.
    fn circle(path: Path, x: f32, y: f32, radius: f32, reverse: bool) -> Path {
        let direction = if reverse { -1.0 } else { 1.0 };
        let handle = radius * QUARTER_CIRCLE_HANDLE * direction;
        let vertical = radius * direction;
        path.move_to(x + radius, y)
            .cubic_to(
                x + radius,
                y + handle,
                x + handle,
                y + vertical,
                x,
                y + vertical,
            )
            .cubic_to(
                x - handle,
                y + vertical,
                x - radius,
                y + handle,
                x - radius,
                y,
            )
            .cubic_to(
                x - radius,
                y - handle,
                x - handle,
                y - vertical,
                x,
                y - vertical,
            )
            .cubic_to(
                x + handle,
                y - vertical,
                x + radius,
                y - handle,
                x + radius,
                y,
            )
            .close()
    }

    /// A set of paths that exercises lines, quadratic and cubic curves, holes,
    /// overlaps and self intersections.
    fn sample_paths() -> Vec<Path> {
        let mut random = Random(0x1234_5678);
        let mut paths = Vec::new();

        for sides in 3..=12 {
            paths.push(radial_polygon(Path::new(), &vec![0.45; sides], false));
        }
        for points in 5..=9 {
            let radii: Vec<f32> = (0..points * 2)
                .map(|index| if index % 2 == 0 { 0.45 } else { 0.2 })
                .collect();
            paths.push(radial_polygon(Path::new(), &radii, points % 2 == 0));
        }
        paths.push(circle(Path::new(), 0.5, 0.5, 0.45, false));
        paths.push(circle(
            circle(Path::new(), 0.5, 0.5, 0.45, false),
            0.5,
            0.5,
            0.25,
            true,
        ));
        paths.push(circle(
            circle(Path::new(), 0.4, 0.5, 0.3, false),
            0.6,
            0.5,
            0.3,
            false,
        ));
        for _ in 0..5 {
            let points: Vec<[f32; 2]> = (0..8).map(|_| [random.next(), random.next()]).collect();
            paths.push(Path::new().polygon(&points));
        }
        for _ in 0..5 {
            let mut path = Path::new().move_to(random.next(), random.next());
            for _ in 0..6 {
                path = path.quad_to(random.next(), random.next(), random.next(), random.next());
            }
            paths.push(path.close());
        }
        for _ in 0..3 {
            let mut path = Path::new().move_to(random.next(), random.next());
            for _ in 0..4 {
                path = path.cubic_to(
                    random.next(),
                    random.next(),
                    random.next(),
                    random.next(),
                    random.next(),
                    random.next(),
                );
            }
            paths.push(path.close());
        }
        paths.push(
            Path::new()
                .polygon(&[[0.0, 0.0], [0.7, 0.0], [0.7, 0.7], [0.0, 0.7]])
                .polygon(&[[0.3, 0.3], [1.0, 0.3], [1.0, 1.0], [0.3, 1.0]]),
        );
        paths.push(
            Path::new()
                .polygon(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
                .polygon(&[[0.25, 0.25], [0.25, 0.75], [0.75, 0.75], [0.75, 0.25]]),
        );
        paths
    }

    /// Converts every sample path.
    fn convert_all() -> (ShapeStore, Vec<SlugShape>) {
        let mut store = ShapeStore::new();
        let shapes = sample_paths()
            .iter()
            .map(|path| store.add_path(path).expect("the sample path has an area"))
            .collect();
        (store, shapes)
    }

    /// The Slug coverage of `point` with almost hard edges.
    pub(crate) fn coverage(data: &SlugData, glyph: &SlugGlyph, point: [f32; 2]) -> f32 {
        slug_coverage(
            data.curves(),
            data.bands(),
            glyph,
            point,
            [PIXELS_PER_EM; 2],
        )
    }

    /// The nonzero winding number of the outline around the point, computed
    /// from all curves of the glyph without the bands.
    fn winding_number(curves: &[[f32; 6]], point: [f32; 2]) -> i32 {
        let mut winding = 0;

        for curve in curves {
            let [x1, y1, x2, y2, x3, y3] = curve.map(f64::from);
            let a = y1 - 2.0 * y2 + y3;
            let b = 2.0 * (y2 - y1);
            let c = y1 - f64::from(point[1]);

            let mut roots = Vec::new();

            if a.abs() < 1e-12 {
                if b != 0.0 {
                    roots.push(-c / b);
                }
            } else {
                let discriminant = b * b - 4.0 * a * c;
                if discriminant >= 0.0 {
                    let q = -0.5 * (b + b.signum() * discriminant.sqrt());
                    roots.push(q / a);
                    if q != 0.0 {
                        roots.push(c / q);
                    }
                }
            }

            for t in roots.into_iter().filter(|t| (0.0..1.0).contains(t)) {
                let u = 1.0 - t;
                let x = u * u * x1 + 2.0 * u * t * x2 + t * t * x3;
                let slope = 2.0 * a * t + b;

                if x > f64::from(point[0]) && slope != 0.0 {
                    winding += if slope > 0.0 { 1 } else { -1 };
                }
            }
        }

        winding
    }

    /// The curves of a glyph, collected by walking its band lists.
    fn glyph_curves(data: &SlugData, glyph: &SlugGlyph) -> Vec<[f32; 6]> {
        let mut locations: Vec<u32> = Vec::new();
        let band_count = (glyph.band_max_x + glyph.band_max_y + 2) as usize;
        for band in 0..band_count {
            let header = glyph.band_base as usize + band * 2;
            let count = data.bands[header] as usize;
            let list = glyph.band_base as usize + data.bands[header + 1] as usize;
            locations.extend_from_slice(&data.bands[list..list + count]);
        }
        locations.sort_unstable();
        locations.dedup();
        locations
            .iter()
            .map(|&location| {
                let entry = data.curves[location as usize];
                let next = data.curves[location as usize + 1];
                [entry[0], entry[1], entry[2], entry[3], next[0], next[1]]
            })
            .collect()
    }

    #[test]
    fn the_glyph_record_has_32_bytes() {
        assert_eq!(size_of::<SlugGlyph>(), 32);
        let mut store = ShapeStore::new();
        store.add_path(&Path::new().polygon(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]));
        let data = store.slug();
        assert_eq!(data.glyph_bytes().len(), data.glyphs().len() * 32);
        assert_eq!(data.curve_bytes().len(), data.curves().len() * 16);
        assert_eq!(data.band_bytes().len(), data.bands().len() * 4);
        assert_eq!(data.band_bytes()[..4], data.bands()[0].to_ne_bytes());
    }

    #[test]
    fn the_identity_survives_additions_and_not_clones() {
        let triangle = Path::new().polygon(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        let mut store = ShapeStore::new();
        let other = ShapeStore::new();
        assert_ne!(store.slug().identity(), other.slug().identity());

        let identity = store.slug().identity();
        let generation = store.slug().generation();
        store.add_path(&triangle);
        assert_eq!(store.slug().identity(), identity);
        assert_ne!(store.slug().generation(), generation);

        let mut copy = store.clone();
        assert_ne!(copy.slug().identity(), identity);
        assert_eq!(copy.slug().generation(), store.slug().generation());
        assert_eq!(copy.slug().curves(), store.slug().curves());
        assert_eq!(copy.slug().bands(), store.slug().bands());
        assert_eq!(copy.slug().glyphs(), store.slug().glyphs());

        let copy_identity = copy.slug().identity();
        copy.add_path(&triangle);
        store.add_path(&triangle);
        assert_eq!(copy.slug().identity(), copy_identity);
        assert_eq!(store.slug().identity(), identity);
        assert_ne!(copy.slug().generation(), store.slug().generation());
    }

    #[test]
    fn coverage_matches_winding_number() {
        let (store, shapes) = convert_all();
        let data = store.slug();

        let mut samples = 0;
        let mut mismatches = 0;

        for shape in &shapes {
            let glyph = data.glyphs[shape.index as usize];
            let curves = glyph_curves(data, &glyph);
            let bounds = shape.bounds;

            for row in 0..GRID_SIZE {
                for column in 0..GRID_SIZE {
                    // Offsets with irrational ratios keep the samples away from
                    // the exact coordinates of the control points.
                    let point = [
                        bounds.x_min
                            + bounds.width() * ((column as f32 + 0.5 + 0.0731) / GRID_SIZE as f32),
                        bounds.y_min
                            + bounds.height() * ((row as f32 + 0.5 + 0.0377) / GRID_SIZE as f32),
                    ];

                    let expected = winding_number(&curves, point) != 0;
                    let actual = coverage(data, &glyph, point) > 0.5;

                    samples += 1;
                    if expected != actual {
                        mismatches += 1;
                    }
                }
            }
        }

        assert!(samples > 100_000, "only {samples} samples");
        assert!(
            mismatches * 10_000 < samples,
            "{mismatches} of {samples} samples have the wrong coverage"
        );
    }

    #[test]
    fn band_curves_are_sorted() {
        let (store, shapes) = convert_all();
        let data = store.slug();

        for shape in &shapes {
            let glyph = data.glyphs[shape.index as usize];
            let band_count = (glyph.band_max_x + glyph.band_max_y + 2) as usize;

            for band in 0..band_count {
                let header = glyph.band_base as usize + band * 2;
                let count = data.bands[header] as usize;
                let list = glyph.band_base as usize + data.bands[header + 1] as usize;
                let axis = if band <= glyph.band_max_y as usize {
                    0
                } else {
                    1
                };

                let maxima: Vec<f32> = data.bands[list..list + count]
                    .iter()
                    .map(|&location| {
                        let entry = data.curves[location as usize];
                        let next = data.curves[location as usize + 1];
                        entry[axis].max(entry[axis + 2]).max(next[axis])
                    })
                    .collect();

                assert!(
                    maxima.windows(2).all(|pair| pair[0] >= pair[1]),
                    "band curves are not sorted"
                );
            }
        }
    }

    #[test]
    fn degenerate_paths_do_not_panic() {
        let mut store = ShapeStore::new();
        assert_eq!(store.add_path(&Path::new().move_to(0.5, 0.5)), None);
        assert_eq!(
            store.add_path(&Path::new().move_to(0.0, 0.0).move_to(1.0, 1.0).close()),
            None
        );
        store.add_path(&Path::new().polygon(&[[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]]));
        store.add_path(&Path::new().polygon(&[[0.0, 0.5], [0.5, 0.5], [1.0, 0.5]]));
        store.add_path(&Path::new().polygon(&[[f32::NAN, 0.0], [1.0, f32::NAN], [0.0, 1.0]]));
        store.add_path(&Path::new().polygon(&[[f32::NAN; 2], [f32::NAN; 2], [f32::NAN; 2]]));
        store.add_path(
            &Path::new()
                .move_to(0.0, 0.0)
                .quad_to(f32::NAN, f32::NAN, 1.0, 1.0)
                .cubic_to(f32::INFINITY, 0.0, 0.0, f32::NEG_INFINITY, 0.0, 1.0)
                .close(),
        );
        let data = store.slug();
        for glyph in data.glyphs() {
            coverage(data, glyph, [0.5, -0.5]);
        }
    }
}
