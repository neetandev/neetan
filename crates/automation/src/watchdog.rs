//! The cooperative deadline watchdog and its cancel handle.
//!
//! Because a thread cannot be force-terminated, the per-script timeout is
//! enforced cooperatively. A lightweight watchdog thread trips both the r7rs
//! `InterruptToken` (checked by the VM between instructions) and a shared
//! deadline flag at the deadline. The same thread emits periodic wall-elapsed
//! progress heartbeats.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use r7rs::InterruptToken;

use crate::protocol::{MessageProtocol, RunProgress};

/// The interruption channel shared between the executor and its watchdog.
#[derive(Clone, Debug)]
pub struct CancelHandle {
    interrupt: InterruptToken,
    deadline: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

impl CancelHandle {
    /// Creates a fresh cancel handle with its own interrupt token.
    #[must_use]
    pub fn new() -> Self {
        Self {
            interrupt: InterruptToken::new(),
            deadline: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns a clone of the interrupt token to install on the engine.
    #[must_use]
    pub fn interrupt_token(&self) -> InterruptToken {
        self.interrupt.clone()
    }

    /// Trips the deadline: sets the deadline flag and interrupts the VM.
    pub fn trip_deadline(&self) {
        self.deadline.store(true, Ordering::SeqCst);
        self.interrupt.interrupt();
    }

    /// Requests external cancellation: sets the cancel flag and interrupts.
    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.interrupt.interrupt();
    }

    /// Returns whether the deadline has tripped.
    #[must_use]
    pub fn deadline_tripped(&self) -> bool {
        self.deadline.load(Ordering::SeqCst)
    }

    /// Returns whether external cancellation was requested.
    #[must_use]
    pub fn cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

impl Default for CancelHandle {
    fn default() -> Self {
        Self::new()
    }
}

const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(250);

/// A running watchdog thread that trips the deadline and emits heartbeats.
pub struct Watchdog {
    finished: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Watchdog {
    /// Spawns a watchdog for a run that started at `start` with the given
    /// timeout. The watchdog trips `cancel` at the deadline and forwards
    /// wall-elapsed progress heartbeats through `events`.
    #[must_use]
    pub fn spawn(
        cancel: CancelHandle,
        events: Sender<MessageProtocol>,
        timeout: Duration,
        start: Instant,
    ) -> Self {
        let finished = Arc::new(AtomicBool::new(false));
        let finished_thread = Arc::clone(&finished);
        let handle = std::thread::spawn(move || {
            let mut tripped = false;
            loop {
                std::thread::sleep(HEARTBEAT_INTERVAL);
                if finished_thread.load(Ordering::SeqCst) {
                    return;
                }
                let elapsed = start.elapsed();
                if !tripped && elapsed >= timeout {
                    cancel.trip_deadline();
                    tripped = true;
                }
                let progress = RunProgress {
                    wall_elapsed_ms: elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
                    ..RunProgress::default()
                };
                if events.send(MessageProtocol::Progress(progress)).is_err() {
                    return;
                }
            }
        });
        Self {
            finished,
            handle: Some(handle),
        }
    }

    /// Signals the watchdog to stop and joins its thread.
    pub fn stop(&mut self) {
        self.finished.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop();
    }
}
