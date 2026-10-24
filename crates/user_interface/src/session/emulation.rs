//! The emulation of a session: the audio paced frame, fast forward and the
//! hard reset.

use std::time::{Duration, Instant};

use common::{info, warn};

use super::{Session, SessionExit};
use crate::{
    host::Host,
    input::KeyboardForwardingState,
    machines::{initialize_machine, sdl_host_date_time_source},
};

/// The most audio steps of one emulation frame.
const MAX_AUDIO_STEPS: usize = 40;

/// The audio sample rate as a float.
const SAMPLE_RATE: f64 = audio_engine::SAMPLE_RATE as f64;

/// Emulation speed multiplier applied while fast forward is held.
const FAST_FORWARD_FACTOR: f64 = 8.0;

/// Upper bound on the wall-clock interval used to pace a fast-forward step,
/// preventing a large burst after a host stall.
const FAST_FORWARD_MAX_ELAPSED: Duration = Duration::from_millis(100);

impl Session {
    /// Runs the emulation for one frame and ticks the text extractor.
    pub(crate) fn run_frame(&mut self, host: &mut Host) {
        self.run_emulation(host);
        self.machine.tick_text_extractor();
    }

    /// Flushes the media of the machine, creates the machine again and
    /// inserts the first image of every drive. A failed flush is logged.
    pub(super) fn hard_reset(&mut self, host: &mut Host) {
        let result = super::flush_machine_media(self.machine.as_mut());
        self.keep_flush_result(result);

        let mut machine = match initialize_machine(
            &self.config,
            audio_engine::SAMPLE_RATE as u32,
            sdl_host_date_time_source(),
        ) {
            Ok(machine) => machine,
            Err(error) => {
                warn!("Hard reset failed to re-create the machine: {error:#}");
                return;
            }
        };

        if self.config.enable_extractor {
            machine.install_text_extractor(Box::new(text_extractor::ClipboardExtractor::new()));
        }

        let mut fdd1_index = None;
        if let Some(entry) = self.fdd1_entries.first() {
            match machine.insert_floppy_from_path(0, &entry.path) {
                Ok(desc) => {
                    info!("Re-inserted FDD1: {desc} from {}", entry.path.display());
                    fdd1_index = Some(0);
                }
                Err(error) => warn!("Hard reset failed to re-insert FDD1: {error}"),
            }
        }
        let mut fdd2_index = None;
        if let Some(entry) = self.fdd2_entries.first() {
            match machine.insert_floppy_from_path(1, &entry.path) {
                Ok(desc) => {
                    info!("Re-inserted FDD2: {desc} from {}", entry.path.display());
                    fdd2_index = Some(0);
                }
                Err(error) => warn!("Hard reset failed to re-insert FDD2: {error}"),
            }
        }
        let mut cdrom_index = None;
        if let Some(entry) = self.cdrom_entries.first() {
            match machine.insert_cdrom(&entry.path) {
                Ok(desc) => {
                    info!("Re-inserted CD-ROM: {desc} from {}", entry.path.display());
                    cdrom_index = Some(0);
                }
                Err(error) => warn!("Hard reset failed to re-insert CD-ROM: {error}"),
            }
        }

        self.machine = machine;
        self.fdd1_index = fdd1_index;
        self.fdd2_index = fdd2_index;
        self.cdrom_index = cdrom_index;
        self.cpu_hz = self.machine.cpu_clock_hz();
        self.cycle_overshoot = 0;
        self.keyboard_forwarding_state = KeyboardForwardingState::new();
        host.audio_engine.reset_buffer();
        info!("Hard reset complete");
    }

    /// Starts or stops fast forward. Fast forward pauses the audio.
    pub(super) fn set_fast_forward(&mut self, enabled: bool, host: &mut Host) {
        if enabled == self.fast_forward {
            return;
        }
        self.fast_forward = enabled;
        if enabled {
            self.last_emulation_tick = Instant::now();
            host.audio_engine.pause();
        } else {
            host.audio_engine.reset_buffer();
            host.audio_engine.resume();
        }
    }

    /// Runs the machine until the audio engine holds its target buffer, or
    /// at the fast forward rate while fast forward is held.
    fn run_emulation(&mut self, host: &mut Host) {
        if self.media_dialog.is_some() {
            return;
        }

        if self.mouse_captured {
            let dx = self.mouse_dx.round() as i16;
            let dy = self.mouse_dy.round() as i16;
            self.machine.push_mouse_delta(dx, dy);
            self.mouse_dx = 0.0;
            self.mouse_dy = 0.0;
        }

        if self.fast_forward {
            self.run_emulation_fast_forward(host);
            return;
        }

        let audio_engine = &mut host.audio_engine;
        for _ in 0..MAX_AUDIO_STEPS {
            let needed_frames = audio_engine.frames_needed() as u64;
            if needed_frames == 0 {
                break;
            }
            let raw_cycles = (needed_frames as f64 * self.cpu_hz / SAMPLE_RATE).round() as u64;

            // A previous step that overshot by more cycles than this step
            // needs pays for this step. The remainder carries over.
            if self.cycle_overshoot >= raw_cycles {
                self.cycle_overshoot -= raw_cycles;
                audio_engine.push_samples(self.machine.as_mut());
                continue;
            }

            let cycles = raw_cycles - self.cycle_overshoot;
            self.cycle_overshoot = 0;

            let ran_cycles = self.machine.run_for(cycles);
            if ran_cycles > cycles {
                self.cycle_overshoot = ran_cycles - cycles;
            }
            audio_engine.push_samples(self.machine.as_mut());

            if self.machine.shutdown_requested() {
                info!("Guest triggered system shutdown");
                self.request_exit(SessionExit::GuestShutdown);
                return;
            }
        }
    }

    /// Runs the machine at the fast forward rate for the time since the last
    /// step and drops its audio.
    fn run_emulation_fast_forward(&mut self, host: &mut Host) {
        let elapsed = self
            .last_emulation_tick
            .elapsed()
            .min(FAST_FORWARD_MAX_ELAPSED);
        self.last_emulation_tick = Instant::now();

        let target_cycles = (elapsed.as_secs_f64() * self.cpu_hz * FAST_FORWARD_FACTOR) as u64;
        let chunk_cycles =
            (audio_engine::STEP_FRAMES as f64 * self.cpu_hz / SAMPLE_RATE).round() as u64;
        if chunk_cycles == 0 {
            return;
        }

        let mut ran = 0u64;
        while ran < target_cycles {
            self.machine.run_for(chunk_cycles);
            host.audio_engine.discard_samples(self.machine.as_mut());
            ran += chunk_cycles;

            if self.machine.shutdown_requested() {
                info!("Guest triggered system shutdown");
                self.request_exit(SessionExit::GuestShutdown);
                return;
            }
        }
    }
}
