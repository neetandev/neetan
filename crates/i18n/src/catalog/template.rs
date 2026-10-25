//! The walk over the tokens of a value that measures a text or writes it into a catalog.

use super::{Form, Segment, Span, offset};
use crate::{
    catalog::COUNT_PLACEHOLDER,
    parse::{FaultKind, Token, Value},
    schema::KeyInfo,
};

/// The size of texts: the bytes of the text storage and the number of segments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Size {
    /// The bytes of the text storage.
    pub(super) bytes: u32,
    /// The number of segments.
    pub(super) segments: u32,
}

/// The receiver of the parts of a text.
trait Sink {
    /// Receives literal text.
    fn text(&mut self, text: &str);
    /// Receives a placeholder.
    fn argument(&mut self, segment: Segment);
}

/// Walks the tokens of a value and resolves its placeholders to the arguments of a key. A value
/// without a key accepts no placeholder.
fn walk<'a>(
    value: Value<'a>,
    info: Option<&KeyInfo>,
    sink: &mut impl Sink,
) -> Result<(), FaultKind<'a>> {
    let mut buffer = [0; 4];
    for token in value.tokens() {
        match token? {
            Token::Text(text) => sink.text(text),
            Token::Character(character) => sink.text(character.encode_utf8(&mut buffer)),
            Token::Placeholder(name) => {
                let segment = info
                    .and_then(|info| argument_segment(info, name))
                    .ok_or(FaultKind::UnknownPlaceholder(name))?;
                sink.argument(segment);
            }
        }
    }
    Ok(())
}

/// Returns the segment of a placeholder of a key.
fn argument_segment(info: &KeyInfo, name: &str) -> Option<Segment> {
    if info.is_plural() && name == COUNT_PLACEHOLDER {
        return Some(Segment::Count);
    }
    info.placeholder_index(name).map(Segment::Argument)
}

/// Measures the text of a value. A value without a key accepts no placeholder.
pub(super) fn measure<'a>(value: Value<'a>, info: Option<&KeyInfo>) -> Result<Size, FaultKind<'a>> {
    let mut measure = Measure::default();
    walk(value, info, &mut measure)?;
    Ok(measure.size())
}

/// Writes a checked value into the text storage and the segment list and returns its form.
pub(super) fn write_form(
    value: Value<'_>,
    info: Option<&KeyInfo>,
    storage: &mut String,
    segments: &mut Vec<Segment>,
) -> Form {
    let text_start = offset(storage.len());
    let segment_start = offset(segments.len());
    let mut writer = Writer {
        storage,
        segments,
        literal_start: None,
        has_argument: false,
    };
    let result = walk(value, info, &mut writer);
    debug_assert!(result.is_ok());
    if writer.has_argument {
        writer.close_literal();
    }
    Form {
        text: Span {
            start: text_start,
            end: offset(writer.storage.len()),
        },
        segments: Span {
            start: segment_start,
            end: offset(writer.segments.len()),
        },
    }
}

/// The sink that measures a text.
#[derive(Clone, Copy, Debug, Default)]
struct Measure {
    /// The bytes of the literal text.
    bytes: u32,
    /// The number of literal runs between the placeholders.
    runs: u32,
    /// The number of placeholders.
    arguments: u32,
    /// True while the last part was literal text.
    in_literal: bool,
}

impl Measure {
    /// Returns the size of the text. A text without placeholders has no segments.
    fn size(&self) -> Size {
        Size {
            bytes: self.bytes,
            segments: if self.arguments == 0 {
                0
            } else {
                self.runs + self.arguments
            },
        }
    }
}

impl Sink for Measure {
    fn text(&mut self, text: &str) {
        self.bytes += offset(text.len());
        if !self.in_literal {
            self.runs += 1;
            self.in_literal = true;
        }
    }

    fn argument(&mut self, _segment: Segment) {
        self.arguments += 1;
        self.in_literal = false;
    }
}

/// The sink that writes a text.
struct Writer<'s> {
    /// The text storage.
    storage: &'s mut String,
    /// The segment list.
    segments: &'s mut Vec<Segment>,
    /// The start of the current literal run.
    literal_start: Option<u32>,
    /// True after the first placeholder.
    has_argument: bool,
}

impl Writer<'_> {
    /// Ends the current literal run with a literal segment.
    fn close_literal(&mut self) {
        if let Some(start) = self.literal_start.take() {
            self.segments.push(Segment::Literal(Span {
                start,
                end: offset(self.storage.len()),
            }));
        }
    }
}

impl Sink for Writer<'_> {
    fn text(&mut self, text: &str) {
        if self.literal_start.is_none() {
            self.literal_start = Some(offset(self.storage.len()));
        }
        self.storage.push_str(text);
    }

    fn argument(&mut self, segment: Segment) {
        self.close_literal();
        self.segments.push(segment);
        self.has_argument = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plural text with the placeholder `{folder}`.
    static INFO: KeyInfo = KeyInfo::new("manager", "images", &["folder"], true);

    /// Writes a value of [`INFO`] behind existing text and segments. Returns the storage, the
    /// segments and the form.
    fn written(raw: &str) -> (String, Vec<Segment>, Form) {
        let mut storage = String::from("ab");
        let mut segments = vec![Segment::Count];
        let form = write_form(Value::new(raw), Some(&INFO), &mut storage, &mut segments);
        (storage, segments, form)
    }

    #[test]
    fn a_fixed_text_has_no_segments() {
        let (storage, segments, form) = written(r"x\ny {{z}}");
        assert_eq!(storage, "abx\ny {z}");
        assert_eq!(segments, vec![Segment::Count]);
        assert_eq!(form.text, Span { start: 2, end: 9 });
        assert!(form.segments.is_empty());
        assert_eq!(
            measure(Value::new(r"x\ny {{z}}"), Some(&INFO)),
            Ok(Size {
                bytes: 7,
                segments: 0
            })
        );
    }

    #[test]
    fn a_template_joins_its_literal_runs() {
        let raw = r"{count} \;in{{ {folder}";
        let (storage, segments, form) = written(raw);
        assert_eq!(storage, "ab ;in{ ");
        assert_eq!(
            segments,
            vec![
                Segment::Count,
                Segment::Count,
                Segment::Literal(Span { start: 2, end: 8 }),
                Segment::Argument(0),
            ]
        );
        assert_eq!(form.text, Span { start: 2, end: 8 });
        assert_eq!(form.segments, Span { start: 1, end: 4 });
        assert_eq!(
            measure(Value::new(raw), Some(&INFO)),
            Ok(Size {
                bytes: 6,
                segments: 3
            })
        );
    }

    #[test]
    fn unknown_placeholders_fail_the_measure() {
        assert_eq!(
            measure(Value::new("{name}"), Some(&INFO)),
            Err(FaultKind::UnknownPlaceholder("name"))
        );
        assert_eq!(
            measure(Value::new("{folder}"), None),
            Err(FaultKind::UnknownPlaceholder("folder"))
        );
    }
}
