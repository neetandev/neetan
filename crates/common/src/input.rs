//! Host-side physical key and modifier input types shared by the frontends.

/// Host-side physical key identity.
///
/// This is the neutral, machine-agnostic identity of a physical key, shared by
/// the host frontends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(missing_docs)]
pub enum HostKey {
    // Letter keys `A` through `Z`.
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    // Digit keys `0` through `9` on the main row.
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    /// Space bar.
    Space,
    /// Minus / hyphen key.
    Minus,
    /// Equals key.
    Equals,
    /// Left bracket key.
    LeftBracket,
    /// Right bracket key.
    RightBracket,
    /// Semicolon key.
    Semicolon,
    /// Apostrophe key.
    Apostrophe,
    /// Grave / backtick key.
    Grave,
    /// Comma key.
    Comma,
    /// Period key.
    Period,
    /// Slash key.
    Slash,
    /// US backslash key.
    Backslash,
    /// Non-US backslash key (ISO layout key beside the left shift).
    NonUsBackslash,
    /// Escape key.
    Escape,
    /// Backspace key.
    Backspace,
    /// Tab key.
    Tab,
    /// Return / Enter key on the main block.
    Return,
    /// Insert key.
    Insert,
    /// Delete key.
    Delete,
    /// Home key.
    Home,
    /// End key.
    End,
    /// Page-up key.
    PageUp,
    /// Page-down key.
    PageDown,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Pause key.
    Pause,
    /// Print-screen key.
    PrintScreen,
    /// Application / menu key.
    Application,
    // Numeric keypad digits `0` through `9`.
    Kp0,
    Kp1,
    Kp2,
    Kp3,
    Kp4,
    Kp5,
    Kp6,
    Kp7,
    Kp8,
    Kp9,
    /// Numeric keypad minus.
    KpMinus,
    /// Numeric keypad divide.
    KpDivide,
    /// Numeric keypad multiply.
    KpMultiply,
    /// Numeric keypad plus.
    KpPlus,
    /// Numeric keypad comma.
    KpComma,
    /// Numeric keypad period.
    KpPeriod,
    /// Numeric keypad enter.
    KpEnter,
    // Function keys `F1` through `F15`.
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    /// Left shift modifier.
    LeftShift,
    /// Right shift modifier.
    RightShift,
    /// Left control modifier.
    LeftControl,
    /// Right control modifier.
    RightControl,
    /// Left alt modifier.
    LeftAlt,
    /// Right alt modifier.
    RightAlt,
    /// Caps-lock key.
    CapsLock,
    /// Num-lock key.
    NumLock,
    /// International key 1 (JIS backslash / underscore).
    International1,
    /// International key 2 (JIS katakana / hiragana).
    International2,
    /// International key 3 (JIS yen).
    International3,
    /// International key 4 (JIS henkan).
    International4,
    /// International key 5 (JIS muhenkan).
    International5,
}

/// Host modifier state accompanying a [`HostKey`] translation.
///
/// Only the modifiers a machine needs to fold a pre-composed key code are
/// carried. The PC-6000 family resolves Shift and Control itself, so those two
/// bits suffice; matrix machines forward Shift and Control as their own key
/// cells and ignore this entirely.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct KeyModifiers {
    /// Whether a Shift key is held.
    pub shift: bool,
    /// Whether a Control key is held.
    pub ctrl: bool,
}
