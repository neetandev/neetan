//! Read-only decoded text-surface inspection and text waiting.
//!
//! Text decoding is side-effect-free: it reads the current text VRAM through the
//! machine's [`TextSurfaceInspector`] and never performs a device read. The
//! `wait-for-text` loop drives the machine one presented frame at a time and
//! decodes the surface after each frame, stopping at the first substring match.

use common::{
    InspectError, RunRequest, RunTarget, StopReason, TextCell, TextSurfaceInfo,
    TextSurfaceInspector,
};

use super::{AutomationSession, INPUT_DRAIN_INTERVAL_TICKS, OpError};

/// A parsed `wait-for-text` predicate.
pub(crate) struct TextMatch {
    /// The surface identifier to inspect.
    pub(crate) surface: String,
    /// A single row to scan, or `None` to scan the whole screen.
    pub(crate) row: Option<u16>,
    /// The substring that must appear for the wait to succeed.
    pub(crate) contains: String,
}

/// Maps a text-inspection failure to the automation error contract.
fn map_text_error(error: InspectError) -> OpError {
    match error {
        InspectError::UnknownSpace => {
            OpError::Argument("unknown text surface identifier".to_owned())
        }
        InspectError::OutOfRange => OpError::Range,
        InspectError::UnknownProcessor
        | InspectError::UnknownRegister
        | InspectError::NotWritable
        | InspectError::NotPeekable
        | InspectError::Unsupported => {
            OpError::Unsupported("operation is not supported by this text surface".to_owned())
        }
    }
}

/// Renders a row of decoded cells to a UTF-8 string, collapsing full-width
/// continuation cells and mapping unmapped codes to a space.
fn text_cells_to_string(cells: &[TextCell]) -> String {
    let mut output = String::with_capacity(cells.len());
    let mut skip_continuation = false;
    for cell in cells {
        if skip_continuation {
            skip_continuation = false;
            continue;
        }
        match cell.unicode {
            Some(character) => output.push(character),
            None => output.push(' '),
        }
        if cell.display_width == 2 {
            skip_continuation = true;
        }
    }
    output
}

/// Renders a full decoded screen to UTF-8 rows joined by newlines, trimming
/// trailing spaces from each row.
fn text_screen_to_string(rows: &[Vec<TextCell>]) -> String {
    let mut output = String::new();
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        let line = text_cells_to_string(row);
        output.push_str(line.trim_end());
    }
    output
}

impl AutomationSession {
    /// Returns the text inspector, or the precondition failure that blocks it.
    fn text_inspector(&self) -> Result<&dyn TextSurfaceInspector, OpError> {
        let machine = &self.active.as_ref().ok_or(OpError::NoMachine)?.machine;
        machine.text_inspector().ok_or_else(|| {
            OpError::Unsupported("machine does not support text inspection".to_owned())
        })
    }

    /// Returns whether this machine exposes a text inspector.
    pub fn supports_text_inspection(&self) -> bool {
        self.text_inspector().is_ok()
    }

    /// Returns the identifiers of every inspectable text surface.
    pub fn text_surfaces(&self) -> Result<Vec<&'static str>, OpError> {
        let inspector = self.text_inspector()?;
        Ok(inspector
            .text_surfaces()
            .iter()
            .map(|info| info.id)
            .collect())
    }

    /// Returns the geometry of one text surface.
    pub fn text_surface_info(&self, surface: &str) -> Result<TextSurfaceInfo, OpError> {
        self.text_inspector()?
            .text_surface_info(surface)
            .map_err(map_text_error)
    }

    /// Decodes one text cell of a surface.
    pub fn text_cell(&self, surface: &str, row: u16, column: u16) -> Result<TextCell, OpError> {
        self.text_inspector()?
            .text_cell(surface, row, column)
            .map_err(map_text_error)
    }

    /// Decodes every row of a surface, top to bottom.
    pub fn text_screen(&self, surface: &str) -> Result<Vec<Vec<TextCell>>, OpError> {
        self.text_inspector()?
            .text_screen(surface)
            .map_err(map_text_error)
    }

    /// Decodes every row of a surface to a UTF-8 line string, in order.
    pub fn text_screen_lines(&self, surface: &str) -> Result<Vec<String>, OpError> {
        let rows = self.text_screen(surface)?;
        Ok(rows.iter().map(|row| text_cells_to_string(row)).collect())
    }

    /// Decodes a surface to UTF-8 rows and writes it to an artifact.
    pub fn save_text_screen(
        &mut self,
        surface: &str,
        path: &str,
    ) -> Result<std::path::PathBuf, OpError> {
        let rows = self.text_screen(surface)?;
        let text = text_screen_to_string(&rows);
        self.write_artifact(path, text.as_bytes())
    }

    /// Returns the matched text when the predicate holds on the current screen.
    fn text_matches(&self, predicate: &TextMatch) -> Result<Option<String>, OpError> {
        let inspector = self.text_inspector()?;
        let haystack = match predicate.row {
            Some(row) => {
                let cells = inspector
                    .text_row(&predicate.surface, row)
                    .map_err(map_text_error)?;
                text_cells_to_string(&cells)
            }
            None => {
                let rows = inspector
                    .text_screen(&predicate.surface)
                    .map_err(map_text_error)?;
                text_screen_to_string(&rows)
            }
        };
        if haystack.contains(&predicate.contains) {
            Ok(Some(haystack))
        } else {
            Ok(None)
        }
    }

    /// Drives the machine one frame at a time until the predicate matches or the
    /// explicit bounds are exhausted, returning the matched text on success.
    ///
    /// The live text plane is decoded once up front and then after every frame,
    /// so matching happens at frame granularity. Text that appears and vanishes
    /// within a single frame is not observed.
    pub(crate) fn wait_for_text(
        &mut self,
        predicate: TextMatch,
        max_frames: u64,
        max_ticks: u64,
    ) -> Result<Option<String>, OpError> {
        self.text_surface_info(&predicate.surface)?;
        if let Some(text) = self.text_matches(&predicate)? {
            return Ok(Some(text));
        }
        let mut presented = 0u64;
        let mut remaining_ticks = max_ticks;
        loop {
            if self.is_stopped() {
                return Ok(None);
            }
            if presented >= max_frames
                || remaining_ticks == 0
                || self.tick_budget_exhausted()
                || self.frame_budget_exhausted()
            {
                return Ok(None);
            }
            let request = RunRequest {
                target: RunTarget::Frames(1),
                max_ticks: remaining_ticks,
                audio_drain_interval_ticks: INPUT_DRAIN_INTERVAL_TICKS,
            };
            let outcome = self
                .active
                .as_mut()
                .expect("machine present")
                .machine
                .run_automation(request);
            self.consume_budget(&outcome);
            remaining_ticks = remaining_ticks.saturating_sub(outcome.ticks);
            presented = presented.saturating_add(outcome.frames);
            if outcome.stop_reason == StopReason::GuestShutdown {
                return Err(OpError::GuestShutdown);
            }
            if let Some(text) = self.text_matches(&predicate)? {
                return Ok(Some(text));
            }
        }
    }
}
