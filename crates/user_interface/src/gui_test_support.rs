//! The font, the pass state and the screenshot output of the user interface
//! tests without a window.

use std::path::{Path, PathBuf};

use gui::{
    Fonts, Rasterizer, Scene,
    ui::{Input, Theme, UiContext, UiState, Viewport},
};

/// Returns the font file of the repository.
pub(crate) fn repository_font() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(machine_factory::config::DEFAULT_GUI_FONT);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The fonts, the theme and the pass state of a user interface without a window.
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
}

impl GuiHarness {
    /// A harness of `width` x `height` physical pixels at scale 1.
    pub(crate) fn new(width: u32, height: u32) -> Self {
        Self::with_viewport(Viewport::new(width, height, 1.0))
    }

    /// A harness with the target `viewport`.
    pub(crate) fn with_viewport(viewport: Viewport) -> Self {
        let mut fonts = Fonts::new();
        let font = fonts
            .add(repository_font())
            .expect("the repository font parses");
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

    /// Runs one pass of `draw` with the pending input.
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

    /// Draws the scene of the last pass through the CPU rasterizer into RGBA8 pixels.
    pub(crate) fn render(&self) -> Vec<u8> {
        let Viewport { width, height, .. } = self.viewport;
        let mut pixels = vec![0; width as usize * height as usize * 4];
        Rasterizer::new().rasterize(&self.scene, self.fonts.slug(), width, height, &mut pixels);
        pixels
    }

    /// Writes the scene of the last pass as `<name>.ppm` into the directory
    /// of the `GUI_SHOTS` environment variable.
    pub(crate) fn write_screenshot(&self, name: &str) {
        let Viewport { width, height, .. } = self.viewport;
        write_ppm(name, width, height, &self.render());
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
