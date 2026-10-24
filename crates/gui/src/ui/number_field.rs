//! The number field: a text field for a number with two step buttons.

use super::{
    context::{UiContext, UiId},
    focus::Navigation,
    icons::Icon,
    label::Truncation,
    text_field::{
        FieldLook, TEXT_FIELD_LIMIT, TextFieldResponse, draw_value, edit_text, field_frame,
    },
};
use crate::Rect;

/// The width of every number field.
pub const NUMBER_FIELD_WIDTH: f32 = 128.0;

/// The tolerance of the comparisons of decimal numbers in the reach of a
/// text.
const TOLERANCE: f64 = 1e-9;

/// The size of a step button inside the number field.
const STEPPER_SIZE: f32 = 24.0;

/// The reason why the text of a [`NumberField`] holds no number of its
/// range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberError {
    /// The text is empty, for a key that the file does not set.
    Empty,
    /// The text is not a number in the form that typing produces.
    Invalid,
    /// The number lies below the minimum.
    BelowMinimum,
    /// The number lies above the maximum.
    AboveMaximum,
}

/// A field that edits a number as text, with a step-down and a step-up
/// button at its end.
///
/// The field takes typed digits, a decimal point when the number has
/// decimals and a minus sign at the start when the range holds negative
/// numbers. A digit that leads past the maximum is dropped, and so is a
/// digit that leads below the minimum after a minus sign. Left and right
/// and the step buttons step the number down and up by the step, inside the
/// range. A step from an empty field starts at the fallback. A step button
/// at the end of the range is disabled. An empty field shows the
/// placeholder, for a key that the file does not set. A text that holds no
/// number of the range draws the error edge, see [`NumberField::parse`].
/// The field never takes cancel.
#[derive(Clone, Copy, Debug)]
pub struct NumberField<'a> {
    /// The id of the field.
    id: UiId,
    /// The text of an empty field.
    placeholder: &'a str,
    /// The smallest number.
    minimum: f64,
    /// The largest number.
    maximum: f64,
    /// The distance of one step.
    step: f64,
    /// The digits after the decimal point.
    decimals: usize,
    /// The number a step from an empty field starts at.
    fallback: Option<f64>,
    /// Whether the field draws the error edge.
    error: bool,
    /// Whether the field reacts to input.
    enabled: bool,
}

impl<'a> NumberField<'a> {
    /// A field for whole numbers from 0 up that shows `placeholder` while it
    /// is empty.
    pub fn new(id: impl Into<UiId>, placeholder: &'a str) -> Self {
        Self {
            id: id.into(),
            placeholder,
            minimum: 0.0,
            maximum: f64::from(u32::MAX),
            step: 1.0,
            decimals: 0,
            fallback: None,
            error: false,
            enabled: true,
        }
    }

    /// The same field for numbers from `minimum` to `maximum`.
    pub fn range(self, minimum: f64, maximum: f64) -> Self {
        Self {
            minimum,
            maximum: maximum.max(minimum),
            ..self
        }
    }

    /// The same field with the step `step`.
    pub fn step(self, step: f64) -> Self {
        Self { step, ..self }
    }

    /// The same field with `decimals` digits after the decimal point.
    pub fn decimals(self, decimals: usize) -> Self {
        Self { decimals, ..self }
    }

    /// The same field, which starts a step from an empty field at
    /// `fallback`, such as the inherited value. Without a fallback a step
    /// starts at the minimum.
    pub fn fallback(self, fallback: Option<f64>) -> Self {
        Self { fallback, ..self }
    }

    /// The same field with the error edge when `error` is set.
    pub fn error(self, error: bool) -> Self {
        Self { error, ..self }
    }

    /// The same field, disabled unless `enabled`. A disabled field draws
    /// dimmed, ignores input and leaves the focus ring.
    pub fn enabled(self, enabled: bool) -> Self {
        Self { enabled, ..self }
    }

    /// The number of `text` in the form that typing produces: an optional
    /// minus sign for a range with negative numbers, digits without a
    /// leading zero and, for a number with decimals, a decimal point with at
    /// most that many digits after it. `None` for an empty text or any other form, such as a plus
    /// sign or an exponent.
    fn number(&self, text: &str) -> Option<f64> {
        let unsigned = match text.strip_prefix('-') {
            Some(_) if self.minimum >= 0.0 => return None,
            Some(rest) => rest,
            None => text,
        };
        let (whole, fraction) = match unsigned.split_once('.') {
            Some(_) if self.decimals == 0 => return None,
            Some((whole, fraction)) => (whole, fraction),
            None => (unsigned, ""),
        };
        let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
        let leading_zero = whole.len() > 1 && whole.starts_with('0');
        if whole.is_empty()
            || leading_zero
            || !digits(whole)
            || !digits(fraction)
            || fraction.len() > self.decimals
        {
            return None;
        }
        text.parse::<f64>().ok().filter(|value| value.is_finite())
    }

    /// The number of `text` inside the range, or the reason why `text`
    /// holds none.
    pub fn parse(&self, text: &str) -> Result<f64, NumberError> {
        if text.is_empty() {
            return Err(NumberError::Empty);
        }
        let value = self.number(text).ok_or(NumberError::Invalid)?;
        if value < self.minimum {
            Err(NumberError::BelowMinimum)
        } else if value > self.maximum {
            Err(NumberError::AboveMaximum)
        } else {
            Ok(value)
        }
    }

    /// Whether the field flags `text` with the error edge. Without the
    /// focus every text that holds no number of the range is flagged. With
    /// the focus a text is flagged when no continuation that typing can
    /// produce reaches the range, see [`NumberField::reachable`].
    fn flagged(&self, text: &str, focused: bool) -> bool {
        if text.is_empty() {
            false
        } else if focused {
            !self.reachable(text)
        } else {
            self.parse(text).is_err()
        }
    }

    /// Whether typing more characters after `text` can produce a number of
    /// the range.
    ///
    /// The continuations of a text cover intervals of magnitudes. A text
    /// with a decimal point and `f` digits after it covers the magnitudes
    /// from its value up to the last digit that the decimals allow. A whole
    /// number `n` covers the magnitudes `n * 10^k` to `(n + 1) * 10^k - u`
    /// for `k` more digits, where `u` is the smallest step of the decimals.
    /// A lone zero takes no more digits before its decimal point. The text
    /// reaches the range when one of its intervals meets the magnitudes of
    /// the range with its sign.
    fn reachable(&self, text: &str) -> bool {
        let (negative, unsigned) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        if !unsigned.is_empty() && self.number(text).is_none() {
            return false;
        }
        if negative && self.minimum >= 0.0 {
            return false;
        }
        let (low, high) = if negative {
            ((-self.maximum).max(0.0), -self.minimum)
        } else {
            (self.minimum.max(0.0), self.maximum)
        };
        if low > high {
            return false;
        }
        let unit = 10f64.powi(-(self.decimals as i32));
        let meets = |from: f64, to: f64| from <= high + TOLERANCE && to >= low - TOLERANCE;
        if unsigned.is_empty() {
            return true;
        }
        let value: f64 = unsigned.parse().unwrap_or(0.0);
        if let Some((_, fraction)) = unsigned.split_once('.') {
            let free = 10f64.powi(-(fraction.len() as i32)) - unit;
            return meets(value, value + free.max(0.0));
        }
        let fraction_span = if self.decimals > 0 { 1.0 - unit } else { 0.0 };
        if value == 0.0 {
            return meets(0.0, fraction_span);
        }
        let mut scale = 1.0;
        while value * scale <= high + TOLERANCE {
            let top = (value + 1.0) * scale - 1.0 + fraction_span;
            if meets(value * scale, top) {
                return true;
            }
            scale *= 10.0;
        }
        false
    }

    /// The text of `value` with the digits after the decimal point.
    fn format(&self, value: f64) -> String {
        format!("{value:.precision$}", precision = self.decimals)
    }

    /// `text` after one step down or up. A step from an empty or broken
    /// text starts at the fallback or the minimum.
    fn stepped(&self, text: &str, up: bool) -> String {
        let value = match self.number(text) {
            Some(value) if up => value + self.step,
            Some(value) => value - self.step,
            None => self.fallback.unwrap_or(self.minimum),
        };
        self.format(value.clamp(self.minimum, self.maximum))
    }

    /// Whether `text` takes the typed `character`. A digit that leads past
    /// the maximum, or below the minimum after a minus sign, is dropped.
    fn accepts(&self, text: &str, character: char) -> bool {
        match character {
            '0'..='9' => {
                let candidate = format!("{text}{character}");
                self.number(&candidate).is_some_and(|value| {
                    if candidate.starts_with('-') {
                        value >= self.minimum
                    } else {
                        value <= self.maximum
                    }
                })
            }
            '.' => {
                self.decimals > 0
                    && !text.contains('.')
                    && text
                        .bytes()
                        .last()
                        .is_some_and(|byte| byte.is_ascii_digit())
            }
            '-' => self.minimum < 0.0 && text.is_empty(),
            _ => false,
        }
    }

    /// Draws the field in `rect` over `text` and returns what it did in this
    /// pass.
    pub fn show(self, context: &mut UiContext, rect: Rect, text: &mut String) -> TextFieldResponse {
        let id = self.id;
        let theme = context.theme;
        let mut response = TextFieldResponse::default();
        let steppers_right = rect.right() - theme.gap * 0.5;
        let step_top = rect.y + (rect.height - STEPPER_SIZE) * 0.5;
        let down_rect = Rect::new(
            steppers_right - STEPPER_SIZE * 2.0,
            step_top,
            STEPPER_SIZE,
            STEPPER_SIZE,
        );
        let up_rect = Rect::new(
            steppers_right - STEPPER_SIZE,
            step_top,
            STEPPER_SIZE,
            STEPPER_SIZE,
        );
        let value = self.number(text);
        let can_step_down = self.enabled && value.is_none_or(|value| value > self.minimum);
        let can_step_up = self.enabled && value.is_none_or(|value| value < self.maximum);
        let mut hot = [false; 2];
        for (index, (part, area, allowed, up)) in [
            ("down", down_rect, can_step_down, false),
            ("up", up_rect, can_step_up, true),
        ]
        .into_iter()
        .enumerate()
        {
            if !allowed {
                continue;
            }
            let pointer = context.button_behavior(id.child(part), area);
            hot[index] = pointer.hovered || pointer.held;
            if pointer.clicked {
                *text = self.stepped(text, up);
                response.changed = true;
                context.set_focus(id);
            }
        }
        let mut hovered = false;
        if self.enabled {
            if context.has_focus(id) {
                response.changed |=
                    edit_text(context, text, TEXT_FIELD_LIMIT, |text, character| {
                        self.accepts(text, character)
                    });
                let step = match context.navigation(id) {
                    Some(Navigation::Left) => can_step_down.then_some(false),
                    Some(Navigation::Right) => can_step_up.then_some(true),
                    Some(
                        Navigation::Up
                        | Navigation::Down
                        | Navigation::PageUp
                        | Navigation::PageDown
                        | Navigation::Home
                        | Navigation::End
                        | Navigation::Next
                        | Navigation::Previous
                        | Navigation::PreviousTab
                        | Navigation::NextTab,
                    )
                    | None => None,
                };
                if let Some(up) = step {
                    *text = self.stepped(text, up);
                    response.changed = true;
                }
                response.submitted = context.confirmed(id);
            }
            context.text_focusable(id, rect, false);
            hovered = context.button_behavior(id, rect).hovered;
        }
        let focused = self.enabled && context.has_focus(id);
        let invalid = self.flagged(text, focused);
        let look = FieldLook {
            focused,
            hovered,
            error: self.error || invalid,
            enabled: self.enabled,
            read_only: false,
        };
        field_frame(context, rect, look);
        let left = rect.x + theme.gap * 1.25;
        let right = down_rect.x - theme.gap;
        let inner = Rect::new(left, rect.y, (right - left).max(0.0), rect.height);
        let color = look.text_color(context);
        draw_value(
            context,
            inner,
            text,
            self.placeholder,
            focused,
            color,
            Truncation::End,
        );
        let value = self.number(text);
        let can_step_down = self.enabled && value.is_none_or(|value| value > self.minimum);
        let can_step_up = self.enabled && value.is_none_or(|value| value < self.maximum);
        for (area, icon, allowed, hot) in [
            (down_rect, Icon::Down, can_step_down, hot[0]),
            (up_rect, Icon::Up, can_step_up, hot[1]),
        ] {
            let color = if !allowed {
                theme.text_disabled
            } else if hot {
                context
                    .scene
                    .fill(area, theme.radius * 2.0 / 3.0, theme.control_hover);
                theme.text
            } else {
                theme.text_dim
            };
            context.icon(area, icon, theme.icon_size, color);
        }
        if focused {
            context.focus_ring(rect, theme.radius);
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        NavigationSet,
        tests::{Harness, pixel, srgb},
    };

    /// The area of the field.
    const FIELD: Rect = Rect::new(10.0, 10.0, NUMBER_FIELD_WIDTH, 32.0);

    /// The field of the PC-6000 color phase from 0 to 3.
    fn phase() -> NumberField<'static> {
        NumberField::new("phase", "Not set").range(0.0, 3.0)
    }

    /// Draws `field` over `text` with a neighbor on each side.
    fn draw(harness: &mut Harness, field: NumberField, text: &mut String) -> TextFieldResponse {
        harness.frame(|context| {
            let response = field.show(context, FIELD, text);
            for (index, x) in [-200.0, 300.0].into_iter().enumerate() {
                context.focusable(
                    UiId::from("neighbor").indexed(index),
                    Rect::new(x, FIELD.y, 100.0, FIELD.height),
                    NavigationSet::EMPTY,
                );
            }
            response
        })
    }

    #[test]
    fn left_and_right_step_inside_the_range() {
        let mut harness = Harness::new();
        let mut text = String::from("2");
        harness.state.set_focus(UiId::from("phase"));
        draw(&mut harness, phase(), &mut text);
        harness.navigate(Navigation::Right);
        assert!(draw(&mut harness, phase(), &mut text).changed);
        assert_eq!(text, "3");
        harness.navigate(Navigation::Right);
        assert!(!draw(&mut harness, phase(), &mut text).changed);
        assert_eq!(text, "3");
        assert_eq!(harness.state.focused(), Some(UiId::from("phase")));
        for _ in 0..5 {
            harness.navigate(Navigation::Left);
            draw(&mut harness, phase(), &mut text);
        }
        assert_eq!(text, "0");
        assert_eq!(harness.state.focused(), Some(UiId::from("phase")));
    }

    #[test]
    fn digits_type_and_other_characters_are_dropped() {
        let mut harness = Harness::new();
        let mut text = String::new();
        let year = NumberField::new("phase", "Not set").range(0.0, 9999.0);
        harness.state.set_focus(UiId::from("phase"));
        draw(&mut harness, year, &mut text);
        assert!(harness.state.wants_text_input());
        harness.input.text.push_str("19a9-6.5");
        draw(&mut harness, year, &mut text);
        assert_eq!(text, "1996");
        harness.input.text.push('7');
        assert!(!draw(&mut harness, year, &mut text).changed);
        harness.input.backspace = true;
        draw(&mut harness, year, &mut text);
        assert_eq!(text, "199");

        let mut volume = String::new();
        let field = NumberField::new("phase", "Not set")
            .range(0.0, 1.0)
            .step(0.1)
            .decimals(1);
        harness.input.text.push_str("0.75");
        draw(&mut harness, field, &mut volume);
        assert_eq!(volume, "0.7");
        harness.navigate(Navigation::Right);
        draw(&mut harness, field, &mut volume);
        assert_eq!(volume, "0.8");
    }

    #[test]
    fn a_step_from_an_empty_field_starts_at_the_fallback() {
        let mut harness = Harness::new();
        let mut text = String::new();
        let field = phase().fallback(Some(2.0));
        harness.state.set_focus(UiId::from("phase"));
        draw(&mut harness, field, &mut text);
        harness.navigate(Navigation::Right);
        draw(&mut harness, field, &mut text);
        assert_eq!(text, "2");
        let mut text = String::new();
        harness.navigate(Navigation::Left);
        draw(&mut harness, phase(), &mut text);
        assert_eq!(text, "0");
    }

    #[test]
    fn the_step_buttons_step_and_focus_the_field() {
        let mut harness = Harness::new();
        let mut text = String::from("1");
        let up = (FIELD.right() - 4.0 - STEPPER_SIZE * 0.5, FIELD.center().1);
        draw(&mut harness, phase(), &mut text);
        harness.press(up.0, up.1);
        draw(&mut harness, phase(), &mut text);
        harness.release();
        assert!(draw(&mut harness, phase(), &mut text).changed);
        assert_eq!(text, "2");
        assert_eq!(harness.state.focused(), Some(UiId::from("phase")));
    }

    #[test]
    fn cancel_passes_the_field() {
        let mut harness = Harness::new();
        let mut text = String::from("1");
        harness.state.set_focus(UiId::from("phase"));
        draw(&mut harness, phase(), &mut text);
        harness.input.cancel = true;
        let cancelled = harness.frame(|context| {
            phase().show(context, FIELD, &mut text);
            context.cancelled()
        });
        assert!(cancelled);
        assert_eq!(text, "1");
    }

    #[test]
    fn a_disabled_field_ignores_input_and_draws_dimmed() {
        let mut harness = Harness::new();
        let mut text = String::from("1996");
        let field = NumberField::new("year", "Not set").enabled(false);
        harness.navigate(Navigation::Next);
        harness.frame(|context| field.show(context, FIELD, &mut text));
        assert_eq!(harness.state.focused(), None);
        let pixels = harness.render();
        let inside = pixel(
            &pixels,
            harness.viewport.width,
            FIELD.x + 4.0,
            FIELD.center().1,
        );
        assert!(inside[3] < 255, "{inside:?}");
    }

    #[test]
    fn a_hovered_step_draws_its_fill() {
        let mut harness = Harness::new();
        let mut text = String::from("1");
        let up = (FIELD.right() - 4.0 - STEPPER_SIZE * 0.5, FIELD.center().1);
        harness.mouse(up.0, up.1);
        draw(&mut harness, phase(), &mut text);
        let pixels = harness.render();
        let corner = pixel(
            &pixels,
            harness.viewport.width,
            up.0 - STEPPER_SIZE * 0.5 + 3.0,
            up.1 - STEPPER_SIZE * 0.5 + 3.0,
        );
        assert_eq!(corner, srgb(harness.theme.control_hover));
    }

    #[test]
    fn parse_reports_the_value_or_the_reason() {
        let field = NumberField::new("volume", "Not set")
            .range(0.0, 1.0)
            .decimals(1);
        assert_eq!(field.parse("0.5"), Ok(0.5));
        assert_eq!(field.parse("1"), Ok(1.0));
        assert_eq!(field.parse(""), Err(NumberError::Empty));
        assert_eq!(field.parse("2"), Err(NumberError::AboveMaximum));
        for broken in [
            "+1", "1e0", "0.55", ".5", "-0.5", "inf", "NaN", " 1", "1 ", "0x1",
        ] {
            assert_eq!(field.parse(broken), Err(NumberError::Invalid), "{broken}");
        }
        let year = NumberField::new("year", "Not set").range(1980.0, 2000.0);
        assert_eq!(year.parse("199"), Err(NumberError::BelowMinimum));
        assert_eq!(year.parse("1996.0"), Err(NumberError::Invalid));
        let signed = NumberField::new("offset", "Not set").range(-3.0, 3.0);
        assert_eq!(signed.parse("-3"), Ok(-3.0));
        assert_eq!(signed.parse("-4"), Err(NumberError::BelowMinimum));
        assert_eq!(signed.parse("-"), Err(NumberError::Invalid));
    }

    #[test]
    fn negative_ranges_type_symmetrically() {
        let mut harness = Harness::new();
        let field = NumberField::new("phase", "Not set").range(-3.0, 3.0);
        harness.state.set_focus(UiId::from("phase"));
        let mut text = String::new();
        draw(&mut harness, field, &mut text);
        harness.input.text.push_str("-9");
        draw(&mut harness, field, &mut text);
        assert_eq!(text, "-");
        harness.input.text.push_str("2+e");
        draw(&mut harness, field, &mut text);
        assert_eq!(text, "-2");
        harness.navigate(Navigation::Left);
        draw(&mut harness, field, &mut text);
        assert_eq!(text, "-3");
        let mut text = String::new();
        harness.input.text.push_str("9-1");
        draw(&mut harness, field, &mut text);
        assert_eq!(text, "-1");
    }

    #[test]
    fn a_focused_field_flags_only_a_text_that_typing_cannot_bring_into_range() {
        let teens = NumberField::new("age", "Not set").range(10.0, 20.0);
        assert!(teens.flagged("3", true));
        assert!(!teens.flagged("2", true));
        assert!(teens.flagged("21", true));
        assert!(!teens.flagged("1", true));
        assert!(teens.flagged("1", false));
        assert!(!teens.flagged("", false));

        let pair = NumberField::new("ratio", "Not set")
            .range(1.0, 2.0)
            .decimals(1);
        assert!(pair.flagged("0", true));
        assert!(pair.flagged("0.5", true));
        assert!(!pair.flagged("1.", true));
        assert!(!pair.flagged("1.5", true));
        assert!(pair.flagged("2.5", true));

        let volume = NumberField::new("volume", "Not set")
            .range(0.0, 1.0)
            .decimals(1);
        assert!(!volume.flagged("0", true));
        assert!(!volume.flagged("0.", true));
        assert!(volume.flagged("01", true));

        let negative = NumberField::new("depth", "Not set").range(-20.0, -10.0);
        assert!(negative.flagged("-5", true));
        assert!(!negative.flagged("-", true));
        assert!(!negative.flagged("-2", true));
        assert!(!negative.flagged("-1", true));
        assert!(negative.flagged("-30", true));
        assert!(negative.flagged("1", true));
        assert!(negative.flagged("-2", false));

        let signed = NumberField::new("offset", "Not set").range(-3.0, 3.0);
        assert!(!signed.flagged("-", true));
        assert!(!signed.flagged("-3", true));
        assert!(signed.flagged("-5", true));
        assert!(signed.flagged("4", true));
        assert!(signed.flagged("+1", true));
        assert!(signed.flagged("-", false));

        let year = NumberField::new("year", "Not set").range(1980.0, 2000.0);
        assert!(!year.flagged("19", true));
        assert!(!year.flagged("2", true));
        assert!(year.flagged("3", true));
        assert!(year.flagged("2001", true));
    }

    #[test]
    fn a_focused_field_typing_a_valid_number_keeps_its_edge() {
        let mut harness = Harness::new();
        let mut text = String::from("19");
        let field = NumberField::new("year", "Not set").range(1980.0, 2000.0);
        harness.state.set_focus(UiId::from("year"));
        harness.frame(|context| field.show(context, FIELD, &mut text));
        let pixels = harness.render();
        let edge = pixel(
            &pixels,
            harness.viewport.width,
            FIELD.x + 30.0,
            FIELD.y + 0.5,
        );
        assert_eq!(edge, srgb(harness.theme.accent));
    }

    #[test]
    fn a_text_outside_the_range_draws_the_error_edge() {
        let theme = Harness::new().theme;
        for (text, edge) in [
            ("1", theme.danger_text),
            ("15", theme.control_edge),
            ("", theme.control_edge),
        ] {
            let mut harness = Harness::new();
            let mut text = text.to_owned();
            let field = NumberField::new("year", "Not set").range(10.0, 20.0);
            harness.frame(|context| field.show(context, FIELD, &mut text));
            let pixels = harness.render();
            let width = harness.viewport.width;
            assert_eq!(
                pixel(&pixels, width, FIELD.x + 30.0, FIELD.y + 0.5),
                srgb(edge),
                "{text}"
            );
        }
    }
}
