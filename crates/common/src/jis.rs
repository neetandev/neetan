//! Helper functions to handle the JIS X 0208 (with NEC extensions) conversion to and from UTF-8.

mod tables;

use alloc::{string::String, vec::Vec};

use tables::{ANK_TO_UNICODE, JIS_TO_UNICODE, UNICODE_TO_JIS};

/// A character in JIS encoding as used by the PC-98 text VRAM.
///
/// Internal `u16` stores standard JIS format: `(ku << 8) | ten` for JIS X 0208,
/// or `0x00XX` for ANK (JIS X 0201) where XX is the character code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JisChar(u16);

impl save_state::StateEncode for JisChar {
    fn encode_state(&self, output: &mut Vec<u8>) {
        save_state::StateEncode::encode_state(&self.0, output);
    }
}

impl save_state::StateDecode for JisChar {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        Ok(Self(save_state::StateDecode::decode_state(decoder)?))
    }
}

impl JisChar {
    /// ANK space character.
    pub const SPACE: Self = Self(0x0020);

    /// Replacement character (JIS middle dot ・, 0x2126).
    pub const REPLACEMENT: Self = Self(0x2126);

    /// Constructs from text VRAM byte pair.
    ///
    /// PC-98 text VRAM kanji encoding:
    /// - Even byte = row code (JIS ku minus 0x20, range 0x01-0x5D)
    /// - Odd byte  = column code (JIS ten, range 0x21-0x7E)
    ///
    /// For ANK characters, the odd byte is 0x00 and the even byte is the
    /// character code directly.
    #[inline]
    pub const fn from_vram_bytes(even: u8, odd: u8) -> Self {
        if odd == 0 {
            Self(even as u16)
        } else {
            Self(((even as u16 + 0x20) << 8) | odd as u16)
        }
    }

    /// Returns `(even, odd)` bytes for writing to text VRAM.
    ///
    /// Kanji: even = ku - 0x20 (row code), odd = ten (column code).
    /// ANK: even = character code, odd = 0x00.
    #[inline]
    pub const fn to_vram_bytes(self) -> (u8, u8) {
        if self.is_ank() {
            (self.0 as u8, 0x00)
        } else {
            let ku = (self.0 >> 8) as u8;
            let ten = self.0 as u8;
            (ku.wrapping_sub(0x20), ten)
        }
    }

    /// Constructs from a raw JIS code value.
    #[inline]
    pub const fn from_u16(value: u16) -> Self {
        Self(value)
    }

    /// Returns the raw JIS code value.
    #[inline]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Returns `true` if this is an ANK (single-byte) character.
    #[inline]
    pub const fn is_ank(self) -> bool {
        (self.0 & 0xFF00) == 0
    }

    /// Returns `true` if this is a half-width JIS glyph that occupies one text cell.
    #[inline]
    pub const fn is_halfwidth_jis(self) -> bool {
        let ku = (self.0 >> 8) as u8;
        ku >= 0x29 && ku <= 0x2B
    }

    /// Returns the number of text cells this character occupies on a PC-98 text console.
    #[inline]
    pub const fn display_width(self) -> u8 {
        if self.is_ank() || self.is_halfwidth_jis() {
            1
        } else {
            2
        }
    }
}

/// Converts a JIS character to its Unicode equivalent.
///
/// Returns `None` for unmapped JIS codes.
pub fn jis_to_char(jis: JisChar) -> Option<char> {
    let code = jis.as_u16();

    if jis.is_ank() {
        let unicode = ANK_TO_UNICODE[code as usize];
        return if unicode != 0 {
            char::from_u32(unicode as u32)
        } else {
            None
        };
    }

    let ku = (code >> 8) as u8;
    let ten = (code & 0xFF) as u8;

    if !(0x21..=0x7E).contains(&ku) || !(0x21..=0x7E).contains(&ten) {
        return None;
    }

    let index = (ku - 0x21) as usize * 94 + (ten - 0x21) as usize;
    let unicode = JIS_TO_UNICODE[index];
    if unicode == 0 {
        return None;
    }

    char::from_u32(unicode as u32)
}

/// Converts a Unicode character to its JIS equivalent.
///
/// Returns [`JisChar::REPLACEMENT`] for characters that cannot be mapped.
pub fn char_to_jis(ch: char) -> JisChar {
    let cp = ch as u32;

    // ASCII identity fast path
    if (0x20..=0x7D).contains(&cp) {
        return JisChar::from_u16(cp as u16);
    }

    // Non-BMP characters cannot be in our table
    if cp > 0xFFFF {
        return JisChar::REPLACEMENT;
    }

    let cp16 = cp as u16;
    match UNICODE_TO_JIS.binary_search_by_key(&cp16, |&(u, _)| u) {
        Ok(i) => JisChar::from_u16(UNICODE_TO_JIS[i].1),
        Err(_) => JisChar::REPLACEMENT,
    }
}

/// Converts a string to a sequence of JIS characters.
pub fn str_to_jis(text: &str) -> Vec<JisChar> {
    text.chars().map(char_to_jis).collect()
}

/// Returns `true` if the byte is a Shift-JIS lead byte.
pub fn is_shift_jis_lead_byte(byte: u8) -> bool {
    (0x81..=0x9F).contains(&byte) || (0xE0..=0xFC).contains(&byte)
}

/// Returns `true` if the byte is a Shift-JIS trail byte.
pub fn is_shift_jis_trail_byte(byte: u8) -> bool {
    (0x40..=0x7E).contains(&byte) || (0x80..=0xFC).contains(&byte)
}

/// Converts a Shift-JIS double-byte pair to a JIS character.
pub fn shift_jis_pair_to_jis(lead: u8, trail: u8) -> Option<JisChar> {
    if !is_shift_jis_lead_byte(lead) || !is_shift_jis_trail_byte(trail) {
        return None;
    }

    let mut row = if lead <= 0x9F {
        ((lead - 0x81) * 2) + 0x21
    } else {
        ((lead - 0xC1) * 2) + 0x21
    };

    let cell = if trail <= 0x7E {
        trail - 0x1F
    } else if trail <= 0x9E {
        trail - 0x20
    } else {
        row += 1;
        trail - 0x7E
    };

    Some(JisChar::from_u16(((row as u16) << 8) | cell as u16))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftJisChar {
    Single(u8),
    Double([u8; 2]),
}

impl ShiftJisChar {
    #[inline]
    pub const fn len(self) -> usize {
        match self {
            Self::Single(_) => 1,
            Self::Double(_) => 2,
        }
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        false
    }

    #[inline]
    pub fn write_to(self, output: &mut Vec<u8>) {
        match self {
            Self::Single(byte) => output.push(byte),
            Self::Double(bytes) => output.extend_from_slice(&bytes),
        }
    }
}

/// Converts a JIS character to Shift-JIS bytes.
pub fn jis_to_shift_jis(jis: JisChar) -> Option<ShiftJisChar> {
    if jis.is_ank() {
        return Some(ShiftJisChar::Single(jis.as_u16() as u8));
    }

    let code = jis.as_u16();
    let row = (code >> 8) as u8;
    let cell = code as u8;
    if !(0x21..=0x7E).contains(&row) || !(0x21..=0x7E).contains(&cell) {
        return None;
    }

    let lead = if row <= 0x5E {
        0x81 + ((row - 0x21) / 2)
    } else {
        0xE0 + ((row - 0x5F) / 2)
    };

    let trail = if row & 1 == 0 {
        cell + 0x7E
    } else if cell <= 0x5F {
        cell + 0x1F
    } else {
        cell + 0x20
    };

    Some(ShiftJisChar::Double([lead, trail]))
}

/// Reads character cells from a text VRAM byte slice and converts to a String.
///
/// Each character cell is 2 bytes (even = ten/char code, odd = ku/0x00).
/// Reads `count` cells starting at `byte_offset` with stride 2.
/// Unmapped characters become U+FFFD (replacement character).
pub fn jis_slice_to_string(text_vram: &[u8], byte_offset: usize, count: usize) -> String {
    let mut result = String::with_capacity(count);
    for i in 0..count {
        let offset = byte_offset + i * 2;
        if offset + 1 >= text_vram.len() {
            break;
        }
        let even = text_vram[offset];
        let odd = text_vram[offset + 1];
        let jis = JisChar::from_vram_bytes(even, odd);
        match jis_to_char(jis) {
            Some(ch) => result.push(ch),
            None => result.push('\u{FFFD}'),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ank_ascii_round_trip() {
        for byte in 0x20u8..=0x7D {
            let ch = byte as char;
            let jis = char_to_jis(ch);
            assert!(jis.is_ank());
            assert_eq!(jis.as_u16(), byte as u16);
            if byte == 0x5C {
                // ANK 0x5C displays as yen sign on PC-98 (JIS X 0201)
                assert_eq!(jis_to_char(jis), Some('\u{00A5}'));
            } else {
                assert_eq!(jis_to_char(jis), Some(ch));
            }
        }
    }

    #[test]
    fn ank_yen_sign() {
        let jis = char_to_jis('\u{00A5}');
        assert!(jis.is_ank());
        assert_eq!(jis.as_u16(), 0x5C);
        assert_eq!(jis_to_char(jis), Some('\u{00A5}'));
    }

    #[test]
    fn ank_overline() {
        let jis = char_to_jis('\u{203E}');
        assert!(jis.is_ank());
        assert_eq!(jis.as_u16(), 0x7E);
        assert_eq!(jis_to_char(jis), Some('\u{203E}'));
    }

    #[test]
    fn ank_halfwidth_katakana() {
        // U+FF61 (。) -> 0xA1
        let jis = char_to_jis('\u{FF61}');
        assert!(jis.is_ank());
        assert_eq!(jis.as_u16(), 0xA1);
        assert_eq!(jis_to_char(jis), Some('\u{FF61}'));

        // U+FF9F (゚) -> 0xDF
        let jis = char_to_jis('\u{FF9F}');
        assert!(jis.is_ank());
        assert_eq!(jis.as_u16(), 0xDF);
        assert_eq!(jis_to_char(jis), Some('\u{FF9F}'));
    }

    #[test]
    fn jis_x0208_ideographic_space() {
        // JIS 0x2121 = U+3000 (ideographic space)
        let jis = JisChar::from_u16(0x2121);
        assert!(!jis.is_ank());
        assert_eq!(jis_to_char(jis), Some('\u{3000}'));
        assert_eq!(char_to_jis('\u{3000}'), jis);
    }

    #[test]
    fn jis_x0208_common_kanji() {
        // 人 = U+4EBA, JIS 0x3F4D
        let jis = char_to_jis('人');
        assert_eq!(jis_to_char(jis), Some('人'));

        // 日 = U+65E5
        let jis = char_to_jis('日');
        assert_eq!(jis_to_char(jis), Some('日'));
    }

    #[test]
    fn jis_x0208_fullwidth_latin() {
        // Ａ = U+FF21, JIS 0x2341
        let jis = char_to_jis('\u{FF21}');
        assert_eq!(jis_to_char(jis), Some('\u{FF21}'));
    }

    #[test]
    fn unmapped_emoji_returns_replacement() {
        let jis = char_to_jis('😀');
        assert_eq!(jis, JisChar::REPLACEMENT);
    }

    #[test]
    fn invalid_jis_returns_none() {
        // JIS code with ku/ten outside valid range
        assert_eq!(jis_to_char(JisChar::from_u16(0x7F7F)), None);
        assert_eq!(jis_to_char(JisChar::from_u16(0x2020)), None);
    }

    #[test]
    fn jis_char_vram_bytes_round_trip() {
        // Kanji: JIS 0x2121 (ku=0x21, ten=0x21)
        // VRAM: even = ku - 0x20 = 0x01, odd = ten = 0x21
        let jis = JisChar::from_u16(0x2121);
        let (even, odd) = jis.to_vram_bytes();
        assert_eq!(even, 0x01); // row code (ku - 0x20)
        assert_eq!(odd, 0x21); // column code (ten)
        assert_eq!(JisChar::from_vram_bytes(even, odd), jis);

        // Non-symmetric kanji: JIS 0x2821 (ku=0x28, ten=0x21)
        // VRAM: even = 0x08, odd = 0x21
        let jis = JisChar::from_u16(0x2821);
        let (even, odd) = jis.to_vram_bytes();
        assert_eq!(even, 0x08); // row code (ku - 0x20)
        assert_eq!(odd, 0x21); // column code (ten)
        assert_eq!(JisChar::from_vram_bytes(even, odd), jis);

        // ANK
        let jis = JisChar::from_u16(0x0041);
        let (even, odd) = jis.to_vram_bytes();
        assert_eq!(even, 0x41); // 'A'
        assert_eq!(odd, 0x00);
        assert_eq!(JisChar::from_vram_bytes(even, odd), jis);
    }

    #[test]
    fn jis_char_is_ank() {
        assert!(JisChar::from_u16(0x0041).is_ank());
        assert!(JisChar::SPACE.is_ank());
        assert!(!JisChar::from_u16(0x2121).is_ank());
        assert!(!JisChar::REPLACEMENT.is_ank());
    }

    #[test]
    fn jis_char_display_width_matches_halfwidth_blocks() {
        assert_eq!(JisChar::from_u16(0x0041).display_width(), 1);
        assert_eq!(JisChar::from_u16(0x2B30).display_width(), 1);
        assert_eq!(JisChar::from_u16(0x2121).display_width(), 2);
    }

    #[test]
    fn str_to_jis_ascii() {
        let jis_chars = str_to_jis("Hello");
        assert_eq!(jis_chars.len(), 5);
        for jis in &jis_chars {
            assert!(jis.is_ank());
        }
        assert_eq!(jis_chars[0].as_u16(), b'H' as u16);
        assert_eq!(jis_chars[4].as_u16(), b'o' as u16);
    }

    #[test]
    fn jis_slice_to_string_basic() {
        // Write "AB" as ANK into a VRAM-like buffer
        let vram = [0x41, 0x00, 0x42, 0x00];
        let s = jis_slice_to_string(&vram, 0, 2);
        assert_eq!(s, "AB");
    }

    #[test]
    fn jis_slice_to_string_kanji() {
        // Ideographic space (JIS 0x2121, ku=0x21, ten=0x21)
        // VRAM encoding: even = ku - 0x20 = 0x01, odd = ten = 0x21
        let vram = [0x01, 0x21];
        let s = jis_slice_to_string(&vram, 0, 1);
        assert_eq!(s, "\u{3000}");
    }

    #[test]
    fn unicode_to_jis_table_sorted() {
        for window in UNICODE_TO_JIS.windows(2) {
            assert!(
                window[0].0 < window[1].0,
                "UNICODE_TO_JIS not sorted: 0x{:04X} >= 0x{:04X}",
                window[0].0,
                window[1].0
            );
        }
    }

    #[test]
    fn round_trip_consistency() {
        for (ku_offset, row) in JIS_TO_UNICODE.chunks(94).enumerate() {
            let ku = (ku_offset as u8) + 0x21;
            for (ten_offset, &unicode) in row.iter().enumerate() {
                if unicode == 0 {
                    continue;
                }
                let ten = (ten_offset as u8) + 0x21;
                let jis_code = (ku as u16) << 8 | ten as u16;
                let jis = JisChar::from_u16(jis_code);

                if let Some(ch) = jis_to_char(jis) {
                    let round_tripped = char_to_jis(ch);
                    // Round-trip may map to a different JIS code (CP932 duplicates),
                    // but both should produce the same Unicode character.
                    assert_eq!(
                        jis_to_char(round_tripped),
                        Some(ch),
                        "Round-trip mismatch for JIS 0x{:04X} -> U+{:04X} -> JIS 0x{:04X}",
                        jis_code,
                        ch as u32,
                        round_tripped.as_u16()
                    );
                }
            }
        }
    }

    #[test]
    fn shift_jis_lead_byte_ranges() {
        assert!(is_shift_jis_lead_byte(0x81));
        assert!(is_shift_jis_lead_byte(0x9F));
        assert!(is_shift_jis_lead_byte(0xE0));
        assert!(is_shift_jis_lead_byte(0xFC));
        assert!(!is_shift_jis_lead_byte(0x80));
        assert!(!is_shift_jis_lead_byte(0xA0));
    }

    #[test]
    fn shift_jis_trail_byte_ranges() {
        assert!(is_shift_jis_trail_byte(0x40));
        assert!(is_shift_jis_trail_byte(0x7E));
        assert!(is_shift_jis_trail_byte(0x80));
        assert!(is_shift_jis_trail_byte(0xFC));
        assert!(!is_shift_jis_trail_byte(0x7F));
        assert!(!is_shift_jis_trail_byte(0xFD));
    }

    #[test]
    fn shift_jis_pair_to_jis_ideographic_space() {
        assert_eq!(
            shift_jis_pair_to_jis(0x81, 0x40),
            Some(JisChar::from_u16(0x2121))
        );
    }

    #[test]
    fn shift_jis_pair_to_jis_hiragana_a() {
        assert_eq!(
            shift_jis_pair_to_jis(0x82, 0xA0),
            Some(JisChar::from_u16(0x2422))
        );
    }

    #[test]
    fn shift_jis_pair_to_jis_rejects_invalid_pairs() {
        assert_eq!(shift_jis_pair_to_jis(0x20, 0xA0), None);
        assert_eq!(shift_jis_pair_to_jis(0x82, 0x7F), None);
    }

    #[test]
    fn jis_to_shift_jis_encodes_ank() {
        assert_eq!(
            jis_to_shift_jis(JisChar::from_u16(0x41)),
            Some(ShiftJisChar::Single(0x41))
        );
    }

    #[test]
    fn jis_to_shift_jis_encodes_double_byte() {
        assert_eq!(
            jis_to_shift_jis(JisChar::from_u16(0x2422)),
            Some(ShiftJisChar::Double([0x82, 0xA0]))
        );
    }
}
