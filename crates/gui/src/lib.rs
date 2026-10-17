//! A data-driven user interface renderer.
//!
//! A [`Scene`] collects the draw items of a frame: rectangles with rounded
//! corners, strokes and gradients, soft shadows, arcs, lines, glyphs and
//! paths. Every pixel takes its coverage from the exact geometry of each item.
//! Shapes use signed distance functions. Glyphs and paths use the Slug
//! algorithm on the data of a [`ShapeStore`]. The output stays sharp at every
//! scale. A [`Rasterizer`] draws a scene on the CPU.
//!
//! The [`truetype`] module reads TrueType fonts with `glyf` outlines. The
//! [`fonts`] module names the font file of the user interface. [`Fonts`]
//! holds the loaded faces, shapes text into [`Run`]s, cuts runs to a width
//! and wraps text into lines.
//!
//! The [`ui`] module holds the immediate mode widgets that draw into a
//! scene: the context with the input and the focus of a frame, the theme,
//! the icons and the widgets.
//!
//! The records are `#[repr(C)]` and offer byte views for an upload to the GPU.

#![deny(missing_docs)]
#![deny(unsafe_code)]

mod bytes;
mod color;
pub mod fonts;
mod outline;
mod path;
mod rasterizer;
mod scene;
mod shapes;
mod slug;
pub mod truetype;
pub mod ui;

pub use color::{Color, linear_to_srgb, srgb_to_linear};
pub use fonts::{FontId, FontMetrics, Fonts, Line, Run, RunGlyph};
pub use outline::OutlineSink;
pub use path::Path;
pub use rasterizer::{BLUE_NOISE, BLUE_NOISE_SIZE, Rasterizer};
pub use scene::{Instance, RectStyle, Scene};
pub use shapes::ShapeStore;
pub use slug::{Bounds, SlugData, SlugGlyph, SlugShape};

/// A rectangle in logical units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// The left edge.
    pub x: f32,
    /// The top edge.
    pub y: f32,
    /// The width.
    pub width: f32,
    /// The height.
    pub height: f32,
}

impl Rect {
    /// A rectangle with its top left corner at `(x, y)`.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Whether `(x, y)` lies inside the rectangle.
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }

    /// The rectangle shrunk by `amount` on every side.
    pub fn inset(self, amount: f32) -> Self {
        Self::new(
            self.x + amount,
            self.y + amount,
            (self.width - amount * 2.0).max(0.0),
            (self.height - amount * 2.0).max(0.0),
        )
    }

    /// The rectangle grown by `amount` on every side.
    pub fn expand(self, amount: f32) -> Self {
        Self::new(
            self.x - amount,
            self.y - amount,
            self.width + amount * 2.0,
            self.height + amount * 2.0,
        )
    }

    /// The center point.
    pub fn center(self) -> (f32, f32) {
        (self.x + self.width * 0.5, self.y + self.height * 0.5)
    }

    /// The right edge.
    pub fn right(self) -> f32 {
        self.x + self.width
    }

    /// The bottom edge.
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    /// Splits off a strip of `height` at the top. Returns the strip and the
    /// rest. The strip stays within the rectangle, and a negative or NaN
    /// height gives an empty strip.
    pub fn cut_top(self, height: f32) -> (Self, Self) {
        let height = height.min(self.height).max(0.0);
        (
            Self::new(self.x, self.y, self.width, height),
            Self::new(self.x, self.y + height, self.width, self.height - height),
        )
    }

    /// Splits off a strip of `height` at the bottom. Returns the strip and
    /// the rest. The strip never exceeds the rectangle.
    pub fn cut_bottom(self, height: f32) -> (Self, Self) {
        let height = height.min(self.height).max(0.0);
        (
            Self::new(self.x, self.bottom() - height, self.width, height),
            Self::new(self.x, self.y, self.width, self.height - height),
        )
    }

    /// Splits off a strip of `width` at the left. Returns the strip and the
    /// rest. The strip never exceeds the rectangle.
    pub fn cut_left(self, width: f32) -> (Self, Self) {
        let width = width.min(self.width).max(0.0);
        (
            Self::new(self.x, self.y, width, self.height),
            Self::new(self.x + width, self.y, self.width - width, self.height),
        )
    }

    /// Splits off a strip of `width` at the right. Returns the strip and the
    /// rest. The strip never exceeds the rectangle.
    pub fn cut_right(self, width: f32) -> (Self, Self) {
        let width = width.min(self.width).max(0.0);
        (
            Self::new(self.right() - width, self.y, width, self.height),
            Self::new(self.x, self.y, self.width - width, self.height),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangles_contain_their_inside() {
        let rect = Rect::new(1.0, 2.0, 3.0, 4.0);
        assert!(rect.contains(1.0, 2.0));
        assert!(!rect.contains(4.0, 2.0));
        assert_eq!(rect.center(), (2.5, 4.0));
        assert_eq!(rect.inset(1.0), Rect::new(2.0, 3.0, 1.0, 2.0));
        assert_eq!(rect.inset(2.0), Rect::new(3.0, 4.0, 0.0, 0.0));
        assert_eq!(rect.expand(1.0), Rect::new(0.0, 1.0, 5.0, 6.0));
        assert_eq!((rect.right(), rect.bottom()), (4.0, 6.0));
    }

    #[test]
    fn cuts_split_a_rectangle_into_two() {
        let rect = Rect::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!(
            rect.cut_top(10.0),
            (
                Rect::new(10.0, 20.0, 100.0, 10.0),
                Rect::new(10.0, 30.0, 100.0, 40.0)
            )
        );
        assert_eq!(
            rect.cut_bottom(10.0),
            (
                Rect::new(10.0, 60.0, 100.0, 10.0),
                Rect::new(10.0, 20.0, 100.0, 40.0)
            )
        );
        assert_eq!(
            rect.cut_left(30.0),
            (
                Rect::new(10.0, 20.0, 30.0, 50.0),
                Rect::new(40.0, 20.0, 70.0, 50.0)
            )
        );
        assert_eq!(
            rect.cut_right(30.0),
            (
                Rect::new(80.0, 20.0, 30.0, 50.0),
                Rect::new(10.0, 20.0, 70.0, 50.0)
            )
        );
        assert_eq!(rect.cut_top(80.0).1.height, 0.0);
    }

    #[test]
    fn cuts_of_broken_sizes_stay_finite() {
        let negative = Rect::new(10.0, 20.0, -5.0, -8.0);
        for size in [-3.0, 0.0, 4.0, f32::NAN] {
            assert_eq!(negative.cut_top(size).0.height, 0.0);
            assert_eq!(negative.cut_bottom(size).0.height, 0.0);
            assert_eq!(negative.cut_left(size).0.width, 0.0);
            assert_eq!(negative.cut_right(size).0.width, 0.0);
            let (strip, rest) = Rect::new(0.0, 0.0, 10.0, 10.0).cut_top(size);
            assert!(strip.height >= 0.0 && strip.height <= 10.0, "{size}");
            assert_eq!(strip.height + rest.height, 10.0);
        }
        let (strip, _) = Rect::new(0.0, 0.0, 10.0, f32::NAN).cut_top(4.0);
        assert_eq!(strip.height, 4.0);
    }
}
