//! PC-9801-14 Music Generator sound board.
//!
//! The board combines:
//! - A single TMS3631-RI104 synthesizer (see [`tms3631`]).
//! - An 8255A PPI that writes channel keys via a handshake protocol on Port C.
//! - An 8253 PIT with only counter #2 accessible, wired through a strap
//!   switch to one of INT0/INT41/INT5/INT6 (default INT5 = IRQ 12).

pub mod tms3631;

use tms3631::Tms3631;

/// Timers exposed by the PC-9801-14 board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Soundboard14Timer {
    /// Counter 2 terminal count.
    Timer,
}

/// Default IRQ line: strap SW defaults to INT5 which maps to IRQ 12.
const DEFAULT_IRQ_LINE: u8 = 12;

/// Port 0x008E read: I/O-address DIP switch value (per undoc98).
const DIP_SWITCH_VALUE: u8 = 0x08;

/// Port 0x018E read: interrupt strap = INT5 (bits 7:6 = 11b).
const STRAP_SWITCH_VALUE: u8 = 0x80;

/// 8253 PIT counter #2 input clock: 1.9968 MHz / 8.
const PIT_CLOCK_HZ: u32 = 1_996_800 / 8;

/// Action the bus must process after a Soundboard14 operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Soundboard14Action {
    /// Schedule the 8253 counter #2 to fire at the given CPU cycle.
    ScheduleTimer {
        /// Timer event kind.
        kind: Soundboard14Timer,
        /// Absolute CPU cycle when the timer should fire.
        fire_cycle: u64,
    },
    /// Cancel the previously scheduled timer.
    CancelTimer {
        /// Timer event kind.
        kind: Soundboard14Timer,
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

/// Port C handshake decoder state.
///
/// Port C on the 8255A PPI carries bit 7 = CE, bit 6 = WCLK, bits 5:0 = key
/// data. The TMS3631 latches one channel per WCLK pulse, starting from CH1
/// when CE goes high.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PortCState {
    /// Last value written to Port C (bit 7 = CE, bit 6 = WCLK).
    pub last: u8,
    /// Channel index currently addressed for the next write.
    pub channel: u8,
    /// Handshake-armed flag (latched on CE rising edge).
    pub sync: bool,
}

/// 8253 counter #2 state (the only channel exposed on this board).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pit8253State {
    /// Control-word register (from 0x018E writes). 0 means no mode loaded.
    pub control: u8,
    /// Current reload value (16-bit).
    pub reload: u16,
    /// Next byte of the reload being assembled (LSB then MSB).
    pub writing_msb_next: bool,
    /// Whether the counter is actively counting.
    pub running: bool,
    /// CPU cycle the counter was most recently loaded/fired at.
    pub start_cycle: u64,
    /// Current CPU-clock frequency (needed to convert PIT ticks to cycles).
    pub cpu_clock_hz: u32,
}

/// Snapshot of PC-9801-14 state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Soundboard14State {
    /// 8255A Port A (envelope 1 latch; otherwise unused).
    pub port_a: u8,
    /// 8255A Port B (envelope 2 latch; otherwise unused).
    pub port_b: u8,
    /// 8255A Port C handshake state.
    pub port_c: PortCState,
    /// 8255A mode register latch (0x008E writes).
    pub ppi_mode: u8,
    /// Latched 6-bit keys for each of the 8 channels.
    pub keys: [u8; 8],
    /// 8253 counter #2 state.
    pub pit: Pit8253State,
    /// IRQ line on the PIC.
    pub irq_line: u8,
    /// Whether the IRQ output is currently asserted.
    pub irq_asserted: bool,
}

impl Default for Soundboard14State {
    fn default() -> Self {
        Self {
            port_a: 0,
            port_b: 0,
            port_c: PortCState::default(),
            ppi_mode: 0,
            keys: [0; 8],
            pit: Pit8253State::default(),
            irq_line: DEFAULT_IRQ_LINE,
            irq_asserted: false,
        }
    }
}

/// PC-9801-14 Music Generator Board.
pub struct Soundboard14 {
    /// Current state (saveable).
    pub state: Soundboard14State,
    chip: Tms3631,
    cpu_clock_hz: u32,
    /// Actions accumulated by port writes since the last `drain_actions()`.
    pending_actions: Vec<Soundboard14Action>,
    /// Reusable buffer returned by `drain_actions()`. Cleared on entry of
    /// each `drain_actions()` call so the slice reflects only new events.
    action_buffer: Vec<Soundboard14Action>,
}

impl Soundboard14 {
    /// Creates a new board instance configured for the given CPU and sample
    /// rate.
    pub fn new(cpu_clock_hz: u32, sample_rate: u32) -> Self {
        let mut state = Soundboard14State::default();
        state.pit.cpu_clock_hz = cpu_clock_hz;
        Self {
            state,
            chip: Tms3631::new(sample_rate),
            cpu_clock_hz,
            pending_actions: Vec::new(),
            action_buffer: Vec::new(),
        }
    }

    /// Returns the configured IRQ line number.
    pub fn irq_line(&self) -> u8 {
        self.state.irq_line
    }

    /// Read 0x0088 / 0x008A / 0x008C: PPI Port A / B / C latched values.
    pub fn read_port_a(&self) -> u8 {
        self.state.port_a
    }

    /// Read Port B latch (0x008A).
    pub fn read_port_b(&self) -> u8 {
        self.state.port_b
    }

    /// Read Port C latch (0x008C).
    pub fn read_port_c(&self) -> u8 {
        self.state.port_c.last
    }

    /// Read 0x008E: fixed DIP-switch value (undoc98).
    pub fn read_dip_switch(&self) -> u8 {
        DIP_SWITCH_VALUE
    }

    /// Read 0x018E: fixed strap-switch value (INT5 by default).
    pub fn read_strap_switch(&self) -> u8 {
        STRAP_SWITCH_VALUE
    }

    /// Write 0x0088: latch 8255A Port A (envelope 1). No audible effect.
    pub fn write_port_a(&mut self, value: u8) {
        self.state.port_a = value;
    }

    /// Write 0x008A: latch 8255A Port B (envelope 2). No audible effect.
    pub fn write_port_b(&mut self, value: u8) {
        self.state.port_b = value;
    }

    /// Write 0x008C: run the TMS3631 key-write handshake, latching the
    /// current channel's key when a valid WCLK strobe arrives.
    pub fn write_port_c(&mut self, value: u8) {
        let previous = self.state.port_c.last;
        let ce_high = value & 0x80 != 0;
        let ce_previously_high = previous & 0x80 != 0;
        let wclk_high = value & 0x40 != 0;
        let wclk_previously_high = previous & 0x40 != 0;
        let key = value & 0x3F;

        if ce_high {
            if !ce_previously_high {
                // CE rising edge: start write cycle at CH1.
                self.state.port_c.sync = true;
                self.state.port_c.channel = 0;
            } else if self.state.port_c.sync {
                // First sample with sync armed: write to current channel.
                self.state.port_c.sync = false;
                let ch = self.state.port_c.channel & 0x07;
                self.state.keys[ch as usize] = key;
                self.chip.set_key(ch, key);
            } else if !wclk_high && wclk_previously_high {
                // WCLK falling edge: advance to the next channel and re-arm.
                self.state.port_c.sync = true;
                self.state.port_c.channel = (self.state.port_c.channel + 1) & 0x07;
            }
        }

        self.state.port_c.last = value;
    }

    /// Write 0x008E: 8255A mode register. Hardware-wise this (re)configures
    /// the PPI's port directions.
    pub fn write_ppi_mode(&mut self, value: u8) {
        self.state.ppi_mode = value;
    }

    /// Write 0x0188 / 0x018A: TMS3631 channel-enable mask.
    pub fn write_enable_mask(&mut self, value: u8) {
        self.chip.set_enable(value);
    }

    /// Read 0x0188 / 0x018A: current channel-enable mask (write-only on the real board).
    pub fn read_enable_mask(&self) -> u8 {
        self.chip.enable_mask()
    }

    /// Read 0x018C: the current 8253 counter #2 value.
    pub fn read_pit_counter(&self) -> u8 {
        // We return open-bus value as is NP21W doing.
        0xFF
    }

    /// Write 0x018C: reload the 8253 counter #2 (LSB then MSB).
    pub fn write_pit_counter(&mut self, value: u8, current_cycle: u64) {
        if self.state.pit.writing_msb_next {
            self.state.pit.reload = (self.state.pit.reload & 0x00FF) | (u16::from(value) << 8);
            self.state.pit.writing_msb_next = false;
            self.start_pit(current_cycle);
        } else {
            self.state.pit.reload = (self.state.pit.reload & 0xFF00) | u16::from(value);
            self.state.pit.writing_msb_next = true;
            // If the control word loaded mode + RL=LSB only, the timer
            // starts on this write. Otherwise it waits for the MSB.
            let rl_bits = (self.state.pit.control >> 4) & 0x03;
            if rl_bits == 0x01 {
                self.start_pit(current_cycle);
            }
        }
    }

    /// Write 0x018E: 8253 control-word register.
    ///
    /// Format: `SC[7:6] RL[5:4] M[3:1] BCD[0]`.
    pub fn write_pit_control(&mut self, value: u8, _current_cycle: u64) {
        self.state.pit.control = value;
        self.state.pit.writing_msb_next = false;
        self.state.pit.reload = 0;
        self.state.pit.running = false;
        // Cancel any previously scheduled event until the counter is reloaded.
        self.pending_actions.push(Soundboard14Action::CancelTimer {
            kind: Soundboard14Timer::Timer,
        });
    }

    /// Called by the bus when the scheduler fires `MusicGen14Timer`.
    pub fn timer_expired(&mut self, current_cycle: u64) {
        // Assert the IRQ.
        if !self.state.irq_asserted {
            self.state.irq_asserted = true;
            self.pending_actions.push(Soundboard14Action::AssertIrq {
                irq: self.state.irq_line,
            });
        }
        // Mode 2 (rate generator) and mode 3 (square wave) are periodic.
        let mode = (self.state.pit.control >> 1) & 0x07;
        let periodic = matches!(mode, 2 | 3 | 6 | 7);
        if periodic && self.state.pit.reload != 0 {
            self.start_pit(current_cycle);
        } else {
            self.state.pit.running = false;
        }
    }

    fn start_pit(&mut self, current_cycle: u64) {
        let reload = if self.state.pit.reload == 0 {
            0x10000u64
        } else {
            u64::from(self.state.pit.reload)
        };
        let cpu_ticks = reload * u64::from(self.cpu_clock_hz) / u64::from(PIT_CLOCK_HZ);
        self.state.pit.start_cycle = current_cycle;
        self.state.pit.running = true;
        self.pending_actions
            .push(Soundboard14Action::ScheduleTimer {
                kind: Soundboard14Timer::Timer,
                fire_cycle: current_cycle.saturating_add(cpu_ticks),
            });
    }

    /// Drain accumulated actions for the bus to apply.
    ///
    /// Returns a borrow of the reused `action_buffer`; the buffer is cleared
    /// on entry and refilled from `pending_actions`, matching the
    /// non-allocating pattern used by the 26K/86 boards.
    pub fn drain_actions(&mut self) -> &[Soundboard14Action] {
        self.action_buffer.clear();
        self.action_buffer.append(&mut self.pending_actions);
        self.action_buffer.as_slice()
    }

    /// Returns true if any actions have been queued since the last drain.
    pub fn has_actions(&self) -> bool {
        !self.pending_actions.is_empty()
    }

    /// Renders the board's stereo audio into `output`, additively.
    pub fn generate_samples(&mut self, volume: f32, output: &mut [f32]) {
        // 32767 is the nominal i16 peak; feet values are ~(15 << 5) * 15 = 7200
        // and CH1 adds up to 4 * 15 (scaled). The combined peak fits inside i16.
        const BASE_SCALE: f32 = 1.0 / 32768.0;
        self.chip.render(output, BASE_SCALE * volume);
    }

    /// Creates a snapshot of the current state for save/restore.
    pub fn save_state(&self) -> Soundboard14State {
        self.state.clone()
    }

    /// Restores from a saved state.
    pub fn load_state(
        &mut self,
        saved: &Soundboard14State,
        cpu_clock_hz: u32,
        _sample_rate: u32,
        _current_cycle: u64,
    ) {
        self.state = saved.clone();
        self.state.pit.cpu_clock_hz = cpu_clock_hz;
        self.cpu_clock_hz = cpu_clock_hz;
        // Re-synthesize the TMS3631 state from the saved key values.
        self.chip.reset();
        for (ch, key) in self.state.keys.iter().enumerate() {
            self.chip.set_key(ch as u8, *key);
        }
        // The enable mask is not captured separately from the chip; the
        // last-known chip setting is restored on the next write.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPU_CLOCK: u32 = 10_000_000;
    const SAMPLE_RATE: u32 = 48_000;

    #[test]
    fn port_c_handshake_programs_each_channel_sequentially() {
        let mut board = Soundboard14::new(CPU_CLOCK, SAMPLE_RATE);

        // CE rising: sync armed, channel reset to 0.
        board.write_port_c(0x80); // CE high, WCLK low, key=0
        // Next write with CE still high: latch key into CH0.
        let key0 = (3u8 << 4) | 10; // top octave A
        board.write_port_c(0x80 | key0);
        assert_eq!(board.state.keys[0], key0);

        // Toggle WCLK high then low to advance to CH1.
        board.write_port_c(0xC0); // WCLK rising (bit 6 on)
        board.write_port_c(0x80); // WCLK falling -> advance channel to 1
        let key1 = (3u8 << 4) | 1; // top octave C
        board.write_port_c(0x80 | key1);
        assert_eq!(board.state.keys[1], key1);
    }

    #[test]
    fn read_ports_return_expected_values() {
        let board = Soundboard14::new(CPU_CLOCK, SAMPLE_RATE);
        assert_eq!(board.read_dip_switch(), 0x08);
        assert_eq!(board.read_strap_switch(), 0x80);
        assert_eq!(board.read_enable_mask(), 0);
    }

    #[test]
    fn enable_mask_round_trip() {
        let mut board = Soundboard14::new(CPU_CLOCK, SAMPLE_RATE);
        board.write_enable_mask(0x5A);
        assert_eq!(board.read_enable_mask(), 0x5A);
    }

    #[test]
    fn write_control_cancels_scheduled_timer() {
        let mut board = Soundboard14::new(CPU_CLOCK, SAMPLE_RATE);
        board.write_pit_control(0xB6, 0);
        let actions = board.drain_actions();
        assert_eq!(
            actions,
            vec![Soundboard14Action::CancelTimer {
                kind: Soundboard14Timer::Timer
            }]
        );
    }

    #[test]
    fn program_mode3_schedules_timer() {
        let mut board = Soundboard14::new(CPU_CLOCK, SAMPLE_RATE);
        board.write_pit_control(0xB6, 0); // mode 3, LSB then MSB
        let _ = board.drain_actions();
        board.write_pit_counter(0xFF, 0); // LSB
        board.write_pit_counter(0xFF, 0); // MSB -> should schedule
        let actions = board.drain_actions();
        assert!(matches!(
            actions.last(),
            Some(Soundboard14Action::ScheduleTimer {
                kind: Soundboard14Timer::Timer,
                ..
            })
        ));
    }

    #[test]
    fn timer_expired_asserts_irq_and_reschedules_in_mode3() {
        let mut board = Soundboard14::new(CPU_CLOCK, SAMPLE_RATE);
        board.write_pit_control(0xB6, 0); // mode 3, LSB then MSB
        let _ = board.drain_actions();
        board.write_pit_counter(0xFF, 0);
        board.write_pit_counter(0xFF, 0);
        let _ = board.drain_actions();

        board.timer_expired(1_000_000);
        let actions = board.drain_actions();
        let has_assert = actions
            .iter()
            .any(|a| matches!(a, Soundboard14Action::AssertIrq { irq: 12 }));
        let has_reschedule = actions
            .iter()
            .any(|a| matches!(a, Soundboard14Action::ScheduleTimer { .. }));
        assert!(has_assert && has_reschedule);
    }
}
