//! The keyboard, mouse and gamepad events of a session and its hotkeys.

use std::time::Instant;

use common::{KeyModifiers, info};
use sdl3::{
    event::Event,
    gamepad::{GamepadAxis, GamepadButton},
    keyboard::{Mod, Scancode},
    mouse::MouseButton,
};

use super::{Session, SessionExit};
use crate::{
    config::Backend,
    gui_host::keys::pointer_or_text_event,
    host::Host,
    hotkeys::{Hotkey, HotkeyFeatures},
    input::{JoystickKey, host_key_from_scancode},
};

impl Session {
    /// Handles one keyboard, text, mouse or gamepad event.
    pub(crate) fn handle_event(&mut self, event: &Event, host: &mut Host) {
        match event {
            Event::KeyDown {
                scancode,
                keymod,
                repeat,
                ..
            } => self.handle_key_down(*scancode, keymod, *repeat, host),
            Event::KeyUp {
                scancode, repeat, ..
            } => self.handle_key_up(*scancode, *repeat, host),
            Event::TextInput { .. } if host.right_control_held => {}
            Event::TextInput { .. }
            | Event::TextEditing { .. }
            | Event::MouseMotion { .. }
            | Event::MouseButtonDown { .. }
            | Event::MouseButtonUp { .. }
            | Event::MouseWheel { .. }
                if self.overlay.is_some() =>
            {
                if let Some(gui_event) = pointer_or_text_event(event, host.shift_held) {
                    self.handle_gui_event(gui_event, host);
                }
            }
            Event::MouseMotion { xrel, yrel, .. } if self.mouse_captured => {
                self.mouse_dx += xrel;
                self.mouse_dy += yrel;
            }
            Event::MouseButtonDown { mouse_btn, .. } if self.mouse_captured => {
                self.set_mouse_button(*mouse_btn, true);
            }
            Event::MouseButtonUp { mouse_btn, .. } if self.mouse_captured => {
                self.set_mouse_button(*mouse_btn, false);
            }
            Event::GamepadButtonDown { which, button } => {
                self.handle_gamepad_button(*which, *button, true, host);
            }
            Event::GamepadButtonUp { which, button } => {
                self.handle_gamepad_button(*which, *button, false, host);
            }
            Event::GamepadAxisMotion { which, axis, value } => {
                self.handle_gamepad_axis(*which, *axis, *value, host);
            }
            _ => {}
        }
    }

    /// Handles a key press: the overlay keys while an overlay is open,
    /// otherwise the machine keyboard, the keyboard joystick and the hotkeys.
    fn handle_key_down(
        &mut self,
        scancode: Option<Scancode>,
        keymod: &Mod,
        repeat: bool,
        host: &mut Host,
    ) {
        if self.overlay.is_some() {
            self.handle_overlay_key(scancode, keymod, repeat, host);
            return;
        }
        let modifiers = KeyModifiers {
            shift: keymod.shift(),
            ctrl: keymod.ctrl(),
        };
        let host_key = scancode.and_then(host_key_from_scancode);
        let native_override = scancode.and_then(|scancode| self.key_overrides.get(scancode));
        self.keyboard_forwarding_state.handle_key_down(
            scancode,
            keymod.rctrl(),
            host_key,
            modifiers,
            native_override,
            repeat,
        );
        self.keyboard_forwarding_state
            .apply_pending_actions(self.machine.as_mut());

        if let Some(key) = JoystickKey::from_scancode(scancode) {
            key.apply(&mut self.keyboard_joystick, true);
            self.update_joystick(host);
        }

        if !keymod.rctrl() {
            return;
        }
        if repeat {
            if scancode == Some(Hotkey::FastForward.scancode()) {
                self.set_fast_forward(true, host);
            }
            return;
        }
        if let Some(hotkey) = scancode.and_then(Hotkey::from_scancode) {
            self.run_hotkey(hotkey, host);
        }
    }

    /// Runs `hotkey` when it works on the machine. While an overlay is open,
    /// only the hotkeys that work over an overlay run. A media hotkey always
    /// reaches the media dialog, which reports a missing drive itself.
    fn run_hotkey(&mut self, hotkey: Hotkey, host: &mut Host) {
        if self.overlay.is_some() && !hotkey.works_over_overlay() {
            return;
        }
        let media = matches!(hotkey, Hotkey::Media(_));
        if !media && !hotkey.available(&self.hotkey_features(host)) {
            info!("The hotkey {hotkey:?} does nothing on this machine");
            return;
        }
        match hotkey {
            Hotkey::Help => self.help_hotkey(host),
            Hotkey::Exit => self.request_exit(SessionExit::UserExit),
            Hotkey::Fullscreen => host.set_fullscreen(!host.fullscreen),
            Hotkey::MouseCapture => self.toggle_mouse_capture(host),
            Hotkey::HardReset => self.hard_reset(host),
            Hotkey::FastForward => self.set_fast_forward(true, host),
            Hotkey::Crt => self.toggle_crt(),
            Hotkey::Scaling => self.toggle_scaling(host),
            Hotkey::CompositePhase => self.cycle_composite_phase(),
            Hotkey::Media(drive) => self.media_hotkey(drive, host),
            Hotkey::QuickSave => self.quick_save(),
            Hotkey::QuickLoad => self.quick_load(host),
        }
    }

    /// The features of the machine and the backend that decide which
    /// hotkeys work.
    pub(super) fn hotkey_features(&self, host: &Host) -> HotkeyFeatures {
        HotkeyFeatures {
            drives: self.media_drives(),
            mouse: self.machine.startup_capabilities().mouse,
            crt_effect: host.backend == Backend::Modern,
            composite_video: self.config.target.has_composite_video(),
        }
    }

    /// Handles a key release: stops fast forward and releases the key on the
    /// machine keyboard and the keyboard joystick.
    fn handle_key_up(&mut self, scancode: Option<Scancode>, repeat: bool, host: &mut Host) {
        let fast_forward_released =
            scancode == Some(Hotkey::FastForward.scancode()) || scancode == Some(Scancode::RCtrl);
        if self.fast_forward && fast_forward_released {
            self.set_fast_forward(false, host);
        }
        if self.overlay.is_some() {
            return;
        }
        self.keyboard_forwarding_state
            .handle_key_up(scancode, repeat);
        self.keyboard_forwarding_state
            .apply_pending_actions(self.machine.as_mut());
        if let Some(key) = JoystickKey::from_scancode(scancode) {
            key.apply(&mut self.keyboard_joystick, false);
            self.update_joystick(host);
        }
    }

    /// Records a mouse button change of the captured mouse and passes the
    /// buttons to the machine.
    fn set_mouse_button(&mut self, button: MouseButton, pressed: bool) {
        match button {
            MouseButton::Left => self.mouse_left = pressed,
            MouseButton::Right => self.mouse_right = pressed,
            MouseButton::Middle => self.mouse_middle = pressed,
            MouseButton::X1 | MouseButton::X2 | MouseButton::Unknown => {}
        }
        self.machine
            .set_mouse_buttons(self.mouse_left, self.mouse_right, self.mouse_middle);
    }

    /// Handles a key press while an overlay is open: the hotkeys that work
    /// over an overlay and the keys of the open overlay.
    fn handle_overlay_key(
        &mut self,
        scancode: Option<Scancode>,
        keymod: &Mod,
        repeat: bool,
        host: &mut Host,
    ) {
        let Some(scancode) = scancode else {
            return;
        };
        if keymod.rctrl() {
            if !repeat && let Some(hotkey) = Hotkey::from_scancode(scancode) {
                self.run_hotkey(hotkey, host);
            }
            return;
        }
        if let Some(gui_event) = host
            .gui
            .as_ref()
            .and_then(|gui| gui.key_event(scancode, keymod, repeat))
        {
            self.handle_gui_event(gui_event, host);
        }
    }

    /// Records a gamepad button change and updates the joystick port. While
    /// an overlay is open, the button goes to the overlay.
    fn handle_gamepad_button(
        &mut self,
        which: u32,
        button: GamepadButton,
        pressed: bool,
        host: &mut Host,
    ) {
        if !host.is_active_gamepad(which) {
            return;
        }
        if self.overlay.is_some() {
            let gui_event = host
                .gui
                .as_mut()
                .and_then(|gui| gui.gamepad.button(button, pressed, Instant::now()));
            if let Some(gui_event) = gui_event {
                self.handle_gui_event(gui_event, host);
            }
            return;
        }
        match button {
            GamepadButton::DpadUp => self.gamepad_buttons.up = pressed,
            GamepadButton::DpadDown => self.gamepad_buttons.down = pressed,
            GamepadButton::DpadLeft => self.gamepad_buttons.left = pressed,
            GamepadButton::DpadRight => self.gamepad_buttons.right = pressed,
            // Face and shoulder buttons cover the 6-button pad (A/B/C/X/Y/Z);
            // machines with only two triggers use just A and B.
            GamepadButton::South => self.gamepad_buttons.trigger1 = pressed,
            GamepadButton::East => self.gamepad_buttons.trigger2 = pressed,
            GamepadButton::West => self.gamepad_buttons.button_c = pressed,
            GamepadButton::North => self.gamepad_buttons.button_x = pressed,
            GamepadButton::LeftShoulder => self.gamepad_buttons.button_y = pressed,
            GamepadButton::RightShoulder => self.gamepad_buttons.button_z = pressed,
            GamepadButton::Start => self.gamepad_buttons.run = pressed,
            GamepadButton::Back => self.gamepad_buttons.select = pressed,
            _ => return,
        }
        self.update_joystick(host);
    }

    /// Records a left-stick axis change and updates the joystick port. While
    /// an overlay is open, the stick goes to the overlay.
    fn handle_gamepad_axis(&mut self, which: u32, axis: GamepadAxis, value: i16, host: &mut Host) {
        if !host.is_active_gamepad(which) {
            return;
        }
        if self.overlay.is_some() {
            let gui_event = host
                .gui
                .as_mut()
                .and_then(|gui| gui.gamepad.axis(axis, value, Instant::now()));
            if let Some(gui_event) = gui_event {
                self.handle_gui_event(gui_event, host);
            }
            return;
        }
        match axis {
            GamepadAxis::LeftX => self.gamepad_axis_x = value,
            GamepadAxis::LeftY => self.gamepad_axis_y = value,
            _ => return,
        }
        self.update_joystick(host);
    }
}
