//! Display inspection: framebuffer reads, hashing, screenshots, and matching.

use std::path::PathBuf;

use sdl3::surface::Surface;

use super::{AutomationSession, OpError};
use crate::{capabilities::resolve_within, screen};

/// A decoded expected screen retained across a native wait.
struct ExpectedScreen {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Builds the error raised when a screen read happens before the first
/// presentation. The stable symbol set has no dedicated code, so this reuses
/// `neetan/argument` as a usage-precondition failure.
fn screen_unavailable() -> OpError {
    OpError::Argument("screen is not available until the first presentation".to_owned())
}

impl AutomationSession {
    /// Returns whether the current epoch has presented at least one frame.
    ///
    /// The screen is unavailable until the first presentation, so every other
    /// screen read fails until this is true.
    #[must_use]
    pub fn screen_available(&self) -> bool {
        self.has_machine() && self.timeline().epoch_frames > 0
    }

    /// Returns the valid framebuffer dimensions, or an error before the screen
    /// is available.
    fn screen_dimensions_checked(&self) -> Result<(u32, u32), OpError> {
        if !self.has_machine() {
            return Err(OpError::NoMachine);
        }
        if !self.screen_available() {
            return Err(screen_unavailable());
        }
        let machine = &self.active.as_ref().expect("machine present").machine;
        let (width, height) = machine.display_dimensions();
        if width == 0 || height == 0 {
            return Err(screen_unavailable());
        }
        Ok((width, height))
    }

    /// Returns the tightly packed valid RGBA8 region as `(width, height, bytes)`.
    fn screen_rgba_bytes(&self) -> Result<(u32, u32, Vec<u8>), OpError> {
        let (width, height) = self.screen_dimensions_checked()?;
        let machine = &self.active.as_ref().expect("machine present").machine;
        let needed = width as usize * height as usize * 4;
        let framebuffer = machine.display_framebuffer();
        if framebuffer.len() < needed {
            return Err(screen_unavailable());
        }
        Ok((width, height, framebuffer[..needed].to_vec()))
    }

    /// Returns the valid framebuffer dimensions.
    pub fn screen_size(&self) -> Result<(u32, u32), OpError> {
        self.screen_dimensions_checked()
    }

    /// Returns a copy of the tightly packed valid RGBA8 pixels.
    pub fn screen_rgba(&self) -> Result<Vec<u8>, OpError> {
        self.screen_rgba_bytes().map(|(_, _, bytes)| bytes)
    }

    /// Returns the `(red, green, blue, alpha)` bytes of one pixel.
    pub fn screen_pixel(&self, x: u32, y: u32) -> Result<(u8, u8, u8, u8), OpError> {
        let (width, height, bytes) = self.screen_rgba_bytes()?;
        if x >= width || y >= height {
            return Err(OpError::Range);
        }
        let base = (y as usize * width as usize + x as usize) * 4;
        Ok((
            bytes[base],
            bytes[base + 1],
            bytes[base + 2],
            bytes[base + 3],
        ))
    }

    /// Returns the 64-character lowercase hex BLAKE3 hash of the screen.
    pub fn screen_hash(&self) -> Result<String, OpError> {
        let (width, height, bytes) = self.screen_rgba_bytes()?;
        Ok(screen::screen_hash_hex(width, height, &bytes))
    }

    /// Encodes the current screen to PNG and writes it beneath the artifact root.
    pub fn save_screenshot(&mut self, path: &str) -> Result<PathBuf, OpError> {
        let (width, height, bytes) = self.screen_rgba_bytes()?;
        let surface = Surface::from_rgba8(width, height, &bytes)
            .map_err(|error| OpError::Io(format!("cannot build screenshot surface: {error}")))?;
        let png = surface
            .save_png()
            .map_err(|error| OpError::Io(format!("cannot encode screenshot: {error}")))?;
        self.write_artifact(path, &png)
    }

    /// Reports whether the screen matches the expected PNG within `tolerance`.
    ///
    /// Dimensions must match exactly. The metric is the normalized RGB RMSE with
    /// alpha ignored. This is a pure query and writes nothing.
    pub fn screen_matches(&self, expected_path: &str, tolerance: f64) -> Result<bool, OpError> {
        let (width, height, actual) = self.screen_rgba_bytes()?;
        let expected = self.read_expected_rgba(expected_path, width, height)?;
        Ok(screen::rgb_matches(
            &actual, &expected, width, height, tolerance,
        ))
    }

    /// Reports whether a screen region matches the expected PNG within
    /// `tolerance`. The expected image dimensions must equal the region size.
    pub fn screen_region_matches(
        &self,
        expected_path: &str,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        tolerance: f64,
    ) -> Result<bool, OpError> {
        let (full_width, full_height, full) = self.screen_rgba_bytes()?;
        let region = screen::extract_region(&full, full_width, full_height, x, y, width, height)
            .ok_or(OpError::Range)?;
        let expected = self.read_expected_rgba(expected_path, width, height)?;
        Ok(screen::rgb_matches(
            &region, &expected, width, height, tolerance,
        ))
    }

    /// Advances at presentation boundaries until the expected screen appears.
    ///
    /// The expected PNG is decoded once. An exhausted wait writes a best-effort
    /// comparison artifact and reports its path through the diagnostic stream.
    pub fn wait_for_screen(
        &mut self,
        expected_path: &str,
        tolerance: f64,
        maximum_frames: u64,
        maximum_ticks: u64,
    ) -> Result<bool, OpError> {
        if !self.has_machine() {
            return Err(OpError::NoMachine);
        }
        let expected = self.read_expected_screen(expected_path)?;
        let mut frames = 0u64;
        let mut remaining_ticks = maximum_ticks;

        loop {
            if self.current_screen_matches(&expected, tolerance)? {
                return Ok(true);
            }
            if frames >= maximum_frames || remaining_ticks == 0 {
                self.write_wait_comparison(expected_path, &expected);
                return Ok(false);
            }

            let outcome = self
                .advance_frames(1, remaining_ticks)
                .map_err(OpError::from_run)?;
            frames = frames.saturating_add(outcome.frames);
            remaining_ticks = remaining_ticks.saturating_sub(outcome.ticks);
            match outcome.stop_reason {
                common::StopReason::TargetReached => {}
                common::StopReason::GuestShutdown => return Err(OpError::GuestShutdown),
                common::StopReason::TickLimit
                | common::StopReason::Cancelled
                | common::StopReason::CounterExhausted
                | common::StopReason::MachineError => {
                    self.write_wait_comparison(expected_path, &expected);
                    return Ok(false);
                }
            }
        }
    }

    /// Writes a side-by-side comparison PNG (expected left, actual right) beneath
    /// the artifact root, for triaging a failed screen check.
    pub fn screen_comparison_image(
        &mut self,
        expected_path: &str,
        out_path: &str,
    ) -> Result<PathBuf, OpError> {
        let (width, height, actual) = self.screen_rgba_bytes()?;
        let expected = self.read_expected_rgba(expected_path, width, height)?;
        let combined = screen::side_by_side(&expected, &actual, width, height);
        let surface = Surface::from_rgba8(width * 2, height, &combined)
            .map_err(|error| OpError::Io(format!("cannot build comparison surface: {error}")))?;
        let png = surface
            .save_png()
            .map_err(|error| OpError::Io(format!("cannot encode comparison image: {error}")))?;
        self.write_artifact(out_path, &png)
    }

    /// Reads an expected PNG beneath the read root and returns its RGBA8 bytes,
    /// requiring the given exact dimensions.
    fn read_expected_rgba(&self, path: &str, width: u32, height: u32) -> Result<Vec<u8>, OpError> {
        let expected = self.read_expected_screen(path)?;
        if (expected.width, expected.height) != (width, height) {
            return Err(OpError::Argument(format!(
                "{path} is {}x{} but {width}x{height} was expected",
                expected.width, expected.height
            )));
        }
        Ok(expected.rgba)
    }

    /// Reads and decodes an expected PNG without constraining its dimensions.
    fn read_expected_screen(&self, path: &str) -> Result<ExpectedScreen, OpError> {
        let resolved = resolve_within(&self.read_root, path).map_err(OpError::PathEscape)?;
        let bytes = std::fs::read(&resolved)
            .map_err(|error| OpError::Io(format!("cannot read {path}: {error}")))?;
        let surface = Surface::load_png(&bytes)
            .map_err(|error| OpError::Io(format!("cannot decode {path}: {error}")))?;
        let (width, height) = surface.dimensions();
        let rgba = surface
            .to_rgba8()
            .map_err(|error| OpError::Io(format!("cannot read {path} pixels: {error}")))?;
        Ok(ExpectedScreen {
            width,
            height,
            rgba,
        })
    }

    /// Checks the current screen, treating unavailable or different sizes as a miss.
    fn current_screen_matches(
        &self,
        expected: &ExpectedScreen,
        tolerance: f64,
    ) -> Result<bool, OpError> {
        if !self.screen_available() {
            return Ok(false);
        }
        let (width, height, actual) = self.screen_rgba_bytes()?;
        if (width, height) != (expected.width, expected.height) {
            return Ok(false);
        }
        Ok(screen::rgb_matches(
            &actual,
            &expected.rgba,
            width,
            height,
            tolerance,
        ))
    }

    /// Writes and reports the final comparison for an exhausted screen wait.
    fn write_wait_comparison(&mut self, expected_path: &str, expected: &ExpectedScreen) {
        let output_path = comparison_output_name(expected_path);
        let result = self.write_wait_comparison_inner(&output_path, expected);
        match result {
            Ok(()) => self.emit_output(format!("artifact: {output_path}\n")),
            Err(error) => self.emit_output(format!(
                "comparison artifact unavailable for {expected_path}: {}\n",
                error.message()
            )),
        }
    }

    /// Builds the timeout comparison from the cached expected pixels.
    fn write_wait_comparison_inner(
        &mut self,
        output_path: &str,
        expected: &ExpectedScreen,
    ) -> Result<(), OpError> {
        let (actual_width, actual_height, actual) = self.screen_rgba_bytes()?;
        let (width, height, combined) = screen::side_by_side_native_size(
            &expected.rgba,
            expected.width,
            expected.height,
            &actual,
            actual_width,
            actual_height,
        )
        .ok_or(OpError::Range)?;
        let surface = Surface::from_rgba8(width, height, &combined)
            .map_err(|error| OpError::Io(format!("cannot build comparison surface: {error}")))?;
        let png = surface
            .save_png()
            .map_err(|error| OpError::Io(format!("cannot encode comparison: {error}")))?;
        self.write_artifact(output_path, &png)?;
        Ok(())
    }

    /// Resolves an artifact path, charges the byte budget, and writes the bytes.
    fn write_artifact(&mut self, path: &str, bytes: &[u8]) -> Result<PathBuf, OpError> {
        let resolved = resolve_within(&self.artifact_root, path).map_err(OpError::PathEscape)?;
        self.charge_artifact_bytes(bytes.len() as u128)?;
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                OpError::Io(format!("cannot create artifact directory: {error}"))
            })?;
        }
        std::fs::write(&resolved, bytes)
            .map_err(|error| OpError::Io(format!("cannot write artifact {path}: {error}")))?;
        Ok(resolved)
    }

    /// Charges `bytes` against the artifact-byte budget when one is set.
    fn charge_artifact_bytes(&mut self, bytes: u128) -> Result<(), OpError> {
        if let Some(remaining) = self.budgets.artifact_bytes.as_mut() {
            if *remaining < bytes {
                *remaining = 0;
                return Err(OpError::Io("artifact byte budget exhausted".to_owned()));
            }
            *remaining -= bytes;
        }
        Ok(())
    }
}

/// Derives the comparison artifact name from an expected image path.
fn comparison_output_name(expected_path: &str) -> String {
    let stem = std::path::Path::new(expected_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("screen");
    format!("{stem}-compare.png")
}
