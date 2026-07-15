//! CGROM (Character Generator ROM) address controller.
//!
//! Manages the character code, line, and left/right selector registers
//! used to address glyph data in the font ROM.
//!
//! Ports: 0xA1 W (code high), 0xA3 W (code low), 0xA5 W (line selector).
//! Read/write of glyph data at port 0xA9 is handled by the bus using
//! the address computed by this device.

use common::{JisChar, TextExtractor};

/// Converts a `state.code` value into a [`JisChar`] using the PC-98
/// CGROM port convention.
///
/// For full-width characters, port 0xA1 carries the JIS column (ten) in
/// the high byte of `code`, and port 0xA3 carries `ku - 0x20` in the
/// low byte. To reconstruct a proper JIS code `(ku << 8) | ten`, we
/// add 0x20 back to the low byte and swap the two bytes.
///
/// For ANK / half-width characters (`code & 0xFF00 == 0`), the low byte
/// is the character code directly and [`JisChar::from_u16`] does the
/// right thing.
fn decode_jis_from_state_code(code: u16) -> JisChar {
    if (code & 0xFF00) == 0 {
        JisChar::from_u16(code)
    } else {
        let ten = u16::from((code >> 8) as u8);
        let ku = u16::from(((code & 0x00FF) as u8).wrapping_add(0x20));
        JisChar::from_u16((ku << 8) | ten)
    }
}

/// Line selector mask: bits 0-4 select glyph scanline (0-31).
const LINE_MASK: u8 = 0x1F;

/// Line selector bit 5 (inverted) produces left/right half offset.
/// When bit 5 is 0 (inverted = 1), lr = 0x800 (right half).
/// When bit 5 is 1 (inverted = 0), lr = 0x000 (left half).
const LR_SELECT_BIT: u8 = 0x20;

/// Left/right half offset value: 0x800 bytes into the character glyph.
const LR_OFFSET: u16 = 0x0800;

/// Code mask for 7-bit encoding in both high and low bytes.
const CODE_7BIT_MASK: u16 = 0x7F7F;

/// Base offset for half-width (ANK) characters in the font ROM.
const HALFWIDTH_BASE: usize = 0x80000;

/// Line bit 4: when set for half-width characters, the read returns 0.
const LINE_HALFWIDTH_OVERFLOW: u8 = 0x10;

/// Low nibble mask for line within a 16-line glyph.
const LINE_LOW_MASK: u8 = 0x0F;

/// User-definable character range: code low byte mask and value.
/// Characters with (code & 0x007E) == 0x0056 are writable.
const USER_CHAR_MASK: u16 = 0x007E;
const USER_CHAR_VALUE: u16 = 0x0056;

/// KAC code-access bank offset used for the CG window path.
const KAC_CODE_ACCESS_OFFSET: usize = 0x1000;

/// Default "low" address for the CG window - points to an inert region
/// near the end of font ROM (reads return whatever is there, effectively 0).
const CG_WINDOW_DEFAULT_LOW: usize = 0x7FFF0;

/// CG window address computation result.
///
/// The CG window maps 0xA4000–0xA4FFF in physical memory to font ROM.
/// Even-offset bytes read from `low`, odd-offset bytes read from `high`.
/// Writes go to `high` only when `writable` is true and the address is odd.
pub struct CgWindow {
    /// Font ROM base for even-offset reads: `fontrom[low + ((addr >> 1) & 0xF)]`.
    pub low: usize,
    /// Font ROM base for odd-offset reads/writes: `fontrom[high + ((addr >> 1) & 0xF)]`.
    pub high: usize,
    /// Whether writes to odd addresses are permitted (user-definable chars only).
    pub writable: bool,
}

save_state::runtime_state! {
/// Authoritative CGROM controller state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgromState {
    /// Character code register (set via ports 0xA1/0xA3).
    pub code: u16,
    /// Line selector, bits 0-4 (set via port 0xA5).
    pub line: u8,
    /// Left/right half offset: 0 or 0x800 (derived from port 0xA5 bit 5).
    pub lr: u16,
    /// When true, all CG addresses are writable (VX+ CG RAM).
    /// When false, only user-definable characters are writable (VM CG ROM).
    pub cg_ram: bool,
}}

/// CGROM address controller.
pub struct Cgrom {
    /// Embedded state for save/restore.
    pub state: CgromState,
}

impl_simple_state_accessors!(Cgrom, CgromState);

impl Default for Cgrom {
    fn default() -> Self {
        Self::new()
    }
}

impl Cgrom {
    /// Creates a new CGROM controller with all registers zeroed.
    pub fn new() -> Self {
        Self {
            state: CgromState {
                code: 0,
                line: 0,
                lr: 0,
                cg_ram: false,
            },
        }
    }

    /// Reports the currently addressed glyph to the given sink.
    pub fn notify_read(&self, extractor: &mut dyn TextExtractor) {
        extractor.push_jis(decode_jis_from_state_code(self.state.code));
    }

    /// Writes the code high byte (port 0xA1).
    pub fn write_code_high(&mut self, value: u8) {
        self.state.code = (value as u16) << 8 | (self.state.code & 0x00FF);
    }

    /// Writes the code low byte (port 0xA3).
    pub fn write_code_low(&mut self, value: u8) {
        self.state.code = (self.state.code & 0xFF00) | value as u16;
    }

    /// Writes the line selector (port 0xA5).
    pub fn write_line_selector(&mut self, value: u8) {
        self.state.line = value & LINE_MASK;
        self.state.lr = if value & LR_SELECT_BIT == 0 {
            LR_OFFSET
        } else {
            0
        };
    }

    /// Computes the font ROM read address for port 0xA9 reads.
    ///
    /// Returns `Some(address)` if the read is valid, `None` if it would
    /// return 0 (out-of-range half-width character with line bit 4 set).
    pub fn read_address(&self, kac_dot_access_mode: bool) -> Option<usize> {
        let code = self.state.code;
        let low_byte = code & 0x00FF;
        let kac_offset = if kac_dot_access_mode {
            0
        } else {
            KAC_CODE_ACCESS_OFFSET
        };

        if (0x09..0x0C).contains(&low_byte) {
            if self.state.lr == 0 {
                let offset = ((code & CODE_7BIT_MASK) as usize) << 4;
                Some(offset + kac_offset + (self.state.line & LINE_LOW_MASK) as usize)
            } else {
                None
            }
        } else if code & 0xFF00 != 0 {
            let offset = ((code & CODE_7BIT_MASK) as usize) << 4;
            Some(
                offset
                    + kac_offset
                    + self.state.lr as usize
                    + (self.state.line & LINE_LOW_MASK) as usize,
            )
        } else if self.state.line & LINE_HALFWIDTH_OVERFLOW == 0 {
            Some(HALFWIDTH_BASE + ((code as usize) << 4) + self.state.line as usize)
        } else {
            None
        }
    }

    /// Computes the CG window mapping for memory-mapped access at 0xA4000–0xA4FFF.
    ///
    /// Only meaningful on VX+ machines (GRCG chip >= 2). On VM, the caller should
    /// not route through this path.
    ///
    /// `font_8x16_mode`: true when mode1 bit 3 (font select) is set, which
    /// selects the 8x16-cell font (7x13-dot font in 400-line). Affects the
    /// half-width character offset (+0x2000 when false, i.e. 6x8 mode).
    pub fn compute_window(&self, font_8x16_mode: bool) -> CgWindow {
        let code = self.state.code;
        let low_byte = (code & 0x007F) as u8;

        if code & 0xFF00 == 0 {
            // Half-width (ANK) character.
            let mut high = HALFWIDTH_BASE + ((code as usize) << 4);
            if !font_8x16_mode {
                high += 0x2000;
            }
            CgWindow {
                low: CG_WINDOW_DEFAULT_LOW,
                high,
                writable: false,
            }
        } else {
            // Full-width (kanji) character.
            let mut high = ((code & CODE_7BIT_MASK) as usize) << 4;
            let mut low = CG_WINDOW_DEFAULT_LOW;
            let mut writable = false;

            if (0x56..0x58).contains(&low_byte) {
                // User-definable characters - writable.
                writable = true;
                high += self.state.lr as usize;
            } else if (0x09..0x0C).contains(&low_byte) {
                // Special range: right half is unmapped.
                if self.state.lr != 0 {
                    high = CG_WINDOW_DEFAULT_LOW;
                }
            } else if (0x0C..0x10).contains(&low_byte) || (0x58..0x60).contains(&low_byte) {
                high += self.state.lr as usize;
            } else {
                // Default: low = base, high = base + 0x800.
                low = high;
                high += LR_OFFSET as usize;
            }

            CgWindow {
                low,
                high,
                writable,
            }
        }
    }

    /// Computes the font ROM write address for port 0xA9 writes.
    ///
    /// On VM (CG ROM): returns `Some(address)` only for user-definable
    /// characters (code low byte matching `USER_CHAR_VALUE`).
    /// On VX+ (CG RAM): returns `Some(address)` for all valid characters.
    pub fn write_address(&self, kac_dot_access_mode: bool) -> Option<usize> {
        if self.state.cg_ram {
            self.read_address(kac_dot_access_mode)
        } else if (self.state.code & USER_CHAR_MASK) == USER_CHAR_VALUE {
            let offset = ((self.state.code & CODE_7BIT_MASK) as usize) << 4;
            let kac_offset = if kac_dot_access_mode {
                0
            } else {
                KAC_CODE_ACCESS_OFFSET
            };
            Some(
                offset
                    + kac_offset
                    + self.state.lr as usize
                    + (self.state.line & LINE_LOW_MASK) as usize,
            )
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use common::{JisChar, TextExtractor};

    use super::{Cgrom, HALFWIDTH_BASE, KAC_CODE_ACCESS_OFFSET};

    struct CollectingExtractor {
        pushes: Rc<RefCell<Vec<JisChar>>>,
    }

    impl TextExtractor for CollectingExtractor {
        fn push_jis(&mut self, code: JisChar) {
            self.pushes.borrow_mut().push(code);
        }

        fn tick(&mut self) {}
    }

    #[test]
    fn lr_offset_follows_line_selector_bit5() {
        let mut cgrom = Cgrom::new();

        cgrom.write_line_selector(0x00);
        assert_eq!(cgrom.state.lr, 0x0800);

        cgrom.write_line_selector(0x20);
        assert_eq!(cgrom.state.lr, 0x0000);
    }

    #[test]
    fn kac_code_access_adds_offset_for_fullwidth_reads() {
        let mut cgrom = Cgrom::new();
        cgrom.write_code_high(0x21);
        cgrom.write_code_low(0x21);
        cgrom.write_line_selector(0x23);

        let dot_access_address = cgrom.read_address(true).unwrap();
        let code_access_address = cgrom.read_address(false).unwrap();
        assert_eq!(
            code_access_address,
            dot_access_address + KAC_CODE_ACCESS_OFFSET
        );
    }

    #[test]
    fn kac_mode_does_not_shift_halfwidth_reads() {
        let mut cgrom = Cgrom::new();
        cgrom.write_code_high(0x00);
        cgrom.write_code_low(0x41);
        cgrom.write_line_selector(0x23);

        let dot_access_address = cgrom.read_address(true).unwrap();
        let code_access_address = cgrom.read_address(false).unwrap();
        assert_eq!(
            dot_access_address,
            HALFWIDTH_BASE + (0x41usize << 4) + 3,
            "halfwidth mapping should stay in ANK bank",
        );
        assert_eq!(code_access_address, dot_access_address);
    }

    #[test]
    fn kac_code_access_adds_offset_for_user_font_writes() {
        let mut cgrom = Cgrom::new();
        cgrom.write_code_high(0x21);
        cgrom.write_code_low(0x56);
        cgrom.write_line_selector(0x23);

        let dot_access_address = cgrom.write_address(true).unwrap();
        let code_access_address = cgrom.write_address(false).unwrap();
        assert_eq!(
            code_access_address,
            dot_access_address + KAC_CODE_ACCESS_OFFSET
        );
    }

    #[test]
    fn decode_jis_from_state_code_handles_fullwidth() {
        // JIS 0x2121 stored as ten=0x21 in high byte, ku-0x20=0x01 in low byte.
        assert_eq!(
            super::decode_jis_from_state_code(0x2101),
            JisChar::from_u16(0x2121)
        );
        // JIS 0x4F50 stored as ten=0x50, ku-0x20=0x2F.
        assert_eq!(
            super::decode_jis_from_state_code(0x502F),
            JisChar::from_u16(0x4F50)
        );
    }

    #[test]
    fn decode_jis_from_state_code_handles_ank() {
        assert_eq!(
            super::decode_jis_from_state_code(0x0041),
            JisChar::from_u16(0x0041)
        );
    }

    #[test]
    fn notify_read_pushes_ank_code() {
        let pushes = Rc::new(RefCell::new(Vec::new()));
        let mut sink = CollectingExtractor {
            pushes: pushes.clone(),
        };
        let mut cgrom = Cgrom::new();

        cgrom.write_code_high(0x00);
        cgrom.write_code_low(0x41); // ANK 'A'
        cgrom.notify_read(&mut sink);

        let pushes = pushes.borrow();
        assert_eq!(pushes.len(), 1);
        assert_eq!(pushes[0], JisChar::from_u16(0x0041));
        assert!(pushes[0].is_ank());
    }

    #[test]
    fn notify_read_pushes_fullwidth_code() {
        let pushes = Rc::new(RefCell::new(Vec::new()));
        let mut sink = CollectingExtractor {
            pushes: pushes.clone(),
        };
        let mut cgrom = Cgrom::new();

        // JIS 0x2121 (full-width space): port 0xA1 = ten = 0x21,
        // port 0xA3 = ku - 0x20 = 0x01.
        cgrom.write_code_high(0x21);
        cgrom.write_code_low(0x01);
        cgrom.notify_read(&mut sink);

        let pushes = pushes.borrow();
        assert_eq!(pushes.len(), 1);
        assert_eq!(pushes[0], JisChar::from_u16(0x2121));
    }
}
