//! The draw items of one frame.

use std::ops::Range;

use crate::{Bounds, Color, Rect, Run, SlugShape, bytes::as_bytes};

/// The record of one draw item for the rasterizer and the GPU shader.
///
/// All positions are in physical pixels and all colors are linear and
/// premultiplied. The meaning of `parameters` and `second_parameters` depends
/// on the kind in `kind[0]`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Instance {
    /// The item rectangle as left, top, width and height.
    pub rect: [f32; 4],
    /// The clip rectangle as left, top, right and bottom.
    pub clip: [f32; 4],
    /// The main color. The fill of a rectangle, at the top for a gradient.
    pub color: [f32; 4],
    /// The stroke color of a rectangle.
    pub second_color: [f32; 4],
    /// The corner radii top left, top right, bottom right and bottom left.
    pub radii: [f32; 4],
    /// Rectangle: stroke width and gradient flag. Shadow: sigma. Arc: center,
    /// radius and width. Line: start and end point. Glyph: the em space left
    /// and top edge, and the em units per pixel along x and y.
    pub parameters: [f32; 4],
    /// Rectangle: the gradient color at the bottom edge. Arc: start angle and
    /// sweep. Line: width.
    pub second_parameters: [f32; 4],
    /// The kind and, for a glyph, the index of the glyph in the glyph array.
    pub kind: [u32; 4],
}

impl Instance {
    /// A rectangle with rounded corners, a stroke and a gradient.
    pub const KIND_RECTANGLE: u32 = 0;
    /// The soft shadow of a rounded rectangle.
    pub const KIND_SHADOW: u32 = 1;
    /// A ring segment with round caps.
    pub const KIND_ARC: u32 = 2;
    /// A line with round caps.
    pub const KIND_LINE: u32 = 3;
    /// A Slug outline of a glyph or a path.
    pub const KIND_GLYPH: u32 = 4;

    /// The record as bytes for a GPU upload.
    pub fn as_bytes(&self) -> &[u8] {
        as_bytes(std::slice::from_ref(self))
    }
}

/// The look of a rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectStyle {
    /// The corner radii top left, top right, bottom right and bottom left.
    pub radii: [f32; 4],
    /// The fill color, at the top edge for a gradient.
    pub fill: Color,
    /// The fill color at the bottom edge of a vertical gradient.
    pub fill_bottom: Option<Color>,
    /// The width of the stroke inside the edge.
    pub stroke_width: f32,
    /// The stroke color.
    pub stroke: Color,
}

impl RectStyle {
    /// A fill with the same radius at every corner.
    pub fn filled(color: Color, radius: f32) -> Self {
        Self {
            radii: [radius; 4],
            fill: color,
            ..Self::default()
        }
    }

    /// The same style with a stroke.
    pub fn with_stroke(self, width: f32, color: Color) -> Self {
        Self {
            stroke_width: width,
            stroke: color,
            ..self
        }
    }

    /// The same style with a vertical gradient that ends in `bottom`.
    pub fn with_gradient(self, bottom: Color) -> Self {
        Self {
            fill_bottom: Some(bottom),
            ..self
        }
    }
}

/// The draw items of one frame in painter's order.
///
/// All methods take logical units. The scene multiplies them by the scale
/// factor and records physical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// The physical pixels for each logical unit.
    scale: f32,
    /// The color the frame starts with.
    clear: Color,
    /// The recorded draw items.
    instances: Vec<Instance>,
    /// The clip stack in physical pixels as left, top, right and bottom.
    clips: Vec<[f32; 4]>,
    /// The items of the overlay, which follow every other item.
    overlay: Vec<Instance>,
    /// The number of open overlays.
    overlay_depth: u32,
    /// The clip stacks outside the open overlays.
    outer_clips: Vec<Vec<[f32; 4]>>,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    /// An empty scene at scale 1.
    pub fn new() -> Self {
        Self {
            scale: 1.0,
            clear: Color::TRANSPARENT,
            instances: Vec::new(),
            clips: Vec::new(),
            overlay: Vec::new(),
            overlay_depth: 0,
            outer_clips: Vec::new(),
        }
    }

    /// Empties the scene for a new frame at `scale` physical pixels for each
    /// logical unit. The frame starts with the color `clear`.
    pub fn begin(&mut self, scale: f32, clear: Color) {
        self.scale = scale;
        self.clear = clear;
        self.instances.clear();
        self.clips.clear();
        self.overlay.clear();
        self.overlay_depth = 0;
        self.outer_clips.clear();
    }

    /// Starts the overlay: the items until [`Scene::end_overlay`] follow
    /// every other item of the frame once [`Scene::finish`] runs. The
    /// overlay starts without a clip.
    pub fn begin_overlay(&mut self) {
        self.outer_clips.push(std::mem::take(&mut self.clips));
        self.overlay_depth += 1;
    }

    /// Ends the overlay of the last [`Scene::begin_overlay`] and restores
    /// the clip stack outside it.
    pub fn end_overlay(&mut self) {
        if let Some(clips) = self.outer_clips.pop() {
            self.clips = clips;
            self.overlay_depth -= 1;
        }
    }

    /// Ends the frame: the items of the overlay follow the other items.
    pub fn finish(&mut self) {
        self.instances.append(&mut self.overlay);
    }

    /// The physical pixels for each logical unit.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The color the frame starts with.
    pub fn clear_color(&self) -> Color {
        self.clear
    }

    /// The number of draw items.
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    /// Whether the scene has no draw items.
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    /// The draw items in painter's order.
    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }

    /// The draw items as bytes for a GPU upload.
    pub fn instance_bytes(&self) -> &[u8] {
        as_bytes(&self.instances)
    }

    /// Limits the following items to `rect` inside the current clip. Clips
    /// snap to whole physical pixels.
    pub fn push_clip(&mut self, rect: Rect) {
        let [left, top, right, bottom] = self.snapped(rect);
        let current = self.clip();
        self.clips.push([
            left.max(current[0]),
            top.max(current[1]),
            right.min(current[2]),
            bottom.min(current[3]),
        ]);
    }

    /// Ends the clip of the last [`Scene::push_clip`].
    pub fn pop_clip(&mut self) {
        self.clips.pop();
    }

    /// The current clip rectangle in physical pixels.
    fn clip(&self) -> [f32; 4] {
        self.clips
            .last()
            .copied()
            .unwrap_or([0.0, 0.0, f32::MAX, f32::MAX])
    }

    /// The edges of `rect` in physical pixels, rounded to whole pixels.
    fn snapped(&self, rect: Rect) -> [f32; 4] {
        [
            (rect.x * self.scale).round(),
            (rect.y * self.scale).round(),
            ((rect.x + rect.width) * self.scale).round(),
            ((rect.y + rect.height) * self.scale).round(),
        ]
    }

    /// Records `instance` unless its geometry is not finite, its rectangle
    /// is empty or it lies outside the clip. `margin` is the distance in
    /// pixels the item reaches past its rectangle.
    fn push(&mut self, mut instance: Instance, margin: f32) {
        let finite = |values: [f32; 4]| values.iter().all(|value| value.is_finite());
        let clip = self.clip();
        let [x, y, width, height] = instance.rect;
        if !finite(instance.rect)
            || !finite(instance.radii)
            || !finite(instance.parameters)
            || !finite(instance.second_parameters)
            || width <= 0.0
            || height <= 0.0
            || x - margin >= clip[2]
            || y - margin >= clip[3]
            || x + width + margin <= clip[0]
            || y + height + margin <= clip[1]
            || clip[0] >= clip[2]
            || clip[1] >= clip[3]
        {
            return;
        }
        instance.clip = clip;
        if self.overlay_depth > 0 {
            self.overlay.push(instance);
        } else {
            self.instances.push(instance);
        }
    }

    /// Records `instance` without snapping its edges. `margin` is the
    /// distance in pixels the item reaches past its rectangle.
    #[cfg(test)]
    pub(crate) fn push_instance(&mut self, instance: Instance, margin: f32) {
        self.push(instance, margin);
    }

    /// A rectangle with rounded corners, a fill and a stroke. Its edges snap
    /// to whole physical pixels.
    pub fn rect(&mut self, rect: Rect, style: &RectStyle) {
        let [left, top, right, bottom] = self.snapped(rect);
        if right <= left || bottom <= top {
            return;
        }
        let fill = style.fill.to_linear();
        let (gradient, bottom_color) = match style.fill_bottom {
            Some(color) => (1.0, color.to_linear()),
            None => (0.0, fill),
        };
        let stroke_width = if style.stroke_width > 0.0 {
            (style.stroke_width * self.scale).round().max(1.0)
        } else {
            0.0
        };
        self.push(
            Instance {
                rect: [left, top, right - left, bottom - top],
                color: fill,
                second_color: style.stroke.to_linear(),
                radii: style.radii.map(|radius| radius * self.scale),
                parameters: [stroke_width, gradient, 0.0, 0.0],
                second_parameters: bottom_color,
                kind: [Instance::KIND_RECTANGLE, 0, 0, 0],
                ..Instance::default()
            },
            1.0,
        );
    }

    /// A filled rectangle with the same radius at every corner.
    pub fn fill(&mut self, rect: Rect, radius: f32, color: Color) {
        self.rect(rect, &RectStyle::filled(color, radius));
    }

    /// The soft shadow of a rectangle with rounded corners. `blur` is the
    /// standard deviation of the Gaussian blur. `offset` shifts the shadow.
    pub fn shadow(&mut self, rect: Rect, radius: f32, blur: f32, offset: (f32, f32), color: Color) {
        let scale = self.scale;
        let sigma = (blur * scale).max(0.5);
        self.push(
            Instance {
                rect: [
                    (rect.x + offset.0) * scale,
                    (rect.y + offset.1) * scale,
                    rect.width * scale,
                    rect.height * scale,
                ],
                color: color.to_linear(),
                radii: [radius * scale; 4],
                parameters: [sigma, 0.0, 0.0, 0.0],
                kind: [Instance::KIND_SHADOW, 0, 0, 0],
                ..Instance::default()
            },
            3.0 * sigma + 1.0,
        );
    }

    /// A ring segment with round caps around `center`. The arc starts at the
    /// angle `start` and sweeps clockwise on screen by `sweep`. Angles are in
    /// radians and start at the positive x-axis.
    pub fn arc(
        &mut self,
        center: (f32, f32),
        radius: f32,
        width: f32,
        (start, sweep): (f32, f32),
        color: Color,
    ) {
        let scale = self.scale;
        let center = (center.0 * scale, center.1 * scale);
        let radius = radius * scale;
        let width = width * scale;
        let reach = radius + width * 0.5;
        self.push(
            Instance {
                rect: [center.0 - reach, center.1 - reach, reach * 2.0, reach * 2.0],
                color: color.to_linear(),
                parameters: [center.0, center.1, radius, width],
                second_parameters: [start, sweep, 0.0, 0.0],
                kind: [Instance::KIND_ARC, 0, 0, 0],
                ..Instance::default()
            },
            1.0,
        );
    }

    /// A line from `from` to `to` with round caps.
    pub fn line(&mut self, from: (f32, f32), to: (f32, f32), width: f32, color: Color) {
        let scale = self.scale;
        let from = (from.0 * scale, from.1 * scale);
        let to = (to.0 * scale, to.1 * scale);
        let half = width * scale * 0.5;
        let left = from.0.min(to.0) - half;
        let top = from.1.min(to.1) - half;
        self.push(
            Instance {
                rect: [
                    left,
                    top,
                    from.0.max(to.0) + half - left,
                    from.1.max(to.1) + half - top,
                ],
                color: color.to_linear(),
                parameters: [from.0, from.1, to.0, to.1],
                second_parameters: [width * scale, 0.0, 0.0, 0.0],
                kind: [Instance::KIND_LINE, 0, 0, 0],
                ..Instance::default()
            },
            1.0,
        );
    }

    /// The glyphs of `run` at `size` logical units per em. The baseline
    /// starts at `origin` and snaps to whole physical pixels.
    pub fn run(&mut self, origin: (f32, f32), size: f32, run: &Run, color: Color) {
        self.run_glyphs(origin, size, run, 0..run.glyphs.len(), color);
    }

    /// The glyphs `glyphs` of `run` at `size` logical units per em, for a line
    /// of a wrapped text. The first glyph of the range starts at `origin`, and
    /// the baseline snaps to whole physical pixels. A range outside the run
    /// draws nothing.
    pub fn run_glyphs(
        &mut self,
        origin: (f32, f32),
        size: f32,
        run: &Run,
        glyphs: Range<usize>,
        color: Color,
    ) {
        let Some(selected) = run.glyphs.get(glyphs) else {
            return;
        };
        let Some(first) = selected.first() else {
            return;
        };
        let start = first.x;
        let baseline = (origin.1 * self.scale).round() / self.scale;
        for glyph in selected {
            if let Some(shape) = glyph.shape {
                self.slug(
                    (origin.0 + (glyph.x - start) * size, baseline),
                    size,
                    shape,
                    color,
                );
            }
        }
    }

    /// A path from [`crate::ShapeStore::add_path`]. The point `(0, 0)` of the
    /// path lies at `origin` and one path unit spans `size` logical units.
    pub fn path(&mut self, origin: (f32, f32), size: f32, shape: SlugShape, color: Color) {
        self.slug(origin, size, shape, color);
    }

    /// A Slug outline whose em space origin lies at `origin`. The em space
    /// y-axis points up.
    fn slug(&mut self, origin: (f32, f32), size: f32, shape: SlugShape, color: Color) {
        let Bounds { x_min, y_max, .. } = shape.bounds;
        let pixels_per_em = size * self.scale;
        if pixels_per_em <= 0.0 {
            return;
        }
        let left = (origin.0 + x_min * size) * self.scale;
        let top = (origin.1 - y_max * size) * self.scale;
        self.push(
            Instance {
                rect: [
                    left,
                    top,
                    shape.bounds.width() * pixels_per_em,
                    shape.bounds.height() * pixels_per_em,
                ],
                color: color.to_linear(),
                parameters: [x_min, y_max, pixels_per_em.recip(), -pixels_per_em.recip()],
                kind: [Instance::KIND_GLYPH, shape.index, 0, 0],
                ..Instance::default()
            },
            1.0,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Path, RunGlyph, ShapeStore};

    #[test]
    fn the_instance_record_has_128_bytes() {
        assert_eq!(size_of::<Instance>(), 128);
        let mut scene = Scene::new();
        scene.fill(Rect::new(0.0, 0.0, 1.0, 1.0), 0.0, Color::WHITE);
        scene.fill(Rect::new(1.0, 1.0, 1.0, 1.0), 0.0, Color::WHITE);
        assert_eq!(scene.instance_bytes().len(), 256);
        assert_eq!(scene.instance_bytes()[132..136], 1.0f32.to_ne_bytes());
    }

    #[test]
    fn items_outside_the_clip_are_dropped() {
        let mut scene = Scene::new();
        scene.begin(2.0, Color::BLACK);
        scene.push_clip(Rect::new(10.0, 10.0, 20.0, 20.0));
        scene.fill(Rect::new(40.0, 10.0, 5.0, 5.0), 0.0, Color::WHITE);
        scene.fill(Rect::new(15.0, 15.0, 5.0, 5.0), 0.0, Color::WHITE);
        scene.pop_clip();
        scene.fill(Rect::new(40.0, 10.0, 5.0, 5.0), 0.0, Color::WHITE);
        assert_eq!(scene.len(), 2);
        assert_eq!(scene.instances()[0].clip, [20.0, 20.0, 60.0, 60.0]);
        assert_eq!(scene.instances()[1].clip[2], f32::MAX);
    }

    #[test]
    fn the_overlay_follows_the_other_items_with_its_own_clip() {
        let mut scene = Scene::new();
        scene.begin(1.0, Color::BLACK);
        scene.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0));
        scene.begin_overlay();
        scene.fill(Rect::new(20.0, 20.0, 5.0, 5.0), 0.0, Color::WHITE);
        scene.end_overlay();
        scene.fill(Rect::new(1.0, 1.0, 5.0, 5.0), 0.0, Color::BLACK);
        scene.fill(Rect::new(20.0, 20.0, 5.0, 5.0), 0.0, Color::BLACK);
        scene.pop_clip();
        assert_eq!(scene.len(), 1);
        scene.finish();
        assert_eq!(scene.len(), 2);
        assert_eq!(scene.instances()[1].rect, [20.0, 20.0, 5.0, 5.0]);
        assert_eq!(scene.instances()[1].color, Color::WHITE.to_linear());
        scene.begin(1.0, Color::BLACK);
        scene.begin_overlay();
        scene.fill(Rect::new(1.0, 1.0, 5.0, 5.0), 0.0, Color::WHITE);
        scene.begin(1.0, Color::BLACK);
        scene.finish();
        assert!(scene.is_empty());
    }

    #[test]
    fn every_kind_is_culled_by_the_clip() {
        let mut store = ShapeStore::new();
        let shape = store
            .add_path(&Path::new().polygon(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]))
            .unwrap();
        let mut scene = Scene::new();
        scene.begin(1.0, Color::BLACK);
        scene.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0));
        scene.shadow(
            Rect::new(30.0, 0.0, 5.0, 5.0),
            1.0,
            2.0,
            (0.0, 0.0),
            Color::BLACK,
        );
        scene.arc((40.0, 5.0), 3.0, 1.0, (0.0, 1.0), Color::WHITE);
        scene.line((30.0, 0.0), (40.0, 0.0), 1.0, Color::WHITE);
        scene.path((30.0, 0.0), 5.0, shape, Color::WHITE);
        assert!(scene.is_empty());
        scene.shadow(
            Rect::new(14.0, 0.0, 5.0, 5.0),
            1.0,
            2.0,
            (0.0, 0.0),
            Color::BLACK,
        );
        scene.path((2.0, 2.0), 5.0, shape, Color::WHITE);
        scene.arc((12.0, 5.0), 2.0, 1.0, (0.0, 1.0), Color::WHITE);
        scene.line((10.5, 5.0), (20.0, 5.0), 1.0, Color::WHITE);
        let kinds: Vec<u32> = scene
            .instances()
            .iter()
            .map(|instance| instance.kind[0])
            .collect();
        assert_eq!(
            kinds,
            [
                Instance::KIND_SHADOW,
                Instance::KIND_GLYPH,
                Instance::KIND_ARC,
                Instance::KIND_LINE
            ]
        );
        assert_eq!(scene.instances()[1].kind, [Instance::KIND_GLYPH, 0, 0, 0]);
    }

    #[test]
    fn items_without_finite_geometry_or_area_are_dropped() {
        let mut scene = Scene::new();
        let shadow = |scene: &mut Scene, rect: Rect, blur: f32| {
            scene.shadow(rect, 2.0, blur, (0.0, 0.0), Color::BLACK);
        };
        shadow(&mut scene, Rect::new(f32::NAN, 10.0, 5.0, 5.0), 3.0);
        shadow(&mut scene, Rect::new(10.0, 10.0, f32::INFINITY, 5.0), 3.0);
        shadow(&mut scene, Rect::new(10.0, 10.0, 5.0, 5.0), f32::INFINITY);
        shadow(&mut scene, Rect::new(10.0, 10.0, 0.0, 5.0), 3.0);
        shadow(&mut scene, Rect::new(10.0, 10.0, 5.0, -1.0), 3.0);
        scene.fill(Rect::new(1.0, 1.0, 2.0, 2.0), f32::NAN, Color::WHITE);
        scene.line((0.0, 0.0), (f32::NAN, 4.0), 1.0, Color::WHITE);
        scene.arc((5.0, 5.0), 3.0, 1.0, (0.0, f32::INFINITY), Color::WHITE);
        assert!(scene.is_empty());
        shadow(&mut scene, Rect::new(10.0, 10.0, 5.0, 5.0), 3.0);
        assert_eq!(scene.len(), 1);
    }

    #[test]
    fn a_stroked_gradient_rectangle_packs_its_style() {
        let mut scene = Scene::new();
        scene.begin(1.5, Color::BLACK);
        let style = RectStyle::filled(Color::WHITE, 2.0)
            .with_stroke(1.0, Color::hex(0xFF0000))
            .with_gradient(Color::BLACK);
        scene.rect(Rect::new(1.0, 1.0, 3.0, 3.0), &style);
        let instance = scene.instances()[0];
        assert_eq!(instance.rect, [2.0, 2.0, 4.0, 4.0]);
        assert_eq!(instance.color, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(instance.second_color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(instance.radii, [3.0; 4]);
        assert_eq!(instance.parameters, [2.0, 1.0, 0.0, 0.0]);
        assert_eq!(instance.second_parameters, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(instance.kind, [Instance::KIND_RECTANGLE, 0, 0, 0]);
    }

    #[test]
    fn a_path_packs_its_em_space_mapping() {
        let mut store = ShapeStore::new();
        let triangle = Path::new().polygon(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        store.add_path(&triangle).unwrap();
        let square = Path::new().polygon(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        let shape = store.add_path(&square).unwrap();
        let mut scene = Scene::new();
        scene.begin(2.0, Color::BLACK);
        scene.path((3.0, 4.0), 10.0, shape, Color::WHITE);
        let instance = scene.instances()[0];
        let Bounds { x_min, y_max, .. } = shape.bounds;
        assert_eq!(instance.rect, [6.0, 8.0, 20.0, 20.0]);
        assert_eq!(instance.parameters, [x_min, y_max, 1.0 / 20.0, -1.0 / 20.0]);
        assert_eq!(instance.kind, [Instance::KIND_GLYPH, 1, 0, 0]);
    }

    #[test]
    fn a_run_snaps_its_baseline_to_physical_pixels() {
        let mut store = ShapeStore::new();
        let square = Path::new().polygon(&[[0.0, -1.0], [1.0, -1.0], [1.0, 0.0], [0.0, 0.0]]);
        let shape = store.add_path(&square).unwrap();
        let mut run = Run::default();
        run.push(RunGlyph::plain(Some(shape), 0), 1.0);
        run.push(RunGlyph::plain(None, 1), 0.5);
        run.push(RunGlyph::plain(Some(shape), 2), 1.0);
        let mut scene = Scene::new();
        scene.begin(1.5, Color::BLACK);
        scene.run((2.0, 10.3), 4.0, &run, Color::WHITE);
        assert_eq!(scene.len(), 2);
        assert_eq!(scene.instances()[0].rect, [3.0, 9.0, 6.0, 6.0]);
        assert_eq!(scene.instances()[1].rect, [12.0, 9.0, 6.0, 6.0]);
        scene.begin(1.5, Color::BLACK);
        scene.run_glyphs((2.0, 10.3), 4.0, &run, 1..3, Color::WHITE);
        assert_eq!(scene.len(), 1);
        assert_eq!(scene.instances()[0].rect, [6.0, 9.0, 6.0, 6.0]);
        scene.run_glyphs((2.0, 10.3), 4.0, &run, 2..4, Color::WHITE);
        scene.run_glyphs((2.0, 10.3), 4.0, &run, 1..1, Color::WHITE);
        assert_eq!(scene.len(), 1);
    }

    #[test]
    fn rectangles_snap_to_physical_pixels() {
        let mut scene = Scene::new();
        scene.begin(1.5, Color::BLACK);
        scene.fill(Rect::new(1.0, 1.0, 3.0, 3.0), 2.0, Color::WHITE);
        let instance = scene.instances()[0];
        assert_eq!(instance.rect, [2.0, 2.0, 4.0, 4.0]);
        assert_eq!(instance.radii, [3.0; 4]);
    }
}
