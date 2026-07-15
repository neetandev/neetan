//! FM-7 keyboard.
//!
//! The keyboard translates an FM-7 physical scancode into a 9-bit F-BASIC keycode
//! using one of eight modifier-selected tables, queues it in a small FIFO, and
//! drips one entry at a time into the read latch on a periodic latch event. The
//! latch drives a single interrupt line that the bus fans out to the main CPU IRQ
//! and the sub CPU FIRQ. The BREAK key is handled separately: it never enters the
//! keycode path and instead reports a level-tracked pressed state that the bus
//! turns into the main CPU FIRQ.
//!
//! The base FM-7 keyboard is modeled here. The FM-77AV serial encoder
//! (`0xD431`/`0xD432`, LEDs, programmable repeat, RTC) lives in the [`encoder`]
//! submodule and is driven from the bus.

pub mod encoder;
mod keytables;

use keytables::{FM16BETA_TABLES, KeycodeTables, SCANCODE_COUNT, STANDARD_TABLES};

/// Translated keycode table set selected by the FM-77AV encoder scancode mode.
/// The base FM-7 always uses the standard set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeycodeTableSet {
    /// Standard F-BASIC keycodes.
    Standard,
    /// FM-16beta compatible keycodes.
    Fm16Beta,
}

impl KeycodeTableSet {
    /// The modifier-selected tables of this set.
    fn tables(self) -> &'static KeycodeTables {
        match self {
            Self::Standard => &STANDARD_TABLES,
            Self::Fm16Beta => &FM16BETA_TABLES,
        }
    }
}

/// Physical scancode of the CTRL key.
const SCANCODE_CTRL: u8 = 0x52;
/// Physical scancode of the left SHIFT key.
const SCANCODE_SHIFT_LEFT: u8 = 0x53;
/// Physical scancode of the right SHIFT key.
const SCANCODE_SHIFT_RIGHT: u8 = 0x54;
/// Physical scancode of the CAPS key (toggles on press).
const SCANCODE_CAPS: u8 = 0x55;
/// Physical scancode of the GRAPH key.
const SCANCODE_GRAPH: u8 = 0x56;
/// Physical scancode of the KANA key (toggles on press).
const SCANCODE_KANA: u8 = 0x5A;
/// Physical scancode of the BREAK key.
const SCANCODE_BREAK: u8 = 0x5C;

/// Ninth bit of a keycode, presented on `0xFD00`/`0xD400` bit 7.
const KEYCODE_HIGH_BIT: u16 = 0x0100;

/// Release flag OR-ed onto a physical scancode in the raw scancode mode.
const SCAN_RELEASE_BIT: u16 = 0x80;

/// First lowercase ASCII letter subject to caps-lock case folding.
const LOWERCASE_FIRST: u16 = 0x61;
/// Last lowercase ASCII letter subject to caps-lock case folding.
const LOWERCASE_LAST: u16 = 0x7A;
/// First uppercase ASCII letter subject to caps-lock case folding.
const UPPERCASE_FIRST: u16 = 0x41;
/// Last uppercase ASCII letter subject to caps-lock case folding.
const UPPERCASE_LAST: u16 = 0x5A;
/// Distance between an ASCII letter's uppercase and lowercase forms.
const CASE_DISTANCE: u16 = 0x20;

/// Depth of the pending keycode FIFO. The oldest entry is dropped on overflow.
const KEY_FIFO_CAPACITY: usize = 16;

save_state::runtime_state! {
/// Fixed-capacity FIFO of pending keycodes, dropping the oldest on overflow.
#[derive(Clone)]
struct KeyFifo {
    entries: [u16; KEY_FIFO_CAPACITY],
    head: usize,
    len: usize,
}}

impl KeyFifo {
    /// Creates an empty FIFO.
    fn new() -> Self {
        Self {
            entries: [0; KEY_FIFO_CAPACITY],
            head: 0,
            len: 0,
        }
    }

    /// Appends `code`, dropping the oldest entry when the FIFO is full.
    fn push(&mut self, code: u16) {
        if self.len == KEY_FIFO_CAPACITY {
            self.head = (self.head + 1) % KEY_FIFO_CAPACITY;
            self.len -= 1;
        }
        let tail = (self.head + self.len) % KEY_FIFO_CAPACITY;
        self.entries[tail] = code;
        self.len += 1;
    }

    /// Removes and returns the oldest entry, if any.
    fn pop(&mut self) -> Option<u16> {
        if self.len == 0 {
            return None;
        }
        let code = self.entries[self.head];
        self.head = (self.head + 1) % KEY_FIFO_CAPACITY;
        self.len -= 1;
        Some(code)
    }
}

save_state::runtime_state! {
/// Base FM-7 keyboard state: modifiers, the pending-keycode FIFO, and the read
/// latch with its interrupt line.
#[derive(Clone)]
pub struct Fm7Keyboard {
    shift_left: bool,
    shift_right: bool,
    ctrl: bool,
    graph: bool,
    caps: bool,
    kana: bool,
    break_pressed: bool,
    /// Non-modifier key currently held, used to suppress host auto-repeat.
    held_scancode: Option<u8>,
    fifo: KeyFifo,
    /// The 9-bit keycode presented on the read ports.
    keycode: u16,
    /// State of the shared interrupt line raised by a latched keycode.
    interrupt_asserted: bool,
}}

impl Fm7Keyboard {
    /// Validates FIFO indexes and keycode width.
    pub fn validate_runtime_state(&self) -> Result<(), save_state::StateValidationError> {
        if self.fifo.head >= KEY_FIFO_CAPACITY
            || self.fifo.len > KEY_FIFO_CAPACITY
            || self.keycode > 0x01FF
        {
            return Err(save_state::StateValidationError::new(
                "FM-7 keyboard state is invalid",
            ));
        }
        Ok(())
    }
}

impl Fm7Keyboard {
    /// Creates a keyboard with all keys released and no pending codes.
    pub fn new() -> Self {
        Self {
            shift_left: false,
            shift_right: false,
            ctrl: false,
            graph: false,
            caps: false,
            kana: false,
            break_pressed: false,
            held_scancode: None,
            fifo: KeyFifo::new(),
            keycode: 0,
            interrupt_asserted: false,
        }
    }

    /// Applies a key event: `scancode` is an FM-7 physical scancode and `pressed`
    /// distinguishes a make from a break. Modifier keys only update internal
    /// state; other keys enqueue their keycode on press, translated through
    /// `table_set`. The translated modes emit make codes only, so a release
    /// enqueues nothing.
    pub fn push(&mut self, scancode: u8, pressed: bool, table_set: KeycodeTableSet) {
        if Self::is_modifier(scancode) {
            self.set_modifier(scancode, pressed);
            return;
        }
        if !pressed {
            if self.held_scancode == Some(scancode) {
                self.held_scancode = None;
            }
            return;
        }
        if self.held_scancode == Some(scancode) {
            return;
        }
        self.held_scancode = Some(scancode);
        let code = self.translate(scancode, table_set);
        if code != 0 {
            self.fifo.push(code);
        }
    }

    /// Applies a key event in the FM-77AV raw scancode mode: every key,
    /// modifiers included, enqueues its physical scancode on press and the
    /// scancode with [`SCAN_RELEASE_BIT`] set on release. Modifier state still
    /// tracks so a later switch back to a translated mode starts consistent.
    pub fn push_scan(&mut self, scancode: u8, pressed: bool) {
        if scancode == 0 {
            return;
        }
        if Self::is_modifier(scancode) {
            self.set_modifier(scancode, pressed);
        } else if pressed {
            if self.held_scancode == Some(scancode) {
                return;
            }
            self.held_scancode = Some(scancode);
        } else if self.held_scancode == Some(scancode) {
            self.held_scancode = None;
        }
        let code = if pressed {
            u16::from(scancode)
        } else {
            u16::from(scancode) | SCAN_RELEASE_BIT
        };
        self.fifo.push(code);
    }

    /// Whether `scancode` is one of the modifier keys tracked without a keycode.
    fn is_modifier(scancode: u8) -> bool {
        matches!(
            scancode,
            SCANCODE_CTRL
                | SCANCODE_SHIFT_LEFT
                | SCANCODE_SHIFT_RIGHT
                | SCANCODE_CAPS
                | SCANCODE_GRAPH
                | SCANCODE_KANA
                | SCANCODE_BREAK
        )
    }

    /// Updates the modifier latches for a modifier `scancode`. CAPS and KANA
    /// toggle on press; the others track the physical key level.
    fn set_modifier(&mut self, scancode: u8, pressed: bool) {
        match scancode {
            SCANCODE_CTRL => self.ctrl = pressed,
            SCANCODE_SHIFT_LEFT => self.shift_left = pressed,
            SCANCODE_SHIFT_RIGHT => self.shift_right = pressed,
            SCANCODE_GRAPH => self.graph = pressed,
            SCANCODE_BREAK => self.break_pressed = pressed,
            SCANCODE_CAPS if pressed => self.caps = !self.caps,
            SCANCODE_KANA if pressed => self.kana = !self.kana,
            _ => {}
        }
    }

    /// Translates `scancode` into its 9-bit keycode for the current modifier
    /// state, selecting the matching table of `table_set` and applying caps-lock
    /// case folding on the unmodified path.
    fn translate(&self, scancode: u8, table_set: KeycodeTableSet) -> u16 {
        let index = usize::from(scancode);
        if index >= SCANCODE_COUNT {
            return 0;
        }
        let tables = table_set.tables();
        let shift = self.shift_left || self.shift_right;
        if self.ctrl {
            return if shift {
                tables.ctrl_shift[index]
            } else {
                tables.ctrl[index]
            };
        }
        if self.graph {
            return if shift {
                tables.graph_shift[index]
            } else {
                tables.graph[index]
            };
        }
        if self.kana {
            return if shift {
                tables.kana_shift[index]
            } else {
                tables.kana[index]
            };
        }
        let code = if shift {
            tables.shift[index]
        } else {
            tables.normal[index]
        };
        if self.caps { fold_case(code) } else { code }
    }

    /// Moves the next pending keycode into the read latch and asserts the
    /// interrupt line. Returns whether a keycode was latched.
    pub fn latch_next(&mut self) -> bool {
        match self.fifo.pop() {
            Some(code) => {
                self.keycode = code;
                self.interrupt_asserted = true;
                true
            }
            None => false,
        }
    }

    /// Whether the latched keycode's ninth bit is set (`0xFD00`/`0xD400` bit 7).
    pub fn keycode_high(&self) -> bool {
        self.keycode & KEYCODE_HIGH_BIT != 0
    }

    /// Reads the low byte of the latched keycode and clears the interrupt line,
    /// matching a main `0xFD01` or sub `0xD401` read.
    pub fn read_low(&mut self) -> u8 {
        self.interrupt_asserted = false;
        self.keycode as u8
    }

    /// Whether the keycode interrupt line is currently asserted.
    pub fn interrupt_asserted(&self) -> bool {
        self.interrupt_asserted
    }

    /// Whether the BREAK key is currently held.
    pub fn break_pressed(&self) -> bool {
        self.break_pressed
    }

    /// Whether `scancode` is a key the FM-77AV encoder may auto-repeat: a
    /// non-modifier key within the repeatable range.
    pub fn is_repeatable(scancode: u8) -> bool {
        !Self::is_modifier(scancode) && encoder::is_repeatable_scancode(scancode)
    }

    /// Re-enqueues the translated keycode for `scancode`, used by the FM-77AV
    /// encoder to generate an auto-repeat keystroke.
    pub fn enqueue_repeat(&mut self, scancode: u8, table_set: KeycodeTableSet) {
        let code = self.translate(scancode, table_set);
        if code != 0 {
            self.fifo.push(code);
        }
    }
}

impl Default for Fm7Keyboard {
    fn default() -> Self {
        Self::new()
    }
}

/// Swaps the case of an ASCII letter keycode for caps-lock; other codes are
/// returned unchanged.
fn fold_case(code: u16) -> u16 {
    match code {
        LOWERCASE_FIRST..=LOWERCASE_LAST => code - CASE_DISTANCE,
        UPPERCASE_FIRST..=UPPERCASE_LAST => code + CASE_DISTANCE,
        _ => code,
    }
}
