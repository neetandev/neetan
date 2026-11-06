//! Logical input: key presses, text entry, joystick, and mouse control.

use common::{HostKey, JoystickState, KeyModifiers, RunOutcome, RunTarget};

use super::{AutomationSession, INPUT_DRAIN_INTERVAL_TICKS, OpError, TrackedControls};
use crate::input::{
    CharStroke, KeyboardLayout, MouseButton, apply_joystick_control, char_to_stroke,
    is_text_character,
};

impl AutomationSession {
    /// Presses a host key and records it as held.
    pub fn key_down(&mut self, key: HostKey) -> Result<(), OpError> {
        let active = self.active.as_mut().ok_or(OpError::NoMachine)?;
        let machine = &mut active.machine;
        if !machine.supports_host_key(key) {
            return Err(OpError::Unsupported(format!(
                "key {key:?} is not supported by this machine"
            )));
        }
        machine.push_host_key(key, true, KeyModifiers::default());
        active.tracked.keys_down.insert(key);
        Ok(())
    }

    /// Releases a host key.
    pub fn key_up(&mut self, key: HostKey) -> Result<(), OpError> {
        let active = self.active.as_mut().ok_or(OpError::NoMachine)?;
        let machine = &mut active.machine;
        if !machine.supports_host_key(key) {
            return Err(OpError::Unsupported(format!(
                "key {key:?} is not supported by this machine"
            )));
        }
        machine.push_host_key(key, false, KeyModifiers::default());
        active.tracked.keys_down.remove(&key);
        Ok(())
    }

    /// Holds a key across `frames` presentations, then always releases it.
    pub fn key_tap(
        &mut self,
        key: HostKey,
        frames: u64,
        max_ticks: u64,
    ) -> Result<RunOutcome, OpError> {
        self.key_down(key)?;
        let outcome = self
            .run(
                RunTarget::Frames(frames),
                max_ticks,
                INPUT_DRAIN_INTERVAL_TICKS,
            )
            .map_err(OpError::from_run);
        let _ = self.key_up(key);
        outcome
    }

    /// Types a run of text, validating every character before injecting any.
    ///
    /// Each character is typed with the key and shift state of the keyboard
    /// layout of the active machine. The shift state reaches a matrix machine
    /// as its Shift key and a pre-composing machine as the key modifiers.
    pub fn type_text(
        &mut self,
        text: &str,
        spacing_frames: u64,
        max_ticks_per_char: u64,
    ) -> Result<(), OpError> {
        let machine = &self.active.as_ref().ok_or(OpError::NoMachine)?.machine;
        let layout = KeyboardLayout::for_target(machine.automation_descriptor().target)
            .ok_or_else(|| OpError::Unsupported("this machine has no text layout".to_owned()))?;
        let has_shift_key = machine.supports_host_key(HostKey::LeftShift);
        let mut strokes = Vec::new();
        for character in text.chars() {
            if !is_text_character(character) {
                return Err(OpError::Argument(format!(
                    "character {character:?} is not in the supported set"
                )));
            }
            let stroke = char_to_stroke(layout, character)
                .filter(|stroke| {
                    machine
                        .translate_host_key(stroke.key, stroke_modifiers(*stroke))
                        .is_some()
                })
                .ok_or_else(|| {
                    OpError::Unsupported(format!(
                        "character {character:?} cannot be typed on the keyboard of this machine"
                    ))
                })?;
            strokes.push(stroke);
        }
        for stroke in strokes {
            let press_shift = stroke.shift && has_shift_key;
            if press_shift {
                self.key_down(HostKey::LeftShift)?;
            }
            self.push_stroke_key(stroke, true)?;
            let _ = self.run(
                RunTarget::Frames(spacing_frames),
                max_ticks_per_char,
                INPUT_DRAIN_INTERVAL_TICKS,
            );
            self.push_stroke_key(stroke, false)?;
            if press_shift {
                self.key_up(HostKey::LeftShift)?;
            }
        }
        Ok(())
    }

    /// Presses or releases the key of a text stroke with the stroke modifiers
    /// and tracks it as held while pressed.
    fn push_stroke_key(&mut self, stroke: CharStroke, pressed: bool) -> Result<(), OpError> {
        let active = self.active.as_mut().ok_or(OpError::NoMachine)?;
        active
            .machine
            .push_host_key(stroke.key, pressed, stroke_modifiers(stroke));
        if pressed {
            active.tracked.keys_down.insert(stroke.key);
        } else {
            active.tracked.keys_down.remove(&stroke.key);
        }
        Ok(())
    }

    /// Sets a joystick control on the port at `index`.
    pub fn joystick_set(
        &mut self,
        index: usize,
        control: &str,
        pressed: bool,
    ) -> Result<(), OpError> {
        if self.active.is_none() {
            return Err(OpError::NoMachine);
        }
        let ports = self
            .input_capabilities()
            .map_or(0, |input| input.joystick_ports);
        if index >= ports as usize {
            return Err(OpError::Unsupported(format!(
                "joystick port {index} is not available"
            )));
        }
        let active = self.active.as_mut().expect("machine present");
        let state = active.tracked.joysticks.entry(index).or_default();
        if !apply_joystick_control(state, control, pressed) {
            return Err(OpError::Unsupported(format!(
                "unknown joystick control {control:?}"
            )));
        }
        let state = *state;
        active.machine.set_joystick(index, state);
        Ok(())
    }

    /// Clears every control on the joystick port at `index`.
    pub fn joystick_clear(&mut self, index: usize) -> Result<(), OpError> {
        if self.active.is_none() {
            return Err(OpError::NoMachine);
        }
        let ports = self
            .input_capabilities()
            .map_or(0, |input| input.joystick_ports);
        if index >= ports as usize {
            return Err(OpError::Unsupported(format!(
                "joystick port {index} is not available"
            )));
        }
        let active = self.active.as_mut().expect("machine present");
        active
            .tracked
            .joysticks
            .insert(index, JoystickState::default());
        active.machine.set_joystick(index, JoystickState::default());
        Ok(())
    }

    /// Accumulates a relative mouse movement.
    pub fn mouse_move(&mut self, delta_x: i128, delta_y: i128) -> Result<(), OpError> {
        if self.active.is_none() {
            return Err(OpError::NoMachine);
        }
        if self
            .input_capabilities()
            .map_or(0, |input| input.mouse_buttons)
            == 0
        {
            return Err(OpError::Unsupported("this machine has no mouse".to_owned()));
        }
        let delta_x = i16::try_from(delta_x).map_err(|_| OpError::Range)?;
        let delta_y = i16::try_from(delta_y).map_err(|_| OpError::Range)?;
        self.active
            .as_mut()
            .expect("machine present")
            .machine
            .push_mouse_delta(delta_x, delta_y);
        Ok(())
    }

    /// Sets a mouse button state.
    pub fn mouse_button(&mut self, button: MouseButton, pressed: bool) -> Result<(), OpError> {
        if self.active.is_none() {
            return Err(OpError::NoMachine);
        }
        if self
            .input_capabilities()
            .map_or(0, |input| input.mouse_buttons)
            == 0
        {
            return Err(OpError::Unsupported("this machine has no mouse".to_owned()));
        }
        let active = self.active.as_mut().expect("machine present");
        match button {
            MouseButton::Left => active.tracked.mouse_buttons.0 = pressed,
            MouseButton::Right => active.tracked.mouse_buttons.1 = pressed,
            MouseButton::Middle => active.tracked.mouse_buttons.2 = pressed,
        }
        let (left, right, middle) = active.tracked.mouse_buttons;
        active.machine.set_mouse_buttons(left, right, middle);
        Ok(())
    }

    /// Releases every tracked key, joystick, and mouse button.
    pub fn release_all_controls(&mut self) {
        if let Some(active) = self.active.as_mut() {
            for key in active.tracked.keys_down.iter().rev() {
                let machine = &mut active.machine;
                machine.push_host_key(*key, false, KeyModifiers::default());
            }
            for index in active.tracked.joysticks.keys() {
                active
                    .machine
                    .set_joystick(*index, JoystickState::default());
            }
            active.machine.set_mouse_buttons(false, false, false);
            active.tracked = TrackedControls::default();
        }
    }
}

/// The key modifiers that carry the shift state of a text stroke.
fn stroke_modifiers(stroke: CharStroke) -> KeyModifiers {
    KeyModifiers {
        shift: stroke.shift,
        ..KeyModifiers::default()
    }
}
