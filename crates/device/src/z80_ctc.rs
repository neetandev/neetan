//! Zilog Z80 CTC (Counter/Timer Circuit).
//!
//! Four independent counter/timer channels sharing one mode-2 interrupt vector
//! base. Each channel is programmed with a control word optionally followed by a
//! time constant. In timer mode a channel counts the CTC clock down through a
//! prescaler (16 or 256) times the time constant and produces a zero-count
//! event; in counter mode it counts external trigger edges. On zero count a
//! channel with interrupts enabled raises its mode-2 vector.
//!
//! The CTC is clocked at the CPU clock, so all timer periods here are expressed
//! directly in CPU cycles. The owning machine schedules the next zero-count from
//! [`Z80Ctc::zero_cycle`] and calls [`Z80Ctc::elapse`] when it is due.

/// Number of channels in a Z80 CTC.
pub const CHANNEL_COUNT: usize = 4;

/// Control word: interrupt enable (bit 7).
const CONTROL_INTERRUPT: u8 = 0x80;
/// Control word: counter mode when set, timer mode when clear (bit 6).
const CONTROL_COUNTER_MODE: u8 = 0x40;
/// Control word: prescaler 256 when set, 16 when clear (bit 5). Timer mode only.
const CONTROL_PRESCALER_256: u8 = 0x20;
/// Control word: a time constant byte follows this control word (bit 2).
const CONTROL_TIME_CONSTANT_FOLLOWS: u8 = 0x04;
/// Control word: software reset (bit 1).
const CONTROL_RESET: u8 = 0x02;
/// Control word vs. vector selector (bit 0): 1 = control word, 0 = vector.
const CONTROL_WORD_SELECT: u8 = 0x01;

/// Prescaler divisor when the 256 bit is clear.
const PRESCALER_16: u64 = 16;
/// Prescaler divisor when the 256 bit is set.
const PRESCALER_256: u64 = 256;

save_state::runtime_state! {
/// One CTC channel.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Z80CtcChannel {
    /// Last control word written.
    mode: u8,
    /// Time constant (1..=256; a written 0 means 256).
    time_constant: u16,
    /// Whether the next data write is the time constant byte.
    expects_time_constant: bool,
    /// Whether a zero-count interrupt is latched.
    interrupt_pending: bool,
    /// Whether this channel's interrupt has been acknowledged but not yet
    /// dismissed by `RETI`.
    interrupt_in_service: bool,
    /// CPU cycle of the next zero count in timer mode, if running.
    zero_cycle: Option<u64>,
    /// Down counter value in counter mode.
    down_counter: u16,
}}

impl Z80CtcChannel {
    fn new() -> Self {
        Self {
            mode: CONTROL_RESET,
            time_constant: 0x100,
            expects_time_constant: false,
            interrupt_pending: false,
            interrupt_in_service: false,
            zero_cycle: None,
            down_counter: 0x100,
        }
    }

    fn is_counter_mode(&self) -> bool {
        (self.mode & CONTROL_COUNTER_MODE) != 0
    }

    fn interrupt_enabled(&self) -> bool {
        (self.mode & CONTROL_INTERRUPT) != 0
    }

    /// Timer period in CPU cycles (prescaler times time constant).
    fn period_cycles(&self) -> u64 {
        let prescaler = if (self.mode & CONTROL_PRESCALER_256) != 0 {
            PRESCALER_256
        } else {
            PRESCALER_16
        };
        prescaler * u64::from(self.time_constant)
    }

    fn prescaler(&self) -> u64 {
        if (self.mode & CONTROL_PRESCALER_256) != 0 {
            PRESCALER_256
        } else {
            PRESCALER_16
        }
    }
}

save_state::runtime_state! {
/// Zilog Z80 CTC device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Z80Ctc {
    channels: [Z80CtcChannel; CHANNEL_COUNT],
    /// Mode-2 interrupt vector base (masked to `0xF8`).
    vector_base: u8,
}}

impl Default for Z80Ctc {
    fn default() -> Self {
        Self::new()
    }
}

impl Z80Ctc {
    /// Creates a CTC with all channels reset.
    pub fn new() -> Self {
        Self {
            channels: [
                Z80CtcChannel::new(),
                Z80CtcChannel::new(),
                Z80CtcChannel::new(),
                Z80CtcChannel::new(),
            ],
            vector_base: 0,
        }
    }

    /// Captures all channels and daisy-chain interrupt state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores all channels and daisy-chain interrupt state.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        if state
            .channels
            .iter()
            .any(|channel| !(1..=0x100).contains(&channel.time_constant))
        {
            return Err(save_state::StateValidationError::new(
                "Z80 CTC time constant is invalid",
            ));
        }
        *self = state;
        Ok(())
    }

    /// Resets every channel and clears pending interrupts.
    pub fn reset(&mut self) {
        for channel in &mut self.channels {
            *channel = Z80CtcChannel::new();
        }
    }

    /// Writes to a channel register (control word, time constant, or - on
    /// channel 0 - the interrupt vector base). `now` is the current CPU cycle.
    pub fn write(&mut self, channel: usize, value: u8, now: u64) {
        let index = channel & 0x03;

        if self.channels[index].expects_time_constant {
            self.channels[index].expects_time_constant = false;
            self.channels[index].time_constant = if value == 0 { 0x100 } else { u16::from(value) };
            self.arm_channel(index, now);
            return;
        }

        if (value & CONTROL_WORD_SELECT) == 0 {
            // Vector write; only channel 0 latches the shared base.
            if index == 0 {
                self.vector_base = value & 0xF8;
            }
            return;
        }

        self.channels[index].mode = value;
        if (value & CONTROL_RESET) != 0 {
            self.channels[index].zero_cycle = None;
        }
        // Disabling interrupts in a control word discards a latched request.
        if (value & CONTROL_INTERRUPT) == 0 {
            self.channels[index].interrupt_pending = false;
        }
        // A control word alone never starts or re-phases the timer; counting
        // (re)starts only when a time constant is written.
        self.channels[index].expects_time_constant = (value & CONTROL_TIME_CONSTANT_FOLLOWS) != 0;
    }

    /// Arms a channel after a time-constant load: timer channels schedule the
    /// next zero count, counter channels reload the down counter.
    fn arm_channel(&mut self, index: usize, now: u64) {
        let channel = &mut self.channels[index];
        if channel.is_counter_mode() {
            channel.down_counter = channel.time_constant;
            channel.zero_cycle = None;
        } else {
            channel.zero_cycle = Some(now + channel.period_cycles());
        }
    }

    /// Reads a channel's current down-counter value. `now` is the current CPU
    /// cycle, used to derive the live count for running timer channels.
    pub fn read(&self, channel: usize, now: u64) -> u8 {
        let channel = &self.channels[channel & 0x03];
        if channel.is_counter_mode() {
            return (channel.down_counter & 0xFF) as u8;
        }
        match channel.zero_cycle {
            Some(zero_cycle) => {
                let remaining = zero_cycle.saturating_sub(now);
                let prescaler = channel.prescaler();
                let count = remaining
                    .div_ceil(prescaler)
                    .min(u64::from(channel.time_constant));
                (count & 0xFF) as u8
            }
            None => (channel.time_constant & 0xFF) as u8,
        }
    }

    /// Applies an external trigger edge to a counter-mode channel, decrementing
    /// its down counter and latching an interrupt on zero.
    pub fn trigger(&mut self, channel: usize) {
        let channel = &mut self.channels[channel & 0x03];
        if !channel.is_counter_mode() || channel.zero_cycle.is_some() {
            return;
        }
        if channel.down_counter > 0 {
            channel.down_counter -= 1;
        }
        if channel.down_counter == 0 {
            channel.down_counter = channel.time_constant;
            if channel.interrupt_enabled() {
                channel.interrupt_pending = true;
            }
        }
    }

    /// The next scheduled zero-count cycle for `channel`, if it is a running
    /// timer channel.
    pub fn zero_cycle(&self, channel: usize) -> Option<u64> {
        self.channels[channel & 0x03].zero_cycle
    }

    /// The earliest scheduled zero-count cycle across all channels.
    pub fn next_zero_cycle(&self) -> Option<u64> {
        self.channels
            .iter()
            .filter_map(|channel| channel.zero_cycle)
            .min()
    }

    /// Handles a channel's scheduled zero count: latches an interrupt when
    /// enabled and re-arms the timer for the next period.
    pub fn elapse(&mut self, channel: usize, now: u64) {
        let index = channel & 0x03;
        let channel = &mut self.channels[index];
        if channel.is_counter_mode() {
            channel.zero_cycle = None;
            return;
        }
        if channel.interrupt_enabled() {
            channel.interrupt_pending = true;
        }
        channel.zero_cycle = Some(now + channel.period_cycles());
    }

    /// The mode-2 interrupt vector for `channel` (`base | channel << 1`).
    pub fn interrupt_vector(&self, channel: usize) -> u8 {
        (self.vector_base & 0xF8) | ((channel as u8 & 0x03) << 1)
    }

    /// Whether a channel is requesting an interrupt that the internal daisy
    /// chain allows through: channels are walked in priority order (channel 0
    /// highest) and a channel under service blocks itself and every
    /// lower-priority channel, while a higher-priority channel may still nest.
    pub fn has_pending(&self) -> bool {
        for channel in &self.channels {
            if channel.interrupt_in_service {
                return false;
            }
            if channel.interrupt_pending {
                return true;
            }
        }
        false
    }

    /// Whether any channel is under service (acknowledged but not dismissed).
    pub fn has_in_service(&self) -> bool {
        self.channels
            .iter()
            .any(|channel| channel.interrupt_in_service)
    }

    /// Acknowledges the highest-priority pending interrupt the internal daisy
    /// chain allows through (channel 0 first), clearing it, marking the
    /// channel as under service and returning its mode-2 vector.
    pub fn acknowledge(&mut self) -> u8 {
        for index in 0..CHANNEL_COUNT {
            if self.channels[index].interrupt_in_service {
                break;
            }
            if self.channels[index].interrupt_pending {
                self.channels[index].interrupt_pending = false;
                self.channels[index].interrupt_in_service = true;
                return self.interrupt_vector(index);
            }
        }
        self.vector_base & 0xF8
    }

    /// Dismisses the channel under service, as an executed `RETI` does. A zero
    /// count that occurred while the channel was under service is discarded:
    /// a channel does not queue a second interrupt behind its own handler.
    pub fn notify_reti(&mut self) {
        for channel in &mut self.channels {
            if channel.interrupt_in_service {
                channel.interrupt_in_service = false;
                channel.interrupt_pending = false;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program_timer(ctc: &mut Z80Ctc, channel: usize, prescaler_256: bool, tconst: u8, now: u64) {
        let mut control = CONTROL_WORD_SELECT | CONTROL_INTERRUPT | CONTROL_TIME_CONSTANT_FOLLOWS;
        if prescaler_256 {
            control |= CONTROL_PRESCALER_256;
        }
        ctc.write(channel, control, now);
        ctc.write(channel, tconst, now);
    }

    #[test]
    fn vector_base_is_shared_and_offset_per_channel() {
        let mut ctc = Z80Ctc::new();
        ctc.write(0, 0xA0, 0); // vector write (bit0 clear), base = 0xA0
        assert_eq!(ctc.interrupt_vector(0), 0xA0);
        assert_eq!(ctc.interrupt_vector(1), 0xA2);
        assert_eq!(ctc.interrupt_vector(3), 0xA6);
    }

    #[test]
    fn timer_schedules_and_reloads_zero_count() {
        let mut ctc = Z80Ctc::new();
        ctc.write(0, 0x00, 0); // set base 0x00
        program_timer(&mut ctc, 0, false, 10, 0);
        // Prescaler 16 * time constant 10 = 160 cycles.
        assert_eq!(ctc.zero_cycle(0), Some(160));
        assert!(!ctc.has_pending());

        ctc.elapse(0, 160);
        assert!(ctc.has_pending());
        assert_eq!(ctc.zero_cycle(0), Some(320));
        assert_eq!(ctc.acknowledge(), 0x00);
        assert!(!ctc.has_pending());
    }

    #[test]
    fn time_constant_zero_means_256() {
        let mut ctc = Z80Ctc::new();
        program_timer(&mut ctc, 1, true, 0, 0);
        // Prescaler 256 * 256 = 65536 cycles.
        assert_eq!(ctc.zero_cycle(1), Some(65536));
    }

    #[test]
    fn read_returns_the_live_down_counter() {
        let mut ctc = Z80Ctc::new();
        program_timer(&mut ctc, 0, false, 10, 0);
        assert_eq!(ctc.read(0, 0), 10);
        // Halfway through: 80 cycles elapsed of 160, 5 counts left.
        assert_eq!(ctc.read(0, 80), 5);
    }

    #[test]
    fn counter_mode_decrements_on_triggers() {
        let mut ctc = Z80Ctc::new();
        let control = CONTROL_WORD_SELECT
            | CONTROL_INTERRUPT
            | CONTROL_COUNTER_MODE
            | CONTROL_TIME_CONSTANT_FOLLOWS;
        ctc.write(2, control, 0);
        ctc.write(2, 2, 0);
        assert_eq!(ctc.zero_cycle(2), None);
        ctc.trigger(2);
        assert!(!ctc.has_pending());
        ctc.trigger(2);
        assert!(ctc.has_pending());
        assert_eq!(ctc.acknowledge(), 0x04);
    }

    #[test]
    fn control_word_with_interrupts_off_discards_a_pending_request() {
        let mut ctc = Z80Ctc::new();
        program_timer(&mut ctc, 1, false, 10, 0);
        ctc.elapse(1, 160);
        assert!(ctc.has_pending());
        // Stop the channel with interrupts disabled (reset + control select).
        ctc.write(1, CONTROL_WORD_SELECT | CONTROL_RESET, 200);
        assert!(
            !ctc.has_pending(),
            "stopping the channel must drop the latched request"
        );
    }

    #[test]
    fn control_word_without_time_constant_does_not_start_or_rephase() {
        let mut ctc = Z80Ctc::new();
        program_timer(&mut ctc, 0, false, 10, 0);
        assert_eq!(ctc.zero_cycle(0), Some(160));
        // A plain mode change must not re-phase the running timer.
        ctc.write(0, CONTROL_WORD_SELECT | CONTROL_INTERRUPT, 100);
        assert_eq!(ctc.zero_cycle(0), Some(160));

        // A stopped channel stays stopped until a time constant is written.
        ctc.write(0, CONTROL_WORD_SELECT | CONTROL_RESET, 200);
        assert_eq!(ctc.zero_cycle(0), None);
        ctc.write(0, CONTROL_WORD_SELECT | CONTROL_INTERRUPT, 300);
        assert_eq!(ctc.zero_cycle(0), None);
    }

    #[test]
    fn reset_stops_a_running_timer() {
        let mut ctc = Z80Ctc::new();
        program_timer(&mut ctc, 0, false, 10, 0);
        assert!(ctc.zero_cycle(0).is_some());
        ctc.write(0, CONTROL_WORD_SELECT | CONTROL_RESET, 0);
        assert_eq!(ctc.zero_cycle(0), None);
    }

    #[test]
    fn reti_discards_a_zero_count_latched_during_service() {
        // A channel whose handler runs longer than its own period must not
        // re-interrupt back-to-back after RETI: zero counts that occur while
        // the channel is under service are discarded, and the next interrupt
        // needs a fresh zero count.
        let mut ctc = Z80Ctc::new();
        program_timer(&mut ctc, 1, true, 16, 0);
        ctc.elapse(1, 4096);
        assert!(ctc.has_pending());
        ctc.acknowledge();
        assert!(!ctc.has_pending());

        // The timer keeps running and hits zero again while under service;
        // the internal chain hides the request from the CPU.
        ctc.elapse(1, 8192);
        assert!(!ctc.has_pending());

        // RETI discards it rather than letting it re-fire back-to-back.
        ctc.notify_reti();
        assert!(!ctc.has_pending());

        // A zero count after RETI interrupts normally again.
        ctc.elapse(1, 12288);
        assert!(ctc.has_pending());
    }

    #[test]
    fn reti_only_dismisses_the_serviced_channel() {
        let mut ctc = Z80Ctc::new();
        program_timer(&mut ctc, 0, false, 10, 0);
        program_timer(&mut ctc, 1, false, 10, 0);
        ctc.elapse(0, 160);
        assert_eq!(ctc.acknowledge(), 0x00);

        // Channel 1 becomes pending while channel 0 is under service.
        ctc.elapse(1, 160);
        ctc.notify_reti();
        assert!(ctc.has_pending(), "channel 1's request must survive RETI");
        assert_eq!(ctc.acknowledge(), 0x02);
    }

    #[test]
    fn a_higher_priority_channel_nests_over_a_lower_in_service_channel() {
        let mut ctc = Z80Ctc::new();
        program_timer(&mut ctc, 0, false, 10, 0);
        program_timer(&mut ctc, 2, false, 10, 0);

        // Channel 2 fires first and its handler is entered.
        ctc.elapse(2, 160);
        assert_eq!(ctc.acknowledge(), 0x04);
        assert!(!ctc.has_pending());

        // Channel 0 outranks the channel under service and may nest.
        ctc.elapse(0, 200);
        assert!(ctc.has_pending());
        assert_eq!(ctc.acknowledge(), 0x00);
        assert!(!ctc.has_pending());

        // RETI dismisses the nested channel 0 handler first, then channel 2.
        ctc.notify_reti();
        assert!(ctc.has_in_service());
        ctc.notify_reti();
        assert!(!ctc.has_in_service());
    }

    #[test]
    fn a_lower_priority_channel_stays_blocked_while_a_higher_one_is_in_service() {
        let mut ctc = Z80Ctc::new();
        program_timer(&mut ctc, 0, false, 10, 0);
        program_timer(&mut ctc, 2, false, 10, 0);

        ctc.elapse(0, 160);
        assert_eq!(ctc.acknowledge(), 0x00);

        // Channel 2 requests while channel 0 is under service: blocked.
        ctc.elapse(2, 200);
        assert!(!ctc.has_pending());
        assert_eq!(
            ctc.acknowledge(),
            0x00,
            "invalid ack returns the bare vector base"
        );

        // Channel 0's RETI releases the chain and channel 2 gets through.
        ctc.notify_reti();
        assert!(ctc.has_pending());
        assert_eq!(ctc.acknowledge(), 0x04);
    }

    #[test]
    fn reti_without_a_serviced_channel_is_a_no_op() {
        let mut ctc = Z80Ctc::new();
        program_timer(&mut ctc, 0, false, 10, 0);
        ctc.elapse(0, 160);
        ctc.notify_reti();
        assert!(ctc.has_pending(), "an unacknowledged request is kept");
    }
}
