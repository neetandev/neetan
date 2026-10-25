//! The host of the graphical user interface: the font, the theme, the
//! catalog of the language, the widget state, the input of the next pass and
//! the scene of the last pass, and the mapping of the SDL events to the
//! input.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use common::{info, warn};
use gui::{
    Fonts, Scene,
    ui::{
        ActionButton, Input, Navigation, Shortcut, Theme, ThemeKind, UiContext, UiState, Viewport,
    },
};
use sdl3::{
    event::Event,
    gamepad::{GamepadAxis, GamepadButton},
    keyboard::{Mod, Scancode},
    mouse::MouseButton,
    video::Window,
};
use sdl3_backend::Overlay;

use crate::{config::GuiTheme, texts::LanguageCatalog};

/// The delay before a held gamepad direction repeats.
const REPEAT_DELAY: Duration = Duration::from_millis(400);

/// The interval between the repeats of a held gamepad direction.
const REPEAT_INTERVAL: Duration = Duration::from_millis(80);

/// The most passes in a row that the widgets ask for without an event in
/// between. A widget that asks on every pass stops there.
const MAXIMUM_CHAINED_PASSES: u32 = 4;

/// The stick deflection that engages a direction.
const STICK_PRESS: i16 = 16384;

/// The stick deflection below which an engaged direction releases.
const STICK_RELEASE: i16 = 11000;

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
    /// A gamepad button for an action of the screen: X, Y or Start.
    Action(ActionButton),
    /// A shortcut key.
    Shortcut(Shortcut),
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
/// navigation keys and backspace. Left Ctrl + Page Up and Left Ctrl + Page
/// Down switch the tab like the shoulder buttons. The right control key
/// belongs to the shell and never forms a shortcut.
pub(crate) fn key_event(scancode: Scancode, keymod: &Mod, repeat: bool) -> Option<GuiEvent> {
    let control = keymod.lctrl();
    let navigation = match scancode {
        Scancode::Up => Some(Navigation::Up),
        Scancode::Down => Some(Navigation::Down),
        Scancode::Left => Some(Navigation::Left),
        Scancode::Right => Some(Navigation::Right),
        Scancode::PageUp if control => Some(Navigation::PreviousTab),
        Scancode::PageDown if control => Some(Navigation::NextTab),
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
    if scancode == Scancode::Backspace {
        return Some(GuiEvent::Backspace);
    }
    if repeat || keymod.rctrl() {
        return None;
    }
    let shortcut = match scancode {
        Scancode::Return | Scancode::KpEnter => return Some(GuiEvent::Confirm),
        Scancode::Escape => return Some(GuiEvent::Cancel),
        Scancode::Delete => Shortcut::Delete,
        Scancode::Insert => Shortcut::Insert,
        Scancode::F2 => Shortcut::F2,
        Scancode::F5 => Shortcut::F5,
        Scancode::F10 => Shortcut::F10,
        Scancode::F if control => Shortcut::ControlF,
        Scancode::D if control => Shortcut::ControlD,
        Scancode::S if control => Shortcut::ControlS,
        _ => return None,
    };
    Some(GuiEvent::Shortcut(shortcut))
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
/// synthesized repeat while a direction is held, and the face, shoulder and
/// start buttons.
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

    /// The event of a button change at `now`. A confirms, B cancels, the
    /// shoulder buttons switch tabs, and X, Y and Start are action buttons.
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
            GamepadButton::West => Some(GuiEvent::Action(ActionButton::X)),
            GamepadButton::North => Some(GuiEvent::Action(ActionButton::Y)),
            GamepadButton::Start => Some(GuiEvent::Action(ActionButton::Start)),
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

    /// The time of the next repeat of the held direction, or `None` while no
    /// direction is held.
    pub(crate) fn next_repeat(&self) -> Option<Instant> {
        self.repeat.map(|repeat| repeat.next)
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

/// The theme kind of the GUI theme setting `theme`.
pub(crate) fn theme_kind(theme: GuiTheme) -> ThemeKind {
    match theme {
        GuiTheme::Dark => ThemeKind::Dark,
        GuiTheme::Light => ThemeKind::Light,
    }
}

/// The host of the graphical user interface.
pub(crate) struct GuiHost {
    /// The font and the icon paths.
    fonts: Fonts,
    /// The look of the widgets.
    theme: Theme,
    /// The texts of the selected language.
    catalog: LanguageCatalog,
    /// The generation of the catalog. Each switch of the catalog counts it up.
    catalog_generation: u64,
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
    /// Whether pointer motion, wheel steps, a window change or a widget wait
    /// for a pass.
    pending: bool,
    /// The passes in a row that the widgets asked for.
    chained_passes: u32,
    /// Whether SDL text input runs.
    text_input: bool,
    /// The text input area and the cursor offset last passed to SDL, in
    /// window coordinates.
    text_area: Option<[i32; 5]>,
    /// The gamepad navigation.
    pub(crate) gamepad: GamepadNavigation,
}

impl GuiHost {
    /// Loads the first font file of `paths` that exists and builds the theme
    /// `kind` with the texts of `catalog`. Returns a message for the user
    /// when no file exists or the file does not parse.
    pub(crate) fn load(
        paths: &[PathBuf],
        kind: ThemeKind,
        catalog: LanguageCatalog,
    ) -> Result<Self, String> {
        let Some(path) = paths.iter().find(|path| path.is_file()) else {
            let searched: Vec<String> = paths
                .iter()
                .map(|path| path.display().to_string())
                .collect();
            return Err(format!(
                "the GUI font {} was not found. The gui-font setting selects the font file",
                searched.join(" or ")
            ));
        };
        let data = std::fs::read(path).map_err(|error| {
            format!("the GUI font {} could not be read: {error}", path.display())
        })?;
        let host = Self::new(data, kind, catalog).map_err(|error| {
            format!(
                "the GUI font {} could not be loaded: {error}",
                path.display()
            )
        })?;
        info!("GUI font: {}", path.display());
        Ok(host)
    }

    /// A host with the font file `data`, the theme `kind` and the texts of
    /// `catalog`.
    pub(crate) fn new(
        data: Vec<u8>,
        kind: ThemeKind,
        catalog: LanguageCatalog,
    ) -> Result<Self, gui::truetype::FontError> {
        let mut fonts = Fonts::new();
        let font = fonts.add(data)?;
        let theme = Theme::new(&mut fonts, font, kind);
        Ok(Self {
            fonts,
            theme,
            catalog,
            catalog_generation: 0,
            state: UiState::default(),
            input: Input::default(),
            scene: Scene::new(),
            viewport: Viewport::new(0, 0, 1.0),
            pixel_density: 1.0,
            pending: true,
            chained_passes: 0,
            text_input: false,
            text_area: None,
            gamepad: GamepadNavigation::default(),
        })
    }

    /// Replaces the catalog with `catalog` and counts the catalog generation
    /// up. The next pass draws with the new texts.
    pub(crate) fn set_catalog(&mut self, catalog: LanguageCatalog) {
        self.catalog = catalog;
        self.catalog_generation += 1;
        self.pending = true;
    }

    /// The texts of the selected language.
    #[cfg(test)]
    pub(crate) fn catalog(&self) -> &i18n::Catalog {
        self.catalog.catalog()
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

    /// Opens the modal layer `id` before the next pass, for a dialog that
    /// opens between passes, and waits for a pass.
    pub(crate) fn open_layer(&mut self, id: gui::ui::UiId) {
        self.state.open_layer(id);
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
        self.chained_passes = 0;
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
            GuiEvent::Action(button) => input.action = Some(button),
            GuiEvent::Shortcut(shortcut) => input.shortcut = Some(shortcut),
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

    /// Whether pointer motion, wheel steps, a window change or a widget that
    /// asked for another pass wait for a pass.
    pub(crate) fn pending(&self) -> bool {
        self.pending
    }

    /// Runs one pass of `draw` with the input so far and keeps its scene. A
    /// widget that asks for another pass leaves the host pending, up to
    /// [`MAXIMUM_CHAINED_PASSES`] passes in a row without an event.
    pub(crate) fn pass<R>(&mut self, draw: impl FnOnce(&mut UiContext) -> R) -> R {
        let mut context = UiContext::new(
            &mut self.scene,
            &mut self.fonts,
            &self.theme,
            self.catalog.catalog(),
            &self.input,
            &mut self.state,
            self.viewport,
        )
        .with_catalog_generation(self.catalog_generation);
        let result = draw(&mut context);
        context.finish();
        self.input.advance();
        let requested = self.state.take_pass_request();
        if requested && self.chained_passes < MAXIMUM_CHAINED_PASSES {
            self.chained_passes += 1;
            self.pending = true;
        } else {
            self.chained_passes = 0;
            self.pending = false;
        }
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
        GuiHost::new(
            crate::gui_test_support::repository_font(),
            ThemeKind::Dark,
            LanguageCatalog::English,
        )
        .expect("the repository font parses")
    }

    /// Verifies that a missing font file gives the path and the setting that selects it.
    #[test]
    fn a_missing_font_mentions_its_path_and_the_setting() {
        let paths = [PathBuf::from("/nonexistent/neetan/font.ttf")];
        let Err(message) = GuiHost::load(&paths, ThemeKind::Dark, LanguageCatalog::English) else {
            panic!("a missing font does not load");
        };
        assert_eq!(
            message,
            "the GUI font /nonexistent/neetan/font.ttf was not found. The gui-font setting selects the font file"
        );
    }

    /// Verifies that a missing font gives every searched path.
    #[test]
    fn a_missing_font_mentions_every_searched_path() {
        let paths = [
            PathBuf::from("/nonexistent/root/font.ttf"),
            PathBuf::from("/nonexistent/share/font.ttf"),
        ];
        let Err(message) = GuiHost::load(&paths, ThemeKind::Dark, LanguageCatalog::English) else {
            panic!("a missing font does not load");
        };
        assert_eq!(
            message,
            "the GUI font /nonexistent/root/font.ttf or /nonexistent/share/font.ttf was not found. The gui-font setting selects the font file"
        );
    }

    /// Verifies that each GUI theme setting selects its theme kind.
    #[test]
    fn the_gui_theme_selects_the_theme_kind() {
        assert_eq!(theme_kind(GuiTheme::Dark), ThemeKind::Dark);
        assert_eq!(theme_kind(GuiTheme::Light), ThemeKind::Light);
        let host = GuiHost::new(
            crate::gui_test_support::repository_font(),
            ThemeKind::Light,
            LanguageCatalog::English,
        )
        .expect("the repository font parses");
        assert_eq!(host.theme.colors, gui::ui::Palette::of(ThemeKind::Light));
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
        assert_eq!(
            gamepad.button(GamepadButton::West, true, now),
            Some(GuiEvent::Action(ActionButton::X))
        );
        assert_eq!(
            gamepad.button(GamepadButton::North, true, now),
            Some(GuiEvent::Action(ActionButton::Y))
        );
        assert_eq!(
            gamepad.button(GamepadButton::Start, true, now),
            Some(GuiEvent::Action(ActionButton::Start))
        );
        assert_eq!(gamepad.button(GamepadButton::Start, false, now), None);
        assert_eq!(gamepad.button(GamepadButton::Back, true, now), None);
    }

    /// Verifies that an action button reaches the input of the next pass and needs a pass of its own.
    #[test]
    fn an_action_button_reaches_the_input() {
        let mut host = host();
        assert!(host.apply(GuiEvent::Action(ActionButton::Y)));
        assert_eq!(host.input.action, Some(ActionButton::Y));
        let seen = host.pass(|context| context.action_button());
        assert_eq!(seen, Some(ActionButton::Y));
        assert_eq!(host.input.action, None);
    }

    /// Verifies that a switch of the catalog counts the generation up and reaches the next pass.
    #[test]
    fn a_catalog_switch_reaches_the_next_pass() {
        let mut host = host();
        host.pass(|_| ());
        assert!(!host.pending());
        assert_eq!(
            host.pass(|context| context.catalog_generation),
            0,
            "the first catalog has the generation 0"
        );
        let german = crate::texts::test_catalog("[common]\nsave = Speichern\n");
        host.set_catalog(LanguageCatalog::Loaded(Box::new(german)));
        assert!(host.pending());
        assert_eq!(host.catalog().name(), "Deutsch");
        let (generation, save) = host.pass(|context| {
            (
                context.catalog_generation,
                context.catalog.text(crate::texts::common::SAVE).to_owned(),
            )
        });
        assert_eq!(generation, 1);
        assert_eq!(save, "Speichern");
    }

    /// Verifies that a widget that asks for another pass leaves the host pending.
    #[test]
    fn a_pass_request_leaves_the_host_pending() {
        let mut host = host();
        host.pass(|context| context.request_pass());
        assert!(host.pending());
        host.pass(|_| ());
        assert!(!host.pending());
    }

    /// Verifies that a widget that asks on every pass stops after the bound of chained passes.
    #[test]
    fn chained_pass_requests_are_bounded() {
        let mut host = host();
        let mut passes = 0;
        while host.pending() && passes < 100 {
            host.pass(|context| context.request_pass());
            passes += 1;
        }
        assert_eq!(passes, MAXIMUM_CHAINED_PASSES + 1);
        host.apply(GuiEvent::Confirm);
        host.pass(|context| context.request_pass());
        assert!(host.pending());
    }

    /// Verifies the shortcut keys, the left control key and that the right control key forms no shortcut.
    #[test]
    fn shortcut_keys_map_to_shortcuts() {
        const LEFT_CONTROL: Mod = Mod(0x0040);
        const RIGHT_CONTROL: Mod = Mod(0x0080);
        let key = |scancode, keymod: &Mod| key_event(scancode, keymod, false);
        assert_eq!(
            key(Scancode::Delete, &NO_MODIFIERS),
            Some(GuiEvent::Shortcut(Shortcut::Delete))
        );
        assert_eq!(
            key(Scancode::Insert, &NO_MODIFIERS),
            Some(GuiEvent::Shortcut(Shortcut::Insert))
        );
        assert_eq!(
            key(Scancode::F2, &NO_MODIFIERS),
            Some(GuiEvent::Shortcut(Shortcut::F2))
        );
        assert_eq!(
            key(Scancode::F5, &NO_MODIFIERS),
            Some(GuiEvent::Shortcut(Shortcut::F5))
        );
        assert_eq!(
            key(Scancode::F10, &NO_MODIFIERS),
            Some(GuiEvent::Shortcut(Shortcut::F10))
        );
        assert_eq!(
            key(Scancode::F, &LEFT_CONTROL),
            Some(GuiEvent::Shortcut(Shortcut::ControlF))
        );
        assert_eq!(
            key(Scancode::D, &LEFT_CONTROL),
            Some(GuiEvent::Shortcut(Shortcut::ControlD))
        );
        assert_eq!(
            key(Scancode::S, &LEFT_CONTROL),
            Some(GuiEvent::Shortcut(Shortcut::ControlS))
        );
        assert_eq!(key(Scancode::S, &NO_MODIFIERS), None);
        assert_eq!(key(Scancode::S, &RIGHT_CONTROL), None);
        assert_eq!(key(Scancode::Delete, &RIGHT_CONTROL), None);
        assert_eq!(key_event(Scancode::Delete, &NO_MODIFIERS, true), None);
        assert_eq!(
            key(Scancode::PageUp, &LEFT_CONTROL),
            Some(GuiEvent::Navigation(Navigation::PreviousTab))
        );
        assert_eq!(
            key(Scancode::PageDown, &LEFT_CONTROL),
            Some(GuiEvent::Navigation(Navigation::NextTab))
        );
        assert_eq!(
            key(Scancode::PageDown, &RIGHT_CONTROL),
            Some(GuiEvent::Navigation(Navigation::PageDown))
        );
    }

    /// Verifies that a shortcut reaches the input and that a text field holds back a shortcut without a modifier.
    #[test]
    fn a_shortcut_reaches_the_input_unless_a_text_field_holds_the_focus() {
        let mut host = host();
        host.set_window_metrics(800, 600, 1.0, 1.0);
        let field = gui::Rect::new(0.0, 0.0, 200.0, 32.0);
        let draw = |context: &mut UiContext, focus_field: bool| {
            let mut text = String::new();
            if focus_field {
                context.set_focus(gui::ui::UiId::from("field"));
            }
            gui::ui::text_field(context, "field", field, &mut text, "Filter");
            context.shortcut()
        };
        host.apply(GuiEvent::Shortcut(Shortcut::F2));
        assert_eq!(
            host.pass(|context| draw(context, false)),
            Some(Shortcut::F2)
        );
        host.pass(|context| draw(context, true));
        host.apply(GuiEvent::Shortcut(Shortcut::F2));
        assert_eq!(host.pass(|context| draw(context, true)), None);
        host.apply(GuiEvent::Shortcut(Shortcut::ControlF));
        assert_eq!(
            host.pass(|context| draw(context, true)),
            Some(Shortcut::ControlF)
        );
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
        assert_eq!(gamepad.next_repeat(), Some(first + REPEAT_INTERVAL * 2));
        assert_eq!(gamepad.button(GamepadButton::DpadDown, false, first), None);
        assert_eq!(gamepad.next_repeat(), None);
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
