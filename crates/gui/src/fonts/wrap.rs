//! Line breaking of a shaped text into a width.

use super::{Line, Run, cluster_at};

/// Splits `text` into lines no wider than `limit` em and writes them into
/// `lines` with their widths in em. `run` is the shaped `text` with one glyph
/// for each character.
///
/// A line breaks after a run of spaces and between two CJK characters when
/// the kinsoku rules allow it, and only after a character other than a space
/// on the same line. A line feed, a carriage return and the pair of both
/// always break. A word wider than the limit breaks before a character with
/// an advance. The lines leave out the trailing spaces and the line breaks.
pub(super) fn break_lines(run: &Run, text: &str, limit: f32, lines: &mut Vec<Line>) {
    lines.clear();
    let position = |index: usize| run.glyphs.get(index).map_or(run.advance, |glyph| glyph.x);
    let mut start = 0;
    let mut opportunity = None;
    let mut previous = None;
    let mut content = false;
    let mut after_carriage_return = false;
    for (index, character) in text.chars().enumerate() {
        if character == '\n' && after_carriage_return {
            start = index + 1;
            after_carriage_return = false;
            continue;
        }
        after_carriage_return = character == '\r';
        if character == '\n' || character == '\r' {
            push_line(lines, run, text, start, index);
            start = index + 1;
            opportunity = None;
            previous = None;
            content = false;
            continue;
        }
        if content
            && let Some(previous) = previous
            && can_break(previous, character)
        {
            opportunity = Some(index);
        }
        previous = Some(character);
        if is_space(character) {
            continue;
        }
        while content
            && position(index + 1) > position(index)
            && position(index + 1) - position(start) > limit
        {
            let end = opportunity.take().unwrap_or(index);
            push_line(lines, run, text, start, end);
            start = end;
            content = end < index;
        }
        content = true;
    }
    push_line(lines, run, text, start, run.glyphs.len());
}

/// Adds the line of the glyphs from `start` to `end` of `run` without its
/// trailing spaces to `lines`.
fn push_line(lines: &mut Vec<Line>, run: &Run, text: &str, start: usize, mut end: usize) {
    while end > start && text[cluster_at(run, end - 1, text)..].starts_with(is_space) {
        end -= 1;
    }
    let position = |index: usize| run.glyphs.get(index).map_or(run.advance, |glyph| glyph.x);
    lines.push(Line {
        bytes: cluster_at(run, start, text)..cluster_at(run, end, text),
        glyphs: start..end,
        width: position(end) - position(start),
    });
}

/// Whether a line may break between `before` and `after`.
fn can_break(before: char, after: char) -> bool {
    if forbids_line_end(before) || forbids_line_start(after) {
        return false;
    }
    (is_space(before) && !is_space(after)) || (is_cjk(before) && is_cjk(after))
}

/// Whether `character` is a space that allows a break after it: a white space
/// other than the no-break spaces, or the zero width space.
pub(super) fn is_space(character: char) -> bool {
    character == '\u{200B}'
        || (character.is_whitespace() && !matches!(character, '\u{A0}' | '\u{2007}' | '\u{202F}'))
}

/// Whether `character` belongs to a script that breaks between any two
/// characters: kana, kanji, Hangul syllables, CJK punctuation and the full
/// width forms.
fn is_cjk(character: char) -> bool {
    matches!(
        character,
        '\u{2E80}'..='\u{2FDF}'
            | '\u{3001}'..='\u{303F}'
            | '\u{3040}'..='\u{33FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{AC00}'..='\u{D7AF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FE30}'..='\u{FE4F}'
            | '\u{FF01}'..='\u{FFEF}'
            | '\u{20000}'..='\u{3FFFF}'
    )
}

/// Whether a line must not start with `character`: closing brackets and
/// punctuation, the prolonged sound mark, iteration marks and small kana.
fn forbids_line_start(character: char) -> bool {
    matches!(
        character,
        // 、。，．・：；？！ー
        '\u{3001}' | '\u{3002}' | '\u{FF0C}' | '\u{FF0E}' | '\u{30FB}' | '\u{FF1A}' | '\u{FF1B}'
            | '\u{FF1F}' | '\u{FF01}' | '\u{30FC}'
            // 」』）］｝〕〉》】〗〙〟’”
            | '\u{300D}' | '\u{300F}' | '\u{FF09}' | '\u{FF3D}' | '\u{FF5D}' | '\u{3015}'
            | '\u{3009}' | '\u{300B}' | '\u{3011}' | '\u{3017}' | '\u{3019}' | '\u{301F}'
            | '\u{2019}' | '\u{201D}'
            // 々〻ゝゞヽヾ゛゜〃
            | '\u{3005}' | '\u{303B}' | '\u{309D}' | '\u{309E}' | '\u{30FD}' | '\u{30FE}'
            | '\u{309B}' | '\u{309C}' | '\u{3003}'
            // 〞｠゠
            | '\u{301E}' | '\u{FF60}' | '\u{30A0}'
            // ぁぃぅぇぉっゃゅょゎゕゖ
            | '\u{3041}' | '\u{3043}' | '\u{3045}' | '\u{3047}' | '\u{3049}' | '\u{3063}'
            | '\u{3083}' | '\u{3085}' | '\u{3087}' | '\u{308E}' | '\u{3095}' | '\u{3096}'
            // ァィゥェォッャュョヮヵヶ
            | '\u{30A1}' | '\u{30A3}' | '\u{30A5}' | '\u{30A7}' | '\u{30A9}' | '\u{30C3}'
            | '\u{30E3}' | '\u{30E5}' | '\u{30E7}' | '\u{30EE}' | '\u{30F5}' | '\u{30F6}'
            // ㇰ から ㇿ
            | '\u{31F0}'..='\u{31FF}'
            // ｡｣､･ｧｨｩｪｫｬｭｮｯｰ
            | '\u{FF61}' | '\u{FF63}' | '\u{FF64}' | '\u{FF65}' | '\u{FF67}'..='\u{FF70}'
    )
}

/// Whether a line must not end with `character`: opening brackets.
fn forbids_line_end(character: char) -> bool {
    matches!(
        character,
        // 「『（［｛〔〈《【〖〘〝‘“｢｟
        '\u{300C}'
            | '\u{300E}'
            | '\u{FF08}'
            | '\u{FF3B}'
            | '\u{FF5B}'
            | '\u{3014}'
            | '\u{3008}'
            | '\u{300A}'
            | '\u{3010}'
            | '\u{3016}'
            | '\u{3018}'
            | '\u{301D}'
            | '\u{2018}'
            | '\u{201C}'
            | '\u{FF62}'
            | '\u{FF5F}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RunGlyph;

    /// A run of `text` in which every character advances by one em and the
    /// zero width space by nothing.
    fn monospaced(text: &str) -> Run {
        let mut run = Run::default();
        for (cluster, character) in text.char_indices() {
            let advance = if character == '\u{200B}' { 0.0 } else { 1.0 };
            run.push(RunGlyph::plain(None, cluster), advance);
        }
        run
    }

    /// The lines of `text` at the width `limit` in em.
    fn lines(text: &str, limit: f32) -> Vec<&str> {
        let mut lines = Vec::new();
        break_lines(&monospaced(text), text, limit, &mut lines);
        lines.into_iter().map(|line| &text[line.bytes]).collect()
    }

    #[test]
    fn latin_breaks_at_spaces() {
        assert_eq!(
            lines("the quick brown fox", 10.0),
            ["the quick", "brown fox"]
        );
        assert_eq!(lines("the  quick", 5.0), ["the", "quick"]);
        assert_eq!(lines("fits", 10.0), ["fits"]);
        assert_eq!(lines("", 10.0), [""]);
    }

    #[test]
    fn leading_spaces_stay_on_the_first_line() {
        assert_eq!(lines("  abcdef", 5.0), ["  abc", "def"]);
        assert_eq!(lines("    ab", 2.0), ["    a", "b"]);
        assert_eq!(lines("  ab cd", 4.0), ["  ab", "cd"]);
    }

    #[test]
    fn lines_report_their_glyphs_and_widths() {
        let text = "ab cd\nef";
        let mut lines = Vec::new();
        break_lines(&monospaced(text), text, 3.0, &mut lines);
        assert_eq!(
            lines,
            [
                Line {
                    bytes: 0..2,
                    glyphs: 0..2,
                    width: 2.0,
                },
                Line {
                    bytes: 3..5,
                    glyphs: 3..5,
                    width: 2.0,
                },
                Line {
                    bytes: 6..8,
                    glyphs: 6..8,
                    width: 2.0,
                },
            ]
        );
    }

    #[test]
    fn a_long_word_breaks_at_a_character() {
        assert_eq!(lines("abcdefgh ij", 3.0), ["abc", "def", "gh", "ij"]);
        assert_eq!(
            lines("a\u{E9}\u{301}\u{1F600}x", 1.0),
            ["a", "\u{E9}", "\u{301}", "\u{1F600}", "x"]
        );
        assert_eq!(lines("abc", 0.0), ["a", "b", "c"]);
        assert_eq!(lines("abc", f32::INFINITY), ["abc"]);
    }

    #[test]
    fn no_break_spaces_join_words() {
        assert_eq!(lines("ab\u{A0}cd ef", 5.0), ["ab\u{A0}cd", "ef"]);
    }

    #[test]
    fn a_zero_width_space_allows_a_break() {
        assert_eq!(lines("abc\u{200B}def", 4.0), ["abc", "def"]);
        assert_eq!(lines("abc\u{200B}def", 6.0), ["abc\u{200B}def"]);
    }

    #[test]
    fn line_feeds_and_carriage_returns_break() {
        assert_eq!(lines("ab\ncd\r\n\nef", 10.0), ["ab", "cd", "", "ef"]);
        assert_eq!(lines("ab\rcd\r\rx", 10.0), ["ab", "cd", "", "x"]);
        assert_eq!(lines("ab\r", 10.0), ["ab", ""]);
    }

    #[test]
    fn cjk_breaks_between_characters() {
        assert_eq!(lines("日本語の文章", 4.0), ["日本語の", "文章"]);
        assert_eq!(
            lines("한국어 문장입니다", 4.0),
            ["한국어", "문장입니", "다"]
        );
        assert_eq!(lines("日本abc", 3.0), ["日", "本ab", "c"]);
    }

    #[test]
    fn kinsoku_keeps_punctuation_with_its_neighbor() {
        assert_eq!(lines("これです。次", 4.0), ["これで", "す。次"]);
        assert_eq!(lines("あいう「えお」", 4.0), ["あいう", "「えお」"]);
        assert_eq!(lines("ちょっと", 3.0), ["ちょっ", "と"]);
        assert_eq!(lines("あいうー", 3.0), ["あい", "うー"]);
        assert_eq!(lines("あいう〃", 3.0), ["あい", "う〃"]);
        assert_eq!(lines("あいう｟え", 4.0), ["あいう", "｟え"]);
    }
}
