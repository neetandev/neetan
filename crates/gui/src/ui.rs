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
//! [`Navigation`] events of the input give the focus to another widget of
//! the ring or in a direction, or reach the focused widget when it handles
//! them itself. The focused widget reacts to confirm. A click gives the
//! focus to the clicked widget. Mouse hover leaves the focus where it is.
//!
//! A pass draws in layers: the base, each open modal in
//! [`UiContext::layer`] and a popup in [`UiContext::popup`]. The modals form
//! a stack in the order they open. The top layer of the last pass blocks
//! every widget of the layers below it from the start of a pass: a blocked
//! widget draws and joins the focus ring, and ignores the pointer, the
//! wheel, navigation, the shoulder buttons, the action buttons, the
//! shortcuts, confirm and cancel. Navigation stays in the layer of the
//! focused widget. A layer that opens takes the focus and asks for another
//! pass. A layer that closes gives the focus back and asks for another
//! pass.

mod button;
mod checkbox;
mod confirm;
mod context;
mod dropdown;
mod focus;
mod icons;
mod input;
mod label;
mod layer;
mod list;
mod modal;
mod number_field;
mod scroll;
mod scroll_bar;
mod state;
mod status_line;
mod table;
mod tabs;
mod text_field;
mod theme;

pub use button::{
    BUTTON_MIN_WIDTH, Button, ButtonKind, PRIMARY_BUTTON_MIN_WIDTH, button, icon_button,
    toggle_button,
};
pub use checkbox::{Checkbox, checkbox};
pub use confirm::{ConfirmDialog, ConfirmResponse, DIALOG_WIDTH};
pub use context::{PointerResponse, TextStyle, UiContext, UiId, ui_id, ui_id_indexed};
pub use dropdown::{Dropdown, POPUP_MAX_ROWS, dropdown};
pub use focus::{Navigation, NavigationSet};
pub use icons::{Icon, Icons};
pub use input::{ActionButton, Input, Shortcut, Viewport};
pub use label::{
    Align, Truncation, WRAP_LINE_HEIGHT, label, wrapped_height, wrapped_label, wrapped_line_height,
};
pub use list::{
    LIST_PADDING, List, ListResponse, ListRow, ListState, PATH_LINE_HEIGHT, RowContent, RowKind,
    RowMark, caption_row, list,
};
pub use modal::{CLOSE_SIZE, HEADER_HEIGHT, ModalResponse, modal, modal_in};
pub use number_field::{NUMBER_FIELD_WIDTH, NumberError, NumberField};
pub use scroll::{ScrollState, scroll_area};
pub use state::UiState;
pub use status_line::{
    ACTIVATE_KEYS, CANCEL_KEYS, Hint, HintKey, STATUS_LINE_HEIGHT, StatusEnd, distinct_hints,
    status_line,
};
pub use table::{
    CellTone, Column, ColumnWidth, EmptyHint, Sort, SortDirection, Table, TableCell, TableResponse,
    TableRow, TableState,
};
pub use tabs::{Tab, tab_bar};
pub use text_field::{TEXT_FIELD_LIMIT, TextField, TextFieldResponse, text_field};
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

    /// The RGBA8 pixel of the output `pixels` of a target `width` pixels
    /// wide at the pixel coordinates `(x, y)`.
    pub(crate) fn pixel(pixels: &[u8], width: u32, x: f32, y: f32) -> [u8; 4] {
        let index = (y.floor() as usize * width as usize + x.floor() as usize) * 4;
        [
            pixels[index],
            pixels[index + 1],
            pixels[index + 2],
            pixels[index + 3],
        ]
    }

    /// The RGBA8 pixel of the opaque color `color` in the output of the
    /// rasterizer.
    pub(crate) fn srgb(color: Color) -> [u8; 4] {
        [color.r, color.g, color.b, 255]
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
                    RowContent::new("Eject disk")
                        .icon(Some(Icon::Eject))
                        .show(context, row);
                } else {
                    let marker = (row.index == LOADED_ROW).then_some(Icon::Check);
                    RowContent::new(FILES[row.index - 1])
                        .icon(Some(Icon::Disk))
                        .mark(marker.map(RowMark::Marker))
                        .show(context, row);
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

    /// The screenshot of every widget of the GUI additions in its states.
    mod gallery {
        use super::*;

        /// The gap between two cells of the gallery.
        const CELL_GAP: f32 = 12.0;

        /// The height of the rows of the dropdowns, which leaves room for the
        /// popup of the press.
        const DROPDOWN_ROW_HEIGHT: f32 = 330.0;

        /// The height of the state caption at the top of a cell.
        const CAPTION_HEIGHT: f32 = 22.0;

        /// The models of the PC-98 for the dropdowns.
        const MODELS: [&str; 7] = [
            "PC9801F", "PC9801VM", "PC9801VX", "PC9801RS", "PC9801RA", "PC9821AS", "PC9821AP",
        ];

        /// A state of a widget in a row of the gallery.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum State {
            /// Without the pointer and the focus.
            Idle,
            /// Under the pointer.
            Hover,
            /// Under a held press.
            Pressed,
            /// With the focus.
            Focus,
            /// Disabled.
            Disabled,
        }

        impl State {
            /// Every state in the order of a row.
            const ALL: [State; 5] = [
                State::Idle,
                State::Hover,
                State::Pressed,
                State::Focus,
                State::Disabled,
            ];

            /// The caption of the state.
            fn caption(self) -> &'static str {
                match self {
                    State::Idle => "Idle",
                    State::Hover => "Hover",
                    State::Pressed => "Pressed",
                    State::Focus => "Focus",
                    State::Disabled => "Disabled",
                }
            }
        }

        /// The columns of the game table.
        const COLUMNS: [Column; 4] = [
            Column {
                caption: "Title",
                width: ColumnWidth::Weight(3.0),
                align: Align::Start,
            },
            Column {
                caption: "Original title",
                width: ColumnWidth::Weight(3.0),
                align: Align::Start,
            },
            Column {
                caption: "Developer",
                width: ColumnWidth::Weight(2.0),
                align: Align::Start,
            },
            Column {
                caption: "Year",
                width: ColumnWidth::Fixed(64.0),
                align: Align::End,
            },
        ];

        /// The games of the first table, sorted by title.
        const GAMES: [[&str; 4]; 5] = [
            ["Brandish", "ブランディッシュ", "Nihon Falcom", "1991"],
            ["Dokyusei", "同級生", "Elf", "1992"],
            [
                "Dragon Slayer: The Legend of Heroes",
                "ドラゴンスレイヤー英雄伝説",
                "Nihon Falcom",
                "1989",
            ],
            ["Edge", "エッジ", "TGL", "1993"],
            [
                "Gen'ei Toshi: Illusion City",
                "幻影都市",
                "Microcabin",
                "1991",
            ],
        ];

        /// The games of the second table, sorted by year with a caption row.
        const GAMES_BY_YEAR: [[&str; 4]; 6] = [
            ["Edge", "エッジ", "TGL", "1993"],
            ["Dokyusei", "同級生", "Elf", "1992"],
            ["Before 1992", "", "", ""],
            ["Brandish", "ブランディッシュ", "Nihon Falcom", "1991"],
            [
                "Gen'ei Toshi: Illusion City",
                "幻影都市",
                "Microcabin",
                "1991",
            ],
            [
                "Dragon Slayer: The Legend of Heroes",
                "ドラゴンスレイヤー英雄伝説",
                "Nihon Falcom",
                "1989",
            ],
        ];

        /// The rows of the form in the scroll area: the label, the key and
        /// the inherited value.
        const FORM: [(&str, &str, &str); 6] = [
            ("Aspect mode", "aspect-mode", "Built-in default: 4:3"),
            ("Window mode", "window-mode", "Built-in default: windowed"),
            ("CRT effect", "crt", "System default: off"),
            ("Scaling", "scaling", "System default: nearest"),
            ("GDC clock", "force-gdc-clock", "Built-in default: auto"),
            ("Graphics board", "graphicboard", "Built-in default: none"),
        ];

        /// A function that runs one pass of a cell.
        type Pass<'a> = dyn FnMut(&mut Harness) + 'a;

        /// A text field variant of the gallery.
        struct Variant {
            /// The caption of the cell.
            caption: &'static str,
            /// The leading icon.
            icon: Option<Icon>,
            /// The text.
            text: &'static str,
            /// Whether the field draws the error edge.
            error: bool,
            /// Whether the field reacts to input.
            enabled: bool,
            /// Whether the field is read-only.
            read_only: bool,
        }

        /// The pixels of the whole gallery.
        struct Gallery {
            /// The physical pixels for each logical unit.
            scale: f32,
            /// The width in physical pixels.
            width: u32,
            /// The height in physical pixels.
            height: u32,
            /// The RGBA8 pixels.
            pixels: Vec<u8>,
        }

        impl Gallery {
            /// A gallery of `width` x `height` logical units at `scale`.
            fn new(width: f32, height: f32, scale: f32) -> Self {
                let width = (width * scale) as u32;
                let height = (height * scale) as u32;
                let background = Color::hex(0x131519);
                let pixels = [background.r, background.g, background.b, 255]
                    .repeat(width as usize * height as usize);
                Self {
                    scale,
                    width,
                    height,
                    pixels,
                }
            }

            /// Renders a cell of `size` at `origin` with the state caption
            /// `caption`. `draw` draws the widget into the area below the
            /// caption in every pass. `script` runs the passes of the state
            /// after a first pass, through the pass function it receives.
            fn cell(
                &mut self,
                origin: (f32, f32),
                size: (f32, f32),
                caption: &str,
                mut draw: impl FnMut(&mut UiContext, Rect),
                script: impl FnOnce(&mut Harness, Rect, &mut Pass<'_>),
            ) {
                let scale = self.scale;
                let width = (size.0 * scale) as u32;
                let height = (size.1 * scale) as u32;
                let mut harness = Harness::with_viewport(Viewport::new(width, height, scale));
                let area = Rect::new(
                    CELL_GAP,
                    CAPTION_HEIGHT,
                    size.0 - CELL_GAP * 2.0,
                    size.1 - CAPTION_HEIGHT - CELL_GAP * 0.5,
                );
                let mut pass = |harness: &mut Harness| {
                    harness.frame(|context| {
                        let bounds = context.bounds();
                        let theme = context.theme;
                        context.fill(bounds, theme.panel);
                        context.text_left(
                            Rect::new(CELL_GAP, 2.0, bounds.width, CAPTION_HEIGHT - 4.0),
                            &theme.small(),
                            caption,
                        );
                        draw(context, area);
                    });
                };
                pass(&mut harness);
                script(&mut harness, area, &mut pass);
                let pixels = harness.render();
                let left = (origin.0 * scale) as usize;
                let top = (origin.1 * scale) as usize;
                for row in 0..height as usize {
                    let target_row = top + row;
                    if target_row >= self.height as usize {
                        break;
                    }
                    let columns = (width as usize).min((self.width as usize).saturating_sub(left));
                    let source = row * width as usize * 4;
                    let target = (target_row * self.width as usize + left) * 4;
                    self.pixels[target..target + columns * 4]
                        .copy_from_slice(&pixels[source..source + columns * 4]);
                }
            }

            /// Writes the gallery through the PPM helper.
            fn write(&self, name: &str) {
                write_ppm(name, self.width, self.height, &self.pixels);
            }
        }

        /// Runs the state `state` of a widget at the area `area`: the
        /// pointer at `points.0` from the top left of the area for the
        /// hover, a held press at `points.1` for the press, the focus on
        /// `focus` for the focus.
        fn script(
            state: State,
            points: ((f32, f32), (f32, f32)),
            focus: UiId,
        ) -> impl FnOnce(&mut Harness, Rect, &mut Pass<'_>) {
            move |harness, area, pass| {
                let scale = harness.viewport.scale;
                let at = |(x, y): (f32, f32)| ((area.x + x) * scale, (area.y + y) * scale);
                match state {
                    State::Hover => {
                        let (x, y) = at(points.0);
                        harness.mouse(x, y);
                    }
                    State::Pressed => {
                        let (x, y) = at(points.1);
                        harness.press(x, y);
                    }
                    State::Focus => {
                        harness.state.set_focus(focus);
                        pass(harness);
                        harness.navigate(Navigation::Home);
                    }
                    State::Idle | State::Disabled => {}
                }
                pass(harness);
            }
        }

        /// The widget area of a control in a cell area.
        fn control(area: Rect, width: f32) -> Rect {
            Rect::new(area.x, area.y, width.min(area.width), 32.0)
        }

        /// Renders the gallery at `scale`.
        fn render(scale: f32) {
            let mut gallery =
                Gallery::new(1340.0, 2044.0 + (DROPDOWN_ROW_HEIGHT - 72.0) * 2.0, scale);
            let row_height = 72.0;
            let mut top = CELL_GAP;
            let state_cells =
                |gallery: &mut Gallery,
                 top: f32,
                 (width, height): (f32, f32),
                 name: &str,
                 points: ((f32, f32), (f32, f32)),
                 draw: &dyn Fn(&mut UiContext, Rect, State)| {
                    for (index, state) in State::ALL.into_iter().enumerate() {
                        let x = CELL_GAP + index as f32 * (width + CELL_GAP);
                        let caption = format!("{name}: {}", state.caption());
                        gallery.cell(
                            (x, top),
                            (width, height),
                            &caption,
                            |context, area| draw(context, area, state),
                            script(state, points, UiId::from("widget")),
                        );
                    }
                };
            let center = ((40.0, 16.0), (40.0, 16.0));

            state_cells(
                &mut gallery,
                top,
                (240.0, DROPDOWN_ROW_HEIGHT),
                "Dropdown",
                center,
                &|context, area, state| {
                    let mut value = Some(4);
                    Dropdown::new("widget", &MODELS)
                        .enabled(state != State::Disabled)
                        .show(context, control(area, 216.0), &mut value);
                },
            );
            top += DROPDOWN_ROW_HEIGHT + CELL_GAP;
            state_cells(
                &mut gallery,
                top,
                (240.0, DROPDOWN_ROW_HEIGHT),
                "Dropdown, inherited",
                center,
                &|context, area, state| {
                    let mut value = None;
                    Dropdown::new("widget", &MODELS)
                        .inherited("System default: PC9801RA")
                        .enabled(state != State::Disabled)
                        .show(context, control(area, 216.0), &mut value);
                },
            );
            top += DROPDOWN_ROW_HEIGHT + CELL_GAP;
            state_cells(
                &mut gallery,
                top,
                (240.0, row_height),
                "Checkbox",
                center,
                &|context, area, state| {
                    let mut value = matches!(state, State::Focus | State::Disabled);
                    Checkbox::new("widget", "Show all files")
                        .enabled(state != State::Disabled)
                        .show(context, control(area, 216.0), &mut value);
                },
            );
            top += row_height + CELL_GAP;
            state_cells(
                &mut gallery,
                top,
                (240.0, row_height),
                "Number field",
                ((NUMBER_FIELD_WIDTH - 16.0, 16.0), (40.0, 16.0)),
                &|context, area, state| {
                    let mut text = match state {
                        State::Pressed => String::new(),
                        State::Focus => "3".to_owned(),
                        State::Idle | State::Hover | State::Disabled => "1996".to_owned(),
                    };
                    let field =
                        NumberField::new("widget", "Not set").enabled(state != State::Disabled);
                    let field = if state == State::Focus {
                        field.range(0.0, 3.0)
                    } else {
                        field.range(0.0, 9999.0)
                    };
                    field.show(context, control(area, NUMBER_FIELD_WIDTH), &mut text);
                },
            );
            top += row_height + CELL_GAP;
            for (index, (caption, value)) in [("Grown, unset", ""), ("Grown, set", "1")]
                .into_iter()
                .enumerate()
            {
                let mut text = value.to_owned();
                gallery.cell(
                    (CELL_GAP + index as f32 * (240.0 + CELL_GAP), top),
                    (240.0, row_height),
                    &format!("Number field: {caption}"),
                    |context, area| {
                        let field =
                            NumberField::new("widget", "Built-in default: 0").range(0.0, 3.0);
                        let width = field.width(context);
                        field.show(context, control(area, width), &mut text);
                    },
                    |harness, _, pass| pass(harness),
                );
            }
            top += row_height + CELL_GAP;

            let variants = [
                Variant {
                    caption: "Folder icon",
                    icon: Some(Icon::Folder),
                    text: "games/98/YU-NO/yu-no.hdi",
                    error: false,
                    enabled: true,
                    read_only: false,
                },
                Variant {
                    caption: "No icon",
                    icon: None,
                    text: "Install to HDD 1 first. Needs the 86 board for music.",
                    error: false,
                    enabled: true,
                    read_only: false,
                },
                Variant {
                    caption: "Error edge",
                    icon: Some(Icon::Folder),
                    text: "/mnt/archive/mame/pc88va2",
                    error: true,
                    enabled: true,
                    read_only: false,
                },
                Variant {
                    caption: "Disabled",
                    icon: Some(Icon::Folder),
                    text: "",
                    error: false,
                    enabled: false,
                    read_only: false,
                },
                Variant {
                    caption: "Read-only",
                    icon: Some(Icon::Folder),
                    text: "config/pc98/brandish-2-the-planet-buster.conf",
                    error: false,
                    enabled: true,
                    read_only: true,
                },
            ];
            for (index, variant) in variants.into_iter().enumerate() {
                let x = CELL_GAP + index as f32 * (252.0 + CELL_GAP);
                let mut text = variant.text.to_owned();
                let truncation = if variant.icon.is_some() {
                    Truncation::Middle
                } else {
                    Truncation::End
                };
                gallery.cell(
                    (x, top),
                    (252.0, row_height),
                    &format!("Text field: {}", variant.caption),
                    |context, area| {
                        TextField::new("widget", "Not set")
                            .icon(variant.icon)
                            .clears_on_cancel(false)
                            .error(variant.error)
                            .enabled(variant.enabled)
                            .read_only(variant.read_only)
                            .truncation(truncation)
                            .show(context, control(area, 228.0), &mut text);
                    },
                    |harness, _, pass| pass(harness),
                );
            }
            top += row_height + CELL_GAP;
            state_cells(
                &mut gallery,
                top,
                (240.0, row_height),
                "Danger button",
                center,
                &|context, area, state| {
                    Button::new("widget", "Delete")
                        .icon(Icon::Delete)
                        .kind(ButtonKind::Danger)
                        .enabled(state != State::Disabled)
                        .show(context, control(area, 104.0));
                },
            );
            top += row_height + CELL_GAP;

            let texts = [
                (
                    "Wrapping label: body text",
                    false,
                    "The changes to YU-NO: A Girl Who Chants Love at the Bound of this World are not saved.",
                ),
                (
                    "Wrapping label: small text",
                    true,
                    "The file config/pc98/yu-no.conf is removed. The disk images stay where they are.",
                ),
            ];
            for (index, (caption, small, text)) in texts.into_iter().enumerate() {
                gallery.cell(
                    (CELL_GAP + index as f32 * (276.0 + CELL_GAP), top),
                    (276.0, 110.0),
                    caption,
                    |context, area| {
                        let theme = context.theme;
                        let style = if small { theme.small() } else { theme.body() };
                        wrapped_label(
                            context,
                            Rect::new(area.x, area.y, 250.0, 0.0),
                            &style,
                            text,
                            Align::Start,
                            usize::MAX,
                        );
                    },
                    |harness, _, pass| pass(harness),
                );
            }
            let kinds = [RowKind::Disabled, RowKind::Normal];
            let mut list_state = ListState::new();
            gallery.cell(
                (CELL_GAP * 3.0 + 552.0, top),
                (276.0, 110.0),
                "List: disabled list and row",
                |context, area| {
                    List::new("disabled", 2).kinds(&kinds).enabled(false).show(
                        context,
                        Rect::new(area.x, area.y, 250.0, 70.0),
                        &mut list_state,
                        |context, row| {
                            RowContent::new("Brandish (Disk 1 of 4).hdm")
                                .icon(Some(Icon::Disk))
                                .show(context, row);
                        },
                    );
                },
                |harness, _, pass| pass(harness),
            );
            let kinds = [RowKind::Caption, RowKind::Normal, RowKind::Disabled];
            let mut list_state = ListState::new();
            gallery.cell(
                (CELL_GAP * 4.0 + 828.0, top),
                (276.0, 140.0),
                "List: caption, focused and disabled rows",
                |context, area| {
                    List::new("rows", 3).kinds(&kinds).show(
                        context,
                        Rect::new(area.x, area.y, 250.0, 100.0),
                        &mut list_state,
                        |context, row| match row.kind {
                            RowKind::Caption => {
                                caption_row(context, row, "Files that fail to parse")
                            }
                            RowKind::Normal | RowKind::Disabled => {
                                RowContent::new("Lemmings.d88")
                                    .icon(Some(Icon::Disk))
                                    .mark(Some(RowMark::Marker(Icon::Check)))
                                    .show(context, row);
                            }
                        },
                    );
                },
                |harness, _, pass| {
                    harness.state.set_focus(UiId::from("rows"));
                    pass(harness);
                },
            );
            top += 140.0 + CELL_GAP;

            let mut value = Some(4);
            gallery.cell(
                (CELL_GAP, top),
                (420.0, 360.0),
                "Dropdown popup: open, PC9801VX highlighted",
                |context, area| {
                    Dropdown::new("model", &MODELS)
                        .inherited("System default: PC9801RA")
                        .show(context, control(area, 280.0), &mut value);
                },
                |harness, _, pass| {
                    harness.state.set_focus(UiId::from("model"));
                    pass(harness);
                    harness.input.confirm = true;
                    pass(harness);
                    for _ in 0..2 {
                        harness.navigate(Navigation::Up);
                        pass(harness);
                    }
                },
            );
            let mut scroll = ScrollState::new();
            scroll.offset = 52.0;
            let mut values = [None; FORM.len()];
            values[1] = Some(1);
            let form = |context: &mut UiContext,
                        content: Rect,
                        values: &mut [Option<usize>; FORM.len()]| {
                let theme = context.theme;
                for (index, (label, key, inherited)) in FORM.iter().enumerate() {
                    let row_top = content.y + index as f32 * 40.0 + 4.0;
                    let label_area = Rect::new(content.x + 12.0, row_top, 172.0, 32.0);
                    context.text_in(
                        Rect::new(label_area.x, label_area.y, label_area.width, 18.0),
                        &theme.body(),
                        label,
                        Align::Start,
                        Truncation::End,
                    );
                    context.text_in(
                        Rect::new(label_area.x, label_area.y + 16.0, label_area.width, 16.0),
                        &theme.small(),
                        key,
                        Align::Start,
                        Truncation::End,
                    );
                    Dropdown::new(
                        UiId::from("form").indexed(index),
                        &["windowed", "fullscreen"],
                    )
                    .inherited(inherited)
                    .show(
                        context,
                        Rect::new(content.x + 200.0, row_top, 360.0, 32.0),
                        &mut values[index],
                    );
                }
                FORM.len() as f32 * 40.0
            };
            gallery.cell(
                (CELL_GAP * 2.0 + 420.0, top),
                (800.0, 250.0),
                "Scroll area: Scaling holds the focus",
                |context, area| {
                    scroll_area(
                        context,
                        "scroll",
                        Rect::new(area.x, area.y, 760.0, 200.0),
                        &mut scroll,
                        |context, content| form(context, content, &mut values),
                    );
                },
                |harness, _, pass| {
                    harness.state.set_focus(UiId::from("form").indexed(3));
                    pass(harness);
                },
            );
            let mut scroll = ScrollState::new();
            scroll.offset = 52.0;
            let mut values = [None; FORM.len()];
            gallery.cell(
                (CELL_GAP * 2.0 + 420.0, top + 250.0 + CELL_GAP),
                (800.0, 250.0),
                "Scroll area: hovered thumb",
                |context, area| {
                    scroll_area(
                        context,
                        "scroll",
                        Rect::new(area.x, area.y, 760.0, 200.0),
                        &mut scroll,
                        |context, content| form(context, content, &mut values),
                    );
                },
                |harness, area, pass| {
                    let scale = harness.viewport.scale;
                    harness.mouse((area.x + 755.0) * scale, (area.y + 100.0) * scale);
                    pass(harness);
                },
            );
            top += 512.0 + CELL_GAP;

            let table_cell_draw = |games: &'static [[&'static str; 4]]| {
                move |context: &mut UiContext, row: &TableRow| {
                    let game = games[row.row.index];
                    if row.row.kind == RowKind::Caption {
                        caption_row(context, &row.row, game[0]);
                        return;
                    }
                    for (column, text) in game.iter().enumerate() {
                        let tone = if column >= 2 {
                            CellTone::Dim
                        } else {
                            CellTone::Normal
                        };
                        TableCell::new(text)
                            .tone(tone)
                            .show(context, row, column, column);
                    }
                }
            };
            let mut table = TableState {
                sort: Some(Sort {
                    column: 0,
                    direction: SortDirection::Ascending,
                }),
                ..TableState::new()
            };
            table.list.select(1);
            gallery.cell(
                (CELL_GAP, top),
                (640.0, 260.0),
                "Table: sorted by title, Developer hovered, rows focused",
                |context, area| {
                    Table::new("games", &COLUMNS, GAMES.len()).show(
                        context,
                        Rect::new(area.x, area.y, 616.0, 225.0),
                        &mut table,
                        table_cell_draw(&GAMES),
                    );
                },
                |harness, area, pass| {
                    let scale = harness.viewport.scale;
                    harness.state.set_focus(UiId::from("games"));
                    harness.mouse((area.x + 420.0) * scale, (area.y + 20.0) * scale);
                    pass(harness);
                },
            );
            let kinds = [
                RowKind::Normal,
                RowKind::Normal,
                RowKind::Caption,
                RowKind::Normal,
                RowKind::Normal,
                RowKind::Normal,
            ];
            let mut table = TableState {
                sort: Some(Sort {
                    column: 3,
                    direction: SortDirection::Descending,
                }),
                picked: 3,
                ..TableState::new()
            };
            table.list.select(1);
            let header = Table::new("games", &COLUMNS, 0).header_id();
            gallery.cell(
                (CELL_GAP * 2.0 + 640.0, top),
                (640.0, 290.0),
                "Table: sorted by year, header focused, caption row",
                |context, area| {
                    Table::new("games", &COLUMNS, GAMES_BY_YEAR.len())
                        .kinds(&kinds)
                        .show(
                            context,
                            Rect::new(area.x, area.y, 616.0, 255.0),
                            &mut table,
                            table_cell_draw(&GAMES_BY_YEAR),
                        );
                },
                |harness, _, pass| {
                    harness.state.set_focus(header);
                    pass(harness);
                },
            );
            top += 290.0 + CELL_GAP;

            gallery.cell(
                (CELL_GAP, top),
                (580.0, 290.0),
                "Confirm dialog: Cancel holds the focus",
                |context, area| {
                    let dialog = ConfirmDialog::new(
                        "delete",
                        "Delete game",
                        "Delete YU-NO: A Girl Who Chants Love at the Bound of this World?",
                        "Delete",
                    )
                    .detail(
                        "The file config/pc98/yu-no.conf is removed. The disk images stay where they are.",
                    )
                    .action_icon(Icon::Delete)
                    .danger(true)
                    .area(area);
                    dialog.show(context);
                },
                |harness, _, pass| {
                    harness.navigate(Navigation::Home);
                    pass(harness);
                },
            );
            let lines: [(&str, &[Hint], StatusEnd); 3] = [
                (
                    "Status line: library",
                    &[
                        Hint {
                            keys: &[HintKey::Key("Enter"), HintKey::Pad("A")],
                            label: "Launch",
                        },
                        Hint {
                            keys: &[HintKey::Key("F2"), HintKey::Pad("X")],
                            label: "Edit",
                        },
                        Hint {
                            keys: &[
                                HintKey::Key("Ctrl+PgUp"),
                                HintKey::Key("Ctrl+PgDn"),
                                HintKey::Shoulder("LB"),
                                HintKey::Shoulder("RB"),
                            ],
                            label: "System",
                        },
                        Hint {
                            keys: &[HintKey::Key("F10"), HintKey::Key("Start")],
                            label: "Settings",
                        },
                    ],
                    StatusEnd::Path("config/pc98/yu-no.conf"),
                ),
                (
                    "Status line: dialog",
                    &[
                        Hint {
                            keys: &[HintKey::Key("Enter"), HintKey::Pad("A")],
                            label: "Delete",
                        },
                        Hint {
                            keys: &[HintKey::Key("Esc"), HintKey::Pad("B")],
                            label: "Cancel",
                        },
                    ],
                    StatusEnd::Path("config/pc98/yu-no.conf"),
                ),
                (
                    "Status line: error",
                    &[Hint {
                        keys: &[HintKey::Key("Enter"), HintKey::Pad("A")],
                        label: "Delete",
                    }],
                    StatusEnd::Error("config/x68k/nemesis-90.conf line 4: unknown machine X68030"),
                ),
            ];
            for (index, (caption, hints, end)) in lines.into_iter().enumerate() {
                gallery.cell(
                    (
                        CELL_GAP * 2.0 + 580.0,
                        top + index as f32 * (62.0 + CELL_GAP),
                    ),
                    (700.0, 62.0),
                    caption,
                    |context, area| {
                        status_line(
                            context,
                            Rect::new(
                                0.0,
                                area.y + 4.0,
                                context.bounds().width,
                                STATUS_LINE_HEIGHT,
                            ),
                            hints,
                            end,
                        );
                    },
                    |harness, _, pass| pass(harness),
                );
            }
            top += 290.0 + CELL_GAP;

            gallery.cell(
                (CELL_GAP, top),
                (1276.0, 150.0),
                "Icons at 64, 16 and 12",
                |context, area| {
                    let theme = context.theme;
                    for (index, icon) in Icon::ALL.into_iter().enumerate() {
                        let x = area.x + index as f32 * 66.0;
                        let big = Rect::new(x, area.y, 64.0, 64.0);
                        context.fill(big, theme.well);
                        context.icon(big, icon, 64.0, theme.text);
                        context.icon(
                            Rect::new(x + 8.0, area.y + 72.0, 16.0, 16.0),
                            icon,
                            16.0,
                            theme.text,
                        );
                        context.icon(
                            Rect::new(x + 36.0, area.y + 74.0, 12.0, 12.0),
                            icon,
                            12.0,
                            theme.text,
                        );
                    }
                },
                |harness, _, pass| pass(harness),
            );
            gallery.write(&format!("gui_widgets_{scale}x"));
        }

        #[test]
        #[ignore = "writes a screenshot into the directory of GUI_SHOTS"]
        fn widget_gallery_screenshot() {
            render(1.0);
            render(2.0);
        }
    }
}
