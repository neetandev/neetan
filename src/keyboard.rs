use common::{JoystickState, Machine};
use sdl3::keyboard::Scancode;

/// A joystick input the keyboard fallback can drive.
///
/// The fallback is only used when no gamepad is connected. The arrow keys drive
/// the four directions and Z/X drive the two triggers; these keys also reach the
/// emulated keyboard, which is acceptable since PC-88 software reads either the
/// keyboard or the joystick port for a given control, not both at once.
pub(crate) enum JoystickKey {
    Up,
    Down,
    Left,
    Right,
    Trigger1,
    Trigger2,
}

impl JoystickKey {
    /// Maps a host scancode to a joystick input, if it is one of the fallback keys.
    pub(crate) fn from_scancode(scancode: Option<Scancode>) -> Option<Self> {
        match scancode {
            Some(Scancode::Up) => Some(Self::Up),
            Some(Scancode::Down) => Some(Self::Down),
            Some(Scancode::Left) => Some(Self::Left),
            Some(Scancode::Right) => Some(Self::Right),
            Some(Scancode::Z) => Some(Self::Trigger1),
            Some(Scancode::X) => Some(Self::Trigger2),
            _ => None,
        }
    }

    /// Sets or clears this input in `state`.
    pub(crate) fn apply(self, state: &mut JoystickState, pressed: bool) {
        match self {
            Self::Up => state.up = pressed,
            Self::Down => state.down = pressed,
            Self::Left => state.left = pressed,
            Self::Right => state.right = pressed,
            Self::Trigger1 => state.trigger1 = pressed,
            Self::Trigger2 => state.trigger2 = pressed,
        }
    }
}

pub fn pc98_scancode_from_name(name: &str) -> Option<u8> {
    let name_lower = name.to_ascii_lowercase();
    Some(match name_lower.as_str() {
        "esc" => 0x00,
        "1" => 0x01,
        "2" => 0x02,
        "3" => 0x03,
        "4" => 0x04,
        "5" => 0x05,
        "6" => 0x06,
        "7" => 0x07,
        "8" => 0x08,
        "9" => 0x09,
        "0" => 0x0A,
        "minus" => 0x0B,
        "caret" => 0x0C,
        "yen" => 0x0D,
        "bs" => 0x0E,
        "tab" => 0x0F,
        "q" => 0x10,
        "w" => 0x11,
        "e" => 0x12,
        "r" => 0x13,
        "t" => 0x14,
        "y" => 0x15,
        "u" => 0x16,
        "i" => 0x17,
        "o" => 0x18,
        "p" => 0x19,
        "at" => 0x1A,
        "leftbracket" => 0x1B,
        "return" => 0x1C,
        "a" => 0x1D,
        "s" => 0x1E,
        "d" => 0x1F,
        "f" => 0x20,
        "g" => 0x21,
        "h" => 0x22,
        "j" => 0x23,
        "k" => 0x24,
        "l" => 0x25,
        "semicolon" => 0x26,
        "colon" => 0x27,
        "rightbracket" => 0x28,
        "z" => 0x29,
        "x" => 0x2A,
        "c" => 0x2B,
        "v" => 0x2C,
        "b" => 0x2D,
        "n" => 0x2E,
        "m" => 0x2F,
        "comma" => 0x30,
        "period" => 0x31,
        "slash" => 0x32,
        "underscore" => 0x33,
        "space" => 0x34,
        "xfer" => 0x35,
        "rollup" => 0x36,
        "rolldown" => 0x37,
        "ins" => 0x38,
        "del" => 0x39,
        "up" => 0x3A,
        "left" => 0x3B,
        "right" => 0x3C,
        "down" => 0x3D,
        "home" => 0x3E,
        "help" => 0x3F,
        "kpminus" => 0x40,
        "kpdivide" => 0x41,
        "kp7" => 0x42,
        "kp8" => 0x43,
        "kp9" => 0x44,
        "kpmultiply" => 0x45,
        "kp4" => 0x46,
        "kp5" => 0x47,
        "kp6" => 0x48,
        "kpplus" => 0x49,
        "kp1" => 0x4A,
        "kp2" => 0x4B,
        "kp3" => 0x4C,
        "kpequals" => 0x4D,
        "kp0" => 0x4E,
        "kpcomma" => 0x4F,
        "kpperiod" => 0x50,
        "nfer" => 0x51,
        "vf1" => 0x52,
        "vf2" => 0x53,
        "vf3" => 0x54,
        "vf4" => 0x55,
        "vf5" => 0x56,
        "stop" => 0x60,
        "copy" => 0x61,
        "f1" => 0x62,
        "f2" => 0x63,
        "f3" => 0x64,
        "f4" => 0x65,
        "f5" => 0x66,
        "f6" => 0x67,
        "f7" => 0x68,
        "f8" => 0x69,
        "f9" => 0x6A,
        "f10" => 0x6B,
        "shift" => 0x70,
        "caps" => 0x71,
        "kana" => 0x72,
        "grph" => 0x73,
        "ctrl" => 0x74,
        _ => return None,
    })
}

pub struct KeyMap {
    mappings: [u8; Scancode::COUNT],
}

impl KeyMap {
    pub const fn new() -> Self {
        Self {
            mappings: build_default_map(),
        }
    }

    pub const fn new_pc88() -> Self {
        Self {
            mappings: build_pc88_default_map(),
        }
    }

    pub fn set(&mut self, host: Scancode, pc98_code: u8) {
        self.mappings[host.index()] = pc98_code;
    }

    pub fn lookup(&self, host: Scancode) -> u8 {
        self.mappings[host.index()]
    }
}

pub(crate) struct KeyboardForwardingState {
    gui_modifier_active: bool,
    guest_pressed_pc98_scancodes: [Option<u8>; Scancode::COUNT],
    pending_pressed_pc98_scancode: Option<u8>,
    pending_released_pc98_scancodes: Vec<u8>,
}

impl KeyboardForwardingState {
    pub(crate) fn new() -> Self {
        Self {
            gui_modifier_active: false,
            guest_pressed_pc98_scancodes: [None; Scancode::COUNT],
            pending_pressed_pc98_scancode: None,
            pending_released_pc98_scancodes: Vec::with_capacity(Scancode::COUNT),
        }
    }

    pub(crate) fn handle_key_down(
        &mut self,
        scancode: Option<Scancode>,
        gui_modifier_active: bool,
        repeat: bool,
        key_map: &KeyMap,
    ) {
        self.clear_pending_actions();

        if repeat {
            return;
        }

        if gui_modifier_active && !self.gui_modifier_active {
            self.release_all_guest_keys();
        }
        self.gui_modifier_active = gui_modifier_active;

        if self.gui_modifier_active {
            return;
        }

        let Some(scancode) = scancode else {
            return;
        };

        let scancode_index = scancode.index();
        if self.guest_pressed_pc98_scancodes[scancode_index].is_some() {
            return;
        }

        let pc98_scancode = key_map.lookup(scancode);
        self.guest_pressed_pc98_scancodes[scancode_index] = Some(pc98_scancode);
        self.pending_pressed_pc98_scancode = Some(pc98_scancode);
    }

    pub(crate) fn handle_key_up(
        &mut self,
        scancode: Option<Scancode>,
        repeat: bool,
        key_map: &KeyMap,
    ) -> Option<u8> {
        if repeat {
            return None;
        }

        let scancode = scancode?;
        let scancode_index = scancode.index();
        let pc98_scancode = self.guest_pressed_pc98_scancodes[scancode_index]?;
        self.guest_pressed_pc98_scancodes[scancode_index] = None;

        let expected_pc98_scancode = key_map.lookup(scancode);
        debug_assert_eq!(pc98_scancode, expected_pc98_scancode);

        Some(pc98_scancode | 0x80)
    }

    fn release_all_guest_keys(&mut self) {
        for guest_pressed_pc98_scancode in &mut self.guest_pressed_pc98_scancodes {
            if let Some(pc98_scancode) = guest_pressed_pc98_scancode.take() {
                self.pending_released_pc98_scancodes
                    .push(pc98_scancode | 0x80);
            }
        }
    }

    pub(crate) fn apply_pending_actions(&mut self, machine: &mut dyn Machine) {
        for &released_pc98_scancode in &self.pending_released_pc98_scancodes {
            machine.push_keyboard_scancode(released_pc98_scancode);
        }

        if let Some(pressed_pc98_scancode) = self.pending_pressed_pc98_scancode {
            machine.push_keyboard_scancode(pressed_pc98_scancode);
        }

        self.clear_pending_actions();
    }

    #[cfg(test)]
    fn pending_pressed_pc98_scancode(&self) -> Option<u8> {
        self.pending_pressed_pc98_scancode
    }

    #[cfg(test)]
    fn pending_released_pc98_scancodes(&self) -> &[u8] {
        &self.pending_released_pc98_scancodes
    }

    fn clear_pending_actions(&mut self) {
        self.pending_pressed_pc98_scancode = None;
        self.pending_released_pc98_scancodes.clear();
    }
}

#[allow(clippy::just_underscores_and_digits)]
const fn build_default_map() -> [u8; Scancode::COUNT] {
    use Scancode::*;

    const ALL_SCANCODES: &[(Scancode, u8)] = &[
        (Escape, 0x00),
        (_1, 0x01),
        (_2, 0x02),
        (_3, 0x03),
        (_4, 0x04),
        (_5, 0x05),
        (_6, 0x06),
        (_7, 0x07),
        (_8, 0x08),
        (_9, 0x09),
        (_0, 0x0A),
        (Minus, 0x0B),
        (Equals, 0x0C),
        (Backslash, 0x0D),
        (Backspace, 0x0E),
        (Tab, 0x0F),
        (Q, 0x10),
        (W, 0x11),
        (E, 0x12),
        (R, 0x13),
        (T, 0x14),
        (Y, 0x15),
        (U, 0x16),
        (I, 0x17),
        (O, 0x18),
        (P, 0x19),
        (Grave, 0x1A),
        (LeftBracket, 0x1B),
        (Return, 0x1C),
        (A, 0x1D),
        (S, 0x1E),
        (D, 0x1F),
        (F, 0x20),
        (G, 0x21),
        (H, 0x22),
        (J, 0x23),
        (K, 0x24),
        (L, 0x25),
        (Semicolon, 0x26),
        (Apostrophe, 0x27),
        (RightBracket, 0x28),
        (Z, 0x29),
        (X, 0x2A),
        (C, 0x2B),
        (V, 0x2C),
        (B, 0x2D),
        (N, 0x2E),
        (M, 0x2F),
        (Comma, 0x30),
        (Period, 0x31),
        (Slash, 0x32),
        (NonUsBackslash, 0x33),
        (Space, 0x34),
        (RAlt, 0x35),
        (PageUp, 0x36),
        (PageDown, 0x37),
        (Insert, 0x38),
        (Delete, 0x39),
        (Up, 0x3A),
        (Left, 0x3B),
        (Right, 0x3C),
        (Down, 0x3D),
        (Home, 0x3E),
        (End, 0x3F),
        (KpMinus, 0x40),
        (KpDivide, 0x41),
        (Kp7, 0x42),
        (Kp8, 0x43),
        (Kp9, 0x44),
        (KpMultiply, 0x45),
        (Kp4, 0x46),
        (Kp5, 0x47),
        (Kp6, 0x48),
        (KpPlus, 0x49),
        (Kp1, 0x4A),
        (Kp2, 0x4B),
        (Kp3, 0x4C),
        (KpEnter, 0x4D),
        (Kp0, 0x4E),
        (KpComma, 0x4F),
        (KpPeriod, 0x50),
        (Application, 0x51),
        (F11, 0x52),
        (F12, 0x53),
        (F13, 0x54),
        (F14, 0x55),
        (F15, 0x56),
        (Pause, 0x60),
        (PrintScreen, 0x61),
        (F1, 0x62),
        (F2, 0x63),
        (F3, 0x64),
        (F4, 0x65),
        (F5, 0x66),
        (F6, 0x67),
        (F7, 0x68),
        (F8, 0x69),
        (F9, 0x6A),
        (F10, 0x6B),
        (LShift, 0x70),
        (RShift, 0x70),
        (CapsLock, 0x71),
        (NumLock, 0x72),
        (LAlt, 0x73),
        (LCtrl, 0x74),
        (RCtrl, 0x74),
    ];

    let mut map = [0u8; Scancode::COUNT];
    let mut i = 0;
    while i < ALL_SCANCODES.len() {
        let (scancode, pc98) = ALL_SCANCODES[i];
        map[scancode.index()] = pc98;
        i += 1;
    }
    map
}

pub fn parse_key_binding(host_name: &str, pc98_name: &str) -> Option<(Scancode, u8)> {
    let host = Scancode::from_name(host_name)?;
    let pc98 = pc98_scancode_from_name(pc98_name)?;
    Some((host, pc98))
}

/// Encodes a PC-88 keyboard matrix position as `row << 3 | column`. The high bit
/// stays clear so the forwarding layer can set it to mark a key release.
const fn pc88_cell(row: u8, column: u8) -> u8 {
    (row << 3) | column
}

/// Maps a PC-88 key name to its matrix code, used for `key.*` config overrides.
/// Cells follow the standard PC-8801 keyboard matrix (ports 0x00-0x0E).
pub fn pc88_matrix_code_from_name(name: &str) -> Option<u8> {
    let name_lower = name.to_ascii_lowercase();
    Some(match name_lower.as_str() {
        // Numeric keypad (matrix rows 0-1).
        "kp0" => pc88_cell(0, 0),
        "kp1" => pc88_cell(0, 1),
        "kp2" => pc88_cell(0, 2),
        "kp3" => pc88_cell(0, 3),
        "kp4" => pc88_cell(0, 4),
        "kp5" => pc88_cell(0, 5),
        "kp6" => pc88_cell(0, 6),
        "kp7" => pc88_cell(0, 7),
        "kp8" => pc88_cell(1, 0),
        "kp9" => pc88_cell(1, 1),
        "kpmultiply" => pc88_cell(1, 2),
        "kpplus" => pc88_cell(1, 3),
        "kpequals" => pc88_cell(1, 4),
        "kpcomma" => pc88_cell(1, 5),
        "kpperiod" => pc88_cell(1, 6),
        "return" | "kpenter" => pc88_cell(1, 7),
        // '@' and letters (matrix rows 2-5).
        "at" => pc88_cell(2, 0),
        "a" => pc88_cell(2, 1),
        "b" => pc88_cell(2, 2),
        "c" => pc88_cell(2, 3),
        "d" => pc88_cell(2, 4),
        "e" => pc88_cell(2, 5),
        "f" => pc88_cell(2, 6),
        "g" => pc88_cell(2, 7),
        "h" => pc88_cell(3, 0),
        "i" => pc88_cell(3, 1),
        "j" => pc88_cell(3, 2),
        "k" => pc88_cell(3, 3),
        "l" => pc88_cell(3, 4),
        "m" => pc88_cell(3, 5),
        "n" => pc88_cell(3, 6),
        "o" => pc88_cell(3, 7),
        "p" => pc88_cell(4, 0),
        "q" => pc88_cell(4, 1),
        "r" => pc88_cell(4, 2),
        "s" => pc88_cell(4, 3),
        "t" => pc88_cell(4, 4),
        "u" => pc88_cell(4, 5),
        "v" => pc88_cell(4, 6),
        "w" => pc88_cell(4, 7),
        "x" => pc88_cell(5, 0),
        "y" => pc88_cell(5, 1),
        "z" => pc88_cell(5, 2),
        // Symbol cluster (matrix row 5).
        "leftbracket" => pc88_cell(5, 3),
        "yen" => pc88_cell(5, 4),
        "rightbracket" => pc88_cell(5, 5),
        "caret" => pc88_cell(5, 6),
        "minus" => pc88_cell(5, 7),
        // Digits (matrix rows 6-7).
        "0" => pc88_cell(6, 0),
        "1" => pc88_cell(6, 1),
        "2" => pc88_cell(6, 2),
        "3" => pc88_cell(6, 3),
        "4" => pc88_cell(6, 4),
        "5" => pc88_cell(6, 5),
        "6" => pc88_cell(6, 6),
        "7" => pc88_cell(6, 7),
        "8" => pc88_cell(7, 0),
        "9" => pc88_cell(7, 1),
        // Punctuation (matrix row 7).
        "colon" => pc88_cell(7, 2),
        "semicolon" => pc88_cell(7, 3),
        "comma" => pc88_cell(7, 4),
        "period" => pc88_cell(7, 5),
        "slash" => pc88_cell(7, 6),
        "underscore" => pc88_cell(7, 7),
        // Control cluster (matrix row 8).
        "home" => pc88_cell(8, 0),
        "up" => pc88_cell(8, 1),
        "right" => pc88_cell(8, 2),
        "del" | "ins" | "bs" => pc88_cell(8, 3),
        "grph" => pc88_cell(8, 4),
        "kana" => pc88_cell(8, 5),
        "shift" => pc88_cell(8, 6),
        "ctrl" => pc88_cell(8, 7),
        // Matrix row 9.
        "stop" => pc88_cell(9, 0),
        "f1" => pc88_cell(9, 1),
        "f2" => pc88_cell(9, 2),
        "f3" => pc88_cell(9, 3),
        "f4" => pc88_cell(9, 4),
        "f5" => pc88_cell(9, 5),
        "space" => pc88_cell(9, 6),
        "esc" => pc88_cell(9, 7),
        // Matrix row 10.
        "tab" => pc88_cell(10, 0),
        "down" => pc88_cell(10, 1),
        "left" => pc88_cell(10, 2),
        "help" => pc88_cell(10, 3),
        "copy" => pc88_cell(10, 4),
        "kpminus" => pc88_cell(10, 5),
        "kpdivide" => pc88_cell(10, 6),
        "caps" => pc88_cell(10, 7),
        // Matrix row 11.
        "rollup" => pc88_cell(11, 0),
        "rolldown" => pc88_cell(11, 1),
        _ => return None,
    })
}

pub fn parse_key_binding_pc88(host_name: &str, pc88_name: &str) -> Option<(Scancode, u8)> {
    let host = Scancode::from_name(host_name)?;
    let code = pc88_matrix_code_from_name(pc88_name)?;
    Some((host, code))
}

/// Default PC-88 key map: host scancodes to 16x8 matrix codes derived from the
/// PC-8801 keyboard matrix. Host keys with no PC-88 equivalent map to an unused
/// matrix cell so they have no effect.
#[allow(clippy::just_underscores_and_digits)]
const fn build_pc88_default_map() -> [u8; Scancode::COUNT] {
    use Scancode::*;

    /// Unused matrix cell (row 15 is not part of the PC-88 matrix).
    const UNMAPPED: u8 = pc88_cell(15, 7);

    const ALL_SCANCODES: &[(Scancode, u8)] = &[
        // Numeric keypad (matrix rows 0-1).
        (Kp0, pc88_cell(0, 0)),
        (Kp1, pc88_cell(0, 1)),
        (Kp2, pc88_cell(0, 2)),
        (Kp3, pc88_cell(0, 3)),
        (Kp4, pc88_cell(0, 4)),
        (Kp5, pc88_cell(0, 5)),
        (Kp6, pc88_cell(0, 6)),
        (Kp7, pc88_cell(0, 7)),
        (Kp8, pc88_cell(1, 0)),
        (Kp9, pc88_cell(1, 1)),
        (KpMultiply, pc88_cell(1, 2)),
        (KpPlus, pc88_cell(1, 3)),
        // Matrix cell (1, 4) is the keypad '=' key; no host scancode maps to it
        // by default, but the `kpequals` config name can bind it.
        (KpComma, pc88_cell(1, 5)),
        (KpPeriod, pc88_cell(1, 6)),
        // RETURN sits at (1, 7); both Enter keys map to it.
        (Return, pc88_cell(1, 7)),
        (KpEnter, pc88_cell(1, 7)),
        // '@' and letters (matrix rows 2-5).
        (LeftBracket, pc88_cell(2, 0)),
        (A, pc88_cell(2, 1)),
        (B, pc88_cell(2, 2)),
        (C, pc88_cell(2, 3)),
        (D, pc88_cell(2, 4)),
        (E, pc88_cell(2, 5)),
        (F, pc88_cell(2, 6)),
        (G, pc88_cell(2, 7)),
        (H, pc88_cell(3, 0)),
        (I, pc88_cell(3, 1)),
        (J, pc88_cell(3, 2)),
        (K, pc88_cell(3, 3)),
        (L, pc88_cell(3, 4)),
        (M, pc88_cell(3, 5)),
        (N, pc88_cell(3, 6)),
        (O, pc88_cell(3, 7)),
        (P, pc88_cell(4, 0)),
        (Q, pc88_cell(4, 1)),
        (R, pc88_cell(4, 2)),
        (S, pc88_cell(4, 3)),
        (T, pc88_cell(4, 4)),
        (U, pc88_cell(4, 5)),
        (V, pc88_cell(4, 6)),
        (W, pc88_cell(4, 7)),
        (X, pc88_cell(5, 0)),
        (Y, pc88_cell(5, 1)),
        (Z, pc88_cell(5, 2)),
        // Symbol cluster (matrix row 5): '[' ']' yen '^' '-' mapped from the
        // matching US-layout host keys.
        (RightBracket, pc88_cell(5, 3)),
        (NonUsBackslash, pc88_cell(5, 4)),
        (Backslash, pc88_cell(5, 5)),
        (Equals, pc88_cell(5, 6)),
        (Minus, pc88_cell(5, 7)),
        // Digits (matrix rows 6-7).
        (_0, pc88_cell(6, 0)),
        (_1, pc88_cell(6, 1)),
        (_2, pc88_cell(6, 2)),
        (_3, pc88_cell(6, 3)),
        (_4, pc88_cell(6, 4)),
        (_5, pc88_cell(6, 5)),
        (_6, pc88_cell(6, 6)),
        (_7, pc88_cell(6, 7)),
        (_8, pc88_cell(7, 0)),
        (_9, pc88_cell(7, 1)),
        // Punctuation (matrix row 7).
        (Apostrophe, pc88_cell(7, 2)),
        (Semicolon, pc88_cell(7, 3)),
        (Comma, pc88_cell(7, 4)),
        (Period, pc88_cell(7, 5)),
        (Slash, pc88_cell(7, 6)),
        // Control cluster (matrix row 8). The "Del Ins" key serves backspace and
        // delete; Grph/Kana/Shift/Ctrl carry the modifiers.
        (Home, pc88_cell(8, 0)),
        (Up, pc88_cell(8, 1)),
        (Right, pc88_cell(8, 2)),
        (Backspace, pc88_cell(8, 3)),
        (Delete, pc88_cell(8, 3)),
        (LAlt, pc88_cell(8, 4)),
        (RAlt, pc88_cell(8, 5)),
        (LShift, pc88_cell(8, 6)),
        (RShift, pc88_cell(8, 6)),
        (LCtrl, pc88_cell(8, 7)),
        (RCtrl, pc88_cell(8, 7)),
        // Matrix row 9: Stop, F1-F5, space, escape.
        (Pause, pc88_cell(9, 0)),
        (F1, pc88_cell(9, 1)),
        (F2, pc88_cell(9, 2)),
        (F3, pc88_cell(9, 3)),
        (F4, pc88_cell(9, 4)),
        (F5, pc88_cell(9, 5)),
        (Space, pc88_cell(9, 6)),
        (Escape, pc88_cell(9, 7)),
        // Matrix row 10.
        (Tab, pc88_cell(10, 0)),
        (Down, pc88_cell(10, 1)),
        (Left, pc88_cell(10, 2)),
        (End, pc88_cell(10, 3)),
        (PrintScreen, pc88_cell(10, 4)),
        (KpMinus, pc88_cell(10, 5)),
        (KpDivide, pc88_cell(10, 6)),
        (CapsLock, pc88_cell(10, 7)),
        // Matrix row 11: Roll Up / Roll Down.
        (PageUp, pc88_cell(11, 0)),
        (PageDown, pc88_cell(11, 1)),
    ];

    let mut map = [UNMAPPED; Scancode::COUNT];
    let mut i = 0;
    while i < ALL_SCANCODES.len() {
        let (scancode, code) = ALL_SCANCODES[i];
        map[scancode.index()] = code;
        i += 1;
    }
    map
}

#[cfg(test)]
mod tests {
    use sdl3::keyboard::Scancode;

    use super::{KeyMap, KeyboardForwardingState};

    #[test]
    fn pc88_return_and_backspace_map_to_standard_matrix_cells() {
        use super::{KeyMap, pc88_cell, pc88_matrix_code_from_name};

        // RETURN is at matrix row 1, column 7 (shared with the keypad enter);
        // the "Del Ins" key at row 8, column 3 serves backspace and delete.
        let map = KeyMap::new_pc88();
        assert_eq!(map.lookup(Scancode::Return), pc88_cell(1, 7));
        assert_eq!(map.lookup(Scancode::KpEnter), pc88_cell(1, 7));
        assert_eq!(map.lookup(Scancode::Backspace), pc88_cell(8, 3));
        assert_eq!(map.lookup(Scancode::Delete), pc88_cell(8, 3));

        assert_eq!(pc88_matrix_code_from_name("return"), Some(pc88_cell(1, 7)));
        assert_eq!(pc88_matrix_code_from_name("bs"), Some(pc88_cell(8, 3)));
    }

    #[test]
    fn normal_left_alt_is_forwarded_to_the_guest() {
        let mut keyboard_forwarding_state = KeyboardForwardingState::new();
        let key_map = KeyMap::new();

        keyboard_forwarding_state.handle_key_down(Some(Scancode::LAlt), false, false, &key_map);
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            Some(0x73)
        );
        assert!(
            keyboard_forwarding_state
                .pending_released_pc98_scancodes()
                .is_empty()
        );

        let key_up_scancode =
            keyboard_forwarding_state.handle_key_up(Some(Scancode::LAlt), false, &key_map);
        assert_eq!(key_up_scancode, Some(0xF3));
    }

    #[test]
    fn gui_combo_does_not_forward_left_alt_or_function_keys() {
        let mut keyboard_forwarding_state = KeyboardForwardingState::new();
        let key_map = KeyMap::new();

        keyboard_forwarding_state.handle_key_down(None, true, false, &key_map);
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            None
        );
        assert!(
            keyboard_forwarding_state
                .pending_released_pc98_scancodes()
                .is_empty()
        );

        keyboard_forwarding_state.handle_key_down(Some(Scancode::LAlt), true, false, &key_map);
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            None
        );
        assert!(
            keyboard_forwarding_state
                .pending_released_pc98_scancodes()
                .is_empty()
        );

        keyboard_forwarding_state.handle_key_down(Some(Scancode::F9), true, false, &key_map);
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            None
        );
        assert!(
            keyboard_forwarding_state
                .pending_released_pc98_scancodes()
                .is_empty()
        );

        let function_key_up_scancode =
            keyboard_forwarding_state.handle_key_up(Some(Scancode::F9), false, &key_map);
        assert_eq!(function_key_up_scancode, None);

        let left_alt_key_up_scancode =
            keyboard_forwarding_state.handle_key_up(Some(Scancode::LAlt), false, &key_map);
        assert_eq!(left_alt_key_up_scancode, None);
    }

    #[test]
    fn gui_activation_releases_guest_keys_that_were_already_held() {
        let mut keyboard_forwarding_state = KeyboardForwardingState::new();
        let key_map = KeyMap::new();

        keyboard_forwarding_state.handle_key_down(Some(Scancode::LAlt), false, false, &key_map);
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            Some(0x73)
        );

        keyboard_forwarding_state.handle_key_down(None, true, false, &key_map);
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            None
        );
        assert_eq!(
            keyboard_forwarding_state.pending_released_pc98_scancodes(),
            [0xF3]
        );

        let left_alt_key_up_scancode =
            keyboard_forwarding_state.handle_key_up(Some(Scancode::LAlt), false, &key_map);
        assert_eq!(left_alt_key_up_scancode, None);
    }

    #[test]
    fn forwarding_recovers_after_gui_is_released() {
        let mut keyboard_forwarding_state = KeyboardForwardingState::new();
        let key_map = KeyMap::new();

        keyboard_forwarding_state.handle_key_down(None, true, false, &key_map);
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            None
        );
        assert!(
            keyboard_forwarding_state
                .pending_released_pc98_scancodes()
                .is_empty()
        );

        keyboard_forwarding_state.handle_key_down(Some(Scancode::LAlt), true, false, &key_map);
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            None
        );
        assert!(
            keyboard_forwarding_state
                .pending_released_pc98_scancodes()
                .is_empty()
        );

        keyboard_forwarding_state.handle_key_down(Some(Scancode::A), false, false, &key_map);
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            Some(0x1D)
        );
        assert!(
            keyboard_forwarding_state
                .pending_released_pc98_scancodes()
                .is_empty()
        );
    }
}
