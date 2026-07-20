use common::{HostKey, JoystickState, KeyModifiers, Machine};
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

/// Maps an SDL scancode to its neutral host key, independent of machine family.
///
/// This is the single host-side identity table for the interactive frontend. It
/// is a neutral one-to-one mapping that names the physical key and encodes no
/// machine-specific behavior. Each machine crate translates the resulting
/// [`HostKey`] into its own native code.
#[allow(clippy::just_underscores_and_digits)]
pub(crate) fn host_key_from_scancode(scancode: Scancode) -> Option<HostKey> {
    use Scancode::*;
    Some(match scancode {
        A => HostKey::A,
        B => HostKey::B,
        C => HostKey::C,
        D => HostKey::D,
        E => HostKey::E,
        F => HostKey::F,
        G => HostKey::G,
        H => HostKey::H,
        I => HostKey::I,
        J => HostKey::J,
        K => HostKey::K,
        L => HostKey::L,
        M => HostKey::M,
        N => HostKey::N,
        O => HostKey::O,
        P => HostKey::P,
        Q => HostKey::Q,
        R => HostKey::R,
        S => HostKey::S,
        T => HostKey::T,
        U => HostKey::U,
        V => HostKey::V,
        W => HostKey::W,
        X => HostKey::X,
        Y => HostKey::Y,
        Z => HostKey::Z,
        _0 => HostKey::Digit0,
        _1 => HostKey::Digit1,
        _2 => HostKey::Digit2,
        _3 => HostKey::Digit3,
        _4 => HostKey::Digit4,
        _5 => HostKey::Digit5,
        _6 => HostKey::Digit6,
        _7 => HostKey::Digit7,
        _8 => HostKey::Digit8,
        _9 => HostKey::Digit9,
        Space => HostKey::Space,
        Minus => HostKey::Minus,
        Equals => HostKey::Equals,
        Backslash => HostKey::Backslash,
        Backspace => HostKey::Backspace,
        Tab => HostKey::Tab,
        Grave => HostKey::Grave,
        LeftBracket => HostKey::LeftBracket,
        RightBracket => HostKey::RightBracket,
        Return => HostKey::Return,
        Semicolon => HostKey::Semicolon,
        Apostrophe => HostKey::Apostrophe,
        Comma => HostKey::Comma,
        Period => HostKey::Period,
        Slash => HostKey::Slash,
        NonUsBackslash => HostKey::NonUsBackslash,
        International1 => HostKey::International1,
        International2 => HostKey::International2,
        International3 => HostKey::International3,
        International4 => HostKey::International4,
        International5 => HostKey::International5,
        Escape => HostKey::Escape,
        Insert => HostKey::Insert,
        Delete => HostKey::Delete,
        Home => HostKey::Home,
        End => HostKey::End,
        PageUp => HostKey::PageUp,
        PageDown => HostKey::PageDown,
        Up => HostKey::Up,
        Down => HostKey::Down,
        Left => HostKey::Left,
        Right => HostKey::Right,
        Kp0 => HostKey::Kp0,
        Kp1 => HostKey::Kp1,
        Kp2 => HostKey::Kp2,
        Kp3 => HostKey::Kp3,
        Kp4 => HostKey::Kp4,
        Kp5 => HostKey::Kp5,
        Kp6 => HostKey::Kp6,
        Kp7 => HostKey::Kp7,
        Kp8 => HostKey::Kp8,
        Kp9 => HostKey::Kp9,
        KpMinus => HostKey::KpMinus,
        KpDivide => HostKey::KpDivide,
        KpMultiply => HostKey::KpMultiply,
        KpPlus => HostKey::KpPlus,
        KpEnter => HostKey::KpEnter,
        KpComma => HostKey::KpComma,
        KpPeriod => HostKey::KpPeriod,
        F1 => HostKey::F1,
        F2 => HostKey::F2,
        F3 => HostKey::F3,
        F4 => HostKey::F4,
        F5 => HostKey::F5,
        F6 => HostKey::F6,
        F7 => HostKey::F7,
        F8 => HostKey::F8,
        F9 => HostKey::F9,
        F10 => HostKey::F10,
        F11 => HostKey::F11,
        F12 => HostKey::F12,
        F13 => HostKey::F13,
        F14 => HostKey::F14,
        F15 => HostKey::F15,
        Pause => HostKey::Pause,
        PrintScreen => HostKey::PrintScreen,
        Application => HostKey::Application,
        LShift => HostKey::LeftShift,
        RShift => HostKey::RightShift,
        CapsLock => HostKey::CapsLock,
        NumLock => HostKey::NumLock,
        LAlt => HostKey::LeftAlt,
        RAlt => HostKey::RightAlt,
        LCtrl => HostKey::LeftControl,
        RCtrl => HostKey::RightControl,
    })
}

/// What was injected for a held physical key, replayed verbatim on release.
///
/// Latching the translation inputs (not the resolved native code) keeps the
/// "press-time code is authoritative" guarantee: on release the same host key
/// and press-time modifiers re-resolve to the identical native code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HeldKey {
    /// Resolved through the machine from this host key and press-time modifiers.
    Host {
        /// The neutral host key.
        key: HostKey,
        /// The modifier state captured at press time.
        modifiers: KeyModifiers,
    },
    /// A raw native override code from a `key.*` config binding.
    Native(u8),
}

/// Per-scancode native key-code overrides from `key.*` config bindings.
///
/// An override injects a raw native code for a physical key, bypassing the
/// neutral host-key path. It is populated per target from the config file.
#[derive(Clone, Copy)]
pub struct KeyOverrides {
    codes: [Option<u8>; Scancode::COUNT],
}

impl Default for KeyOverrides {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyOverrides {
    /// Returns an empty override table.
    pub const fn new() -> Self {
        Self {
            codes: [None; Scancode::COUNT],
        }
    }

    /// Sets the native override code for a host scancode.
    pub fn set(&mut self, host: Scancode, code: u8) {
        self.codes[host.index()] = Some(code);
    }

    /// Returns the native override code for a host scancode, when set.
    pub(crate) fn get(&self, host: Scancode) -> Option<u8> {
        self.codes[host.index()]
    }
}

/// Dispatches a held key press or release to the machine.
fn dispatch_held_key(machine: &mut dyn Machine, held: HeldKey, pressed: bool) {
    match held {
        HeldKey::Host { key, modifiers } => machine.push_host_key(key, pressed, modifiers),
        HeldKey::Native(code) => {
            machine.push_keyboard_scancode(if pressed { code } else { code | 0x80 });
        }
    }
}

pub(crate) struct KeyboardForwardingState {
    shortcut_modifier_active: bool,
    held_keys: [Option<HeldKey>; Scancode::COUNT],
    pending_press: Option<HeldKey>,
    pending_releases: Vec<HeldKey>,
}

impl KeyboardForwardingState {
    pub(crate) fn new() -> Self {
        Self {
            shortcut_modifier_active: false,
            held_keys: [None; Scancode::COUNT],
            pending_press: None,
            pending_releases: Vec::with_capacity(Scancode::COUNT),
        }
    }

    pub(crate) fn handle_key_down(
        &mut self,
        scancode: Option<Scancode>,
        shortcut_modifier_active: bool,
        host_key: Option<HostKey>,
        modifiers: KeyModifiers,
        native_override: Option<u8>,
        repeat: bool,
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
        if self.held_keys[scancode_index].is_some() {
            return;
        }

        let held = match native_override {
            Some(code) => HeldKey::Native(code),
            None => match host_key {
                Some(key) => HeldKey::Host { key, modifiers },
                None => return,
            },
        };
        self.held_keys[scancode_index] = Some(held);
        self.pending_press = Some(held);
    }

    pub(crate) fn handle_key_up(&mut self, scancode: Option<Scancode>, repeat: bool) {
        self.clear_pending_actions();

        if repeat {
            return;
        }

        let Some(scancode) = scancode else {
            return;
        };
        let scancode_index = scancode.index();
        if let Some(held) = self.held_keys[scancode_index].take() {
            self.pending_releases.push(held);
        }
    }

    fn release_all_guest_keys(&mut self) {
        for held_key in &mut self.held_keys {
            if let Some(held) = held_key.take() {
                self.pending_releases.push(held);
            }
        }
    }

    pub(crate) fn apply_pending_actions(&mut self, machine: &mut dyn Machine) {
        for &released in &self.pending_releases {
            dispatch_held_key(machine, released, false);
        }

        if let Some(pressed) = self.pending_press {
            dispatch_held_key(machine, pressed, true);
        }

        self.clear_pending_actions();
    }

    fn clear_pending_actions(&mut self) {
        self.pending_press = None;
        self.pending_releases.clear();
    }
}
