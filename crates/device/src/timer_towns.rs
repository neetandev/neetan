//! FM Towns programmable interval timer.
//!
//! Two i8253 blocks provide six counter channels (0x0040-0x0047 and
//! 0x0050-0x0057). Unlike a PC-98, the timer interrupt is not taken directly
//! from a counter's OUT pin: channel 0 and channel 1 feed latched flags
//! (`TMOUT`), and the interval-control register at 0x0060 gates each with an
//! enable flag (`TMMSK`). IRQ 0 is asserted while either enabled flag is set.
//!
//! Only the behavior needed for the core chipset is modeled: counter readback,
//! and the channel-0 (periodic, mode 3) and channel-1 (one-shot, mode 0)
//! interrupt sources. The remaining channels are storage for register readback.

/// Base counter input clock (channels 0-3 and 5). Also the beep tone clock:
/// the buzzer frequency is this divided by channel 2's reload value.
pub const TIMER_CLOCK_HZ: u32 = 307_200;

/// The timer channel whose reload value sets the beep (buzzer) frequency.
const CHANNEL_BEEP: usize = 2;

/// The two interrupt-capable channels.
const CHANNEL_TIMER0: usize = 0;
const CHANNEL_TIMER1: usize = 1;

/// Control-word fields.
const CONTROL_CHANNEL_SHIFT: u8 = 6;
const CONTROL_ACCESS_SHIFT: u8 = 4;
const CONTROL_ACCESS_MASK: u8 = 0x03;
const CONTROL_MODE_SHIFT: u8 = 1;
const CONTROL_MODE_MASK: u8 = 0x07;

/// Access (read/load) format in the control word.
const ACCESS_LATCH: u8 = 0;
const ACCESS_LOW_ONLY: u8 = 1;
const ACCESS_HIGH_ONLY: u8 = 2;
const ACCESS_LOW_THEN_HIGH: u8 = 3;

/// Interval-control register (0x0060) bits.
const INTERVAL_TIMER0_ENABLE: u8 = 0x01;
const INTERVAL_TIMER1_ENABLE: u8 = 0x02;
const INTERVAL_SOUND_ENABLE: u8 = 0x04;
const INTERVAL_CLEAR_TIMER0_OUT: u8 = 0x80;

/// Interval-status readback bits.
const STATUS_TIMER0_OUT: u8 = 0x01;
const STATUS_TIMER1_OUT: u8 = 0x02;
const STATUS_TIMER0_ENABLE: u8 = 0x04;
const STATUS_TIMER1_ENABLE: u8 = 0x08;
const STATUS_SOUND_ENABLE: u8 = 0x10;

/// State of one counter channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct TimerChannel {
    /// Programmed reload value (0 means 65536).
    initial_count: u16,
    /// Counter mode (0-5); only 0, 2, and 3 are meaningfully modeled.
    mode: u8,
    /// Read/load access format.
    access: u8,
    /// Whether the channel is counting.
    counting: bool,
    /// Load phase for 16-bit (low-then-high) writes: false = low byte next.
    load_high: bool,
    /// Read phase for 16-bit reads: false = low byte next.
    read_high: bool,
    /// Assembled reload value during a low-then-high write.
    load_scratch: u16,
    /// CPU cycle at which the current count period began.
    last_load_cycle: u64,
}

save_state::runtime_state! {
/// Authoritative progress of one FM Towns interval-timer channel.
#[derive(Clone)]
struct TimerChannelRuntimeState {
    initial_count: u16,
    mode: u8,
    access: u8,
    counting: bool,
    load_high: bool,
    read_high: bool,
    load_scratch: u16,
    last_load_cycle: u64,
}}

save_state::runtime_state! {
/// Authoritative FM Towns interval timer state.
#[derive(Clone)]
pub struct TownsTimerRuntimeState {
    channels: [TimerChannelRuntimeState; 6],
    timer_out: [bool; 2],
    timer_enable: [bool; 2],
    sound_enable: bool,
}}

/// Snapshot of the timer state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TownsTimerState {
    channels: [TimerChannel; 6],
    /// Latched OUT flags for channel 0 and 1.
    timer_out: [bool; 2],
    /// Interrupt-enable flags for channel 0 and 1.
    timer_enable: [bool; 2],
    /// Beep (SOUND) enable flag.
    sound_enable: bool,
}

/// FM Towns interval timer.
pub struct TownsTimer {
    state: TownsTimerState,
}

impl Default for TownsTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl TownsTimer {
    /// Creates a timer in its reset state.
    pub fn new() -> Self {
        Self {
            state: TownsTimerState {
                channels: [TimerChannel::default(); 6],
                timer_out: [false; 2],
                timer_enable: [false; 2],
                sound_enable: false,
            },
        }
    }

    /// Captures every counter phase and interrupt latch.
    pub fn capture_state(&self) -> TownsTimerRuntimeState {
        TownsTimerRuntimeState {
            channels: self.state.channels.map(|channel| TimerChannelRuntimeState {
                initial_count: channel.initial_count,
                mode: channel.mode,
                access: channel.access,
                counting: channel.counting,
                load_high: channel.load_high,
                read_high: channel.read_high,
                load_scratch: channel.load_scratch,
                last_load_cycle: channel.last_load_cycle,
            }),
            timer_out: self.state.timer_out,
            timer_enable: self.state.timer_enable,
            sound_enable: self.state.sound_enable,
        }
    }

    /// Restores every counter phase and interrupt latch.
    pub fn restore_state(
        &mut self,
        state: TownsTimerRuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        if state
            .channels
            .iter()
            .any(|channel| channel.mode > 7 || channel.access > ACCESS_LOW_THEN_HIGH)
        {
            return Err(save_state::StateValidationError::new(
                "FM Towns timer mode is invalid",
            ));
        }
        self.state = TownsTimerState {
            channels: state.channels.map(|channel| TimerChannel {
                initial_count: channel.initial_count,
                mode: channel.mode,
                access: channel.access,
                counting: channel.counting,
                load_high: channel.load_high,
                read_high: channel.read_high,
                load_scratch: channel.load_scratch,
                last_load_cycle: channel.last_load_cycle,
            }),
            timer_out: state.timer_out,
            timer_enable: state.timer_enable,
            sound_enable: state.sound_enable,
        };
        Ok(())
    }

    /// Handles a control-word write to a timer block. `block` is 0 for channels
    /// 0-2 (port 0x0046) or 1 for channels 3-5 (port 0x0056).
    pub fn write_control(&mut self, block: usize, value: u8) {
        let channel = block * 3 + ((value >> CONTROL_CHANNEL_SHIFT) & 0x03) as usize;
        if channel >= self.state.channels.len() {
            return;
        }
        let access = (value >> CONTROL_ACCESS_SHIFT) & CONTROL_ACCESS_MASK;
        if access == ACCESS_LATCH {
            // A latch command does not change the mode or reload value.
            return;
        }
        let channel = &mut self.state.channels[channel];
        channel.access = access;
        channel.mode = (value >> CONTROL_MODE_SHIFT) & CONTROL_MODE_MASK;
        channel.counting = false;
        channel.load_high = false;
        channel.read_high = false;
    }

    /// Handles a counter write for a global channel index (0-5). Returns `true`
    /// when the channel finished (re)loading, so the caller can reschedule its
    /// interrupt event.
    pub fn write_counter(&mut self, channel: usize, value: u8, current_cycle: u64) -> bool {
        if channel >= self.state.channels.len() {
            return false;
        }
        let channel = &mut self.state.channels[channel];
        let loaded = match channel.access {
            ACCESS_LOW_ONLY => {
                channel.initial_count = u16::from(value);
                true
            }
            ACCESS_HIGH_ONLY => {
                channel.initial_count = u16::from(value) << 8;
                true
            }
            ACCESS_LOW_THEN_HIGH => {
                if !channel.load_high {
                    channel.load_scratch = u16::from(value);
                    channel.load_high = true;
                    false
                } else {
                    channel.initial_count = channel.load_scratch | (u16::from(value) << 8);
                    channel.load_high = false;
                    true
                }
            }
            _ => false,
        };
        if loaded {
            channel.counting = true;
            channel.last_load_cycle = current_cycle;
        }
        loaded
    }

    /// Reads a counter byte for a global channel index (0-5).
    pub fn read_counter(&mut self, channel: usize, current_cycle: u64, cpu_clock_hz: u32) -> u8 {
        if channel >= self.state.channels.len() {
            return 0xFF;
        }
        let count = self.current_count(channel, current_cycle, cpu_clock_hz);
        let channel = &mut self.state.channels[channel];
        match channel.access {
            ACCESS_LOW_ONLY => count as u8,
            ACCESS_HIGH_ONLY => (count >> 8) as u8,
            _ => {
                let byte = if channel.read_high {
                    (count >> 8) as u8
                } else {
                    count as u8
                };
                channel.read_high = !channel.read_high;
                byte
            }
        }
    }

    /// Handles a write to the interval-control register (0x0060).
    pub fn write_interval_control(&mut self, value: u8) {
        self.state.timer_enable[0] = value & INTERVAL_TIMER0_ENABLE != 0;
        self.state.timer_enable[1] = value & INTERVAL_TIMER1_ENABLE != 0;
        self.state.sound_enable = value & INTERVAL_SOUND_ENABLE != 0;
        if value & INTERVAL_CLEAR_TIMER0_OUT != 0 {
            self.state.timer_out[0] = false;
        }
    }

    /// Reads the interval-status register (0x0060).
    pub fn read_interval_status(&self) -> u8 {
        let mut status = 0;
        if self.state.timer_out[0] {
            status |= STATUS_TIMER0_OUT;
        }
        if self.state.timer_out[1] {
            status |= STATUS_TIMER1_OUT;
        }
        if self.state.timer_enable[0] {
            status |= STATUS_TIMER0_ENABLE;
        }
        if self.state.timer_enable[1] {
            status |= STATUS_TIMER1_ENABLE;
        }
        if self.state.sound_enable {
            status |= STATUS_SOUND_ENABLE;
        }
        status
    }

    /// Latches the OUT flag for an interrupt-capable channel (0 or 1) when its
    /// scheduled edge fires.
    pub fn latch_channel_out(&mut self, channel: usize) {
        if channel < self.state.timer_out.len() {
            self.state.timer_out[channel] = true;
        }
    }

    /// Clears channel 1's OUT flag (on any access to its counter register).
    pub fn clear_timer1_out(&mut self) {
        self.state.timer_out[1] = false;
    }

    /// Whether the timer is currently asserting IRQ 0.
    pub fn irq_active(&self) -> bool {
        (self.state.timer_out[0] && self.state.timer_enable[0])
            || (self.state.timer_out[1] && self.state.timer_enable[1])
    }

    /// Whether the SOUND (beep) output is enabled.
    pub fn sound_enabled(&self) -> bool {
        self.state.sound_enable
    }

    /// Channel 2's reload value, which sets the beep tone frequency
    /// ([`TIMER_CLOCK_HZ`] divided by this). A reload below 2 is treated as
    /// silence, matching the hardware's minimum divisor.
    pub fn beep_reload(&self) -> u16 {
        let reload = self.state.channels[CHANNEL_BEEP].initial_count;
        if reload < 2 { 0 } else { reload }
    }

    /// The period, in CPU cycles, of an interrupt-capable channel's next edge,
    /// or `None` if the channel is not producing interrupt edges. Channel 0 is
    /// periodic in mode 3; channel 1 is one-shot in mode 0.
    pub fn interrupt_period_cycles(&self, channel: usize, cpu_clock_hz: u32) -> Option<u64> {
        let state = self.state.channels.get(channel)?;
        if !state.counting {
            return None;
        }
        let periodic = match channel {
            CHANNEL_TIMER0 => state.mode == 3 || state.mode == 2,
            CHANNEL_TIMER1 => state.mode == 0,
            _ => return None,
        };
        if !periodic {
            return None;
        }
        let ticks = u64::from(effective_count(state.initial_count));
        Some(ticks * u64::from(cpu_clock_hz) / u64::from(TIMER_CLOCK_HZ))
    }

    /// Computes the current counter value for readback.
    fn current_count(&self, channel: usize, current_cycle: u64, cpu_clock_hz: u32) -> u16 {
        let state = &self.state.channels[channel];
        if !state.counting {
            return state.initial_count;
        }
        let period = u64::from(effective_count(state.initial_count));
        let elapsed_cycles = current_cycle.saturating_sub(state.last_load_cycle);
        let elapsed_ticks = elapsed_cycles * u64::from(TIMER_CLOCK_HZ) / u64::from(cpu_clock_hz);
        match state.mode {
            0 => {
                // One-shot down-count that saturates at zero.
                let remaining = period.saturating_sub(elapsed_ticks);
                remaining as u16
            }
            _ => {
                // Periodic down-count that wraps at the reload value.
                let position = elapsed_ticks % period;
                (period - position) as u16
            }
        }
    }
}

/// A programmed reload of zero means a full 65536-tick period.
fn effective_count(initial_count: u16) -> u32 {
    if initial_count == 0 {
        0x1_0000
    } else {
        u32::from(initial_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPU_CLOCK: u32 = 66_000_000;

    /// Programs channel 0 as a periodic mode-3 counter with a 16-bit reload.
    fn program_timer0(timer: &mut TownsTimer, count: u16) {
        // Control word: channel 0, low-then-high, mode 3.
        let control = (ACCESS_LOW_THEN_HIGH << CONTROL_ACCESS_SHIFT) | (3 << CONTROL_MODE_SHIFT);
        timer.write_control(0, control);
        assert!(!timer.write_counter(0, count as u8, 0));
        assert!(timer.write_counter(0, (count >> 8) as u8, 0));
    }

    #[test]
    fn timer0_period_matches_clock() {
        let mut timer = TownsTimer::new();
        program_timer0(&mut timer, 0x3000);
        let period = timer.interrupt_period_cycles(0, CPU_CLOCK).unwrap();
        assert_eq!(
            period,
            0x3000 * u64::from(CPU_CLOCK) / u64::from(TIMER_CLOCK_HZ)
        );
    }

    #[test]
    fn irq_requires_out_and_enable() {
        let mut timer = TownsTimer::new();
        program_timer0(&mut timer, 100);
        assert!(!timer.irq_active());

        // The scheduled edge latches OUT, but the interrupt is masked.
        timer.latch_channel_out(0);
        assert!(!timer.irq_active());

        // Enabling channel 0 in the interval-control register asserts IRQ 0.
        timer.write_interval_control(INTERVAL_TIMER0_ENABLE);
        assert!(timer.irq_active());

        // Clearing OUT via bit 7 deasserts it.
        timer.write_interval_control(INTERVAL_TIMER0_ENABLE | INTERVAL_CLEAR_TIMER0_OUT);
        assert!(!timer.irq_active());
    }

    #[test]
    fn interval_status_reports_flags() {
        let mut timer = TownsTimer::new();
        timer.latch_channel_out(0);
        timer.write_interval_control(INTERVAL_TIMER0_ENABLE | INTERVAL_SOUND_ENABLE);
        let status = timer.read_interval_status();
        assert_eq!(status & STATUS_TIMER0_OUT, STATUS_TIMER0_OUT);
        assert_eq!(status & STATUS_TIMER0_ENABLE, STATUS_TIMER0_ENABLE);
        assert_eq!(status & STATUS_SOUND_ENABLE, STATUS_SOUND_ENABLE);
        assert!(timer.sound_enabled());
    }

    #[test]
    fn counter_readback_counts_down() {
        let mut timer = TownsTimer::new();
        program_timer0(&mut timer, 0x8000);
        // Immediately after load, the count is the reload value.
        assert_eq!(timer.read_counter(0, 0, CPU_CLOCK), 0x00);
        assert_eq!(timer.read_counter(0, 0, CPU_CLOCK), 0x80);
        // After some cycles it has decreased by the elapsed tick count.
        let cycles = 880_000u64;
        let elapsed_ticks = cycles * u64::from(TIMER_CLOCK_HZ) / u64::from(CPU_CLOCK);
        let low = timer.read_counter(0, cycles, CPU_CLOCK);
        let high = timer.read_counter(0, cycles, CPU_CLOCK);
        let count = u16::from(low) | (u16::from(high) << 8);
        assert_eq!(count, 0x8000 - elapsed_ticks as u16);
    }

    #[test]
    fn beep_reload_tracks_channel2_and_guards_low_values() {
        let mut timer = TownsTimer::new();
        let control = (ACCESS_LOW_THEN_HIGH << CONTROL_ACCESS_SHIFT)
            | (3 << CONTROL_MODE_SHIFT)
            | (2 << CONTROL_CHANNEL_SHIFT);
        timer.write_control(0, control);
        timer.write_counter(2, 0x34, 0);
        timer.write_counter(2, 0x12, 0);
        assert_eq!(timer.beep_reload(), 0x1234);

        // A reload below 2 (including zero) is reported as silence.
        timer.write_counter(2, 0x01, 0);
        timer.write_counter(2, 0x00, 0);
        assert_eq!(timer.beep_reload(), 0);
    }

    #[test]
    fn timer1_one_shot_period() {
        let mut timer = TownsTimer::new();
        let control = (ACCESS_LOW_THEN_HIGH << CONTROL_ACCESS_SHIFT) | (0 << CONTROL_MODE_SHIFT);
        timer.write_control(0, control | (1 << CONTROL_CHANNEL_SHIFT));
        timer.write_counter(1, 0x00, 0);
        timer.write_counter(1, 0x10, 0); // count 0x1000
        assert_eq!(
            timer.interrupt_period_cycles(1, CPU_CLOCK),
            Some(0x1000 * u64::from(CPU_CLOCK) / u64::from(TIMER_CLOCK_HZ))
        );
    }
}
