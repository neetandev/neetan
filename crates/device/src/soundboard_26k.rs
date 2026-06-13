//! PC-9801-26K sound board: YM2203 (OPN) FM + SSG synthesis with resampling.

use common::EventKind;
use ymfm_oxide::Ym2203;

pub use crate::opn_fm::FmSampleRemainder;
use crate::opn_fm::{FmTimerAction, OpnFm, OpnFmTiming};

/// Snapshot of the PC-9801-26K sound board state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Soundboard26kState {
    /// Address latch (write-only via port 0x0188).
    pub address: u8,
    /// IRQ line number (default 12 = INT5).
    pub irq_line: u8,
    /// Whether the IRQ output is currently asserted.
    pub irq_asserted: bool,
    /// CPU cycle at which the busy flag clears.
    pub busy_end_cycle: u64,
    /// CPU cycle at which the current audio output frame started.
    /// Only advanced by `generate_samples()`.
    pub audio_frame_start_cycle: u64,
    /// CPU cycle up to which the FM chip has been clocked.
    /// Advanced by `sync()` on every port access.
    pub fm_sync_cursor: u64,
    /// Fractional sample remainder carried across frames.
    pub sample_remainder: FmSampleRemainder,
    /// Whether this board uses alternate timer event kinds (dual-board config).
    pub alternate_timers: bool,
}

impl Default for Soundboard26kState {
    fn default() -> Self {
        Self {
            address: 0,
            irq_line: 12,
            irq_asserted: false,
            busy_end_cycle: 0,
            audio_frame_start_cycle: 0,
            fm_sync_cursor: 0,
            sample_remainder: FmSampleRemainder::default(),
            alternate_timers: false,
        }
    }
}

/// Action the bus must process after a sound board operation.
#[derive(Clone, Copy)]
pub enum Soundboard26kAction {
    /// Schedule a timer to fire at the given cycle.
    ScheduleTimer {
        /// Timer event kind.
        kind: EventKind,
        /// CPU cycle at which the timer should fire.
        fire_cycle: u64,
    },
    /// Cancel a previously scheduled timer.
    CancelTimer {
        /// Timer event kind.
        kind: EventKind,
    },
    /// Assert an IRQ line on the PIC.
    AssertIrq {
        /// IRQ line number.
        irq: u8,
    },
    /// Deassert an IRQ line on the PIC.
    DeassertIrq {
        /// IRQ line number.
        irq: u8,
    },
}

/// PC-9801-26K sound board: YM2203 (OPN) FM + SSG synthesis with resampling.
pub struct Soundboard26k {
    /// Current device state (saveable).
    pub state: Soundboard26kState,
    core: OpnFm<Ym2203>,
    action_buffer: Vec<Soundboard26kAction>,
}

impl Soundboard26k {
    /// Creates a new PC-9801-26K sound board instance.
    ///
    /// When `alternate_timers` is `true`, uses `FmTimer2A`/`FmTimer2B` event
    /// kinds instead of `FmTimerA`/`FmTimerB` (for dual-board configurations).
    pub fn new(cpu_clock_hz: u32, sample_rate: u32, alternate_timers: bool) -> Self {
        let core = OpnFm::<Ym2203>::new(cpu_clock_hz, sample_rate);
        let state = Soundboard26kState {
            alternate_timers,
            ..Default::default()
        };
        Self {
            state,
            core,
            action_buffer: Vec::new(),
        }
    }

    const fn timer_kind(&self, timer_id: u8) -> EventKind {
        match (self.state.alternate_timers, timer_id) {
            (true, 0) => EventKind::FmTimer2A,
            (true, _) => EventKind::FmTimer2B,
            (false, 0) => EventKind::FmTimerA,
            (false, _) => EventKind::FmTimerB,
        }
    }

    /// Returns the currently latched register address.
    pub fn address(&self) -> u8 {
        self.state.address
    }

    /// Returns the configured IRQ line number.
    pub fn irq_line(&self) -> u8 {
        self.state.irq_line
    }

    /// Sets the address latch to a specific value (used by ITF initialization).
    pub fn set_address(&mut self, address: u8) {
        self.state.address = address;
    }

    /// Reads the chip status register (port 0x0188 read).
    pub fn read_status(&mut self, current_cycle: u64) -> u8 {
        self.core.read_status(current_cycle)
    }

    /// Reads data from the currently addressed register (port 0x018A read).
    ///
    /// Caller must call `drain_actions()` afterward.
    pub fn read_data(&mut self, current_cycle: u64) -> u8 {
        self.core.read_data(current_cycle)
    }

    /// Writes the register address latch (port 0x0188 write).
    ///
    /// Caller must call `drain_actions()` afterward.
    pub fn write_address(&mut self, value: u8, current_cycle: u64) {
        self.state.address = value;
        self.core.write_address(value, current_cycle);
    }

    /// Writes data to the currently addressed register (port 0x018A write).
    ///
    /// Caller must call `drain_actions()` afterward.
    pub fn write_data(&mut self, value: u8, current_cycle: u64) {
        self.core.write_data(value, current_cycle);
    }

    /// Notifies the chip that a timer has expired.
    ///
    /// Caller must call `drain_actions()` afterward.
    pub fn timer_expired(&mut self, timer_id: u32, current_cycle: u64) {
        self.core.timer_expired(timer_id, current_cycle);
    }

    /// Drains pending actions from the chip.
    ///
    /// Returns actions the bus must process (timer scheduling and IRQ
    /// assertion/deassertion). Also updates `state.irq_asserted` internally.
    pub fn drain_actions(&mut self) -> &[Soundboard26kAction] {
        self.action_buffer.clear();

        // At most two timer actions; copy out to release the core borrow.
        let timers: [Option<FmTimerAction>; 2] = {
            let actions = self.core.drain_timers();
            let mut out = [None, None];
            for (slot, action) in out.iter_mut().zip(actions.iter()) {
                *slot = Some(*action);
            }
            out
        };
        for action in timers.into_iter().flatten() {
            match action {
                FmTimerAction::Cancel { timer_id } => {
                    self.action_buffer.push(Soundboard26kAction::CancelTimer {
                        kind: self.timer_kind(timer_id),
                    });
                }
                FmTimerAction::Schedule {
                    timer_id,
                    fire_cycle,
                } => {
                    self.action_buffer.push(Soundboard26kAction::ScheduleTimer {
                        kind: self.timer_kind(timer_id),
                        fire_cycle,
                    });
                }
            }
        }

        if let Some(asserted) = self.core.take_irq_change() {
            self.state.irq_asserted = asserted;
            if asserted {
                self.action_buffer.push(Soundboard26kAction::AssertIrq {
                    irq: self.state.irq_line,
                });
            } else {
                self.action_buffer.push(Soundboard26kAction::DeassertIrq {
                    irq: self.state.irq_line,
                });
            }
        }
        self.action_buffer.as_slice()
    }

    /// Generates resampled FM+SSG audio and mixes it into `output`.
    ///
    /// `output` is interleaved stereo (`[L, R, L, R, …]`); the YM2203 (mono)
    /// output is duplicated to both channels.
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

    /// Creates a snapshot of the current state for save/restore.
    pub fn save_state(&self) -> Soundboard26kState {
        let timing = self.core.timing();
        let mut state = self.state.clone();
        state.sample_remainder = timing.sample_remainder;
        state.fm_sync_cursor = timing.fm_sync_cursor;
        state.busy_end_cycle = timing.busy_end_cycle;
        state.audio_frame_start_cycle = timing.audio_frame_start_cycle;
        state.irq_asserted = timing.irq_asserted;
        state
    }

    /// Restores from a saved state, recreating the ymfm chip.
    pub fn load_state(
        &mut self,
        saved: &Soundboard26kState,
        cpu_clock_hz: u32,
        sample_rate: u32,
        current_cycle: u64,
    ) {
        self.state = saved.clone();
        self.core.reload(cpu_clock_hz, sample_rate, current_cycle);
        self.core.set_timing(OpnFmTiming {
            sample_remainder: saved.sample_remainder,
            fm_sync_cursor: saved.fm_sync_cursor,
            busy_end_cycle: saved.busy_end_cycle,
            audio_frame_start_cycle: saved.audio_frame_start_cycle,
            irq_asserted: saved.irq_asserted,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_timer_a_schedules_with_board_event_kind() {
        let mut board = Soundboard26k::new(4_000_000, 48_000, false);
        board.write_address(0x24, 0);
        board.write_data(0xFF, 0);
        board.write_address(0x25, 0);
        board.write_data(0x03, 0);
        board.write_address(0x27, 0);
        board.write_data(0x01, 0);
        let scheduled = board.drain_actions().iter().any(|a| {
            matches!(
                a,
                Soundboard26kAction::ScheduleTimer {
                    kind: EventKind::FmTimerA,
                    ..
                }
            )
        });
        assert!(scheduled, "timer A load maps to FmTimerA");
    }

    #[test]
    fn alternate_timers_use_secondary_event_kinds() {
        let mut board = Soundboard26k::new(4_000_000, 48_000, true);
        board.write_address(0x24, 0);
        board.write_data(0xFF, 0);
        board.write_address(0x25, 0);
        board.write_data(0x03, 0);
        board.write_address(0x27, 0);
        board.write_data(0x01, 0);
        let scheduled = board.drain_actions().iter().any(|a| {
            matches!(
                a,
                Soundboard26kAction::ScheduleTimer {
                    kind: EventKind::FmTimer2A,
                    ..
                }
            )
        });
        assert!(scheduled, "alternate timers map timer A to FmTimer2A");
    }

    #[test]
    fn save_state_round_trips_timing() {
        let mut board = Soundboard26k::new(4_000_000, 48_000, false);
        board.write_address(0x28, 100);
        board.write_data(0xF0, 100);
        let mut out = vec![0.0f32; 64 * 2];
        board.generate_samples(2_000, 4_000_000, 1.0, &mut out);
        let saved = board.save_state();
        let mut restored = Soundboard26k::new(4_000_000, 48_000, false);
        restored.load_state(&saved, 4_000_000, 48_000, 2_000);
        assert_eq!(restored.save_state(), saved);
    }
}
