use std::marker::PhantomData;

use sdl3_sys::{gamepad as ffi, init};

use crate::Error;

/// A gamepad button.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum GamepadButton {
    /// Bottom face button (e.g. Xbox A).
    South,
    /// Right face button (e.g. Xbox B).
    East,
    /// Left face button (e.g. Xbox X).
    West,
    /// Top face button (e.g. Xbox Y).
    North,
    /// Back / select button.
    Back,
    /// Guide button.
    Guide,
    /// Start button.
    Start,
    /// Left shoulder button.
    LeftShoulder,
    /// Right shoulder button.
    RightShoulder,
    /// Directional pad up.
    DpadUp,
    /// Directional pad down.
    DpadDown,
    /// Directional pad left.
    DpadLeft,
    /// Directional pad right.
    DpadRight,
    /// An unrecognized button.
    Unknown,
}

impl GamepadButton {
    /// Converts a raw SDL3 gamepad button code to a `GamepadButton`.
    pub fn from_raw(button: u8) -> Self {
        match i32::from(button) {
            x if x == ffi::SDL_GAMEPAD_BUTTON_SOUTH.0 => Self::South,
            x if x == ffi::SDL_GAMEPAD_BUTTON_EAST.0 => Self::East,
            x if x == ffi::SDL_GAMEPAD_BUTTON_WEST.0 => Self::West,
            x if x == ffi::SDL_GAMEPAD_BUTTON_NORTH.0 => Self::North,
            x if x == ffi::SDL_GAMEPAD_BUTTON_BACK.0 => Self::Back,
            x if x == ffi::SDL_GAMEPAD_BUTTON_GUIDE.0 => Self::Guide,
            x if x == ffi::SDL_GAMEPAD_BUTTON_START.0 => Self::Start,
            x if x == ffi::SDL_GAMEPAD_BUTTON_LEFT_SHOULDER.0 => Self::LeftShoulder,
            x if x == ffi::SDL_GAMEPAD_BUTTON_RIGHT_SHOULDER.0 => Self::RightShoulder,
            x if x == ffi::SDL_GAMEPAD_BUTTON_DPAD_UP.0 => Self::DpadUp,
            x if x == ffi::SDL_GAMEPAD_BUTTON_DPAD_DOWN.0 => Self::DpadDown,
            x if x == ffi::SDL_GAMEPAD_BUTTON_DPAD_LEFT.0 => Self::DpadLeft,
            x if x == ffi::SDL_GAMEPAD_BUTTON_DPAD_RIGHT.0 => Self::DpadRight,
            _ => Self::Unknown,
        }
    }
}

/// A gamepad analog axis.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum GamepadAxis {
    /// Left stick horizontal axis.
    LeftX,
    /// Left stick vertical axis.
    LeftY,
    /// Right stick horizontal axis.
    RightX,
    /// Right stick vertical axis.
    RightY,
    /// Left trigger axis.
    LeftTrigger,
    /// Right trigger axis.
    RightTrigger,
    /// An unrecognized axis.
    Unknown,
}

impl GamepadAxis {
    /// Converts a raw SDL3 gamepad axis code to a `GamepadAxis`.
    pub fn from_raw(axis: u8) -> Self {
        match i32::from(axis) {
            x if x == ffi::SDL_GAMEPAD_AXIS_LEFTX.0 => Self::LeftX,
            x if x == ffi::SDL_GAMEPAD_AXIS_LEFTY.0 => Self::LeftY,
            x if x == ffi::SDL_GAMEPAD_AXIS_RIGHTX.0 => Self::RightX,
            x if x == ffi::SDL_GAMEPAD_AXIS_RIGHTY.0 => Self::RightY,
            x if x == ffi::SDL_GAMEPAD_AXIS_LEFT_TRIGGER.0 => Self::LeftTrigger,
            x if x == ffi::SDL_GAMEPAD_AXIS_RIGHT_TRIGGER.0 => Self::RightTrigger,
            _ => Self::Unknown,
        }
    }
}

/// Manages the SDL3 gamepad subsystem. Calls `SDL_QuitSubSystem(GAMEPAD)` on drop.
pub struct GamepadSubsystem {
    _marker: PhantomData<*mut ()>,
}

impl GamepadSubsystem {
    pub(crate) fn new() -> Result<Self, Error> {
        // Safety: Called from the main thread after SDL_Init.
        let ok = unsafe { init::SDL_InitSubSystem(init::SDL_INIT_GAMEPAD) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(Self {
            _marker: PhantomData,
        })
    }

    /// Opens the gamepad with the given joystick instance ID.
    pub fn open(&self, instance_id: u32) -> Result<Gamepad, Error> {
        let id = sdl3_sys::joystick::SDL_JoystickID(instance_id);
        // Safety: A valid instance ID is passed; SDL returns NULL on failure.
        let handle = unsafe { ffi::SDL_OpenGamepad(id) };
        if handle.is_null() {
            return Err(crate::get_error());
        }
        Ok(Gamepad {
            handle,
            instance_id,
        })
    }
}

impl Drop for GamepadSubsystem {
    fn drop(&mut self) {
        // Safety: Matches the SDL_InitSubSystem call in new().
        unsafe { init::SDL_QuitSubSystem(init::SDL_INIT_GAMEPAD) }
    }
}

/// An opened gamepad. Calls `SDL_CloseGamepad` on drop.
pub struct Gamepad {
    handle: *mut ffi::SDL_Gamepad,
    instance_id: u32,
}

impl Gamepad {
    /// Returns the joystick instance ID this gamepad was opened with.
    pub fn instance_id(&self) -> u32 {
        self.instance_id
    }
}

impl Drop for Gamepad {
    fn drop(&mut self) {
        // Safety: The handle came from SDL_OpenGamepad and is closed once.
        unsafe { ffi::SDL_CloseGamepad(self.handle) }
    }
}
