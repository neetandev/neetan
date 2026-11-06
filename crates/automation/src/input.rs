//! Logical key and text mapping for the automation frontend.
//!
//! Maps the Scheme-facing key names onto the neutral [`HostKey`] identity, and
//! the ASCII characters onto the host keys of the keyboard layout of each
//! machine family. This layer is SDL-free: it never references an
//! SDL scan code. Each machine translates the resulting host key into its own
//! native scan code.

use common::{HostKey, JoystickState};

/// Resolves a Scheme key-name symbol to a host key.
///
/// Names are lowercase. Common aliases are accepted. Returns `None` for an
/// unknown name.
pub fn key_from_name(name: &str) -> Option<HostKey> {
    let key = match name {
        "a" => HostKey::A,
        "b" => HostKey::B,
        "c" => HostKey::C,
        "d" => HostKey::D,
        "e" => HostKey::E,
        "f" => HostKey::F,
        "g" => HostKey::G,
        "h" => HostKey::H,
        "i" => HostKey::I,
        "j" => HostKey::J,
        "k" => HostKey::K,
        "l" => HostKey::L,
        "m" => HostKey::M,
        "n" => HostKey::N,
        "o" => HostKey::O,
        "p" => HostKey::P,
        "q" => HostKey::Q,
        "r" => HostKey::R,
        "s" => HostKey::S,
        "t" => HostKey::T,
        "u" => HostKey::U,
        "v" => HostKey::V,
        "w" => HostKey::W,
        "x" => HostKey::X,
        "y" => HostKey::Y,
        "z" => HostKey::Z,
        "0" => HostKey::Digit0,
        "1" => HostKey::Digit1,
        "2" => HostKey::Digit2,
        "3" => HostKey::Digit3,
        "4" => HostKey::Digit4,
        "5" => HostKey::Digit5,
        "6" => HostKey::Digit6,
        "7" => HostKey::Digit7,
        "8" => HostKey::Digit8,
        "9" => HostKey::Digit9,
        "space" => HostKey::Space,
        "minus" => HostKey::Minus,
        "equals" | "caret" => HostKey::Equals,
        "backslash" => HostKey::Backslash,
        "nonusbackslash" | "underscore" => HostKey::NonUsBackslash,
        "grave" | "at" => HostKey::Grave,
        "leftbracket" => HostKey::LeftBracket,
        "rightbracket" => HostKey::RightBracket,
        "semicolon" => HostKey::Semicolon,
        "comma" => HostKey::Comma,
        "period" => HostKey::Period,
        "slash" => HostKey::Slash,
        "apostrophe" | "colon" => HostKey::Apostrophe,
        "esc" | "escape" => HostKey::Escape,
        "bs" | "backspace" => HostKey::Backspace,
        "tab" => HostKey::Tab,
        "return" | "enter" => HostKey::Return,
        "ins" | "insert" => HostKey::Insert,
        "del" | "delete" => HostKey::Delete,
        "home" => HostKey::Home,
        "end" | "help" => HostKey::End,
        "up" => HostKey::Up,
        "down" => HostKey::Down,
        "left" => HostKey::Left,
        "right" => HostKey::Right,
        "pageup" | "rollup" => HostKey::PageUp,
        "pagedown" | "rolldown" => HostKey::PageDown,
        "pause" | "stop" => HostKey::Pause,
        "printscreen" | "copy" => HostKey::PrintScreen,
        "application" | "nfer" | "muhenkan" => HostKey::Application,
        "kp0" => HostKey::Kp0,
        "kp1" => HostKey::Kp1,
        "kp2" => HostKey::Kp2,
        "kp3" => HostKey::Kp3,
        "kp4" => HostKey::Kp4,
        "kp5" => HostKey::Kp5,
        "kp6" => HostKey::Kp6,
        "kp7" => HostKey::Kp7,
        "kp8" => HostKey::Kp8,
        "kp9" => HostKey::Kp9,
        "kpminus" => HostKey::KpMinus,
        "kpdivide" => HostKey::KpDivide,
        "kpmultiply" => HostKey::KpMultiply,
        "kpplus" => HostKey::KpPlus,
        "kpcomma" => HostKey::KpComma,
        "kpperiod" => HostKey::KpPeriod,
        "kpenter" => HostKey::KpEnter,
        "f1" => HostKey::F1,
        "f2" => HostKey::F2,
        "f3" => HostKey::F3,
        "f4" => HostKey::F4,
        "f5" => HostKey::F5,
        "f6" => HostKey::F6,
        "f7" => HostKey::F7,
        "f8" => HostKey::F8,
        "f9" => HostKey::F9,
        "f10" => HostKey::F10,
        // The PC-98 VF1-VF5 and X68000 XF1-XF5 keys sit on the F11-F15 region.
        "f11" | "vf1" | "xf1" => HostKey::F11,
        "f12" | "vf2" | "xf2" => HostKey::F12,
        "f13" | "vf3" | "xf3" => HostKey::F13,
        "f14" | "vf4" | "xf4" => HostKey::F14,
        "f15" | "vf5" | "xf5" => HostKey::F15,
        "shift" | "leftshift" => HostKey::LeftShift,
        "rightshift" => HostKey::RightShift,
        "ctrl" | "control" | "leftcontrol" => HostKey::LeftControl,
        "rightcontrol" => HostKey::RightControl,
        // Left Alt drives the GRPH key on the JIS families.
        "alt" | "leftalt" | "grph" | "graph" => HostKey::LeftAlt,
        // Right Alt drives the transfer (henkan / XFER) key on the JIS families.
        "rightalt" | "xfer" | "henkan" | "convert" => HostKey::RightAlt,
        "caps" | "capslock" => HostKey::CapsLock,
        "numlock" | "kana" => HostKey::NumLock,
        "international1" | "ro" => HostKey::International1,
        "international2" | "katakana" => HostKey::International2,
        "international3" | "yen" => HostKey::International3,
        "international4" => HostKey::International4,
        "international5" => HostKey::International5,
        _ => return None,
    };
    Some(key)
}

/// A shifted or unshifted host key produced from an ASCII character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharStroke {
    /// Whether the shift modifier must be held to produce the character.
    pub shift: bool,
    /// The base host key.
    pub key: HostKey,
}

/// The character layout of a machine keyboard.
///
/// A layout names the host key and the shift state that type a printable ASCII
/// character on the keyboard of one machine family. The host key is the key
/// that the family translates into the native key carrying the character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardLayout {
    /// PC-9801 keyboard with the key data of the built-in BIOS. The yen key
    /// types `\` and Shift + `@` types `~`. It has no `` ` ``.
    Pc98,
    /// PC-8801 key matrix. It has no `` ` `` and no `~`.
    Pc88,
    /// PC-88VA keyboard: the PC-98 key codes with the PC-8801 character set.
    Pc88Va,
    /// PC-6001 sub-controller. A letter key types the capital letter and
    /// Shift selects the small letter.
    Pc6000,
    /// JIS keyboard of the MSX, FM-7, FM Towns and X68000. Shift + `@` types
    /// `` ` `` and Shift + `^` types `~`.
    Jis,
    /// Sharp X1 keyboard. Shift + `^` types `` ` `` and Shift + `@` types `~`.
    X1,
    /// OADG 106-key keyboard of a DOS/V PC/AT.
    At106,
}

impl KeyboardLayout {
    /// Returns the keyboard layout of an automation target, or `None` for a
    /// target without a known layout.
    pub fn for_target(target: &str) -> Option<Self> {
        let layout = match target {
            "pc98" => Self::Pc98,
            "pc88" => Self::Pc88,
            "pc88va" => Self::Pc88Va,
            "pc60" => Self::Pc6000,
            "msx" | "fm7" | "towns" | "x68k" => Self::Jis,
            "x1" => Self::X1,
            "at" => Self::At106,
            _ => return None,
        };
        Some(layout)
    }

    /// Whether Shift selects the capital letters.
    fn shift_types_capitals(self) -> bool {
        !matches!(self, Self::Pc6000)
    }

    /// The symbol strokes of this layout as `(character, shift, key)` entries.
    fn symbols(self) -> &'static [(char, bool, HostKey)] {
        match self {
            Self::Pc98 => PC98_SYMBOLS,
            Self::X1 => X1_SYMBOLS,
            Self::Pc88 => PC88_SYMBOLS,
            Self::Pc88Va => PC88VA_SYMBOLS,
            Self::Pc6000 => PC6000_SYMBOLS,
            Self::Jis => JIS_SYMBOLS,
            Self::At106 => AT106_SYMBOLS,
        }
    }
}

/// Symbol strokes of the PC-98 keyboard.
const PC98_SYMBOLS: &[(char, bool, HostKey)] = &[
    ('-', false, HostKey::Minus),
    ('=', true, HostKey::Minus),
    ('^', false, HostKey::Equals),
    ('\\', false, HostKey::Backslash),
    ('|', true, HostKey::Backslash),
    ('@', false, HostKey::Grave),
    ('~', true, HostKey::Grave),
    ('[', false, HostKey::LeftBracket),
    ('{', true, HostKey::LeftBracket),
    (';', false, HostKey::Semicolon),
    ('+', true, HostKey::Semicolon),
    (':', false, HostKey::Apostrophe),
    ('*', true, HostKey::Apostrophe),
    (']', false, HostKey::RightBracket),
    ('}', true, HostKey::RightBracket),
    ('<', true, HostKey::Comma),
    ('>', true, HostKey::Period),
    ('_', true, HostKey::NonUsBackslash),
];

/// Symbol strokes of the X1 keyboard.
const X1_SYMBOLS: &[(char, bool, HostKey)] = &[
    ('-', false, HostKey::Minus),
    ('=', true, HostKey::Minus),
    ('^', false, HostKey::Equals),
    ('`', true, HostKey::Equals),
    ('\\', false, HostKey::Backslash),
    ('|', true, HostKey::Backslash),
    ('@', false, HostKey::Grave),
    ('~', true, HostKey::Grave),
    ('[', false, HostKey::LeftBracket),
    ('{', true, HostKey::LeftBracket),
    (';', false, HostKey::Semicolon),
    ('+', true, HostKey::Semicolon),
    (':', false, HostKey::Apostrophe),
    ('*', true, HostKey::Apostrophe),
    (']', false, HostKey::RightBracket),
    ('}', true, HostKey::RightBracket),
    ('<', true, HostKey::Comma),
    ('>', true, HostKey::Period),
    ('_', true, HostKey::NonUsBackslash),
];

/// Symbol strokes of the PC-88VA keyboard.
const PC88VA_SYMBOLS: &[(char, bool, HostKey)] = &[
    ('-', false, HostKey::Minus),
    ('=', true, HostKey::Minus),
    ('^', false, HostKey::Equals),
    ('\\', false, HostKey::Backslash),
    ('|', true, HostKey::Backslash),
    ('@', false, HostKey::Grave),
    ('[', false, HostKey::LeftBracket),
    ('{', true, HostKey::LeftBracket),
    (';', false, HostKey::Semicolon),
    ('+', true, HostKey::Semicolon),
    (':', false, HostKey::Apostrophe),
    ('*', true, HostKey::Apostrophe),
    (']', false, HostKey::RightBracket),
    ('}', true, HostKey::RightBracket),
    ('<', true, HostKey::Comma),
    ('>', true, HostKey::Period),
    ('_', true, HostKey::NonUsBackslash),
];

/// Symbol strokes of the JIS keyboards of the MSX, FM-7, FM Towns and X68000.
const JIS_SYMBOLS: &[(char, bool, HostKey)] = &[
    ('-', false, HostKey::Minus),
    ('=', true, HostKey::Minus),
    ('^', false, HostKey::Equals),
    ('~', true, HostKey::Equals),
    ('\\', false, HostKey::Backslash),
    ('|', true, HostKey::Backslash),
    ('@', false, HostKey::Grave),
    ('`', true, HostKey::Grave),
    ('[', false, HostKey::LeftBracket),
    ('{', true, HostKey::LeftBracket),
    (';', false, HostKey::Semicolon),
    ('+', true, HostKey::Semicolon),
    (':', false, HostKey::Apostrophe),
    ('*', true, HostKey::Apostrophe),
    (']', false, HostKey::RightBracket),
    ('}', true, HostKey::RightBracket),
    ('<', true, HostKey::Comma),
    ('>', true, HostKey::Period),
    ('_', true, HostKey::NonUsBackslash),
];

/// Symbol strokes of the PC-8801 key matrix, whose host keys follow the key
/// positions of a JIS host keyboard.
const PC88_SYMBOLS: &[(char, bool, HostKey)] = &[
    ('-', false, HostKey::Minus),
    ('=', true, HostKey::Minus),
    ('^', false, HostKey::Equals),
    ('\\', false, HostKey::NonUsBackslash),
    ('|', true, HostKey::NonUsBackslash),
    ('@', false, HostKey::LeftBracket),
    ('[', false, HostKey::RightBracket),
    ('{', true, HostKey::RightBracket),
    (';', false, HostKey::Semicolon),
    ('+', true, HostKey::Semicolon),
    (':', false, HostKey::Apostrophe),
    ('*', true, HostKey::Apostrophe),
    (']', false, HostKey::Backslash),
    ('}', true, HostKey::Backslash),
    ('<', true, HostKey::Comma),
    ('>', true, HostKey::Period),
    ('_', true, HostKey::International1),
];

/// Symbol strokes of the PC-6001 sub-controller codes.
const PC6000_SYMBOLS: &[(char, bool, HostKey)] = &[
    ('-', false, HostKey::Minus),
    ('=', true, HostKey::Digit0),
    ('^', false, HostKey::Equals),
    ('[', false, HostKey::LeftBracket),
    (']', false, HostKey::RightBracket),
    (';', false, HostKey::Semicolon),
    (':', true, HostKey::Period),
];

/// Symbol strokes of the OADG 106-key keyboard, whose set-1 scan codes follow
/// the key positions of the host keyboard.
const AT106_SYMBOLS: &[(char, bool, HostKey)] = &[
    ('-', false, HostKey::Minus),
    ('=', true, HostKey::Minus),
    ('^', false, HostKey::Equals),
    ('~', true, HostKey::Equals),
    ('\\', false, HostKey::International3),
    ('|', true, HostKey::International3),
    ('@', false, HostKey::LeftBracket),
    ('`', true, HostKey::LeftBracket),
    ('[', false, HostKey::RightBracket),
    ('{', true, HostKey::RightBracket),
    (']', false, HostKey::Backslash),
    ('}', true, HostKey::Backslash),
    (';', false, HostKey::Semicolon),
    ('+', true, HostKey::Semicolon),
    (':', false, HostKey::Apostrophe),
    ('*', true, HostKey::Apostrophe),
    ('<', true, HostKey::Comma),
    ('>', true, HostKey::Period),
    ('_', true, HostKey::International1),
];

/// Whether a character belongs to the Version-1 text set: printable ASCII plus
/// carriage return, line feed, tab and backspace.
pub fn is_text_character(character: char) -> bool {
    matches!(character, ' '..='~' | '\r' | '\n' | '\t' | '\u{8}')
}

/// Converts a Version-1 text character into a host-key stroke on `layout`.
///
/// Returns `None` for a character outside the text set and for a character
/// that the keyboard of the layout cannot type.
pub fn char_to_stroke(layout: KeyboardLayout, character: char) -> Option<CharStroke> {
    if let Some(key) = letter_key(character.to_ascii_lowercase()) {
        let shift = character.is_ascii_uppercase() == layout.shift_types_capitals();
        return Some(CharStroke { shift, key });
    }
    let (shift, key) = match character {
        '0' => (false, HostKey::Digit0),
        '1' => (false, HostKey::Digit1),
        '2' => (false, HostKey::Digit2),
        '3' => (false, HostKey::Digit3),
        '4' => (false, HostKey::Digit4),
        '5' => (false, HostKey::Digit5),
        '6' => (false, HostKey::Digit6),
        '7' => (false, HostKey::Digit7),
        '8' => (false, HostKey::Digit8),
        '9' => (false, HostKey::Digit9),
        '!' => (true, HostKey::Digit1),
        '"' => (true, HostKey::Digit2),
        '#' => (true, HostKey::Digit3),
        '$' => (true, HostKey::Digit4),
        '%' => (true, HostKey::Digit5),
        '&' => (true, HostKey::Digit6),
        '\'' => (true, HostKey::Digit7),
        '(' => (true, HostKey::Digit8),
        ')' => (true, HostKey::Digit9),
        ',' => (false, HostKey::Comma),
        '.' => (false, HostKey::Period),
        '/' => (false, HostKey::Slash),
        '?' => (true, HostKey::Slash),
        ' ' => (false, HostKey::Space),
        '\r' | '\n' => (false, HostKey::Return),
        '\t' => (false, HostKey::Tab),
        '\u{8}' => (false, HostKey::Backspace),
        _ => {
            return layout
                .symbols()
                .iter()
                .find(|(symbol, _, _)| *symbol == character)
                .map(|&(_, shift, key)| CharStroke { shift, key });
        }
    };
    Some(CharStroke { shift, key })
}

/// Applies a named joystick control to a state, returning whether the control
/// name was recognized.
pub fn apply_joystick_control(state: &mut JoystickState, control: &str, pressed: bool) -> bool {
    match control {
        "up" => state.up = pressed,
        "down" => state.down = pressed,
        "left" => state.left = pressed,
        "right" => state.right = pressed,
        "trigger1" | "button-a" => state.trigger1 = pressed,
        "trigger2" | "button-b" => state.trigger2 = pressed,
        "button-c" => state.button_c = pressed,
        "button-x" => state.button_x = pressed,
        "button-y" => state.button_y = pressed,
        "button-z" => state.button_z = pressed,
        "run" | "start" => state.run = pressed,
        "select" => state.select = pressed,
        _ => return false,
    }
    true
}

/// A logical mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    /// Left mouse button.
    Left,
    /// Right mouse button.
    Right,
    /// Middle mouse button.
    Middle,
}

/// Resolves a Scheme mouse-button name to a logical button.
pub fn mouse_button_from_name(name: &str) -> Option<MouseButton> {
    match name {
        "left" => Some(MouseButton::Left),
        "right" => Some(MouseButton::Right),
        "middle" => Some(MouseButton::Middle),
        _ => None,
    }
}

/// Maps a lowercase ASCII letter to its host key, or `None` for any other
/// character.
fn letter_key(character: char) -> Option<HostKey> {
    let key = match character {
        'a' => HostKey::A,
        'b' => HostKey::B,
        'c' => HostKey::C,
        'd' => HostKey::D,
        'e' => HostKey::E,
        'f' => HostKey::F,
        'g' => HostKey::G,
        'h' => HostKey::H,
        'i' => HostKey::I,
        'j' => HostKey::J,
        'k' => HostKey::K,
        'l' => HostKey::L,
        'm' => HostKey::M,
        'n' => HostKey::N,
        'o' => HostKey::O,
        'p' => HostKey::P,
        'q' => HostKey::Q,
        'r' => HostKey::R,
        's' => HostKey::S,
        't' => HostKey::T,
        'u' => HostKey::U,
        'v' => HostKey::V,
        'w' => HostKey::W,
        'x' => HostKey::X,
        'y' => HostKey::Y,
        'z' => HostKey::Z,
        _ => return None,
    };
    Some(key)
}
