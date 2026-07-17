//! Cycle-synchronized one-bit audio output.

/// Default full-scale level of the one-bit output.
const FULL_SCALE_LEVEL: f32 = 1.0;

save_state::runtime_state! {
/// Complete one-bit DAC output and streaming state.
#[derive(Debug, Clone)]
pub struct OneBitDacState {
    high: bool,
    cpu_clock_hz: u32,
    sample_rate: u32,
    frame_start_cycle: u64,
    pending_levels: Vec<u8>,
    sample_remainder: f64,
}}

/// One-bit DAC with buffered cycle-synchronized output.
#[derive(Debug, Clone)]
pub struct OneBitDac {
    high: bool,
    cpu_clock_hz: u32,
    sample_rate: u32,
    frame_start_cycle: u64,
    pending_levels: Vec<u8>,
    sample_remainder: f64,
}

impl OneBitDac {
    /// Creates a low one-bit output.
    pub const fn new() -> Self {
        Self {
            high: false,
            cpu_clock_hz: 0,
            sample_rate: 0,
            frame_start_cycle: 0,
            pending_levels: Vec::new(),
            sample_remainder: 0.0,
        }
    }

    /// Configures the CPU and host audio clocks.
    pub fn configure_audio(&mut self, cpu_clock_hz: u32, sample_rate: u32) {
        self.cpu_clock_hz = cpu_clock_hz;
        self.sample_rate = sample_rate;
    }

    /// Current output level.
    pub const fn high(&self) -> bool {
        self.high
    }

    /// Records an output-level change at `cycle`.
    pub fn set_high(&mut self, high: bool, cycle: u64) {
        self.sync(cycle);
        self.high = high;
    }

    /// Captures the current output level and pending samples.
    pub fn capture_state(&self) -> OneBitDacState {
        OneBitDacState {
            high: self.high,
            cpu_clock_hz: self.cpu_clock_hz,
            sample_rate: self.sample_rate,
            frame_start_cycle: self.frame_start_cycle,
            pending_levels: self.pending_levels.clone(),
            sample_remainder: self.sample_remainder,
        }
    }

    /// Restores the current output level and queued transitions.
    pub fn restore_state(
        &mut self,
        state: OneBitDacState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.cpu_clock_hz != self.cpu_clock_hz
            || state.sample_rate != self.sample_rate
            || !state.sample_remainder.is_finite()
            || !(0.0..1.0).contains(&state.sample_remainder)
            || state.pending_levels.iter().any(|&level| level > 1)
        {
            return Err(save_state::StateValidationError::new(
                "one-bit DAC state is invalid",
            ));
        }
        self.high = state.high;
        self.frame_start_cycle = state.frame_start_cycle;
        self.pending_levels = state.pending_levels;
        self.sample_remainder = state.sample_remainder;
        Ok(())
    }

    /// Buffers output levels elapsed through `current_cycle`.
    pub fn sync(&mut self, current_cycle: u64) {
        let frame_cycles = current_cycle.saturating_sub(self.frame_start_cycle);
        if frame_cycles == 0 || self.cpu_clock_hz == 0 || self.sample_rate == 0 {
            self.frame_start_cycle = current_cycle;
            return;
        }
        let exact_samples = frame_cycles as f64 * f64::from(self.sample_rate)
            / f64::from(self.cpu_clock_hz)
            + self.sample_remainder;
        let sample_count = exact_samples as usize;
        self.sample_remainder = exact_samples - sample_count as f64;
        self.pending_levels.resize(
            self.pending_levels.len() + sample_count,
            u8::from(self.high),
        );
        self.frame_start_cycle = current_cycle;
    }

    /// Mixes the elapsed output into interleaved stereo samples.
    pub fn mix_samples(
        &mut self,
        frame_end_cycle: u64,
        cpu_clock_hz: u32,
        sample_rate: u32,
        volume: f32,
        output: &mut [f32],
    ) -> usize {
        if cpu_clock_hz == 0 || sample_rate == 0 {
            return 0;
        }
        if self.cpu_clock_hz == 0 && self.sample_rate == 0 {
            self.configure_audio(cpu_clock_hz, sample_rate);
        }
        debug_assert_eq!(self.cpu_clock_hz, cpu_clock_hz);
        debug_assert_eq!(self.sample_rate, sample_rate);
        self.sync(frame_end_cycle);

        let frame_count = self.pending_levels.len().min(output.len() / 2);
        for (frame, &high) in self.pending_levels[..frame_count].iter().enumerate() {
            if high != 0 {
                let sample = volume * FULL_SCALE_LEVEL;
                output[frame * 2] += sample;
                output[frame * 2 + 1] += sample;
            }
        }
        self.pending_levels.drain(..frame_count);
        frame_count * 2
    }
}

impl Default for OneBitDac {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_changes_only_the_later_samples() {
        let mut dac = OneBitDac::new();
        dac.configure_audio(100, 10);
        dac.set_high(true, 50);
        let mut output = [0.0; 20];
        let written = dac.mix_samples(100, 100, 10, 0.5, &mut output);
        assert_eq!(written, 20);
        assert!(output[..10].iter().all(|sample| *sample == 0.0));
        assert!(output[10..].iter().all(|sample| *sample == 0.5));
    }

    #[test]
    fn save_state_preserves_pending_levels() {
        let mut dac = OneBitDac::new();
        dac.configure_audio(1_000, 100);
        dac.set_high(true, 0);
        let mut small_output = [0.0; 20];
        dac.mix_samples(1_000, 1_000, 100, 1.0, &mut small_output);
        assert_eq!(dac.pending_levels.len(), 90);
        assert_eq!(dac.sample_remainder, 0.0);

        let encoded = save_state::encode_runtime_state(&dac.capture_state());
        let decoded = save_state::decode_runtime_state(&encoded, 1 << 20).unwrap();
        let mut restored = OneBitDac::new();
        restored.configure_audio(1_000, 100);
        restored.restore_state(decoded).unwrap();

        assert_eq!(restored.pending_levels, vec![1; 90]);
        assert_eq!(restored.sample_remainder, 0.0);
    }

    #[test]
    fn level_changes_do_not_change_pending_audio() {
        fn configured() -> OneBitDac {
            let mut dac = OneBitDac::new();
            dac.configure_audio(1_000, 100);
            dac.set_high(true, 0);
            dac
        }

        let mut uninterrupted = configured();
        let mut expected = [0.0; 200];
        uninterrupted.mix_samples(1_000, 1_000, 100, 1.0, &mut expected);

        let mut buffered = configured();
        let mut first = [0.0; 20];
        buffered.mix_samples(1_000, 1_000, 100, 1.0, &mut first);
        buffered.set_high(false, 1_000);
        let mut second = [0.0; 180];
        buffered.mix_samples(1_000, 1_000, 100, 1.0, &mut second);

        assert_eq!(&expected[..20], &first);
        assert_eq!(&expected[20..], &second);
        let mut after_change = [0.0; 20];
        buffered.mix_samples(1_100, 1_000, 100, 1.0, &mut after_change);
        assert_eq!(after_change, [0.0; 20]);
    }
}
