//! The host of the graphical user interface: the font, the theme, the widget
//! state, the input of the next pass and the scene of the last pass, and the
//! mapping of the SDL events to the input.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use common::{error, warn};
use gui::{
    Fonts, Scene,
    fonts::{FONT_DIRECTORY, FONT_FILE},
    ui::{Input, Navigation, Theme, UiContext, UiState, Viewport},
};
use sdl3::{
    event::Event,
    gamepad::{GamepadAxis, GamepadButton},
    keyboard::{Mod, Scancode},
    mouse::MouseButton,
    video::Window,
};
use sdl3_backend::Overlay;

/// The delay before a held gamepad direction repeats.
const REPEAT_DELAY: Duration = Duration::from_millis(400);

/// The interval between the repeats of a held gamepad direction.
const REPEAT_INTERVAL: Duration = Duration::from_millis(80);

/// The stick deflection that engages a direction.
const STICK_PRESS: i16 = 16384;

/// The stick deflection below which an engaged direction releases.
const STICK_RELEASE: i16 = 11000;

/// The paths to look for the font file, in order: next to the executable
/// (Windows archive), in `share/neetan` beside the executable folder
/// (AppImage and FHS installs), in the `Resources` folder of a macOS bundle,
/// and in the font folder of the package that `cargo run` started.
fn font_candidates(
    executable_directory: Option<&Path>,
    cargo_manifest_directory: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(directory) = executable_directory {
        candidates.push(directory.join(FONT_DIRECTORY).join(FONT_FILE));
        if let Some(parent) = directory.parent() {
            candidates.push(
                parent
                    .join("share")
                    .join("neetan")
                    .join(FONT_DIRECTORY)
                    .join(FONT_FILE),
            );
            candidates.push(
                parent
                    .join("Resources")
                    .join(FONT_DIRECTORY)
                    .join(FONT_FILE),
            );
        }
    }
    if let Some(directory) = cargo_manifest_directory {
        candidates.push(directory.join(FONT_DIRECTORY).join(FONT_FILE));
    }
    candidates
}

/// Formats `paths` as a comma separated list for a log line.
fn path_list(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A discrete or accumulating input event of the user interface.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum GuiEvent {
    /// The pointer moved to a position in window coordinates.
    PointerMoved {
        /// The x position in window coordinates.
        x: f32,
        /// The y position in window coordinates.
        y: f32,
    },
    /// A mouse button went down or up.
    Button {
        /// The button.
        button: MouseButton,
        /// Whether the button went down.
        pressed: bool,
        /// Whether the press is the second press of a double click.
        double: bool,
        /// The x position in window coordinates.
        x: f32,
        /// The y position in window coordinates.
        y: f32,
    },
    /// The wheel turned by steps. Positive steps scroll up.
    Wheel {
        /// The vertical steps.
        steps: f32,
        /// The x position in window coordinates.
        x: f32,
        /// The y position in window coordinates.
        y: f32,
    },
    /// A navigation key or gamepad direction.
    Navigation(Navigation),
    /// A confirm key or the confirm button of a gamepad.
    Confirm,
    /// The escape key or the cancel button of a gamepad.
    Cancel,
    /// The backspace key.
    Backspace,
    /// Committed text.
    Text(String),
    /// The new composition of an input method.
    Composition(String),
}

impl GuiEvent {
    /// Whether the event needs its own pass. Pointer motion and wheel steps
    /// add up until the next pass.
    fn is_discrete(&self) -> bool {
        !matches!(self, Self::PointerMoved { .. } | Self::Wheel { .. })
    }
}

/// The event of a pressed key. A repeated key press counts only for the
/// navigation keys and backspace.
pub(crate) fn key_event(scancode: Scancode, keymod: &Mod, repeat: bool) -> Option<GuiEvent> {
    let navigation = match scancode {
        Scancode::Up => Some(Navigation::Up),
        Scancode::Down => Some(Navigation::Down),
        Scancode::Left => Some(Navigation::Left),
        Scancode::Right => Some(Navigation::Right),
        Scancode::PageUp => Some(Navigation::PageUp),
        Scancode::PageDown => Some(Navigation::PageDown),
        Scancode::Home => Some(Navigation::Home),
        Scancode::End => Some(Navigation::End),
        Scancode::Tab if keymod.shift() => Some(Navigation::Previous),
        Scancode::Tab => Some(Navigation::Next),
        _ => None,
    };
    if let Some(navigation) = navigation {
        return Some(GuiEvent::Navigation(navigation));
    }
    match scancode {
        Scancode::Backspace => Some(GuiEvent::Backspace),
        Scancode::Return | Scancode::KpEnter if !repeat => Some(GuiEvent::Confirm),
        Scancode::Escape if !repeat => Some(GuiEvent::Cancel),
        _ => None,
    }
}

/// The event of a mouse, text or input method event of SDL. Keyboard and
/// gamepad events have their own mapping.
pub(crate) fn pointer_or_text_event(event: &Event) -> Option<GuiEvent> {
    match event {
        Event::MouseMotion { x, y, .. } => Some(GuiEvent::PointerMoved { x: *x, y: *y }),
        Event::MouseButtonDown {
            mouse_btn,
            clicks,
            x,
            y,
        } => Some(GuiEvent::Button {
            button: *mouse_btn,
            pressed: true,
            double: *clicks >= 2,
            x: *x,
            y: *y,
        }),
        Event::MouseButtonUp {
            mouse_btn, x, y, ..
        } => Some(GuiEvent::Button {
            button: *mouse_btn,
            pressed: false,
            double: false,
            x: *x,
            y: *y,
        }),
        Event::MouseWheel {
            y,
            mouse_x,
            mouse_y,
            ..
        } => Some(GuiEvent::Wheel {
            steps: *y,
            x: *mouse_x,
            y: *mouse_y,
        }),
        Event::TextInput { text } => Some(GuiEvent::Text(text.clone())),
        Event::TextEditing { text, .. } => Some(GuiEvent::Composition(text.clone())),
        _ => None,
    }
}

/// The control that holds a gamepad direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectionSource {
    /// The D-pad.
    Dpad,
    /// The left stick.
    Stick,
}

/// A held direction that repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Repeat {
    /// The direction.
    navigation: Navigation,
    /// The time of the next repeat.
    next: Instant,
}

/// The navigation of a gamepad: the D-pad and the left stick with a
/// synthesized repeat while a direction is held, and the face and shoulder
/// buttons.
#[derive(Debug, Default)]
pub(crate) struct GamepadNavigation {
    /// The held directions of the D-pad and the stick, the most recent last.
    held: Vec<(DirectionSource, Navigation)>,
    /// The horizontal deflection of the left stick.
    stick_x: i16,
    /// The vertical deflection of the left stick.
    stick_y: i16,
    /// The engaged direction of the left stick.
    stick: Option<Navigation>,
    /// The direction that repeats.
    repeat: Option<Repeat>,
}

impl GamepadNavigation {
    /// Forgets the held directions.
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Holds `navigation` from `source`, starts its repeat at `now` and
    /// returns its event.
    fn press(&mut self, source: DirectionSource, navigation: Navigation, now: Instant) -> GuiEvent {
        self.held.retain(|&entry| entry != (source, navigation));
        self.held.push((source, navigation));
        self.repeat = Some(Repeat {
            navigation,
            next: now + REPEAT_DELAY,
        });
        GuiEvent::Navigation(navigation)
    }

    /// Releases `navigation` from `source`. The repeat falls back to the most
    /// recent direction still held, which repeats after the initial delay
    /// from `now`, or stops.
    fn release(&mut self, source: DirectionSource, navigation: Navigation, now: Instant) {
        self.held.retain(|&entry| entry != (source, navigation));
        let fallback = self.held.last().map(|&(_, held)| held);
        let current = self.repeat.map(|repeat| repeat.navigation);
        if fallback != current {
            self.repeat = fallback.map(|navigation| Repeat {
                navigation,
                next: now + REPEAT_DELAY,
            });
        }
    }

    /// The event of a button change at `now`.
    pub(crate) fn button(
        &mut self,
        button: GamepadButton,
        pressed: bool,
        now: Instant,
    ) -> Option<GuiEvent> {
        let direction = match button {
            GamepadButton::DpadUp => Some(Navigation::Up),
            GamepadButton::DpadDown => Some(Navigation::Down),
            GamepadButton::DpadLeft => Some(Navigation::Left),
            GamepadButton::DpadRight => Some(Navigation::Right),
            _ => None,
        };
        if let Some(navigation) = direction {
            if pressed {
                return Some(self.press(DirectionSource::Dpad, navigation, now));
            }
            self.release(DirectionSource::Dpad, navigation, now);
            return None;
        }
        if !pressed {
            return None;
        }
        match button {
            GamepadButton::South => Some(GuiEvent::Confirm),
            GamepadButton::East => Some(GuiEvent::Cancel),
            GamepadButton::LeftShoulder => Some(GuiEvent::Navigation(Navigation::PreviousTab)),
            GamepadButton::RightShoulder => Some(GuiEvent::Navigation(Navigation::NextTab)),
            _ => None,
        }
    }

    /// The event of a stick change at `now`. A direction engages past
    /// [`STICK_PRESS`] on the axis with the larger deflection and releases
    /// below [`STICK_RELEASE`].
    pub(crate) fn axis(&mut self, axis: GamepadAxis, value: i16, now: Instant) -> Option<GuiEvent> {
        match axis {
            GamepadAxis::LeftX => self.stick_x = value,
            GamepadAxis::LeftY => self.stick_y = value,
            _ => return None,
        }
        let horizontal = self.stick_x.unsigned_abs();
        let vertical = self.stick_y.unsigned_abs();
        let still_engaged = match self.stick {
            Some(Navigation::Left | Navigation::Right) => horizontal >= STICK_RELEASE as u16,
            Some(_) => vertical >= STICK_RELEASE as u16,
            None => false,
        };
        let engaged = if still_engaged {
            match self.stick {
                Some(Navigation::Left | Navigation::Right) => Some(if self.stick_x < 0 {
                    Navigation::Left
                } else {
                    Navigation::Right
                }),
                _ => Some(if self.stick_y < 0 {
                    Navigation::Up
                } else {
                    Navigation::Down
                }),
            }
        } else if horizontal.max(vertical) >= STICK_PRESS as u16 {
            Some(if horizontal >= vertical {
                if self.stick_x < 0 {
                    Navigation::Left
                } else {
                    Navigation::Right
                }
            } else if self.stick_y < 0 {
                Navigation::Up
            } else {
                Navigation::Down
            })
        } else {
            None
        };
        if engaged == self.stick {
            return None;
        }
        let previous = std::mem::replace(&mut self.stick, engaged);
        if let Some(previous) = previous {
            self.release(DirectionSource::Stick, previous, now);
        }
        engaged.map(|navigation| self.press(DirectionSource::Stick, navigation, now))
    }

    /// The repeat event of the held direction that is due at `now`.
    pub(crate) fn poll(&mut self, now: Instant) -> Option<GuiEvent> {
        let repeat = self.repeat.as_mut()?;
        if now < repeat.next {
            return None;
        }
        let next = repeat.next + REPEAT_INTERVAL;
        repeat.next = if next <= now {
            now + REPEAT_INTERVAL
        } else {
            next
        };
        Some(GuiEvent::Navigation(repeat.navigation))
    }
}

/// The host of the graphical user interface.
pub(crate) struct GuiHost {
    /// The font and the icon paths.
    fonts: Fonts,
    /// The look of the widgets.
    theme: Theme,
    /// The widget state between passes.
    state: UiState,
    /// The input of the next pass.
    input: Input,
    /// The scene of the last pass.
    scene: Scene,
    /// The size of the window in physical pixels and the scale factor.
    viewport: Viewport,
    /// The physical pixels for each window coordinate.
    pixel_density: f32,
    /// Whether pointer motion, wheel steps or a window change wait for a pass.
    pending: bool,
    /// Whether SDL text input runs.
    text_input: bool,
    /// The text input area and the cursor offset last passed to SDL, in
    /// window coordinates.
    text_area: Option<[i32; 5]>,
    /// The gamepad navigation.
    pub(crate) gamepad: GamepadNavigation,
}

impl GuiHost {
    /// Locates and loads the font. On failure logs one error with every path
    /// tried and returns `None`.
    pub(crate) fn load() -> Option<Self> {
        let executable_directory = std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        let cargo_manifest_directory = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from);
        let candidates = font_candidates(
            executable_directory.as_deref(),
            cargo_manifest_directory.as_deref(),
        );
        let Some(path) = candidates.iter().find(|path| path.is_file()) else {
            error!(
                "GUI font {FONT_FILE} not found, the media dialog is disabled. Tried: {}",
                path_list(&candidates)
            );
            return None;
        };
        let result = std::fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|data| Self::new(data).map_err(|error| error.to_string()));
        match result {
            Ok(host) => Some(host),
            Err(error) => {
                error!(
                    "Failed to load the GUI font {FONT_FILE} from {}: {error}. The media dialog is disabled. Tried: {}",
                    path.display(),
                    path_list(&candidates)
                );
                None
            }
        }
    }

    /// A host with the font file `data`.
    pub(crate) fn new(data: Vec<u8>) -> Result<Self, gui::truetype::FontError> {
        let mut fonts = Fonts::new();
        let font = fonts.add(data)?;
        let theme = Theme::new(&mut fonts, font);
        Ok(Self {
            fonts,
            theme,
            state: UiState::default(),
            input: Input::default(),
            scene: Scene::new(),
            viewport: Viewport::new(0, 0, 1.0),
            pixel_density: 1.0,
            pending: true,
            text_input: false,
            text_area: None,
            gamepad: GamepadNavigation::default(),
        })
    }

    /// Forgets the widget state, the buttons and the composition, for a
    /// dialog that opens. The pointer position stays.
    pub(crate) fn reset(&mut self) {
        self.state.reset();
        self.input = Input {
            mouse_x: self.input.mouse_x,
            mouse_y: self.input.mouse_y,
            ..Input::default()
        };
        self.gamepad.clear();
        self.pending = true;
    }

    /// Takes the size, the display scale and the pixel density of `window`.
    /// A change waits for a pass.
    pub(crate) fn update_window(&mut self, window: &Window) {
        let (width, height) = window.size_in_pixels();
        self.set_window_metrics(
            width,
            height,
            window.display_scale(),
            window.pixel_density(),
        );
    }

    /// Takes the size in physical pixels, the scale factor and the pixel
    /// density of the window. A change waits for a pass.
    fn set_window_metrics(&mut self, width: u32, height: u32, scale: f32, pixel_density: f32) {
        let viewport = Viewport::new(width, height, scale);
        let pixel_density = if pixel_density.is_finite() && pixel_density > 0.0 {
            pixel_density
        } else {
            1.0
        };
        if viewport != self.viewport || pixel_density != self.pixel_density {
            self.viewport = viewport;
            self.pixel_density = pixel_density;
            self.pending = true;
        }
    }

    /// Adds `event` to the input of the next pass. Returns whether the event
    /// needs a pass now.
    pub(crate) fn apply(&mut self, event: GuiEvent) -> bool {
        let discrete = event.is_discrete();
        let density = self.pixel_density;
        let input = &mut self.input;
        match event {
            GuiEvent::PointerMoved { x, y } => {
                input.mouse_x = x * density;
                input.mouse_y = y * density;
            }
            GuiEvent::Button {
                button,
                pressed,
                double,
                x,
                y,
            } => {
                input.mouse_x = x * density;
                input.mouse_y = y * density;
                match button {
                    MouseButton::Left => {
                        input.left = pressed;
                        input.left_double = pressed && double;
                    }
                    MouseButton::Right => input.right = pressed,
                    MouseButton::Middle => input.middle = pressed,
                    MouseButton::X1 | MouseButton::X2 | MouseButton::Unknown => return false,
                }
            }
            GuiEvent::Wheel { steps, x, y } => {
                input.mouse_x = x * density;
                input.mouse_y = y * density;
                input.wheel += steps;
            }
            GuiEvent::Navigation(navigation) => input.navigation = Some(navigation),
            GuiEvent::Confirm => input.confirm = true,
            GuiEvent::Cancel => input.cancel = true,
            GuiEvent::Backspace => input.backspace = true,
            GuiEvent::Text(text) => {
                input.text.push_str(&text);
                input.composition.clear();
            }
            GuiEvent::Composition(text) => input.composition = text,
        }
        if !discrete {
            self.pending = true;
        }
        discrete
    }

    /// Whether pointer motion, wheel steps or a window change wait for a
    /// pass.
    pub(crate) fn pending(&self) -> bool {
        self.pending
    }

    /// Runs one pass of `draw` with the input so far and keeps its scene.
    pub(crate) fn pass<R>(&mut self, draw: impl FnOnce(&mut UiContext) -> R) -> R {
        let mut context = UiContext::new(
            &mut self.scene,
            &mut self.fonts,
            &self.theme,
            &self.input,
            &mut self.state,
            self.viewport,
        );
        let result = draw(&mut context);
        context.finish();
        self.input.advance();
        self.pending = false;
        result
    }

    /// Starts or stops the SDL text input as the focused widget asks, and
    /// passes the area of the focused text field to the input method.
    pub(crate) fn sync_text_input(&mut self, window: &Window) {
        let wants = self.state.wants_text_input();
        if wants != self.text_input {
            let result = if wants {
                window.start_text_input()
            } else {
                window.stop_text_input()
            };
            if let Err(error) = result {
                warn!("Failed to switch the text input: {error}");
            }
            self.text_input = wants;
            self.text_area = None;
            if !wants {
                self.input.composition.clear();
            }
        }
        if !wants {
            return;
        }
        let Some(area) = self.state.text_input_area() else {
            return;
        };
        let density = self.pixel_density;
        let caret = self.state.text_input_caret().unwrap_or(0.0);
        let window_area = [
            (area.x / density).round() as i32,
            (area.y / density).round() as i32,
            (area.width / density).round() as i32,
            (area.height / density).round() as i32,
            (caret / density).round() as i32,
        ];
        if self.text_area != Some(window_area) {
            let [x, y, width, height, cursor] = window_area;
            if let Err(error) = window.set_text_input_area(x, y, width, height, cursor) {
                warn!("Failed to set the text input area: {error}");
            }
            self.text_area = Some(window_area);
        }
    }

    /// Stops the SDL text input, for a dialog that closes.
    pub(crate) fn stop_text_input(&mut self, window: Option<&Window>) {
        if self.text_input
            && let Some(window) = window
            && let Err(error) = window.stop_text_input()
        {
            warn!("Failed to stop the text input: {error}");
        }
        self.text_input = false;
        self.text_area = None;
        self.input.composition.clear();
    }

    /// The scene of the last pass as the overlay of a frame.
    pub(crate) fn overlay(&self) -> Overlay<'_> {
        Overlay {
            scene: &self.scene,
            slug: self.fonts.slug(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No modifier keys.
    const NO_MODIFIERS: Mod = Mod(0);

    /// The left shift key.
    const SHIFT: Mod = Mod(0x0001);

    /// A host with the font of the repository.
    fn host() -> GuiHost {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(FONT_DIRECTORY)
            .join(FONT_FILE);
        GuiHost::new(std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn the_font_lookup_follows_the_install_layouts() {
        let candidates = font_candidates(
            Some(Path::new("/opt/neetan/bin")),
            Some(Path::new("/home/user/neetan")),
        );
        assert_eq!(
            candidates,
            [
                PathBuf::from("/opt/neetan/bin/font/SarasaUiJ-Regular.ttf"),
                PathBuf::from("/opt/neetan/share/neetan/font/SarasaUiJ-Regular.ttf"),
                PathBuf::from("/opt/neetan/Resources/font/SarasaUiJ-Regular.ttf"),
                PathBuf::from("/home/user/neetan/font/SarasaUiJ-Regular.ttf"),
            ]
        );
        assert!(font_candidates(None, None).is_empty());
    }

    #[test]
    fn keys_map_to_navigation_and_actions() {
        let key = |scancode, keymod: &Mod, repeat| key_event(scancode, keymod, repeat);
        assert_eq!(
            key(Scancode::Up, &NO_MODIFIERS, false),
            Some(GuiEvent::Navigation(Navigation::Up))
        );
        assert_eq!(
            key(Scancode::PageDown, &NO_MODIFIERS, false),
            Some(GuiEvent::Navigation(Navigation::PageDown))
        );
        assert_eq!(
            key(Scancode::End, &NO_MODIFIERS, false),
            Some(GuiEvent::Navigation(Navigation::End))
        );
        assert_eq!(
            key(Scancode::Tab, &NO_MODIFIERS, false),
            Some(GuiEvent::Navigation(Navigation::Next))
        );
        assert_eq!(
            key(Scancode::Tab, &SHIFT, false),
            Some(GuiEvent::Navigation(Navigation::Previous))
        );
        assert_eq!(
            key(Scancode::Return, &NO_MODIFIERS, false),
            Some(GuiEvent::Confirm)
        );
        assert_eq!(
            key(Scancode::KpEnter, &NO_MODIFIERS, false),
            Some(GuiEvent::Confirm)
        );
        assert_eq!(
            key(Scancode::Escape, &NO_MODIFIERS, false),
            Some(GuiEvent::Cancel)
        );
        assert_eq!(
            key(Scancode::Backspace, &NO_MODIFIERS, false),
            Some(GuiEvent::Backspace)
        );
        assert_eq!(key(Scancode::A, &NO_MODIFIERS, false), None);
    }

    #[test]
    fn only_navigation_and_backspace_repeat() {
        assert_eq!(
            key_event(Scancode::Down, &NO_MODIFIERS, true),
            Some(GuiEvent::Navigation(Navigation::Down))
        );
        assert_eq!(
            key_event(Scancode::Backspace, &NO_MODIFIERS, true),
            Some(GuiEvent::Backspace)
        );
        assert_eq!(key_event(Scancode::Return, &NO_MODIFIERS, true), None);
        assert_eq!(key_event(Scancode::KpEnter, &NO_MODIFIERS, true), None);
        assert_eq!(key_event(Scancode::Escape, &NO_MODIFIERS, true), None);
    }

    #[test]
    fn mouse_and_text_events_map_to_the_input() {
        assert_eq!(
            pointer_or_text_event(&Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                clicks: 2,
                x: 10.0,
                y: 20.0,
            }),
            Some(GuiEvent::Button {
                button: MouseButton::Left,
                pressed: true,
                double: true,
                x: 10.0,
                y: 20.0,
            })
        );
        assert_eq!(
            pointer_or_text_event(&Event::MouseWheel {
                x: 0.0,
                y: -1.0,
                mouse_x: 3.0,
                mouse_y: 4.0,
            }),
            Some(GuiEvent::Wheel {
                steps: -1.0,
                x: 3.0,
                y: 4.0,
            })
        );
        assert_eq!(
            pointer_or_text_event(&Event::TextInput { text: "か".into() }),
            Some(GuiEvent::Text("か".into()))
        );
        assert_eq!(
            pointer_or_text_event(&Event::TextEditing {
                text: "かな".into(),
                start: 2,
                length: 0,
            }),
            Some(GuiEvent::Composition("かな".into()))
        );
        assert_eq!(pointer_or_text_event(&Event::Quit), None);
    }

    #[test]
    fn pointer_positions_scale_to_physical_pixels() {
        let mut host = host();
        host.set_window_metrics(1600, 1200, 2.0, 2.0);
        assert!(!host.apply(GuiEvent::PointerMoved { x: 100.0, y: 50.5 }));
        assert_eq!((host.input.mouse_x, host.input.mouse_y), (200.0, 101.0));
        assert!(!host.apply(GuiEvent::Wheel {
            steps: 1.0,
            x: 100.0,
            y: 50.0,
        }));
        assert!(!host.apply(GuiEvent::Wheel {
            steps: 2.0,
            x: 100.0,
            y: 50.0,
        }));
        assert_eq!(host.input.wheel, 3.0);
        assert!(host.apply(GuiEvent::Button {
            button: MouseButton::Left,
            pressed: true,
            double: true,
            x: 10.0,
            y: 20.0,
        }));
        assert!(host.input.left && host.input.left_double);
        assert_eq!((host.input.mouse_x, host.input.mouse_y), (20.0, 40.0));
        assert!(host.pending());
        host.pass(|_| ());
        assert!(!host.pending());
        assert_eq!(host.input.wheel, 0.0);
        assert!(host.input.left && host.input.left_previous);
    }

    #[test]
    fn window_changes_wait_for_a_pass() {
        let mut host = host();
        host.pass(|_| ());
        host.set_window_metrics(800, 600, 1.0, 1.0);
        assert!(host.pending());
        host.pass(|_| ());
        host.set_window_metrics(800, 600, 1.0, 1.0);
        assert!(!host.pending());
        host.set_window_metrics(800, 600, 1.5, 1.0);
        assert!(host.pending());
    }

    #[test]
    fn gamepad_buttons_map_to_actions() {
        let mut gamepad = GamepadNavigation::default();
        let now = Instant::now();
        assert_eq!(
            gamepad.button(GamepadButton::South, true, now),
            Some(GuiEvent::Confirm)
        );
        assert_eq!(gamepad.button(GamepadButton::South, false, now), None);
        assert_eq!(
            gamepad.button(GamepadButton::East, true, now),
            Some(GuiEvent::Cancel)
        );
        assert_eq!(
            gamepad.button(GamepadButton::LeftShoulder, true, now),
            Some(GuiEvent::Navigation(Navigation::PreviousTab))
        );
        assert_eq!(
            gamepad.button(GamepadButton::RightShoulder, true, now),
            Some(GuiEvent::Navigation(Navigation::NextTab))
        );
        assert_eq!(gamepad.button(GamepadButton::Start, true, now), None);
    }

    #[test]
    fn a_held_dpad_direction_repeats() {
        let mut gamepad = GamepadNavigation::default();
        let start = Instant::now();
        assert_eq!(
            gamepad.button(GamepadButton::DpadDown, true, start),
            Some(GuiEvent::Navigation(Navigation::Down))
        );
        assert_eq!(gamepad.poll(start + Duration::from_millis(399)), None);
        let first = start + REPEAT_DELAY;
        assert_eq!(
            gamepad.poll(first),
            Some(GuiEvent::Navigation(Navigation::Down))
        );
        assert_eq!(gamepad.poll(first + Duration::from_millis(79)), None);
        assert_eq!(
            gamepad.poll(first + REPEAT_INTERVAL),
            Some(GuiEvent::Navigation(Navigation::Down))
        );
        assert_eq!(gamepad.button(GamepadButton::DpadDown, false, first), None);
        assert_eq!(gamepad.poll(first + Duration::from_secs(1)), None);
    }

    #[test]
    fn a_late_poll_repeats_once() {
        let mut gamepad = GamepadNavigation::default();
        let start = Instant::now();
        gamepad.button(GamepadButton::DpadUp, true, start);
        let late = start + Duration::from_secs(2);
        assert!(gamepad.poll(late).is_some());
        assert_eq!(gamepad.poll(late), None);
        assert!(gamepad.poll(late + REPEAT_INTERVAL).is_some());
    }

    #[test]
    fn the_stick_has_a_dead_zone_and_hysteresis() {
        let mut gamepad = GamepadNavigation::default();
        let now = Instant::now();
        assert_eq!(gamepad.axis(GamepadAxis::LeftY, 12000, now), None);
        assert_eq!(
            gamepad.axis(GamepadAxis::LeftY, 20000, now),
            Some(GuiEvent::Navigation(Navigation::Down))
        );
        assert_eq!(gamepad.axis(GamepadAxis::LeftY, 12000, now), None);
        assert!(gamepad.poll(now + REPEAT_DELAY).is_some());
        assert_eq!(gamepad.axis(GamepadAxis::LeftY, 9000, now), None);
        assert_eq!(gamepad.poll(now + Duration::from_secs(5)), None);
        assert_eq!(
            gamepad.axis(GamepadAxis::LeftX, -30000, now),
            Some(GuiEvent::Navigation(Navigation::Left))
        );
        assert_eq!(gamepad.axis(GamepadAxis::LeftY, -32768, now), None);
        assert_eq!(
            gamepad.axis(GamepadAxis::LeftX, 0, now),
            Some(GuiEvent::Navigation(Navigation::Up))
        );
        assert_eq!(gamepad.axis(GamepadAxis::RightX, 32767, now), None);
    }

    #[test]
    fn a_release_falls_back_to_the_most_recent_held_direction() {
        let mut gamepad = GamepadNavigation::default();
        let start = Instant::now();
        gamepad.button(GamepadButton::DpadDown, true, start);
        gamepad.axis(GamepadAxis::LeftX, 30000, start);
        gamepad.button(GamepadButton::DpadUp, true, start);
        let release = start + Duration::from_millis(100);
        gamepad.button(GamepadButton::DpadUp, false, release);
        assert_eq!(gamepad.poll(release + Duration::from_millis(399)), None);
        assert_eq!(
            gamepad.poll(release + REPEAT_DELAY),
            Some(GuiEvent::Navigation(Navigation::Right))
        );
        gamepad.axis(GamepadAxis::LeftX, 0, release);
        assert_eq!(
            gamepad.poll(release + REPEAT_DELAY * 2),
            Some(GuiEvent::Navigation(Navigation::Down))
        );
        gamepad.button(GamepadButton::DpadDown, false, release);
        assert_eq!(gamepad.poll(release + Duration::from_secs(5)), None);
    }

    #[test]
    fn a_commit_ends_the_composition() {
        let mut host = host();
        host.apply(GuiEvent::Composition("にほ".into()));
        host.apply(GuiEvent::Text("日本".into()));
        assert_eq!(host.input.text, "日本");
        assert!(host.input.composition.is_empty());
    }

    #[test]
    fn the_stick_and_the_dpad_share_the_repeat() {
        let mut gamepad = GamepadNavigation::default();
        let now = Instant::now();
        gamepad.button(GamepadButton::DpadRight, true, now);
        gamepad.axis(GamepadAxis::LeftX, 30000, now);
        gamepad.axis(GamepadAxis::LeftX, 0, now);
        assert!(gamepad.poll(now + REPEAT_DELAY).is_some());
        gamepad.button(GamepadButton::DpadRight, false, now);
        assert_eq!(gamepad.poll(now + Duration::from_secs(5)), None);
    }
}
