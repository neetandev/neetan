//! The window, the engines and the input devices of the shell that outlive a
//! session.

use audio_engine::AudioEngine;
use common::{info, warn};
use sdl3::{
    gamepad::{Gamepad, GamepadSubsystem},
    video::Window,
};
use sdl3_backend::{DisplayAspectMode, GraphicsEngine, Scaling};

use crate::{
    config::{AspectMode, Backend, ScalingMode},
    gui_host::GuiHost,
};

/// The width of a new window in screen coordinates.
const INITIAL_WINDOW_WIDTH: u32 = 1280;
/// The height of a new window with the 4:3 aspect mode.
const INITIAL_WINDOW_HEIGHT_ASPECT_4_BY_3: u32 = 960;
/// The height of a new window with the 1:1 aspect mode.
const INITIAL_WINDOW_HEIGHT_ASPECT_1_BY_1: u32 = 800;

/// The window, the engines, the gamepad and the GUI host. They stay across
/// sessions.
///
/// The field order is the drop order: the engines go before the window.
pub(crate) struct Host {
    /// The graphics engine.
    pub(crate) graphics_engine: Box<dyn GraphicsEngine>,
    /// The audio engine. Its consumption rate paces the emulation.
    pub(crate) audio_engine: AudioEngine,
    /// The host of the graphical user interface. `None` when the font did
    /// not load, which disables the media dialog.
    pub(crate) gui: Option<GuiHost>,
    /// The opened gamepad, if one is connected.
    pub(crate) gamepad: Option<Gamepad>,
    /// The SDL3 gamepad subsystem, if it initialized.
    gamepad_subsystem: Option<GamepadSubsystem>,
    /// The window.
    pub(crate) window: Window,
    /// The active graphics backend.
    pub(crate) backend: Backend,
    /// Whether the window is in fullscreen mode.
    pub(crate) fullscreen: bool,
    /// The display scale factor of the window.
    pub(crate) scale_factor: f32,
    /// Whether the right Ctrl key is held.
    pub(crate) right_control_held: bool,
    /// Whether a media hotkey already logged that the GUI is disabled.
    pub(crate) gui_disabled_reported: bool,
}

/// The size and the fullscreen state of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowState {
    /// The size of the window in screen coordinates.
    pub(crate) size: (u32, u32),
    /// Whether the window is in fullscreen mode.
    pub(crate) fullscreen: bool,
}

/// The parts of a new [`Host`].
pub(crate) struct HostParts {
    /// The window.
    pub(crate) window: Window,
    /// The graphics engine.
    pub(crate) graphics_engine: Box<dyn GraphicsEngine>,
    /// The active graphics backend.
    pub(crate) backend: Backend,
    /// The audio engine.
    pub(crate) audio_engine: AudioEngine,
    /// The SDL3 gamepad subsystem, if it initialized.
    pub(crate) gamepad_subsystem: Option<GamepadSubsystem>,
    /// Whether the window starts in fullscreen mode.
    pub(crate) fullscreen: bool,
}

impl Host {
    /// Creates the host from its parts and loads the GUI font.
    pub(crate) fn new(parts: HostParts) -> Self {
        let HostParts {
            window,
            graphics_engine,
            backend,
            audio_engine,
            gamepad_subsystem,
            fullscreen,
        } = parts;
        let scale_factor = window.display_scale();
        info!("Window created with scale factor: {scale_factor}");
        Self {
            graphics_engine,
            audio_engine,
            gui: GuiHost::load(),
            gamepad: None,
            gamepad_subsystem,
            window,
            backend,
            fullscreen,
            scale_factor,
            right_control_held: false,
            gui_disabled_reported: false,
        }
    }

    /// Opens the gamepad `which` if no gamepad is in use.
    pub(crate) fn open_gamepad(&mut self, which: u32) {
        if self.gamepad.is_some() {
            return;
        }
        let Some(subsystem) = self.gamepad_subsystem.as_ref() else {
            return;
        };
        match subsystem.open(which) {
            Ok(pad) => {
                info!("Gamepad connected (instance {which})");
                self.gamepad = Some(pad);
            }
            Err(error) => warn!("Failed to open gamepad {which}: {error}"),
        }
    }

    /// Closes the gamepad `which` when it is the opened one. Returns whether
    /// it was.
    pub(crate) fn close_gamepad(&mut self, which: u32) -> bool {
        if !self.is_active_gamepad(which) {
            return false;
        }
        self.gamepad = None;
        if let Some(gui) = self.gui.as_mut() {
            gui.gamepad.clear();
        }
        true
    }

    /// Whether `which` is the instance of the opened gamepad.
    pub(crate) fn is_active_gamepad(&self, which: u32) -> bool {
        self.gamepad
            .as_ref()
            .is_some_and(|pad| pad.instance_id() == which)
    }

    /// Takes the size and the scale of the window into the user interface.
    pub(crate) fn update_gui_window(&mut self) {
        if let Some(gui) = self.gui.as_mut() {
            gui.update_window(&self.window);
        }
    }

    /// Switches the window into or out of fullscreen mode.
    pub(crate) fn set_fullscreen(&mut self, fullscreen: bool) {
        if let Err(error) = self.window.set_fullscreen(fullscreen) {
            warn!("Failed to toggle fullscreen: {error}");
        } else {
            self.fullscreen = fullscreen;
        }
    }

    /// The size and the fullscreen state of the window.
    pub(crate) fn window_state(&self) -> WindowState {
        WindowState {
            size: self.window.size(),
            fullscreen: self.fullscreen,
        }
    }

    /// Restores the fullscreen state of `state` and, in windowed mode, its
    /// size.
    pub(crate) fn restore_window_state(&mut self, state: WindowState) {
        if state.fullscreen != self.fullscreen {
            self.set_fullscreen(state.fullscreen);
        }
        if state.fullscreen || self.window.size() == state.size {
            return;
        }
        let (width, height) = state.size;
        if let Err(error) = self.window.set_size(width, height) {
            warn!("Failed to restore the window size {width} x {height}: {error}");
        }
    }

    /// Whether the GUI is available. Logs once when it is not.
    pub(crate) fn gui_available(&mut self) -> bool {
        if self.gui.is_some() {
            return true;
        }
        if !self.gui_disabled_reported {
            warn!("The media dialog is disabled because the GUI font did not load");
            self.gui_disabled_reported = true;
        }
        false
    }
}

/// The size of a new window for `aspect_mode`.
pub(crate) fn initial_window_size(aspect_mode: AspectMode) -> (u32, u32) {
    let initial_height = match aspect_mode {
        AspectMode::Aspect4By3 => INITIAL_WINDOW_HEIGHT_ASPECT_4_BY_3,
        AspectMode::Aspect1By1 => INITIAL_WINDOW_HEIGHT_ASPECT_1_BY_1,
    };
    (INITIAL_WINDOW_WIDTH, initial_height)
}

/// The window aspect ratio of `aspect_mode`.
pub(crate) fn aspect_ratio_for_mode(aspect_mode: AspectMode) -> f32 {
    match aspect_mode {
        AspectMode::Aspect4By3 => 4.0 / 3.0,
        AspectMode::Aspect1By1 => 16.0 / 10.0,
    }
}

/// The display aspect mode of the graphics engine for `aspect_mode`.
pub(crate) fn graphics_display_aspect_mode(aspect_mode: AspectMode) -> DisplayAspectMode {
    match aspect_mode {
        AspectMode::Aspect4By3 => DisplayAspectMode::Aspect4By3,
        AspectMode::Aspect1By1 => DisplayAspectMode::Aspect1By1,
    }
}

/// The scaling of the graphics engine for `mode`.
pub(crate) fn graphics_scaling(mode: ScalingMode) -> Scaling {
    match mode {
        ScalingMode::Nearest => Scaling::Nearest,
        ScalingMode::Bilinear => Scaling::Bilinear,
        ScalingMode::Pixelart => Scaling::Pixelart,
    }
}

#[cfg(test)]
mod tests {
    use super::{aspect_ratio_for_mode, initial_window_size};
    use crate::config::AspectMode;

    #[test]
    fn a_new_window_has_the_locked_aspect_ratio() {
        for mode in [AspectMode::Aspect4By3, AspectMode::Aspect1By1] {
            let (width, height) = initial_window_size(mode);
            assert_eq!(width as f32 / height as f32, aspect_ratio_for_mode(mode));
        }
    }
}
