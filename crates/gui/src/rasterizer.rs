//! The CPU rasterizer of a scene.
//!
//! The rasterizer is the reference implementation of the scene renderer. The
//! GPU shader evaluates the same math for every pixel.
//!
//! The target splits into horizontal strips. Every instance is binned into the
//! strips its pixel area touches. A worker blends the instances of a strip in
//! painter's order into a buffer of linear, premultiplied `f32` colors and
//! encodes the strip into the output. The workers take the strips from a
//! shared queue.
//!
//! The worker checks the kind of an instance once and blends it row by row
//! with the span shader of the kind. A shader computes the constants of the
//! instance and of every row once.
//!
//! The interior of a rectangle has a coverage of exactly 1 and blends its
//! fill without the distance function. The opaque interior of a rectangle
//! replaces every color below it. A worker skips the runs of pixels of an
//! instance that a later opaque interior covers. The output stays the same as
//! with the evaluation of every pixel.

mod curves;
mod fragment;
mod math;
mod sdf;
mod shadow;
pub(crate) mod slug;

use std::{num::NonZeroUsize, ops::Range, sync::Mutex, thread};

use self::{
    curves::curves,
    fragment::{ArcShader, GlyphShader, LineShader, RectangleShader, ShadowShader, SpanShader},
};
use crate::{Color, Instance, Scene, SlugData};

/// The width and the height of the blue noise tile.
pub const BLUE_NOISE_SIZE: usize = 128;

/// A tile of blue noise with one byte for each texel, in rows from the top.
/// The gradient dither reads it as values from 0 to 1.
pub static BLUE_NOISE: &[u8; BLUE_NOISE_SIZE * BLUE_NOISE_SIZE] =
    include_bytes!("../assets/bluenoise.r8");

/// The number of occluders a run search compares at once.
const OCCLUDER_CHUNK: usize = 16;

/// The number of pixel rows in one strip.
const STRIP_HEIGHT: usize = 4;

/// The distance in pixels past the corner radius and the stroke at which the
/// coverage of a rectangle is exactly 1. The coverage reaches 1 at half a pixel
/// inside the edge. The rest absorbs rounding.
const OPAQUE_INSET: f32 = 1.0;

/// The largest edge coordinate of a rectangle whose interior is computed.
/// Past it the rounding of the distance in `f32` exceeds the slack of
/// [`OPAQUE_INSET`].
const OPAQUE_COORDINATE_LIMIT: f32 = 1_048_576.0;

/// A block of pixels as half open ranges of columns and rows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PixelArea {
    /// The first column.
    left: usize,
    /// The first row.
    top: usize,
    /// The column past the last column.
    right: usize,
    /// The row past the last row.
    bottom: usize,
}

impl PixelArea {
    /// The pixels of a target of `width` x `height` pixels whose centers lie
    /// inside the edges `[left, top, right, bottom]`. A center on the left or
    /// top edge lies inside, a center on the right or bottom edge outside, as
    /// with the rasterization rule of the GPU.
    fn inside(edges: [f32; 4], width: usize, height: usize) -> Self {
        let first = |edge: f32, limit: usize| (edge - 0.5).ceil().clamp(0.0, limit as f32) as usize;
        let area = Self {
            left: first(edges[0], width),
            top: first(edges[1], height),
            right: first(edges[2], width),
            bottom: first(edges[3], height),
        };
        if area.is_empty() {
            return Self::default();
        }
        area
    }

    /// The pixels `instance` evaluates. The item rectangle grows by the
    /// margin of the kind and is limited by the clip and the target. An
    /// instance whose rectangle or margin is not finite evaluates no pixel.
    fn of(instance: &Instance, width: usize, height: usize) -> Self {
        let [x, y, item_width, item_height] = instance.rect;
        let [clip_left, clip_top, clip_right, clip_bottom] = instance.clip;
        let margin = if instance.kind[0] == Instance::KIND_SHADOW {
            3.0 * instance.parameters[0] + 1.0
        } else {
            1.0
        };
        if ![x, y, item_width, item_height, margin]
            .iter()
            .all(|value| value.is_finite())
        {
            return Self::default();
        }
        let edges = [
            (x - margin).max(clip_left),
            (y - margin).max(clip_top),
            (x + item_width + margin).min(clip_right),
            (y + item_height + margin).min(clip_bottom),
        ];
        Self::inside(edges, width, height)
    }

    /// The pixels in both areas.
    fn intersection(self, other: Self) -> Self {
        let area = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        if area.is_empty() {
            return Self::default();
        }
        area
    }

    /// Whether the area holds no pixel.
    fn is_empty(&self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }

    /// The rows of the area within the rows `rows`.
    fn rows_within(&self, rows: Range<usize>) -> Range<usize> {
        self.top.max(rows.start)..self.bottom.min(rows.end)
    }
}

/// The pixels of a rectangle that its shape and its stroke cover fully. They
/// form a cross of two bands. Within the straight part of an edge the corners
/// do not reduce the coverage.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Interior {
    /// The rows inset by the corner radius and the stroke, with the columns
    /// inset by the stroke.
    wide: PixelArea,
    /// The rows inset by the stroke, with the columns inset by the corner
    /// radius and the stroke.
    tall: PixelArea,
    /// Whether the fill is opaque in the interior. The interior then hides
    /// every color below it.
    opaque: bool,
}

impl Interior {
    /// The interior of `instance` within `area`. Only rectangles have one.
    /// A negative corner radius insets the bands by its magnitude. Its
    /// rounding in the distance then stays within the slack.
    fn of(instance: &Instance, area: PixelArea, width: usize, height: usize) -> Self {
        if instance.kind[0] != Instance::KIND_RECTANGLE {
            return Self::default();
        }
        let [x, y, item_width, item_height] = instance.rect;
        let outermost = [x, y, x + item_width, y + item_height]
            .into_iter()
            .fold(0.0f32, |largest, edge| largest.max(edge.abs()));
        if outermost > OPAQUE_COORDINATE_LIMIT {
            return Self::default();
        }
        let half_side = item_width.min(item_height) * 0.5;
        let radius = instance
            .radii
            .into_iter()
            .map(|radius| radius.min(half_side).abs())
            .fold(0.0f32, f32::max);
        let stroke_inset = instance.parameters[0].max(0.0) + OPAQUE_INSET;
        let corner_inset = radius + stroke_inset;
        let band = |inset_x: f32, inset_y: f32| {
            let edges = [
                x + inset_x,
                y + inset_y,
                x + item_width - inset_x,
                y + item_height - inset_y,
            ];
            PixelArea::inside(edges, width, height).intersection(area)
        };
        let gradient = instance.parameters[1] > 0.5;
        Self {
            wide: band(stroke_inset, corner_inset),
            tall: band(corner_inset, stroke_inset),
            opaque: instance.color[3] == 1.0 && (!gradient || instance.second_parameters[3] == 1.0),
        }
    }

    /// The columns of the interior in the row `y`. A row outside the
    /// interior yields an empty range.
    fn columns(&self, y: usize) -> Range<usize> {
        for band in [self.wide, self.tall] {
            if (band.top..band.bottom).contains(&y) {
                return band.left..band.right;
            }
        }
        0..0
    }

    /// The bands that hide every color below them.
    fn opaque_bands(&self) -> impl Iterator<Item = PixelArea> {
        [self.wide, self.tall]
            .into_iter()
            .filter(|band| self.opaque && !band.is_empty())
    }
}

/// The inputs of a rasterized frame that are not part of the instances.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FrameKey {
    /// The width of the target in pixels.
    width: u32,
    /// The height of the target in pixels.
    height: u32,
    /// The color the frame starts with.
    clear: Color,
    /// The generation of the Slug data.
    slug_generation: u64,
}

/// The shared read only state of the workers of one frame.
struct Frame<'a> {
    /// The draw items in painter's order.
    instances: &'a [Instance],
    /// The pixels every instance evaluates.
    areas: &'a [PixelArea],
    /// The interior of every instance.
    interiors: &'a [Interior],
    /// The instance indices of every strip in painter's order.
    bins: &'a [Vec<u32>],
    /// The Slug data of the glyphs and paths.
    slug: &'a SlugData,
    /// The width of the target in pixels.
    width: usize,
    /// The linear, premultiplied color the frame starts with.
    clear: [f32; 4],
    /// The encoded output bytes of the clear color.
    encoded_clear: [u8; 4],
}

/// The working memory of one worker.
#[derive(Default)]
struct StripBuffers {
    /// The linear, premultiplied color of every pixel of the strip.
    colors: Vec<[f32; 4]>,
    /// For every pixel of the strip, the bin position plus one of the last
    /// instance that is opaque there, or 0.
    occluders: Vec<u32>,
}

/// Draws a [`Scene`] on the CPU into RGBA8 pixels.
///
/// The output holds sRGB encoded, premultiplied color with a linear alpha in
/// rows from the top. It suits a texture with premultiplied alpha blending.
/// The rasterizer keeps its buffers between frames and allocates only when
/// the target or the scene grows.
pub struct Rasterizer {
    /// The number of workers.
    thread_count: usize,
    /// The working memory of every worker.
    strip_buffers: Vec<StripBuffers>,
    /// The instance indices of every strip.
    bins: Vec<Vec<u32>>,
    /// The pixels every instance evaluates.
    areas: Vec<PixelArea>,
    /// The interior of every instance.
    interiors: Vec<Interior>,
    /// The inputs of the last frame besides its instances. `None` before the
    /// first frame.
    last_frame: Option<FrameKey>,
    /// The instances of the last frame as bytes. A bitwise comparison
    /// treats NaN like any other value.
    last_instance_bytes: Vec<u8>,
}

impl Default for Rasterizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Rasterizer {
    /// A rasterizer with one worker for each hardware thread.
    pub fn new() -> Self {
        Self::with_thread_count(thread::available_parallelism().map_or(1, NonZeroUsize::get))
    }

    /// A rasterizer with `thread_count` workers. A count of 1 renders on the
    /// calling thread.
    pub fn with_thread_count(thread_count: usize) -> Self {
        Self {
            thread_count: thread_count.max(1),
            strip_buffers: Vec::new(),
            bins: Vec::new(),
            areas: Vec::new(),
            interiors: Vec::new(),
            last_frame: None,
            last_instance_bytes: Vec::new(),
        }
    }

    /// The number of workers.
    pub fn thread_count(&self) -> usize {
        self.thread_count
    }

    /// Forgets the last frame. The next call of [`Rasterizer::rasterize`]
    /// draws in any case.
    pub fn invalidate(&mut self) {
        self.last_frame = None;
    }

    /// Draws `scene` with the glyphs and paths of `slug` into `output`, a
    /// target of `width` x `height` pixels with 4 bytes for each pixel.
    ///
    /// The call leaves `output` untouched and returns `false` when the scene,
    /// the target size and the [`SlugData::generation`] equal those of the
    /// last drawn frame. The caller passes the same buffer every frame, or
    /// calls [`Rasterizer::invalidate`] after it replaced the buffer.
    ///
    /// # Panics
    ///
    /// Panics when the length of `output` differs from `width * height * 4`.
    pub fn rasterize(
        &mut self,
        scene: &Scene,
        slug: &SlugData,
        width: u32,
        height: u32,
        output: &mut [u8],
    ) -> bool {
        let pixel_width = width as usize;
        let pixel_height = height as usize;
        assert_eq!(
            output.len(),
            pixel_width * pixel_height * 4,
            "the output does not match the target size"
        );

        let key = FrameKey {
            width,
            height,
            clear: scene.clear_color(),
            slug_generation: slug.generation(),
        };
        if self.last_frame == Some(key) && self.last_instance_bytes == scene.instance_bytes() {
            return false;
        }
        self.last_frame = None;
        self.last_instance_bytes.clear();
        self.last_instance_bytes
            .extend_from_slice(scene.instance_bytes());

        if output.is_empty() {
            self.last_frame = Some(key);
            return true;
        }

        self.bin(scene.instances(), pixel_width, pixel_height);

        let clear = scene.clear_color().to_linear();
        let frame = Frame {
            instances: scene.instances(),
            areas: &self.areas,
            interiors: &self.interiors,
            bins: &self.bins,
            slug,
            width: pixel_width,
            clear,
            encoded_clear: curves().encode_pixel(clear),
        };

        let workers = if scene.is_empty() {
            1
        } else {
            self.thread_count.min(self.bins.len())
        };
        if self.strip_buffers.len() < workers {
            self.strip_buffers
                .resize_with(workers, StripBuffers::default);
        }

        let strips = Mutex::new(
            output
                .chunks_mut(pixel_width * 4 * STRIP_HEIGHT)
                .enumerate(),
        );
        let (buffers, other_buffers) = self.strip_buffers[..workers]
            .split_first_mut()
            .expect("there is at least one worker");
        let frame = &frame;
        let strips = &strips;
        thread::scope(|scope| {
            for other in other_buffers {
                scope.spawn(move || work(frame, strips, other));
            }
            work(frame, strips, buffers);
        });
        self.last_frame = Some(key);
        true
    }

    /// Computes the pixel areas of every instance and bins the instances into
    /// the strips they touch.
    fn bin(&mut self, instances: &[Instance], width: usize, height: usize) {
        let strip_count = height.div_ceil(STRIP_HEIGHT);
        self.bins.resize_with(strip_count, Vec::new);
        for bin in &mut self.bins {
            bin.clear();
        }
        self.areas.clear();
        self.interiors.clear();
        for (index, instance) in instances.iter().enumerate() {
            let area = PixelArea::of(instance, width, height);
            self.areas.push(area);
            self.interiors
                .push(Interior::of(instance, area, width, height));
            if area.is_empty() {
                continue;
            }
            for bin in &mut self.bins[area.top / STRIP_HEIGHT..=(area.bottom - 1) / STRIP_HEIGHT] {
                bin.push(index as u32);
            }
        }
    }
}

/// Renders strips from the shared queue until it is empty.
fn work<'a>(
    frame: &Frame,
    strips: &Mutex<impl Iterator<Item = (usize, &'a mut [u8])>>,
    buffers: &mut StripBuffers,
) {
    loop {
        let next = match strips.lock() {
            Ok(mut strips) => strips.next(),
            Err(_) => None,
        };
        let Some((strip, rows)) = next else {
            return;
        };
        render_strip(frame, strip, rows, buffers);
    }
}

/// Blends the instances of the strip `strip` and encodes the result into
/// `rows`, the output bytes of the strip.
fn render_strip(frame: &Frame, strip: usize, rows: &mut [u8], buffers: &mut StripBuffers) {
    let width = frame.width;
    let bin = &frame.bins[strip];
    if bin.is_empty() {
        rows.as_chunks_mut::<4>().0.fill(frame.encoded_clear);
        return;
    }

    let row_count = rows.len() / (width * 4);
    let strip_rows = strip * STRIP_HEIGHT..strip * STRIP_HEIGHT + row_count;
    let StripBuffers { colors, occluders } = buffers;
    colors.clear();
    colors.resize(width * row_count, frame.clear);
    occluders.clear();
    occluders.resize(width * row_count, 0);

    let mut last_occluder = 0;
    for (position, &index) in bin.iter().enumerate() {
        let order = position as u32 + 1;
        for band in frame.interiors[index as usize].opaque_bands() {
            for y in band.rows_within(strip_rows.clone()) {
                let row_start = (y - strip_rows.start) * width;
                occluders[row_start + band.left..row_start + band.right].fill(order);
                last_occluder = order;
            }
        }
    }

    let mut target = StripTarget {
        colors,
        occluders,
        width,
        top: strip_rows.start,
    };
    for (position, &index) in bin.iter().enumerate() {
        let order = position as u32 + 1;
        let index = index as usize;
        let instance = &frame.instances[index];
        let area = frame.areas[index];
        let pixels = InstancePixels {
            rows: area.rows_within(strip_rows.clone()),
            columns: area.left..area.right,
            order,
            occluded: order < last_occluder,
        };
        match instance.kind[0] {
            Instance::KIND_RECTANGLE if instance.parameters[1] > 0.5 => {
                let shader = RectangleShader::gradient(
                    instance,
                    frame.interiors[index],
                    area.left,
                    area.right,
                );
                target.draw(&shader, pixels);
            }
            Instance::KIND_RECTANGLE => {
                let shader = RectangleShader::solid(instance, frame.interiors[index]);
                target.draw(&shader, pixels);
            }
            Instance::KIND_SHADOW => target.draw(&ShadowShader::new(instance), pixels),
            Instance::KIND_ARC => target.draw(&ArcShader::new(instance), pixels),
            Instance::KIND_LINE => target.draw(&LineShader::new(instance), pixels),
            Instance::KIND_GLYPH => {
                if let Some(shader) = GlyphShader::new(instance, frame.slug) {
                    target.draw(&shader, pixels);
                }
            }
            _ => {}
        }
    }

    let curves = curves();
    let mut previous = (frame.clear, frame.encoded_clear);
    for (pixel, &color) in rows.as_chunks_mut::<4>().0.iter_mut().zip(colors.iter()) {
        if color != previous.0 {
            previous = (color, curves.encode_pixel(color));
        }
        *pixel = previous.1;
    }
}

/// The pixels of an instance within a strip.
struct InstancePixels {
    /// The rows of the target.
    rows: Range<usize>,
    /// The columns of the target.
    columns: Range<usize>,
    /// The bin position of the instance plus one.
    order: u32,
    /// Whether a later opaque interior may cover some of the pixels.
    occluded: bool,
}

/// The strip buffers of a worker with the position of the strip.
struct StripTarget<'a> {
    /// The linear, premultiplied color of every pixel of the strip.
    colors: &'a mut [[f32; 4]],
    /// For every pixel of the strip, the bin position plus one of the last
    /// instance that is opaque there, or 0.
    occluders: &'a [u32],
    /// The width of the target in pixels.
    width: usize,
    /// The first row of the strip.
    top: usize,
}

impl StripTarget<'_> {
    /// Blends the pixels `pixels` of an instance with `shader`. The pixels a
    /// later opaque interior covers keep their color. A row without a visible
    /// pixel computes no row constants.
    fn draw<S: SpanShader>(&mut self, shader: &S, pixels: InstancePixels) {
        let InstancePixels {
            rows,
            columns,
            order,
            occluded,
        } = pixels;
        for y in rows {
            let row_start = (y - self.top) * self.width;
            let colors = &mut self.colors[row_start..row_start + self.width];
            if !occluded {
                shader.blend_span(&shader.row(y), columns.start, &mut colors[columns.clone()]);
                continue;
            }
            let occluders = &self.occluders[row_start + columns.start..row_start + columns.end];
            let mut row = None;
            let mut start = 0;
            while let Some(skipped) =
                find_occluder(&occluders[start..], |occluder| occluder <= order)
            {
                start += skipped;
                let length = find_occluder(&occluders[start..], |occluder| occluder > order)
                    .unwrap_or(occluders.len() - start);
                let left = columns.start + start;
                let row = row.get_or_insert_with(|| shader.row(y));
                shader.blend_span(row, left, &mut colors[left..left + length]);
                start += length;
            }
        }
    }
}

/// The index of the first occluder in `occluders` that satisfies
/// `predicate`. The search compares chunks of [`OCCLUDER_CHUNK`] occluders
/// at once.
#[inline(always)]
fn find_occluder(occluders: &[u32], predicate: impl Fn(u32) -> bool) -> Option<usize> {
    let (chunks, rest) = occluders.as_chunks::<OCCLUDER_CHUNK>();
    for (chunk_index, chunk) in chunks.iter().enumerate() {
        if chunk
            .iter()
            .fold(false, |found, &occluder| found | predicate(occluder))
        {
            let offset = chunk.iter().position(|&occluder| predicate(occluder))?;
            return Some(chunk_index * OCCLUDER_CHUNK + offset);
        }
    }
    let offset = rest.iter().position(|&occluder| predicate(occluder))?;
    Some(chunks.len() * OCCLUDER_CHUNK + offset)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{
        f32::consts::{PI, TAU},
        path::PathBuf,
        time::Instant,
    };

    use super::{fragment::tests::shade, *};
    use crate::{Path, Rect, RectStyle, ShapeStore, SlugShape};

    /// The environment variable that names the directory of the screenshots.
    const SHOTS_VARIABLE: &str = "GUI_SHOTS";

    /// The size of a square of the checkerboard behind a screenshot.
    const CHECKER_SIZE: usize = 8;

    /// Writes the RGBA8 output of the rasterizer as `<name>.ppm` into the
    /// directory of the `GUI_SHOTS` environment variable. The output is composited
    /// over a gray checkerboard. Without the variable nothing is written.
    pub(crate) fn write_ppm(name: &str, width: u32, height: u32, rgba: &[u8]) {
        let Some(directory) = std::env::var_os(SHOTS_VARIABLE) else {
            eprintln!("{SHOTS_VARIABLE} is not set, {name} is not written");
            return;
        };
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).expect("the screenshot directory can be created");
        let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
        for (index, pixel) in rgba.as_chunks::<4>().0.iter().enumerate() {
            let x = index % width as usize;
            let y = index / width as usize;
            let background: u32 = if (x / CHECKER_SIZE + y / CHECKER_SIZE).is_multiple_of(2) {
                0x50
            } else {
                0x60
            };
            let remaining = 255 - u32::from(pixel[3]);
            for &channel in &pixel[..3] {
                bytes.push(
                    (u32::from(channel) + (background * remaining + 127) / 255).min(255) as u8,
                );
            }
        }
        let path = directory.join(format!("{name}.ppm"));
        std::fs::write(&path, bytes).expect("the screenshot can be written");
        eprintln!("wrote {}", path.display());
    }

    /// Rasterizes `scene` into a new buffer.
    fn render(scene: &Scene, slug: &SlugData, width: u32, height: u32) -> Vec<u8> {
        let mut output = vec![0; width as usize * height as usize * 4];
        Rasterizer::new().rasterize(scene, slug, width, height, &mut output);
        output
    }

    /// The sum of the alpha of all pixels, in pixels.
    fn covered_area(output: &[u8]) -> f32 {
        output
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| f32::from(pixel[3]) / 255.0)
            .sum()
    }

    /// Asserts that `actual` lies within `tolerance` relative to `expected`.
    fn assert_close(actual: f32, expected: f32, tolerance: f32, what: &str) {
        assert!(
            (actual - expected).abs() <= expected * tolerance,
            "{what}: area {actual} differs from {expected}"
        );
    }

    /// A scene at scale 1 that starts transparent.
    fn empty_scene() -> Scene {
        let mut scene = Scene::new();
        scene.begin(1.0, Color::TRANSPARENT);
        scene
    }

    /// A circle of four cubic curves with its center at `(x, y)`.
    fn circle(path: Path, x: f32, y: f32, radius: f32) -> Path {
        let handle = radius * 0.552_284_8;
        path.move_to(x + radius, y)
            .cubic_to(
                x + radius,
                y + handle,
                x + handle,
                y + radius,
                x,
                y + radius,
            )
            .cubic_to(
                x - handle,
                y + radius,
                x - radius,
                y + handle,
                x - radius,
                y,
            )
            .cubic_to(
                x - radius,
                y - handle,
                x - handle,
                y - radius,
                x,
                y - radius,
            )
            .cubic_to(
                x + handle,
                y - radius,
                x + radius,
                y - handle,
                x + radius,
                y,
            )
            .close()
    }

    /// A star with `points` tips inside the unit box.
    fn star(points: usize) -> Path {
        let vertices: Vec<[f32; 2]> = (0..points * 2)
            .map(|index| {
                let radius = if index % 2 == 0 { 0.5 } else { 0.22 };
                let angle = TAU * index as f32 / (points * 2) as f32 - PI * 0.5;
                [0.5 + radius * angle.cos(), 0.5 + radius * angle.sin()]
            })
            .collect();
        Path::new().polygon(&vertices)
    }

    /// The icon shapes of the sample scene.
    fn sample_shapes(store: &mut ShapeStore) -> Vec<SlugShape> {
        let paths = [
            circle(Path::new(), 0.5, 0.5, 0.45),
            Path::new().polygon(&[[0.1, 0.9], [0.5, 0.1], [0.9, 0.9]]),
            star(5),
            Path::new()
                .polygon(&[[0.1, 0.1], [0.9, 0.1], [0.9, 0.9], [0.1, 0.9]])
                .polygon(&[[0.3, 0.3], [0.3, 0.7], [0.7, 0.7], [0.7, 0.3]]),
            Path::new()
                .move_to(0.1, 0.8)
                .quad_to(0.5, -0.2, 0.9, 0.8)
                .quad_to(0.5, 0.4, 0.1, 0.8)
                .close(),
            circle(circle(Path::new(), 0.5, 0.5, 0.45), 0.5, 0.5, 0.2),
        ];
        paths
            .iter()
            .map(|path| store.add_path(path).expect("the sample path has an area"))
            .collect()
    }

    /// A dialog over a veil with every kind of item. It fills a target of
    /// `width` x `height` pixels at a scale of one logical unit for each 1080th of
    /// the height.
    fn sample_scene(width: u32, height: u32) -> (Scene, ShapeStore) {
        let mut store = ShapeStore::new();
        let shapes = sample_shapes(&mut store);
        let scale = height as f32 / 1080.0;
        let logical_width = width as f32 / scale;
        let logical_height = 1080.0;

        let mut scene = Scene::new();
        scene.begin(scale, Color::TRANSPARENT);
        scene.fill(
            Rect::new(0.0, 0.0, logical_width, logical_height),
            0.0,
            Color::hex(0x101018).with_alpha(0.6),
        );

        let panel = Rect::new(
            logical_width * 0.5 - 600.0,
            logical_height * 0.5 - 400.0,
            1200.0,
            800.0,
        );
        scene.shadow(panel, 16.0, 24.0, (0.0, 8.0), Color::BLACK.with_alpha(0.6));
        scene.rect(
            panel,
            &RectStyle::filled(Color::hex(0x2A2D3A), 16.0)
                .with_gradient(Color::hex(0x1E2029))
                .with_stroke(1.0, Color::hex(0x4A4F66)),
        );

        let header = Rect::new(panel.x, panel.y, panel.width, 72.0);
        scene.rect(
            header,
            &RectStyle {
                radii: [16.0, 16.0, 0.0, 0.0],
                fill: Color::hex(0x3B4160),
                fill_bottom: Some(Color::hex(0x30354D)),
                ..RectStyle::default()
            },
        );
        for index in 0..40 {
            let shape = shapes[index % shapes.len()];
            scene.path(
                (panel.x + 24.0 + index as f32 * 22.0, panel.y + 26.0),
                20.0,
                shape,
                Color::hex(0xF0F0F8),
            );
        }

        for tab in 0..3 {
            let tab_rect = Rect::new(
                panel.x + 24.0 + tab as f32 * 180.0,
                panel.y + 88.0,
                168.0,
                40.0,
            );
            let style = if tab == 0 {
                RectStyle::filled(Color::hex(0x5870D0), 8.0)
            } else {
                RectStyle::filled(Color::hex(0x2A2D3A), 8.0).with_stroke(1.0, Color::hex(0x4A4F66))
            };
            scene.rect(tab_rect, &style);
        }

        let list = Rect::new(panel.x + 24.0, panel.y + 144.0, panel.width - 48.0, 560.0);
        scene.fill(list, 8.0, Color::hex(0x16181F));
        scene.push_clip(list);
        for row in 0..24 {
            let row_y = list.y + row as f32 * 28.0 - 10.0;
            if row == 5 {
                scene.fill(
                    Rect::new(list.x + 4.0, row_y, list.width - 8.0, 28.0),
                    6.0,
                    Color::hex(0x3E5BC2),
                );
            } else if row % 2 == 1 {
                scene.fill(
                    Rect::new(list.x, row_y, list.width, 28.0),
                    0.0,
                    Color::hex(0x1B1E27),
                );
            }
            let glyph_count = 30 + (row * 7) % 30;
            for index in 0..glyph_count {
                let shape = shapes[(row + index) % shapes.len()];
                scene.path(
                    (list.x + 40.0 + index as f32 * 12.0, row_y + 7.0),
                    13.0,
                    shape,
                    if row == 5 {
                        Color::WHITE
                    } else {
                        Color::hex(0xC8CCD8)
                    },
                );
            }
            scene.line(
                (list.x + 12.0, row_y + 14.0),
                (list.x + 28.0, row_y + 14.0),
                2.0,
                Color::hex(0x8890A8),
            );
        }
        scene.fill(
            Rect::new(list.x + list.width - 10.0, list.y + 40.0, 6.0, 120.0),
            3.0,
            Color::hex(0x6A7090),
        );
        scene.pop_clip();

        for index in 0..4 {
            let center = (
                panel.x + 60.0 + index as f32 * 70.0,
                panel.y + panel.height - 46.0,
            );
            scene.arc(center, 18.0, 4.0, (0.0, TAU), Color::hex(0x30354D));
            scene.arc(
                center,
                18.0,
                4.0,
                (-PI * 0.5, TAU * (index as f32 + 1.0) / 5.0),
                Color::hex(0x7FA0FF),
            );
        }
        scene.line(
            (panel.x + 24.0, panel.y + 716.0),
            (panel.x + panel.width - 24.0, panel.y + 716.0),
            1.0,
            Color::hex(0x4A4F66),
        );
        for button in 0..2 {
            let button_rect = Rect::new(
                panel.x + panel.width - 360.0 + button as f32 * 176.0,
                panel.y + panel.height - 68.0,
                160.0,
                44.0,
            );
            scene.shadow(
                button_rect,
                8.0,
                4.0,
                (0.0, 2.0),
                Color::BLACK.with_alpha(0.4),
            );
            scene.rect(
                button_rect,
                &RectStyle::filled(Color::hex(0x5870D0), 8.0).with_gradient(Color::hex(0x4058B0)),
            );
        }
        scene.line(
            (panel.x + 40.0, panel.y + 300.0),
            (panel.x + 400.0, panel.y + 360.0),
            3.0,
            Color::hex(0xFF8040).with_alpha(0.8),
        );
        (scene, store)
    }

    #[test]
    fn an_axis_aligned_rectangle_covers_exactly_its_pixels() {
        let mut scene = empty_scene();
        scene.fill(Rect::new(10.0, 20.0, 30.0, 15.0), 0.0, Color::WHITE);
        let output = render(&scene, &SlugData::default(), 64, 64);
        for (index, pixel) in output.as_chunks::<4>().0.iter().enumerate() {
            let x = index % 64;
            let y = index / 64;
            let inside = (10..40).contains(&x) && (20..35).contains(&y);
            let expected = if inside { [255; 4] } else { [0; 4] };
            assert_eq!(*pixel, expected, "pixel {x}, {y}");
        }
    }

    #[test]
    fn shapes_cover_their_analytic_area() {
        let mut scene = empty_scene();
        scene.fill(Rect::new(5.0, 5.0, 50.0, 30.0), 8.0, Color::WHITE);
        let area = covered_area(&render(&scene, &SlugData::default(), 64, 64));
        assert_close(area, 1500.0 - (4.0 - PI) * 64.0, 0.002, "rounded rectangle");

        let mut scene = empty_scene();
        scene.arc((32.3, 32.7), 20.0, 6.0, (0.4, TAU), Color::WHITE);
        let area = covered_area(&render(&scene, &SlugData::default(), 64, 64));
        assert_close(area, TAU * 20.0 * 6.0, 0.002, "ring");

        let mut scene = empty_scene();
        scene.arc((32.3, 32.7), 20.0, 6.0, (0.4, PI), Color::WHITE);
        let area = covered_area(&render(&scene, &SlugData::default(), 64, 64));
        assert_close(area, PI * 20.0 * 6.0 + PI * 9.0, 0.005, "half ring");

        let mut scene = empty_scene();
        scene.line((10.2, 10.7), (50.6, 40.1), 5.0, Color::WHITE);
        let area = covered_area(&render(&scene, &SlugData::default(), 64, 64));
        let length = (40.4f32 * 40.4 + 29.4 * 29.4).sqrt();
        assert_close(area, length * 5.0 + PI * 6.25, 0.005, "line");
    }

    #[test]
    fn slug_paths_cover_their_analytic_area_in_both_windings() {
        let square = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let triangle = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        for (points, expected) in [(&square[..], 1600.0), (&triangle[..], 800.0)] {
            let reversed: Vec<[f32; 2]> = points.iter().rev().copied().collect();
            let mut outputs = Vec::new();
            for outline in [points, &reversed[..]] {
                let mut store = ShapeStore::new();
                let shape = store.add_path(&Path::new().polygon(outline)).unwrap();
                let mut scene = empty_scene();
                scene.path((10.25, 12.5), 40.0, shape, Color::WHITE);
                let output = render(&scene, store.slug(), 64, 64);
                assert_close(covered_area(&output), expected, 0.002, "path");
                outputs.push(output);
            }
            for (first, second) in outputs[0].iter().zip(&outputs[1]) {
                assert!(first.abs_diff(*second) <= 1);
            }
        }
    }

    #[test]
    fn the_clip_limits_every_kind() {
        let mut store = ShapeStore::new();
        let triangle = store
            .add_path(&Path::new().polygon(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]))
            .unwrap();
        let clip = Rect::new(16.0, 16.0, 32.0, 32.0);
        let draws: [&dyn Fn(&mut Scene); 5] = [
            &|scene| scene.fill(Rect::new(4.0, 4.0, 56.0, 56.0), 6.0, Color::WHITE),
            &|scene| {
                scene.shadow(
                    Rect::new(10.0, 10.0, 44.0, 44.0),
                    6.0,
                    4.0,
                    (0.0, 0.0),
                    Color::WHITE,
                )
            },
            &|scene| scene.arc((32.0, 32.0), 22.0, 8.0, (0.0, TAU), Color::WHITE),
            &|scene| scene.line((2.0, 2.0), (62.0, 62.0), 6.0, Color::WHITE),
            &|scene| scene.path((2.0, 2.0), 60.0, triangle, Color::WHITE),
        ];
        for (kind, draw) in draws.iter().enumerate() {
            let mut scene = empty_scene();
            scene.push_clip(clip);
            draw(&mut scene);
            scene.pop_clip();
            let output = render(&scene, store.slug(), 64, 64);
            let mut inside_area = 0.0;
            for (index, pixel) in output.as_chunks::<4>().0.iter().enumerate() {
                let x = (index % 64) as f32 + 0.5;
                let y = (index / 64) as f32 + 0.5;
                if clip.contains(x, y) {
                    inside_area += f32::from(pixel[3]);
                } else {
                    assert_eq!(*pixel, [0; 4], "kind {kind} draws outside the clip");
                }
            }
            assert!(inside_area > 0.0, "kind {kind} draws nothing");
        }
    }

    #[test]
    fn the_output_is_premultiplied_srgb() {
        let mut scene = empty_scene();
        scene.fill(Rect::new(0.0, 0.0, 2.0, 2.0), 0.0, Color::hex(0x336699));
        scene.fill(
            Rect::new(2.0, 0.0, 2.0, 2.0),
            0.0,
            Color::hex(0x336699).with_alpha(0.5),
        );
        let output = render(&scene, &SlugData::default(), 4, 2);
        assert_eq!(output[0..4], [0x33, 0x66, 0x99, 0xFF]);
        let alpha = 128.0 / 255.0;
        let premultiplied = |channel: u8| (f32::from(channel) * alpha).round() as u8;
        assert_eq!(
            output[8..12],
            [
                premultiplied(0x33),
                premultiplied(0x66),
                premultiplied(0x99),
                128
            ]
        );
        assert_eq!(output[4..8], output[0..4]);
        assert_eq!(output[12..16], output[8..12]);
    }

    #[test]
    fn the_clear_color_fills_the_background() {
        let mut scene = Scene::new();
        scene.begin(1.0, Color::hex(0x204060));
        let output = render(&scene, &SlugData::default(), 3, 40);
        for pixel in output.as_chunks::<4>().0.iter() {
            assert_eq!(*pixel, [0x20, 0x40, 0x60, 0xFF]);
        }
    }

    #[test]
    fn parallel_and_single_threaded_output_is_identical() {
        let (scene, store) = sample_scene(640, 400);
        let mut single = vec![0; 640 * 400 * 4];
        let mut parallel = vec![0; 640 * 400 * 4];
        assert!(Rasterizer::with_thread_count(1).rasterize(
            &scene,
            store.slug(),
            640,
            400,
            &mut single
        ));
        assert!(Rasterizer::with_thread_count(7).rasterize(
            &scene,
            store.slug(),
            640,
            400,
            &mut parallel
        ));
        assert!(single.iter().any(|&byte| byte != 0));
        assert_eq!(single, parallel);
    }

    /// A small deterministic pseudo random number generator.
    struct Random(u64);

    impl Random {
        /// The next value from 0 to 1.
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }

        /// The next value from 0 to 255.
        fn byte(&mut self) -> u8 {
            (self.next() * 255.0) as u8
        }
    }

    /// Rasterizes `scene` pixel by pixel without strips, occlusion or the
    /// encoding cache.
    fn render_naive(scene: &Scene, slug: &SlugData, width: u32, height: u32) -> Vec<u8> {
        let (width, height) = (width as usize, height as usize);
        let mut colors = vec![scene.clear_color().to_linear(); width * height];
        for instance in scene.instances() {
            let area = PixelArea::of(instance, width, height);
            for y in area.top..area.bottom {
                for x in area.left..area.right {
                    let source = shade(instance, slug, [x as f32 + 0.5, y as f32 + 0.5]);
                    let destination = &mut colors[y * width + x];
                    let remaining = 1.0 - source[3];
                    for channel in 0..4 {
                        destination[channel] = source[channel] + destination[channel] * remaining;
                    }
                }
            }
        }
        let curves = curves();
        colors
            .iter()
            .flat_map(|&color| curves.encode_pixel(color))
            .collect()
    }

    /// Asserts that the rasterizer and the naive renderer produce the same
    /// bytes for `scene`.
    fn assert_matches_naive(scene: &Scene, slug: &SlugData, width: u32, height: u32, what: &str) {
        let mut output = vec![0; width as usize * height as usize * 4];
        Rasterizer::with_thread_count(3).rasterize(scene, slug, width, height, &mut output);
        let expected = render_naive(scene, slug, width, height);
        let pixels = output.as_chunks::<4>().0.iter();
        let expected_pixels = expected.as_chunks::<4>().0.iter();
        if let Some((index, (actual, wanted))) = pixels
            .zip(expected_pixels)
            .enumerate()
            .find(|(_, (actual, wanted))| actual != wanted)
        {
            panic!(
                "{what}: pixel {}, {} is {actual:?} in place of {wanted:?}",
                index % width as usize,
                index / width as usize
            );
        }
    }

    #[test]
    fn random_scenes_match_the_naive_renderer() {
        let mut store = ShapeStore::new();
        let shapes = sample_shapes(&mut store);
        for seed in 1..200u64 {
            let mut random = Random(seed * 7919 + 1);
            let mut scene = Scene::new();
            scene.begin(
                0.5 + random.next() * 2.0,
                Color::hex(0x102030).with_alpha(random.next()),
            );
            for _ in 0..30 {
                let pick = random.next();
                let rect = Rect::new(
                    random.next() * 120.0 - 20.0,
                    random.next() * 90.0 - 20.0,
                    random.next() * 80.0,
                    random.next() * 60.0,
                );
                let alpha = if random.next() < 0.6 {
                    255
                } else {
                    random.byte()
                };
                let color = Color::rgba(random.byte(), random.byte(), random.byte(), alpha);
                let clipped = random.next() < 0.3;
                if clipped {
                    scene.push_clip(Rect::new(
                        random.next() * 60.0,
                        random.next() * 40.0,
                        random.next() * 80.0,
                        random.next() * 60.0,
                    ));
                }
                if pick < 0.5 {
                    let mut style = RectStyle {
                        radii: [0; 4].map(|_| random.next() * 20.0),
                        fill: color,
                        ..RectStyle::default()
                    };
                    if random.next() < 0.4 {
                        let bottom_alpha = if random.next() < 0.7 { 255 } else { 128 };
                        style = style.with_gradient(Color::rgba(10, 200, 30, bottom_alpha));
                    }
                    if random.next() < 0.4 {
                        let stroke = Color::rgba(200, 10, 30, random.byte());
                        style = style.with_stroke(random.next() * 6.0, stroke);
                    }
                    scene.rect(rect, &style);
                } else if pick < 0.65 {
                    let blur = random.next() * 8.0;
                    scene.shadow(rect, random.next() * 10.0, blur, (2.0, 3.0), color);
                } else if pick < 0.8 {
                    let angles = (random.next() * 6.0, random.next() * 7.0 - 3.5);
                    let (radius, width) = (random.next() * 30.0, random.next() * 8.0);
                    scene.arc((rect.x, rect.y), radius, width, angles, color);
                } else if pick < 0.9 {
                    let to = (rect.x + rect.width, rect.y + rect.height);
                    scene.line((rect.x, rect.y), to, random.next() * 5.0, color);
                } else {
                    let shape =
                        shapes[(random.next() * shapes.len() as f32) as usize % shapes.len()];
                    scene.path((rect.x, rect.y), random.next() * 40.0, shape, color);
                }
                if clipped {
                    scene.pop_clip();
                }
            }
            assert_matches_naive(&scene, store.slug(), 97, 71, &format!("seed {seed}"));
        }
    }

    #[test]
    fn large_coordinates_match_the_naive_renderer() {
        for magnitude in [1.0e5f32, 1.0e6, 4.0e6, 1.0e7, 3.0e7, 1.0e8] {
            for offset in [0.0f32, 0.3, 0.7, 13.0] {
                for (scale, radius, stroke, extent) in [
                    (1.0, 4.0, 2.0, [40.0, 30.0]),
                    (1.37, 0.0, 0.0, [30.0, 20.0]),
                    (1.0, 0.0, 0.0, [40.0, 30.0]),
                    (1.37, 4.0, 2.0, [30.0, 20.0]),
                ] {
                    let mut scene = Scene::new();
                    scene.begin(scale, Color::TRANSPARENT);
                    scene.fill(Rect::new(0.0, 0.0, 64.0, 64.0), 0.0, Color::hex(0xFF0000));
                    let rect = Rect::new(
                        -magnitude,
                        -magnitude,
                        magnitude + extent[0] + offset,
                        magnitude + extent[1] + offset,
                    );
                    let style = RectStyle::filled(Color::WHITE, radius)
                        .with_stroke(stroke, Color::hex(0x0000FF));
                    scene.rect(rect, &style);
                    let what = format!("magnitude {magnitude}, offset {offset}, scale {scale}");
                    assert_matches_naive(&scene, &SlugData::default(), 64, 64, &what);
                }
            }
        }
    }

    /// A rectangle instance with the edges `rect` as left, top, width and
    /// height in physical pixels. The edges do not snap to whole pixels.
    fn rectangle_instance(rect: [f32; 4], radii: [f32; 4], style: &RectStyle) -> Instance {
        let fill = style.fill.to_linear();
        let (gradient, bottom) = match style.fill_bottom {
            Some(color) => (1.0, color.to_linear()),
            None => (0.0, fill),
        };
        Instance {
            rect,
            color: fill,
            second_color: style.stroke.to_linear(),
            radii,
            parameters: [style.stroke_width, gradient, 0.0, 0.0],
            second_parameters: bottom,
            kind: [Instance::KIND_RECTANGLE, 0, 0, 0],
            ..Instance::default()
        }
    }

    /// A random color whose alpha is opaque in about half the cases.
    fn random_color(random: &mut Random) -> Color {
        let alpha = if random.next() < 0.5 {
            255
        } else {
            random.byte()
        };
        Color::rgba(random.byte(), random.byte(), random.byte(), alpha)
    }

    #[test]
    fn fractional_rectangles_match_the_naive_renderer() {
        let stroke_widths = [0.0, 0.5, 1.0, 2.75, 7.3, 60.0];
        let mut random = Random(4711);
        for case in 0..80 {
            let mut scene = Scene::new();
            scene.begin(1.0, Color::hex(0x203040).with_alpha(random.next()));
            for _ in 0..6 {
                let rect = [
                    random.next() * 180.0 - 20.0,
                    random.next() * 90.0 - 10.0,
                    1.0 + random.next() * 170.0,
                    1.0 + random.next() * 80.0,
                ];
                let radii = if random.next() < 0.5 {
                    [random.next() * 40.0; 4]
                } else {
                    [0; 4].map(|_| random.next() * 30.0)
                };
                let mut style = RectStyle {
                    fill: random_color(&mut random),
                    stroke_width: stroke_widths[case % stroke_widths.len()] * random.next(),
                    stroke: random_color(&mut random),
                    ..RectStyle::default()
                };
                if random.next() < 0.4 {
                    style = style.with_gradient(random_color(&mut random));
                }
                let clipped = random.next() < 0.3;
                if clipped {
                    scene.push_clip(Rect::new(
                        random.next() * 100.0,
                        random.next() * 50.0,
                        random.next() * 120.0,
                        random.next() * 60.0,
                    ));
                }
                scene.push_instance(rectangle_instance(rect, radii, &style), 1.0);
                if clipped {
                    scene.pop_clip();
                }
            }
            let what = format!("case {case}");
            assert_matches_naive(&scene, &SlugData::default(), 211, 103, &what);
        }
    }

    #[test]
    fn gradients_wider_than_the_noise_tile_match_the_naive_renderer() {
        let cases = [
            (0.0, 300.0, 0.0, 0.0, 255, 255),
            (37.4, 257.3, 12.0, 2.5, 255, 255),
            (-61.7, 470.0, 30.0, 1.0, 255, 128),
            (5.5, 140.25, 4.0, 0.0, 200, 0),
            (130.0, 129.0, 0.0, 3.0, 0, 255),
            (-3.0, 400.0, 90.0, 0.0, 255, 255),
        ];
        for (index, (left, width, radius, stroke, top_alpha, bottom_alpha)) in
            cases.into_iter().enumerate()
        {
            let mut scene = empty_scene();
            scene.fill(
                Rect::new(0.0, 0.0, 384.0, 96.0),
                0.0,
                Color::hex(0x808080).with_alpha(0.5),
            );
            let style = RectStyle {
                fill: Color::rgba(0x20, 0x40, 0x60, top_alpha),
                stroke_width: stroke,
                stroke: Color::hex(0xFFC040),
                ..RectStyle::default()
            }
            .with_gradient(Color::rgba(0x30, 0x58, 0x78, bottom_alpha));
            let rect = [left, 3.25, width, 89.5];
            scene.push_instance(rectangle_instance(rect, [radius; 4], &style), 1.0);
            scene.rect(
                Rect::new(left, 10.0, width, 70.0),
                &RectStyle::filled(Color::hex(0x102030), radius)
                    .with_gradient(Color::hex(0xF0E0D0)),
            );
            let what = format!("gradient {index}");
            assert_matches_naive(&scene, &SlugData::default(), 384, 96, &what);
        }
    }

    #[test]
    fn shadows_match_the_naive_renderer() {
        let cases = [
            (Rect::new(20.0, 15.0, 120.0, 60.0), 8.0, 6.0, (0.0, 4.0)),
            (Rect::new(-40.3, 7.7, 300.0, 40.1), 30.0, 0.5, (1.5, -2.25)),
            (Rect::new(60.5, 30.25, 20.0, 10.0), 50.0, 12.0, (0.0, 0.0)),
            (
                Rect::new(-100.0, -100.0, 500.0, 400.0),
                0.0,
                24.0,
                (3.0, 8.0),
            ),
            (Rect::new(10.0, 10.0, 1.0, 1.0), 2.0, 3.0, (0.0, 0.0)),
        ];
        for (index, (rect, radius, blur, offset)) in cases.into_iter().enumerate() {
            for (scale, clipped) in [(1.0, false), (1.0, true), (1.37, false), (0.83, true)] {
                let mut scene = Scene::new();
                scene.begin(scale, Color::TRANSPARENT);
                scene.fill(Rect::new(0.0, 0.0, 200.0, 120.0), 0.0, Color::hex(0xE0E0F0));
                if clipped {
                    scene.push_clip(Rect::new(30.0, 20.0, 110.0, 70.0));
                }
                scene.shadow(rect, radius, blur, offset, Color::BLACK.with_alpha(0.6));
                if clipped {
                    scene.pop_clip();
                }
                let what = format!("shadow {index}, scale {scale}, clipped {clipped}");
                assert_matches_naive(&scene, &SlugData::default(), 200, 120, &what);
            }
        }
        let mut random = Random(2718);
        for case in 0..60 {
            let mut scene = Scene::new();
            scene.begin(0.6 + random.next() * 1.5, Color::hex(0xE0E0F0));
            for _ in 0..4 {
                let rect = Rect::new(
                    random.next() * 150.0 - 20.0,
                    random.next() * 90.0 - 20.0,
                    random.next() * 120.0,
                    random.next() * 70.0,
                );
                let offset = (random.next() * 6.0 - 3.0, random.next() * 6.0 - 3.0);
                let blur = random.next() * 16.0;
                scene.shadow(
                    rect,
                    random.next() * 20.0,
                    blur,
                    offset,
                    random_color(&mut random),
                );
            }
            let what = format!("random shadows {case}");
            assert_matches_naive(&scene, &SlugData::default(), 200, 120, &what);
        }
    }

    #[test]
    fn arcs_through_both_branches_match_the_naive_renderer() {
        let sweeps = [-8.0, -TAU, -3.5, -1.2, 0.0, 0.3, 2.0, PI, 5.9, TAU, 9.0];
        let starts = [-2.0, 0.0, 0.7, PI, 5.0];
        for sweep in sweeps {
            let mut scene = empty_scene();
            for (column, &start) in starts.iter().enumerate() {
                let center = (20.3 + column as f32 * 38.0, 30.6);
                scene.arc(center, 14.25, 5.5, (start, sweep), Color::WHITE);
                let center = (20.3 + column as f32 * 38.0, 70.1);
                let color = Color::hex(0x7FA0FF).with_alpha(0.7);
                scene.arc(center, 4.0, 9.5, (start, sweep), color);
            }
            let what = format!("sweep {sweep}");
            assert_matches_naive(&scene, &SlugData::default(), 197, 96, &what);
        }
    }

    #[test]
    fn dark_and_light_text_matches_the_naive_renderer() {
        let mut store = ShapeStore::new();
        let shapes = sample_shapes(&mut store);
        let colors = [
            Color::BLACK,
            Color::hex(0x202020),
            Color::hex(0x606878),
            Color::hex(0x7A7A80),
            Color::hex(0x8890A0),
            Color::hex(0xC8CCD8),
            Color::WHITE,
            Color::hex(0x000000).with_alpha(0.5),
            Color::hex(0xFFFFFF).with_alpha(0.3),
            Color::hex(0x8890A0).with_alpha(0.6),
            Color::TRANSPARENT,
        ];
        for clear in [Color::WHITE, Color::hex(0x101018)] {
            let mut scene = Scene::new();
            scene.begin(1.0, clear);
            for (row, &color) in colors.iter().enumerate() {
                for (column, &shape) in shapes.iter().enumerate() {
                    let origin = (3.3 + column as f32 * 21.7, 2.6 + row as f32 * 15.2);
                    scene.path(origin, 13.0 + column as f32 * 1.5, shape, color);
                }
            }
            scene.push_instance(
                Instance {
                    rect: [10.0, 10.0, 20.0, 20.0],
                    color: [1.0; 4],
                    parameters: [0.0, 1.0, 0.05, -0.05],
                    kind: [Instance::KIND_GLYPH, 999, 0, 0],
                    ..Instance::default()
                },
                1.0,
            );
            let what = format!("text over {clear:?}");
            assert_matches_naive(&scene, store.slug(), 140, 170, &what);
        }
    }

    /// The largest difference between two linear values that the 8 bit
    /// outputs of the rasterizer allow.
    const QUANTIZATION_TOLERANCE: f32 = 0.01;

    /// The linear value of the 8 bit sRGB output channel `channel`.
    fn linear_output(channel: u8) -> f32 {
        crate::srgb_to_linear(f32::from(channel) / 255.0)
    }

    /// Renders a triangle path in `color` over a target cleared to `clear`.
    fn triangle_render(color: Color, clear: Color) -> Vec<u8> {
        let mut store = ShapeStore::new();
        let triangle = store
            .add_path(&Path::new().polygon(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]))
            .unwrap();
        let mut scene = Scene::new();
        scene.begin(1.0, clear);
        scene.path((10.25, 12.5), 40.0, triangle, color);
        render(&scene, store.slug(), 64, 64)
    }

    /// The linear coverage of each pixel of the triangle path.
    fn triangle_coverage() -> Vec<f32> {
        triangle_render(Color::WHITE, Color::TRANSPARENT)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| f32::from(pixel[3]) / 255.0)
            .collect()
    }

    /// Verifies that a black path on white covers every pixel at least as much
    /// as the linear blend and the edges more.
    #[test]
    fn dark_text_on_a_light_fill_covers_more_than_the_linear_blend() {
        let coverage = triangle_coverage();
        let output = triangle_render(Color::BLACK, Color::WHITE);
        let mut darkness_area = 0.0;
        for (pixel, &expected) in output.as_chunks::<4>().0.iter().zip(&coverage) {
            let darkness = 1.0 - linear_output(pixel[0]);
            assert!(darkness >= expected - QUANTIZATION_TOLERANCE);
            darkness_area += darkness;
        }
        let linear_area: f32 = coverage.iter().sum();
        assert!(
            darkness_area > linear_area * 1.02,
            "{darkness_area} against {linear_area}"
        );
    }

    /// Verifies that a white path on black keeps the linear coverage.
    #[test]
    fn light_text_on_a_dark_fill_keeps_the_linear_coverage() {
        let coverage = triangle_coverage();
        let output = triangle_render(Color::WHITE, Color::BLACK);
        for (pixel, &expected) in output.as_chunks::<4>().0.iter().zip(&coverage) {
            let brightness = linear_output(pixel[0]);
            assert!((brightness - expected).abs() <= QUANTIZATION_TOLERANCE);
        }
    }

    #[test]
    fn occluded_runs_match_the_naive_renderer() {
        let mut store = ShapeStore::new();
        let shapes = sample_shapes(&mut store);
        for seed in 1..40u64 {
            let mut random = Random(seed * 104_729 + 7);
            let mut scene = empty_scene();
            scene.fill(
                Rect::new(0.0, 0.0, 160.0, 90.0),
                0.0,
                Color::hex(0x101018).with_alpha(0.6),
            );
            scene.shadow(
                Rect::new(20.0, 10.0, 120.0, 70.0),
                8.0,
                6.0,
                (0.0, 3.0),
                Color::BLACK.with_alpha(0.5),
            );
            for index in 0..24 {
                let shape = shapes[index % shapes.len()];
                let origin = (random.next() * 150.0, random.next() * 80.0);
                scene.path(
                    origin,
                    9.0 + random.next() * 10.0,
                    shape,
                    Color::hex(0xF0F0F8),
                );
            }
            for _ in 0..5 {
                let rect = [
                    random.next() * 150.0 - 10.0,
                    random.next() * 80.0 - 10.0,
                    5.0 + random.next() * 60.0,
                    5.0 + random.next() * 40.0,
                ];
                let mut style = RectStyle::filled(
                    Color::rgba(random.byte(), random.byte(), random.byte(), 255),
                    random.next() * 12.0,
                );
                if random.next() < 0.5 {
                    style = style.with_stroke(
                        random.next() * 4.0,
                        Color::hex(0xFF0000).with_alpha(random.next()),
                    );
                }
                if random.next() < 0.3 {
                    style = style.with_gradient(Color::hex(0x204060));
                }
                scene.push_instance(rectangle_instance(rect, style.radii, &style), 1.0);
                let origin = (rect[0] + random.next() * rect[2], rect[1]);
                scene.line(
                    origin,
                    (origin.0 + 20.0, origin.1 + 30.0),
                    2.5,
                    random_color(&mut random),
                );
                scene.arc(
                    (rect[0], rect[1]),
                    6.0,
                    3.0,
                    (0.0, 4.0),
                    random_color(&mut random),
                );
            }
            let what = format!("seed {seed}");
            assert_matches_naive(&scene, store.slug(), 160, 90, &what);
        }
    }

    #[test]
    fn negative_corner_radii_match_the_naive_renderer() {
        for radius in [-1.0e8, -3.0e7, -1.0e6, -40.0, -5.0, -0.25] {
            for alpha in [255, 128] {
                let mut scene = empty_scene();
                let style = RectStyle::filled(Color::rgba(200, 100, 50, alpha), 0.0)
                    .with_stroke(1.5, Color::hex(0x20C040));
                let rect = [10.25, 5.5, 80.0, 40.0];
                scene.push_instance(
                    rectangle_instance(rect, [radius, 0.0, 3.0, radius * 0.5], &style),
                    1.0,
                );
                let what = format!("radius {radius}, alpha {alpha}");
                assert_matches_naive(&scene, &SlugData::default(), 120, 60, &what);
            }
        }
    }

    #[test]
    fn the_occluder_search_finds_the_first_match() {
        let mut random = Random(99);
        for length in [0, 1, 15, 16, 17, 31, 32, 33, 100] {
            for _ in 0..50 {
                let occluders: Vec<u32> =
                    (0..length).map(|_| (random.next() * 4.0) as u32).collect();
                let order = (random.next() * 4.0) as u32;
                let visible = |occluder: u32| occluder <= order;
                let hidden = |occluder: u32| occluder > order;
                assert_eq!(
                    find_occluder(&occluders, visible),
                    occluders.iter().position(|&occluder| visible(occluder))
                );
                assert_eq!(
                    find_occluder(&occluders, hidden),
                    occluders.iter().position(|&occluder| hidden(occluder))
                );
            }
        }
    }

    #[test]
    fn edges_that_are_not_finite_evaluate_no_pixel() {
        let instance = Instance {
            rect: [f32::NAN, 10.0, 5.0, 5.0],
            clip: [0.0, 0.0, f32::MAX, f32::MAX],
            parameters: [3.0, 0.0, 0.0, 0.0],
            kind: [Instance::KIND_SHADOW, 0, 0, 0],
            ..Instance::default()
        };
        assert!(PixelArea::of(&instance, 64, 64).is_empty());
        let instance = Instance {
            rect: [10.0, 10.0, 5.0, 5.0],
            parameters: [f32::INFINITY, 0.0, 0.0, 0.0],
            ..instance
        };
        assert!(PixelArea::of(&instance, 64, 64).is_empty());
    }

    /// The largest distance in 8 bit steps between a dithered channel and its
    /// exact value: half a step of rounding, the noise of up to half a step
    /// and its bias of half a step over 255, and the error of the tables.
    const DITHER_TOLERANCE: f32 = 1.02;

    #[test]
    fn a_gradient_dithers_within_one_step_of_its_blend() {
        let (top, bottom) = (Color::hex(0x204060), Color::hex(0x305878));
        let mut scene = empty_scene();
        scene.rect(
            Rect::new(0.0, 0.0, 128.0, 64.0),
            &RectStyle::filled(top, 0.0).with_gradient(bottom),
        );
        let output = render(&scene, &SlugData::default(), 128, 64);
        let (top, bottom) = (top.to_linear(), bottom.to_linear());
        for (y, row) in output.as_chunks::<{ 128 * 4 }>().0.iter().enumerate() {
            let t = (y as f32 + 0.5) / 64.0;
            for channel in 0..3 {
                let blend = top[channel] + (bottom[channel] - top[channel]) * t;
                let expected = crate::linear_to_srgb(blend) * 255.0;
                let mut sum = 0.0;
                for pixel in row.as_chunks::<4>().0 {
                    let value = f32::from(pixel[channel]);
                    assert!(
                        (value - expected).abs() <= DITHER_TOLERANCE,
                        "row {y}: {value} for {expected}"
                    );
                    sum += value;
                }
                let mean = sum / 128.0;
                assert!(
                    (mean - expected).abs() < 0.15,
                    "row {y}: mean {mean} for {expected}"
                );
            }
        }
    }

    #[test]
    fn replaced_slug_data_of_equal_lengths_draws_again() {
        let mut first = ShapeStore::new();
        let shape = first
            .add_path(&Path::new().polygon(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]))
            .unwrap();
        let mut second = ShapeStore::new();
        second
            .add_path(&Path::new().polygon(&[[0.0, 0.0], [0.5, 0.0], [0.5, 1.0], [0.0, 1.0]]))
            .unwrap();
        let lengths = |store: &ShapeStore| {
            let slug = store.slug();
            [slug.curves().len(), slug.bands().len(), slug.glyphs().len()]
        };
        assert_eq!(lengths(&first), lengths(&second));
        let mut scene = empty_scene();
        scene.path((4.0, 4.0), 40.0, shape, Color::WHITE);
        let mut rasterizer = Rasterizer::new();
        let mut output = vec![0; 64 * 64 * 4];
        assert!(rasterizer.rasterize(&scene, first.slug(), 64, 64, &mut output));
        let first_output = output.clone();
        assert!(rasterizer.rasterize(&scene, second.slug(), 64, 64, &mut output));
        assert_ne!(first_output, output);
        assert!(!rasterizer.rasterize(&scene, second.slug(), 64, 64, &mut output));
    }

    #[test]
    fn an_unchanged_frame_is_skipped() {
        let (mut scene, mut store) = sample_scene(64, 48);
        let mut rasterizer = Rasterizer::with_thread_count(2);
        let mut output = vec![0; 64 * 48 * 4];
        assert!(rasterizer.rasterize(&scene, store.slug(), 64, 48, &mut output));
        assert!(!rasterizer.rasterize(&scene, store.slug(), 64, 48, &mut output));

        let mut larger = vec![0; 64 * 49 * 4];
        assert!(rasterizer.rasterize(&scene, store.slug(), 64, 49, &mut larger));
        assert!(!rasterizer.rasterize(&scene, store.slug(), 64, 49, &mut larger));

        store.add_path(&star(6)).unwrap();
        assert!(rasterizer.rasterize(&scene, store.slug(), 64, 49, &mut larger));
        assert!(!rasterizer.rasterize(&scene, store.slug(), 64, 49, &mut larger));

        scene.fill(Rect::new(0.0, 0.0, 200.0, 200.0), 0.0, Color::WHITE);
        assert!(rasterizer.rasterize(&scene, store.slug(), 64, 49, &mut larger));
        assert!(!rasterizer.rasterize(&scene, store.slug(), 64, 49, &mut larger));

        rasterizer.invalidate();
        assert!(rasterizer.rasterize(&scene, store.slug(), 64, 49, &mut larger));
    }

    #[test]
    fn an_empty_target_draws_nothing() {
        let (scene, store) = sample_scene(64, 48);
        let mut rasterizer = Rasterizer::new();
        assert!(rasterizer.rasterize(&scene, store.slug(), 0, 0, &mut []));
        assert!(!rasterizer.rasterize(&scene, store.slug(), 0, 0, &mut []));
    }

    #[test]
    fn an_opaque_rectangle_hides_every_item_below_it() {
        let (mut scene, store) = sample_scene(320, 200);
        let rect = Rect::new(250.0, 180.0, 900.0, 700.0);
        let style = RectStyle::filled(Color::hex(0x406080), 40.0)
            .with_gradient(Color::hex(0x204060))
            .with_stroke(6.0, Color::hex(0xFF0000).with_alpha(0.5));
        scene.rect(rect, &style);
        let mut alone = empty_scene();
        alone.begin(scene.scale(), Color::TRANSPARENT);
        alone.rect(rect, &style);
        let covered = render(&scene, store.slug(), 320, 200);
        let expected = render(&alone, store.slug(), 320, 200);

        let instance = alone.instances()[0];
        let [x, y, width, height] = instance.rect;
        let inset = instance.radii[0] + instance.parameters[0] + OPAQUE_INSET;
        let inside = Rect::new(
            x + inset,
            y + inset,
            width - 2.0 * inset,
            height - 2.0 * inset,
        );
        let mut compared = 0;
        for (index, (first, second)) in covered
            .as_chunks::<4>()
            .0
            .iter()
            .zip(expected.as_chunks::<4>().0.iter())
            .enumerate()
        {
            let pixel_x = (index % 320) as f32 + 0.5;
            let pixel_y = (index / 320) as f32 + 0.5;
            if inside.contains(pixel_x, pixel_y) {
                assert_eq!(first, second, "pixel {pixel_x}, {pixel_y}");
                compared += 1;
            }
        }
        assert!(compared > 10_000);
    }

    #[test]
    #[ignore = "writes a screenshot into the directory of GUI_SHOTS"]
    fn sample_scene_screenshot() {
        let (scene, store) = sample_scene(1280, 720);
        let output = render(&scene, store.slug(), 1280, 720);
        write_ppm("rasterizer_sample", 1280, 720, &output);
    }

    #[test]
    #[ignore = "prints frame times, run with --release"]
    fn full_screen_frame_time() {
        const FRAMES: u32 = 20;
        for (width, height) in [(1920, 1080), (3840, 2160)] {
            let (scene, store) = sample_scene(width, height);
            let mut output = vec![0; width as usize * height as usize * 4];
            for thread_count in [
                1,
                thread::available_parallelism().map_or(1, NonZeroUsize::get),
            ] {
                let mut rasterizer = Rasterizer::with_thread_count(thread_count);
                rasterizer.rasterize(&scene, store.slug(), width, height, &mut output);
                let start = Instant::now();
                for _ in 0..FRAMES {
                    rasterizer.invalidate();
                    rasterizer.rasterize(&scene, store.slug(), width, height, &mut output);
                }
                let frame_time = start.elapsed() / FRAMES;
                println!(
                    "{width} x {height}, {} instances, {thread_count} threads: {:.2} ms",
                    scene.len(),
                    frame_time.as_secs_f64() * 1000.0
                );
            }
        }
    }

    /// Prints the mean and the best frame time of `scene` on one thread and on
    /// every hardware thread.
    fn print_frame_times(name: &str, scene: &Scene, slug: &SlugData, width: u32, height: u32) {
        /// The number of timed frames for each thread count.
        const FRAMES: u32 = 20;
        let mut output = vec![0; width as usize * height as usize * 4];
        for thread_count in [
            1,
            thread::available_parallelism().map_or(1, NonZeroUsize::get),
        ] {
            let mut rasterizer = Rasterizer::with_thread_count(thread_count);
            rasterizer.rasterize(scene, slug, width, height, &mut output);
            let mut best = f64::MAX;
            let mut total = 0.0;
            for _ in 0..FRAMES {
                rasterizer.invalidate();
                let start = Instant::now();
                rasterizer.rasterize(scene, slug, width, height, &mut output);
                let milliseconds = start.elapsed().as_secs_f64() * 1000.0;
                best = best.min(milliseconds);
                total += milliseconds;
            }
            println!(
                "{name}, {width} x {height}, {thread_count} threads: mean {:.2} ms, best {:.2} ms",
                total / f64::from(FRAMES),
                best
            );
        }
    }

    #[test]
    #[ignore = "prints frame times, run with --release"]
    fn single_instance_frame_time() {
        let (width, height) = (3840, 2160);
        let full_screen = Rect::new(0.0, 0.0, width as f32, height as f32);
        let slug = SlugData::default();

        let mut veil = empty_scene();
        veil.fill(full_screen, 0.0, Color::hex(0x101018).with_alpha(0.6));
        print_frame_times("veil", &veil, &slug, width, height);

        let mut opaque = empty_scene();
        opaque.fill(full_screen, 0.0, Color::hex(0x101018));
        print_frame_times("opaque rectangle", &opaque, &slug, width, height);

        let mut gradient = empty_scene();
        gradient.rect(
            full_screen,
            &RectStyle::filled(Color::hex(0x2A2D3A), 16.0).with_gradient(Color::hex(0x1E2029)),
        );
        print_frame_times("gradient rectangle", &gradient, &slug, width, height);

        let mut shadow = empty_scene();
        shadow.shadow(
            Rect::new(400.0, 300.0, 2400.0, 1600.0),
            32.0,
            48.0,
            (0.0, 16.0),
            Color::BLACK.with_alpha(0.6),
        );
        print_frame_times("shadow", &shadow, &slug, width, height);
    }
}
