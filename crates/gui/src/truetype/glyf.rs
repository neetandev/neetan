//! The `loca` and `glyf` tables: the quadratic glyph outlines.

use super::{
    FontError, GlyphId, Tag,
    reader::{Reader, slice, u16_at, u32_at},
};
use crate::OutlineSink;

/// The deepest nesting of composite glyphs.
const MAX_COMPOSITE_DEPTH: u8 = 8;

/// The most components that one outline may draw, counted over all nesting
/// levels.
const MAX_COMPONENTS: u32 = 1024;

/// The most points that one outline may draw, counted over all components.
const MAX_POINTS: usize = 1 << 16;

/// The point flag of a point on the curve.
const ON_CURVE_POINT: u8 = 0x01;

/// The point flag of a one byte x coordinate.
const X_SHORT_VECTOR: u8 = 0x02;

/// The point flag of a one byte y coordinate.
const Y_SHORT_VECTOR: u8 = 0x04;

/// The point flag that repeats the flag by the count in the next byte.
const REPEAT_FLAG: u8 = 0x08;

/// The point flag of a repeated x coordinate, or of a positive one byte x
/// coordinate.
const X_IS_SAME_OR_POSITIVE: u8 = 0x10;

/// The point flag of a repeated y coordinate, or of a positive one byte y
/// coordinate.
const Y_IS_SAME_OR_POSITIVE: u8 = 0x20;

/// The component flag of 16-bit arguments.
const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;

/// The component flag of arguments that hold an offset.
const ARGS_ARE_XY_VALUES: u16 = 0x0002;

/// The component flag of a uniform scale.
const WE_HAVE_A_SCALE: u16 = 0x0008;

/// The component flag of a following component.
const MORE_COMPONENTS: u16 = 0x0020;

/// The component flag of separate x and y scales.
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;

/// The component flag of a 2 x 2 transform.
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;

/// The component flag of an offset that the transform scales.
const SCALED_COMPONENT_OFFSET: u16 = 0x0800;

/// The component flag of an offset that the transform does not scale.
const UNSCALED_COMPONENT_OFFSET: u16 = 0x1000;

/// An affine transform of glyph points.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Transform {
    /// The x scale.
    xx: f32,
    /// The share of x in the new y.
    yx: f32,
    /// The share of y in the new x.
    xy: f32,
    /// The y scale.
    yy: f32,
    /// The x offset.
    x_offset: f32,
    /// The y offset.
    y_offset: f32,
}

impl Transform {
    /// The transform that keeps every point.
    const IDENTITY: Self = Self {
        xx: 1.0,
        yx: 0.0,
        xy: 0.0,
        yy: 1.0,
        x_offset: 0.0,
        y_offset: 0.0,
    };

    /// The transformed point `(x, y)`.
    fn apply(&self, x: f32, y: f32) -> [f32; 2] {
        [
            self.xx * x + self.xy * y + self.x_offset,
            self.yx * x + self.yy * y + self.y_offset,
        ]
    }

    /// The transform that applies `inner` first and `self` second.
    fn after(&self, inner: &Self) -> Self {
        let [x_offset, y_offset] = self.apply(inner.x_offset, inner.y_offset);
        Self {
            xx: self.xx * inner.xx + self.xy * inner.yx,
            yx: self.yx * inner.xx + self.yy * inner.yx,
            xy: self.xx * inner.xy + self.xy * inner.yy,
            yy: self.yx * inner.xy + self.yy * inner.yy,
            x_offset,
            y_offset,
        }
    }
}

/// The components and points that an outline may still draw.
struct Budget {
    /// The remaining components.
    components: u32,
    /// The remaining points.
    points: usize,
}

/// The glyph outlines of a font.
#[derive(Clone, Copy)]
pub(crate) struct Outlines<'a> {
    /// The `loca` table.
    loca: &'a [u8],
    /// The `glyf` table.
    glyf: &'a [u8],
    /// Whether `loca` holds 32-bit offsets.
    long_offsets: bool,
    /// The number of glyphs.
    glyph_count: u16,
}

impl<'a> Outlines<'a> {
    /// The outlines of `glyph_count` glyphs. `loca` holds 32-bit offsets when
    /// `long_offsets` is set and 16-bit offsets in units of two bytes
    /// otherwise.
    pub(crate) fn new(
        loca: &'a [u8],
        glyf: &'a [u8],
        long_offsets: bool,
        glyph_count: u16,
    ) -> Result<Self, FontError> {
        let entry_size = if long_offsets { 4 } else { 2 };
        if loca.len() < (usize::from(glyph_count) + 1) * entry_size {
            return Err(FontError::InvalidTable(Tag::LOCA));
        }
        Ok(Self {
            loca,
            glyf,
            long_offsets,
            glyph_count,
        })
    }

    /// The `glyf` data of `glyph`. An empty glyph has no data.
    fn glyph_data(&self, glyph: u16) -> Result<&'a [u8], FontError> {
        if glyph >= self.glyph_count {
            return Err(FontError::UnknownGlyph(GlyphId(glyph)));
        }
        let index = usize::from(glyph);
        let (start, end) = if self.long_offsets {
            (
                u32_at(self.loca, index * 4)? as usize,
                u32_at(self.loca, index * 4 + 4)? as usize,
            )
        } else {
            (
                usize::from(u16_at(self.loca, index * 2)?) * 2,
                usize::from(u16_at(self.loca, index * 2 + 2)?) * 2,
            )
        };
        if end < start {
            return Err(FontError::InvalidTable(Tag::LOCA));
        }
        slice(self.glyf, start, end - start)
    }

    /// Emits the outline of `glyph` in font units to `sink`.
    pub(crate) fn draw(&self, glyph: u16, sink: &mut dyn OutlineSink) -> Result<(), FontError> {
        let mut budget = Budget {
            components: MAX_COMPONENTS,
            points: MAX_POINTS,
        };
        self.draw_glyph(glyph, &Transform::IDENTITY, 0, &mut budget, sink)
    }

    /// Emits the outline of `glyph` through `transform`. `depth` counts the
    /// composite levels above the glyph and `budget` the components and
    /// points that the outline may still draw.
    fn draw_glyph(
        &self,
        glyph: u16,
        transform: &Transform,
        depth: u8,
        budget: &mut Budget,
        sink: &mut dyn OutlineSink,
    ) -> Result<(), FontError> {
        let data = self.glyph_data(glyph)?;
        if data.is_empty() {
            return Ok(());
        }
        let mut reader = Reader::new(data);
        let contour_count = reader.i16()?;
        reader.skip(8)?;
        match contour_count {
            0 => Ok(()),
            1.. => draw_simple(
                reader,
                contour_count.unsigned_abs(),
                transform,
                budget,
                sink,
            ),
            _ => self.draw_composite(reader, transform, depth, budget, sink),
        }
    }

    /// Emits the components of the composite glyph at `reader`.
    fn draw_composite(
        &self,
        mut reader: Reader,
        transform: &Transform,
        depth: u8,
        budget: &mut Budget,
        sink: &mut dyn OutlineSink,
    ) -> Result<(), FontError> {
        if depth >= MAX_COMPOSITE_DEPTH {
            return Err(FontError::OutlineLimit);
        }
        loop {
            let flags = reader.u16()?;
            let glyph = reader.u16()?;
            let offset_values = flags & ARGS_ARE_XY_VALUES != 0;
            let (first, second) = match (flags & ARG_1_AND_2_ARE_WORDS != 0, offset_values) {
                (true, true) => (f32::from(reader.i16()?), f32::from(reader.i16()?)),
                (true, false) => (f32::from(reader.u16()?), f32::from(reader.u16()?)),
                (false, true) => (f32::from(reader.i8()?), f32::from(reader.i8()?)),
                (false, false) => (f32::from(reader.u8()?), f32::from(reader.u8()?)),
            };
            let mut component = Transform::IDENTITY;
            if flags & WE_HAVE_A_SCALE != 0 {
                component.xx = reader.f2dot14()?;
                component.yy = component.xx;
            } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
                component.xx = reader.f2dot14()?;
                component.yy = reader.f2dot14()?;
            } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
                component.xx = reader.f2dot14()?;
                component.yx = reader.f2dot14()?;
                component.xy = reader.f2dot14()?;
                component.yy = reader.f2dot14()?;
            }
            // Arguments that match two points are not supported. The component
            // keeps its own position.
            if offset_values {
                let scaled =
                    flags & SCALED_COMPONENT_OFFSET != 0 && flags & UNSCALED_COMPONENT_OFFSET == 0;
                [component.x_offset, component.y_offset] = if scaled {
                    component.apply(first, second)
                } else {
                    [first, second]
                };
            }
            budget.components = budget
                .components
                .checked_sub(1)
                .ok_or(FontError::OutlineLimit)?;
            let combined = transform.after(&component);
            self.draw_glyph(glyph, &combined, depth + 1, budget, sink)?;
            if flags & MORE_COMPONENTS == 0 {
                return Ok(());
            }
        }
    }
}

/// Decodes the points of a simple glyph in order.
struct Points<'a> {
    /// The reader of the flags.
    flags: Reader<'a>,
    /// The reader of the x coordinates.
    x_coordinates: Reader<'a>,
    /// The reader of the y coordinates.
    y_coordinates: Reader<'a>,
    /// The flag of the current repetition.
    flag: u8,
    /// The number of points that still repeat `flag`.
    repeat: u8,
    /// The x coordinate of the last point.
    x: i32,
    /// The y coordinate of the last point.
    y: i32,
}

impl Points<'_> {
    /// The next point as its coordinates and whether it lies on the curve.
    fn next_point(&mut self) -> Result<(i32, i32, bool), FontError> {
        if self.repeat > 0 {
            self.repeat -= 1;
        } else {
            self.flag = self.flags.u8()?;
            if self.flag & REPEAT_FLAG != 0 {
                self.repeat = self.flags.u8()?;
            }
        }
        let flag = self.flag;
        self.x = self.x.wrapping_add(coordinate_delta(
            &mut self.x_coordinates,
            flag,
            X_SHORT_VECTOR,
            X_IS_SAME_OR_POSITIVE,
        )?);
        self.y = self.y.wrapping_add(coordinate_delta(
            &mut self.y_coordinates,
            flag,
            Y_SHORT_VECTOR,
            Y_IS_SAME_OR_POSITIVE,
        )?);
        Ok((self.x, self.y, flag & ON_CURVE_POINT != 0))
    }
}

/// Reads one coordinate delta of a point with `flag`. `short` and
/// `same_or_positive` are the flag bits of the axis.
fn coordinate_delta(
    reader: &mut Reader,
    flag: u8,
    short: u8,
    same_or_positive: u8,
) -> Result<i32, FontError> {
    Ok(match (flag & short != 0, flag & same_or_positive != 0) {
        (true, true) => i32::from(reader.u8()?),
        (true, false) => -i32::from(reader.u8()?),
        (false, true) => 0,
        (false, false) => i32::from(reader.i16()?),
    })
}

/// The byte size of a coordinate of a point with `flag`.
fn coordinate_size(flag: u8, short: u8, same_or_positive: u8) -> usize {
    if flag & short != 0 {
        1
    } else if flag & same_or_positive != 0 {
        0
    } else {
        2
    }
}

/// Emits the `contour_count` contours of the simple glyph at `reader` and
/// charges its points to `budget`.
fn draw_simple(
    mut reader: Reader,
    contour_count: u16,
    transform: &Transform,
    budget: &mut Budget,
    sink: &mut dyn OutlineSink,
) -> Result<(), FontError> {
    let end_points = reader.bytes(usize::from(contour_count) * 2)?;
    let mut point_count = 0;
    for contour in 0..usize::from(contour_count) {
        let end = usize::from(u16_at(end_points, contour * 2)?) + 1;
        if end <= point_count {
            return Err(FontError::InvalidTable(Tag::GLYF));
        }
        point_count = end;
    }
    budget.points = budget
        .points
        .checked_sub(point_count)
        .ok_or(FontError::OutlineLimit)?;
    let instruction_length = reader.u16()?;
    reader.skip(usize::from(instruction_length))?;

    let flags = reader;
    let mut x_length = 0;
    let mut y_length = 0;
    let mut counted = 0;
    while counted < point_count {
        let flag = reader.u8()?;
        let repeat = if flag & REPEAT_FLAG != 0 {
            usize::from(reader.u8()?) + 1
        } else {
            1
        };
        let repeat = repeat.min(point_count - counted);
        x_length += coordinate_size(flag, X_SHORT_VECTOR, X_IS_SAME_OR_POSITIVE) * repeat;
        y_length += coordinate_size(flag, Y_SHORT_VECTOR, Y_IS_SAME_OR_POSITIVE) * repeat;
        counted += repeat;
    }
    let x_coordinates = Reader::new(reader.bytes(x_length)?);
    let y_coordinates = Reader::new(reader.bytes(y_length)?);

    let mut points = Points {
        flags,
        x_coordinates,
        y_coordinates,
        flag: 0,
        repeat: 0,
        x: 0,
        y: 0,
    };
    let mut start = 0;
    for contour in 0..usize::from(contour_count) {
        let end = usize::from(u16_at(end_points, contour * 2)?) + 1;
        let mut builder = ContourEmitter::default();
        for _ in start..end {
            let (x, y, on_curve) = points.next_point()?;
            builder.push(transform.apply(x as f32, y as f32), on_curve, sink);
        }
        builder.finish(sink);
        start = end;
    }
    Ok(())
}

/// The midpoint of two points.
fn midpoint(first: [f32; 2], second: [f32; 2]) -> [f32; 2] {
    [(first[0] + second[0]) * 0.5, (first[1] + second[1]) * 0.5]
}

/// Turns the points of one contour into drawing commands.
///
/// Two consecutive off-curve points imply an on-curve point at their midpoint.
/// A contour that starts with an off-curve point begins at the first on-curve
/// point and closes over the off-curve points of its start.
#[derive(Default)]
struct ContourEmitter {
    /// The point where the contour begins.
    first_on: Option<[f32; 2]>,
    /// The off-curve point before `first_on`.
    first_off: Option<[f32; 2]>,
    /// The off-curve point that the next segment uses as its control point.
    last_off: Option<[f32; 2]>,
}

impl ContourEmitter {
    /// Adds the next point of the contour.
    fn push(&mut self, point: [f32; 2], on_curve: bool, sink: &mut dyn OutlineSink) {
        if self.first_on.is_none() {
            if on_curve {
                self.first_on = Some(point);
                sink.move_to(point[0], point[1]);
            } else if let Some(first_off) = self.first_off {
                let start = midpoint(first_off, point);
                self.first_on = Some(start);
                self.last_off = Some(point);
                sink.move_to(start[0], start[1]);
            } else {
                self.first_off = Some(point);
            }
            return;
        }
        match (self.last_off, on_curve) {
            (Some(control), true) => {
                sink.quad_to(control[0], control[1], point[0], point[1]);
                self.last_off = None;
            }
            (Some(control), false) => {
                let end = midpoint(control, point);
                sink.quad_to(control[0], control[1], end[0], end[1]);
                self.last_off = Some(point);
            }
            (None, true) => sink.line_to(point[0], point[1]),
            (None, false) => self.last_off = Some(point),
        }
    }

    /// Closes the contour over its start.
    fn finish(self, sink: &mut dyn OutlineSink) {
        let Some(first_on) = self.first_on else {
            return;
        };
        match (self.first_off, self.last_off) {
            (Some(first_off), Some(control)) => {
                let end = midpoint(control, first_off);
                sink.quad_to(control[0], control[1], end[0], end[1]);
                sink.quad_to(first_off[0], first_off[1], first_on[0], first_on[1]);
            }
            (Some(control), None) | (None, Some(control)) => {
                sink.quad_to(control[0], control[1], first_on[0], first_on[1]);
            }
            (None, None) => {}
        }
        sink.close();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A drawing command recorded by [`Recorder`].
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(crate) enum Command {
        /// A contour start.
        Move([f32; 2]),
        /// A line.
        Line([f32; 2]),
        /// A quadratic curve with its control and end point.
        Quad([f32; 2], [f32; 2]),
        /// A contour end.
        Close,
    }

    /// An outline sink that records the commands.
    #[derive(Default)]
    pub(crate) struct Recorder {
        /// The recorded commands.
        pub(crate) commands: Vec<Command>,
    }

    impl Recorder {
        /// The number of contours.
        pub(crate) fn contours(&self) -> usize {
            self.commands
                .iter()
                .filter(|command| matches!(command, Command::Move(_)))
                .count()
        }
    }

    impl OutlineSink for Recorder {
        fn move_to(&mut self, x: f32, y: f32) {
            self.commands.push(Command::Move([x, y]));
        }

        fn line_to(&mut self, x: f32, y: f32) {
            self.commands.push(Command::Line([x, y]));
        }

        fn quad_to(&mut self, control_x: f32, control_y: f32, x: f32, y: f32) {
            self.commands
                .push(Command::Quad([control_x, control_y], [x, y]));
        }

        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
            unreachable!("TrueType outlines have no cubic curves");
        }

        fn close(&mut self) {
            self.commands.push(Command::Close);
        }
    }

    /// The `glyf` data of a simple glyph from contours of `(x, y, on curve)`
    /// points. The coordinates use the long form.
    pub(crate) fn simple_glyph(contours: &[&[(i16, i16, bool)]]) -> Vec<u8> {
        let mut data = (contours.len() as i16).to_be_bytes().to_vec();
        data.extend([0; 8]);
        let mut end = 0;
        for contour in contours {
            end += contour.len();
            data.extend(((end - 1) as u16).to_be_bytes());
        }
        data.extend([0, 0]);
        let points: Vec<_> = contours.iter().flat_map(|contour| contour.iter()).collect();
        for &&(_, _, on_curve) in &points {
            data.push(u8::from(on_curve));
        }
        let mut previous = (0, 0);
        for &&(x, _, _) in &points {
            data.extend((x - previous.0).to_be_bytes());
            previous.0 = x;
        }
        for &&(_, y, _) in &points {
            data.extend((y - previous.1).to_be_bytes());
            previous.1 = y;
        }
        data
    }

    /// The `glyf` data of a composite glyph from components of glyph id,
    /// flags and the bytes of the arguments and the transform.
    pub(crate) fn composite_glyph(components: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
        let mut data = (-1i16).to_be_bytes().to_vec();
        data.extend([0; 8]);
        for (index, (glyph, flags, arguments)) in components.iter().enumerate() {
            let more = if index + 1 < components.len() {
                MORE_COMPONENTS
            } else {
                0
            };
            data.extend((flags | more).to_be_bytes());
            data.extend(glyph.to_be_bytes());
            data.extend(arguments);
        }
        data
    }

    /// A long `loca` table and the `glyf` table of `glyphs`.
    pub(crate) fn glyph_tables(glyphs: &[Vec<u8>]) -> (Vec<u8>, Vec<u8>) {
        let mut loca = 0u32.to_be_bytes().to_vec();
        let mut glyf = Vec::new();
        for glyph in glyphs {
            glyf.extend(glyph);
            loca.extend((glyf.len() as u32).to_be_bytes());
        }
        (loca, glyf)
    }

    /// Records the outline of `glyph` of `glyphs`.
    fn record(glyphs: &[Vec<u8>], glyph: u16) -> Result<Recorder, FontError> {
        let (loca, glyf) = glyph_tables(glyphs);
        let outlines = Outlines::new(&loca, &glyf, true, glyphs.len() as u16)?;
        let mut recorder = Recorder::default();
        outlines.draw(glyph, &mut recorder)?;
        Ok(recorder)
    }

    #[test]
    fn a_square_draws_lines() {
        let square = simple_glyph(&[&[
            (0, 0, true),
            (100, 0, true),
            (100, 100, true),
            (0, 100, true),
        ]]);
        let recorder = record(&[square], 0).unwrap();
        assert_eq!(
            recorder.commands,
            [
                Command::Move([0.0, 0.0]),
                Command::Line([100.0, 0.0]),
                Command::Line([100.0, 100.0]),
                Command::Line([0.0, 100.0]),
                Command::Close,
            ]
        );
    }

    #[test]
    fn two_off_curve_points_imply_an_on_curve_point() {
        let glyph = simple_glyph(&[&[
            (0, 0, true),
            (100, 0, false),
            (100, 100, false),
            (0, 100, true),
        ]]);
        let recorder = record(&[glyph], 0).unwrap();
        assert_eq!(
            recorder.commands,
            [
                Command::Move([0.0, 0.0]),
                Command::Quad([100.0, 0.0], [100.0, 50.0]),
                Command::Quad([100.0, 100.0], [0.0, 100.0]),
                Command::Close,
            ]
        );
    }

    #[test]
    fn a_contour_may_start_off_the_curve() {
        let glyph = simple_glyph(&[&[(100, 0, false), (100, 100, true), (0, 100, true)]]);
        let recorder = record(&[glyph], 0).unwrap();
        assert_eq!(
            recorder.commands,
            [
                Command::Move([100.0, 100.0]),
                Command::Line([0.0, 100.0]),
                Command::Quad([100.0, 0.0], [100.0, 100.0]),
                Command::Close,
            ]
        );

        let circle = simple_glyph(&[&[
            (0, -10, false),
            (10, 0, false),
            (0, 10, false),
            (-10, 0, false),
        ]]);
        let recorder = record(&[circle], 0).unwrap();
        assert_eq!(
            recorder.commands,
            [
                Command::Move([5.0, -5.0]),
                Command::Quad([10.0, 0.0], [5.0, 5.0]),
                Command::Quad([0.0, 10.0], [-5.0, 5.0]),
                Command::Quad([-10.0, 0.0], [-5.0, -5.0]),
                Command::Quad([0.0, -10.0], [5.0, -5.0]),
                Command::Close,
            ]
        );
    }

    #[test]
    fn short_and_repeated_coordinates_decode() {
        // Four points: a repeated flag for the short positive x deltas, one
        // byte negative and repeated y values.
        let mut glyph = 1i16.to_be_bytes().to_vec();
        glyph.extend([0; 8]);
        glyph.extend(3u16.to_be_bytes());
        glyph.extend([0, 0]);
        let on = ON_CURVE_POINT;
        glyph.extend([
            on | X_SHORT_VECTOR | X_IS_SAME_OR_POSITIVE | Y_IS_SAME_OR_POSITIVE | REPEAT_FLAG,
            1,
            on | X_IS_SAME_OR_POSITIVE | Y_SHORT_VECTOR,
            on | X_SHORT_VECTOR | Y_IS_SAME_OR_POSITIVE,
        ]);
        glyph.extend([10, 20]);
        glyph.extend([5]);
        glyph.extend([7]);
        let recorder = record(&[glyph], 0).unwrap();
        assert_eq!(
            recorder.commands,
            [
                Command::Move([10.0, 0.0]),
                Command::Line([30.0, 0.0]),
                Command::Line([30.0, -7.0]),
                Command::Line([25.0, -7.0]),
                Command::Close,
            ]
        );
    }

    #[test]
    fn composite_glyphs_offset_and_transform_their_components() {
        let triangle = simple_glyph(&[&[(0, 0, true), (10, 0, true), (0, 10, true)]]);
        let mut arguments = Vec::new();
        arguments.extend(100i16.to_be_bytes());
        arguments.extend((-50i16).to_be_bytes());
        arguments.extend(0x2000i16.to_be_bytes());
        let scaled = (
            0,
            ARG_1_AND_2_ARE_WORDS | ARGS_ARE_XY_VALUES | WE_HAVE_A_SCALE,
            arguments,
        );
        let mut arguments = vec![5u8, 0xFB];
        arguments.extend(0x4000i16.to_be_bytes());
        arguments.extend(0i16.to_be_bytes());
        arguments.extend(0x2000i16.to_be_bytes());
        arguments.extend(0x4000i16.to_be_bytes());
        let sheared = (0, ARGS_ARE_XY_VALUES | WE_HAVE_A_TWO_BY_TWO, arguments);
        let mut arguments = vec![1u8, 2];
        arguments.extend(0x8000u16.to_be_bytes());
        arguments.extend(0x4000u16.to_be_bytes());
        let mirrored = (
            0,
            ARGS_ARE_XY_VALUES | WE_HAVE_AN_X_AND_Y_SCALE | SCALED_COMPONENT_OFFSET,
            arguments,
        );
        let composite = composite_glyph(&[scaled, sheared, mirrored]);
        let recorder = record(&[triangle, composite], 1).unwrap();
        let moves: Vec<_> = recorder
            .commands
            .iter()
            .filter(|command| !matches!(command, Command::Close))
            .copied()
            .collect();
        assert_eq!(
            moves,
            [
                Command::Move([100.0, -50.0]),
                Command::Line([105.0, -50.0]),
                Command::Line([100.0, -45.0]),
                Command::Move([5.0, -5.0]),
                Command::Line([15.0, -5.0]),
                Command::Line([10.0, 5.0]),
                Command::Move([-2.0, 2.0]),
                Command::Line([-22.0, 2.0]),
                Command::Line([-2.0, 12.0]),
            ]
        );
    }

    #[test]
    fn the_unscaled_offset_flag_wins() {
        let triangle = simple_glyph(&[&[(0, 0, true), (10, 0, true), (0, 10, true)]]);
        let mut arguments = vec![3u8, 4];
        arguments.extend(0x8000u16.to_be_bytes());
        let flags = ARGS_ARE_XY_VALUES | WE_HAVE_A_SCALE;
        let both = composite_glyph(&[(
            0,
            flags | SCALED_COMPONENT_OFFSET | UNSCALED_COMPONENT_OFFSET,
            arguments.clone(),
        )]);
        let scaled = composite_glyph(&[(0, flags | SCALED_COMPONENT_OFFSET, arguments)]);
        let glyphs = [triangle, both, scaled];
        assert_eq!(
            record(&glyphs, 1).unwrap().commands[0],
            Command::Move([3.0, 4.0])
        );
        assert_eq!(
            record(&glyphs, 2).unwrap().commands[0],
            Command::Move([-6.0, -8.0])
        );
    }

    #[test]
    fn the_points_of_an_outline_are_limited() {
        let points: Vec<_> = (0..4096).map(|index| (index as i16, 0, true)).collect();
        let mut glyphs = vec![simple_glyph(&[&points])];
        let components: Vec<_> = (0..16)
            .map(|_| (0, ARGS_ARE_XY_VALUES, vec![0, 0]))
            .collect();
        glyphs.push(composite_glyph(&components));
        glyphs.push(composite_glyph(&components[..15]));
        glyphs.push(composite_glyph(&[
            (2, ARGS_ARE_XY_VALUES, vec![0, 0]),
            (0, ARGS_ARE_XY_VALUES, vec![0, 0]),
            (0, ARGS_ARE_XY_VALUES, vec![0, 0]),
        ]));
        assert!(record(&glyphs, 1).is_ok());
        assert!(record(&glyphs, 2).is_ok());
        assert_eq!(record(&glyphs, 3).err(), Some(FontError::OutlineLimit));
    }

    #[test]
    fn nested_composites_combine_their_transforms() {
        let dot = simple_glyph(&[&[(0, 0, true), (1, 0, true), (0, 1, true)]]);
        let offset = |x: i8, y: i8| (0, ARGS_ARE_XY_VALUES, vec![x as u8, y as u8]);
        let inner = composite_glyph(&[offset(10, 0), offset(0, 20)]);
        let mut arguments = vec![3u8, 4];
        arguments.extend(0x8000u16.to_be_bytes());
        let outer = composite_glyph(&[(1, ARGS_ARE_XY_VALUES | WE_HAVE_A_SCALE, arguments)]);
        let recorder = record(&[dot, inner, outer], 2).unwrap();
        assert_eq!(recorder.contours(), 2);
        assert_eq!(recorder.commands[0], Command::Move([-17.0, 4.0]));
        assert_eq!(recorder.commands[4], Command::Move([3.0, -36.0]));
    }

    #[test]
    fn recursive_composites_hit_the_limit() {
        let own = composite_glyph(&[(0, ARGS_ARE_XY_VALUES, vec![0, 0])]);
        assert_eq!(record(&[own], 0).err(), Some(FontError::OutlineLimit));

        let dot = simple_glyph(&[&[(0, 0, true), (1, 0, true), (0, 1, true)]]);
        let mut glyphs = vec![dot];
        for level in 0..6 {
            let components: Vec<_> = (0..8)
                .map(|_| (level as u16, ARGS_ARE_XY_VALUES, vec![0, 0]))
                .collect();
            glyphs.push(composite_glyph(&components));
        }
        assert!(record(&glyphs, 3).is_ok());
        assert_eq!(record(&glyphs, 6).err(), Some(FontError::OutlineLimit));
    }

    #[test]
    fn broken_glyph_data_fails() {
        let square = simple_glyph(&[&[(0, 0, true), (100, 0, true), (100, 100, true)]]);
        for length in 1..square.len() {
            assert!(record(&[square[..length].to_vec()], 0).is_err());
        }
        let mut reversed = simple_glyph(&[&[(0, 0, true)], &[(1, 1, true), (2, 2, true)]]);
        reversed[10..14].copy_from_slice(&[0, 2, 0, 1]);
        assert_eq!(
            record(&[reversed], 0).err(),
            Some(FontError::InvalidTable(Tag::GLYF))
        );
        let missing = composite_glyph(&[(5, ARGS_ARE_XY_VALUES, vec![0, 0])]);
        assert_eq!(
            record(&[missing], 0).err(),
            Some(FontError::UnknownGlyph(GlyphId(5)))
        );
        assert_eq!(
            record(&[], 0).err(),
            Some(FontError::UnknownGlyph(GlyphId(0)))
        );
        assert_eq!(
            Outlines::new(&[0, 0], &[], false, 1).err(),
            Some(FontError::InvalidTable(Tag::LOCA))
        );
    }

    #[test]
    fn short_offsets_count_in_words() {
        let mut square = simple_glyph(&[&[(0, 0, true), (10, 0, true), (10, 10, true)]]);
        square.resize(square.len().next_multiple_of(2), 0);
        let mut glyf = square.clone();
        glyf.extend(&square);
        let words = (square.len() / 2) as u16;
        let mut loca = Vec::new();
        for offset in [0, words, words * 2] {
            loca.extend(offset.to_be_bytes());
        }
        let outlines = Outlines::new(&loca, &glyf, false, 2).unwrap();
        let mut recorder = Recorder::default();
        outlines.draw(1, &mut recorder).unwrap();
        assert_eq!(recorder.contours(), 1);
    }
}
