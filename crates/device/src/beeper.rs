//! PC-98 beeper.
//!
//! Two hardware variants exist (per undoc98 `io_tcu.txt`):
//!
//! * [`BeeperKind::Fixed`] - PC-9801 first generation, E, F, M: a
//!   fixed-frequency hardware oscillator (~2 kHz) gated by PPI Port C bit 3.
//!   PIT channel 1 on these machines is the memory-refresh generator and is
//!   not allowed to alter the audible tone, so [`Beeper::set_pit_reload`]
//!   no-ops in this mode.
//! * [`BeeperKind::PitDriven`] - PC-9801U, VM, and later: PIT channel 1
//!   drives a 1-bit DAC speaker, so the beep frequency follows the PIT ch1
//!   reload value.
//!
//! In both cases samples are generated analytically without scheduler events.
//! Mid-frame state changes (PPI gate toggles and PIT counter reloads on
//! PIT-driven machines) are logged and replayed during sample generation for
//! cycle-accurate output.

use std::ops::{Deref, DerefMut};

use common::BeeperKind;

/// Base amplitude for the beeper square wave.
const BEEPER_BASE_AMPLITUDE: f32 = 0.5;

/// Polynomial Band-Limited Step correction.
fn poly_blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

save_state::runtime_state! {
/// Fractional sample remainder for drift-free sample count accumulation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SampleRemainder(pub f64);
}

impl Eq for SampleRemainder {}

impl Default for SampleRemainder {
    fn default() -> Self {
        Self(0.0)
    }
}

save_state::runtime_state! {
/// Authoritative beeper state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeeperState {
    /// Whether the buzzer is enabled (PPI port C bit 3 inverted: 0 = sound on).
    pub buzzer_enabled: bool,
    /// PIT channel 1 reload value.
    pub pit_reload: u16,
    /// CPU cycle when PIT channel 1 was last loaded.
    pub pit_last_load_cycle: u64,
    /// CPU cycle at which the current audio frame started.
    pub frame_start_cycle: u64,
    /// Fractional sample remainder carried across frames.
    pub sample_remainder: SampleRemainder,
    pre_frame_buzzer: bool,
    pre_frame_pit_reload: u16,
    pre_frame_pit_last_load: u64,
    buzzer_transitions: Vec<BuzzerTransition>,
    pit_transitions: Vec<PitTransition>,
}}

save_state::runtime_state! {
/// One timestamped beeper gate transition.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BuzzerTransition {
    cycle: u64,
    enabled: bool,
}}

save_state::runtime_state! {
/// One timestamped PIT reload transition observed by the beeper.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PitTransition {
    cycle: u64,
    reload: u16,
    last_load_cycle: u64,
}}

/// PC-98 beeper device.
pub struct Beeper {
    /// Embedded state for save/restore.
    pub state: BeeperState,
    /// Hardware architecture variant. Set at construction; not part of save state.
    kind: BeeperKind,
}

impl Deref for Beeper {
    type Target = BeeperState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for Beeper {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Beeper {
    /// Creates a new beeper in the muted state for the given hardware variant.
    ///
    /// `pit_clock_hz` is needed up front so that [`BeeperKind::Fixed`] can
    /// translate its target frequency into the equivalent PIT-tick reload value
    /// used by the analytical sample generator.
    pub fn new(kind: BeeperKind, pit_clock_hz: u32) -> Self {
        let pit_reload = match kind {
            BeeperKind::Fixed { hz } if hz > 0 => (pit_clock_hz / hz) as u16,
            BeeperKind::Fixed { .. } | BeeperKind::PitDriven => 0,
        };
        Self {
            state: BeeperState {
                buzzer_enabled: false,
                pit_reload,
                pit_last_load_cycle: 0,
                frame_start_cycle: 0,
                sample_remainder: SampleRemainder::default(),
                pre_frame_buzzer: false,
                pre_frame_pit_reload: pit_reload,
                pre_frame_pit_last_load: 0,
                buzzer_transitions: Vec::new(),
                pit_transitions: Vec::new(),
            },
            kind,
        }
    }

    /// Captures the complete beeper history.
    pub fn capture_state(&self) -> BeeperState {
        self.state.clone()
    }

    /// Restores the complete beeper history.
    pub fn restore_state(
        &mut self,
        state: BeeperState,
    ) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &())
    }

    /// Returns the hardware architecture variant of this beeper.
    pub fn kind(&self) -> BeeperKind {
        self.kind
    }

    /// Records a buzzer gate change. Called when PPI port C bit 3 changes.
    pub fn set_buzzer_enabled(&mut self, enabled: bool, cycle: u64) {
        if enabled != self.state.buzzer_enabled {
            self.state
                .buzzer_transitions
                .push(BuzzerTransition { cycle, enabled });
            self.state.buzzer_enabled = enabled;
        }
    }

    /// Records a PIT channel 1 reload. Called when PIT ch1 counter is loaded.
    ///
    /// On [`BeeperKind::Fixed`] machines (PC-9801 first/E/F/M) PIT ch1 is the
    /// memory-refresh generator and writes must not change the audible tone -
    /// this function is a no-op in that case.
    pub fn set_pit_reload(&mut self, reload: u16, last_load_cycle: u64) {
        if matches!(self.kind, BeeperKind::Fixed { .. }) {
            return;
        }
        self.state.pit_transitions.push(PitTransition {
            cycle: last_load_cycle,
            reload,
            last_load_cycle,
        });
        self.state.pit_reload = reload;
        self.state.pit_last_load_cycle = last_load_cycle;
    }

    /// Fills `output` with interleaved stereo audio samples (`[L, R, L, R, …]`)
    /// for the current frame, returning the number of `f32` values written
    /// (i.e. `frames × 2`).
    ///
    /// Covers the interval `[frame_start_cycle, frame_end_cycle)`. After
    /// generation, advances `frame_start_cycle` and clears transition logs.
    pub fn generate_samples(
        &mut self,
        frame_end_cycle: u64,
        cpu_clock_hz: u32,
        pit_clock_hz: u32,
        sample_rate: u32,
        volume: f32,
        output: &mut [f32],
    ) -> usize {
        self.render(
            frame_end_cycle,
            cpu_clock_hz,
            pit_clock_hz,
            sample_rate,
            volume,
            output,
            false,
        )
    }

    /// Like [`Beeper::generate_samples`] but mixes the beep additively on top of
    /// the existing `output` contents instead of overwriting them. Used where the
    /// beeper shares the mix with other sound sources (FM Towns).
    pub fn mix_samples(
        &mut self,
        frame_end_cycle: u64,
        cpu_clock_hz: u32,
        pit_clock_hz: u32,
        sample_rate: u32,
        volume: f32,
        output: &mut [f32],
    ) -> usize {
        self.render(
            frame_end_cycle,
            cpu_clock_hz,
            pit_clock_hz,
            sample_rate,
            volume,
            output,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render(
        &mut self,
        frame_end_cycle: u64,
        cpu_clock_hz: u32,
        pit_clock_hz: u32,
        sample_rate: u32,
        volume: f32,
        output: &mut [f32],
        additive: bool,
    ) -> usize {
        let frame_start = self.state.frame_start_cycle;
        let frame_cycles = frame_end_cycle.saturating_sub(frame_start);
        if frame_cycles == 0 || sample_rate == 0 {
            self.finish_frame(frame_end_cycle);
            return 0;
        }

        let frame_capacity = output.len() / 2;
        let exact_samples = (frame_cycles as f64 * f64::from(sample_rate))
            / f64::from(cpu_clock_hz)
            + self.state.sample_remainder.0;
        let frame_count = (exact_samples as usize).min(frame_capacity);
        self.state.sample_remainder = SampleRemainder(exact_samples - frame_count as f64);

        if frame_count == 0 {
            self.finish_frame(frame_end_cycle);
            return 0;
        }

        let amplitude = volume * BEEPER_BASE_AMPLITUDE;
        let pit_ratio = f64::from(pit_clock_hz) / f64::from(cpu_clock_hz);

        let mut current_buzzer = self.state.pre_frame_buzzer;
        let mut current_reload = self.state.pre_frame_pit_reload;
        let mut current_last_load = self.state.pre_frame_pit_last_load;

        let mut buz_idx = 0;
        let mut pit_idx = 0;

        let cycles_per_sample = frame_cycles as f64 / frame_count as f64;
        let mut dt = if current_reload > 0 {
            (pit_ratio * cycles_per_sample) / f64::from(current_reload)
        } else {
            0.0
        };

        for i in 0..frame_count {
            let cycle = frame_start + ((i as u64 * frame_cycles) / frame_count as u64);

            while buz_idx < self.state.buzzer_transitions.len()
                && self.state.buzzer_transitions[buz_idx].cycle <= cycle
            {
                current_buzzer = self.state.buzzer_transitions[buz_idx].enabled;
                buz_idx += 1;
            }

            while pit_idx < self.state.pit_transitions.len()
                && self.state.pit_transitions[pit_idx].cycle <= cycle
            {
                current_reload = self.state.pit_transitions[pit_idx].reload;
                current_last_load = self.state.pit_transitions[pit_idx].last_load_cycle;
                pit_idx += 1;
                dt = if current_reload > 0 {
                    (pit_ratio * cycles_per_sample) / f64::from(current_reload)
                } else {
                    0.0
                };
            }

            if !current_buzzer || current_reload == 0 {
                if !additive {
                    output[i * 2] = 0.0;
                    output[i * 2 + 1] = 0.0;
                }
                continue;
            }

            let elapsed_cpu = cycle.saturating_sub(current_last_load);
            let elapsed_pit = elapsed_cpu as f64 * pit_ratio;
            let reload = f64::from(current_reload);
            let phase = (elapsed_pit % reload) / reload;

            let mut value = if phase < 0.5 { 1.0 } else { -1.0 };
            value += poly_blep(phase, dt);
            value -= poly_blep((phase + 0.5) % 1.0, dt);
            let sample = amplitude * value as f32;
            if additive {
                output[i * 2] += sample;
                output[i * 2 + 1] += sample;
            } else {
                output[i * 2] = sample;
                output[i * 2 + 1] = sample;
            }
        }

        self.finish_frame(frame_end_cycle);
        frame_count * 2
    }

    fn finish_frame(&mut self, frame_end_cycle: u64) {
        self.state.buzzer_transitions.clear();
        self.state.pit_transitions.clear();
        self.state.frame_start_cycle = frame_end_cycle;
        self.state.pre_frame_buzzer = self.state.buzzer_enabled;
        self.state.pre_frame_pit_reload = self.state.pit_reload;
        self.state.pre_frame_pit_last_load = self.state.pit_last_load_cycle;
    }
}

impl save_state::ValidateState for BeeperState {
    fn validate_state(&self, _context: &()) -> Result<(), save_state::StateValidationError> {
        let buzzer_ordered = self
            .buzzer_transitions
            .windows(2)
            .all(|pair| pair[0].cycle <= pair[1].cycle);
        let pit_ordered = self
            .pit_transitions
            .windows(2)
            .all(|pair| pair[0].cycle <= pair[1].cycle);
        if !buzzer_ordered || !pit_ordered || !self.sample_remainder.0.is_finite() {
            return Err(save_state::StateValidationError::new(
                "beeper streaming history is invalid",
            ));
        }
        Ok(())
    }
}

impl save_state::AfterRestore for Beeper {
    fn after_restore(&mut self) {}
}

impl save_state::RestoreTarget for Beeper {
    type State = BeeperState;
    type ValidationContext = ();

    fn replace_state(&mut self, state: Self::State) {
        self.state = state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_state_preserves_mid_frame_transitions() {
        let mut original = Beeper::new(BeeperKind::PitDriven, 2_457_600);
        original.set_pit_reload(998, 100);
        original.set_buzzer_enabled(true, 180);
        original.set_pit_reload(1200, 310);
        original.set_buzzer_enabled(false, 470);
        let encoded = save_state::encode_runtime_state(&original.capture_state());
        let decoded = save_state::decode_runtime_state::<BeeperState>(&encoded, 1 << 16).unwrap();
        let mut restored = Beeper::new(BeeperKind::PitDriven, 2_457_600);
        restored.restore_state(decoded).unwrap();

        let mut expected = [0.0f32; 128];
        let mut actual = [0.0f32; 128];
        original.generate_samples(1000, 8_000_000, 2_457_600, 48_000, 1.0, &mut expected);
        restored.generate_samples(1000, 8_000_000, 2_457_600, 48_000, 1.0, &mut actual);
        assert!(
            expected
                .iter()
                .zip(actual)
                .all(|(left, right)| left.to_bits() == right.to_bits())
        );
    }
}
