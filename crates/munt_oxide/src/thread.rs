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

//! MT-32 render thread and shared buffer.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
};

use resampler::{Attenuation, Latency, ResamplerFir};

use crate::context::{MuntContext, MuntContextError};

const OUTPUT_RATE: u32 = 48_000;
const STEP_FRAMES: u32 = 240;

/// Shared buffer between the emulation thread and the MT-32 render thread.
///
/// Both Vecs live here permanently and keep their capacity - no allocations
/// after the first chunk. The two threads synchronize via `midi_ready` and
/// `render_done` flags on a single condvar.
pub struct MuntSharedBuffer {
    /// MIDI bytes written by the emu thread, read by the render thread.
    pub midi: Vec<u8>,
    /// Audio samples written by the render thread, read by the emu thread.
    pub audio: Vec<f32>,
    /// Set by the emu thread to wake the render thread.
    pub midi_ready: bool,
    /// Set by the render thread when audio is ready to read.
    pub render_done: bool,
    /// Set to cleanly shut down the render thread.
    pub shutdown: bool,
}

/// Shared buffer and join handle returned by [`MuntThread::start`].
pub type MuntChannels = (Arc<(Mutex<MuntSharedBuffer>, Condvar)>, JoinHandle<()>);

/// Handle to start the MT-32 render thread.
pub struct MuntThread;

impl MuntThread {
    /// Starts the MT-32 render thread.
    pub fn start(rom_directory: &Path) -> Result<MuntChannels, MuntError> {
        let shared = Arc::new((
            Mutex::new(MuntSharedBuffer {
                midi: Vec::new(),
                audio: Vec::new(),
                midi_ready: false,
                render_done: true,
                shutdown: false,
            }),
            Condvar::new(),
        ));
        let shared_clone = Arc::clone(&shared);
        let rom_directory = rom_directory.to_owned();
        let (initialization_sender, initialization_receiver) = mpsc::sync_channel(1);

        let join_handle = thread::Builder::new()
            .name("mt32-render".into())
            .spawn(move || {
                // Initialize in new thread, so to not starve the main thread of stack space.
                initialize_and_render(rom_directory, shared_clone, initialization_sender);
            })
            .map_err(MuntError::ThreadSpawn)?;

        match initialization_receiver.recv() {
            Ok(Ok(())) => Ok((shared, join_handle)),
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
}

fn initialize_and_render(
    rom_directory: PathBuf,
    shared: Arc<(Mutex<MuntSharedBuffer>, Condvar)>,
    initialization_sender: SyncSender<Result<(), MuntError>>,
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

    if initialization_sender.send(Ok(())).is_err() {
        return;
    }

    render_thread_main(context, native_rate, shared);
}

fn render_thread_main(
    mut context: MuntContext,
    native_rate: u32,
    shared: Arc<(Mutex<MuntSharedBuffer>, Condvar)>,
) {
    let native_frames_per_chunk =
        (u64::from(native_rate) * u64::from(STEP_FRAMES)).div_ceil(u64::from(OUTPUT_RATE)) as u32;

    let mut resampler = ResamplerFir::new_from_hz(
        2,
        native_rate,
        OUTPUT_RATE,
        Latency::default(),
        Attenuation::default(),
    );

    let native_sample_count = native_frames_per_chunk as usize * 2;
    let mut native_buffer = vec![0.0f32; native_sample_count];
    let resample_output_size = resampler.buffer_size_output();
    let mut resample_output = vec![0.0f32; resample_output_size];
    let mut local_midi = Vec::new();

    let (mutex, condvar) = &*shared;

    loop {
        // Wait for MIDI data or shutdown.
        {
            let mut buf = condvar
                .wait_while(mutex.lock().unwrap(), |buf| {
                    !buf.midi_ready && !buf.shutdown
                })
                .unwrap();

            if buf.shutdown {
                break;
            }

            // Swap MIDI bytes into our local buffer.
            std::mem::swap(&mut buf.midi, &mut local_midi);
            buf.midi_ready = false;
        }
        // Lock released - emu thread can continue.

        // Feed MIDI.
        context.parse_stream(&local_midi);
        local_midi.clear();

        // Render audio at native rate.
        context.render(&mut native_buffer, native_frames_per_chunk);

        let produced =
            match resampler.resample(&native_buffer[..native_sample_count], &mut resample_output) {
                Ok((_consumed, produced)) => produced,
                Err(_) => {
                    let mut buf = mutex.lock().unwrap();
                    buf.audio.clear();
                    buf.render_done = true;
                    condvar.notify_one();
                    continue;
                }
            };

        // Publish audio and signal the emu thread.
        {
            let mut buf = mutex.lock().unwrap();
            buf.audio.clear();
            buf.audio.extend_from_slice(&resample_output[..produced]);
            buf.render_done = true;
            condvar.notify_one();
        }
    }
}

/// Errors that can occur when starting the MT-32 render thread.
#[derive(Debug)]
pub enum MuntError {
    /// Failed to create or initialize the MT-32 context.
    Context(MuntContextError),
    /// The reported sample rate is zero.
    InvalidSampleRate,
    /// Failed to spawn the render thread.
    ThreadSpawn(std::io::Error),
    /// The render thread exited before completing initialization.
    InitializationThreadExited,
}

impl std::fmt::Display for MuntError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Context(error) => write!(f, "MT-32 initialization failed: {error}"),
            Self::InvalidSampleRate => write!(f, "MT-32 reported sample rate of 0"),
            Self::ThreadSpawn(error) => write!(f, "failed to spawn MT-32 thread: {error}"),
            Self::InitializationThreadExited => {
                f.write_str("MT-32 render thread exited during initialization")
            }
        }
    }
}

impl std::error::Error for MuntError {}
