//! Logical key and text mapping for the automation frontend.
//!
//! Maps the Scheme-facing key names and ASCII characters onto the neutral
//! [`HostKey`] identity. This layer is SDL-free: it never references an
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

/// Converts a Version-1 text character into a host-key stroke.
///
/// The supported set is printable ASCII plus carriage return, tab, and
/// backspace. The punctuation mapping follows a US layout. Returns `None` for a
/// character outside the supported set.
pub fn char_to_stroke(character: char) -> Option<CharStroke> {
    let (shift, key) = match character {
        'a'..='z' => (false, letter_key(character)),
        'A'..='Z' => (true, letter_key(character.to_ascii_lowercase())),
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
        ' ' => (false, HostKey::Space),
        '\r' | '\n' => (false, HostKey::Return),
        '\t' => (false, HostKey::Tab),
        '\u{8}' => (false, HostKey::Backspace),
        '-' => (false, HostKey::Minus),
        '=' => (false, HostKey::Equals),
        '[' => (false, HostKey::LeftBracket),
        ']' => (false, HostKey::RightBracket),
        ';' => (false, HostKey::Semicolon),
        '\'' => (false, HostKey::Apostrophe),
        ',' => (false, HostKey::Comma),
        '.' => (false, HostKey::Period),
        '/' => (false, HostKey::Slash),
        '@' => (false, HostKey::Grave),
        '^' => (false, HostKey::Equals),
        '!' => (true, HostKey::Digit1),
        '"' => (true, HostKey::Digit2),
        '#' => (true, HostKey::Digit3),
        '$' => (true, HostKey::Digit4),
        '%' => (true, HostKey::Digit5),
        '&' => (true, HostKey::Digit6),
        '(' => (true, HostKey::Digit8),
        ')' => (true, HostKey::Digit9),
        '+' => (true, HostKey::Semicolon),
        '*' => (true, HostKey::Apostrophe),
        ':' => (false, HostKey::Apostrophe),
        '_' => (true, HostKey::NonUsBackslash),
        _ => return None,
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

/// Maps a lowercase ASCII letter to its host key.
fn letter_key(character: char) -> HostKey {
    match character {
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
        _ => HostKey::Z,
    }
}
