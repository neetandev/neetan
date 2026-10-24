//! The keyboard, mouse and gamepad events of a session and its hotkeys.

use std::time::Instant;

use common::KeyModifiers;
use sdl3::{
    event::Event,
    gamepad::{GamepadAxis, GamepadButton},
    keyboard::{Mod, Scancode},
    mouse::MouseButton,
};

use super::{Session, SessionExit};
use crate::{
    gui_host::{key_event, pointer_or_text_event},
    host::Host,
    input::{JoystickKey, host_key_from_scancode},
    media_dialog::Drive,
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
                if self.media_dialog.is_some() =>
            {
                if let Some(gui_event) = pointer_or_text_event(event) {
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

    /// Handles a key press: the dialog keys while the media dialog is open,
    /// otherwise the machine keyboard, the keyboard joystick and the hotkeys.
    fn handle_key_down(
        &mut self,
        scancode: Option<Scancode>,
        keymod: &Mod,
        repeat: bool,
        host: &mut Host,
    ) {
        if self.media_dialog.is_some() {
            self.handle_dialog_key(scancode, keymod, repeat, host);
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
            if scancode == Some(Scancode::F) {
                self.set_fast_forward(true, host);
            }
            return;
        }
        match scancode {
            Some(Scancode::M) => self.toggle_mouse_capture(host),
            Some(Scancode::Q) => self.request_exit(SessionExit::Quit),
            Some(Scancode::Return) => host.set_fullscreen(!host.fullscreen),
            Some(Scancode::C) => self.toggle_crt(host),
            Some(Scancode::S) => self.toggle_scaling(host),
            Some(Scancode::P) => self.cycle_composite_phase(),
            Some(Scancode::_5) => self.quick_save(),
            Some(Scancode::_9) => self.quick_load(host),
            Some(Scancode::R) => self.hard_reset(host),
            Some(Scancode::F) => self.set_fast_forward(true, host),
            _ => {
                if let Some(drive) = media_hotkey_drive(scancode) {
                    self.media_hotkey(drive, host);
                }
            }
        }
    }

    /// Handles a key release: stops fast forward and releases the key on the
    /// machine keyboard and the keyboard joystick.
    fn handle_key_up(&mut self, scancode: Option<Scancode>, repeat: bool, host: &mut Host) {
        if self.fast_forward && matches!(scancode, Some(Scancode::F) | Some(Scancode::RCtrl)) {
            self.set_fast_forward(false, host);
        }
        if self.media_dialog.is_some() {
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

    /// Handles a key press while the media dialog is open: the media hotkeys
    /// and the keys of the dialog.
    fn handle_dialog_key(
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
            if !repeat && let Some(drive) = media_hotkey_drive(Some(scancode)) {
                self.media_hotkey(drive, host);
            }
            return;
        }
        if let Some(gui_event) = key_event(scancode, keymod, repeat) {
            self.handle_gui_event(gui_event, host);
        }
    }

    /// Records a gamepad button change and updates the joystick port. While
    /// the media dialog is open, the button goes to the dialog.
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
        if self.media_dialog.is_some() {
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
    /// the media dialog is open, the stick goes to the dialog.
    fn handle_gamepad_axis(&mut self, which: u32, axis: GamepadAxis, value: i16, host: &mut Host) {
        if !host.is_active_gamepad(which) {
            return;
        }
        if self.media_dialog.is_some() {
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

/// The drive of the media hotkey on `scancode`: right Ctrl with 1, 2 or 3.
fn media_hotkey_drive(scancode: Option<Scancode>) -> Option<Drive> {
    match scancode {
        Some(Scancode::_1) => Some(Drive::Floppy(0)),
        Some(Scancode::_2) => Some(Drive::Floppy(1)),
        Some(Scancode::_3) => Some(Drive::CdRom),
        _ => None,
    }
}
