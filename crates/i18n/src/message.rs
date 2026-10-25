//! The texts of a catalog: fixed texts, messages with placeholders and lists.

use core::fmt;

use crate::{
    catalog::{Catalog, Form, Segment},
    schema::Key,
};

/// A text with its arguments in the argument order of the schema. A plural message also holds
/// the count that selects the plural form.
#[must_use]
#[derive(Clone, Copy)]
pub struct Message<'a, const N: usize> {
    /// The key of the text.
    key: Key,
    /// The count of a plural text.
    count: Option<u64>,
    /// The arguments in the argument order of the schema.
    arguments: [&'a dyn fmt::Display; N],
}

impl<'a, const N: usize> Message<'a, N> {
    /// Creates a message for a text with placeholders.
    pub const fn new(key: Key, arguments: [&'a dyn fmt::Display; N]) -> Message<'a, N> {
        Message {
            key,
            count: None,
            arguments,
        }
    }

    /// Creates a message for a plural text.
    pub const fn plural(
        key: Key,
        count: u64,
        arguments: [&'a dyn fmt::Display; N],
    ) -> Message<'a, N> {
        Message {
            key,
            count: Some(count),
            arguments,
        }
    }

    /// Returns the key of the text.
    pub const fn key(&self) -> Key {
        self.key
    }

    /// Returns the count of a plural message.
    pub const fn count(&self) -> Option<u64> {
        self.count
    }
}

/// The kind of a list: the conjunction that joins its items.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListKind {
    /// A list joined with "and".
    And,
    /// A list joined with "or".
    Or,
}

/// The pattern of a list that joins two parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ListPattern {
    /// Joins the two items of a list of two.
    Pair,
    /// Joins an item to the rest of a longer list.
    Middle,
    /// Joins the last two items of a longer list.
    End,
}

/// The number of patterns of one list kind.
const PATTERNS_PER_KIND: usize = 3;

impl ListKind {
    /// Returns the index of a pattern in [`crate::schema::LIST_PATTERN_KEYS`].
    const fn pattern_index(self, pattern: ListPattern) -> usize {
        let kind_offset = match self {
            ListKind::And => 0,
            ListKind::Or => PATTERNS_PER_KIND,
        };
        let pattern_offset = match pattern {
            ListPattern::Pair => 0,
            ListPattern::Middle => 1,
            ListPattern::End => 2,
        };
        kind_offset + pattern_offset
    }
}

/// A message that formats through [`fmt::Display`].
#[derive(Clone, Copy)]
pub struct MessageDisplay<'c, 'a, const N: usize> {
    /// The catalog of the text.
    catalog: &'c Catalog,
    /// The message.
    message: Message<'a, N>,
}

impl<const N: usize> fmt::Display for MessageDisplay<'_, '_, N> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.catalog.write(formatter, self.message)
    }
}

/// A list that formats through [`fmt::Display`].
pub struct ListDisplay<'c, 'i, T> {
    /// The catalog of the list patterns.
    catalog: &'c Catalog,
    /// The kind of the list.
    kind: ListKind,
    /// The items of the list.
    items: &'i [T],
}

impl<T> Clone for ListDisplay<'_, '_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ListDisplay<'_, '_, T> {}

impl<T: fmt::Display> fmt::Display for ListDisplay<'_, '_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.catalog.write_list(formatter, self.kind, self.items)
    }
}

/// The literal parts of a list pattern: before `{first}`, between `{first}` and `{second}`, and
/// after `{second}`.
struct PatternParts<'c> {
    /// The segments before `{first}`.
    prefix: &'c [Segment],
    /// The segments between `{first}` and `{second}`.
    infix: &'c [Segment],
    /// The segments after `{second}`.
    suffix: &'c [Segment],
}

impl Catalog {
    /// Returns a fixed text.
    ///
    /// The key belongs to a text without placeholders and without plural forms. A debug build
    /// panics for any other key.
    pub fn text(&self, key: Key) -> &str {
        let info = self.schema.info(key);
        debug_assert!(
            !info.is_plural() && info.placeholders().is_empty(),
            "the text `{}.{}` is a plural text or holds placeholders",
            info.section(),
            info.key()
        );
        self.slice(self.entries[key.index()].form(None).text)
    }

    /// Writes a message.
    pub fn write<const N: usize>(
        &self,
        out: &mut (impl fmt::Write + ?Sized),
        message: Message<'_, N>,
    ) -> fmt::Result {
        let form = self.entries[message.key.index()].form(message.count);
        self.write_form(out, form, &message.arguments, message.count)
    }

    /// Returns the message as a value that formats through [`fmt::Display`].
    ///
    /// The value ignores the width, the fill and every other formatting option of the formatter.
    pub fn display<'c, 'a, const N: usize>(
        &'c self,
        message: Message<'a, N>,
    ) -> MessageDisplay<'c, 'a, N> {
        MessageDisplay {
            catalog: self,
            message,
        }
    }

    /// Writes a list of items joined with the patterns of the section `[list]`.
    ///
    /// An empty list writes nothing and one item writes the item. Two items use the pair pattern.
    /// A longer list joins each leading item to the rest with the middle pattern and the last two
    /// items with the end pattern. The write takes one pass over the items and allocates nothing.
    pub fn write_list(
        &self,
        out: &mut (impl fmt::Write + ?Sized),
        kind: ListKind,
        items: &[impl fmt::Display],
    ) -> fmt::Result {
        match items {
            [] => Ok(()),
            [item] => write!(out, "{item}"),
            [first, second] => self.write_pattern(out, kind, ListPattern::Pair, first, second),
            [leading @ .., second_last, last] => {
                let middle = self.pattern_parts(kind, ListPattern::Middle);
                for item in leading {
                    self.write_literals(out, middle.prefix)?;
                    write!(out, "{item}")?;
                    self.write_literals(out, middle.infix)?;
                }
                self.write_pattern(out, kind, ListPattern::End, second_last, last)?;
                for _ in leading {
                    self.write_literals(out, middle.suffix)?;
                }
                Ok(())
            }
        }
    }

    /// Returns a list of items joined with the patterns of the section `[list]` as a value that
    /// formats through [`fmt::Display`]. The list joins its items as [`Catalog::write_list`] does.
    ///
    /// The value ignores the width, the fill and every other formatting option of the formatter.
    pub fn display_list<'c, 'i, T: fmt::Display>(
        &'c self,
        kind: ListKind,
        items: &'i [T],
    ) -> ListDisplay<'c, 'i, T> {
        ListDisplay {
            catalog: self,
            kind,
            items,
        }
    }

    /// Returns the literal parts of a list pattern. The load guarantees that the pattern holds
    /// `{first}` and `{second}` once each and in this order.
    fn pattern_parts(&self, kind: ListKind, pattern: ListPattern) -> PatternParts<'_> {
        let key = self.schema.list_key(kind.pattern_index(pattern));
        let form = self.entries[key.index()].form(None);
        let segments = &self.segments[form.segments.range()];
        let first = segments
            .iter()
            .position(|segment| *segment == Segment::Argument(0));
        let second = segments
            .iter()
            .position(|segment| *segment == Segment::Argument(1));
        debug_assert!(matches!((first, second), (Some(first), Some(second)) if first < second));
        let first = first.unwrap_or(segments.len());
        let second = second.unwrap_or(segments.len()).max(first);
        PatternParts {
            prefix: &segments[..first],
            infix: segments.get(first + 1..second).unwrap_or(&[]),
            suffix: segments.get(second + 1..).unwrap_or(&[]),
        }
    }

    /// Writes the literal segments of a part of a list pattern.
    fn write_literals(
        &self,
        out: &mut (impl fmt::Write + ?Sized),
        segments: &[Segment],
    ) -> fmt::Result {
        for segment in segments {
            if let Segment::Literal(span) = *segment {
                out.write_str(self.slice(span))?;
            }
        }
        Ok(())
    }

    /// Writes a list pattern that joins two parts.
    fn write_pattern(
        &self,
        out: &mut (impl fmt::Write + ?Sized),
        kind: ListKind,
        pattern: ListPattern,
        first: &dyn fmt::Display,
        second: &dyn fmt::Display,
    ) -> fmt::Result {
        let key = self.schema.list_key(kind.pattern_index(pattern));
        let form = self.entries[key.index()].form(None);
        self.write_form(out, form, &[first, second], None)
    }

    /// Writes one form of a text with its arguments.
    fn write_form(
        &self,
        out: &mut (impl fmt::Write + ?Sized),
        form: Form,
        arguments: &[&dyn fmt::Display],
        count: Option<u64>,
    ) -> fmt::Result {
        if form.segments.is_empty() {
            return out.write_str(self.slice(form.text));
        }
        for segment in &self.segments[form.segments.range()] {
            match *segment {
                Segment::Literal(span) => out.write_str(self.slice(span))?,
                Segment::Argument(index) => {
                    debug_assert!(usize::from(index) < arguments.len());
                    if let Some(argument) = arguments.get(usize::from(index)) {
                        write!(out, "{argument}")?;
                    }
                }
                Segment::Count => {
                    if let Some(count) = count {
                        write!(out, "{count}")?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use core::fmt::Write;

    use common::allocation_counter::count_allocations;

    use super::*;
    use crate::{fixture, generate::leak_schema, schema::Schema};

    /// The sections of the English reference below the header and the list patterns.
    const REFERENCE: &str = "[common]\ncancel = Cancel\nbraces = Use {{braces}} and \\\\ and \\;\n\
                             two-lines = First\\nSecond\n\
                             [manager]\ndelete-question = Delete {name}?\n\
                             full-name = {first-name} {last-name}\n\
                             images.one = One image in {folder}\n\
                             images.other = {count} images in {folder}\n\
                             [note]\nchoices = The choices are {list}.\n";

    /// The sections of the Japanese translation below the header and above the list patterns.
    const JAPANESE: &str = "[common]\ncancel = キャンセル\nbraces = {{括弧}}\ntwo-lines = 一\\n二\n\
                            [manager]\ndelete-question = {name}を削除しますか？\n\
                            full-name = {last-name}{first-name}\n\
                            images.other = {folder}に{count}枚\n\
                            [note]\nchoices = 選択肢は{list}です。\n";

    /// The schema and the catalogs of the tests.
    struct Catalogs {
        /// The schema of the English reference.
        schema: &'static Schema,
        /// The English reference catalog.
        english: Catalog,
        /// The Japanese catalog with the English fallback.
        japanese: Catalog,
    }

    /// Builds the schema and the catalogs of the tests.
    fn catalogs() -> Catalogs {
        let reference = fixture::english(REFERENCE);
        let schema = leak_schema(&reference);
        let english = Catalog::reference("en", &reference, schema).unwrap();
        let japanese = fixture::japanese(JAPANESE);
        let loaded = Catalog::load("ja", &japanese, schema, &english).unwrap();
        assert!(loaded.faults.is_empty(), "{:?}", loaded.faults);
        Catalogs {
            schema,
            english,
            japanese: loaded.catalog,
        }
    }

    /// Returns the key of a section and a key name.
    fn key(schema: &Schema, section: &str, key: &str) -> Key {
        schema.find(section, key).unwrap()
    }

    /// Writes a message into a new string.
    fn written<const N: usize>(catalog: &Catalog, message: Message<'_, N>) -> String {
        let mut out = String::new();
        catalog.write(&mut out, message).unwrap();
        out
    }

    /// Writes a list into a new string.
    fn list(catalog: &Catalog, kind: ListKind, items: &[impl fmt::Display]) -> String {
        let mut out = String::new();
        catalog.write_list(&mut out, kind, items).unwrap();
        out
    }

    #[test]
    fn fixed_texts_resolve_their_escapes() {
        let catalogs = catalogs();
        let schema = catalogs.schema;
        let english = &catalogs.english;
        assert_eq!(english.text(key(schema, "common", "cancel")), "Cancel");
        assert_eq!(
            english.text(key(schema, "common", "braces")),
            "Use {braces} and \\ and ;"
        );
        assert_eq!(
            english.text(key(schema, "common", "two-lines")),
            "First\nSecond"
        );
        let japanese = &catalogs.japanese;
        assert_eq!(japanese.text(key(schema, "common", "braces")), "{括弧}");
        assert_eq!(japanese.text(key(schema, "common", "two-lines")), "一\n二");
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "holds placeholders")]
    fn text_of_a_message_panics_in_a_debug_build() {
        let catalogs = catalogs();
        let _ = catalogs
            .english
            .text(key(catalogs.schema, "manager", "delete-question"));
    }

    #[test]
    fn messages_fill_their_placeholders() {
        let catalogs = catalogs();
        let delete_question = key(catalogs.schema, "manager", "delete-question");
        let message = Message::new(delete_question, [&"Lemmings"]);
        assert_eq!(written(&catalogs.english, message), "Delete Lemmings?");
        assert_eq!(
            written(&catalogs.japanese, message),
            "Lemmingsを削除しますか？"
        );
        assert_eq!(
            format!("[{}]", catalogs.english.display(message)),
            "[Delete Lemmings?]"
        );
    }

    #[test]
    fn placeholders_in_another_order_than_the_reference() {
        let catalogs = catalogs();
        let full_name = key(catalogs.schema, "manager", "full-name");
        let message = Message::new(full_name, [&"Taro", &"Yamada"]);
        assert_eq!(written(&catalogs.english, message), "Taro Yamada");
        assert_eq!(written(&catalogs.japanese, message), "YamadaTaro");
    }

    #[test]
    fn plural_categories_of_both_rules() {
        let catalogs = catalogs();
        let images = key(catalogs.schema, "manager", "images");
        let message = |count| Message::plural(images, count, [&"A:"]);
        assert_eq!(written(&catalogs.english, message(0)), "0 images in A:");
        assert_eq!(written(&catalogs.english, message(1)), "One image in A:");
        assert_eq!(written(&catalogs.english, message(2)), "2 images in A:");
        assert_eq!(written(&catalogs.japanese, message(1)), "A:に1枚");
        assert_eq!(written(&catalogs.japanese, message(7)), "A:に7枚");
    }

    #[test]
    fn a_plural_text_of_the_fallback_keeps_the_rule_of_the_fallback() {
        let catalogs = catalogs();
        let source =
            fixture::japanese(&JAPANESE.replace("images.other = {folder}に{count}枚\n", ""));
        let loaded = Catalog::load("ja", &source, catalogs.schema, &catalogs.english).unwrap();
        let images = key(catalogs.schema, "manager", "images");
        let message = |count| Message::plural(images, count, [&"A:"]);
        assert_eq!(written(&loaded.catalog, message(1)), "One image in A:");
        assert_eq!(written(&loaded.catalog, message(3)), "3 images in A:");
    }

    #[test]
    fn lists_of_every_length() {
        let catalogs = catalogs();
        let english = &catalogs.english;
        assert_eq!(list(english, ListKind::And, &[] as &[&str]), "");
        assert_eq!(list(english, ListKind::And, &[&"a"]), "a");
        assert_eq!(list(english, ListKind::And, &[&"a", &"b"]), "a and b");
        assert_eq!(
            list(english, ListKind::And, &[&"a", &"b", &"c", &"d", &"e"]),
            "a, b, c, d, and e"
        );
        assert_eq!(list(english, ListKind::Or, &[&"a", &"b"]), "a or b");
        assert_eq!(
            list(english, ListKind::Or, &[&"a", &"b", &"c"]),
            "a; b; or c"
        );
        let japanese = &catalogs.japanese;
        assert_eq!(list(japanese, ListKind::And, &[&"甲"]), "甲");
        assert_eq!(list(japanese, ListKind::And, &[&"甲", &"乙"]), "甲と乙");
        assert_eq!(
            list(japanese, ListKind::Or, &[&1, &2, &3, &4, &5]),
            "1、2、3、4か5"
        );
    }

    #[test]
    fn middle_patterns_wrap_the_rest_of_the_list() {
        let reference = fixture::english(REFERENCE).replace(
            "and-middle = {first}, {second}",
            "and-middle = <{first} + {second}>",
        );
        let schema = leak_schema(&reference);
        let english = Catalog::reference("en", &reference, schema).unwrap();
        assert_eq!(
            list(&english, ListKind::And, &[&"a", &"b", &"c", &"d"]),
            "<a + <b + c, and d>>"
        );
    }

    #[test]
    fn a_long_list_writes_without_recursion() {
        let catalogs = catalogs();
        let items: Vec<usize> = (0..20_000).collect();
        let written = list(&catalogs.english, ListKind::And, &items);
        assert!(written.starts_with("0, 1, 2, "));
        assert!(written.ends_with("19998, and 19999"));
        assert_eq!(written.matches(", ").count(), 19_999);
    }

    #[test]
    fn lists_take_slices_of_any_display_type() {
        let catalogs = catalogs();
        let english = &catalogs.english;
        assert_eq!(
            list(english, ListKind::And, &["a", "b", "c"]),
            "a, b, and c"
        );
        let names = [String::from("x"), String::from("y")];
        assert_eq!(list(english, ListKind::Or, &names), "x or y");
        let mixed: [&dyn fmt::Display; 2] = [&"a", &2];
        assert_eq!(list(english, ListKind::And, &mixed), "a and 2");
    }

    #[test]
    fn a_list_is_the_argument_of_a_message() {
        let catalogs = catalogs();
        let choices = key(catalogs.schema, "note", "choices");
        let english = &catalogs.english;
        let japanese = &catalogs.japanese;
        let items = ["low", "high"];
        let english_list = english.display_list(ListKind::Or, &items);
        assert_eq!(
            english
                .display(Message::new(choices, [&english_list]))
                .to_string(),
            "The choices are low or high."
        );
        let japanese_list = japanese.display_list(ListKind::Or, &items);
        assert_eq!(
            japanese
                .display(Message::new(choices, [&japanese_list]))
                .to_string(),
            "選択肢はlowかhighです。"
        );
        assert_eq!(english_list.to_string(), "low or high");
    }

    #[test]
    fn displays_ignore_the_formatting_options() {
        let catalogs = catalogs();
        let english = &catalogs.english;
        let delete_question = key(catalogs.schema, "manager", "delete-question");
        let message = Message::new(delete_question, [&"X"]);
        assert_eq!(format!("{:>20}", english.display(message)), "Delete X?");
        assert_eq!(
            format!("{:*<9}", english.display_list(ListKind::And, &["a"])),
            "a"
        );
    }

    #[test]
    fn lookups_and_writes_allocate_nothing() {
        let catalogs = catalogs();
        let schema = catalogs.schema;
        let japanese = &catalogs.japanese;
        let cancel = key(schema, "common", "cancel");
        let full_name = key(schema, "manager", "full-name");
        let images = key(schema, "manager", "images");
        let choices = key(schema, "note", "choices");
        let names = [String::from("x"), String::from("y")];
        let mut out = String::with_capacity(256);
        let (allocations, ()) = count_allocations(|| {
            assert_eq!(japanese.text(cancel), "キャンセル");
            japanese
                .write(&mut out, Message::new(full_name, [&"Taro", &"Yamada"]))
                .unwrap();
            japanese
                .write(&mut out, Message::plural(images, 12, [&"A:"]))
                .unwrap();
            write!(
                out,
                "{}",
                japanese.display(Message::new(full_name, [&1, &2]))
            )
            .unwrap();
            japanese
                .write_list(&mut out, ListKind::And, &[&"a", &"b", &"c", &"d", &"e"])
                .unwrap();
            catalogs
                .english
                .write_list(&mut out, ListKind::Or, &[&"a", &"b"])
                .unwrap();
            japanese
                .write_list(&mut out, ListKind::And, &["p", "q"])
                .unwrap();
            write!(out, "{}", japanese.display_list(ListKind::Or, &names)).unwrap();
            let list = japanese.display_list(ListKind::And, &["s", "t", "u"]);
            japanese
                .write(&mut out, Message::new(choices, [&list]))
                .unwrap();
        });
        assert_eq!(allocations, 0);
        assert_eq!(
            out,
            "YamadaTaroA:に12枚21a、b、c、d、ea or bpとqxかy選択肢はs、t、uです。"
        );
    }
}
