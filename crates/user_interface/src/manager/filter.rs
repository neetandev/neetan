//! The filter of the library: a text that the title, the original title, the
//! developer or the year of a game contains.
//!
//! The match ignores case and the width of full-width Latin letters and
//! digits and of half-width katakana, and treats katakana and hiragana
//! alike. A filter typed in hiragana finds a title in katakana.

use super::rows::GameRow;

/// The distance from a full-width form to its ASCII character.
const FULL_WIDTH_OFFSET: u32 = 0xFEE0;

/// The distance from a katakana character to its hiragana character.
const KATAKANA_OFFSET: u32 = 0x60;

/// The first full-width form of an ASCII character, the exclamation mark.
const FIRST_FULL_WIDTH: char = '\u{FF01}';

/// The last full-width form of an ASCII character, the tilde.
const LAST_FULL_WIDTH: char = '\u{FF5E}';

/// The ideographic space.
const IDEOGRAPHIC_SPACE: char = '\u{3000}';

/// The first katakana character with a hiragana character, the small a.
const FIRST_KATAKANA: char = '\u{30A1}';

/// The last katakana character with a hiragana character, the small ke.
const LAST_KATAKANA: char = '\u{30F6}';

/// The first half-width katakana form, the half-width ideographic full stop.
const FIRST_HALF_WIDTH_KATAKANA: char = '\u{FF61}';

/// The last half-width katakana form, the half-width semi-voiced sound mark.
const LAST_HALF_WIDTH_KATAKANA: char = '\u{FF9F}';

/// The half-width voiced sound mark.
const HALF_WIDTH_VOICED_MARK: char = '\u{FF9E}';

/// The half-width semi-voiced sound mark.
const HALF_WIDTH_SEMI_VOICED_MARK: char = '\u{FF9F}';

/// The katakana u, which the voiced sound mark turns into the katakana vu.
const KATAKANA_U: char = '\u{30A6}';

/// The katakana vu.
const KATAKANA_VU: char = '\u{30F4}';

/// The full-width code point of each half-width katakana form from
/// [`FIRST_HALF_WIDTH_KATAKANA`] to [`LAST_HALF_WIDTH_KATAKANA`].
const HALF_WIDTH_KATAKANA: [u32; 63] = [
    0x3002, 0x300C, 0x300D, 0x3001, 0x30FB, 0x30F2, 0x30A1, 0x30A3, 0x30A5, 0x30A7, 0x30A9, 0x30E3,
    0x30E5, 0x30E7, 0x30C3, 0x30FC, 0x30A2, 0x30A4, 0x30A6, 0x30A8, 0x30AA, 0x30AB, 0x30AD, 0x30AF,
    0x30B1, 0x30B3, 0x30B5, 0x30B7, 0x30B9, 0x30BB, 0x30BD, 0x30BF, 0x30C1, 0x30C4, 0x30C6, 0x30C8,
    0x30CA, 0x30CB, 0x30CC, 0x30CD, 0x30CE, 0x30CF, 0x30D2, 0x30D5, 0x30D8, 0x30DB, 0x30DE, 0x30DF,
    0x30E0, 0x30E1, 0x30E2, 0x30E4, 0x30E6, 0x30E8, 0x30E9, 0x30EA, 0x30EB, 0x30EC, 0x30ED, 0x30EF,
    0x30F3, 0x309B, 0x309C,
];

/// The katakana from ka to to, which the voiced sound mark turns into the
/// next code point, such as shi into ji.
const VOICED_KATAKANA: &[u32] = &[
    0x30AB, 0x30AD, 0x30AF, 0x30B1, 0x30B3, 0x30B5, 0x30B7, 0x30B9, 0x30BB, 0x30BD, 0x30BF, 0x30C1,
    0x30C4, 0x30C6, 0x30C8,
];

/// The katakana from ha to ho, which the voiced sound mark turns into the
/// next code point and the semi-voiced sound mark into the one after it.
const HA_ROW_KATAKANA: &[u32] = &[0x30CF, 0x30D2, 0x30D5, 0x30D8, 0x30DB];

/// The full-width form of the half-width katakana `character`.
fn full_width_katakana(character: char) -> char {
    let index = character as usize - FIRST_HALF_WIDTH_KATAKANA as usize;
    char::from_u32(HALF_WIDTH_KATAKANA[index]).unwrap_or(character)
}

/// The katakana `base` with the sound mark `mark` of a half-width text, or
/// `None` when the two do not combine.
fn with_sound_mark(base: char, mark: char) -> Option<char> {
    let code = base as u32;
    let combined = match mark {
        HALF_WIDTH_VOICED_MARK if base == KATAKANA_U => KATAKANA_VU as u32,
        HALF_WIDTH_VOICED_MARK
            if VOICED_KATAKANA.contains(&code) || HA_ROW_KATAKANA.contains(&code) =>
        {
            code + 1
        }
        HALF_WIDTH_SEMI_VOICED_MARK if HA_ROW_KATAKANA.contains(&code) => code + 2,
        _ => return None,
    };
    char::from_u32(combined)
}

/// Returns `text` with every half-width katakana form in its full-width
/// form. A sound mark joins the katakana before it where the two combine.
fn widen_katakana(text: &str) -> String {
    let mut widened = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if !(FIRST_HALF_WIDTH_KATAKANA..=LAST_HALF_WIDTH_KATAKANA).contains(&character) {
            widened.push(character);
            continue;
        }
        let base = full_width_katakana(character);
        let combined = characters
            .peek()
            .and_then(|&mark| with_sound_mark(base, mark));
        match combined {
            Some(combined) => {
                widened.push(combined);
                characters.next();
            }
            None => widened.push(base),
        }
    }
    widened
}

/// Returns the form of `character` that the match compares: the ASCII
/// character of a full-width form, a space for the ideographic space and
/// the hiragana character of a katakana character.
fn fold_character(character: char) -> char {
    let shifted = |offset: u32| char::from_u32(character as u32 - offset).unwrap_or(character);
    match character {
        FIRST_FULL_WIDTH..=LAST_FULL_WIDTH => shifted(FULL_WIDTH_OFFSET),
        IDEOGRAPHIC_SPACE => ' ',
        FIRST_KATAKANA..=LAST_KATAKANA => shifted(KATAKANA_OFFSET),
        _ => character,
    }
}

/// Returns `text` in the form that the filter and the sort compare: in
/// lower case, with full-width forms, half-width katakana and katakana
/// folded.
pub(crate) fn fold(text: &str) -> String {
    widen_katakana(text)
        .chars()
        .map(fold_character)
        .flat_map(char::to_lowercase)
        .collect()
}

/// A filter text in its folded form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Filter {
    /// The folded text without the spaces around it. Empty matches every row.
    needle: String,
}

impl Filter {
    /// The filter of the typed `text`.
    pub(crate) fn new(text: &str) -> Self {
        Self {
            needle: fold(text.trim()),
        }
    }

    /// Whether the filter lets every row pass.
    pub(crate) fn is_empty(&self) -> bool {
        self.needle.is_empty()
    }

    /// Whether `text` contains the filter.
    fn finds(&self, text: &str) -> bool {
        fold(text).contains(&self.needle)
    }

    /// Whether the title, the original title, the developer or the year of
    /// the game `row` contains the filter. A file with an error matches by
    /// its file name.
    pub(crate) fn matches(&self, row: &GameRow) -> bool {
        if self.is_empty() {
            return true;
        }
        if !row.launchable() {
            return self.finds(&row.file_name);
        }
        [
            &row.title,
            &row.title_original,
            &row.developer,
            &row.year_text,
        ]
        .into_iter()
        .any(|text| self.finds(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::rows::tests::{broken, game};

    /// Verifies that the fold lowers the case and folds full-width forms and katakana.
    #[test]
    fn the_fold_ignores_case_width_and_kana() {
        assert_eq!(fold("YU-NO"), "yu-no");
        assert_eq!(fold("ＹＵ－ＮＯ　２"), "yu-no 2");
        assert_eq!(fold("エッジ"), "えっじ");
        assert_eq!(fold("ヴ"), "ゔ");
        assert_eq!(fold("ー"), "ー");
        assert_eq!(fold("三國志"), "三國志");
    }

    /// Verifies that the filter searches the title, the original title, the developer and the year.
    #[test]
    fn the_filter_searches_every_column() {
        let row = game("edge", "Edge", "エッジ", "TGL", "1993");
        for text in [
            "",
            "  ",
            "edge",
            "EDGE",
            "dg",
            "エッジ",
            "えっじ",
            "ッジ",
            "tgl",
            "1993",
            "199",
            "ｅｄｇｅ",
        ] {
            assert!(Filter::new(text).matches(&row), "{text}");
        }
        for text in ["edges", "falcom", "1992", "ブランディッシュ"] {
            assert!(!Filter::new(text).matches(&row), "{text}");
        }
    }

    /// Verifies that a game without metadata matches by its file stem and a file with an error by its file name.
    #[test]
    fn rows_without_metadata_match_by_their_file() {
        let plain = game("yu-no", "", "", "", "");
        assert!(Filter::new("yu-no").matches(&plain));
        assert!(!Filter::new("conf").matches(&plain));
        let file = broken("nemesis-90");
        assert!(Filter::new("nemesis").matches(&file));
        assert!(Filter::new(".conf").matches(&file));
        assert!(!Filter::new("x68030").matches(&file));
    }

    /// Verifies that half-width katakana with and without sound marks match their full-width forms.
    #[test]
    fn half_width_katakana_matches() {
        let row = game("edge", "Edge", "エッジ", "", "");
        assert!(Filter::new("ｴｯｼﾞ").matches(&row));
        assert!(Filter::new("ｴｯ").matches(&row));
        assert!(!Filter::new("ｴｯｼ").matches(&row));
        assert!(!Filter::new("ｴｯｼﾟ").matches(&row));
        assert_eq!(fold("ﾊﾟﾋﾟﾌﾞｳﾞｶﾞﾅﾞ"), fold("パピブヴガナ゛"));
        assert_eq!(fold("ｦｧｰﾝ｡｢｣､･"), fold("ヲァーン。「」、・"));
        assert_eq!(fold("ﾞ"), "゛");
        let half_width: String = (FIRST_HALF_WIDTH_KATAKANA..=LAST_HALF_WIDTH_KATAKANA).collect();
        assert!(!widen_katakana(&half_width).chars().any(|character| {
            (FIRST_HALF_WIDTH_KATAKANA..=LAST_HALF_WIDTH_KATAKANA).contains(&character)
        }));
    }

    /// Verifies that Japanese titles in kanji, hiragana and katakana match.
    #[test]
    fn japanese_text_matches() {
        let row = game(
            "yu-no",
            "YU-NO",
            "この世の果てで恋を唄う少女YU-NO",
            "エルフ",
            "1996",
        );
        assert!(Filter::new("恋を唄う").matches(&row));
        assert!(Filter::new("yu-no").matches(&row));
        assert!(Filter::new("えるふ").matches(&row));
        assert!(Filter::new("コノ").matches(&row));
        assert!(!Filter::new("恋を歌う").matches(&row));
    }
}
