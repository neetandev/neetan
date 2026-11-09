//! The font, the pass state and the screenshot output of the user interface
//! tests without a window.

use std::{
    convert::Infallible,
    path::{Path, PathBuf},
};

use gui::{
    Fonts, Instance, Rasterizer, Rect, Scene,
    component::{Component, Ended, Layer, LayerRequests, Status, Update, run, run_child},
    ui::{
        Button, Input, KEYCAP_HEIGHT, STATUS_LINE_HEIGHT, StatusEnd, Theme, ThemeKind, UiContext,
        UiId, UiState, Viewport, status_line,
    },
};
use i18n::Catalog;

use crate::{
    host::MINIMUM_WINDOW_SIZE,
    texts::{ENGLISH_TAG, Hints, LanguageCatalog, repository::repository_languages},
};

/// The width of a window of the minimum size of the manager.
pub(crate) const MINIMUM_WIDTH: f32 = MINIMUM_WINDOW_SIZE.0 as f32;

/// The height of a window of the minimum size of the manager.
pub(crate) const MINIMUM_HEIGHT: f32 = MINIMUM_WINDOW_SIZE.1 as f32;

/// A window of the minimum size of the manager.
pub(crate) const MINIMUM_WINDOW: Rect = Rect::new(0.0, 0.0, MINIMUM_WIDTH, MINIMUM_HEIGHT);

/// The area above the status line of a window of the minimum size of the manager.
pub(crate) const MINIMUM_SCREEN: Rect =
    Rect::new(0.0, 0.0, MINIMUM_WIDTH, MINIMUM_HEIGHT - STATUS_LINE_HEIGHT);

/// Returns the font file of the repository.
pub(crate) fn repository_font() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(machine_factory::config::DEFAULT_GUI_FONT);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The allocations, the text fills and the wraps of a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PassCounts {
    /// The allocations and reallocations of the pass.
    pub(crate) allocations: usize,
    /// The fills of the text buffers of the pass.
    pub(crate) fills: usize,
    /// The wraps of the wrapped texts of the pass.
    pub(crate) wraps: usize,
}

impl PassCounts {
    /// The counts of a pass that allocates nothing, fills no text and wraps no text.
    pub(crate) const NONE: Self = Self {
        allocations: 0,
        fills: 0,
        wraps: 0,
    };
}

/// The fonts, the theme, the catalog and the pass state of a user interface
/// without a window.
pub(crate) struct GuiHarness {
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
    /// The kind of the theme.
    kind: ThemeKind,
    /// The texts of the passes.
    catalog: LanguageCatalog,
    /// The generation of the catalog. Each switch of the catalog counts it up.
    catalog_generation: u64,
}

impl GuiHarness {
    /// A harness in the dark theme with the English texts of a window of the
    /// minimum size of the manager at scale 1.
    pub(crate) fn minimum() -> Self {
        let (width, height) = MINIMUM_WINDOW_SIZE;
        Self::new(width, height)
    }

    /// A harness in the dark theme with the English texts of `width` x
    /// `height` physical pixels at scale 1.
    pub(crate) fn new(width: u32, height: u32) -> Self {
        Self::with_viewport(
            Viewport::new(width, height, 1.0),
            ThemeKind::Dark,
            LanguageCatalog::English,
        )
    }

    /// A harness in the theme `kind` with the texts of `catalog` and the
    /// target `viewport`.
    pub(crate) fn with_viewport(
        viewport: Viewport,
        kind: ThemeKind,
        catalog: LanguageCatalog,
    ) -> Self {
        let mut fonts = Fonts::new();
        let font = fonts
            .add(repository_font())
            .expect("the repository font parses");
        let theme = Theme::new(&mut fonts, font, kind);
        Self {
            fonts,
            theme,
            scene: Scene::new(),
            state: UiState::default(),
            input: Input::default(),
            viewport,
            kind,
            catalog,
            catalog_generation: 0,
        }
    }

    /// Replaces the catalog with `catalog` and counts the catalog generation
    /// up.
    pub(crate) fn set_catalog(&mut self, catalog: LanguageCatalog) {
        self.catalog = catalog;
        self.catalog_generation += 1;
    }

    /// Runs one pass of `draw` with the pending input.
    pub(crate) fn frame<R>(&mut self, draw: impl FnOnce(&mut UiContext) -> R) -> R {
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
        result
    }

    /// Runs one pass of `draw` with the pending input and counts its allocations, its text
    /// fills and its wraps.
    pub(crate) fn counted_frame<R>(
        &mut self,
        draw: impl FnOnce(&mut UiContext) -> R,
    ) -> (PassCounts, R) {
        let (allocations, (fills, (wraps, result))) = allocation_counter::count_allocations(|| {
            gui::ui::count_fills(|| gui::ui::count_wraps(|| self.frame(draw)))
        });
        let counts = PassCounts {
            allocations,
            fills,
            wraps,
        };
        (counts, result)
    }

    /// Runs one pass of `draw` with the pending input and then the passes
    /// that the widgets ask for, at most four, as the GUI host does.
    /// Returns the result of the last pass.
    pub(crate) fn settle<R>(&mut self, mut draw: impl FnMut(&mut UiContext) -> R) -> R {
        let mut result = self.frame(&mut draw);
        for _ in 0..4 {
            if !self.state.take_pass_request() {
                break;
            }
            result = self.frame(&mut draw);
        }
        result
    }

    /// Runs one pass of `component` in `area` with `input` and the pending input, and then the
    /// passes that it asks for, at most four, as the GUI host does. Stops after the first pass
    /// that ends the component. Returns the status of the last pass and the effects of every
    /// pass.
    pub(crate) fn run_component<'input, C: Component>(
        &mut self,
        component: &mut C,
        area: Rect,
        input: C::Input<'input>,
    ) -> (Status<C::Output>, Vec<C::Effect>)
    where
        C::Input<'input>: Copy,
    {
        let mut effects = Vec::new();
        let mut pass = |harness: &mut Self| {
            harness.frame(|context| {
                run(component, context, area, input, &mut |effect| {
                    effects.push(effect);
                })
            })
        };
        let mut status = pass(self);
        for _ in 0..4 {
            if !matches!(status, Status::Open) || !self.state.take_pass_request() {
                break;
            }
            status = pass(self);
        }
        (status, effects)
    }

    /// Draws the scene of the last pass through the CPU rasterizer into RGBA8 pixels.
    pub(crate) fn render(&self) -> Vec<u8> {
        let Viewport { width, height, .. } = self.viewport;
        let mut pixels = vec![0; width as usize * height as usize * 4];
        Rasterizer::new().rasterize(&self.scene, self.fonts.slug(), width, height, &mut pixels);
        pixels
    }

    /// Writes the scene of the last pass as `<name>.ppm` into the directory
    /// of the `GUI_SHOTS` environment variable. A light theme writes
    /// `light_<name>.ppm`. A catalog of another language than English puts
    /// its tag in front, for example `ja_light_<name>.ppm`.
    pub(crate) fn write_screenshot(&self, name: &str) {
        let Viewport { width, height, .. } = self.viewport;
        let name = match self.kind {
            ThemeKind::Dark => name.to_owned(),
            ThemeKind::Light => format!("light_{name}"),
        };
        let tag = self.catalog.catalog().tag();
        let name = if tag == ENGLISH_TAG {
            name
        } else {
            format!("{tag}_{name}")
        };
        write_ppm(&name, width, height, &self.render());
    }
}

/// Applies `message` to `component` outside of a pass with the English texts. Returns the status
/// of the update and the ids of the layers that it opens for the next pass.
pub(crate) fn apply<C: Component<Effect = Infallible>>(
    component: &mut C,
    message: C::Message,
) -> (Status<C::Output>, Vec<UiId>) {
    let (status, layers, _) = apply_with_effects(component, message);
    (status, layers)
}

/// Applies `message` to `component` outside of a pass with the English texts. Returns the status
/// of the update, the ids of the layers that it opens for the next pass and its effects.
pub(crate) fn apply_with_effects<C: Component>(
    component: &mut C,
    message: C::Message,
) -> (Status<C::Output>, Vec<UiId>, Vec<C::Effect>) {
    let mut layers = LayerRequests::new();
    let mut effects = Vec::new();
    let mut emit = |effect| effects.push(effect);
    let catalog = LanguageCatalog::English;
    let mut update = Update::new(catalog.catalog(), &mut emit, &mut layers);
    let status = component.update(message, &mut update);
    (status, layers.ids().to_vec(), effects)
}

/// Asserts that the status line of a window of the minimum size shows every key cap of the
/// hints that `hints` gives in each language of the repository.
pub(crate) fn assert_hints_fit(hints: impl for<'c> Fn(&'c Catalog) -> Hints<'c>) {
    let mut harness = GuiHarness::minimum();
    let line = Rect::new(
        0.0,
        MINIMUM_SCREEN.height,
        MINIMUM_WIDTH,
        STATUS_LINE_HEIGHT,
    );
    for language in repository_languages() {
        harness.set_catalog(language.catalog());
        let (report, keys) = harness.frame(|context| {
            let catalog = context.catalog;
            let hints = hints(catalog);
            status_line(context, line, &hints, StatusEnd::None);
            let labels: Vec<&str> = hints.iter().map(|hint| hint.label).collect();
            let keys: usize = hints.iter().map(|hint| hint.keys.len()).sum();
            (format!("{}: {}", catalog.tag(), labels.join(", ")), keys)
        });
        let caps = harness
            .scene
            .instances()
            .iter()
            .filter(|instance| {
                instance.kind[0] == Instance::KIND_RECTANGLE && instance.rect[3] == KEYCAP_HEIGHT
            })
            .count();
        assert_eq!(caps, keys, "{report}");
    }
}

/// Writes the RGBA8 `pixels` as `<name>.ppm` into the directory of the
/// `GUI_SHOTS` environment variable. Without the variable nothing is written.
pub(crate) fn write_ppm(name: &str, width: u32, height: u32, pixels: &[u8]) {
    let Some(directory) = std::env::var_os("GUI_SHOTS") else {
        eprintln!("GUI_SHOTS is not set, {name} is not written");
        return;
    };
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory).expect("the screenshot folder is created");
    let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
    for pixel in pixels.as_chunks::<4>().0 {
        bytes.extend_from_slice(&pixel[..3]);
    }
    std::fs::write(directory.join(format!("{name}.ppm")), bytes)
        .expect("the screenshot is written");
}

/// The id of the button of the [`AskScreen`] that opens its dialog.
pub(crate) const ASK_BUTTON: &str = "ask";

/// The id of the button of the [`AnswerDialog`].
pub(crate) const ANSWER_BUTTON: &str = "answer";

/// The id of the modal layer of the [`AnswerDialog`].
const ANSWER_LAYER: &str = "answer dialog";

/// What the [`AskScreen`] reports when its dialog was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Answered;

/// A message of the [`AskScreen`].
pub(crate) enum AskMessage {
    /// The ask button opens the dialog.
    Ask,
    /// The dialog ended.
    Answer(Ended<()>),
}

/// A screen with a button that opens an [`AnswerDialog`] in an update, for the tests of the
/// passes of a component.
#[derive(Default)]
pub(crate) struct AskScreen {
    /// The message buffer.
    messages: Vec<AskMessage>,
    /// The open dialog.
    dialog: Option<AnswerDialog>,
}

impl Component for AskScreen {
    type Message = AskMessage;
    type Output = Infallible;
    type Effect = Answered;
    type Input<'a> = ();

    fn messages(&mut self) -> &mut Vec<AskMessage> {
        &mut self.messages
    }

    fn view(
        &mut self,
        context: &mut UiContext,
        area: Rect,
        _input: (),
        messages: &mut Vec<AskMessage>,
    ) {
        if Button::new(ASK_BUTTON, "Ask").show(context, Rect::new(area.x, area.y, 120.0, 32.0)) {
            messages.push(AskMessage::Ask);
        }
        if let Some(dialog) = self.dialog.as_mut() {
            run_child(dialog, context, area, (), messages, |event| {
                AskMessage::Answer(event.into_ended())
            });
        }
    }

    fn update(
        &mut self,
        message: AskMessage,
        update: &mut Update<'_, Answered>,
    ) -> Status<Infallible> {
        match message {
            AskMessage::Ask => {
                self.dialog = Some(AnswerDialog::default());
                update.open::<AnswerDialog>();
            }
            AskMessage::Answer(ended) => {
                self.dialog = None;
                if ended == Ended::Done(()) {
                    update.emit(Answered);
                }
            }
        }
        Status::Open
    }
}

/// A dialog in a modal layer with one button that answers it.
#[derive(Default)]
pub(crate) struct AnswerDialog {
    /// The message buffer.
    messages: Vec<()>,
}

impl Component for AnswerDialog {
    type Message = ();
    type Output = ();
    type Effect = Infallible;
    type Input<'a> = ();

    const LAYER: Layer = Layer::Modal(ANSWER_LAYER);

    fn messages(&mut self) -> &mut Vec<()> {
        &mut self.messages
    }

    fn view(&mut self, context: &mut UiContext, area: Rect, _input: (), messages: &mut Vec<()>) {
        let button = Rect::new(area.x + 200.0, area.y + 200.0, 120.0, 32.0);
        if Button::new(ANSWER_BUTTON, "Answer").show(context, button) {
            messages.push(());
        }
    }

    fn update(&mut self, _message: (), _update: &mut Update<'_, Infallible>) -> Status<()> {
        Status::Done(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that `run_component` runs a component until it settles and returns its status
    /// and its effects, and that a dialog that an update opens draws and takes the focus
    /// within the settled passes.
    #[test]
    fn run_component_settles_and_returns_the_effects() {
        let mut harness = GuiHarness::minimum();
        let mut screen = AskScreen::default();
        harness.state.set_focus(UiId::from(ASK_BUTTON));
        assert_eq!(
            harness.run_component(&mut screen, MINIMUM_SCREEN, ()),
            (Status::Open, Vec::new())
        );
        harness.input.confirm = true;
        assert_eq!(
            harness.run_component(&mut screen, MINIMUM_SCREEN, ()),
            (Status::Open, Vec::new())
        );
        assert!(!harness.state.take_pass_request());
        assert_eq!(harness.state.open_modals(), 1);
        assert_eq!(harness.state.focused(), Some(UiId::from(ANSWER_BUTTON)));
        harness.input.confirm = true;
        assert_eq!(
            harness.run_component(&mut screen, MINIMUM_SCREEN, ()),
            (Status::Open, vec![Answered])
        );
        assert_eq!(harness.state.open_modals(), 0);
        assert_eq!(harness.state.focused(), Some(UiId::from(ASK_BUTTON)));
    }
}
