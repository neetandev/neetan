// Copyright (C) 2003, 2004, 2005, 2006, 2008, 2009 Dean Beeler, Jerome Fisher
// Copyright (C) 2011-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
//  This program is free software: you can redistribute it and/or modify
//  it under the terms of the GNU Lesser General Public License as published by
//  the Free Software Foundation, either version 2.1 of the License, or
//  (at your option) any later version.
//
//  This program is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU Lesser General Public License for more details.
//
//  You should have received a copy of the GNU Lesser General Public License
//  along with this program.  If not, see <http://www.gnu.org/licenses/>.

//! MT-32 render actor and transactional state barriers.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
    time::Duration,
};

use resampler::{Attenuation, Latency, ResamplerFir, ResamplerFirState};
use save_state::ResourceBinding;

use crate::{
    context::{MuntContext, MuntContextError},
    state::MuntState,
};

const OUTPUT_RATE: u32 = 48_000;
const STEP_FRAMES: u32 = 240;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_MIDI_BUFFER_SIZE: usize = 32768;
const MAX_AUDIO_BUFFER_SIZE: usize = 8192;

/// Complete mutable state owned by the MT-32 render actor.
#[derive(Clone)]
pub struct MuntWorkerState {
    context: MuntState,
    resampler: ResamplerFirState,
    native_rate: u32,
}

/// Complete MT-32 actor state including machine-side pending buffers.
#[derive(Clone)]
pub struct MuntActorState {
    worker: Box<MuntWorkerState>,
    pending_audio: Vec<f32>,
    midi_buffer: Vec<u8>,
}

crate::impl_state_codec!(MuntWorkerState {
    context,
    resampler,
    native_rate,
});

crate::impl_state_codec!(MuntActorState {
    worker,
    pending_audio,
    midi_buffer,
});

enum MuntCommand {
    Render { midi: Vec<u8>, audio: Vec<f32> },
    Capture,
    PrepareRestore(Box<MuntWorkerState>),
    CommitRestore,
    AbortRestore,
    Reset,
    Shutdown,
}

enum MuntResponse {
    Rendered { midi: Vec<u8>, audio: Vec<f32> },
    Captured(Box<MuntWorkerState>),
    RestorePrepared(Result<(), String>),
    RestoreCommitted,
    RestoreAborted,
    ResetComplete,
}

/// Machine-side handle for the MT-32 render actor.
pub struct MuntActor {
    command_sender: SyncSender<MuntCommand>,
    response_receiver: Receiver<MuntResponse>,
    join_handle: Option<JoinHandle<()>>,
    midi_buffer: Option<Vec<u8>>,
    pending_audio: Option<Vec<f32>>,
    render_pending: bool,
    prepared_frontend: Option<(Vec<f32>, Vec<u8>)>,
    resource_bindings: Vec<ResourceBinding>,
    response_timeout: Duration,
}

impl MuntActor {
    /// Starts the MT-32 actor and loads its ROM resources.
    pub fn start(rom_directory: &Path) -> Result<Self, MuntError> {
        let (command_sender, command_receiver) = mpsc::sync_channel(2);
        let (response_sender, response_receiver) = mpsc::sync_channel(2);
        let (initialization_sender, initialization_receiver) = mpsc::sync_channel(1);
        let rom_directory = rom_directory.to_owned();

        let join_handle = thread::Builder::new()
            .name("mt32-render".into())
            .spawn(move || {
                // Initialize here so the large synth state uses the worker stack.
                initialize_and_render(
                    rom_directory,
                    command_receiver,
                    response_sender,
                    initialization_sender,
                );
            })
            .map_err(MuntError::ThreadSpawn)?;

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
                Err(MuntError::InitializationThreadExited)
            }
        }
    }

    /// Returns exact identities for the loaded control and PCM ROMs.
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
        let command = MuntCommand::Render { midi, audio };
        if self.command_sender.send(command).is_ok() {
            self.render_pending = true;
        }
    }

    /// Captures the actor after all earlier render commands complete.
    pub fn capture_state(&mut self) -> Result<MuntActorState, MuntError> {
        if self.prepared_frontend.is_some() {
            return Err(MuntError::InvalidActorState(
                "capture requested during prepared restore".to_owned(),
            ));
        }
        self.complete_render()?;
        self.send(MuntCommand::Capture)?;
        let worker = match self.receive()? {
            MuntResponse::Captured(worker) => worker,
            _ => return Err(MuntError::UnexpectedResponse),
        };
        let midi_buffer = self
            .midi_buffer
            .as_ref()
            .ok_or_else(|| MuntError::InvalidActorState("MIDI buffer is in flight".to_owned()))?;
        let pending_audio = self
            .pending_audio
            .as_ref()
            .ok_or_else(|| MuntError::InvalidActorState("audio buffer is in flight".to_owned()))?;
        Ok(MuntActorState {
            worker,
            pending_audio: pending_audio.clone(),
            midi_buffer: midi_buffer.clone(),
        })
    }

    /// Validates and stages a restore without changing active synth state.
    pub fn prepare_restore(&mut self, mut state: MuntActorState) -> Result<(), MuntError> {
        if self.prepared_frontend.is_some() {
            return Err(MuntError::InvalidActorState(
                "a restore is already prepared".to_owned(),
            ));
        }
        if state.pending_audio.len() > MAX_AUDIO_BUFFER_SIZE
            || state.midi_buffer.len() > MAX_MIDI_BUFFER_SIZE
        {
            return Err(MuntError::InvalidActorState(
                "frontend buffer is too large".to_owned(),
            ));
        }
        state
            .pending_audio
            .try_reserve_exact(MAX_AUDIO_BUFFER_SIZE - state.pending_audio.len())
            .map_err(|error| MuntError::InvalidActorState(error.to_string()))?;
        state
            .midi_buffer
            .try_reserve_exact(MAX_MIDI_BUFFER_SIZE - state.midi_buffer.len())
            .map_err(|error| MuntError::InvalidActorState(error.to_string()))?;
        self.complete_render()?;
        self.send(MuntCommand::PrepareRestore(state.worker))?;
        match self.receive()? {
            MuntResponse::RestorePrepared(Ok(())) => {
                self.prepared_frontend = Some((state.pending_audio, state.midi_buffer));
                Ok(())
            }
            MuntResponse::RestorePrepared(Err(message)) => {
                Err(MuntError::InvalidActorState(message))
            }
            _ => Err(MuntError::UnexpectedResponse),
        }
    }

    /// Commits the previously prepared restore.
    pub fn commit_restore(&mut self) -> Result<(), MuntError> {
        let Some((pending_audio, midi_buffer)) = self.prepared_frontend.take() else {
            return Err(MuntError::InvalidActorState(
                "no restore is prepared".to_owned(),
            ));
        };
        self.send(MuntCommand::CommitRestore)?;
        match self.receive()? {
            MuntResponse::RestoreCommitted => {
                self.pending_audio = Some(pending_audio);
                self.midi_buffer = Some(midi_buffer);
                self.render_pending = false;
                Ok(())
            }
            _ => Err(MuntError::UnexpectedResponse),
        }
    }

    /// Aborts the previously prepared restore.
    pub fn abort_restore(&mut self) -> Result<(), MuntError> {
        if self.prepared_frontend.take().is_none() {
            return Ok(());
        }
        self.send(MuntCommand::AbortRestore)?;
        match self.receive()? {
            MuntResponse::RestoreAborted => Ok(()),
            _ => Err(MuntError::UnexpectedResponse),
        }
    }

    /// Resets the synth and streaming state to its initialized state.
    pub fn reset(&mut self) -> Result<(), MuntError> {
        if self.prepared_frontend.is_some() {
            return Err(MuntError::InvalidActorState(
                "reset requested during prepared restore".to_owned(),
            ));
        }
        self.complete_render()?;
        self.send(MuntCommand::Reset)?;
        match self.receive()? {
            MuntResponse::ResetComplete => {
                if let Some(pending_audio) = &mut self.pending_audio {
                    pending_audio.clear();
                }
                if let Some(midi_buffer) = &mut self.midi_buffer {
                    midi_buffer.clear();
                }
                Ok(())
            }
            _ => Err(MuntError::UnexpectedResponse),
        }
    }

    fn complete_render(&mut self) -> Result<(), MuntError> {
        if !self.render_pending {
            return Ok(());
        }
        match self.receive()? {
            MuntResponse::Rendered { midi, audio } => {
                self.midi_buffer = Some(midi);
                self.pending_audio = Some(audio);
                self.render_pending = false;
                Ok(())
            }
            _ => Err(MuntError::UnexpectedResponse),
        }
    }

    fn send(&self, command: MuntCommand) -> Result<(), MuntError> {
        self.command_sender
            .send(command)
            .map_err(|_| MuntError::WorkerDisconnected)
    }

    fn receive(&self) -> Result<MuntResponse, MuntError> {
        self.response_receiver
            .recv_timeout(self.response_timeout)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => MuntError::WorkerTimeout,
                mpsc::RecvTimeoutError::Disconnected => MuntError::WorkerDisconnected,
            })
    }
}

impl Drop for MuntActor {
    fn drop(&mut self) {
        let _ = self.command_sender.send(MuntCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

fn initialize_and_render(
    rom_directory: PathBuf,
    command_receiver: Receiver<MuntCommand>,
    response_sender: SyncSender<MuntResponse>,
    initialization_sender: SyncSender<Result<Vec<ResourceBinding>, MuntError>>,
) {
    let context = match MuntContext::new(&rom_directory) {
        Ok(context) => context,
        Err(error) => {
            let _ = initialization_sender.send(Err(MuntError::Context(error)));
            return;
        }
    };
    let native_rate = context.sample_rate();
    if native_rate == 0 {
        let _ = initialization_sender.send(Err(MuntError::InvalidSampleRate));
        return;
    }
    let resource_bindings = context.resource_bindings().to_vec();
    if initialization_sender.send(Ok(resource_bindings)).is_err() {
        return;
    }
    render_actor_main(context, native_rate, command_receiver, response_sender);
}

fn render_actor_main(
    mut context: MuntContext,
    native_rate: u32,
    command_receiver: Receiver<MuntCommand>,
    response_sender: SyncSender<MuntResponse>,
) {
    let native_frames_per_chunk =
        (u64::from(native_rate) * u64::from(STEP_FRAMES)).div_ceil(u64::from(OUTPUT_RATE)) as u32;
    let native_sample_count = native_frames_per_chunk as usize * 2;
    let mut native_buffer = vec![0.0f32; native_sample_count];
    let mut resampler = new_resampler(native_rate);
    let mut resample_output = vec![0.0f32; resampler.buffer_size_output()];
    let mut prepared: Option<(MuntState, ResamplerFir)> = None;
    let initial_context_state = context.capture_state();

    while let Ok(command) = command_receiver.recv() {
        match command {
            MuntCommand::Render {
                mut midi,
                mut audio,
            } => {
                context.parse_stream(&midi);
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
                    .send(MuntResponse::Rendered { midi, audio })
                    .is_err()
                {
                    break;
                }
            }
            MuntCommand::Capture => {
                let state = MuntWorkerState {
                    context: context.capture_state(),
                    resampler: resampler.capture_state(),
                    native_rate,
                };
                if response_sender
                    .send(MuntResponse::Captured(Box::new(state)))
                    .is_err()
                {
                    break;
                }
            }
            MuntCommand::PrepareRestore(state) => {
                let mut state = *state;
                context.attach_resources(&mut state.context);
                let result = if state.native_rate != native_rate {
                    Err("MT-32 native sample rate differs".to_owned())
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
                    .send(MuntResponse::RestorePrepared(result))
                    .is_err()
                {
                    break;
                }
            }
            MuntCommand::CommitRestore => {
                if let Some((state, candidate_resampler)) = prepared.take() {
                    context.restore_state(state);
                    resampler = candidate_resampler;
                }
                if response_sender
                    .send(MuntResponse::RestoreCommitted)
                    .is_err()
                {
                    break;
                }
            }
            MuntCommand::AbortRestore => {
                prepared = None;
                if response_sender.send(MuntResponse::RestoreAborted).is_err() {
                    break;
                }
            }
            MuntCommand::Reset => {
                context.restore_state(initial_context_state.clone());
                resampler = new_resampler(native_rate);
                prepared = None;
                if response_sender.send(MuntResponse::ResetComplete).is_err() {
                    break;
                }
            }
            MuntCommand::Shutdown => break,
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

/// Errors returned by the MT-32 actor.
#[derive(Debug)]
pub enum MuntError {
    /// Failed to create or initialize the MT-32 context.
    Context(MuntContextError),
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

impl std::fmt::Display for MuntError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Context(error) => write!(formatter, "MT-32 initialization failed: {error}"),
            Self::InvalidSampleRate => formatter.write_str("MT-32 reported sample rate of 0"),
            Self::ThreadSpawn(error) => write!(formatter, "failed to spawn MT-32 actor: {error}"),
            Self::InitializationThreadExited => {
                formatter.write_str("MT-32 render actor exited during initialization")
            }
            Self::WorkerDisconnected => formatter.write_str("MT-32 render actor disconnected"),
            Self::WorkerTimeout => formatter.write_str("MT-32 render actor timed out"),
            Self::UnexpectedResponse => {
                formatter.write_str("MT-32 render actor returned an unexpected response")
            }
            Self::InvalidActorState(message) => write!(formatter, "invalid MT-32 state: {message}"),
        }
    }
}

impl std::error::Error for MuntError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_reports_worker_timeout() {
        let (command_sender, _command_receiver) = mpsc::sync_channel(4);
        let (_response_sender, response_receiver) = mpsc::sync_channel(4);
        let mut actor = MuntActor {
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
            Err(MuntError::WorkerTimeout)
        ));
    }

    #[test]
    fn capture_reports_worker_disconnect() {
        let (command_sender, _command_receiver) = mpsc::sync_channel(4);
        let (response_sender, response_receiver) = mpsc::sync_channel(4);
        drop(response_sender);
        let mut actor = MuntActor {
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
            Err(MuntError::WorkerDisconnected)
        ));
    }
}
#[test]
fn actor_snapshot_keeps_worker_state_off_the_stack() {
    assert!(size_of::<MuntActorState>() <= 128);
}
