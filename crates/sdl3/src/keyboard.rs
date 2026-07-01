use sdl3_sys::scancode::SDL_Scancode;

/// A physical keyboard key.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Scancode {
    /// Escape key.
    Escape,
    /// Number row 1.
    _1,
    /// Number row 2.
    _2,
    /// Number row 3.
    _3,
    /// Number row 4.
    _4,
    /// Number row 5.
    _5,
    /// Number row 6.
    _6,
    /// Number row 7.
    _7,
    /// Number row 8.
    _8,
    /// Number row 9.
    _9,
    /// Number row 0.
    _0,
    /// Minus / hyphen key.
    Minus,
    /// Equals key.
    Equals,
    /// Backslash key.
    Backslash,
    /// Backspace key.
    Backspace,
    /// Tab key.
    Tab,
    /// Q key.
    Q,
    /// W key.
    W,
    /// E key.
    E,
    /// R key.
    R,
    /// T key.
    T,
    /// Y key.
    Y,
    /// U key.
    U,
    /// I key.
    I,
    /// O key.
    O,
    /// P key.
    P,
    /// Return / Enter key.
    Return,
    /// Left bracket key.
    LeftBracket,
    /// Right bracket key.
    RightBracket,
    /// Grave accent / tilde key.
    Grave,
    /// Semicolon key.
    Semicolon,
    /// Apostrophe key.
    Apostrophe,
    /// Non-US backslash key.
    NonUsBackslash,
    /// A key.
    A,
    /// S key.
    S,
    /// D key.
    D,
    /// F key.
    F,
    /// G key.
    G,
    /// H key.
    H,
    /// J key.
    J,
    /// K key.
    K,
    /// L key.
    L,
    /// Z key.
    Z,
    /// X key.
    X,
    /// C key.
    C,
    /// V key.
    V,
    /// B key.
    B,
    /// N key.
    N,
    /// M key.
    M,
    /// Comma key.
    Comma,
    /// Period key.
    Period,
    /// Slash key.
    Slash,
    /// Space bar.
    Space,
    /// Page Down key.
    PageDown,
    /// Page Up key.
    PageUp,
    /// Insert key.
    Insert,
    /// Delete key.
    Delete,
    /// Up arrow key.
    Up,
    /// Left arrow key.
    Left,
    /// Right arrow key.
    Right,
    /// Down arrow key.
    Down,
    /// Home key.
    Home,
    /// End key.
    End,
    /// Keypad minus.
    KpMinus,
    /// Keypad divide.
    KpDivide,
    /// Keypad 0.
    Kp0,
    /// Keypad 1.
    Kp1,
    /// Keypad 2.
    Kp2,
    /// Keypad 3.
    Kp3,
    /// Keypad 4.
    Kp4,
    /// Keypad 5.
    Kp5,
    /// Keypad 6.
    Kp6,
    /// Keypad 7.
    Kp7,
    /// Keypad 8.
    Kp8,
    /// Keypad 9.
    Kp9,
    /// Keypad multiply.
    KpMultiply,
    /// Keypad plus.
    KpPlus,
    /// Keypad enter.
    KpEnter,
    /// Keypad comma.
    KpComma,
    /// Keypad period.
    KpPeriod,
    /// F1 key.
    F1,
    /// F2 key.
    F2,
    /// F3 key.
    F3,
    /// F4 key.
    F4,
    /// F5 key.
    F5,
    /// F6 key.
    F6,
    /// F7 key.
    F7,
    /// F8 key.
    F8,
    /// F9 key.
    F9,
    /// F10 key.
    F10,
    /// F11 key.
    F11,
    /// F12 key.
    F12,
    /// F13 key.
    F13,
    /// F14 key.
    F14,
    /// F15 key.
    F15,
    /// Print Screen key.
    PrintScreen,
    /// Pause key.
    Pause,
    /// Num Lock key.
    NumLock,
    /// Application / Menu key.
    Application,
    /// Left Shift key.
    LShift,
    /// Right Shift key.
    RShift,
    /// Caps Lock key.
    CapsLock,
    /// Left Alt key.
    LAlt,
    /// Right Alt key.
    RAlt,
    /// Left Ctrl key.
    LCtrl,
    /// Right Ctrl key.
    RCtrl,
}

impl Scancode {
    /// Total number of `Scancode` variants.
    pub const COUNT: usize = 106;

    /// Converts a raw SDL3 scancode to a `Scancode`, returning `None` for unmapped keys.
    pub fn from_raw(raw: SDL_Scancode) -> Option<Self> {
        Some(match raw {
            SDL_Scancode::ESCAPE => Self::Escape,
            SDL_Scancode::_1 => Self::_1,
            SDL_Scancode::_2 => Self::_2,
            SDL_Scancode::_3 => Self::_3,
            SDL_Scancode::_4 => Self::_4,
            SDL_Scancode::_5 => Self::_5,
            SDL_Scancode::_6 => Self::_6,
            SDL_Scancode::_7 => Self::_7,
            SDL_Scancode::_8 => Self::_8,
            SDL_Scancode::_9 => Self::_9,
            SDL_Scancode::_0 => Self::_0,
            SDL_Scancode::MINUS => Self::Minus,
            SDL_Scancode::EQUALS => Self::Equals,
            SDL_Scancode::BACKSLASH => Self::Backslash,
            SDL_Scancode::BACKSPACE => Self::Backspace,
            SDL_Scancode::TAB => Self::Tab,
            SDL_Scancode::Q => Self::Q,
            SDL_Scancode::W => Self::W,
            SDL_Scancode::E => Self::E,
            SDL_Scancode::R => Self::R,
            SDL_Scancode::T => Self::T,
            SDL_Scancode::Y => Self::Y,
            SDL_Scancode::U => Self::U,
            SDL_Scancode::I => Self::I,
            SDL_Scancode::O => Self::O,
            SDL_Scancode::P => Self::P,
            SDL_Scancode::RETURN => Self::Return,
            SDL_Scancode::LEFTBRACKET => Self::LeftBracket,
            SDL_Scancode::RIGHTBRACKET => Self::RightBracket,
            SDL_Scancode::GRAVE => Self::Grave,
            SDL_Scancode::SEMICOLON => Self::Semicolon,
            SDL_Scancode::APOSTROPHE => Self::Apostrophe,
            SDL_Scancode::NONUSBACKSLASH => Self::NonUsBackslash,
            SDL_Scancode::A => Self::A,
            SDL_Scancode::S => Self::S,
            SDL_Scancode::D => Self::D,
            SDL_Scancode::F => Self::F,
            SDL_Scancode::G => Self::G,
            SDL_Scancode::H => Self::H,
            SDL_Scancode::J => Self::J,
            SDL_Scancode::K => Self::K,
            SDL_Scancode::L => Self::L,
            SDL_Scancode::Z => Self::Z,
            SDL_Scancode::X => Self::X,
            SDL_Scancode::C => Self::C,
            SDL_Scancode::V => Self::V,
            SDL_Scancode::B => Self::B,
            SDL_Scancode::N => Self::N,
            SDL_Scancode::M => Self::M,
            SDL_Scancode::COMMA => Self::Comma,
            SDL_Scancode::PERIOD => Self::Period,
            SDL_Scancode::SLASH => Self::Slash,
            SDL_Scancode::SPACE => Self::Space,
            SDL_Scancode::PAGEDOWN => Self::PageDown,
            SDL_Scancode::PAGEUP => Self::PageUp,
            SDL_Scancode::INSERT => Self::Insert,
            SDL_Scancode::DELETE => Self::Delete,
            SDL_Scancode::UP => Self::Up,
            SDL_Scancode::LEFT => Self::Left,
            SDL_Scancode::RIGHT => Self::Right,
            SDL_Scancode::DOWN => Self::Down,
            SDL_Scancode::HOME => Self::Home,
            SDL_Scancode::END => Self::End,
            SDL_Scancode::KP_MINUS => Self::KpMinus,
            SDL_Scancode::KP_DIVIDE => Self::KpDivide,
            SDL_Scancode::KP_0 => Self::Kp0,
            SDL_Scancode::KP_1 => Self::Kp1,
            SDL_Scancode::KP_2 => Self::Kp2,
            SDL_Scancode::KP_3 => Self::Kp3,
            SDL_Scancode::KP_4 => Self::Kp4,
            SDL_Scancode::KP_5 => Self::Kp5,
            SDL_Scancode::KP_6 => Self::Kp6,
            SDL_Scancode::KP_7 => Self::Kp7,
            SDL_Scancode::KP_8 => Self::Kp8,
            SDL_Scancode::KP_9 => Self::Kp9,
            SDL_Scancode::KP_MULTIPLY => Self::KpMultiply,
            SDL_Scancode::KP_PLUS => Self::KpPlus,
            SDL_Scancode::KP_ENTER => Self::KpEnter,
            SDL_Scancode::KP_COMMA => Self::KpComma,
            SDL_Scancode::KP_PERIOD => Self::KpPeriod,
            SDL_Scancode::F1 => Self::F1,
            SDL_Scancode::F2 => Self::F2,
            SDL_Scancode::F3 => Self::F3,
            SDL_Scancode::F4 => Self::F4,
            SDL_Scancode::F5 => Self::F5,
            SDL_Scancode::F6 => Self::F6,
            SDL_Scancode::F7 => Self::F7,
            SDL_Scancode::F8 => Self::F8,
            SDL_Scancode::F9 => Self::F9,
            SDL_Scancode::F10 => Self::F10,
            SDL_Scancode::F11 => Self::F11,
            SDL_Scancode::F12 => Self::F12,
            SDL_Scancode::F13 => Self::F13,
            SDL_Scancode::F14 => Self::F14,
            SDL_Scancode::F15 => Self::F15,
            SDL_Scancode::PRINTSCREEN => Self::PrintScreen,
            SDL_Scancode::PAUSE => Self::Pause,
            SDL_Scancode::NUMLOCKCLEAR => Self::NumLock,
            SDL_Scancode::APPLICATION => Self::Application,
            SDL_Scancode::LSHIFT => Self::LShift,
            SDL_Scancode::RSHIFT => Self::RShift,
            SDL_Scancode::CAPSLOCK => Self::CapsLock,
            SDL_Scancode::LALT => Self::LAlt,
            SDL_Scancode::RALT => Self::RAlt,
            SDL_Scancode::LCTRL => Self::LCtrl,
            SDL_Scancode::RCTRL => Self::RCtrl,
            _ => return None,
        })
    }

    /// Returns a unique index for this scancode variant, suitable for array indexing.
    pub const fn index(self) -> usize {
        match self {
            Self::Escape => 0,
            Self::_1 => 1,
            Self::_2 => 2,
            Self::_3 => 3,
            Self::_4 => 4,
            Self::_5 => 5,
            Self::_6 => 6,
            Self::_7 => 7,
            Self::_8 => 8,
            Self::_9 => 9,
            Self::_0 => 10,
            Self::Minus => 11,
            Self::Equals => 12,
            Self::Backslash => 13,
            Self::Backspace => 14,
            Self::Tab => 15,
            Self::Q => 16,
            Self::W => 17,
            Self::E => 18,
            Self::R => 19,
            Self::T => 20,
            Self::Y => 21,
            Self::U => 22,
            Self::I => 23,
            Self::O => 24,
            Self::P => 25,
            Self::Return => 26,
            Self::LeftBracket => 27,
            Self::RightBracket => 28,
            Self::Grave => 29,
            Self::Semicolon => 30,
            Self::Apostrophe => 31,
            Self::NonUsBackslash => 32,
            Self::A => 33,
            Self::S => 34,
            Self::D => 35,
            Self::F => 36,
            Self::G => 37,
            Self::H => 38,
            Self::J => 39,
            Self::K => 40,
            Self::L => 41,
            Self::Z => 42,
            Self::X => 43,
            Self::C => 44,
            Self::V => 45,
            Self::B => 46,
            Self::N => 47,
            Self::M => 48,
            Self::Comma => 49,
            Self::Period => 50,
            Self::Slash => 51,
            Self::Space => 52,
            Self::PageDown => 53,
            Self::PageUp => 54,
            Self::Insert => 55,
            Self::Delete => 56,
            Self::Up => 57,
            Self::Left => 58,
            Self::Right => 59,
            Self::Down => 60,
            Self::Home => 61,
            Self::End => 62,
            Self::KpMinus => 63,
            Self::KpDivide => 64,
            Self::Kp0 => 65,
            Self::Kp1 => 66,
            Self::Kp2 => 67,
            Self::Kp3 => 68,
            Self::Kp4 => 69,
            Self::Kp5 => 70,
            Self::Kp6 => 71,
            Self::Kp7 => 72,
            Self::Kp8 => 73,
            Self::Kp9 => 74,
            Self::KpMultiply => 75,
            Self::KpPlus => 76,
            Self::KpEnter => 77,
            Self::KpComma => 78,
            Self::KpPeriod => 79,
            Self::F1 => 80,
            Self::F2 => 81,
            Self::F3 => 82,
            Self::F4 => 83,
            Self::F5 => 84,
            Self::F6 => 85,
            Self::F7 => 86,
            Self::F8 => 87,
            Self::F9 => 88,
            Self::F10 => 89,
            Self::F11 => 90,
            Self::F12 => 91,
            Self::F13 => 92,
            Self::F14 => 93,
            Self::F15 => 94,
            Self::PrintScreen => 95,
            Self::Pause => 96,
            Self::NumLock => 97,
            Self::Application => 98,
            Self::LShift => 99,
            Self::RShift => 100,
            Self::CapsLock => 101,
            Self::LAlt => 102,
            Self::RAlt => 103,
            Self::LCtrl => 104,
            Self::RCtrl => 105,
        }
    }

    /// Parses a scancode from its variant name (case-insensitive).
    pub fn from_name(name: &str) -> Option<Self> {
        let name_lower = name.to_ascii_lowercase();
        Some(match name_lower.as_str() {
            "escape" => Self::Escape,
            "1" => Self::_1,
            "2" => Self::_2,
            "3" => Self::_3,
            "4" => Self::_4,
            "5" => Self::_5,
            "6" => Self::_6,
            "7" => Self::_7,
            "8" => Self::_8,
            "9" => Self::_9,
            "0" => Self::_0,
            "minus" => Self::Minus,
            "equals" => Self::Equals,
            "backslash" => Self::Backslash,
            "backspace" => Self::Backspace,
            "tab" => Self::Tab,
            "q" => Self::Q,
            "w" => Self::W,
            "e" => Self::E,
            "r" => Self::R,
            "t" => Self::T,
            "y" => Self::Y,
            "u" => Self::U,
            "i" => Self::I,
            "o" => Self::O,
            "p" => Self::P,
            "return" => Self::Return,
            "leftbracket" => Self::LeftBracket,
            "rightbracket" => Self::RightBracket,
            "grave" => Self::Grave,
            "semicolon" => Self::Semicolon,
            "apostrophe" => Self::Apostrophe,
            "nonusbackslash" => Self::NonUsBackslash,
            "a" => Self::A,
            "s" => Self::S,
            "d" => Self::D,
            "f" => Self::F,
            "g" => Self::G,
            "h" => Self::H,
            "j" => Self::J,
            "k" => Self::K,
            "l" => Self::L,
            "z" => Self::Z,
            "x" => Self::X,
            "c" => Self::C,
            "v" => Self::V,
            "b" => Self::B,
            "n" => Self::N,
            "m" => Self::M,
            "comma" => Self::Comma,
            "period" => Self::Period,
            "slash" => Self::Slash,
            "space" => Self::Space,
            "pagedown" => Self::PageDown,
            "pageup" => Self::PageUp,
            "insert" => Self::Insert,
            "delete" => Self::Delete,
            "up" => Self::Up,
            "left" => Self::Left,
            "right" => Self::Right,
            "down" => Self::Down,
            "home" => Self::Home,
            "end" => Self::End,
            "kpminus" => Self::KpMinus,
            "kpdivide" => Self::KpDivide,
            "kp0" => Self::Kp0,
            "kp1" => Self::Kp1,
            "kp2" => Self::Kp2,
            "kp3" => Self::Kp3,
            "kp4" => Self::Kp4,
            "kp5" => Self::Kp5,
            "kp6" => Self::Kp6,
            "kp7" => Self::Kp7,
            "kp8" => Self::Kp8,
            "kp9" => Self::Kp9,
            "kpmultiply" => Self::KpMultiply,
            "kpplus" => Self::KpPlus,
            "kpenter" => Self::KpEnter,
            "kpcomma" => Self::KpComma,
            "kpperiod" => Self::KpPeriod,
            "f1" => Self::F1,
            "f2" => Self::F2,
            "f3" => Self::F3,
            "f4" => Self::F4,
            "f5" => Self::F5,
            "f6" => Self::F6,
            "f7" => Self::F7,
            "f8" => Self::F8,
            "f9" => Self::F9,
            "f10" => Self::F10,
            "f11" => Self::F11,
            "f12" => Self::F12,
            "f13" => Self::F13,
            "f14" => Self::F14,
            "f15" => Self::F15,
            "printscreen" => Self::PrintScreen,
            "pause" => Self::Pause,
            "numlock" => Self::NumLock,
            "application" => Self::Application,
            "lshift" => Self::LShift,
            "rshift" => Self::RShift,
            "capslock" => Self::CapsLock,
            "lalt" => Self::LAlt,
            "ralt" => Self::RAlt,
            "lctrl" => Self::LCtrl,
            "rctrl" => Self::RCtrl,
            _ => return None,
        })
    }

    /// Returns the display name of this scancode variant.
    pub fn name(self) -> &'static str {
        match self {
            Self::Escape => "Escape",
            Self::_1 => "1",
            Self::_2 => "2",
            Self::_3 => "3",
            Self::_4 => "4",
            Self::_5 => "5",
            Self::_6 => "6",
            Self::_7 => "7",
            Self::_8 => "8",
            Self::_9 => "9",
            Self::_0 => "0",
            Self::Minus => "Minus",
            Self::Equals => "Equals",
            Self::Backslash => "Backslash",
            Self::Backspace => "Backspace",
            Self::Tab => "Tab",
            Self::Q => "Q",
            Self::W => "W",
            Self::E => "E",
            Self::R => "R",
            Self::T => "T",
            Self::Y => "Y",
            Self::U => "U",
            Self::I => "I",
            Self::O => "O",
            Self::P => "P",
            Self::Return => "Return",
            Self::LeftBracket => "LeftBracket",
            Self::RightBracket => "RightBracket",
            Self::Grave => "Grave",
            Self::Semicolon => "Semicolon",
            Self::Apostrophe => "Apostrophe",
            Self::NonUsBackslash => "NonUsBackslash",
            Self::A => "A",
            Self::S => "S",
            Self::D => "D",
            Self::F => "F",
            Self::G => "G",
            Self::H => "H",
            Self::J => "J",
            Self::K => "K",
            Self::L => "L",
            Self::Z => "Z",
            Self::X => "X",
            Self::C => "C",
            Self::V => "V",
            Self::B => "B",
            Self::N => "N",
            Self::M => "M",
            Self::Comma => "Comma",
            Self::Period => "Period",
            Self::Slash => "Slash",
            Self::Space => "Space",
            Self::PageDown => "PageDown",
            Self::PageUp => "PageUp",
            Self::Insert => "Insert",
            Self::Delete => "Delete",
            Self::Up => "Up",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Down => "Down",
            Self::Home => "Home",
            Self::End => "End",
            Self::KpMinus => "KpMinus",
            Self::KpDivide => "KpDivide",
            Self::Kp0 => "Kp0",
            Self::Kp1 => "Kp1",
            Self::Kp2 => "Kp2",
            Self::Kp3 => "Kp3",
            Self::Kp4 => "Kp4",
            Self::Kp5 => "Kp5",
            Self::Kp6 => "Kp6",
            Self::Kp7 => "Kp7",
            Self::Kp8 => "Kp8",
            Self::Kp9 => "Kp9",
            Self::KpMultiply => "KpMultiply",
            Self::KpPlus => "KpPlus",
            Self::KpEnter => "KpEnter",
            Self::KpComma => "KpComma",
            Self::KpPeriod => "KpPeriod",
            Self::F1 => "F1",
            Self::F2 => "F2",
            Self::F3 => "F3",
            Self::F4 => "F4",
            Self::F5 => "F5",
            Self::F6 => "F6",
            Self::F7 => "F7",
            Self::F8 => "F8",
            Self::F9 => "F9",
            Self::F10 => "F10",
            Self::F11 => "F11",
            Self::F12 => "F12",
            Self::F13 => "F13",
            Self::F14 => "F14",
            Self::F15 => "F15",
            Self::PrintScreen => "PrintScreen",
            Self::Pause => "Pause",
            Self::NumLock => "NumLock",
            Self::Application => "Application",
            Self::LShift => "LShift",
            Self::RShift => "RShift",
            Self::CapsLock => "CapsLock",
            Self::LAlt => "LAlt",
            Self::RAlt => "RAlt",
            Self::LCtrl => "LCtrl",
            Self::RCtrl => "RCtrl",
        }
    }
}

/// Keyboard modifier flags (Shift, Ctrl, Alt, etc.).
pub struct Mod(pub u16);

impl Mod {
    /// Returns `true` if either Shift key is held.
    #[inline]
    pub const fn shift(&self) -> bool {
        self.0 & 0x0003 != 0
    }

    /// Returns `true` if either Ctrl key is held.
    #[inline]
    pub const fn ctrl(&self) -> bool {
        self.0 & 0x00C0 != 0
    }

    /// Returns `true` if either Alt key is held.
    #[inline]
    pub const fn alt(&self) -> bool {
        self.0 & 0x0300 != 0
    }

    /// Returns `true` if either GUI (Super/Command) key is held.
    #[inline]
    pub const fn gui(&self) -> bool {
        self.0 & 0x0C00 != 0
    }

    /// Returns `true` if both Alt and GUI (Super/Command) are held.
    #[inline]
    pub const fn alt_gui(&self) -> bool {
        self.alt() && self.gui()
    }
}
