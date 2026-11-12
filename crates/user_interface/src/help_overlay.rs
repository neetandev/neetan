//! The help overlay: the hotkeys of the running machine, each with its key
//! caps and its label.

use std::convert::Infallible;

use gui::{
    Rect,
    component::{Component, Layer, Status, Update},
    ui::{
        Align, Navigation, NavigationSet, ScrollState, Truncation, UiContext, UiId, keycap,
        keycap_width, label, modal_in, scroll_area,
    },
};

use crate::{
    hotkeys::{Hotkey, HotkeyList, MODIFIER},
    texts,
};

/// The size of the overlay panel in logical units: the header and one row
/// for each hotkey.
const PANEL_SIZE: (f32, f32) = (560.0, 500.0);

/// The id of the modal layer of the overlay.
const HELP_LAYER_ID: &str = "help";

/// The sign between the two key caps of a hotkey.
const PLUS: &str = "+";

/// The id of the list of the hotkeys.
const LIST_ID: &str = "hotkeys";

/// The navigation events that scroll the list.
const SCROLL_KEYS: NavigationSet = NavigationSet::of(&[
    Navigation::Up,
    Navigation::Down,
    Navigation::PageUp,
    Navigation::PageDown,
    Navigation::Home,
    Navigation::End,
]);

/// A request of the user to the overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HelpMessage {
    /// The close button or Esc closes the overlay.
    Close,
}

/// The state of the open help overlay.
pub(crate) struct HelpOverlay {
    /// The hotkeys of the list.
    hotkeys: HotkeyList,
    /// The scroll position of the list.
    scroll: ScrollState,
    /// The message buffer.
    messages: Vec<HelpMessage>,
}

impl HelpOverlay {
    /// An overlay that lists `hotkeys` from the top.
    pub(crate) fn new(hotkeys: HotkeyList) -> Self {
        Self {
            hotkeys,
            scroll: ScrollState::new(),
            messages: Vec::new(),
        }
    }
}

/// The width of the plus sign between the key caps with its gap on both
/// sides.
fn plus_width(context: &mut UiContext) -> f32 {
    let style = context.theme.dim();
    context.measure(&style, PLUS) + context.theme.gap
}

/// The width of the two key caps of `hotkey` with the plus sign between
/// them.
fn keys_width(context: &mut UiContext, hotkey: Hotkey) -> f32 {
    keycap_width(context, MODIFIER) + plus_width(context) + keycap_width(context, hotkey.key())
}

/// The width of the key column: the widest key caps of `hotkeys`.
fn key_column_width(context: &mut UiContext, hotkeys: &[Hotkey]) -> f32 {
    hotkeys
        .iter()
        .map(|&hotkey| keys_width(context, hotkey))
        .fold(0.0, f32::max)
}

/// The scroll offset of `offset` after `navigation` in a view of
/// `view_height` with rows of `row_height`.
fn scrolled(offset: f32, navigation: Navigation, view_height: f32, row_height: f32) -> f32 {
    let page = (view_height - row_height).max(row_height);
    match navigation {
        Navigation::Up => offset - row_height,
        Navigation::Down => offset + row_height,
        Navigation::PageUp => offset - page,
        Navigation::PageDown => offset + page,
        Navigation::Home => 0.0,
        Navigation::End => f32::MAX,
        Navigation::Left
        | Navigation::Right
        | Navigation::Next
        | Navigation::Previous
        | Navigation::PreviousTab
        | Navigation::NextTab => offset,
    }
}

impl Component for HelpOverlay {
    type Message = HelpMessage;
    type Output = Infallible;
    type Effect = Infallible;
    type Input<'a> = ();

    const LAYER: Layer = Layer::Modal(HELP_LAYER_ID);

    fn messages(&mut self) -> &mut Vec<HelpMessage> {
        &mut self.messages
    }

    /// Draws the panel centered in `area` with one row for each hotkey: the
    /// key caps joined by a plus sign in the key column and the label after
    /// them.
    fn view(
        &mut self,
        context: &mut UiContext,
        area: Rect,
        _input: (),
        messages: &mut Vec<HelpMessage>,
    ) {
        let theme = context.theme;
        let catalog = context.catalog;
        let response = modal_in(
            context,
            HELP_LAYER_ID,
            area,
            PANEL_SIZE,
            catalog.text(texts::hotkeys::TITLE),
        );
        if response.closed {
            messages.push(HelpMessage::Close);
        }
        let view = response.content.cut_top(theme.gap * 0.5).1;
        let id = UiId::from(LIST_ID);
        context.default_focus(id);
        context.focusable(id, view, SCROLL_KEYS);
        if let Some(navigation) = context.navigation(id) {
            self.scroll.offset = scrolled(
                self.scroll.offset,
                navigation,
                view.height,
                theme.row_height,
            );
            context.request_pass();
        }

        let hotkeys = &self.hotkeys;
        let key_column = key_column_width(context, hotkeys);
        let label_offset = key_column + theme.gap * 2.0;
        let plus = plus_width(context);
        let plus_style = theme.dim();
        let style = theme.body();
        scroll_area(
            context,
            id.child("scroll"),
            view,
            &mut self.scroll,
            |context, content| {
                for (index, &hotkey) in hotkeys.iter().enumerate() {
                    let row = Rect::new(
                        content.x,
                        content.y + index as f32 * theme.row_height,
                        content.width,
                        theme.row_height,
                    );
                    let x = keycap(context, row.x, row, MODIFIER);
                    let plus_area = Rect::new(x, row.y, plus, row.height);
                    context.text_centered(plus_area, &plus_style, PLUS);
                    keycap(context, x + plus, row, hotkey.key());
                    let label_area = row.cut_left(label_offset.min(row.width)).1;
                    label(
                        context,
                        label_area,
                        &style,
                        catalog.text(hotkey.label()),
                        Align::Start,
                        Truncation::End,
                    );
                }
                hotkeys.len() as f32 * theme.row_height
            },
        );
    }

    fn update(
        &mut self,
        message: HelpMessage,
        _update: &mut Update<'_, Infallible>,
    ) -> Status<Infallible> {
        match message {
            HelpMessage::Close => Status::Closed,
        }
    }
}

#[cfg(test)]
mod tests {
    use gui::{
        Instance,
        ui::{HEADER_HEIGHT, KEYCAP_HEIGHT, ThemeKind, Viewport},
    };

    use super::*;
    use crate::{
        gui_test_support::GuiHarness,
        host::SESSION_MINIMUM_WINDOW_SIZE,
        hotkeys::HotkeyFeatures,
        media_dialog::available_drives,
        texts::repository::{RepositoryLanguage, repository_languages},
    };

    /// The hotkeys of a PC-98 with two floppy drives, a CD-ROM drive and the CRT effect.
    fn pc98_hotkeys() -> HotkeyList {
        HotkeyFeatures {
            drives: available_drives(2, true),
            mouse: true,
            crt_effect: true,
            composite_video: false,
        }
        .hotkeys()
    }

    /// Runs one pass of `overlay` over the whole target of `gui` with the pending input, and
    /// the passes that it asks for. Returns the status of the last pass.
    fn pass(gui: &mut GuiHarness, overlay: &mut HelpOverlay) -> Status<Infallible> {
        let area = Rect::new(
            0.0,
            0.0,
            gui.viewport.width as f32 / gui.viewport.scale,
            gui.viewport.height as f32 / gui.viewport.scale,
        );
        gui.run_component(overlay, area, ()).0
    }

    /// The number of key caps in the scene of the last pass of `gui`.
    fn key_caps(gui: &GuiHarness) -> usize {
        gui.scene
            .instances()
            .iter()
            .filter(|instance| {
                instance.kind[0] == Instance::KIND_RECTANGLE && instance.rect[3] == KEYCAP_HEIGHT
            })
            .count()
    }

    #[test]
    fn each_row_draws_two_key_caps() {
        let mut gui = GuiHarness::new(1024, 768);
        let hotkeys = pc98_hotkeys();
        let count = hotkeys.len();
        let mut overlay = HelpOverlay::new(hotkeys);
        assert_eq!(pass(&mut gui, &mut overlay), Status::Open);
        assert_eq!(key_caps(&gui), count * 2);
    }

    #[test]
    fn cancel_closes_the_overlay() {
        let mut gui = GuiHarness::new(1024, 768);
        let mut overlay = HelpOverlay::new(pc98_hotkeys());
        assert_eq!(pass(&mut gui, &mut overlay), Status::Open);
        gui.input.cancel = true;
        assert_eq!(pass(&mut gui, &mut overlay), Status::Closed);
    }

    #[test]
    fn the_keys_scroll_a_list_that_overflows() {
        let mut gui = GuiHarness::new(640, 400);
        let mut overlay = HelpOverlay::new(pc98_hotkeys());
        pass(&mut gui, &mut overlay);
        assert_eq!(overlay.scroll.offset, 0.0);
        gui.input.navigation = Some(Navigation::Down);
        pass(&mut gui, &mut overlay);
        assert_eq!(overlay.scroll.offset, gui.theme.row_height);
        gui.input.navigation = Some(Navigation::End);
        pass(&mut gui, &mut overlay);
        let bottom = overlay.scroll.offset;
        assert!(bottom > gui.theme.row_height, "{bottom}");
        gui.input.navigation = Some(Navigation::Down);
        pass(&mut gui, &mut overlay);
        assert_eq!(overlay.scroll.offset, bottom);
        gui.input.navigation = Some(Navigation::Home);
        pass(&mut gui, &mut overlay);
        assert_eq!(overlay.scroll.offset, 0.0);
    }

    /// Verifies that the window of the minimum size of a session shows every hotkey without
    /// scrolling, and that its rows fill the panel.
    #[test]
    fn the_smallest_session_window_shows_every_row() {
        let (width, height) = SESSION_MINIMUM_WINDOW_SIZE;
        let mut gui = GuiHarness::new(width, height);
        let mut overlay = HelpOverlay::new(
            HotkeyFeatures {
                drives: available_drives(2, true),
                mouse: true,
                crt_effect: true,
                composite_video: true,
            }
            .hotkeys(),
        );
        assert_eq!(overlay.hotkeys.len(), Hotkey::ALL.len());
        pass(&mut gui, &mut overlay);
        gui.input.navigation = Some(Navigation::End);
        pass(&mut gui, &mut overlay);
        assert_eq!(overlay.scroll.offset, 0.0);
        let rows = Hotkey::ALL.len() as f32 * gui.theme.row_height;
        let content = PANEL_SIZE.1 - HEADER_HEIGHT - gui.theme.padding - gui.theme.gap * 0.5;
        assert!(content - rows < gui.theme.row_height, "{content} {rows}");
    }

    #[test]
    fn every_label_fits_its_row_in_every_language() {
        let (width, _) = PANEL_SIZE;
        for language in repository_languages() {
            let mut gui = GuiHarness::with_viewport(
                Viewport::new(1024, 768, 1.0),
                ThemeKind::Dark,
                language.catalog(),
            );
            gui.frame(|context| {
                let theme = context.theme;
                let content = width - theme.padding * 3.0;
                let key_column = key_column_width(context, &Hotkey::ALL);
                let style = theme.body();
                for hotkey in Hotkey::ALL {
                    let text = context.catalog.text(hotkey.label());
                    let label = context.measure(&style, text);
                    assert!(key_column + theme.gap * 2.0 + label <= content, "{text}");
                }
            });
        }
    }

    /// Renders the overlay at `scale` in the theme `kind` with the texts of
    /// the catalog of `language`.
    fn screenshot(scale: f32, kind: ThemeKind, language: &RepositoryLanguage) {
        let width = (960.0 * scale) as u32;
        let height = (680.0 * scale) as u32;
        let mut gui = GuiHarness::with_viewport(
            Viewport::new(width, height, scale),
            kind,
            language.catalog(),
        );
        let mut overlay = HelpOverlay::new(pc98_hotkeys());
        pass(&mut gui, &mut overlay);
        gui.write_screenshot(&format!("help_overlay_{scale}x"));
    }

    /// Writes the screenshots of the help overlay in both themes and in every
    /// language of the repository at the scales 1 and 2.
    #[test]
    #[ignore = "writes screenshots into the directory of GUI_SHOTS"]
    fn help_overlay_screenshots() {
        for language in repository_languages() {
            for kind in [ThemeKind::Dark, ThemeKind::Light] {
                screenshot(1.0, kind, language);
                screenshot(2.0, kind, language);
            }
        }
    }
}
