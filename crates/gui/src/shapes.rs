//! The owner of the Slug data of all glyphs and paths.

use std::convert::Infallible;

use crate::{OutlineSink, Path, SlugData, SlugShape, slug::ContourBuilder};

/// Converts outlines into Slug data and keeps that data for the renderers.
///
/// Paths and font glyphs share one store. A [`SlugShape`] indexes into the
/// [`SlugData`] of the store that created it.
#[derive(Clone, Debug, Default)]
pub struct ShapeStore {
    /// The Slug arrays of every converted outline.
    slug: SlugData,
}

impl ShapeStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// The Slug data of every converted outline.
    pub fn slug(&self) -> &SlugData {
        &self.slug
    }

    /// Converts `path` into Slug data. A path without contours returns `None`.
    pub fn add_path(&mut self, path: &Path) -> Option<SlugShape> {
        // Paths use the y-axis pointing down. The Slug data uses it pointing up.
        let Ok(shape) = self.add_outline::<Infallible>([1.0, -1.0], [0.0, 0.0], |sink| {
            path.replay(sink);
            Ok(())
        });
        shape
    }

    /// Converts the outline that `draw` emits into Slug data. Every point `p`
    /// maps to `p * scale + offset` in em space with the y-axis pointing up.
    /// An outline without contours returns `None`. An error of `draw` adds
    /// nothing and returns the error.
    pub fn add_outline<E>(
        &mut self,
        scale: [f32; 2],
        offset: [f32; 2],
        draw: impl FnOnce(&mut dyn OutlineSink) -> Result<(), E>,
    ) -> Result<Option<SlugShape>, E> {
        let mut builder = ContourBuilder::new(scale, offset);
        draw(&mut builder)?;
        Ok(self.slug.add(builder))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slug::tests::coverage;

    #[test]
    fn outlines_append_to_the_same_data() {
        let mut store = ShapeStore::new();
        let square = Path::new().polygon(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        let first = store.add_path(&square).unwrap();
        let second = store
            .add_outline::<Infallible>([0.5, 0.5], [0.0, 0.0], |sink| {
                sink.move_to(0.0, 0.0);
                sink.line_to(2.0, 0.0);
                sink.line_to(2.0, 2.0);
                sink.close();
                Ok(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(first.index, 0);
        assert_eq!(second.index, 1);
        assert_eq!(second.bounds.x_max, 1.0);
        assert_eq!(second.bounds.y_max, 1.0);
        let glyph = store.slug().glyphs()[1];
        assert_eq!(coverage(store.slug(), &glyph, [0.75, 0.25]), 1.0);
        assert_eq!(coverage(store.slug(), &glyph, [0.25, 0.75]), 0.0);
    }

    #[test]
    fn the_offset_shifts_the_outline() {
        let mut store = ShapeStore::new();
        let shape = store
            .add_outline::<Infallible>([1.0, 1.0], [0.25, -0.5], |sink| {
                sink.move_to(0.0, 0.0);
                sink.line_to(1.0, 0.0);
                sink.line_to(1.0, 2.0);
                sink.close();
                Ok(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(shape.bounds.x_min, 0.25);
        assert_eq!(shape.bounds.x_max, 1.25);
        assert_eq!(shape.bounds.y_min, -0.5);
        assert_eq!(shape.bounds.y_max, 1.5);
    }

    #[test]
    fn a_failed_outline_adds_nothing() {
        let mut store = ShapeStore::new();
        let result = store.add_outline([1.0, 1.0], [0.0, 0.0], |sink| {
            sink.move_to(0.0, 0.0);
            sink.line_to(1.0, 0.0);
            sink.line_to(1.0, 1.0);
            Err("truncated outline")
        });
        assert_eq!(result, Err("truncated outline"));
        assert!(store.slug().glyphs().is_empty());
        assert!(store.slug().curves().is_empty());
        assert!(store.slug().bands().is_empty());
    }
}
