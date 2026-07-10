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

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
};

use resampler::{Attenuation, Latency, ResamplerFir};

use crate::context::Sc55Context;

const OUTPUT_RATE: u32 = 48_000;
const STEP_FRAMES: u32 = 240;

/// Shared buffer between the emulation thread and the SC-55 render thread.
///
/// Both Vecs live here permanently and keep their capacity - no allocations
/// after the first chunk. The two threads synchronize via `midi_ready` and
/// `render_done` flags on a single condvar.
pub struct Sc55SharedBuffer {
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

/// Shared buffer and join handle returned by [`Sc55Thread::start`].
pub type Sc55Channels = (Arc<(Mutex<Sc55SharedBuffer>, Condvar)>, JoinHandle<()>);

/// Handle to a running SC-55 render thread.
pub struct Sc55Thread;

impl Sc55Thread {
    /// Starts the SC-55 render thread.
    pub fn start(rom_directory: &Path) -> Result<Sc55Channels, Sc55Error> {
        let shared = Arc::new((
            Mutex::new(Sc55SharedBuffer {
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
            .name("sc55-render".into())
            .spawn(move || {
                // Initialize in new thread, so to not starve the main thread of stack space.
                initialize_and_render(rom_directory, shared_clone, initialization_sender);
            })
            .map_err(Sc55Error::ThreadSpawn)?;

        match initialization_receiver.recv() {
            Ok(Ok(())) => Ok((shared, join_handle)),
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
}

fn initialize_and_render(
    rom_directory: PathBuf,
    shared: Arc<(Mutex<Sc55SharedBuffer>, Condvar)>,
    initialization_sender: SyncSender<Result<(), Sc55Error>>,
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

    if initialization_sender.send(Ok(())).is_err() {
        return;
    }

    render_thread_main(context, native_rate, shared);
}

fn render_thread_main(
    mut context: Sc55Context,
    native_rate: u32,
    shared: Arc<(Mutex<Sc55SharedBuffer>, Condvar)>,
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
        for &byte in &local_midi {
            context.post_midi(byte);
        }
        local_midi.clear();

        // Render audio.
        context.render(&mut native_buffer, native_frames_per_chunk);

        let produced =
            match resampler.resample(&native_buffer[..native_sample_count], &mut resample_output) {
                Ok((_consumed, produced)) => produced,
                Err(_) => continue,
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

/// Errors that can occur when starting the SC-55 render thread.
#[derive(Debug)]
pub enum Sc55Error {
    /// Failed to create or initialize the SC-55 context.
    Context(crate::context::Sc55ContextError),
    /// The reported sample rate is zero.
    InvalidSampleRate,
    /// Failed to spawn the render thread.
    ThreadSpawn(std::io::Error),
    /// The render thread exited before completing initialization.
    InitializationThreadExited,
}

impl std::fmt::Display for Sc55Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Context(error) => write!(f, "SC-55 initialization failed: {error}"),
            Self::InvalidSampleRate => write!(f, "SC-55 reported sample rate of 0"),
            Self::ThreadSpawn(error) => write!(f, "failed to spawn SC-55 thread: {error}"),
            Self::InitializationThreadExited => {
                f.write_str("SC-55 render thread exited during initialization")
            }
        }
    }
}

impl std::error::Error for Sc55Error {}
