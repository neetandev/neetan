//! The font, the pass state and the screenshot output of the user interface
//! tests without a window.

use std::path::{Path, PathBuf};

use gui::{
    Fonts, Rasterizer, Scene,
    ui::{Input, Theme, ThemeKind, UiContext, UiState, Viewport},
};

use crate::texts::{ENGLISH_TAG, LanguageCatalog, repository::japanese_catalog};

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

/// Returns the embedded English catalog.
fn english_catalog() -> LanguageCatalog {
    LanguageCatalog::English
}

/// The catalogs of the languages of the screenshots: English and Japanese.
pub(crate) const SCREENSHOT_LANGUAGES: [fn() -> LanguageCatalog; 2] =
    [english_catalog, japanese_catalog];

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
