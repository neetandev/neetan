//! The display settings of a session and the frame of its machine.

use common::info;
use sdl3_backend::RenderInstructions;

use super::{Session, catch_panic};
use crate::{
    config::{Backend, ScalingMode},
    host::{Host, graphics_scaling},
    on_off,
};

/// The number of composite artifact phases.
const COMPOSITE_PHASE_COUNT: u32 = 4;

impl Session {
    /// The frame of the machine for the graphics engine, or the message of a
    /// panic while reading it.
    pub(crate) fn render_instructions(
        &self,
    ) -> std::result::Result<RenderInstructions<'_>, String> {
        let machine = self.machine.as_ref();
        let (width, height) = catch_panic(|| machine.display_dimensions())?;
        let framebuffer = catch_panic(|| machine.display_framebuffer())?;
        Ok(RenderInstructions {
            framebuffer,
            width,
            height,
            crt: self.crt_enabled,
            composite: self.config.target.has_composite_video(),
            composite_phase: self.composite_phase,
        })
    }

    /// Switches the CRT effect on or off. The legacy backend has no CRT effect.
    pub(super) fn toggle_crt(&mut self, host: &Host) {
        if host.backend == Backend::Legacy {
            return;
        }
        self.crt_enabled = !self.crt_enabled;
        info!("CRT effect set to {}", on_off(self.crt_enabled));
    }

    /// Selects the next composite artifact phase on a machine with composite
    /// video.
    pub(super) fn cycle_composite_phase(&mut self) {
        if !self.config.target.has_composite_video() {
            return;
        }
        self.composite_phase = (self.composite_phase + 1) % COMPOSITE_PHASE_COUNT;
        info!("Composite artifact phase set to {}", self.composite_phase);
    }

    /// Selects the next scaling method of the native texture.
    pub(super) fn toggle_scaling(&mut self, host: &mut Host) {
        self.scaling = match self.scaling {
            ScalingMode::Nearest => ScalingMode::Bilinear,
            ScalingMode::Bilinear => ScalingMode::Pixelart,
            ScalingMode::Pixelart => ScalingMode::Nearest,
        };
        host.graphics_engine
            .set_scaling(graphics_scaling(self.scaling));
        info!("Scaling set to {}", self.scaling);
    }
}

#[cfg(test)]
mod tests {
    use crate::session::test_support::{CallLog, FakeMachine, fake_machine, session_with};

    #[test]
    fn reading_the_frame_reports_a_panic() {
        let log = CallLog::default();
        let session = session_with(fake_machine(&log));
        let instructions = session.render_instructions().unwrap();
        assert_eq!((instructions.width, instructions.height), (2, 1));
        assert_eq!(instructions.framebuffer.len(), 8);

        let broken = session_with(FakeMachine {
            panic_on_framebuffer: true,
            ..fake_machine(&log)
        });
        assert_eq!(
            broken.render_instructions().err(),
            Some("the frame is unreadable".to_owned())
        );
    }
}
