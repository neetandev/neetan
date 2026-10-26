//! The host of the graphical user interface: the font, the theme, the
//! catalog of the language, the widget state, the input of the next pass and
//! the scene of the last pass.

mod gamepad;
pub(crate) mod keys;

use std::path::PathBuf;

use common::{info, warn};
use gamepad::GamepadNavigation;
use gui::{
    Fonts, Scene,
    truetype::FontError,
    ui::{
        ActionButton, EditCommand, EditModifiers, Input, Navigation, NavigationSource, Shortcut,
        Theme, ThemeKind, UiContext, UiId, UiState, Viewport,
    },
};
#[cfg(test)]
use i18n::Catalog;
use keys::key_event;
use sdl3::{
    Error, clipboard,
    keyboard::{Mod, Scancode},
    mouse::MouseButton,
    video::Window,
};
use sdl3_backend::Overlay;

use crate::{config::GuiTheme, texts::LanguageCatalog};

/// The most passes in a row that the widgets ask for without an event in
/// between. A widget that asks on every pass stops there.
const MAXIMUM_CHAINED_PASSES: u32 = 4;

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
    /// A navigation key with the modifier keys held with it.
    KeyNavigation(Navigation, EditModifiers),
    /// A gamepad direction or shoulder button.
    GamepadNavigation(Navigation),
    /// An edit command of the focused text widget: select all, copy, cut or
    /// delete.
    Edit(EditCommand),
    /// A paste of the clipboard text into the focused text widget.
    Paste(String),
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
    ) -> Result<Self, FontError> {
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
    pub(crate) fn catalog(&self) -> &Catalog {
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
    pub(crate) fn open_layer(&mut self, id: UiId) {
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
            GuiEvent::KeyNavigation(navigation, modifiers) => {
                input.navigation = Some(navigation);
                input.navigation_source = NavigationSource::Keyboard;
                input.modifiers = modifiers;
            }
            GuiEvent::GamepadNavigation(navigation) => {
                input.navigation = Some(navigation);
                input.navigation_source = NavigationSource::Gamepad;
                input.modifiers = EditModifiers::default();
            }
            GuiEvent::Edit(command) => input.edit = Some(command),
            GuiEvent::Confirm => input.confirm = true,
            GuiEvent::Cancel => input.cancel = true,
            GuiEvent::Action(button) => input.action = Some(button),
            GuiEvent::Shortcut(shortcut) => input.shortcut = Some(shortcut),
            GuiEvent::Backspace => input.backspace = true,
            GuiEvent::Paste(text) => input.paste.push_str(&text),
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

    /// The event of the pressed key `scancode` with the held keys `keymod`.
    /// The paste key reads the clipboard of the system only while a text
    /// widget wants text input.
    pub(crate) fn key_event(
        &self,
        scancode: Scancode,
        keymod: &Mod,
        repeat: bool,
    ) -> Option<GuiEvent> {
        self.key_event_with_clipboard(scancode, keymod, repeat, clipboard::text)
    }

    /// The event of a pressed key like [`GuiHost::key_event`] with the
    /// clipboard read `read_clipboard`.
    fn key_event_with_clipboard(
        &self,
        scancode: Scancode,
        keymod: &Mod,
        repeat: bool,
        read_clipboard: impl FnOnce() -> Result<Option<String>, Error>,
    ) -> Option<GuiEvent> {
        key_event(
            scancode,
            keymod,
            repeat,
            self.state.wants_text_input(),
            read_clipboard,
        )
    }

    /// Whether pointer motion, wheel steps, a window change or a widget that
    /// asked for another pass wait for a pass.
    pub(crate) fn pending(&self) -> bool {
        self.pending
    }

    /// Runs one pass of `draw` with the input so far and keeps its scene. A
    /// widget that asks for another pass leaves the host pending, up to
    /// [`MAXIMUM_CHAINED_PASSES`] passes in a row without an event. The text
    /// of a copy or a cut in the pass goes to the clipboard of the system.
    pub(crate) fn pass<R>(&mut self, draw: impl FnOnce(&mut UiContext) -> R) -> R {
        self.pass_with_clipboard(draw, clipboard::set_text)
    }

    /// Runs one pass of `draw` like [`GuiHost::pass`] and hands the text of
    /// a copy or a cut in the pass to `write_clipboard`. A failed write logs
    /// a warning.
    fn pass_with_clipboard<R>(
        &mut self,
        draw: impl FnOnce(&mut UiContext) -> R,
        write_clipboard: impl FnOnce(&str) -> Result<(), Error>,
    ) -> R {
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
        if let Some(copied) = self.state.take_copied()
            && let Err(error) = write_clipboard(copied)
        {
            warn!("Failed to write the clipboard: {error}");
        }
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
    use gui::{
        Rect,
        ui::{Palette, text_field},
    };

    use super::{
        keys::tests::{
            COMMAND, LEFT_CONTROL, NO_EDIT_MODIFIERS, NO_MODIFIERS, WORD, clipboard_with,
            unread_clipboard,
        },
        *,
    };
    use crate::{
        gui_test_support::repository_font,
        texts::{common::SAVE, test_catalog},
    };

    /// A host with the font of the repository.
    fn host() -> GuiHost {
        GuiHost::new(repository_font(), ThemeKind::Dark, LanguageCatalog::English)
            .expect("the repository font parses")
    }

    /// The area of the text field of the host tests.
    const FIELD: Rect = Rect::new(0.0, 0.0, 200.0, 32.0);

    /// The event of the pressed key `scancode` with the held keys `keymod`
    /// in `host`. The clipboard read fails the test.
    fn key(host: &GuiHost, scancode: Scancode, keymod: &Mod, repeat: bool) -> Option<GuiEvent> {
        host.key_event_with_clipboard(scancode, keymod, repeat, unread_clipboard)
    }

    /// Applies `event` and runs a pass of the text field over `text` that
    /// writes the copied text into `written`. Returns whether the text
    /// changed and the shortcut of the pass.
    fn clipboard_pass(
        host: &mut GuiHost,
        event: Option<GuiEvent>,
        text: &mut String,
        written: &mut Vec<String>,
    ) -> (bool, Option<Shortcut>) {
        if let Some(event) = event {
            host.apply(event);
        }
        host.pass_with_clipboard(
            |context| {
                let changed = text_field(context, "filter", FIELD, text, "Filter").changed;
                (changed, context.shortcut())
            },
            |copied| {
                written.push(copied.to_owned());
                Ok(())
            },
        )
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
            repository_font(),
            ThemeKind::Light,
            LanguageCatalog::English,
        )
        .expect("the repository font parses");
        assert_eq!(host.theme.colors, Palette::of(ThemeKind::Light));
    }

    /// Verifies that pointer positions scale to physical pixels and that wheel steps add up
    /// until the next pass.
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

    /// Verifies that only a change of the window metrics waits for a pass.
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
        let german = test_catalog("[common]\nsave = Speichern\n");
        host.set_catalog(LanguageCatalog::Loaded(Box::new(german)));
        assert!(host.pending());
        assert_eq!(host.catalog().name(), "Deutsch");
        let (generation, save) = host.pass(|context| {
            (
                context.catalog_generation,
                context.catalog.text(SAVE).to_owned(),
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

    /// Verifies that a key navigation reaches the input with its modifiers from the keyboard and
    /// that a gamepad navigation reaches it from the gamepad without modifiers.
    #[test]
    fn navigation_events_reach_the_input_with_their_source() {
        let mut host = host();
        host.set_window_metrics(800, 600, 1.0, 1.0);
        let read = |context: &mut UiContext| {
            (
                context.input.navigation,
                context.input.navigation_source,
                context.input.modifiers,
            )
        };
        host.apply(GuiEvent::KeyNavigation(Navigation::Left, WORD));
        assert_eq!(
            host.pass(read),
            (Some(Navigation::Left), NavigationSource::Keyboard, WORD)
        );
        host.apply(GuiEvent::GamepadNavigation(Navigation::Right));
        assert_eq!(
            host.pass(read),
            (
                Some(Navigation::Right),
                NavigationSource::Gamepad,
                NO_EDIT_MODIFIERS
            )
        );
        assert_eq!(
            host.pass(read),
            (None, NavigationSource::Keyboard, NO_EDIT_MODIFIERS)
        );
    }

    /// Verifies that delete removes the character after the cursor of a focused text field and
    /// holds back the delete shortcut, and that it reaches the screen as the delete shortcut while
    /// no text widget holds the focus.
    #[test]
    fn delete_edits_a_focused_text_field_and_is_the_delete_shortcut_elsewhere() {
        let mut host = host();
        host.set_window_metrics(800, 600, 1.0, 1.0);
        let mut text = String::from("abc");
        let field = Rect::new(0.0, 0.0, 200.0, 32.0);
        let mut draw = |host: &mut GuiHost, event: Option<GuiEvent>| {
            if let Some(event) = event {
                host.apply(event);
            }
            host.pass(|context| {
                let changed = text_field(context, "filter", field, &mut text, "Filter").changed;
                (changed, context.shortcut())
            })
        };
        let delete = |host: &GuiHost| key(host, Scancode::Delete, &NO_MODIFIERS, false);
        let event = delete(&host);
        assert_eq!(draw(&mut host, event), (false, Some(Shortcut::Delete)));
        host.state.set_focus(UiId::from("filter"));
        draw(&mut host, None);
        let event = key(&host, Scancode::Home, &NO_MODIFIERS, false);
        draw(&mut host, event);
        let event = delete(&host);
        assert_eq!(draw(&mut host, event), (true, None));
        let event = key(&host, Scancode::Delete, &NO_MODIFIERS, true);
        assert_eq!(draw(&mut host, event), (true, None));
        let event = key(&host, Scancode::A, &LEFT_CONTROL, false);
        draw(&mut host, event);
        let event = delete(&host);
        assert_eq!(draw(&mut host, event), (true, None));
        host.state.clear_focus();
        let event = delete(&host);
        assert_eq!(draw(&mut host, event), (false, Some(Shortcut::Delete)));
        assert_eq!(text, "");
    }

    /// Verifies that a copy and a cut of a focused text field reach the clipboard once after
    /// their pass, that a failed write leaves the next pass quiet, and that a paste key inserts
    /// the first line of the clipboard.
    #[test]
    fn copied_text_reaches_the_clipboard_after_the_pass() {
        let mut host = host();
        host.set_window_metrics(800, 600, 1.0, 1.0);
        let mut text = String::from("disk image");
        let mut written = Vec::new();
        host.state.set_focus(UiId::from("filter"));
        clipboard_pass(&mut host, None, &mut text, &mut written);
        let copy = |host: &GuiHost| key(host, Scancode::C, &COMMAND, false);
        let select_all = |host: &GuiHost| key(host, Scancode::A, &COMMAND, false);
        let event = copy(&host);
        clipboard_pass(&mut host, event, &mut text, &mut written);
        assert!(written.is_empty());
        let event = select_all(&host);
        clipboard_pass(&mut host, event, &mut text, &mut written);
        let event = copy(&host);
        assert_eq!(
            clipboard_pass(&mut host, event, &mut text, &mut written),
            (false, None)
        );
        clipboard_pass(&mut host, None, &mut text, &mut written);
        assert_eq!(written, ["disk image"]);

        let cut = key(&host, Scancode::X, &COMMAND, false);
        assert_eq!(
            clipboard_pass(&mut host, cut, &mut text, &mut written),
            (true, None)
        );
        assert_eq!(written, ["disk image", "disk image"]);
        assert!(text.is_empty());

        let paste = host.key_event_with_clipboard(
            Scancode::V,
            &COMMAND,
            false,
            clipboard_with("disk\nimage"),
        );
        assert_eq!(
            clipboard_pass(&mut host, paste, &mut text, &mut written),
            (true, None)
        );
        assert_eq!(text, "disk");

        let event = select_all(&host);
        clipboard_pass(&mut host, event, &mut text, &mut written);
        host.apply(GuiEvent::Edit(EditCommand::Copy));
        host.pass_with_clipboard(
            |context| text_field(context, "filter", FIELD, &mut text, "Filter"),
            |_| Err(Error("no video".into())),
        );
        clipboard_pass(&mut host, None, &mut text, &mut written);
        assert_eq!(written.len(), 2);
    }

    /// Verifies that copy, cut and paste reach no shortcut of the screen and that the shortcuts
    /// with the left control key reach it inside and outside a focused text field.
    #[test]
    fn control_shortcuts_work_beside_the_clipboard_keys() {
        let mut host = host();
        host.set_window_metrics(800, 600, 1.0, 1.0);
        let mut text = String::from("disk");
        let mut written = Vec::new();
        let shortcuts = [
            (Scancode::F, Shortcut::ControlF),
            (Scancode::D, Shortcut::ControlD),
            (Scancode::S, Shortcut::ControlS),
        ];
        for focused in [false, true] {
            if focused {
                host.state.set_focus(UiId::from("filter"));
                clipboard_pass(&mut host, None, &mut text, &mut written);
            }
            for (scancode, shortcut) in shortcuts {
                let event = key(&host, scancode, &LEFT_CONTROL, false);
                assert_eq!(
                    clipboard_pass(&mut host, event, &mut text, &mut written).1,
                    Some(shortcut)
                );
            }
            for scancode in [Scancode::A, Scancode::C, Scancode::X] {
                let event = key(&host, scancode, &COMMAND, false);
                assert_eq!(
                    clipboard_pass(&mut host, event, &mut text, &mut written).1,
                    None
                );
            }
            let event = if focused {
                host.key_event_with_clipboard(Scancode::V, &COMMAND, false, clipboard_with("x"))
            } else {
                key(&host, Scancode::V, &COMMAND, false)
            };
            assert_eq!(
                clipboard_pass(&mut host, event, &mut text, &mut written).1,
                None
            );
        }
        assert_eq!(text, "x");
        assert_eq!(written, ["disk", "disk"]);
    }

    /// Verifies that a shortcut reaches the input and that a text field holds back a shortcut without a modifier.
    #[test]
    fn a_shortcut_reaches_the_input_unless_a_text_field_holds_the_focus() {
        let mut host = host();
        host.set_window_metrics(800, 600, 1.0, 1.0);
        let field = Rect::new(0.0, 0.0, 200.0, 32.0);
        let draw = |context: &mut UiContext, focus_field: bool| {
            let mut text = String::new();
            if focus_field {
                context.set_focus(UiId::from("field"));
            }
            text_field(context, "field", field, &mut text, "Filter");
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

    /// Verifies that committed text ends the composition of an input method.
    #[test]
    fn a_commit_ends_the_composition() {
        let mut host = host();
        host.apply(GuiEvent::Composition("にほ".into()));
        host.apply(GuiEvent::Text("日本".into()));
        assert_eq!(host.input.text, "日本");
        assert!(host.input.composition.is_empty());
    }

    /// Verifies that the paste key reads the clipboard only while a focused text field wants
    /// text input.
    #[test]
    fn an_unfocused_paste_key_reads_no_clipboard() {
        let mut host = host();
        host.set_window_metrics(800, 600, 1.0, 1.0);
        let mut text = String::new();
        let mut written = Vec::new();
        clipboard_pass(&mut host, None, &mut text, &mut written);
        assert_eq!(key(&host, Scancode::V, &COMMAND, false), None);
        assert_eq!(key(&host, Scancode::V, &COMMAND, true), None);
        host.state.set_focus(UiId::from("filter"));
        clipboard_pass(&mut host, None, &mut text, &mut written);
        assert_eq!(
            host.key_event_with_clipboard(Scancode::V, &COMMAND, false, clipboard_with("disk")),
            Some(GuiEvent::Paste("disk".into()))
        );
        host.state.clear_focus();
        clipboard_pass(&mut host, None, &mut text, &mut written);
        assert_eq!(key(&host, Scancode::V, &COMMAND, false), None);
        assert!(text.is_empty());
    }
}
