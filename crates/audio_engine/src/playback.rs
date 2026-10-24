//! The pause state and the fade ramps of the playback.

/// Number of stereo frames over which to fade in/out (250 ms at 48 kHz).
pub(crate) const FADE_FRAMES: u32 = 12_000;

/// Whether the playback device pauses, restarts or keeps its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceChange {
    /// The device keeps its state.
    Keep,
    /// The device restarts with fresh silence.
    Restart,
    /// The device pauses with an empty queue.
    Pause,
}

/// The pause state of the device and the remaining frames of the fades.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlaybackState {
    /// Whether the device is paused.
    device_paused: bool,
    /// Remaining stereo frames to fade out. The device pauses when it reaches 0.
    fade_out_remaining: u32,
    /// Remaining stereo frames to fade in.
    fade_in_remaining: u32,
}

impl PlaybackState {
    /// The state of a paused device without a fade.
    pub(crate) fn paused() -> Self {
        Self {
            device_paused: true,
            fade_out_remaining: 0,
            fade_in_remaining: 0,
        }
    }

    /// Whether the device is paused.
    #[cfg(test)]
    pub(crate) fn device_paused(&self) -> bool {
        self.device_paused
    }

    /// Pauses the device at once and cancels both fades.
    pub(crate) fn stop(&mut self) {
        self.device_paused = true;
        self.fade_out_remaining = 0;
        self.fade_in_remaining = 0;
    }

    /// Begins a fade-out. A paused device stays paused.
    pub(crate) fn pause(&mut self) {
        if self.device_paused {
            return;
        }
        self.fade_out_remaining = FADE_FRAMES;
        self.fade_in_remaining = 0;
    }

    /// Cancels a fade-out and begins a fade-in. Returns whether the paused
    /// device restarts.
    pub(crate) fn resume(&mut self) -> DeviceChange {
        let change = if self.device_paused {
            DeviceChange::Restart
        } else {
            DeviceChange::Keep
        };
        self.device_paused = false;
        self.fade_out_remaining = 0;
        self.fade_in_remaining = FADE_FRAMES;
        change
    }

    /// Applies the fade ramps to the interleaved stereo `samples`. Returns
    /// whether the device pauses because the fade-out completed.
    pub(crate) fn apply_fades(&mut self, samples: &mut [f32]) -> DeviceChange {
        let was_fading_out = self.fade_out_remaining > 0;

        if was_fading_out {
            for frame in samples.as_chunks_mut::<2>().0 {
                let gain = if self.fade_out_remaining > 0 {
                    self.fade_out_remaining -= 1;
                    self.fade_out_remaining as f32 / FADE_FRAMES as f32
                } else {
                    0.0
                };
                frame.iter_mut().for_each(|sample| *sample *= gain);
            }
        }

        if self.fade_in_remaining > 0 {
            for frame in samples.as_chunks_mut::<2>().0 {
                if self.fade_in_remaining == 0 {
                    break;
                }
                let gain = 1.0 - self.fade_in_remaining as f32 / FADE_FRAMES as f32;
                frame.iter_mut().for_each(|sample| *sample *= gain);
                self.fade_in_remaining -= 1;
            }
        }

        if was_fading_out && self.fade_out_remaining == 0 {
            self.device_paused = true;
            DeviceChange::Pause
        } else {
            DeviceChange::Keep
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DeviceChange, FADE_FRAMES, PlaybackState};

    /// Runs `frames` stereo frames of full scale samples through the fades.
    /// Returns the last device change and the samples.
    fn play(state: &mut PlaybackState, frames: usize) -> (DeviceChange, Vec<f32>) {
        let mut samples = vec![1.0; frames * 2];
        let change = state.apply_fades(&mut samples);
        (change, samples)
    }

    /// A running device after the fade-in.
    fn running() -> PlaybackState {
        let mut state = PlaybackState::paused();
        assert_eq!(state.resume(), DeviceChange::Restart);
        play(&mut state, FADE_FRAMES as usize);
        state
    }

    #[test]
    fn a_new_state_is_paused_and_restarts_on_resume() {
        let mut state = PlaybackState::paused();
        assert!(state.device_paused());
        assert_eq!(state.resume(), DeviceChange::Restart);
        assert!(!state.device_paused());
    }

    #[test]
    fn resume_after_stop_and_pause_restarts_the_device() {
        let mut state = running();
        state.stop();
        state.pause();
        assert!(state.device_paused());
        assert_eq!(state.resume(), DeviceChange::Restart);
        assert!(!state.device_paused());
    }

    #[test]
    fn a_completed_fade_out_pauses_the_device() {
        let mut state = running();
        state.pause();
        let (change, _) = play(&mut state, FADE_FRAMES as usize - 1);
        assert_eq!(change, DeviceChange::Keep);
        assert!(!state.device_paused());
        let (change, samples) = play(&mut state, 4);
        assert_eq!(change, DeviceChange::Pause);
        assert!(state.device_paused());
        assert!(samples.iter().all(|sample| *sample == 0.0));
        assert_eq!(state.resume(), DeviceChange::Restart);
    }

    #[test]
    fn resume_during_the_fade_out_keeps_the_device_running() {
        let mut state = running();
        state.pause();
        play(&mut state, 100);
        assert_eq!(state.resume(), DeviceChange::Keep);
        let (change, samples) = play(&mut state, FADE_FRAMES as usize);
        assert_eq!(change, DeviceChange::Keep);
        assert!(!state.device_paused());
        assert!(samples[0] < 0.01);
        assert!(samples[samples.len() - 1] > 0.99);
    }

    #[test]
    fn pause_on_a_paused_device_begins_no_fade() {
        let mut state = PlaybackState::paused();
        state.pause();
        let (change, samples) = play(&mut state, 4);
        assert_eq!(change, DeviceChange::Keep);
        assert!(samples.iter().all(|sample| *sample == 1.0));
    }

    #[test]
    fn the_fade_out_ramps_down() {
        let mut state = running();
        state.pause();
        let (_, samples) = play(&mut state, 2);
        let expected_first = (FADE_FRAMES - 1) as f32 / FADE_FRAMES as f32;
        let expected_second = (FADE_FRAMES - 2) as f32 / FADE_FRAMES as f32;
        assert_eq!(
            samples,
            [
                expected_first,
                expected_first,
                expected_second,
                expected_second
            ]
        );
    }
}
