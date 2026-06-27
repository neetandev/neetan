//! i8253 Programmable Interval Timer for the PC-98.
//!
//! On the PC-98, all GATE pins are hardwired HIGH (active), so modes 1 and 5
//! (which require a rising GATE edge to start) are effectively unusable.
//! No GATE handling is needed. See `undoc98/io_tcu.txt`.
//!
//! The BIOS programs channel 0 in mode 3 (square wave) for the 100 Hz
//! interval timer. Counting uses lazy evaluation: the current count is
//! computed on-demand from the reload value and elapsed CPU cycles.

use std::ops::{Deref, DerefMut};

use common::{EventKind, Scheduler};

/// Control word mask: read/load mode (bits 5:4).
pub const PIT_CTRL_RL: u8 = 0x30;
/// RL field: low byte only (01b).
pub const PIT_RL_L: u8 = 0x10;
/// RL field: high byte only (10b).
pub const PIT_RL_H: u8 = 0x20;
/// RL field: low byte then high byte (11b).
pub const PIT_RL_ALL: u8 = 0x30;
/// Internal flag: control word written but counter not yet loaded.
pub const PIT_STAT_CMD: u8 = 0x40;

/// Channel flag: read toggle (alternates low/high byte on read).
pub const PIT_FLAG_R: u8 = 0x01;
/// Channel flag: write toggle (alternates low/high byte on write).
pub const PIT_FLAG_W: u8 = 0x02;
/// Channel flag: latch extend (second byte of latched word pending).
pub const PIT_FLAG_L: u8 = 0x04;
/// Channel flag: counter value latched.
pub const PIT_FLAG_C: u8 = 0x10;
/// Channel flag: interrupt pending (output not yet asserted).
pub const PIT_FLAG_I: u8 = 0x20;

/// Result of a counter write operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteResult {
    /// Caller should skip event scheduling (incomplete word write or mode 1 inhibit).
    Skip,
    /// First load after a control word - always takes effect immediately.
    InitialLoad,
    /// Reload while already counting - in modes 2/3, should be deferred.
    SubsequentLoad,
}

/// Channel 0 control word.
/// Bits: SC=01, RL=01 (LSB only), M=011 (mode 3, square wave), BCD=0.
/// Stored as `0x56 & 0x3F` = 0x16 (SC bits stripped for the ctrl register).
const CH0_CTRL_WORD: u8 = 0x56;

/// Beep counter reload value for 8 MHz-series machines.
/// 2.4576 MHz / 998 ~ 2.463 kHz (approximately 2 kHz beep).
const BEEP_COUNTER_8MHZ: u16 = 998;

/// Beep counter reload value for 5/10 MHz-series machines.
/// 1.9968 MHz / 1229 ~ 1.625 kHz (approximately 2 kHz beep).
const BEEP_COUNTER_5_10MHZ: u16 = 1229;

/// Channel 2 control word written by BIOS: 0xB6.
/// Bits: SC=10 (ch2), RL=11 (LSB then MSB), M=011 (mode 3, square wave), BCD=0.
/// Stored as `0xB6 & 0x3F` = 0x36 (strip SC bits for the ctrl register).
const CH2_CTRL_WORD: u8 = 0xB6;

/// Snapshot of a single PIT channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I8253PitChannelState {
    /// Control word.
    pub ctrl: u8,
    /// Channel flags.
    pub flag: u8,
    /// Programmed reload value.
    pub value: u16,
    /// Latched counter snapshot.
    pub latch: u16,
    /// CPU cycle when counter was last loaded.
    pub last_load_cycle: u64,
    /// Current output pin state.
    pub output: bool,
    /// Pending reload value for modes 2/3 (applied at next terminal count).
    pub reload_pending: Option<u16>,
}

/// Snapshot of the i8253 PIT (all 3 channels).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I8253PitState {
    /// The three channel snapshots.
    pub channels: [I8253PitChannelState; 3],
}

/// i8253 PIT with 3 channels.
pub struct I8253Pit {
    /// Embedded state for save/restore.
    pub state: I8253PitState,
}

impl Deref for I8253Pit {
    type Target = I8253PitState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for I8253Pit {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Default for I8253Pit {
    fn default() -> Self {
        Self::new(true)
    }
}

impl I8253Pit {
    /// Creates a new PIT in the PC-98 boot default state.
    ///
    /// `is_8mhz_lineage`: selects the beep counter reload value.
    ///   - `true`  -> 998  (8 MHz-series, PIT clock 2.4576 MHz)
    ///   - `false` -> 1229 (5/10 MHz-series, PIT clock 1.9968 MHz)
    pub fn new(is_8mhz_lineage: bool) -> Self {
        let beep_counter = if is_8mhz_lineage {
            BEEP_COUNTER_8MHZ
        } else {
            BEEP_COUNTER_5_10MHZ
        };

        Self {
            state: I8253PitState {
                channels: [
                    // Channel 0: interval timer (100 Hz system tick).
                    // BIOS programs mode 3 (square wave) via control word 0x56.
                    I8253PitChannelState {
                        ctrl: CH0_CTRL_WORD & 0x3F,
                        flag: PIT_FLAG_I,
                        value: 0,
                        latch: 0,
                        last_load_cycle: 0,
                        output: true,
                        reload_pending: None,
                    },
                    // Channel 1: beep tone generator.
                    // Counter value is lineage-dependent; ctrl/flag zeroed.
                    I8253PitChannelState {
                        ctrl: 0,
                        flag: 0,
                        value: beep_counter,
                        latch: 0,
                        last_load_cycle: 0,
                        output: false,
                        reload_pending: None,
                    },
                    // Channel 2: RS-232C baud rate generator.
                    // BIOS programs mode 3 (square wave) via control word 0xB6.
                    I8253PitChannelState {
                        ctrl: CH2_CTRL_WORD & 0x3F,
                        flag: 0,
                        value: 0,
                        latch: 0,
                        last_load_cycle: 0,
                        output: true,
                        reload_pending: None,
                    },
                ],
            },
        }
    }

    /// Creates a new PIT with all registers zeroed.
    pub fn new_zeroed() -> Self {
        Self {
            state: I8253PitState {
                channels: [
                    I8253PitChannelState {
                        ctrl: 0,
                        flag: 0,
                        value: 0,
                        latch: 0,
                        last_load_cycle: 0,
                        output: false,
                        reload_pending: None,
                    },
                    I8253PitChannelState {
                        ctrl: 0,
                        flag: 0,
                        value: 0,
                        latch: 0,
                        last_load_cycle: 0,
                        output: false,
                        reload_pending: None,
                    },
                    I8253PitChannelState {
                        ctrl: 0,
                        flag: 0,
                        value: 0,
                        latch: 0,
                        last_load_cycle: 0,
                        output: false,
                        reload_pending: None,
                    },
                ],
            },
        }
    }

    /// Writes the control word for a channel.
    /// If RL=00, latches the counter. Otherwise, sets the mode/access.
    pub fn write_control(
        &mut self,
        channel: usize,
        value: u8,
        current_cycle: u64,
        cpu_clock_hz: u32,
        pit_clock_hz: u32,
    ) {
        let ch = &mut self.channels[channel];
        if value & PIT_CTRL_RL != 0 {
            // Mode set
            ch.ctrl = (value & 0x3F) | PIT_STAT_CMD;
            ch.flag &= !(PIT_FLAG_R | PIT_FLAG_W | PIT_FLAG_L | PIT_FLAG_C);
            ch.reload_pending = None;
            let mode = (value >> 1) & 7;
            ch.output = mode != 0;
        } else {
            // Latch command: snapshot current count
            let count = get_count(ch, current_cycle, cpu_clock_hz, pit_clock_hz);
            ch.flag |= PIT_FLAG_C;
            ch.flag &= !PIT_FLAG_L;
            ch.latch = count;
        }
    }

    /// Writes a byte to a channel's counter register.
    ///
    /// Returns `WriteResult::Skip` if the caller should skip event scheduling
    /// (word mode first byte, or mode 1 inhibit), `WriteResult::InitialLoad`
    /// for the first load after a control word, or `WriteResult::SubsequentLoad`
    /// for a reload while already counting.
    pub fn write_counter(&mut self, channel: usize, value: u8) -> WriteResult {
        let ch = &mut self.channels[channel];
        let is_initial = ch.ctrl & PIT_STAT_CMD != 0;

        match ch.ctrl & PIT_CTRL_RL {
            PIT_RL_L => {
                ch.value = value as u16;
            }
            PIT_RL_H => {
                ch.value = (value as u16) << 8;
            }
            PIT_RL_ALL => {
                ch.flag ^= PIT_FLAG_W;
                if ch.flag & PIT_FLAG_W != 0 {
                    // First byte (LSB): in mode 0, output drops LOW immediately.
                    let mode = (ch.ctrl >> 1) & 7;
                    if mode == 0 {
                        ch.output = false;
                    }
                    ch.value = (ch.value & 0xFF00) | value as u16;
                    return WriteResult::Skip;
                }
                // Second byte (MSB)
                ch.value = (ch.value & 0x00FF) | ((value as u16) << 8);
            }
            _ => {}
        }

        ch.ctrl &= !PIT_STAT_CMD;

        // Mode 1 with I flag: don't restart
        if (ch.ctrl & 0x06) == 0x02 && (ch.flag & PIT_FLAG_I != 0) {
            return WriteResult::Skip;
        }

        if is_initial {
            WriteResult::InitialLoad
        } else {
            WriteResult::SubsequentLoad
        }
    }

    /// Reads a byte from a channel's counter register.
    pub fn read_counter(
        &mut self,
        channel: usize,
        current_cycle: u64,
        cpu_clock_hz: u32,
        pit_clock_hz: u32,
    ) -> u8 {
        let ch = &mut self.channels[channel];
        let rl = ch.ctrl & PIT_CTRL_RL;

        let word = if ch.flag & (PIT_FLAG_C | PIT_FLAG_L) != 0 {
            ch.flag &= !PIT_FLAG_C;
            if rl == PIT_RL_ALL {
                ch.flag ^= PIT_FLAG_L;
            }
            ch.latch
        } else {
            get_count(ch, current_cycle, cpu_clock_hz, pit_clock_hz)
        };

        match rl {
            PIT_RL_L => word as u8,
            PIT_RL_H => (word >> 8) as u8,
            _ => {
                let result = if ch.flag & PIT_FLAG_R == 0 {
                    word as u8
                } else {
                    (word >> 8) as u8
                };
                ch.flag ^= PIT_FLAG_R;
                result
            }
        }
    }

    /// Computes the current count for a channel.
    pub fn get_count(
        &self,
        channel: usize,
        current_cycle: u64,
        cpu_clock_hz: u32,
        pit_clock_hz: u32,
    ) -> u16 {
        get_count(
            &self.channels[channel],
            current_cycle,
            cpu_clock_hz,
            pit_clock_hz,
        )
    }

    /// Computes the current output pin state for a channel.
    pub fn get_output(
        &self,
        channel: usize,
        current_cycle: u64,
        cpu_clock_hz: u32,
        pit_clock_hz: u32,
    ) -> bool {
        let ch = &self.channels[channel];
        let count_period = count_period(ch);
        let elapsed_cpu = current_cycle.saturating_sub(ch.last_load_cycle);
        if elapsed_cpu == 0 {
            return ch.output;
        }
        let elapsed_pit = elapsed_cpu * pit_clock_hz as u64 / cpu_clock_hz as u64;
        let mode = (ch.ctrl >> 1) & 7;
        match mode {
            0 => elapsed_pit >= count_period,
            2 => {
                let pos = elapsed_pit % count_period;
                pos != count_period - 1
            }
            3 => {
                let high_half = count_period.div_ceil(2);
                let pos = elapsed_pit % count_period;
                pos < high_half
            }
            4 => {
                if elapsed_pit >= count_period {
                    let past = elapsed_pit - count_period;
                    past >= 1
                } else {
                    true
                }
            }
            _ => ch.output,
        }
    }

    /// Schedules the next timer 0 event based on the current reload value.
    pub fn schedule_timer0(
        &self,
        scheduler: &mut Scheduler,
        cpu_clock_hz: u32,
        pit_clock_hz: u32,
        current_cycle: u64,
    ) {
        let cpu_cycles = self.timer0_period_cycles(cpu_clock_hz, pit_clock_hz);
        scheduler.schedule(EventKind::PitTimer0, current_cycle + cpu_cycles);
    }

    /// Returns the timer 0 reload period expressed in CPU cycles. Used by buses
    /// that drive their own scheduler instead of the common [`Scheduler`].
    pub fn timer0_period_cycles(&self, cpu_clock_hz: u32, pit_clock_hz: u32) -> u64 {
        let ch = &self.channels[0];
        let reload = count_period(ch);
        reload * cpu_clock_hz as u64 / pit_clock_hz as u64
    }

    /// Handles the timer 0 event. Returns `true` if an IRQ should be raised.
    pub fn on_timer0_event(
        &mut self,
        scheduler: &mut Scheduler,
        cpu_clock_hz: u32,
        pit_clock_hz: u32,
        current_cycle: u64,
    ) -> bool {
        let raise_irq = self.advance_timer0(current_cycle);
        self.schedule_timer0(scheduler, cpu_clock_hz, pit_clock_hz, current_cycle);
        raise_irq
    }

    /// Advances timer 0 at a fire event: clears/re-arms the interrupt flag,
    /// applies any deferred reload, and updates the output for the channel's
    /// mode. Returns `true` if an IRQ should be raised. The caller is
    /// responsible for rescheduling the next event (see
    /// [`Self::timer0_period_cycles`]).
    pub fn advance_timer0(&mut self, current_cycle: u64) -> bool {
        let ch = &mut self.channels[0];
        let raise_irq = ch.flag & PIT_FLAG_I != 0;
        if raise_irq {
            ch.flag &= !PIT_FLAG_I;
        }

        let mode = (ch.ctrl >> 1) & 7;

        // Apply pending reload if one was deferred from a subsequent write.
        if let Some(pending) = ch.reload_pending.take() {
            ch.value = pending;
        }

        // Update output state at terminal count.
        match mode {
            0 => ch.output = true,
            2 => {} // Output goes LOW for 1 tick then back HIGH; stays HIGH at reload.
            3 => ch.output = !ch.output,
            _ => {}
        }

        // Always reschedule ch0 so get_count() keeps working, but only
        // re-arm the interrupt flag for periodic modes (2 and 3).
        if mode == 2 || mode == 3 {
            ch.flag |= PIT_FLAG_I;
        }
        ch.last_load_cycle = current_cycle;
        raise_irq
    }
}

/// Returns the effective count period in PIT ticks, handling the 0 -> max convention
/// and BCD mode.
fn count_period(ch: &I8253PitChannelState) -> u64 {
    if ch.ctrl & 1 != 0 {
        // BCD mode: 0 means 10000 decimal ticks.
        if ch.value == 0 {
            10000
        } else {
            bcd_to_binary(ch.value)
        }
    } else if ch.value == 0 {
        0x10000u64
    } else {
        ch.value as u64
    }
}

/// Converts a 4-digit BCD value to its binary equivalent.
/// e.g. 0x1234 -> 1234, 0x9999 -> 9999.
fn bcd_to_binary(bcd: u16) -> u64 {
    let d0 = (bcd & 0x000F) as u64;
    let d1 = ((bcd >> 4) & 0x000F) as u64;
    let d2 = ((bcd >> 8) & 0x000F) as u64;
    let d3 = ((bcd >> 12) & 0x000F) as u64;
    d3 * 1000 + d2 * 100 + d1 * 10 + d0
}

/// Converts a binary value (0..=9999) to 4-digit BCD.
/// e.g. 1234 -> 0x1234, 9999 -> 0x9999.
fn binary_to_bcd(mut bin: u64) -> u16 {
    let d3 = bin / 1000;
    bin %= 1000;
    let d2 = bin / 100;
    bin %= 100;
    let d1 = bin / 10;
    let d0 = bin % 10;
    ((d3 << 12) | (d2 << 8) | (d1 << 4) | d0) as u16
}

/// Computes the current count value for a channel via lazy evaluation.
fn get_count(
    ch: &I8253PitChannelState,
    current_cycle: u64,
    cpu_clock_hz: u32,
    pit_clock_hz: u32,
) -> u16 {
    let period = count_period(ch);

    let elapsed_cpu = current_cycle.saturating_sub(ch.last_load_cycle);
    if elapsed_cpu == 0 {
        return ch.value;
    }

    let elapsed_pit = elapsed_cpu * pit_clock_hz as u64 / cpu_clock_hz as u64;
    let is_bcd = ch.ctrl & 1 != 0;

    let mode = (ch.ctrl >> 1) & 7;
    let binary_count = match mode {
        2 => {
            let pos = elapsed_pit % period;
            if pos == 0 { period } else { period - pos }
        }
        3 => {
            // Square wave: counting element decrements by 2 per PIT tick.
            // For odd reload values, the 8253 uses asymmetric half-periods:
            //   HIGH half: (N+1)/2 ticks - first tick decrements by 1, then by 2
            //   LOW  half: (N-1)/2 ticks - first tick decrements by 3, then by 2
            // For even values both halves are N/2 ticks, decrement by 2 throughout.
            let is_odd = period & 1 != 0;
            let high_half = period.div_ceil(2);
            let pos = elapsed_pit % period;
            if pos == 0 {
                period
            } else if pos < high_half {
                if is_odd {
                    period + 1 - pos * 2
                } else {
                    period - pos * 2
                }
            } else {
                let pos_in_low = pos - high_half;
                if pos_in_low == 0 {
                    period
                } else if is_odd {
                    period - 1 - pos_in_low * 2
                } else {
                    period - pos_in_low * 2
                }
            }
        }
        _ => {
            // One-shot / other modes: count down to 0.
            period.saturating_sub(elapsed_pit)
        }
    };

    if is_bcd {
        binary_to_bcd(binary_count)
    } else {
        binary_count as u16
    }
}
