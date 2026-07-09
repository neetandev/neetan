//! Sharp X1 CZ-8BS1 FM sound board: the YM2151 (OPM).

use ymfm_oxide::Ym2151;

use crate::opn_fm::{FmTimerAction, OpnFm};

/// YM2151 input clock on the CZ-8BS1 board: 4 MHz (internal FM sample clock
/// 4 MHz / 64).
const YM2151_CLOCK_HZ: u32 = 4_000_000;

/// Sharp X1 CZ-8BS1 FM sound board: YM2151 (OPM) with resampling.
pub struct SoundBoardOpm {
    core: OpnFm<Ym2151>,
}

impl SoundBoardOpm {
    /// Creates a CZ-8BS1 sound board.
    pub fn new(cpu_clock_hz: u32, sample_rate: u32) -> Self {
        Self {
            core: OpnFm::<Ym2151>::new(cpu_clock_hz, sample_rate, YM2151_CLOCK_HZ),
        }
    }

    /// Advances the chip clock to `current_cycle`.
    pub fn sync(&mut self, current_cycle: u64) {
        self.core.sync(current_cycle);
    }

    /// Reads the OPM status register (port `0x0701` read).
    pub fn read_status(&mut self, current_cycle: u64) -> u8 {
        self.core.read_status(current_cycle)
    }

    /// Latches the OPM register address (port `0x0700` write).
    pub fn write_address(&mut self, value: u8, current_cycle: u64) {
        self.core.write_address(value, current_cycle);
    }

    /// Writes the addressed OPM register (port `0x0701` write).
    pub fn write_data(&mut self, value: u8, current_cycle: u64) {
        self.core.write_data(value, current_cycle);
    }

    /// Notifies the chip that timer `timer_id` has expired.
    pub fn timer_expired(&mut self, timer_id: u32, current_cycle: u64) {
        self.core.timer_expired(timer_id, current_cycle);
    }

    /// Drains pending OPM timer schedule/cancel requests (keyed by `timer_id`).
    pub fn drain_timers(&mut self) -> &[FmTimerAction] {
        self.core.drain_timers()
    }

    /// Returns and clears the coalesced OPM IRQ-output edge.
    pub fn take_irq_change(&mut self) -> Option<bool> {
        self.core.take_irq_change()
    }

    /// Returns whether the OPM IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.core.irq_asserted()
    }

    /// Generates resampled stereo OPM audio and mixes it into `output`.
    pub fn generate_samples(
        &mut self,
        current_cycle: u64,
        cpu_clock_hz: u32,
        volume: f32,
        output: &mut [f32],
    ) {
        self.core
            .generate_samples(current_cycle, cpu_clock_hz, volume, output);
    }
}
