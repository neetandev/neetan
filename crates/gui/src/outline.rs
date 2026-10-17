//! The receiver of outline drawing commands.

/// Receives the contours of an outline as drawing commands.
///
/// A contour starts with [`OutlineSink::move_to`]. The segments that follow
/// continue from the end point of the previous command.
pub trait OutlineSink {
    /// Starts a contour at `(x, y)`.
    fn move_to(&mut self, x: f32, y: f32);

    /// A straight line to `(x, y)`.
    fn line_to(&mut self, x: f32, y: f32);

    /// A quadratic curve over the control point to `(x, y)`.
    fn quad_to(&mut self, control_x: f32, control_y: f32, x: f32, y: f32);

    /// A cubic curve over two control points to `(x, y)`.
    fn curve_to(
        &mut self,
        first_control_x: f32,
        first_control_y: f32,
        second_control_x: f32,
        second_control_y: f32,
        x: f32,
        y: f32,
    );

    /// Closes the contour with a line to its start.
    fn close(&mut self);
}
