//! Logical input: key presses, text entry, joystick, and mouse control.

use common::{HostKey, JoystickState, KeyModifiers, RunOutcome, RunTarget};

use super::{AutomationSession, INPUT_DRAIN_INTERVAL_TICKS, OpError, TrackedControls};
use crate::input::{MouseButton, apply_joystick_control, char_to_stroke};

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
    pub fn type_text(
        &mut self,
        text: &str,
        spacing_frames: u64,
        max_ticks_per_char: u64,
    ) -> Result<(), OpError> {
        let machine = &self.active.as_ref().ok_or(OpError::NoMachine)?.machine;
        let mut strokes = Vec::new();
        for character in text.chars() {
            let stroke = char_to_stroke(character).ok_or_else(|| {
                OpError::Argument(format!(
                    "character {character:?} is not in the supported set"
                ))
            })?;
            if stroke.shift && !machine.supports_host_key(HostKey::LeftShift) {
                return Err(OpError::Unsupported(
                    "this machine cannot produce shifted characters".to_owned(),
                ));
            }
            if !machine.supports_host_key(stroke.key) {
                return Err(OpError::Unsupported(format!(
                    "character {character:?} is not supported by this machine"
                )));
            }
            strokes.push(stroke);
        }
        for stroke in strokes {
            if stroke.shift {
                self.key_down(HostKey::LeftShift)?;
            }
            self.key_down(stroke.key)?;
            let _ = self.run(
                RunTarget::Frames(spacing_frames),
                max_ticks_per_char,
                INPUT_DRAIN_INTERVAL_TICKS,
            );
            self.key_up(stroke.key)?;
            if stroke.shift {
                self.key_up(HostKey::LeftShift)?;
            }
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
