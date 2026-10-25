//! The window, the engines and the input devices of the shell that outlive a
//! session.

use std::path::PathBuf;

use audio_engine::AudioEngine;
use common::{info, warn};
use gui::ui::ThemeKind;
use sdl3::{
    gamepad::{Gamepad, GamepadSubsystem},
    video::Window,
};
use sdl3_backend::{DisplayAspectMode, GraphicsEngine, Scaling};

use crate::{
    config::{AspectMode, Backend, EmulatorConfig, GraphicboardType, ScalingMode},
    gui_host::GuiHost,
};

/// The width of a new window in screen coordinates.
const INITIAL_WINDOW_WIDTH: u32 = 1280;
/// The height of a new window with the 4:3 aspect mode.
const INITIAL_WINDOW_HEIGHT_ASPECT_4_BY_3: u32 = 960;
/// The height of a new window with the 1:1 aspect mode.
const INITIAL_WINDOW_HEIGHT_ASPECT_1_BY_1: u32 = 800;

/// The smallest size of the window of the game manager in logical units of
/// the user interface.
pub(crate) const MINIMUM_WINDOW_SIZE: (u32, u32) = (960, 600);

/// The smallest size of the window of a game session in screen coordinates.
pub(crate) const SESSION_MINIMUM_WINDOW_SIZE: (u32, u32) = (960, 600);

/// The size of the window of the game manager in logical units of the user
/// interface.
pub(crate) const MANAGER_WINDOW_SIZE: (u32, u32) = (1280, 800);

/// The part of the usable display area that a new window fills at most.
const DISPLAY_FILL: f32 = 0.9;

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
    /// not load, which disables the media dialog and the game manager.
    pub(crate) gui: Option<GuiHost>,
    /// The reason the font did not load.
    pub(crate) gui_error: Option<String>,
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

/// The size of a new window and its unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowSize {
    /// A size in screen coordinates.
    Screen((u32, u32)),
    /// A size in logical units of the user interface, which converts into
    /// screen coordinates with the scale of the window.
    Logical((u32, u32)),
}

/// Returns the screen coordinates of one logical unit of the user interface
/// on a window with the display scale `display_scale` and the pixel density
/// `pixel_density`. The user interface draws `display_scale` physical pixels
/// for each logical unit, and the window has `pixel_density` physical pixels
/// for each screen coordinate. A value that is not positive counts as 1.
pub(crate) fn screen_units_per_logical_unit(display_scale: f32, pixel_density: f32) -> f32 {
    let positive = |value: f32| {
        if value.is_finite() && value > 0.0 {
            value
        } else {
            1.0
        }
    };
    positive(display_scale) / positive(pixel_density)
}

/// Returns `size` in logical units in screen coordinates with `factor`
/// screen coordinates for each logical unit, rounded up to whole
/// coordinates.
pub(crate) fn logical_to_screen(size: (u32, u32), factor: f32) -> (u32, u32) {
    let convert = |value: u32| (value as f32 * factor).ceil() as u32;
    (convert(size.0), convert(size.1))
}

/// Returns the minimum size of the manager window in screen coordinates
/// with `factor` screen coordinates for each logical unit. The size keeps
/// inside the usable display area `usable`.
pub(crate) fn manager_minimum_size(factor: f32, usable: Option<(u32, u32)>) -> (u32, u32) {
    let (width, height) = logical_to_screen(MINIMUM_WINDOW_SIZE, factor);
    match usable {
        Some((usable_width, usable_height)) => (width.min(usable_width), height.min(usable_height)),
        None => (width, height),
    }
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
    /// The paths searched for the font file of the user interface in search order.
    pub(crate) gui_font_paths: Vec<PathBuf>,
    /// The theme of the user interface.
    pub(crate) gui_theme: ThemeKind,
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
            gui_font_paths,
            gui_theme,
        } = parts;
        let scale_factor = window.display_scale();
        info!("Window created with scale factor: {scale_factor}");
        let (gui, gui_error) = match GuiHost::load(&gui_font_paths, gui_theme) {
            Ok(gui) => (Some(gui), None),
            Err(message) => (None, Some(message)),
        };
        Self {
            graphics_engine,
            audio_engine,
            gui,
            gui_error,
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

    /// The screen coordinates of one logical unit of the user interface on
    /// the window.
    pub(crate) fn screen_units_per_logical_unit(&self) -> f32 {
        screen_units_per_logical_unit(self.window.display_scale(), self.window.pixel_density())
    }

    /// Sets the minimum size of the manager window for the scale of its
    /// display, inside the usable area of the display.
    pub(crate) fn apply_manager_minimum_size(&self) {
        let minimum = manager_minimum_size(
            self.screen_units_per_logical_unit(),
            self.window.display_usable_size(),
        );
        self.set_minimum_size(minimum);
    }

    /// Sets the minimum size of the window of a game session.
    pub(crate) fn apply_session_minimum_size(&self) {
        self.set_minimum_size(SESSION_MINIMUM_WINDOW_SIZE);
    }

    /// Sets the minimum size of the window to `size` in screen coordinates.
    fn set_minimum_size(&self, (width, height): (u32, u32)) {
        if let Err(error) = self.window.set_minimum_size(width, height) {
            warn!("Failed to set the minimum window size {width} x {height}: {error}");
        }
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
            warn!("The media dialog is unavailable because the GUI font did not load");
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

/// The size of a window of `requested` size on a display with the usable
/// area `usable`. A window that does not fit shrinks with its aspect ratio
/// to the fill part of the display, and never below `minimum`. Every size is
/// in screen coordinates.
pub(crate) fn fit_window_size(
    requested: (u32, u32),
    usable: Option<(u32, u32)>,
    minimum: (u32, u32),
) -> (u32, u32) {
    let Some((usable_width, usable_height)) = usable else {
        return requested;
    };
    let (width, height) = requested;
    let room_width = usable_width as f32 * DISPLAY_FILL;
    let room_height = usable_height as f32 * DISPLAY_FILL;
    let factor = (room_width / width as f32).min(room_height / height as f32);
    if factor >= 1.0 {
        return requested;
    }
    (
        ((width as f32 * factor) as u32).max(minimum.0),
        ((height as f32 * factor) as u32).max(minimum.1),
    )
}

/// Whether the frames of the machine of `config` need the large native target.
pub(crate) fn wants_large_native_target(config: &EmulatorConfig) -> bool {
    config.graphicboard != GraphicboardType::None || config.target.wants_large_native_surface()
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
    use super::*;
    use crate::config::test_support::first_model_config;

    /// Verifies that a window shrinks with its aspect ratio to the display and keeps the minimum size.
    #[test]
    fn a_window_shrinks_with_its_aspect_ratio_to_fit_the_display() {
        let minimum = MINIMUM_WINDOW_SIZE;
        assert_eq!(fit_window_size((1280, 800), None, minimum), (1280, 800));
        assert_eq!(
            fit_window_size((1280, 800), Some((2560, 1400)), minimum),
            (1280, 800)
        );
        assert_eq!(
            fit_window_size((1280, 960), Some((1920, 1000)), minimum),
            (1200, 900)
        );
        assert_eq!(
            fit_window_size((1280, 800), Some((800, 500)), minimum),
            minimum
        );
        assert_eq!(
            fit_window_size((1920, 1200), Some((1920, 1050)), (1440, 900)),
            (1512, 945)
        );
    }

    /// Verifies the conversion from logical units into screen coordinates on X11, Wayland and macOS style scales.
    #[test]
    fn logical_units_convert_into_screen_coordinates() {
        let x11 = screen_units_per_logical_unit(1.5, 1.0);
        assert_eq!(x11, 1.5);
        assert_eq!(logical_to_screen(MINIMUM_WINDOW_SIZE, x11), (1440, 900));
        assert_eq!(logical_to_screen(MANAGER_WINDOW_SIZE, x11), (1920, 1200));
        let wayland = screen_units_per_logical_unit(1.5, 1.5);
        assert_eq!(logical_to_screen(MINIMUM_WINDOW_SIZE, wayland), (960, 600));
        let retina = screen_units_per_logical_unit(2.0, 2.0);
        assert_eq!(logical_to_screen(MANAGER_WINDOW_SIZE, retina), (1280, 800));
        let odd = screen_units_per_logical_unit(1.25, 1.0);
        assert_eq!(logical_to_screen((961, 601), odd), (1202, 752));
        assert_eq!(screen_units_per_logical_unit(0.0, f32::NAN), 1.0);
        let (width, height) = logical_to_screen(MINIMUM_WINDOW_SIZE, x11);
        assert!(width as f32 / x11 >= 960.0 && height as f32 / x11 >= 600.0);
    }

    /// Verifies that the minimum size of the manager keeps inside a small display at the display scale 1.5.
    #[test]
    fn the_manager_minimum_keeps_inside_the_display() {
        assert_eq!(manager_minimum_size(1.5, None), (1440, 900));
        assert_eq!(manager_minimum_size(1.5, Some((2560, 1400))), (1440, 900));
        assert_eq!(manager_minimum_size(1.5, Some((1366, 768))), (1366, 768));
        assert_eq!(manager_minimum_size(1.0, Some((1366, 768))), (960, 600));
    }

    /// Verifies that a game session keeps its 4:3 window of 1280 x 960 at the display scale 1.5, where the manager minimum is wider.
    #[test]
    fn a_session_keeps_its_window_at_the_display_scale_one_and_a_half() {
        let requested = initial_window_size(AspectMode::Aspect4By3);
        assert_eq!(requested, (1280, 960));
        let usable = Some((2560, 1400));
        assert_eq!(
            fit_window_size(requested, usable, SESSION_MINIMUM_WINDOW_SIZE),
            (1280, 960)
        );
        let manager_minimum = manager_minimum_size(1.5, usable);
        assert!(manager_minimum.0 > requested.0);
        assert!(SESSION_MINIMUM_WINDOW_SIZE.0 <= requested.0);
        assert!(SESSION_MINIMUM_WINDOW_SIZE.1 <= requested.1);
    }

    /// Verifies that a graphics board selects the large native target.
    #[test]
    fn a_graphic_board_needs_the_large_native_target() {
        let config = first_model_config();
        let large = config.target.wants_large_native_surface();
        assert_eq!(wants_large_native_target(&config), large);
        let with_board = EmulatorConfig {
            graphicboard: GraphicboardType::Ga1280a,
            ..config
        };
        assert!(wants_large_native_target(&with_board));
    }

    #[test]
    fn a_new_window_has_the_locked_aspect_ratio() {
        for mode in [AspectMode::Aspect4By3, AspectMode::Aspect1By1] {
            let (width, height) = initial_window_size(mode);
            assert_eq!(width as f32 / height as f32, aspect_ratio_for_mode(mode));
        }
    }
}
