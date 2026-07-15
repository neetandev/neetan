//! Roland MT-32 sound module device.
//!
//! Wraps the munt emulation core, managing the render thread internally.
//! The thread is an implementation detail - callers only interact with
//! [`Mt32::new`] (initialization) and [`Mt32::exchange`] (audio synchronization).

use std::path::Path;

use munt_oxide::MuntActor;
pub use munt_oxide::{MuntActorState, MuntError};
use save_state::ResourceBinding;

/// Roland MT-32 sound module.
///
/// Each audio chunk the emulation thread:
/// 1. Waits for the render thread to finish the previous chunk.
/// 2. Mixes the rendered audio into the output.
/// 3. Fills new MIDI data and signals the render thread.
pub struct Mt32 {
    actor: MuntActor,
}

impl Mt32 {
    /// Loads MT-32 ROMs from the given directory and starts the render thread.
    pub fn new(rom_directory: &Path) -> Result<Self, MuntError> {
        Ok(Self {
            actor: MuntActor::start(rom_directory)?,
        })
    }

    /// Waits for the render thread to finish, mixes audio into `output`,
    /// then fills new MIDI data via `fill` and signals the render thread.
    pub fn exchange(
        &mut self,
        volume: f32,
        output: &mut [f32],
        fill: impl FnOnce(&mut [u8]) -> usize,
    ) {
        self.actor.exchange(volume, output, fill);
    }

    /// Returns exact identities for the loaded control and PCM ROMs.
    pub fn resource_bindings(&self) -> &[ResourceBinding] {
        self.actor.resource_bindings()
    }

    /// Captures the worker and pending frontend buffers at a FIFO barrier.
    pub fn capture_state(&mut self) -> Result<MuntActorState, MuntError> {
        self.actor.capture_state()
    }

    /// Validates and stages worker state without changing the active synth.
    pub fn prepare_restore(&mut self, state: MuntActorState) -> Result<(), MuntError> {
        self.actor.prepare_restore(state)
    }

    /// Commits the prepared worker state.
    pub fn commit_restore(&mut self) -> Result<(), MuntError> {
        self.actor.commit_restore()
    }

    /// Aborts the prepared worker state.
    pub fn abort_restore(&mut self) -> Result<(), MuntError> {
        self.actor.abort_restore()
    }

    /// Resets the module and its streaming state.
    pub fn reset(&mut self) -> Result<(), MuntError> {
        self.actor.reset()
    }
}
