//! Sharp X68000 keyboard protocol endpoint.

use std::collections::VecDeque;

/// Keyboard protocol clock frequency used by deadlines.
pub const KEYBOARD_X68K_TICKS_PER_SECOND: u64 = 1_000_000;

/// Power-on delay before a held key begins repeating.
const DEFAULT_REPEAT_DELAY_TICKS: u64 = 500_000;
/// Power-on interval between repeated make codes.
const DEFAULT_REPEAT_INTERVAL_TICKS: u64 = 110_000;

/// Sharp X68000 keyboard endpoint.
#[derive(Debug, Clone)]
pub struct KeyboardX68k {
    pressed: [bool; 128],
    output: VecDeque<u8>,
    system_enabled: bool,
    command_enabled: bool,
    repeat_code: Option<u8>,
    repeat_deadline: Option<u64>,
    repeat_delay_ticks: u64,
    repeat_interval_ticks: u64,
    leds: u8,
    led_brightness: u8,
    mouse_control: bool,
    display_control: u8,
    x68000_display_mode: bool,
    main_display_control_enabled: bool,
    option2_display_control_enabled: bool,
    current_tick: u64,
}

impl Default for KeyboardX68k {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyboardX68k {
    /// Creates a reset keyboard.
    pub fn new() -> Self {
        Self {
            pressed: [false; 128],
            output: VecDeque::new(),
            system_enabled: false,
            command_enabled: false,
            repeat_code: None,
            repeat_deadline: None,
            repeat_delay_ticks: DEFAULT_REPEAT_DELAY_TICKS,
            repeat_interval_ticks: DEFAULT_REPEAT_INTERVAL_TICKS,
            leds: 0,
            led_brightness: 0,
            mouse_control: false,
            display_control: 0,
            x68000_display_mode: true,
            main_display_control_enabled: false,
            option2_display_control_enabled: false,
            current_tick: 0,
        }
    }

    /// Resets keyboard protocol state.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Updates one physical key and queues its make or break code.
    pub fn set_key_state(&mut self, code: u8, pressed: bool, tick: u64) {
        self.advance_to(tick);
        let code = code & 0x7F;
        if code == 0 || self.pressed[usize::from(code)] == pressed {
            return;
        }
        self.pressed[usize::from(code)] = pressed;
        if self.enabled() {
            self.output
                .push_back(if pressed { code } else { code | 0x80 });
        }
        if pressed && is_repeatable(code) {
            self.repeat_code = Some(code);
            self.repeat_deadline = Some(tick + self.repeat_delay_ticks);
        } else if !pressed && self.repeat_code == Some(code) {
            self.repeat_code = None;
            self.repeat_deadline = None;
        }
    }

    /// Applies one command received from the computer.
    pub fn write_command(&mut self, value: u8, tick: u64) {
        self.advance_to(tick);
        match value {
            0x00..=0x3F => self.display_control = value,
            0x40..=0x47 => self.mouse_control = value & 1 != 0,
            0x48..=0x4F => self.set_command_enabled(value & 1 != 0),
            0x50..=0x53 => self.x68000_display_mode = value & 1 != 0,
            0x54..=0x57 => self.led_brightness = value & 3,
            0x58..=0x5B => self.main_display_control_enabled = value & 1 != 0,
            0x5C..=0x5F => self.option2_display_control_enabled = value & 1 != 0,
            0x60..=0x6F => {
                self.repeat_delay_ticks = 200_000 + u64::from(value & 0x0F) * 100_000;
            }
            0x70..=0x7F => {
                let factor = u64::from(value & 0x0F);
                self.repeat_interval_ticks = 30_000 + factor * factor * 5_000;
            }
            0x80..=0xFF => self.leds = !value & 0x7F,
        }
    }

    /// Controls the system-port keyboard transmission gate.
    pub fn set_system_transmit_enabled(&mut self, enabled: bool) {
        let was_enabled = self.enabled();
        self.system_enabled = enabled;
        self.handle_enable_transition(was_enabled);
    }

    /// Advances repeat timing.
    pub fn advance_to(&mut self, tick: u64) {
        if tick < self.current_tick {
            return;
        }
        while let Some(deadline) = self.repeat_deadline {
            if deadline > tick {
                break;
            }
            let Some(code) = self.repeat_code else {
                self.repeat_deadline = None;
                break;
            };
            if self.enabled() && self.pressed[usize::from(code)] {
                self.output.push_back(code);
            }
            self.repeat_deadline = Some(deadline + self.repeat_interval_ticks);
        }
        self.current_tick = tick;
    }

    /// Returns the next repeat deadline.
    pub const fn next_event_tick(&self) -> Option<u64> {
        self.repeat_deadline
    }

    /// Takes the next byte waiting for the MFP receiver.
    pub fn take_output_byte(&mut self) -> Option<u8> {
        self.output.pop_front()
    }

    /// Returns whether output transmission is enabled.
    pub const fn enabled(&self) -> bool {
        self.system_enabled && self.command_enabled
    }

    /// Returns the active-high keyboard LED state.
    pub const fn leds(&self) -> u8 {
        self.leds
    }

    /// Returns the selected LED brightness.
    pub const fn led_brightness(&self) -> u8 {
        self.led_brightness
    }

    /// Returns the mouse-control output.
    pub const fn mouse_control(&self) -> bool {
        self.mouse_control
    }

    fn set_command_enabled(&mut self, enabled: bool) {
        let was_enabled = self.enabled();
        self.command_enabled = enabled;
        self.handle_enable_transition(was_enabled);
    }

    fn handle_enable_transition(&mut self, was_enabled: bool) {
        if !was_enabled && self.enabled() {
            for code in 1_u8..=0x7F {
                if self.pressed[usize::from(code)] && code != 0x74 {
                    self.output.push_back(code);
                }
            }
        }
    }
}

fn is_repeatable(code: u8) -> bool {
    !matches!(code, 0x70..=0x74)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_keyboard() -> KeyboardX68k {
        let mut keyboard = KeyboardX68k::new();
        keyboard.set_system_transmit_enabled(true);
        keyboard.write_command(0x49, 0);
        keyboard
    }

    #[test]
    fn make_break_and_repeat_are_queued() {
        let mut keyboard = enabled_keyboard();
        keyboard.set_key_state(0x1E, true, 0);
        assert_eq!(keyboard.take_output_byte(), Some(0x1E));
        keyboard.advance_to(DEFAULT_REPEAT_DELAY_TICKS);
        assert_eq!(keyboard.take_output_byte(), Some(0x1E));
        keyboard.set_key_state(0x1E, false, DEFAULT_REPEAT_DELAY_TICKS + 1);
        assert_eq!(keyboard.take_output_byte(), Some(0x9E));
    }

    #[test]
    fn enabling_reports_held_keys() {
        let mut keyboard = KeyboardX68k::new();
        keyboard.set_key_state(0x73, true, 0);
        keyboard.set_system_transmit_enabled(true);
        keyboard.write_command(0x49, 0);
        assert_eq!(keyboard.take_output_byte(), Some(0x73));
    }

    #[test]
    fn commands_set_repeat_and_active_low_leds() {
        let mut keyboard = enabled_keyboard();
        keyboard.write_command(0x6F, 0);
        keyboard.write_command(0x7F, 0);
        keyboard.write_command(0xFE, 0);
        assert_eq!(keyboard.repeat_delay_ticks, 1_700_000);
        assert_eq!(keyboard.repeat_interval_ticks, 1_155_000);
        assert_eq!(keyboard.leds(), 1);
    }
}
