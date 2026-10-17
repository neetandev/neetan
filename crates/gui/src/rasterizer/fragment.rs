//! The span shaders of the instance kinds.
//!
//! Every kind has a shader that holds the constants of one instance. It
//! computes the constants of a pixel row once and blends the pixels of a span
//! of that row. The math mirrors the fragment stage of the GPU shader. Every
//! pixel is evaluated at its center in physical pixels with the y-axis pointing
//! down. The colors are linear and premultiplied.

use std::ops::Range;

use super::{
    BLUE_NOISE, BLUE_NOISE_SIZE, Interior,
    curves::curves,
    math::{lerp, saturate},
    sdf::{Arc, ArcRow, Line, LineRow, RoundedRectangleRow, sdf_coverage},
    shadow::{Shadow, ShadowRow},
    slug::slug_coverage,
};
use crate::{Instance, SlugData, SlugGlyph};

/// The luminance weights of the linear red, green and blue channels.
const LUMINANCE_WEIGHTS: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// The number of shadow intensities computed before they are blended.
const SHADOW_CHUNK: usize = 64;

/// A shader that blends one instance into the pixel rows of a target.
pub(crate) trait SpanShader {
    /// The constants of one pixel row.
    type Row;

    /// The constants of the pixel row `y`.
    fn row(&self, y: usize) -> Self::Row;

    /// Blends the instance into `destination`, the pixels of the row `row`
    /// from the column `left` on.
    fn blend_span(&self, row: &Self::Row, left: usize, destination: &mut [[f32; 4]]);
}

/// The center of the pixel row or column `index`. The index of a target
/// pixel fits a `u32`, whose conversion to `f32` vectorizes.
#[inline(always)]
fn pixel_center(index: usize) -> f32 {
    index as u32 as f32 + 0.5
}

/// `color` scaled by `factor`.
#[inline(always)]
fn scale(color: [f32; 4], factor: f32) -> [f32; 4] {
    color.map(|channel| channel * factor)
}

/// Blends `source` over `destination`.
#[inline(always)]
fn blend(destination: &mut [f32; 4], source: [f32; 4]) {
    let remaining = 1.0 - source[3];
    for channel in 0..4 {
        destination[channel] = source[channel] + destination[channel] * remaining;
    }
}

/// Blends `color` with the coverage `coverage` over `destination`. A pixel
/// without coverage stays untouched.
#[inline(always)]
fn blend_coverage(destination: &mut [f32; 4], color: [f32; 4], coverage: f32) {
    if coverage > 0.0 {
        blend(destination, scale(color, coverage));
    }
}

/// The fill of a rectangle.
pub(crate) trait RectangleFill {
    /// The constants of one pixel row.
    type Row;

    /// The constants of the pixel row `y`.
    fn row(&self, y: usize) -> Self::Row;

    /// The fill color at the column `x` of the row `row`.
    fn color(&self, row: &Self::Row, x: usize) -> [f32; 4];

    /// Blends the fill into `destination`, fully covered pixels of the row
    /// `row` from the column `left` on.
    #[inline(always)]
    fn blend_interior(&self, row: &Self::Row, left: usize, destination: &mut [[f32; 4]]) {
        for (x, destination) in (left..).zip(destination) {
            blend(destination, self.color(row, x));
        }
    }
}

/// A fill with one color.
pub(crate) struct SolidFill {
    /// The fill color.
    color: [f32; 4],
}

impl RectangleFill for SolidFill {
    type Row = ();

    #[inline(always)]
    fn row(&self, _y: usize) -> Self::Row {}

    #[inline(always)]
    fn color(&self, _row: &Self::Row, _x: usize) -> [f32; 4] {
        self.color
    }

    /// Blends two pixels at once as eight channels, which suits the vector
    /// registers better than single pixels.
    #[inline(always)]
    fn blend_interior(&self, _row: &Self::Row, _left: usize, destination: &mut [[f32; 4]]) {
        let remaining = 1.0 - self.color[3];
        let source: [f32; 8] = std::array::from_fn(|index| self.color[index % 4]);
        let (pairs, rest) = destination.as_flattened_mut().as_chunks_mut::<8>();
        for pair in pairs {
            for index in 0..8 {
                pair[index] = source[index] + pair[index] * remaining;
            }
        }
        for (index, value) in rest.iter_mut().enumerate() {
            *value = source[index] + *value * remaining;
        }
    }
}

/// A vertical gradient with blue noise of up to half an 8 bit step in sRGB.
/// A smooth gradient over a few steps shows no bands with it.
pub(crate) struct GradientFill {
    /// The color at the top edge.
    top_color: [f32; 4],
    /// The color at the bottom edge.
    bottom_color: [f32; 4],
    /// The top edge of the rectangle.
    top: f32,
    /// The height of the rectangle.
    height: f32,
    /// The first column the instance evaluates.
    first_column: usize,
    /// The number of distinct noise columns the instance evaluates.
    noise_columns: usize,
}

impl GradientFill {
    /// The gradient of the rectangle `instance` over the columns from
    /// `first_column` to `end_column`.
    fn new(instance: &Instance, first_column: usize, end_column: usize) -> Self {
        Self {
            top_color: instance.color,
            bottom_color: instance.second_parameters,
            top: instance.rect[1],
            height: instance.rect[3],
            first_column,
            noise_columns: (end_column - first_column).min(BLUE_NOISE_SIZE),
        }
    }
}

impl RectangleFill for GradientFill {
    /// The dithered fill color of every noise column, indexed by the column
    /// modulo [`BLUE_NOISE_SIZE`].
    type Row = [[f32; 4]; BLUE_NOISE_SIZE];

    fn row(&self, y: usize) -> Self::Row {
        let t = saturate((pixel_center(y) - self.top) / self.height);
        let mixed: [f32; 4] = std::array::from_fn(|channel| {
            lerp(self.top_color[channel], self.bottom_color[channel], t)
        });
        let mut pattern = [mixed; BLUE_NOISE_SIZE];
        let alpha = mixed[3];
        if alpha <= 0.0 {
            return pattern;
        }
        let curves = curves();
        let encoded = [0, 1, 2].map(|channel| curves.linear_to_srgb(mixed[channel] / alpha));
        let noise_row = &BLUE_NOISE[(y % BLUE_NOISE_SIZE) * BLUE_NOISE_SIZE..][..BLUE_NOISE_SIZE];
        for x in self.first_column..self.first_column + self.noise_columns {
            let texel_x = x % BLUE_NOISE_SIZE;
            let texel = f32::from(noise_row[texel_x]) / 255.0;
            let noise = texel + 0.5 / 255.0 - 0.5;
            let channel = |channel: usize| {
                curves.srgb_to_linear(saturate(encoded[channel] + noise / 255.0)) * alpha
            };
            pattern[texel_x] = [channel(0), channel(1), channel(2), alpha];
        }
        pattern
    }

    #[inline(always)]
    fn color(&self, row: &Self::Row, x: usize) -> [f32; 4] {
        row[x % BLUE_NOISE_SIZE]
    }
}

/// A rectangle with rounded corners, an inner stroke and the fill `F`.
pub(crate) struct RectangleShader<F> {
    /// The left edge of the rectangle.
    left: f32,
    /// The top edge of the rectangle.
    top: f32,
    /// Half the width and the height of the rectangle.
    half_size: [f32; 2],
    /// The corner radii top left, top right, bottom right and bottom left.
    radii: [f32; 4],
    /// The width of the stroke inside the edge.
    stroke_width: f32,
    /// The stroke color.
    stroke_color: [f32; 4],
    /// The pixels the shape and the stroke cover fully.
    interior: Interior,
    /// The fill.
    fill: F,
}

/// One pixel row of a [`RectangleShader`].
pub(crate) struct RectangleRow<R> {
    /// The distance of the row.
    geometry: RoundedRectangleRow,
    /// The columns the shape and the stroke cover fully.
    interior: Range<usize>,
    /// The fill constants of the row.
    fill: R,
}

impl RectangleShader<SolidFill> {
    /// The shader of the rectangle `instance` with one fill color.
    pub(crate) fn solid(instance: &Instance, interior: Interior) -> Self {
        Self::new(
            instance,
            interior,
            SolidFill {
                color: instance.color,
            },
        )
    }
}

impl RectangleShader<GradientFill> {
    /// The shader of the rectangle `instance` with a gradient. The instance
    /// evaluates the columns from `first_column` to `end_column`.
    pub(crate) fn gradient(
        instance: &Instance,
        interior: Interior,
        first_column: usize,
        end_column: usize,
    ) -> Self {
        Self::new(
            instance,
            interior,
            GradientFill::new(instance, first_column, end_column),
        )
    }
}

impl<F: RectangleFill> RectangleShader<F> {
    /// The shader of the rectangle `instance` with the fill `fill`.
    fn new(instance: &Instance, interior: Interior, fill: F) -> Self {
        let [left, top, width, height] = instance.rect;
        Self {
            left,
            top,
            half_size: [width * 0.5, height * 0.5],
            radii: instance.radii,
            stroke_width: instance.parameters[0],
            stroke_color: instance.second_color,
            interior,
            fill,
        }
    }

    /// Blends the pixels of an edge span with their coverage.
    #[inline(always)]
    fn blend_edge(&self, row: &RectangleRow<F::Row>, left: usize, destination: &mut [[f32; 4]]) {
        for (x, destination) in (left..).zip(destination) {
            let position_x = pixel_center(x) - self.left - self.half_size[0];
            let distance = row.geometry.distance(position_x);
            let outer = sdf_coverage(distance);
            if outer <= 0.0 {
                continue;
            }
            let fill = self.fill.color(&row.fill, x);
            let source = if self.stroke_width > 0.0 {
                let inner = sdf_coverage(distance + self.stroke_width);
                std::array::from_fn(|channel| {
                    fill[channel] * inner + self.stroke_color[channel] * (outer - inner)
                })
            } else {
                scale(fill, outer)
            };
            blend(destination, source);
        }
    }
}

impl<F: RectangleFill> SpanShader for RectangleShader<F> {
    type Row = RectangleRow<F::Row>;

    fn row(&self, y: usize) -> Self::Row {
        let position_y = pixel_center(y) - self.top - self.half_size[1];
        RectangleRow {
            geometry: RoundedRectangleRow::new(position_y, self.half_size, self.radii),
            interior: self.interior.columns(y),
            fill: self.fill.row(y),
        }
    }

    fn blend_span(&self, row: &Self::Row, left: usize, destination: &mut [[f32; 4]]) {
        let right = left + destination.len();
        let interior_left = row.interior.start.clamp(left, right);
        let interior_right = row.interior.end.clamp(interior_left, right);
        let (left_edge, rest) = destination.split_at_mut(interior_left - left);
        let (inside, right_edge) = rest.split_at_mut(interior_right - interior_left);
        self.blend_edge(row, left, left_edge);
        self.fill.blend_interior(&row.fill, interior_left, inside);
        self.blend_edge(row, interior_right, right_edge);
    }
}

/// The soft shadow of a rectangle with rounded corners.
pub(crate) struct ShadowShader {
    /// The shadow of the rectangle relative to its center.
    shadow: Shadow,
    /// The left edge of the rectangle.
    left: f32,
    /// The top edge of the rectangle.
    top: f32,
    /// Half the width and the height of the rectangle.
    half_size: [f32; 2],
    /// The shadow color.
    color: [f32; 4],
}

impl ShadowShader {
    /// The shader of the shadow `instance`.
    pub(crate) fn new(instance: &Instance) -> Self {
        let [left, top, width, height] = instance.rect;
        let half_size = [width * 0.5, height * 0.5];
        Self {
            shadow: Shadow::new(half_size, instance.radii[0], instance.parameters[0]),
            left,
            top,
            half_size,
            color: instance.color,
        }
    }
}

impl SpanShader for ShadowShader {
    type Row = ShadowRow;

    fn row(&self, y: usize) -> Self::Row {
        self.shadow
            .row(pixel_center(y) - self.top - self.half_size[1])
    }

    fn blend_span(&self, row: &Self::Row, left: usize, destination: &mut [[f32; 4]]) {
        let mut intensities = [0.0f32; SHADOW_CHUNK];
        for (chunk_index, chunk) in destination.chunks_mut(SHADOW_CHUNK).enumerate() {
            let chunk_left = left + chunk_index * SHADOW_CHUNK;
            for (x, intensity) in (chunk_left..).zip(&mut intensities[..chunk.len()]) {
                *intensity = row.intensity(pixel_center(x) - self.left - self.half_size[0]);
            }
            for (destination, &intensity) in chunk.iter_mut().zip(&intensities) {
                blend(destination, scale(self.color, intensity));
            }
        }
    }
}

/// A ring segment with round caps.
pub(crate) struct ArcShader {
    /// The distance of the ring segment relative to its center.
    arc: Arc,
    /// The center of the ring.
    center: [f32; 2],
    /// The color.
    color: [f32; 4],
}

impl ArcShader {
    /// The shader of the arc `instance`.
    pub(crate) fn new(instance: &Instance) -> Self {
        let [center_x, center_y, radius, width] = instance.parameters;
        let [start, sweep, _, _] = instance.second_parameters;
        Self {
            arc: Arc::new(radius, width, start, sweep),
            center: [center_x, center_y],
            color: instance.color,
        }
    }
}

impl SpanShader for ArcShader {
    type Row = ArcRow;

    fn row(&self, y: usize) -> Self::Row {
        self.arc.row(pixel_center(y) - self.center[1])
    }

    fn blend_span(&self, row: &Self::Row, left: usize, destination: &mut [[f32; 4]]) {
        for (x, destination) in (left..).zip(destination) {
            let distance = self.arc.distance(row, pixel_center(x) - self.center[0]);
            blend_coverage(destination, self.color, sdf_coverage(distance));
        }
    }
}

/// A line with round caps.
pub(crate) struct LineShader {
    /// The distance of the line.
    line: Line,
    /// The color.
    color: [f32; 4],
}

impl LineShader {
    /// The shader of the line `instance`.
    pub(crate) fn new(instance: &Instance) -> Self {
        let [from_x, from_y, to_x, to_y] = instance.parameters;
        Self {
            line: Line::new(
                [from_x, from_y],
                [to_x, to_y],
                instance.second_parameters[0],
            ),
            color: instance.color,
        }
    }
}

impl SpanShader for LineShader {
    type Row = LineRow;

    fn row(&self, y: usize) -> Self::Row {
        self.line.row(pixel_center(y))
    }

    fn blend_span(&self, row: &Self::Row, left: usize, destination: &mut [[f32; 4]]) {
        for (x, destination) in (left..).zip(destination) {
            let distance = self.line.distance(row, pixel_center(x));
            blend_coverage(destination, self.color, sdf_coverage(distance));
        }
    }
}

/// A Slug outline of a glyph or a path.
pub(crate) struct GlyphShader<'a> {
    /// The glyph record.
    glyph: &'a SlugGlyph,
    /// The curves of all glyphs.
    curves: &'a [[f32; 4]],
    /// The bands of all glyphs.
    bands: &'a [u32],
    /// The left and the top edge of the item rectangle.
    origin: [f32; 2],
    /// The em space coordinate at the left and the top edge.
    em_origin: [f32; 2],
    /// The em units per pixel along x and y.
    em_per_pixel: [f32; 2],
    /// The pixels per em unit along x and y.
    pixels_per_em: [f32; 2],
    /// The color.
    color: [f32; 4],
    /// The exponent of the coverage curve. Light text has the exponent 1.
    exponent: f32,
}

impl<'a> GlyphShader<'a> {
    /// The shader of the glyph `instance`. A glyph index outside the glyphs of
    /// `slug` yields no shader.
    pub(crate) fn new(instance: &Instance, slug: &'a SlugData) -> Option<Self> {
        let glyph = slug.glyphs().get(instance.kind[1] as usize)?;
        let [em_left, em_top, em_per_pixel_x, em_per_pixel_y] = instance.parameters;
        let color = instance.color;
        let weighted = color[0] * LUMINANCE_WEIGHTS[0]
            + color[1] * LUMINANCE_WEIGHTS[1]
            + color[2] * LUMINANCE_WEIGHTS[2];
        let luminance = weighted / color[3].max(1.0 / 255.0);
        Some(Self {
            glyph,
            curves: slug.curves(),
            bands: slug.bands(),
            origin: [instance.rect[0], instance.rect[1]],
            em_origin: [em_left, em_top],
            em_per_pixel: [em_per_pixel_x, em_per_pixel_y],
            pixels_per_em: [1.0 / em_per_pixel_x.abs(), 1.0 / em_per_pixel_y.abs()],
            color,
            exponent: lerp(2.2, 1.0, saturate(luminance * 4.0)),
        })
    }

    /// Blending in linear light thins dark text on a light background. The
    /// coverage of dark text follows the curve of a blend in sRGB. Light text
    /// keeps the linear coverage.
    #[inline(always)]
    fn text_coverage(&self, coverage: f32) -> f32 {
        if self.exponent == 1.0 || coverage >= 1.0 {
            return coverage;
        }
        1.0 - (1.0 - coverage).powf(self.exponent)
    }
}

impl SpanShader for GlyphShader<'_> {
    /// The em space y coordinate of the row.
    type Row = f32;

    fn row(&self, y: usize) -> Self::Row {
        self.em_origin[1] + (pixel_center(y) - self.origin[1]) * self.em_per_pixel[1]
    }

    fn blend_span(&self, row: &Self::Row, left: usize, destination: &mut [[f32; 4]]) {
        for (x, destination) in (left..).zip(destination) {
            let em_x =
                self.em_origin[0] + (pixel_center(x) - self.origin[0]) * self.em_per_pixel[0];
            let coverage = slug_coverage(
                self.curves,
                self.bands,
                self.glyph,
                [em_x, *row],
                self.pixels_per_em,
            );
            if coverage > 0.0 {
                blend(destination, scale(self.color, self.text_coverage(coverage)));
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        super::{
            sdf::tests::{sdf_arc, sdf_line, sdf_rounded_rectangle},
            shadow::tests::rounded_rectangle_shadow,
        },
        *,
    };

    /// A fully transparent color.
    const TRANSPARENT: [f32; 4] = [0.0; 4];

    /// Adds blue noise of up to half an 8 bit step in sRGB to a premultiplied
    /// color evaluated for one pixel.
    fn dither(color: [f32; 4], pixel: [f32; 2]) -> [f32; 4] {
        let alpha = color[3];
        if alpha <= 0.0 {
            return color;
        }
        let texel_x = (pixel[0] as i32).rem_euclid(BLUE_NOISE_SIZE as i32) as usize;
        let texel_y = (pixel[1] as i32).rem_euclid(BLUE_NOISE_SIZE as i32) as usize;
        let texel = f32::from(BLUE_NOISE[texel_y * BLUE_NOISE_SIZE + texel_x]) / 255.0;
        let noise = texel + 0.5 / 255.0 - 0.5;
        let curves = curves();
        let channel = |value: f32| {
            let srgb = curves.linear_to_srgb(value / alpha) + noise / 255.0;
            curves.srgb_to_linear(saturate(srgb)) * alpha
        };
        [
            channel(color[0]),
            channel(color[1]),
            channel(color[2]),
            alpha,
        ]
    }

    /// A rectangle evaluated for one pixel.
    fn rectangle(instance: &Instance, pixel: [f32; 2]) -> [f32; 4] {
        let [x, y, width, height] = instance.rect;
        let half_size = [width * 0.5, height * 0.5];
        let position = [pixel[0] - x - half_size[0], pixel[1] - y - half_size[1]];
        let distance = sdf_rounded_rectangle(position, half_size, instance.radii);
        let outer = sdf_coverage(distance);
        if outer <= 0.0 {
            return TRANSPARENT;
        }

        let mut fill = instance.color;
        if instance.parameters[1] > 0.5 {
            let t = saturate((pixel[1] - y) / height);
            let bottom = instance.second_parameters;
            let mixed = std::array::from_fn(|channel| lerp(fill[channel], bottom[channel], t));
            fill = dither(mixed, pixel);
        }

        let stroke_width = instance.parameters[0];
        if stroke_width > 0.0 {
            let inner = sdf_coverage(distance + stroke_width);
            let stroke = instance.second_color;
            return std::array::from_fn(|channel| {
                fill[channel] * inner + stroke[channel] * (outer - inner)
            });
        }
        scale(fill, outer)
    }

    /// A shadow evaluated for one pixel.
    fn shadow(instance: &Instance, pixel: [f32; 2]) -> [f32; 4] {
        let [x, y, width, height] = instance.rect;
        let half_size = [width * 0.5, height * 0.5];
        let position = [pixel[0] - x - half_size[0], pixel[1] - y - half_size[1]];
        let intensity = rounded_rectangle_shadow(
            position,
            half_size,
            instance.radii[0],
            instance.parameters[0],
        );
        scale(instance.color, intensity)
    }

    /// An arc evaluated for one pixel.
    fn arc(instance: &Instance, pixel: [f32; 2]) -> [f32; 4] {
        let [center_x, center_y, radius, width] = instance.parameters;
        let [start, sweep, _, _] = instance.second_parameters;
        let distance = sdf_arc(
            [pixel[0] - center_x, pixel[1] - center_y],
            radius,
            width,
            start,
            sweep,
        );
        scale(instance.color, sdf_coverage(distance))
    }

    /// A line evaluated for one pixel.
    fn line(instance: &Instance, pixel: [f32; 2]) -> [f32; 4] {
        let [from_x, from_y, to_x, to_y] = instance.parameters;
        let distance = sdf_line(
            pixel,
            [from_x, from_y],
            [to_x, to_y],
            instance.second_parameters[0],
        );
        scale(instance.color, sdf_coverage(distance))
    }

    /// The text coverage curve evaluated for one pixel.
    fn text_coverage(coverage: f32, color: [f32; 4]) -> f32 {
        let weighted = color[0] * LUMINANCE_WEIGHTS[0]
            + color[1] * LUMINANCE_WEIGHTS[1]
            + color[2] * LUMINANCE_WEIGHTS[2];
        let luminance = weighted / color[3].max(1.0 / 255.0);
        let exponent = lerp(2.2, 1.0, saturate(luminance * 4.0));
        if exponent == 1.0 || coverage >= 1.0 {
            return coverage;
        }
        1.0 - (1.0 - coverage).powf(exponent)
    }

    /// A glyph evaluated for one pixel.
    fn glyph(instance: &Instance, slug: &SlugData, pixel: [f32; 2]) -> [f32; 4] {
        let Some(glyph) = slug.glyphs().get(instance.kind[1] as usize) else {
            return TRANSPARENT;
        };
        let [em_left, em_top, em_per_pixel_x, em_per_pixel_y] = instance.parameters;
        let em = [
            em_left + (pixel[0] - instance.rect[0]) * em_per_pixel_x,
            em_top + (pixel[1] - instance.rect[1]) * em_per_pixel_y,
        ];
        let pixels_per_em = [1.0 / em_per_pixel_x.abs(), 1.0 / em_per_pixel_y.abs()];
        let coverage = slug_coverage(slug.curves(), slug.bands(), glyph, em, pixels_per_em);
        if coverage <= 0.0 {
            return TRANSPARENT;
        }
        scale(instance.color, text_coverage(coverage, instance.color))
    }

    /// The color of `instance` at the pixel center `pixel`, evaluated without
    /// any constant of an instance or a row. It is the reference of the span
    /// shaders.
    pub(crate) fn shade(instance: &Instance, slug: &SlugData, pixel: [f32; 2]) -> [f32; 4] {
        match instance.kind[0] {
            Instance::KIND_RECTANGLE => rectangle(instance, pixel),
            Instance::KIND_SHADOW => shadow(instance, pixel),
            Instance::KIND_ARC => arc(instance, pixel),
            Instance::KIND_LINE => line(instance, pixel),
            Instance::KIND_GLYPH => glyph(instance, slug, pixel),
            _ => TRANSPARENT,
        }
    }
}
