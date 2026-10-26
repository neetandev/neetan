//! Line breaking of a shaped text into a width.

use super::{Line, Run, cluster_at};

/// Splits `text` into lines no wider than `limit` em and writes them into
/// `lines` with their widths in em. `run` is the shaped `text` with one glyph
/// for each character.
///
/// A line breaks where `can_break` allows it, and only after a character
/// other than a space on the same line. A line feed, a carriage return and
/// the pair of both always break. A word wider than the limit breaks before
/// a character with an advance. The lines leave out the trailing spaces and
/// the line breaks.
pub(super) fn break_lines(run: &Run, text: &str, limit: f32, lines: &mut Vec<Line>) {
    lines.clear();
    let position = |index: usize| run.glyphs.get(index).map_or(run.advance, |glyph| glyph.x);
    let mut start = 0;
    let mut opportunity = None;
    let mut previous = None;
    let mut content = false;
    let mut after_carriage_return = false;
    let mut characters = text.chars().enumerate().peekable();
    while let Some((index, character)) = characters.next() {
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
            && can_break(
                previous,
                character,
                characters.peek().map(|&(_, following)| following),
            )
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

/// Whether a line may break between `before` and `after`. `following` is
/// the character after `after`.
///
/// A line breaks after a run of spaces, between two CJK characters and at
/// the border of a CJK character and a word of another script. A Latin word
/// keeps its characters together, a number keeps the CJK character after it
/// as its counter or unit, and a combining mark stays with its base.
fn can_break(before: char, after: char, following: Option<char>) -> bool {
    if forbids_line_end(before) || forbids_line_start(after) || is_mark(after) {
        return false;
    }
    if is_space(before) || is_space(after) {
        return is_space(before) && !is_space(after);
    }
    match (is_cjk(before), is_cjk(after)) {
        (true, true) => true,
        (true, false) => !is_closing_latin(after, following),
        (false, true) => !is_opening_latin(before) && !before.is_ascii_digit(),
        (false, false) => false,
    }
}

/// Whether `character` is a mark that combines with the character before it
/// or joins it: the combining diacritics, the combining kana voicing marks,
/// the variation selectors and the zero width joiner.
fn is_mark(character: char) -> bool {
    matches!(
        character,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{200D}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{3099}'
            | '\u{309A}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FE20}'..='\u{FE2F}'
            | '\u{E0100}'..='\u{E01EF}'
    )
}

/// Whether `character` is Latin punctuation that closes a word and must not
/// start a line after a CJK character. `following` is the character after
/// it. A full stop that a letter follows starts a word, for example the
/// extension `.hdi`.
fn is_closing_latin(character: char, following: Option<char>) -> bool {
    match character {
        '.' => !following.is_some_and(is_word_letter),
        ',' | ':' | ';' | '!' | '?' | ')' | ']' | '}' | '%' | '\'' | '"' => true,
        _ => false,
    }
}

/// Whether `character` is a letter of a script that breaks only at spaces.
fn is_word_letter(character: char) -> bool {
    character.is_alphabetic() && !is_cjk(character)
}

/// Whether `character` is Latin punctuation that opens a word and must not
/// end a line before a CJK character.
fn is_opening_latin(character: char) -> bool {
    matches!(character, '(' | '[' | '{' | '\'' | '"')
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

    /// Asserts for every width from the widest word to the whole text that no line of `text`
    /// starts with a French closing mark or a no-break space and none ends with U+00AB or a
    /// no-break space.
    fn assert_french_marks_stay_with_their_words(text: &str) {
        let widest = text
            .split(' ')
            .map(|word| word.chars().count())
            .max()
            .unwrap_or(0);
        for limit in widest..=text.chars().count() {
            for line in lines(text, limit as f32) {
                assert!(
                    !line.starts_with([':', ';', '!', '?', '\u{BB}', '\u{A0}', '\u{202F}']),
                    "{line:?} at {limit} em"
                );
                assert!(
                    !line.ends_with(['\u{AB}', '\u{A0}', '\u{202F}']),
                    "{line:?} at {limit} em"
                );
            }
        }
    }

    #[test]
    fn french_punctuation_stays_with_the_word_before_its_space() {
        assert_eq!(
            lines("le fichier\u{A0}: abc", 11.0),
            ["le", "fichier\u{A0}:", "abc"]
        );
        assert_eq!(
            lines("Supprimer ce jeu\u{202F}?", 16.0),
            ["Supprimer ce", "jeu\u{202F}?"]
        );
        assert_eq!(
            lines("Oui\u{202F}! Non\u{202F}; fin", 7.0),
            ["Oui\u{202F}!", "Non\u{202F};", "fin"]
        );
        assert_french_marks_stay_with_their_words(
            "Erreur de lecture\u{A0}: le fichier est vide\u{202F}; voulez-vous le remplacer\u{202F}? Oui\u{202F}!",
        );
    }

    #[test]
    fn french_quotes_stay_with_the_quoted_word() {
        assert_eq!(
            lines("le dossier \u{AB}\u{A0}games\u{A0}\u{BB} existe", 12.0),
            ["le dossier", "\u{AB}\u{A0}games\u{A0}\u{BB}", "existe"]
        );
        assert_french_marks_stay_with_their_words(
            "La valeur \u{AB}\u{A0}on\u{A0}\u{BB} ou \u{AB}\u{A0}off\u{A0}\u{BB} est attendue\u{A0}: \u{AB}\u{A0}auto\u{A0}\u{BB}.",
        );
    }

    /// Asserts for every width from the widest word to the whole text that no line of `text`
    /// ends with a Spanish opening mark: U+00BF, U+00A1 or U+00AB.
    fn assert_spanish_opening_marks_stay_with_their_words(text: &str) {
        let widest = text
            .split(' ')
            .map(|word| word.chars().count())
            .max()
            .unwrap_or(0);
        for limit in widest..=text.chars().count() {
            for line in lines(text, limit as f32) {
                assert!(
                    !line.ends_with(['\u{BF}', '\u{A1}', '\u{AB}']),
                    "{line:?} at {limit} em"
                );
            }
        }
    }

    #[test]
    fn spanish_opening_marks_stay_with_the_word_after_them() {
        assert_eq!(
            lines("\u{BF}Eliminar \u{AB}Ys\u{BB}? \u{A1}Listo!", 13.0),
            ["\u{BF}Eliminar", "\u{AB}Ys\u{BB}? \u{A1}Listo!"]
        );
        assert_eq!(
            lines("la carpeta \u{AB}games/\u{BB} existe", 12.0),
            ["la carpeta", "\u{AB}games/\u{BB}", "existe"]
        );
        assert_spanish_opening_marks_stay_with_their_words(
            "\u{BF}Eliminar \u{AB}Ys\u{BB}? Se esperaba \u{AB}on\u{BB} u \u{AB}off\u{BB}. \u{A1}El archivo \u{AB}neetan.conf\u{BB} no existe!",
        );
    }

    /// The characters of Traditional Chinese text that must not start a line:
    /// ，。、；：？！）」
    const TRADITIONAL_CHINESE_CLOSING: [char; 9] = [
        '\u{FF0C}', '\u{3002}', '\u{3001}', '\u{FF1B}', '\u{FF1A}', '\u{FF1F}', '\u{FF01}',
        '\u{FF09}', '\u{300D}',
    ];

    /// The characters of Traditional Chinese text that must not end a line: （「
    const TRADITIONAL_CHINESE_OPENING: [char; 2] = ['\u{FF08}', '\u{300C}'];

    /// The margin in em above the widest run of other scripts in a CJK text: the
    /// brackets and the punctuation that stay with the run.
    const KINSOKU_MARGIN: usize = 4;

    /// Asserts for every width from the widest run of other scripts with its
    /// brackets to the whole text that no line of `text` starts with one of
    /// `closing` and that no line ends with one of `opening`.
    fn assert_chinese_kinsoku(text: &str, closing: &[char], opening: &[char]) {
        let widest = text
            .split(is_cjk)
            .map(|run| run.chars().count())
            .max()
            .unwrap_or(0)
            + KINSOKU_MARGIN;
        for limit in widest..=text.chars().count() {
            for line in lines(text, limit as f32) {
                assert!(!line.starts_with(closing), "{line:?} at {limit} em");
                assert!(!line.ends_with(opening), "{line:?} at {limit} em");
            }
        }
    }

    #[test]
    fn traditional_chinese_breaks_between_han_characters_and_at_latin_words() {
        assert_eq!(
            lines("磁碟機的軟碟映像檔", 4.0),
            ["磁碟機的", "軟碟映像", "檔"]
        );
        assert_eq!(
            lines("沒有CD-ROM光碟機。", 6.0),
            ["沒有", "CD-ROM", "光碟機。"]
        );
        assert_eq!(lines("PC-9801沒有", 8.0), ["PC-9801沒", "有"]);
        assert_eq!(
            lines("磁碟機1的軟碟映像檔", 4.0),
            ["磁碟機", "1的軟碟", "映像檔"]
        );
    }

    #[test]
    fn traditional_chinese_punctuation_stays_with_its_neighbor() {
        assert_eq!(lines("設定未儲存。編輯器", 5.0), ["設定未儲", "存。編輯器"]);
        assert_eq!(
            lines("根目錄是「neetan.conf」", 14.0),
            ["根目錄是", "「neetan.conf」"]
        );
        assert_eq!(
            lines("值為空（需要資料夾）", 4.0),
            ["值為空", "（需要資", "料夾）"]
        );
        for text in [
            "「{game}」尚未儲存。「覆寫」會以編輯器中的變更取代檔案。「取消」會保留變更並返回編輯器。",
            "Neetan會依下列順序尋找，並讀取第一個找到的「neetan.conf」：「--settings <file>」、目前目錄、執行檔的資料夾、SDL的pref path。設定檔中的相對路徑以根目錄為基準。",
            "值「abc」無效（請指定「on」或「off」）",
            "要刪除「天外魔境」嗎？完成！下一步；選項：是",
            "「low」為8\u{A0}MHz，「high」為16\u{A0}MHz。",
        ] {
            assert_chinese_kinsoku(
                text,
                &TRADITIONAL_CHINESE_CLOSING,
                &TRADITIONAL_CHINESE_OPENING,
            );
        }
    }

    /// The characters of Simplified Chinese text that must not start a line:
    /// ，。、；：？！）”’
    const SIMPLIFIED_CHINESE_CLOSING: [char; 10] = [
        '\u{FF0C}', '\u{3002}', '\u{3001}', '\u{FF1B}', '\u{FF1A}', '\u{FF1F}', '\u{FF01}',
        '\u{FF09}', '\u{201D}', '\u{2019}',
    ];

    /// The characters of Simplified Chinese text that must not end a line: （“‘
    const SIMPLIFIED_CHINESE_OPENING: [char; 3] = ['\u{FF08}', '\u{201C}', '\u{2018}'];

    #[test]
    fn simplified_chinese_breaks_between_han_characters_and_at_latin_words() {
        assert_eq!(
            lines("驱动器的软盘镜像文件", 4.0),
            ["驱动器的", "软盘镜像", "文件"]
        );
        assert_eq!(
            lines("没有CD-ROM驱动器。", 6.0),
            ["没有", "CD-ROM", "驱动器。"]
        );
    }

    #[test]
    fn simplified_chinese_quotes_stay_with_their_neighbors() {
        assert_eq!(
            lines("请点击\u{201C}确定\u{201D}按钮", 4.0),
            ["请点击", "\u{201C}确定\u{201D}", "按钮"]
        );
        assert_eq!(
            lines("\u{201C}确定\u{201D}", 2.0),
            ["\u{201C}确", "定\u{201D}"]
        );
        assert_eq!(
            lines("\u{2018}软盘\u{2019}", 2.0),
            ["\u{2018}软", "盘\u{2019}"]
        );
        assert_eq!(
            lines("根目录是\u{201C}neetan.conf\u{201D}", 14.0),
            ["根目录是", "\u{201C}neetan.conf\u{201D}"]
        );
        for text in [
            "\u{201C}{game}\u{201D}尚未保存。\u{201C}覆盖\u{201D}将用编辑器中的更改替换文件。\u{201C}取消\u{201D}将保留更改并返回编辑器。",
            "Neetan按以下顺序查找，并读取找到的第一个\u{201C}neetan.conf\u{201D}：\u{201C}--settings <file>\u{201D}、当前目录、可执行文件所在的文件夹、SDL的pref path。",
            "值\u{201C}abc\u{201D}无效（应为\u{201C}on\u{201D}或\u{201C}off\u{201D}）",
            "要删除\u{201C}天外魔境\u{201D}吗？完成！下一步；选项：是",
            "\u{201C}low\u{201D}为8\u{A0}MHz，\u{201C}high\u{201D}为16\u{A0}MHz。",
            "按\u{2018}确定\u{2019}保存\u{2018}设置\u{2019}。",
        ] {
            assert_chinese_kinsoku(
                text,
                &SIMPLIFIED_CHINESE_CLOSING,
                &SIMPLIFIED_CHINESE_OPENING,
            );
        }
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
        assert_eq!(lines("日本abc", 3.0), ["日本", "abc"]);
    }

    #[test]
    fn latin_words_in_cjk_text_break_at_their_borders() {
        assert_eq!(
            lines("PC9801RAにはCD-ROMドライブがありません。", 10.0),
            ["PC9801RAには", "CD-ROMドライブ", "がありません。"]
        );
        assert_eq!(
            lines("設定midi = mt32に必要です。", 9.0),
            ["設定midi =", "mt32に必要で", "す。"]
        );
        assert_eq!(
            lines("ドライブ1のイメージ", 5.0),
            ["ドライブ", "1のイメー", "ジ"]
        );
        assert_eq!(lines("abcdefgh日本", 4.0), ["abcd", "efgh", "日本"]);
    }

    #[test]
    fn a_number_stays_with_the_cjk_character_after_it() {
        assert_eq!(lines("ゲーム18本", 5.0), ["ゲーム", "18本"]);
        assert_eq!(lines("あいう3行目", 4.0), ["あいう", "3行目"]);
        assert_eq!(lines("あ12345本", 4.0), ["あ", "1234", "5本"]);
        assert_eq!(lines("本18冊", 2.0), ["本", "18", "冊"]);
    }

    #[test]
    fn latin_punctuation_stays_with_its_word() {
        assert_eq!(lines("あいabc.う", 5.0), ["あい", "abc.う"]);
        assert_eq!(lines("あい(abc)う", 5.0), ["あい", "(abc)", "う"]);
        assert_eq!(lines("あいう.", 3.0), ["あい", "う."]);
        assert_eq!(lines("あいう...", 4.0), ["あい", "う..."]);
    }

    #[test]
    fn a_full_stop_before_a_letter_starts_a_word() {
        assert_eq!(lines("あいは.hdi/.nhd、", 10.0), ["あいは", ".hdi/.nhd、"]);
        assert_eq!(lines("あい：.hdi", 4.0), ["あい：", ".hdi"]);
        assert_eq!(lines("あいend.う", 5.0), ["あい", "end.う"]);
        assert_eq!(lines("あいう.え", 3.0), ["あい", "う.え"]);
        assert_eq!(lines("あいう.5", 3.0), ["あい", "う.5"]);
    }

    #[test]
    fn marks_stay_with_their_base() {
        assert_eq!(lines("あいか\u{3099}", 3.0), ["あい", "か\u{3099}"]);
        assert_eq!(lines("あい葛\u{E0100}", 3.0), ["あい", "葛\u{E0100}"]);
        assert_eq!(lines("あいa\u{301}", 3.0), ["あい", "a\u{301}"]);
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
