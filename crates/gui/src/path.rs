//! Filled shapes from code, for icons.

use crate::OutlineSink;

/// One drawing command of a [`Path`].
#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    /// Starts a contour at the point.
    MoveTo([f32; 2]),
    /// A straight line to the point.
    LineTo([f32; 2]),
    /// A quadratic curve over the control point to the end point.
    QuadTo([f32; 2], [f32; 2]),
    /// A cubic curve over two control points to the end point.
    CubicTo([f32; 2], [f32; 2], [f32; 2]),
    /// Closes the contour.
    Close,
}

/// A filled outline of one or more contours with the nonzero fill rule.
///
/// The coordinates use the y-axis pointing down. A renderer scales them like
/// the em units of a glyph. An icon usually spans the box from 0 to 1.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    /// The drawing commands in order.
    commands: Vec<Command>,
}

impl Path {
    /// An empty path.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a contour at `(x, y)`.
    pub fn move_to(mut self, x: f32, y: f32) -> Self {
        self.commands.push(Command::MoveTo([x, y]));
        self
    }

    /// A straight line to `(x, y)`.
    pub fn line_to(mut self, x: f32, y: f32) -> Self {
        self.commands.push(Command::LineTo([x, y]));
        self
    }

    /// A quadratic curve over the control point to `(x, y)`.
    pub fn quad_to(mut self, control_x: f32, control_y: f32, x: f32, y: f32) -> Self {
        self.commands
            .push(Command::QuadTo([control_x, control_y], [x, y]));
        self
    }

    /// A cubic curve over two control points to `(x, y)`.
    pub fn cubic_to(
        mut self,
        first_control_x: f32,
        first_control_y: f32,
        second_control_x: f32,
        second_control_y: f32,
        x: f32,
        y: f32,
    ) -> Self {
        self.commands.push(Command::CubicTo(
            [first_control_x, first_control_y],
            [second_control_x, second_control_y],
            [x, y],
        ));
        self
    }

    /// Closes the contour with a line to its start.
    pub fn close(mut self) -> Self {
        self.commands.push(Command::Close);
        self
    }

    /// A closed polygon through `points`.
    pub fn polygon(self, points: &[[f32; 2]]) -> Self {
        let Some((first, rest)) = points.split_first() else {
            return self;
        };
        let mut path = self.move_to(first[0], first[1]);
        for point in rest {
            path = path.line_to(point[0], point[1]);
        }
        path.close()
    }

    /// Feeds the commands to `sink`.
    pub(crate) fn replay(&self, sink: &mut (impl OutlineSink + ?Sized)) {
        for command in &self.commands {
            match *command {
                Command::MoveTo([x, y]) => sink.move_to(x, y),
                Command::LineTo([x, y]) => sink.line_to(x, y),
                Command::QuadTo([control_x, control_y], [x, y]) => {
                    sink.quad_to(control_x, control_y, x, y)
                }
                Command::CubicTo(
                    [first_control_x, first_control_y],
                    [second_control_x, second_control_y],
                    [x, y],
                ) => sink.curve_to(
                    first_control_x,
                    first_control_y,
                    second_control_x,
                    second_control_y,
                    x,
                    y,
                ),
                Command::Close => sink.close(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ShapeStore, slug::tests::coverage};

    #[test]
    fn a_triangle_covers_its_inside() {
        let mut store = ShapeStore::new();
        let triangle = Path::new().polygon(&[[0.0, 0.0], [1.0, 0.5], [0.0, 1.0]]);
        let shape = store.add_path(&triangle).unwrap();
        assert_eq!(shape.bounds.y_max, 0.0);
        assert_eq!(shape.bounds.y_min, -1.0);
        let glyph = store.slug().glyphs()[shape.index as usize];
        assert_eq!(coverage(store.slug(), &glyph, [0.25, -0.5]), 1.0);
        assert_eq!(coverage(store.slug(), &glyph, [0.9, -0.1]), 0.0);
    }

    #[test]
    fn an_empty_path_adds_nothing() {
        let mut store = ShapeStore::new();
        assert_eq!(store.add_path(&Path::new()), None);
        assert!(store.slug().glyphs().is_empty());
    }
}
