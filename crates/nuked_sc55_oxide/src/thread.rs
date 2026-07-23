/*
 * Copyright (C) 2021, 2024 nukeykt
 *
 *  Redistribution and use of this code or any derivative works are permitted
 *  provided that the following conditions are met:
 *
 *   - Redistributions may not be sold, nor may they be used in a commercial
 *     product or activity.
 *
 *   - Redistributions that are modified from the original source must include the
 *     complete source code, including the source code for all components used by a
 *     binary built from the modified sources. However, as a special exception, the
 *     source code distributed need not include anything that is normally distributed
 *     (in either source or binary form) with the major components (compiler, kernel,
 *     and so on) of the operating system on which the executable runs, unless that
 *     component itself accompanies the executable.
 *
 *   - Redistributions must reproduce the above copyright notice, this list of
 *     conditions and the following disclaimer in the documentation and/or other
 *     materials provided with the distribution.
 *
 *  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
 *  AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 *  IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
 *  ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
 *  LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
 *  CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
 *  SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
 *  INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
 *  CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
 *  ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
 *  POSSIBILITY OF SUCH DAMAGE.
 */

//! SC-55 render actor and transactional state barriers.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
    time::Duration,
};

use resampler::{Attenuation, Latency, ResamplerFir, ResamplerFirState};
use save_state::ResourceBinding;

use crate::{context::Sc55Context, state::Sc55State};

const OUTPUT_RATE: u32 = 48_000;
const STEP_FRAMES: u32 = 240;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_MIDI_BUFFER_SIZE: usize = 32768;
const MAX_AUDIO_BUFFER_SIZE: usize = 8192;

/// Complete mutable state owned by the SC-55 render actor.
#[derive(Clone)]
pub struct Sc55WorkerState {
    context: Sc55State,
    resampler: ResamplerFirState,
    native_rate: u32,
}

/// Complete SC-55 actor state including machine-side pending buffers.
#[derive(Clone)]
pub struct Sc55ActorState {
    worker: Box<Sc55WorkerState>,
    pending_audio: Vec<f32>,
    midi_buffer: Vec<u8>,
}

crate::impl_state_codec!(Sc55WorkerState {
    context,
    resampler,
    native_rate,
});

save_state::impl_boxed_state_decode!(Sc55WorkerState);

crate::impl_state_codec!(Sc55ActorState {
    worker,
    pending_audio,
    midi_buffer,
});

enum Sc55Command {
    Render { midi: Vec<u8>, audio: Vec<f32> },
    Capture,
    PrepareRestore(Box<Sc55WorkerState>),
    CommitRestore,
    AbortRestore,
    Reset,
    Shutdown,
}

enum Sc55Response {
    Rendered { midi: Vec<u8>, audio: Vec<f32> },
    Captured(Box<Sc55WorkerState>),
    RestorePrepared(Result<(), String>),
    RestoreCommitted,
    RestoreAborted,
    ResetComplete,
}

/// Machine-side handle for the SC-55 render actor.
pub struct Sc55Actor {
    command_sender: SyncSender<Sc55Command>,
    response_receiver: Receiver<Sc55Response>,
    join_handle: Option<JoinHandle<()>>,
    midi_buffer: Option<Vec<u8>>,
    pending_audio: Option<Vec<f32>>,
    render_pending: bool,
    prepared_frontend: Option<(Vec<f32>, Vec<u8>)>,
    resource_bindings: Vec<ResourceBinding>,
    response_timeout: Duration,
}

impl Sc55Actor {
    /// Starts the SC-55 actor and loads its ROM resources.
    pub fn start(rom_directory: &Path) -> Result<Self, Sc55Error> {
        let (command_sender, command_receiver) = mpsc::sync_channel(2);
        let (response_sender, response_receiver) = mpsc::sync_channel(2);
        let (initialization_sender, initialization_receiver) = mpsc::sync_channel(1);
        let rom_directory = rom_directory.to_owned();

        let join_handle = thread::Builder::new()
            .name("sc55-render".into())
            .spawn(move || {
                // Initialize here so the large synth state uses the worker stack.
                initialize_and_render(
                    rom_directory,
                    command_receiver,
                    response_sender,
                    initialization_sender,
                );
            })
            .map_err(Sc55Error::ThreadSpawn)?;

        match initialization_receiver.recv() {
            Ok(Ok(resource_bindings)) => Ok(Self {
                command_sender,
                response_receiver,
                join_handle: Some(join_handle),
                midi_buffer: Some(Vec::with_capacity(MAX_MIDI_BUFFER_SIZE)),
                pending_audio: Some(Vec::with_capacity(MAX_AUDIO_BUFFER_SIZE)),
                render_pending: false,
                prepared_frontend: None,
                resource_bindings,
                response_timeout: RESPONSE_TIMEOUT,
            }),
            Ok(Err(error)) => {
                let _ = join_handle.join();
                Err(error)
            }
            Err(_) => {
                let _ = join_handle.join();
                Err(Sc55Error::InitializationThreadExited)
            }
        }
    }

    /// Returns exact identities for the loaded SC-55 ROM set.
    pub fn resource_bindings(&self) -> &[ResourceBinding] {
        &self.resource_bindings
    }

    /// Exchanges one audio chunk and queues the next MIDI batch.
    pub fn exchange(
        &mut self,
        volume: f32,
        output: &mut [f32],
        fill: impl FnOnce(&mut [u8]) -> usize,
    ) {
        if self.complete_render().is_err() {
            return;
        }
        let Some(pending_audio) = self.pending_audio.as_ref() else {
            return;
        };
        for (target, sample) in output.iter_mut().zip(pending_audio) {
            *target += *sample * volume;
        }
        let Some(midi_buffer) = self.midi_buffer.as_mut() else {
            return;
        };
        midi_buffer.resize(MAX_MIDI_BUFFER_SIZE, 0);
        let midi_length = fill(midi_buffer).min(MAX_MIDI_BUFFER_SIZE);
        midi_buffer.truncate(midi_length);
        let Some(midi) = self.midi_buffer.take() else {
            return;
        };
        let Some(audio) = self.pending_audio.take() else {
            self.midi_buffer = Some(midi);
            return;
        };
        let command = Sc55Command::Render { midi, audio };
        if self.command_sender.send(command).is_ok() {
            self.render_pending = true;
        }
    }

    /// Captures the actor after all earlier render commands complete.
    pub fn capture_state(&mut self) -> Result<Sc55ActorState, Sc55Error> {
        if self.prepared_frontend.is_some() {
            return Err(Sc55Error::InvalidActorState(
                "capture requested during prepared restore".to_owned(),
            ));
        }
        self.complete_render()?;
        self.send(Sc55Command::Capture)?;
        let worker = match self.receive()? {
            Sc55Response::Captured(worker) => worker,
            _ => return Err(Sc55Error::UnexpectedResponse),
        };
        let midi_buffer = self
            .midi_buffer
            .as_ref()
            .ok_or_else(|| Sc55Error::InvalidActorState("MIDI buffer is in flight".to_owned()))?;
        let pending_audio = self
            .pending_audio
            .as_ref()
            .ok_or_else(|| Sc55Error::InvalidActorState("audio buffer is in flight".to_owned()))?;
        Ok(Sc55ActorState {
            worker,
            pending_audio: pending_audio.clone(),
            midi_buffer: midi_buffer.clone(),
        })
    }

    /// Validates and stages a restore without changing active synth state.
    pub fn prepare_restore(&mut self, mut state: Sc55ActorState) -> Result<(), Sc55Error> {
        if self.prepared_frontend.is_some() {
            return Err(Sc55Error::InvalidActorState(
                "a restore is already prepared".to_owned(),
            ));
        }
        if state.pending_audio.len() > MAX_AUDIO_BUFFER_SIZE
            || state.midi_buffer.len() > MAX_MIDI_BUFFER_SIZE
        {
            return Err(Sc55Error::InvalidActorState(
                "frontend buffer is too large".to_owned(),
            ));
        }
        state
            .pending_audio
            .try_reserve_exact(MAX_AUDIO_BUFFER_SIZE - state.pending_audio.len())
            .map_err(|error| Sc55Error::InvalidActorState(error.to_string()))?;
        state
            .midi_buffer
            .try_reserve_exact(MAX_MIDI_BUFFER_SIZE - state.midi_buffer.len())
            .map_err(|error| Sc55Error::InvalidActorState(error.to_string()))?;
        self.complete_render()?;
        self.send(Sc55Command::PrepareRestore(state.worker))?;
        match self.receive()? {
            Sc55Response::RestorePrepared(Ok(())) => {
                self.prepared_frontend = Some((state.pending_audio, state.midi_buffer));
                Ok(())
            }
            Sc55Response::RestorePrepared(Err(message)) => {
                Err(Sc55Error::InvalidActorState(message))
            }
            _ => Err(Sc55Error::UnexpectedResponse),
        }
    }

    /// Commits the previously prepared restore.
    pub fn commit_restore(&mut self) -> Result<(), Sc55Error> {
        let Some((pending_audio, midi_buffer)) = self.prepared_frontend.take() else {
            return Err(Sc55Error::InvalidActorState(
                "no restore is prepared".to_owned(),
            ));
        };
        self.send(Sc55Command::CommitRestore)?;
        match self.receive()? {
            Sc55Response::RestoreCommitted => {
                self.pending_audio = Some(pending_audio);
                self.midi_buffer = Some(midi_buffer);
                self.render_pending = false;
                Ok(())
            }
            _ => Err(Sc55Error::UnexpectedResponse),
        }
    }

    /// Aborts the previously prepared restore.
    pub fn abort_restore(&mut self) -> Result<(), Sc55Error> {
        if self.prepared_frontend.take().is_none() {
            return Ok(());
        }
        self.send(Sc55Command::AbortRestore)?;
        match self.receive()? {
            Sc55Response::RestoreAborted => Ok(()),
            _ => Err(Sc55Error::UnexpectedResponse),
        }
    }

    /// Resets the synth and streaming state to its initialized state.
    pub fn reset(&mut self) -> Result<(), Sc55Error> {
        if self.prepared_frontend.is_some() {
            return Err(Sc55Error::InvalidActorState(
                "reset requested during prepared restore".to_owned(),
            ));
        }
        self.complete_render()?;
        self.send(Sc55Command::Reset)?;
        match self.receive()? {
            Sc55Response::ResetComplete => {
                if let Some(pending_audio) = &mut self.pending_audio {
                    pending_audio.clear();
                }
                if let Some(midi_buffer) = &mut self.midi_buffer {
                    midi_buffer.clear();
                }
                Ok(())
            }
            _ => Err(Sc55Error::UnexpectedResponse),
        }
    }

    fn complete_render(&mut self) -> Result<(), Sc55Error> {
        if !self.render_pending {
            return Ok(());
        }
        match self.receive()? {
            Sc55Response::Rendered { midi, audio } => {
                self.midi_buffer = Some(midi);
                self.pending_audio = Some(audio);
                self.render_pending = false;
                Ok(())
            }
            _ => Err(Sc55Error::UnexpectedResponse),
        }
    }

    fn send(&self, command: Sc55Command) -> Result<(), Sc55Error> {
        self.command_sender
            .send(command)
            .map_err(|_| Sc55Error::WorkerDisconnected)
    }

    fn receive(&self) -> Result<Sc55Response, Sc55Error> {
        self.response_receiver
            .recv_timeout(self.response_timeout)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => Sc55Error::WorkerTimeout,
                mpsc::RecvTimeoutError::Disconnected => Sc55Error::WorkerDisconnected,
            })
    }
}

impl Drop for Sc55Actor {
    fn drop(&mut self) {
        let _ = self.command_sender.send(Sc55Command::Shutdown);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

fn initialize_and_render(
    rom_directory: PathBuf,
    command_receiver: Receiver<Sc55Command>,
    response_sender: SyncSender<Sc55Response>,
    initialization_sender: SyncSender<Result<Vec<ResourceBinding>, Sc55Error>>,
) {
    let context = match Sc55Context::new(&rom_directory) {
        Ok(context) => context,
        Err(error) => {
            let _ = initialization_sender.send(Err(Sc55Error::Context(error)));
            return;
        }
    };
    let native_rate = context.sample_rate();
    if native_rate == 0 {
        let _ = initialization_sender.send(Err(Sc55Error::InvalidSampleRate));
        return;
    }
    let resource_bindings = context.resource_bindings().to_vec();
    if initialization_sender.send(Ok(resource_bindings)).is_err() {
        return;
    }
    render_actor_main(context, native_rate, command_receiver, response_sender);
}

fn render_actor_main(
    mut context: Sc55Context,
    native_rate: u32,
    command_receiver: Receiver<Sc55Command>,
    response_sender: SyncSender<Sc55Response>,
) {
    let native_frames_per_chunk =
        (u64::from(native_rate) * u64::from(STEP_FRAMES)).div_ceil(u64::from(OUTPUT_RATE)) as u32;
    let native_sample_count = native_frames_per_chunk as usize * 2;
    let mut native_buffer = vec![0.0f32; native_sample_count];
    let mut resampler = new_resampler(native_rate);
    let mut resample_output = vec![0.0f32; resampler.buffer_size_output()];
    let mut prepared: Option<(Sc55State, ResamplerFir)> = None;
    let initial_context_state = context.capture_state();

    while let Ok(command) = command_receiver.recv() {
        match command {
            Sc55Command::Render {
                mut midi,
                mut audio,
            } => {
                for byte in midi.iter().copied() {
                    context.post_midi(byte);
                }
                midi.clear();
                context.render(&mut native_buffer, native_frames_per_chunk);
                audio.clear();
                if let Ok((_consumed, produced)) =
                    resampler.resample(&native_buffer, &mut resample_output)
                {
                    let copy_length = produced.min(audio.capacity());
                    audio.extend_from_slice(&resample_output[..copy_length]);
                }
                if response_sender
                    .send(Sc55Response::Rendered { midi, audio })
                    .is_err()
                {
                    break;
                }
            }
            Sc55Command::Capture => {
                let state = Sc55WorkerState {
                    context: context.capture_state(),
                    resampler: resampler.capture_state(),
                    native_rate,
                };
                if response_sender
                    .send(Sc55Response::Captured(Box::new(state)))
                    .is_err()
                {
                    break;
                }
            }
            Sc55Command::PrepareRestore(state) => {
                let mut state = *state;
                context.attach_resources(&mut state.context);
                let result = if state.native_rate != native_rate {
                    Err("SC-55 native sample rate differs".to_owned())
                } else if let Err(error) = state.context.validate_for_restore() {
                    Err(error)
                } else {
                    let mut candidate_resampler = new_resampler(native_rate);
                    match candidate_resampler.restore_state(state.resampler) {
                        Ok(()) => {
                            prepared = Some((state.context, candidate_resampler));
                            Ok(())
                        }
                        Err(error) => Err(error.to_string()),
                    }
                };
                if response_sender
                    .send(Sc55Response::RestorePrepared(result))
                    .is_err()
                {
                    break;
                }
            }
            Sc55Command::CommitRestore => {
                if let Some((state, candidate_resampler)) = prepared.take() {
                    context.restore_state(state);
                    resampler = candidate_resampler;
                }
                if response_sender
                    .send(Sc55Response::RestoreCommitted)
                    .is_err()
                {
                    break;
                }
            }
            Sc55Command::AbortRestore => {
                prepared = None;
                if response_sender.send(Sc55Response::RestoreAborted).is_err() {
                    break;
                }
            }
            Sc55Command::Reset => {
                context.restore_state(initial_context_state.clone());
                resampler = new_resampler(native_rate);
                prepared = None;
                if response_sender.send(Sc55Response::ResetComplete).is_err() {
                    break;
                }
            }
            Sc55Command::Shutdown => break,
        }
    }
}

fn new_resampler(native_rate: u32) -> ResamplerFir {
    ResamplerFir::new_from_hz(
        2,
        native_rate,
        OUTPUT_RATE,
        Latency::default(),
        Attenuation::default(),
    )
}

/// Errors returned by the SC-55 actor.
#[derive(Debug)]
pub enum Sc55Error {
    /// Failed to create or initialize the SC-55 context.
    Context(crate::context::Sc55ContextError),
    /// The reported sample rate is zero.
    InvalidSampleRate,
    /// Failed to spawn the render actor.
    ThreadSpawn(std::io::Error),
    /// The render actor exited during initialization.
    InitializationThreadExited,
    /// The render actor disconnected unexpectedly.
    WorkerDisconnected,
    /// The render actor did not answer its barrier in time.
    WorkerTimeout,
    /// The render actor returned a response for another command.
    UnexpectedResponse,
    /// A capture or restore request violated actor state.
    InvalidActorState(String),
}

impl std::fmt::Display for Sc55Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Context(error) => write!(formatter, "SC-55 initialization failed: {error}"),
            Self::InvalidSampleRate => formatter.write_str("SC-55 reported sample rate of 0"),
            Self::ThreadSpawn(error) => write!(formatter, "failed to spawn SC-55 actor: {error}"),
            Self::InitializationThreadExited => {
                formatter.write_str("SC-55 render actor exited during initialization")
            }
            Self::WorkerDisconnected => formatter.write_str("SC-55 render actor disconnected"),
            Self::WorkerTimeout => formatter.write_str("SC-55 render actor timed out"),
            Self::UnexpectedResponse => {
                formatter.write_str("SC-55 render actor returned an unexpected response")
            }
            Self::InvalidActorState(message) => write!(formatter, "invalid SC-55 state: {message}"),
        }
    }
}

impl std::error::Error for Sc55Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_reports_worker_timeout() {
        let (command_sender, _command_receiver) = mpsc::sync_channel(4);
        let (_response_sender, response_receiver) = mpsc::sync_channel(4);
        let mut actor = Sc55Actor {
            command_sender,
            response_receiver,
            join_handle: None,
            midi_buffer: Some(Vec::new()),
            pending_audio: Some(Vec::new()),
            render_pending: false,
            prepared_frontend: None,
            resource_bindings: Vec::new(),
            response_timeout: Duration::from_millis(1),
        };

        assert!(matches!(
            actor.capture_state(),
            Err(Sc55Error::WorkerTimeout)
        ));
    }

    #[test]
    fn capture_reports_worker_disconnect() {
        let (command_sender, _command_receiver) = mpsc::sync_channel(4);
        let (response_sender, response_receiver) = mpsc::sync_channel(4);
        drop(response_sender);
        let mut actor = Sc55Actor {
            command_sender,
            response_receiver,
            join_handle: None,
            midi_buffer: Some(Vec::new()),
            pending_audio: Some(Vec::new()),
            render_pending: false,
            prepared_frontend: None,
            resource_bindings: Vec::new(),
            response_timeout: RESPONSE_TIMEOUT,
        };

        assert!(matches!(
            actor.capture_state(),
            Err(Sc55Error::WorkerDisconnected)
        ));
    }
}
#[test]
fn actor_snapshot_keeps_worker_state_off_the_stack() {
    assert!(size_of::<Sc55ActorState>() <= 128);
}
