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
    ButtonC,
    ButtonX,
    ButtonY,
    ButtonZ,
    Run,
    Select,
}

impl JoystickKey {
    /// Maps a host scancode to a joystick input, if it is one of the fallback keys.
    ///
    /// Arrows drive the directions and Z/X the two face buttons; the extra
    /// 6-button pad inputs sit on nearby keys (C/V/B/N for C/X/Y/Z, Space for
    /// Run, Return for Select).
    pub(crate) fn from_scancode(scancode: Option<Scancode>) -> Option<Self> {
        match scancode {
            Some(Scancode::Up) => Some(Self::Up),
            Some(Scancode::Down) => Some(Self::Down),
            Some(Scancode::Left) => Some(Self::Left),
            Some(Scancode::Right) => Some(Self::Right),
            Some(Scancode::Z) => Some(Self::Trigger1),
            Some(Scancode::X) => Some(Self::Trigger2),
            Some(Scancode::C) => Some(Self::ButtonC),
            Some(Scancode::V) => Some(Self::ButtonX),
            Some(Scancode::B) => Some(Self::ButtonY),
            Some(Scancode::N) => Some(Self::ButtonZ),
            Some(Scancode::Space) => Some(Self::Run),
            Some(Scancode::Return) => Some(Self::Select),
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
            Self::ButtonC => state.button_c = pressed,
            Self::ButtonX => state.button_x = pressed,
            Self::ButtonY => state.button_y = pressed,
            Self::ButtonZ => state.button_z = pressed,
            Self::Run => state.run = pressed,
            Self::Select => state.select = pressed,
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

#[derive(Clone, Copy)]
pub struct KeyMap {
    mappings: [u8; Scancode::COUNT],
    shifted_mappings: [u8; Scancode::COUNT],
    resolve_modifiers: bool,
}

impl KeyMap {
    pub const fn new() -> Self {
        let mappings = build_default_map();
        Self {
            mappings,
            shifted_mappings: mappings,
            resolve_modifiers: false,
        }
    }

    pub const fn new_pc88() -> Self {
        let mappings = build_pc88_default_map();
        Self {
            mappings,
            shifted_mappings: mappings,
            resolve_modifiers: false,
        }
    }

    pub const fn new_pc60() -> Self {
        Self {
            mappings: build_pc60_default_map(),
            shifted_mappings: build_pc60_shifted_map(),
            resolve_modifiers: true,
        }
    }

    /// Sharp X1 key map: host scancodes to sub-CPU key sub-ids. The sub-CPU
    /// resolves the modifier state itself, so modifiers forward as their own
    /// sub-ids and this map ignores the host modifier state. Punctuation keys
    /// whose native virtual-key codes exceed 0x7F use the spare low sub-ids the
    /// bus remaps (so the host release flag, bit 7, never collides).
    pub const fn new_x1() -> Self {
        let mappings = build_x1_default_map();
        Self {
            mappings,
            shifted_mappings: mappings,
            resolve_modifiers: false,
        }
    }

    /// FM-7 key map: host scancodes to FM-7 physical scancodes (0x00-0x66, the
    /// key identifier the machine's keyboard tables index). The machine resolves
    /// Shift / Ctrl / Graph / Kana internally, so modifiers forward as their own
    /// physical scancodes and this map ignores the host modifier state. The
    /// forwarding layer applies the release flag (bit 7), so every mapped value
    /// stays within 0x00-0x7F.
    pub const fn new_fm7() -> Self {
        let mappings = build_fm7_default_map();
        Self {
            mappings,
            shifted_mappings: mappings,
            resolve_modifiers: false,
        }
    }

    /// PC-88VA key map: host scancodes to VA keycodes (the values read at port
    /// 0x1C1). The VA keycode interface reuses the PC-98 scan-code protocol, so the
    /// base map matches the PC-98 default; the machine derives the 88-compatible
    /// scan matrix from the keycode internally.
    pub const fn new_pc88va() -> Self {
        let mappings = build_pc88va_default_map();
        Self {
            mappings,
            shifted_mappings: mappings,
            resolve_modifiers: false,
        }
    }

    /// FM Towns key map: host scancodes to FM Towns JIS scancodes (the second
    /// byte of the keyboard's two-byte serial packet). The machine expands each
    /// forwarded keycode into the make/break packet; modifiers forward as their
    /// own JIS scancodes (Ctrl 0x52, Shift 0x53).
    pub const fn new_towns() -> Self {
        let mappings = build_towns_default_map();
        Self {
            mappings,
            shifted_mappings: mappings,
            resolve_modifiers: false,
        }
    }

    /// X68000 key map: host scancodes to native keyboard matrix codes.
    pub const fn new_x68k() -> Self {
        let mappings = build_x68k_default_map();
        Self {
            mappings,
            shifted_mappings: mappings,
            resolve_modifiers: false,
        }
    }

    /// PC/AT key map for the 106-key JIS (OADG) layout: host scancodes to
    /// set-1 key ids (E0-extended keys use the machineat synthetic ids). The
    /// machine expands each forwarded id into the set-2 make/break sequence;
    /// modifiers forward as their own ids.
    pub const fn new_at() -> Self {
        let mappings = build_at_default_map();
        Self {
            mappings,
            shifted_mappings: mappings,
            resolve_modifiers: false,
        }
    }

    pub fn set(&mut self, host: Scancode, pc98_code: u8) {
        self.mappings[host.index()] = pc98_code;
    }

    pub fn lookup(&self, host: Scancode) -> u8 {
        self.mappings[host.index()]
    }

    /// Resolves a host scancode to a guest keycode, applying the modifier keys
    /// when the target keyboard expects a pre-resolved code (PC-6000). The
    /// PC-88/PC-98 matrices forward Shift/Ctrl as their own cells, so those maps
    /// ignore the modifier state here.
    pub fn resolve(&self, host: Scancode, shift: bool, ctrl: bool) -> u8 {
        let index = host.index();
        let base = self.mappings[index];
        if !self.resolve_modifiers {
            return base;
        }
        if ctrl && (0x40..=0x5F).contains(&base) {
            return base & 0x1F;
        }
        if shift {
            return self.shifted_mappings[index];
        }
        base
    }
}

pub(crate) struct KeyboardForwardingState {
    shortcut_modifier_active: bool,
    guest_pressed_pc98_scancodes: [Option<u8>; Scancode::COUNT],
    pending_pressed_pc98_scancode: Option<u8>,
    pending_released_pc98_scancodes: Vec<u8>,
}

impl KeyboardForwardingState {
    pub(crate) fn new() -> Self {
        Self {
            shortcut_modifier_active: false,
            guest_pressed_pc98_scancodes: [None; Scancode::COUNT],
            pending_pressed_pc98_scancode: None,
            pending_released_pc98_scancodes: Vec::with_capacity(Scancode::COUNT),
        }
    }

    pub(crate) fn handle_key_down(
        &mut self,
        scancode: Option<Scancode>,
        shortcut_modifier_active: bool,
        shift_held: bool,
        ctrl_held: bool,
        repeat: bool,
        key_map: &KeyMap,
    ) {
        self.clear_pending_actions();

        if repeat {
            return;
        }

        if shortcut_modifier_active && !self.shortcut_modifier_active {
            self.release_all_guest_keys();
        }
        self.shortcut_modifier_active = shortcut_modifier_active;

        if self.shortcut_modifier_active {
            return;
        }

        let Some(scancode) = scancode else {
            return;
        };

        let scancode_index = scancode.index();
        if self.guest_pressed_pc98_scancodes[scancode_index].is_some() {
            return;
        }

        let pc98_scancode = key_map.resolve(scancode, shift_held, ctrl_held);
        self.guest_pressed_pc98_scancodes[scancode_index] = Some(pc98_scancode);
        self.pending_pressed_pc98_scancode = Some(pc98_scancode);
    }

    pub(crate) fn handle_key_up(&mut self, scancode: Option<Scancode>, repeat: bool) -> Option<u8> {
        if repeat {
            return None;
        }

        let scancode = scancode?;
        let scancode_index = scancode.index();
        let pc98_scancode = self.guest_pressed_pc98_scancodes[scancode_index]?;
        self.guest_pressed_pc98_scancodes[scancode_index] = None;

        // The press-time code is authoritative: the live modifier state may have
        // changed before release, so it is not re-resolved here.
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
        // JIS keys on a Japanese host keyboard.
        (International1, 0x33), // Ro (backslash / underscore)
        (International2, 0x72), // Katakana/Hiragana (KANA)
        (International3, 0x0D), // Yen
        (International4, 0x35), // Henkan (XFER)
        (International5, 0x51), // Muhenkan (NFER)
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

/// PC-88VA default key map: the PC-98 keycode base with the one VA difference,
/// the dedicated keypad ENTER keycode (the PC-98 keypad has '=' there instead).
const fn build_pc88va_default_map() -> [u8; Scancode::COUNT] {
    let mut map = build_default_map();
    map[Scancode::KpEnter.index()] = 0x79;
    map
}

/// Default Sharp X1 key map: host scancodes to sub-CPU key sub-ids. Alphanumeric
/// and control keys carry their virtual-key code directly; punctuation keys whose
/// virtual-key codes exceed 0x7F carry the spare sub-ids the bus remaps. Unmapped
/// host keys carry 0x00, which the sub-CPU treats as no key.
#[allow(clippy::just_underscores_and_digits)]
const fn build_x1_default_map() -> [u8; Scancode::COUNT] {
    use Scancode::*;

    /// No-key code for host keys without an X1 equivalent.
    const UNMAPPED: u8 = 0x00;

    // Modifier virtual-key codes (identity-mapped by the bus).
    const VK_SHIFT: u8 = 0x10;
    const VK_CTRL: u8 = 0x11;
    const VK_GRAPH: u8 = 0x12;
    const VK_CAPS: u8 = 0x14;

    // Spare sub-ids the bus remaps to the punctuation virtual-key codes > 0x7F.
    const SUBID_COLON: u8 = 0x01; // -> ':'
    const SUBID_SEMICOLON: u8 = 0x02; // -> ';'
    const SUBID_COMMA: u8 = 0x04; // -> ','
    const SUBID_MINUS: u8 = 0x05; // -> '-'
    const SUBID_PERIOD: u8 = 0x06; // -> '.'
    const SUBID_SLASH: u8 = 0x07; // -> '/'
    const SUBID_AT: u8 = 0x0A; // -> '@'
    const SUBID_LEFT_BRACKET: u8 = 0x0B; // -> '['
    const SUBID_BACKSLASH: u8 = 0x0C; // -> '\'
    const SUBID_RIGHT_BRACKET: u8 = 0x0E; // -> ']'
    const SUBID_CARET: u8 = 0x0F; // -> '^'

    const ALL_SCANCODES: &[(Scancode, u8)] = &[
        // Letters carry their virtual-key codes (0x41-0x5A); the sub-CPU applies
        // Shift / Caps to pick the case.
        (A, 0x41),
        (B, 0x42),
        (C, 0x43),
        (D, 0x44),
        (E, 0x45),
        (F, 0x46),
        (G, 0x47),
        (H, 0x48),
        (I, 0x49),
        (J, 0x4A),
        (K, 0x4B),
        (L, 0x4C),
        (M, 0x4D),
        (N, 0x4E),
        (O, 0x4F),
        (P, 0x50),
        (Q, 0x51),
        (R, 0x52),
        (S, 0x53),
        (T, 0x54),
        (U, 0x55),
        (V, 0x56),
        (W, 0x57),
        (X, 0x58),
        (Y, 0x59),
        (Z, 0x5A),
        // Digits (0x30-0x39).
        (_0, 0x30),
        (_1, 0x31),
        (_2, 0x32),
        (_3, 0x33),
        (_4, 0x34),
        (_5, 0x35),
        (_6, 0x36),
        (_7, 0x37),
        (_8, 0x38),
        (_9, 0x39),
        // Control keys and cursor cluster.
        (Space, 0x20),
        (Return, 0x0D),
        (KpEnter, 0x0D),
        (Backspace, 0x08),
        (Delete, 0x2E),
        (Insert, 0x2D),
        (Tab, 0x09),
        (Escape, 0x1B),
        (Home, 0x24),
        (End, 0x23),
        (PageUp, 0x21),
        (PageDown, 0x22),
        (Left, 0x25),
        (Up, 0x26),
        (Right, 0x27),
        (Down, 0x28),
        // Numeric keypad (virtual keys 0x60-0x6F)
        (Kp0, 0x60),
        (Kp1, 0x61),
        (Kp2, 0x62),
        (Kp3, 0x63),
        (Kp4, 0x64),
        (Kp5, 0x65),
        (Kp6, 0x66),
        (Kp7, 0x67),
        (Kp8, 0x68),
        (Kp9, 0x69),
        (KpMultiply, 0x6A),
        (KpPlus, 0x6B),
        (KpComma, 0x6C),
        (KpMinus, 0x6D),
        (KpPeriod, 0x6E),
        (KpDivide, 0x6F),
        // Punctuation via the bus-remapped spare sub-ids.
        (Semicolon, SUBID_SEMICOLON),
        (Apostrophe, SUBID_COLON),
        (Comma, SUBID_COMMA),
        (Period, SUBID_PERIOD),
        (Slash, SUBID_SLASH),
        (Minus, SUBID_MINUS),
        (Grave, SUBID_AT),
        (LeftBracket, SUBID_LEFT_BRACKET),
        (RightBracket, SUBID_RIGHT_BRACKET),
        (Backslash, SUBID_BACKSLASH),
        (Equals, SUBID_CARET),
        // Modifiers forward as their own virtual-key codes.
        (LShift, VK_SHIFT),
        (RShift, VK_SHIFT),
        (LCtrl, VK_CTRL),
        (RCtrl, VK_CTRL),
        (LAlt, VK_GRAPH),
        (RAlt, VK_GRAPH),
        (CapsLock, VK_CAPS),
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

pub fn parse_key_binding(host_name: &str, pc98_name: &str) -> Option<(Scancode, u8)> {
    let host = Scancode::from_name(host_name)?;
    let pc98 = pc98_scancode_from_name(pc98_name)?;
    Some((host, pc98))
}

/// FM-7 default key map: host scancodes to FM-7 physical scancodes. The values
/// are the indices used by the machine's keycode tables (ESC = 0x01, digits
/// `1`-`0` = 0x02-0x0B, `Q` = 0x11, and so on). Host keys without an FM-7
/// equivalent stay UNMAPPED; the machine ignores that scancode.
const fn build_fm7_default_map() -> [u8; Scancode::COUNT] {
    use Scancode::*;

    /// No-key code for host keys without an FM-7 equivalent.
    const UNMAPPED: u8 = 0x00;

    /// Ctrl key physical scancode.
    const SCANCODE_CTRL: u8 = 0x52;
    /// Left Shift physical scancode.
    const SCANCODE_SHIFT_LEFT: u8 = 0x53;
    /// Right Shift physical scancode.
    const SCANCODE_SHIFT_RIGHT: u8 = 0x54;
    /// Caps lock physical scancode (toggles the caps latch).
    const SCANCODE_CAPS: u8 = 0x55;
    /// Graph key physical scancode.
    const SCANCODE_GRAPH: u8 = 0x56;
    /// Break key physical scancode (drives the FIRQ break line directly).
    const SCANCODE_BREAK: u8 = 0x5C;

    const ALL_SCANCODES: &[(Scancode, u8)] = &[
        // Top row: ESC, digits, and the two right-hand symbol keys.
        (Escape, 0x01),
        (_1, 0x02),
        (_2, 0x03),
        (_3, 0x04),
        (_4, 0x05),
        (_5, 0x06),
        (_6, 0x07),
        (_7, 0x08),
        (_8, 0x09),
        (_9, 0x0A),
        (_0, 0x0B),
        (Minus, 0x0C),
        (Equals, 0x0D),
        (Backslash, 0x0E),
        (Backspace, 0x0F),
        // Second row.
        (Tab, 0x10),
        (Q, 0x11),
        (W, 0x12),
        (E, 0x13),
        (R, 0x14),
        (T, 0x15),
        (Y, 0x16),
        (U, 0x17),
        (I, 0x18),
        (O, 0x19),
        (P, 0x1A),
        (Grave, 0x1B),
        (LeftBracket, 0x1C),
        (Return, 0x1D),
        // Home row.
        (A, 0x1E),
        (S, 0x1F),
        (D, 0x20),
        (F, 0x21),
        (G, 0x22),
        (H, 0x23),
        (J, 0x24),
        (K, 0x25),
        (L, 0x26),
        (Semicolon, 0x27),
        (Apostrophe, 0x28),
        (RightBracket, 0x29),
        // Bottom row.
        (Z, 0x2A),
        (X, 0x2B),
        (C, 0x2C),
        (V, 0x2D),
        (B, 0x2E),
        (N, 0x2F),
        (M, 0x30),
        (Comma, 0x31),
        (Period, 0x32),
        (Slash, 0x33),
        (NonUsBackslash, 0x34),
        (Space, 0x35),
        // Numeric keypad.
        (KpMultiply, 0x36),
        (KpDivide, 0x37),
        (KpPlus, 0x38),
        (KpMinus, 0x39),
        (Kp7, 0x3A),
        (Kp8, 0x3B),
        (Kp9, 0x3C),
        (Kp4, 0x3E),
        (Kp5, 0x3F),
        (Kp6, 0x40),
        (KpComma, 0x41),
        (Kp1, 0x42),
        (Kp2, 0x43),
        (Kp3, 0x44),
        (KpEnter, 0x45),
        (Kp0, 0x46),
        (KpPeriod, 0x47),
        // Edit and cursor cluster.
        (Home, 0x49),
        (Delete, 0x4B),
        (Insert, 0x4C),
        (Up, 0x4D),
        (Left, 0x4F),
        (Down, 0x50),
        (Right, 0x51),
        // Modifiers forward as their own physical scancodes.
        (LCtrl, SCANCODE_CTRL),
        (RCtrl, SCANCODE_CTRL),
        (LShift, SCANCODE_SHIFT_LEFT),
        (RShift, SCANCODE_SHIFT_RIGHT),
        (CapsLock, SCANCODE_CAPS),
        (LAlt, SCANCODE_GRAPH),
        (RAlt, SCANCODE_GRAPH),
        (Pause, SCANCODE_BREAK),
        // Function keys F1-F10.
        (F1, 0x5D),
        (F2, 0x5E),
        (F3, 0x5F),
        (F4, 0x60),
        (F5, 0x61),
        (F6, 0x62),
        (F7, 0x63),
        (F8, 0x64),
        (F9, 0x65),
        (F10, 0x66),
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

/// FM Towns default key map: host scancodes to FM Towns JIS scancodes. Keys with
/// no natural FM Towns equivalent stay at 0x00 (NULL); the machine ignores NULL.
const fn build_towns_default_map() -> [u8; Scancode::COUNT] {
    use Scancode::*;

    const ALL_SCANCODES: &[(Scancode, u8)] = &[
        (Escape, 0x01),
        (_1, 0x02),
        (_2, 0x03),
        (_3, 0x04),
        (_4, 0x05),
        (_5, 0x06),
        (_6, 0x07),
        (_7, 0x08),
        (_8, 0x09),
        (_9, 0x0A),
        (_0, 0x0B),
        (Minus, 0x0C),
        (Equals, 0x0D),
        (Backslash, 0x0E),
        (Backspace, 0x0F),
        (Tab, 0x10),
        (Q, 0x11),
        (W, 0x12),
        (E, 0x13),
        (R, 0x14),
        (T, 0x15),
        (Y, 0x16),
        (U, 0x17),
        (I, 0x18),
        (O, 0x19),
        (P, 0x1A),
        (Grave, 0x1B),
        (LeftBracket, 0x1C),
        (Return, 0x1D),
        (A, 0x1E),
        (S, 0x1F),
        (D, 0x20),
        (F, 0x21),
        (G, 0x22),
        (H, 0x23),
        (J, 0x24),
        (K, 0x25),
        (L, 0x26),
        (Semicolon, 0x27),
        (Apostrophe, 0x28),
        (RightBracket, 0x29),
        (Z, 0x2A),
        (X, 0x2B),
        (C, 0x2C),
        (V, 0x2D),
        (B, 0x2E),
        (N, 0x2F),
        (M, 0x30),
        (Comma, 0x31),
        (Period, 0x32),
        (Slash, 0x33),
        (NonUsBackslash, 0x34),
        (Space, 0x35),
        (KpMultiply, 0x36),
        (KpDivide, 0x37),
        (KpPlus, 0x38),
        (KpMinus, 0x39),
        (Kp7, 0x3A),
        (Kp8, 0x3B),
        (Kp9, 0x3C),
        (Kp4, 0x3E),
        (Kp5, 0x3F),
        (Kp6, 0x40),
        (Kp1, 0x42),
        (Kp2, 0x43),
        (Kp3, 0x44),
        (KpEnter, 0x45),
        (Kp0, 0x46),
        (KpPeriod, 0x47),
        (Insert, 0x48),
        (Delete, 0x4B),
        (Up, 0x4D),
        (Home, 0x4E),
        (Left, 0x4F),
        (Down, 0x50),
        (Right, 0x51),
        (LCtrl, 0x52),
        (RCtrl, 0x52),
        (LShift, 0x53),
        (RShift, 0x53),
        (CapsLock, 0x55),
        (F12, 0x5B),
        (LAlt, 0x5C),
        (RAlt, 0x5C),
        (F1, 0x5D),
        (F2, 0x5E),
        (F3, 0x5F),
        (F4, 0x60),
        (F5, 0x61),
        (F6, 0x62),
        (F7, 0x63),
        (F8, 0x64),
        (F9, 0x65),
        (F10, 0x66),
        (F11, 0x69),
        (End, 0x72),
        (PageDown, 0x73),
        (Pause, 0x7C),
        (PrintScreen, 0x7D),
    ];

    let mut map = [0u8; Scancode::COUNT];
    let mut i = 0;
    while i < ALL_SCANCODES.len() {
        let (scancode, towns) = ALL_SCANCODES[i];
        map[scancode.index()] = towns;
        i += 1;
    }
    map
}

/// Maps an X68000 key name to its native `row << 3 | column` code.
pub fn x68k_scancode_from_name(name: &str) -> Option<u8> {
    let name = name.to_ascii_lowercase();
    Some(match name.as_str() {
        "esc" => 0x01,
        "1" => 0x02,
        "2" => 0x03,
        "3" => 0x04,
        "4" => 0x05,
        "5" => 0x06,
        "6" => 0x07,
        "7" => 0x08,
        "8" => 0x09,
        "9" => 0x0A,
        "0" => 0x0B,
        "minus" => 0x0C,
        "caret" => 0x0D,
        "yen" => 0x0E,
        "bs" => 0x0F,
        "tab" => 0x10,
        "q" => 0x11,
        "w" => 0x12,
        "e" => 0x13,
        "r" => 0x14,
        "t" => 0x15,
        "y" => 0x16,
        "u" => 0x17,
        "i" => 0x18,
        "o" => 0x19,
        "p" => 0x1A,
        "at" => 0x1B,
        "leftbracket" => 0x1C,
        "return" => 0x1D,
        "a" => 0x1E,
        "s" => 0x1F,
        "d" => 0x20,
        "f" => 0x21,
        "g" => 0x22,
        "h" => 0x23,
        "j" => 0x24,
        "k" => 0x25,
        "l" => 0x26,
        "semicolon" => 0x27,
        "colon" => 0x28,
        "rightbracket" => 0x29,
        "z" => 0x2A,
        "x" => 0x2B,
        "c" => 0x2C,
        "v" => 0x2D,
        "b" => 0x2E,
        "n" => 0x2F,
        "m" => 0x30,
        "comma" => 0x31,
        "period" => 0x32,
        "slash" => 0x33,
        "underscore" => 0x34,
        "space" => 0x35,
        "home" => 0x36,
        "del" => 0x37,
        "rollup" => 0x38,
        "rolldown" => 0x39,
        "undo" => 0x3A,
        "left" => 0x3B,
        "up" => 0x3C,
        "right" => 0x3D,
        "down" => 0x3E,
        "clear" => 0x3F,
        "kpdivide" => 0x40,
        "kpmultiply" => 0x41,
        "kpminus" => 0x42,
        "kp7" => 0x43,
        "kp8" => 0x44,
        "kp9" => 0x45,
        "kpplus" => 0x46,
        "kp4" => 0x47,
        "kp5" => 0x48,
        "kp6" => 0x49,
        "kpequals" => 0x4A,
        "kp1" => 0x4B,
        "kp2" => 0x4C,
        "kp3" => 0x4D,
        "kpenter" => 0x4E,
        "kp0" => 0x4F,
        "kpcomma" => 0x50,
        "kpperiod" => 0x51,
        "symbol" => 0x52,
        "register" => 0x53,
        "help" => 0x54,
        "xf1" => 0x55,
        "xf2" => 0x56,
        "xf3" => 0x57,
        "xf4" => 0x58,
        "xf5" => 0x59,
        "kana" => 0x5A,
        "romaji" => 0x5B,
        "code" => 0x5C,
        "caps" => 0x5D,
        "ins" => 0x5E,
        "hiragana" => 0x5F,
        "fullwidth" => 0x60,
        "break" => 0x61,
        "copy" => 0x62,
        "f1" => 0x63,
        "f2" => 0x64,
        "f3" => 0x65,
        "f4" => 0x66,
        "f5" => 0x67,
        "f6" => 0x68,
        "f7" => 0x69,
        "f8" => 0x6A,
        "f9" => 0x6B,
        "f10" => 0x6C,
        "shift" => 0x70,
        "ctrl" => 0x71,
        "opt1" => 0x72,
        "opt2" => 0x73,
        _ => return None,
    })
}

/// Parses an X68000 key binding.
pub fn parse_key_binding_x68k(host_name: &str, x68k_name: &str) -> Option<(Scancode, u8)> {
    let host = Scancode::from_name(host_name)?;
    let code = x68k_scancode_from_name(x68k_name)?;
    Some((host, code))
}

#[allow(clippy::just_underscores_and_digits)]
const fn build_x68k_default_map() -> [u8; Scancode::COUNT] {
    use Scancode::*;
    const ALL_SCANCODES: &[(Scancode, u8)] = &[
        (Escape, 0x01),
        (_1, 0x02),
        (_2, 0x03),
        (_3, 0x04),
        (_4, 0x05),
        (_5, 0x06),
        (_6, 0x07),
        (_7, 0x08),
        (_8, 0x09),
        (_9, 0x0A),
        (_0, 0x0B),
        (Minus, 0x0C),
        (Equals, 0x0D),
        (Backslash, 0x0E),
        (Backspace, 0x0F),
        (Tab, 0x10),
        (Q, 0x11),
        (W, 0x12),
        (E, 0x13),
        (R, 0x14),
        (T, 0x15),
        (Y, 0x16),
        (U, 0x17),
        (I, 0x18),
        (O, 0x19),
        (P, 0x1A),
        (Grave, 0x1B),
        (LeftBracket, 0x1C),
        (Return, 0x1D),
        (A, 0x1E),
        (S, 0x1F),
        (D, 0x20),
        (F, 0x21),
        (G, 0x22),
        (H, 0x23),
        (J, 0x24),
        (K, 0x25),
        (L, 0x26),
        (Semicolon, 0x27),
        (Apostrophe, 0x28),
        (RightBracket, 0x29),
        (Z, 0x2A),
        (X, 0x2B),
        (C, 0x2C),
        (V, 0x2D),
        (B, 0x2E),
        (N, 0x2F),
        (M, 0x30),
        (Comma, 0x31),
        (Period, 0x32),
        (Slash, 0x33),
        (NonUsBackslash, 0x34),
        (Space, 0x35),
        (Home, 0x36),
        (Delete, 0x37),
        (PageUp, 0x38),
        (PageDown, 0x39),
        (End, 0x3A),
        (Left, 0x3B),
        (Up, 0x3C),
        (Right, 0x3D),
        (Down, 0x3E),
        (NumLock, 0x3F),
        (KpDivide, 0x40),
        (KpMultiply, 0x41),
        (KpMinus, 0x42),
        (Kp7, 0x43),
        (Kp8, 0x44),
        (Kp9, 0x45),
        (KpPlus, 0x46),
        (Kp4, 0x47),
        (Kp5, 0x48),
        (Kp6, 0x49),
        (Kp1, 0x4B),
        (Kp2, 0x4C),
        (Kp3, 0x4D),
        (KpEnter, 0x4E),
        (Kp0, 0x4F),
        (KpComma, 0x50),
        (KpPeriod, 0x51),
        (Application, 0x54),
        (F11, 0x55),
        (F12, 0x56),
        (F13, 0x57),
        (F14, 0x58),
        (F15, 0x59),
        (CapsLock, 0x5D),
        (Insert, 0x5E),
        (LAlt, 0x5F),
        (RAlt, 0x5A),
        (RCtrl, 0x60),
        (Pause, 0x61),
        (PrintScreen, 0x62),
        (F1, 0x63),
        (F2, 0x64),
        (F3, 0x65),
        (F4, 0x66),
        (F5, 0x67),
        (F6, 0x68),
        (F7, 0x69),
        (F8, 0x6A),
        (F9, 0x6B),
        (F10, 0x6C),
        (LShift, 0x70),
        (RShift, 0x70),
        (LCtrl, 0x71),
    ];
    let mut map = [0; Scancode::COUNT];
    let mut index = 0;
    while index < ALL_SCANCODES.len() {
        let (scancode, code) = ALL_SCANCODES[index];
        map[scancode.index()] = code;
        index += 1;
    }
    map
}

/// Maps a PC-88VA key name to its keycode (the value read at port 0x1C1). The VA
/// keycode interface reuses the PC-98 scan-code protocol, so this defers to the
/// PC-98 name table and adds the few VA-only keys.
pub fn pc88va_keycode_from_name(name: &str) -> Option<u8> {
    if let Some(code) = pc98_scancode_from_name(name) {
        return Some(code);
    }
    let name_lower = name.to_ascii_lowercase();
    Some(match name_lower.as_str() {
        "henkan" | "convert" => 0x35,
        "kettei" | "decide" => 0x51,
        "pc" => 0x7A,
        "zenkaku" => 0x7B,
        _ => return None,
    })
}

pub fn parse_key_binding_pc88va(host_name: &str, va_name: &str) -> Option<(Scancode, u8)> {
    let host = Scancode::from_name(host_name)?;
    let code = pc88va_keycode_from_name(va_name)?;
    Some((host, code))
}

/// Maps an FM Towns key name to its JIS scancode (the second byte of the
/// keyboard's serial packet).
pub fn towns_scancode_from_name(name: &str) -> Option<u8> {
    let name_lower = name.to_ascii_lowercase();
    Some(match name_lower.as_str() {
        "esc" => 0x01,
        "1" => 0x02,
        "2" => 0x03,
        "3" => 0x04,
        "4" => 0x05,
        "5" => 0x06,
        "6" => 0x07,
        "7" => 0x08,
        "8" => 0x09,
        "0" => 0x0B,
        "9" => 0x0A,
        "minus" => 0x0C,
        "caret" => 0x0D,
        "yen" => 0x0E,
        "bs" => 0x0F,
        "tab" => 0x10,
        "q" => 0x11,
        "w" => 0x12,
        "e" => 0x13,
        "r" => 0x14,
        "t" => 0x15,
        "y" => 0x16,
        "u" => 0x17,
        "i" => 0x18,
        "o" => 0x19,
        "p" => 0x1A,
        "at" => 0x1B,
        "leftbracket" => 0x1C,
        "return" => 0x1D,
        "a" => 0x1E,
        "s" => 0x1F,
        "d" => 0x20,
        "f" => 0x21,
        "g" => 0x22,
        "h" => 0x23,
        "j" => 0x24,
        "k" => 0x25,
        "l" => 0x26,
        "semicolon" => 0x27,
        "colon" => 0x28,
        "rightbracket" => 0x29,
        "z" => 0x2A,
        "x" => 0x2B,
        "c" => 0x2C,
        "v" => 0x2D,
        "b" => 0x2E,
        "n" => 0x2F,
        "m" => 0x30,
        "comma" => 0x31,
        "period" => 0x32,
        "slash" => 0x33,
        "underscore" => 0x34,
        "space" => 0x35,
        "kpmultiply" => 0x36,
        "kpdivide" => 0x37,
        "kpplus" => 0x38,
        "kpminus" => 0x39,
        "kp7" => 0x3A,
        "kp8" => 0x3B,
        "kp9" => 0x3C,
        "kp4" => 0x3E,
        "kp5" => 0x3F,
        "kp6" => 0x40,
        "kp1" => 0x42,
        "kp2" => 0x43,
        "kp3" => 0x44,
        "kpenter" => 0x45,
        "kp0" => 0x46,
        "kpperiod" => 0x47,
        "ins" => 0x48,
        "del" => 0x4B,
        "up" => 0x4D,
        "home" => 0x4E,
        "left" => 0x4F,
        "down" => 0x50,
        "right" => 0x51,
        "ctrl" => 0x52,
        "shift" => 0x53,
        "caps" => 0x55,
        "alt" => 0x5C,
        "f1" => 0x5D,
        "f2" => 0x5E,
        "f3" => 0x5F,
        "f4" => 0x60,
        "f5" => 0x61,
        "f6" => 0x62,
        "f7" => 0x63,
        "f8" => 0x64,
        "f9" => 0x65,
        "f10" => 0x66,
        "f11" => 0x69,
        "f12" => 0x5B,
        "cancel" => 0x72,
        "execute" => 0x73,
        "break" => 0x7C,
        "copy" => 0x7D,
        _ => return None,
    })
}

/// Parses an FM Towns `key.*` binding into a host scancode and JIS scancode.
pub fn parse_key_binding_towns(host_name: &str, towns_name: &str) -> Option<(Scancode, u8)> {
    let host = Scancode::from_name(host_name)?;
    let code = towns_scancode_from_name(towns_name)?;
    Some((host, code))
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
        // Matrix rows 13-14: PC-88VA-specific keys (no PC-8801 equivalent).
        "f6" => pc88_cell(13, 0),
        "f7" => pc88_cell(13, 1),
        "f8" => pc88_cell(13, 2),
        "f9" => pc88_cell(13, 3),
        "f10" => pc88_cell(13, 4),
        "henkan" | "convert" => pc88_cell(13, 5),
        "kettei" | "decide" => pc88_cell(13, 6),
        "pc" => pc88_cell(14, 5),
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
        // Matrix rows 13-14: PC-88VA keys absent from the PC-8801 keyboard. F8
        // enters the VA boot setup menu (port 0x0D bit 2); F11/F12 stand in for
        // the 変換 (next page) and 決定 (confirm) keys used to drive that menu.
        (F6, pc88_cell(13, 0)),
        (F7, pc88_cell(13, 1)),
        (F8, pc88_cell(13, 2)),
        (F9, pc88_cell(13, 3)),
        (F10, pc88_cell(13, 4)),
        (F11, pc88_cell(13, 5)),
        (F12, pc88_cell(13, 6)),
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

/// First function-key id on the PC-6001 wire encoding (F1). Function keys are
/// sent as ids 0x60-0x69 so the release bit (0x80) stays free for every key.
const PC60_FUNCTION_KEY_BASE: u8 = 0x60;

/// Maps a key name to a PC-6001 keycode for `key.*` config overrides. Normal
/// keys use their character code; function keys use the wire ids F1-F5.
/// A `0xNN` hex literal binds a raw keycode directly.
pub fn pc60_keycode_from_name(name: &str) -> Option<u8> {
    let lower = name.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        return u8::from_str_radix(hex, 16).ok();
    }
    if lower.len() == 1 {
        let character = lower.as_bytes()[0];
        if character.is_ascii_lowercase() {
            return Some(character - b'a' + b'A');
        }
        if character.is_ascii_digit() || (0x20..=0x5F).contains(&character) {
            return Some(character);
        }
    }
    Some(match lower.as_str() {
        "space" => 0x20,
        "return" | "enter" => 0x0D,
        "backspace" | "bs" => 0x08,
        "tab" => 0x09,
        "up" => 0x1E,
        "down" => 0x1F,
        "left" => 0x1D,
        "right" => 0x1C,
        "f1" => PC60_FUNCTION_KEY_BASE,
        "f2" => PC60_FUNCTION_KEY_BASE + 1,
        "f3" => PC60_FUNCTION_KEY_BASE + 2,
        "f4" => PC60_FUNCTION_KEY_BASE + 3,
        "f5" => PC60_FUNCTION_KEY_BASE + 4,
        _ => return None,
    })
}

pub fn parse_key_binding_pc60(host_name: &str, target_name: &str) -> Option<(Scancode, u8)> {
    let host = Scancode::from_name(host_name)?;
    let code = pc60_keycode_from_name(target_name)?;
    Some((host, code))
}

/// Default PC-6001 key map: host scancodes to firmware keycodes. Normal keys
/// carry their ASCII code; function keys F1-F5 carry the wire ids 0x60-0x64.
/// Unmapped host keys carry 0x00, which the sub-controller treats as no key.
const fn build_pc60_default_map() -> [u8; Scancode::COUNT] {
    use Scancode::*;

    /// No-key code for host keys without a PC-6001 equivalent.
    const UNMAPPED: u8 = 0x00;

    const ALL_SCANCODES: &[(Scancode, u8)] = &[
        // Letters carry their uppercase ASCII codes.
        (A, b'A'),
        (B, b'B'),
        (C, b'C'),
        (D, b'D'),
        (E, b'E'),
        (F, b'F'),
        (G, b'G'),
        (H, b'H'),
        (I, b'I'),
        (J, b'J'),
        (K, b'K'),
        (L, b'L'),
        (M, b'M'),
        (N, b'N'),
        (O, b'O'),
        (P, b'P'),
        (Q, b'Q'),
        (R, b'R'),
        (S, b'S'),
        (T, b'T'),
        (U, b'U'),
        (V, b'V'),
        (W, b'W'),
        (X, b'X'),
        (Y, b'Y'),
        (Z, b'Z'),
        // Digits.
        (_0, b'0'),
        (_1, b'1'),
        (_2, b'2'),
        (_3, b'3'),
        (_4, b'4'),
        (_5, b'5'),
        (_6, b'6'),
        (_7, b'7'),
        (_8, b'8'),
        (_9, b'9'),
        // Punctuation.
        (Space, b' '),
        (Minus, b'-'),
        (Comma, b','),
        (Period, b'.'),
        (Slash, b'/'),
        (Semicolon, b';'),
        (LeftBracket, b'['),
        (RightBracket, b']'),
        (Equals, b'^'),
        // Control keys and cursor cluster.
        (Return, 0x0D),
        (KpEnter, 0x0D),
        (Backspace, 0x08),
        (Delete, 0x08),
        (Tab, 0x09),
        (Right, 0x1C),
        (Left, 0x1D),
        (Up, 0x1E),
        (Down, 0x1F),
        // Function keys (wire ids).
        (F1, PC60_FUNCTION_KEY_BASE),
        (F2, PC60_FUNCTION_KEY_BASE + 1),
        (F3, PC60_FUNCTION_KEY_BASE + 2),
        (F4, PC60_FUNCTION_KEY_BASE + 3),
        (F5, PC60_FUNCTION_KEY_BASE + 4),
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

/// PC-6001 shifted key map: the keycode the sub-controller sends while Shift is
/// held. Keys without a shifted form keep their base code. Mirrors the resolved
/// codes the real keyboard scan produces.
const fn build_pc60_shifted_map() -> [u8; Scancode::COUNT] {
    use Scancode::*;

    const SHIFTED_SCANCODES: &[(Scancode, u8)] = &[
        // Letters shift to lowercase.
        (A, b'a'),
        (B, b'b'),
        (C, b'c'),
        (D, b'd'),
        (E, b'e'),
        (F, b'f'),
        (G, b'g'),
        (H, b'h'),
        (I, b'i'),
        (J, b'j'),
        (K, b'k'),
        (L, b'l'),
        (M, b'm'),
        (N, b'n'),
        (O, b'o'),
        (P, b'p'),
        (Q, b'q'),
        (R, b'r'),
        (S, b's'),
        (T, b't'),
        (U, b'u'),
        (V, b'v'),
        (W, b'w'),
        (X, b'x'),
        (Y, b'y'),
        (Z, b'z'),
        // Number row.
        (_1, b'!'),
        (_2, b'"'),
        (_3, b'#'),
        (_4, b'$'),
        (_5, b'%'),
        (_6, b'&'),
        (_7, b'\''),
        (_8, b'('),
        (_9, b')'),
        (_0, b'='),
        // Punctuation.
        (Comma, b';'),
        (Period, b':'),
        (Slash, b'?'),
        // Shifted function keys carry the upper wire ids (F6-F10).
        (F1, PC60_FUNCTION_KEY_BASE + 5),
        (F2, PC60_FUNCTION_KEY_BASE + 6),
        (F3, PC60_FUNCTION_KEY_BASE + 7),
        (F4, PC60_FUNCTION_KEY_BASE + 8),
        (F5, PC60_FUNCTION_KEY_BASE + 9),
    ];

    let mut map = build_pc60_default_map();
    let mut i = 0;
    while i < SHIFTED_SCANCODES.len() {
        let (scancode, code) = SHIFTED_SCANCODES[i];
        map[scancode.index()] = code;
        i += 1;
    }
    map
}

/// Default PC/AT key map for the 106-key JIS (OADG) layout: host scancodes to
/// set-1 key ids. Unmapped host keys carry 0x00, which the machine drops.
/// The E0-extended keys use the synthetic ids the machineat crate exports.
#[allow(clippy::just_underscores_and_digits)]
const AT_DEFAULT_BINDINGS: &[(Scancode, u8, &str)] = &[
    (Scancode::Escape, 0x01, "Esc"),
    (Scancode::_1, 0x02, "1"),
    (Scancode::_2, 0x03, "2"),
    (Scancode::_3, 0x04, "3"),
    (Scancode::_4, 0x05, "4"),
    (Scancode::_5, 0x06, "5"),
    (Scancode::_6, 0x07, "6"),
    (Scancode::_7, 0x08, "7"),
    (Scancode::_8, 0x09, "8"),
    (Scancode::_9, 0x0A, "9"),
    (Scancode::_0, 0x0B, "0"),
    (Scancode::Minus, 0x0C, "Minus"),
    (Scancode::Equals, 0x0D, "Caret"),
    (Scancode::Backspace, 0x0E, "Backspace"),
    (Scancode::Tab, 0x0F, "Tab"),
    (Scancode::Q, 0x10, "Q"),
    (Scancode::W, 0x11, "W"),
    (Scancode::E, 0x12, "E"),
    (Scancode::R, 0x13, "R"),
    (Scancode::T, 0x14, "T"),
    (Scancode::Y, 0x15, "Y"),
    (Scancode::U, 0x16, "U"),
    (Scancode::I, 0x17, "I"),
    (Scancode::O, 0x18, "O"),
    (Scancode::P, 0x19, "P"),
    (Scancode::LeftBracket, 0x1A, "LeftBracket"),
    (Scancode::RightBracket, 0x1B, "RightBracket"),
    (Scancode::Return, 0x1C, "Return"),
    (Scancode::LCtrl, 0x1D, "LCtrl"),
    (Scancode::A, 0x1E, "A"),
    (Scancode::S, 0x1F, "S"),
    (Scancode::D, 0x20, "D"),
    (Scancode::F, 0x21, "F"),
    (Scancode::G, 0x22, "G"),
    (Scancode::H, 0x23, "H"),
    (Scancode::J, 0x24, "J"),
    (Scancode::K, 0x25, "K"),
    (Scancode::L, 0x26, "L"),
    (Scancode::Semicolon, 0x27, "Semicolon"),
    (Scancode::Apostrophe, 0x28, "Colon"),
    (Scancode::Grave, 0x29, "Zenkaku"),
    (Scancode::LShift, 0x2A, "LShift"),
    (Scancode::Backslash, 0x2B, "Backslash"),
    (Scancode::Z, 0x2C, "Z"),
    (Scancode::X, 0x2D, "X"),
    (Scancode::C, 0x2E, "C"),
    (Scancode::V, 0x2F, "V"),
    (Scancode::B, 0x30, "B"),
    (Scancode::N, 0x31, "N"),
    (Scancode::M, 0x32, "M"),
    (Scancode::Comma, 0x33, "Comma"),
    (Scancode::Period, 0x34, "Period"),
    (Scancode::Slash, 0x35, "Slash"),
    (Scancode::RShift, 0x36, "RShift"),
    (Scancode::KpMultiply, 0x37, "KpMultiply"),
    (Scancode::LAlt, 0x38, "LAlt"),
    (Scancode::Space, 0x39, "Space"),
    (Scancode::CapsLock, 0x3A, "CapsLock"),
    (Scancode::F1, 0x3B, "F1"),
    (Scancode::F2, 0x3C, "F2"),
    (Scancode::F3, 0x3D, "F3"),
    (Scancode::F4, 0x3E, "F4"),
    (Scancode::F5, 0x3F, "F5"),
    (Scancode::F6, 0x40, "F6"),
    (Scancode::F7, 0x41, "F7"),
    (Scancode::F8, 0x42, "F8"),
    (Scancode::F9, 0x43, "F9"),
    (Scancode::F10, 0x44, "F10"),
    (Scancode::NumLock, 0x45, "NumLock"),
    (Scancode::Kp7, 0x47, "Kp7"),
    (Scancode::Kp8, 0x48, "Kp8"),
    (Scancode::Kp9, 0x49, "Kp9"),
    (Scancode::KpMinus, 0x4A, "KpMinus"),
    (Scancode::Kp4, 0x4B, "Kp4"),
    (Scancode::Kp5, 0x4C, "Kp5"),
    (Scancode::Kp6, 0x4D, "Kp6"),
    (Scancode::KpPlus, 0x4E, "KpPlus"),
    (Scancode::Kp1, 0x4F, "Kp1"),
    (Scancode::Kp2, 0x50, "Kp2"),
    (Scancode::Kp3, 0x51, "Kp3"),
    (Scancode::Kp0, 0x52, "Kp0"),
    (Scancode::KpPeriod, 0x53, "KpPeriod"),
    (Scancode::NonUsBackslash, 0x56, "NonUsBackslash"),
    (Scancode::F11, 0x57, "F11"),
    (Scancode::F12, 0x58, "F12"),
    (Scancode::International1, 0x73, "Ro"),
    (Scancode::International2, 0x70, "Kana"),
    (Scancode::International3, 0x7D, "Yen"),
    (Scancode::International4, 0x79, "Henkan"),
    (Scancode::International5, 0x7B, "Muhenkan"),
    (Scancode::Up, machineat::AT_KEY_CURSOR_UP, "Up"),
    (Scancode::Down, machineat::AT_KEY_CURSOR_DOWN, "Down"),
    (Scancode::Left, machineat::AT_KEY_CURSOR_LEFT, "Left"),
    (Scancode::Right, machineat::AT_KEY_CURSOR_RIGHT, "Right"),
    (Scancode::Insert, machineat::AT_KEY_INSERT, "Insert"),
    (Scancode::Delete, machineat::AT_KEY_DELETE, "Delete"),
    (Scancode::Home, machineat::AT_KEY_HOME, "Home"),
    (Scancode::End, machineat::AT_KEY_END, "End"),
    (Scancode::PageUp, machineat::AT_KEY_PAGE_UP, "PageUp"),
    (Scancode::PageDown, machineat::AT_KEY_PAGE_DOWN, "PageDown"),
    (Scancode::KpEnter, machineat::AT_KEY_KEYPAD_ENTER, "KpEnter"),
    (
        Scancode::KpDivide,
        machineat::AT_KEY_KEYPAD_DIVIDE,
        "KpDivide",
    ),
    (Scancode::RCtrl, machineat::AT_KEY_RIGHT_CTRL, "RCtrl"),
    (Scancode::RAlt, machineat::AT_KEY_RIGHT_ALT, "RAlt"),
];

#[allow(clippy::just_underscores_and_digits)]
const fn build_at_default_map() -> [u8; Scancode::COUNT] {
    let mut map = [0u8; Scancode::COUNT];
    let mut i = 0;
    while i < AT_DEFAULT_BINDINGS.len() {
        let (scancode, code, _) = AT_DEFAULT_BINDINGS[i];
        map[scancode.index()] = code;
        i += 1;
    }
    map
}

/// Maps a PC/AT key name to its set-1 key id for `key.*` config overrides.
/// A `0xNN` hex literal binds a raw id directly.
pub fn at_key_from_name(name: &str) -> Option<u8> {
    if let Some(hex) = name.strip_prefix("0x").or_else(|| name.strip_prefix("0X")) {
        return u8::from_str_radix(hex, 16).ok();
    }
    if let Some((_, code, _)) = AT_DEFAULT_BINDINGS
        .iter()
        .find(|(_, _, canonical_name)| canonical_name.eq_ignore_ascii_case(name))
    {
        return Some(*code);
    }
    match name.to_ascii_lowercase().as_str() {
        "escape" => Some(0x01),
        "equals" => Some(0x0D),
        "bs" => Some(0x0E),
        "enter" => Some(0x1C),
        "ctrl" => Some(0x1D),
        "apostrophe" => Some(0x28),
        "grave" | "hankaku" => Some(0x29),
        "shift" => Some(0x2A),
        "alt" => Some(0x38),
        "caps" => Some(0x3A),
        "katakana" | "hiragana" => Some(0x70),
        "underscore" => Some(0x73),
        "xfer" => Some(0x79),
        "nfer" => Some(0x7B),
        "ins" => Some(machineat::AT_KEY_INSERT),
        "del" => Some(machineat::AT_KEY_DELETE),
        _ => None,
    }
}

pub fn parse_key_binding_at(host_name: &str, at_name: &str) -> Option<(Scancode, u8)> {
    let host = Scancode::from_name(host_name)?;
    let code = at_key_from_name(at_name)?;
    Some((host, code))
}

#[cfg(test)]
mod tests {
    use sdl3::keyboard::Scancode;

    use super::{
        AT_DEFAULT_BINDINGS, KeyMap, KeyboardForwardingState, at_key_from_name,
        x68k_scancode_from_name,
    };

    #[test]
    fn at_canonical_names_match_every_default_binding() {
        let key_map = KeyMap::new_at();
        for &(host, code, canonical_name) in AT_DEFAULT_BINDINGS {
            assert_eq!(
                key_map.lookup(host),
                code,
                "default binding for {canonical_name}"
            );
            assert_eq!(
                at_key_from_name(canonical_name),
                Some(code),
                "canonical binding for {canonical_name}"
            );
        }
    }

    #[test]
    fn at_key_names_accept_aliases_and_raw_ids() {
        assert_eq!(at_key_from_name("escape"), Some(0x01));
        assert_eq!(at_key_from_name("a"), Some(0x1E));
        assert_eq!(at_key_from_name("ENTER"), Some(0x1C));
        assert_eq!(at_key_from_name("Equals"), Some(0x0D));
        assert_eq!(at_key_from_name("Apostrophe"), Some(0x28));
        assert_eq!(at_key_from_name("Hankaku"), Some(0x29));
        assert_eq!(at_key_from_name("Katakana"), Some(0x70));
        assert_eq!(at_key_from_name("0x00"), Some(0x00));
        assert_eq!(at_key_from_name("0X66"), Some(0x66));
        assert_eq!(at_key_from_name("0x100"), None);
        assert_eq!(at_key_from_name("not-a-key"), None);
    }

    #[test]
    fn pc88va_maps_host_keys_to_va_keycodes() {
        use super::pc88va_keycode_from_name;

        // The VA keycode interface returns PC-98-style scan codes; the map must
        // distinguish keys that the 88 matrix collapses (e.g. Backspace vs Delete).
        let map = KeyMap::new_pc88va();
        assert_eq!(map.lookup(Scancode::A), 0x1D);
        assert_eq!(map.lookup(Scancode::Return), 0x1C);
        assert_eq!(map.lookup(Scancode::Space), 0x34);
        assert_eq!(map.lookup(Scancode::Backspace), 0x0E);
        assert_eq!(map.lookup(Scancode::Delete), 0x39);
        assert_eq!(map.lookup(Scancode::KpEnter), 0x79);
        assert_eq!(map.lookup(Scancode::Kp0), 0x4E);
        assert_eq!(map.lookup(Scancode::F1), 0x62);
        assert_eq!(map.lookup(Scancode::LShift), 0x70);

        assert_eq!(pc88va_keycode_from_name("a"), Some(0x1D));
        assert_eq!(pc88va_keycode_from_name("return"), Some(0x1C));
        assert_eq!(pc88va_keycode_from_name("pc"), Some(0x7A));
        assert_eq!(pc88va_keycode_from_name("kettei"), Some(0x51));
    }

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
    fn x1_maps_the_numeric_keypad_to_tenkey_virtual_keys() {
        // The X1 tenkey occupies virtual keys 0x60-0x6F; games commonly steer
        // with tenkey 4/6, so the host numpad must reach the sub-CPU.
        let map = KeyMap::new_x1();
        assert_eq!(map.lookup(Scancode::Kp0), 0x60);
        assert_eq!(map.lookup(Scancode::Kp4), 0x64);
        assert_eq!(map.lookup(Scancode::Kp6), 0x66);
        assert_eq!(map.lookup(Scancode::Kp9), 0x69);
        assert_eq!(map.lookup(Scancode::KpMultiply), 0x6A);
        assert_eq!(map.lookup(Scancode::KpPlus), 0x6B);
        assert_eq!(map.lookup(Scancode::KpComma), 0x6C);
        assert_eq!(map.lookup(Scancode::KpMinus), 0x6D);
        assert_eq!(map.lookup(Scancode::KpPeriod), 0x6E);
        assert_eq!(map.lookup(Scancode::KpDivide), 0x6F);
        assert_eq!(map.lookup(Scancode::KpEnter), 0x0D);
    }

    #[test]
    fn normal_left_alt_is_forwarded_to_the_guest() {
        let mut keyboard_forwarding_state = KeyboardForwardingState::new();
        let key_map = KeyMap::new();

        keyboard_forwarding_state.handle_key_down(
            Some(Scancode::LAlt),
            false,
            false,
            false,
            false,
            &key_map,
        );
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            Some(0x73)
        );
        assert!(
            keyboard_forwarding_state
                .pending_released_pc98_scancodes()
                .is_empty()
        );

        let key_up_scancode = keyboard_forwarding_state.handle_key_up(Some(Scancode::LAlt), false);
        assert_eq!(key_up_scancode, Some(0xF3));
    }

    #[test]
    fn right_ctrl_combo_does_not_forward_left_alt_or_function_keys() {
        let mut keyboard_forwarding_state = KeyboardForwardingState::new();
        let key_map = KeyMap::new();

        keyboard_forwarding_state.handle_key_down(None, true, false, false, false, &key_map);
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            None
        );
        assert!(
            keyboard_forwarding_state
                .pending_released_pc98_scancodes()
                .is_empty()
        );

        keyboard_forwarding_state.handle_key_down(
            Some(Scancode::LAlt),
            true,
            false,
            false,
            false,
            &key_map,
        );
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            None
        );
        assert!(
            keyboard_forwarding_state
                .pending_released_pc98_scancodes()
                .is_empty()
        );

        keyboard_forwarding_state.handle_key_down(
            Some(Scancode::F9),
            true,
            false,
            false,
            false,
            &key_map,
        );
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
            keyboard_forwarding_state.handle_key_up(Some(Scancode::F9), false);
        assert_eq!(function_key_up_scancode, None);

        let left_alt_key_up_scancode =
            keyboard_forwarding_state.handle_key_up(Some(Scancode::LAlt), false);
        assert_eq!(left_alt_key_up_scancode, None);
    }

    #[test]
    fn right_ctrl_activation_releases_guest_keys_that_were_already_held() {
        let mut keyboard_forwarding_state = KeyboardForwardingState::new();
        let key_map = KeyMap::new();

        keyboard_forwarding_state.handle_key_down(
            Some(Scancode::LAlt),
            false,
            false,
            false,
            false,
            &key_map,
        );
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            Some(0x73)
        );

        keyboard_forwarding_state.handle_key_down(None, true, false, false, false, &key_map);
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            None
        );
        assert_eq!(
            keyboard_forwarding_state.pending_released_pc98_scancodes(),
            [0xF3]
        );

        let left_alt_key_up_scancode =
            keyboard_forwarding_state.handle_key_up(Some(Scancode::LAlt), false);
        assert_eq!(left_alt_key_up_scancode, None);
    }

    #[test]
    fn forwarding_recovers_after_right_ctrl_is_released() {
        let mut keyboard_forwarding_state = KeyboardForwardingState::new();
        let key_map = KeyMap::new();

        keyboard_forwarding_state.handle_key_down(None, true, false, false, false, &key_map);
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            None
        );
        assert!(
            keyboard_forwarding_state
                .pending_released_pc98_scancodes()
                .is_empty()
        );

        keyboard_forwarding_state.handle_key_down(
            Some(Scancode::LAlt),
            true,
            false,
            false,
            false,
            &key_map,
        );
        assert_eq!(
            keyboard_forwarding_state.pending_pressed_pc98_scancode(),
            None
        );
        assert!(
            keyboard_forwarding_state
                .pending_released_pc98_scancodes()
                .is_empty()
        );

        keyboard_forwarding_state.handle_key_down(
            Some(Scancode::A),
            false,
            false,
            false,
            false,
            &key_map,
        );
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

    #[test]
    fn pc60_shift_resolves_the_number_row_and_symbols() {
        use super::PC60_FUNCTION_KEY_BASE;

        let key_map = KeyMap::new_pc60();
        assert_eq!(key_map.resolve(Scancode::_2, true, false), b'"');
        assert_eq!(key_map.resolve(Scancode::_1, true, false), b'!');
        assert_eq!(key_map.resolve(Scancode::_7, true, false), b'\'');
        assert_eq!(key_map.resolve(Scancode::_0, true, false), b'=');
        assert_eq!(key_map.resolve(Scancode::Comma, true, false), b';');
        assert_eq!(key_map.resolve(Scancode::Period, true, false), b':');
        assert_eq!(key_map.resolve(Scancode::Slash, true, false), b'?');
        assert_eq!(
            key_map.resolve(Scancode::F1, true, false),
            PC60_FUNCTION_KEY_BASE + 5
        );
    }

    #[test]
    fn pc60_shift_lowercases_letters_and_ctrl_makes_control_codes() {
        let key_map = KeyMap::new_pc60();
        assert_eq!(key_map.resolve(Scancode::A, false, false), b'A');
        assert_eq!(key_map.resolve(Scancode::A, true, false), b'a');
        assert_eq!(key_map.resolve(Scancode::A, false, true), 0x01);
        assert_eq!(key_map.resolve(Scancode::C, false, true), 0x03);
        // Ctrl takes precedence over Shift.
        assert_eq!(key_map.resolve(Scancode::A, true, true), 0x01);
    }

    #[test]
    fn pc60_bare_modifiers_stay_no_key() {
        let key_map = KeyMap::new_pc60();
        assert_eq!(key_map.resolve(Scancode::LShift, true, false), 0x00);
        assert_eq!(key_map.resolve(Scancode::LCtrl, false, true), 0x00);
    }

    #[test]
    fn matrix_maps_ignore_modifier_state() {
        for key_map in [KeyMap::new(), KeyMap::new_pc88()] {
            assert_eq!(
                key_map.resolve(Scancode::_2, true, true),
                key_map.lookup(Scancode::_2)
            );
        }
    }

    #[test]
    fn x68000_map_uses_native_matrix_codes() {
        let key_map = KeyMap::new_x68k();
        assert_eq!(key_map.lookup(Scancode::Escape), 0x01);
        assert_eq!(key_map.lookup(Scancode::Left), 0x3B);
        assert_eq!(key_map.lookup(Scancode::Kp7), 0x43);
        assert_eq!(key_map.lookup(Scancode::F11), 0x55);
        assert_eq!(key_map.lookup(Scancode::F1), 0x63);
        assert_eq!(key_map.lookup(Scancode::LShift), 0x70);
        assert_eq!(x68k_scancode_from_name("fullwidth"), Some(0x60));
        assert_eq!(x68k_scancode_from_name("opt2"), Some(0x73));
    }
}
