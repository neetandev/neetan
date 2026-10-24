//! Immediate mode widgets.
//!
//! The widgets keep no tree. In every pass the caller draws its whole user
//! interface again from its own state. A widget is a function that takes the
//! [`UiContext`] of the pass, reacts to the input and records its look into
//! the scene.
//!
//! The caller runs one pass for each discrete input event, see [`Input`],
//! and shows the scene of the last pass. It keeps one [`UiState`] between
//! passes and builds one [`UiContext`] for each pass from the [`Input`], the
//! [`Theme`], the [`crate::Fonts`] and the [`Viewport`], and ends the pass
//! with [`UiContext::finish`]. The layout works in logical units. The scene
//! multiplies them by the scale factor of the viewport and records physical
//! pixels.
//!
//! Every widget works with the mouse, the keyboard and a gamepad. The
//! focusable widgets of a pass form a focus ring in drawing order. The
//! [`Navigation`] events of the input move the focus along the ring or in a
//! direction, or reach the focused widget when it handles them itself. The
//! focused widget reacts to confirm. A click moves the focus to the clicked
//! widget. Mouse hover leaves the focus where it is.

mod button;
mod context;
mod focus;
mod icons;
mod label;
mod list;
mod modal;
mod tabs;
mod text_field;
mod theme;

pub use button::{Button, ButtonKind, button, icon_button, toggle_button};
pub use context::{
    Input, Modifiers, PointerResponse, TextStyle, UiContext, UiId, UiState, Viewport, ui_id,
    ui_id_indexed,
};
pub use focus::{Navigation, NavigationSet};
pub use icons::{Icon, Icons};
pub use label::{Align, Truncation, label};
pub use list::{ListResponse, ListRow, ListState, list, list_row};
pub use modal::{MODAL_HEADER_HEIGHT, ModalResponse, modal};
pub use tabs::{Tab, tab_bar};
pub use text_field::{TEXT_FIELD_LIMIT, TextFieldResponse, text_field};
pub use theme::Theme;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        Color, Fonts, Rasterizer, Rect, Scene, fonts::tests::font_data,
        rasterizer::tests::write_ppm,
    };

    /// The fonts, the theme and the pass state of a user interface without
    /// a window.
    pub(crate) struct Harness {
        /// The fonts with the font of the repository.
        pub(crate) fonts: Fonts,
        /// The theme on that font.
        pub(crate) theme: Theme,
        /// The scene of the last pass.
        pub(crate) scene: Scene,
        /// The state between passes.
        pub(crate) state: UiState,
        /// The input of the next pass.
        pub(crate) input: Input,
        /// The target of the passes.
        pub(crate) viewport: Viewport,
    }

    impl Harness {
        /// A harness with a target of 800 x 600 pixels at scale 1.
        pub(crate) fn new() -> Self {
            Self::with_viewport(Viewport::new(800, 600, 1.0))
        }

        /// A harness with the target `viewport`.
        pub(crate) fn with_viewport(viewport: Viewport) -> Self {
            let mut fonts = Fonts::new();
            let font = fonts.add(font_data()).unwrap();
            let theme = Theme::new(&mut fonts, font);
            Self {
                fonts,
                theme,
                scene: Scene::new(),
                state: UiState::default(),
                input: Input::default(),
                viewport,
            }
        }

        /// Draws one pass with `draw` and the pending input, as a window
        /// does, and returns the result of `draw`.
        pub(crate) fn frame<R>(&mut self, draw: impl FnOnce(&mut UiContext) -> R) -> R {
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
            result
        }

        /// Places the pointer at `(x, y)` in physical pixels.
        pub(crate) fn mouse(&mut self, x: f32, y: f32) {
            self.input.mouse_x = x;
            self.input.mouse_y = y;
        }

        /// Places the pointer at `(x, y)` and holds the left button.
        pub(crate) fn press(&mut self, x: f32, y: f32) {
            self.mouse(x, y);
            self.input.left = true;
        }

        /// Releases the left button.
        pub(crate) fn release(&mut self) {
            self.input.left = false;
        }

        /// Sets the navigation event of the next pass.
        pub(crate) fn navigate(&mut self, navigation: Navigation) {
            assert_eq!(self.input.navigation, None, "one event for each pass");
            self.input.navigation = Some(navigation);
        }

        /// Draws the scene of the last pass through the CPU rasterizer.
        pub(crate) fn render(&self) -> Vec<u8> {
            let Viewport { width, height, .. } = self.viewport;
            let mut output = vec![0; width as usize * height as usize * 4];
            Rasterizer::new().rasterize(&self.scene, self.fonts.slug(), width, height, &mut output);
            output
        }
    }

    /// The image files of the sample dialog.
    const FILES: [&str; 16] = [
        "System Disk.d88",
        "Ys II - Ancient Ys Vanished The Final Chapter (Disk A).d88",
        "ドラゴンスレイヤー英雄伝説 (ディスク1).hdm",
        "ドラゴンスレイヤー英雄伝説 (ディスク2).hdm",
        "Brandish (Disk 1 of 4).fdi",
        "Policenauts (Disk 1 of 4).hdm",
        "MS-DOS 6.2 Boot.hdm",
        "Touhou Reiiden - Highly Responsive to Prayers (1996)(Amusement Makers)[Disk 1 of 2][Japanese].fdi",
        "信長の野望・全国版.d88",
        "Rusty (1993)(C-Lab).hdm",
        "Sorcerian.d88",
        "Princess Maker 2 (Disk A).hdm",
        "幻影都市 (Disk 1).fdi",
        "Lemmings.d88",
        "Xak - The Art of Visual Stage.d88",
        "Last Armageddon.hdm",
    ];

    /// The loaded image of the sample dialog, as a list row.
    const LOADED_ROW: usize = 3;

    /// The tabs of the sample dialog.
    const TABS: [Tab; 3] = [
        Tab {
            caption: "FDD 1",
            icon: Some(Icon::Disk),
        },
        Tab {
            caption: "FDD 2",
            icon: Some(Icon::Disk),
        },
        Tab {
            caption: "CD-ROM",
            icon: Some(Icon::Disc),
        },
    ];

    /// The state of the sample dialog.
    #[derive(Default)]
    struct Sample {
        /// The selected tab.
        tab: usize,
        /// The filter text.
        filter: String,
        /// The list of the eject row and the images.
        list: ListState,
        /// The write protection toggle.
        write_protect: bool,
    }

    /// The layout and the events of a pass of the sample dialog.
    #[derive(Default)]
    struct SampleFrame {
        /// The activated list row.
        activated: Option<usize>,
        /// The insert button was activated.
        inserted: bool,
        /// The dialog asked to close.
        closed: bool,
        /// The area of the list.
        list: Rect,
        /// The area of the insert button.
        insert: Rect,
        /// The area of the filter field.
        filter: Rect,
    }

    /// Draws a stand-in for an emulator frame under the dialog.
    fn emulator_frame(context: &mut UiContext) {
        let bounds = context.bounds();
        context.fill(bounds, Color::hex(0x101860));
        let bars = [
            0xFF4040, 0xFFD040, 0x40D060, 0x40C0FF, 0x6060FF, 0xE060E0, 0xFFFFFF,
        ];
        let width = bounds.width / bars.len() as f32;
        for (index, &bar) in bars.iter().enumerate() {
            context.fill(
                Rect::new(index as f32 * width, bounds.height - 120.0, width, 80.0),
                Color::hex(bar),
            );
        }
        let style = context
            .theme
            .body()
            .with_size(16.0)
            .with_color(Color::WHITE);
        for (line, text) in ["A>dir /w", "SYSTEM   DISK     GAME     DATA", "A>game"]
            .iter()
            .enumerate()
        {
            context.text(24.0, 40.0 + line as f32 * 22.0, &style, text);
        }
    }

    /// Draws the sample dialog that resembles the media dialog.
    fn sample_dialog(context: &mut UiContext, sample: &mut Sample) -> SampleFrame {
        let mut frame = SampleFrame::default();
        emulator_frame(context);
        let response = modal(context, "media", (640.0, 540.0), "Insert disk image");
        frame.closed = response.closed;
        let theme = context.theme;
        let gap = theme.gap;
        let (tabs, rest) = response.content.cut_top(40.0);
        tab_bar(context, "drives", tabs, &TABS, &mut sample.tab);
        let (filter, rest) = rest.cut_top(gap * 1.5).1.cut_top(theme.control_height);
        frame.filter = filter;
        text_field(
            context,
            "filter",
            filter,
            &mut sample.filter,
            "Filter images",
        );
        let (footer, rest) = rest.cut_bottom(theme.control_height);
        let (folder, rest) = rest.cut_top(gap).1.cut_top(24.0);
        let (icon, path) = folder.cut_left(theme.icon_size);
        context.icon(icon, Icon::Folder, theme.icon_size, theme.text_dim);
        label(
            context,
            path.cut_left(gap).1,
            &theme.small(),
            "/home/user/Documents/Emulation/PC-98/Disk images/Adventure and role playing games",
            Align::Start,
            Truncation::Middle,
        );
        let list_rect = rest.cut_top(gap * 0.5).1.cut_bottom(gap * 1.5).1;
        frame.list = list_rect;
        context.default_focus(UiId::from("files"));
        let list_response = list(
            context,
            "files",
            list_rect,
            FILES.len() + 1,
            &mut sample.list,
            |context, row| {
                if row.index == 0 {
                    list_row(context, row, Some(Icon::Eject), "Eject disk", None);
                } else {
                    let marker = (row.index == LOADED_ROW).then_some(Icon::Check);
                    list_row(context, row, Some(Icon::Disk), FILES[row.index - 1], marker);
                }
            },
        );
        frame.activated = list_response.activated;
        let (insert, footer) = footer.cut_right(104.0);
        let (cancel, footer) = footer.cut_right(gap).1.cut_right(96.0);
        let protect = footer.cut_left(150.0).0;
        frame.insert = insert;
        if toggle_button(
            context,
            "protect",
            protect,
            "Write protect",
            sample.write_protect,
        ) {
            sample.write_protect = !sample.write_protect;
        }
        frame.closed |= button(context, "cancel", cancel, "Cancel");
        frame.inserted = Button::new("insert", "Insert")
            .icon(Icon::Check)
            .kind(ButtonKind::Primary)
            .show(context, insert);
        frame
    }

    /// Runs one pass of the sample dialog.
    fn pass(harness: &mut Harness, sample: &mut Sample) -> SampleFrame {
        harness.frame(|context| sample_dialog(context, sample))
    }

    /// Runs one pass of the sample dialog with the navigation event
    /// `navigation`.
    fn navigate(harness: &mut Harness, sample: &mut Sample, navigation: Navigation) -> SampleFrame {
        harness.navigate(navigation);
        pass(harness, sample)
    }

    /// The center of the row `index` of the list of `layout` at the top of
    /// the list, in physical pixels at `scale`.
    fn row_point(harness: &Harness, layout: &SampleFrame, index: usize, scale: f32) -> (f32, f32) {
        let theme = &harness.theme;
        let y = layout.list.y
            + theme.stroke
            + list::LIST_PADDING
            + (index as f32 + 0.5) * theme.row_height;
        (layout.list.center().0 * scale, y * scale)
    }

    #[test]
    fn the_sample_dialog_works_with_the_keyboard_only() {
        let mut harness = Harness::new();
        let mut sample = Sample::default();
        pass(&mut harness, &mut sample);
        assert_eq!(harness.state.focused(), Some(UiId::from("files")));
        navigate(&mut harness, &mut sample, Navigation::Down);
        navigate(&mut harness, &mut sample, Navigation::Down);
        assert_eq!(sample.list.selected, 2);
        navigate(&mut harness, &mut sample, Navigation::Home);
        navigate(&mut harness, &mut sample, Navigation::Up);
        assert_eq!(harness.state.focused(), Some(UiId::from("filter")));
        assert!(harness.state.wants_text_input());
        harness.input.text.push_str("ys");
        pass(&mut harness, &mut sample);
        assert_eq!(sample.filter, "ys");
        harness.input.cancel = true;
        assert!(!pass(&mut harness, &mut sample).closed);
        assert!(sample.filter.is_empty());
        navigate(&mut harness, &mut sample, Navigation::Up);
        navigate(&mut harness, &mut sample, Navigation::Right);
        assert_eq!(harness.state.focused(), Some(UiId::from("drives")));
        assert_eq!(sample.tab, 1);
        for navigation in [
            Navigation::Down,
            Navigation::Down,
            Navigation::Down,
            Navigation::End,
            Navigation::Down,
        ] {
            navigate(&mut harness, &mut sample, navigation);
        }
        assert_eq!(sample.list.selected, FILES.len());
        assert_eq!(harness.state.focused(), Some(UiId::from("insert")));
        harness.input.confirm = true;
        assert!(pass(&mut harness, &mut sample).inserted);
        navigate(&mut harness, &mut sample, Navigation::Left);
        assert_eq!(harness.state.focused(), Some(UiId::from("cancel")));
        harness.input.confirm = true;
        assert!(pass(&mut harness, &mut sample).closed);
        navigate(&mut harness, &mut sample, Navigation::Next);
        navigate(&mut harness, &mut sample, Navigation::Next);
        assert_eq!(
            harness.state.focused(),
            Some(UiId::from("media").child("close"))
        );
        harness.input.cancel = true;
        assert!(pass(&mut harness, &mut sample).closed);
    }

    #[test]
    fn the_sample_dialog_works_with_the_mouse_only() {
        let scale = 2.0;
        let mut harness = Harness::with_viewport(Viewport::new(1600, 1200, scale));
        let mut sample = Sample::default();
        let layout = pass(&mut harness, &mut sample);
        let (x, y) = row_point(&harness, &layout, 4, scale);
        harness.press(x, y);
        pass(&mut harness, &mut sample);
        harness.release();
        pass(&mut harness, &mut sample);
        assert_eq!(sample.list.selected, 4);
        harness.input.left_double = true;
        harness.press(x, y);
        assert_eq!(pass(&mut harness, &mut sample).activated, Some(4));
        harness.release();
        pass(&mut harness, &mut sample);

        let (x, y) = layout.filter.center();
        harness.press(x * scale, y * scale);
        pass(&mut harness, &mut sample);
        harness.release();
        pass(&mut harness, &mut sample);
        assert_eq!(harness.state.focused(), Some(UiId::from("filter")));
        assert_eq!(sample.list.selected, 4);

        let (x, y) = layout.insert.center();
        harness.press(x * scale, y * scale);
        pass(&mut harness, &mut sample);
        harness.release();
        assert!(pass(&mut harness, &mut sample).inserted);
    }

    #[test]
    fn the_sample_dialog_survives_empty_and_broken_viewports() {
        for viewport in [
            Viewport::new(0, 0, 1.0),
            Viewport {
                width: 10,
                height: 10,
                scale: 0.0,
            },
            Viewport {
                width: 300,
                height: 200,
                scale: f32::NAN,
            },
            Viewport::new(1, 1, 1000.0),
            Viewport::new(100, 50, 0.001),
        ] {
            let mut harness = Harness::with_viewport(viewport);
            let mut sample = Sample::default();
            harness.mouse(5.0, 5.0);
            harness.input.wheel = -3.0;
            pass(&mut harness, &mut sample);
            navigate(&mut harness, &mut sample, Navigation::Down);
            for instance in harness.scene.instances() {
                assert!(instance.rect.iter().all(|value| value.is_finite()));
                assert!(instance.rect[2] >= 0.0 && instance.rect[3] >= 0.0);
            }
            let pixels = harness.render();
            assert_eq!(
                pixels.len(),
                viewport.width as usize * viewport.height as usize * 4
            );
        }
    }

    /// Renders the sample dialog at `scale` and writes it through the PPM
    /// helper: once with the focus on the list and the pointer over a row,
    /// once with the focus on the filter field, a composition in it and the
    /// pointer over a button.
    fn sample_screenshot(scale: f32) {
        let width = (960.0 * scale) as u32;
        let height = (640.0 * scale) as u32;
        let mut harness = Harness::with_viewport(Viewport::new(width, height, scale));
        let mut sample = Sample {
            write_protect: true,
            ..Sample::default()
        };
        sample.list.select(LOADED_ROW + 2);
        let layout = navigate(&mut harness, &mut sample, Navigation::Down);
        let (_, hover_y) = row_point(&harness, &layout, 7, scale);
        harness.mouse((layout.list.x + 200.0) * scale, hover_y);
        pass(&mut harness, &mut sample);
        write_ppm(
            &format!("gui_panel_{scale}x"),
            width,
            height,
            &harness.render(),
        );

        navigate(&mut harness, &mut sample, Navigation::Previous);
        harness.input.text.push_str("ドラゴンスレイヤー英雄伝説 (");
        let (x, y) = layout.insert.center();
        harness.mouse((x - 100.0) * scale, y * scale);
        pass(&mut harness, &mut sample);
        harness.input.composition.push_str("でぃすく");
        pass(&mut harness, &mut sample);
        write_ppm(
            &format!("gui_panel_{scale}x_filter"),
            width,
            height,
            &harness.render(),
        );
    }

    #[test]
    #[ignore = "writes a screenshot into the directory of GUI_SHOTS"]
    fn panel_screenshot() {
        sample_screenshot(1.0);
        sample_screenshot(1.5);
        sample_screenshot(2.0);
    }
}
