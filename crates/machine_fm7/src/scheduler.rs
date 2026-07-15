//! Event scheduler.
//!
//! A single monotonic cycle counter (in main-clock units) lives on the bus; this
//! scheduler tracks the fire cycle of each event kind in a flat array.

use common::{SchedulerState, StackVec};

/// Kinds of scheduled events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum EventFm7 {
    /// Start of vertical blanking; presents the frame.
    VBlank,
    /// Start of a display scanline; latches the line.
    Scanline,
    /// Main CPU periodic timer interrupt.
    TimerIrq,
    /// Sub CPU display NMI.
    SubDisplayNmi,
    /// FDC motor spin-up completion.
    FdcMotorOn,
    /// FDC motor spin-down completion.
    FdcMotorOff,
    /// FDC seek / command completion.
    FdcSeekComplete,
    /// Beeper one-shot gate off.
    BeepOneShotOff,
    /// Delayed set of the sub busy flag after a CLR 0xD40A read-modify-write.
    SubBusyClearDelay,
    /// Auto-disarm of the pending CLR 0xD40A read-modify-write window.
    SubBusyDelayDisarm,
    /// Keyboard latch tick: drips one pending keycode into the read latch.
    KeyboardLatch,
    /// Keyboard auto-repeat tick.
    KeyboardRepeat,
    /// Keyboard encoder ACK handshake (AV).
    EncoderAck,
    /// MB61VH010 ALU busy-clear (AV).
    AluBusyClear,
    /// YM2203 (OPN) timer A expiry (AV).
    OpnTimerA,
    /// YM2203 (OPN) timer B expiry (AV).
    OpnTimerB,
    /// FM-77AV keyboard-encoder RTC one-second tick.
    RtcSecond,
    /// Joystick-port mouse nibble-sequence timeout.
    MouseTimeout,
}

#[cfg(test)]
mod trace_identifier_tests {
    use super::*;

    #[test]
    fn trace_identifiers_match_every_event_variant() {
        assert_eq!(
            EventFm7::ALL.len(),
            common::trace_id::scheduled::fm7::ALL.len()
        );
        for (event, identifier) in EventFm7::ALL
            .iter()
            .zip(common::trace_id::scheduled::fm7::ALL)
        {
            assert_eq!(event.trace_name(), *identifier);
        }
    }
}

impl EventFm7 {
    pub(crate) const fn trace_name(self) -> &'static str {
        use common::trace_id::scheduled::fm7;
        match self {
            Self::VBlank => fm7::VIDEO_VBLANK,
            Self::Scanline => fm7::VIDEO_SCANLINE,
            Self::TimerIrq => fm7::TIMER_IRQ,
            Self::SubDisplayNmi => fm7::SUB_DISPLAY_NMI,
            Self::FdcMotorOn => fm7::FDC_MOTOR_ON,
            Self::FdcMotorOff => fm7::FDC_MOTOR_OFF,
            Self::FdcSeekComplete => fm7::FDC_SEEK_COMPLETE,
            Self::BeepOneShotOff => fm7::BEEPER_ONE_SHOT_OFF,
            Self::SubBusyClearDelay => fm7::SUB_BUSY_CLEAR,
            Self::SubBusyDelayDisarm => fm7::SUB_BUSY_DISARM,
            Self::KeyboardLatch => fm7::KEYBOARD_LATCH,
            Self::KeyboardRepeat => fm7::KEYBOARD_REPEAT,
            Self::EncoderAck => fm7::KEYBOARD_ENCODER_ACK,
            Self::AluBusyClear => fm7::ALU_BUSY_CLEAR,
            Self::OpnTimerA => fm7::OPN_TIMER_A,
            Self::OpnTimerB => fm7::OPN_TIMER_B,
            Self::RtcSecond => fm7::RTC_SECOND,
            Self::MouseTimeout => fm7::MOUSE_TIMEOUT,
        }
    }
}

/// Number of distinct FM-7 scheduler event kinds.
const EVENT_COUNT: usize = 18;

impl EventFm7 {
    /// Event kinds indexed by their discriminant.
    const ALL: [EventFm7; EVENT_COUNT] = [
        EventFm7::VBlank,
        EventFm7::Scanline,
        EventFm7::TimerIrq,
        EventFm7::SubDisplayNmi,
        EventFm7::FdcMotorOn,
        EventFm7::FdcMotorOff,
        EventFm7::FdcSeekComplete,
        EventFm7::BeepOneShotOff,
        EventFm7::SubBusyClearDelay,
        EventFm7::SubBusyDelayDisarm,
        EventFm7::KeyboardLatch,
        EventFm7::KeyboardRepeat,
        EventFm7::EncoderAck,
        EventFm7::AluBusyClear,
        EventFm7::OpnTimerA,
        EventFm7::OpnTimerB,
        EventFm7::RtcSecond,
        EventFm7::MouseTimeout,
    ];

    /// The event's slot index in the scheduler's flat array.
    const fn index(self) -> usize {
        self as usize
    }

    /// The event kind stored at the given flat-array slot index.
    const fn from_index(index: usize) -> Self {
        Self::ALL[index]
    }
}

/// A scheduled event together with the cycle it fires at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScheduledEventFm7 {
    /// Cycle at which the event fires.
    pub(crate) fire_cycle: u64,
    /// The event kind.
    pub(crate) kind: EventFm7,
}

/// Tracks the next fire cycle of each event kind.
pub(crate) struct Fm7Scheduler {
    state: SchedulerState,
}

impl Fm7Scheduler {
    /// Creates a scheduler with no events pending.
    pub(crate) fn new() -> Self {
        Self {
            state: SchedulerState::new(EVENT_COUNT),
        }
    }

    /// Captures all pending events.
    pub(crate) fn capture_state(&self) -> SchedulerState {
        self.state.clone()
    }

    /// Restores all pending events.
    pub(crate) fn restore_state(
        &mut self,
        state: SchedulerState,
    ) -> Result<(), save_state::StateValidationError> {
        save_state::ValidateState::validate_state(&state, &EVENT_COUNT)?;
        self.state = state;
        Ok(())
    }

    /// Schedules `kind` to fire at `fire_cycle`, replacing any prior schedule.
    pub(crate) fn schedule(&mut self, kind: EventFm7, fire_cycle: u64) {
        self.state.schedule(kind.index(), fire_cycle);
    }

    /// Cancels any pending schedule for `kind`.
    pub(crate) fn cancel(&mut self, kind: EventFm7) {
        self.state.cancel(kind.index());
    }

    /// Returns the earliest scheduled fire cycle, if any event is pending.
    pub(crate) fn next_event_cycle(&self) -> Option<u64> {
        self.state.next_event_cycle()
    }

    /// Removes and returns all events due at or before `current_cycle`, ordered
    /// by fire cycle.
    pub(crate) fn pop_due_events(
        &mut self,
        current_cycle: u64,
    ) -> StackVec<ScheduledEventFm7, EVENT_COUNT> {
        let indexes = self.state.pop_due::<EVENT_COUNT>(current_cycle);
        let mut due = StackVec::new();
        for event in indexes.iter() {
            due.push(ScheduledEventFm7 {
                fire_cycle: event.fire_cycle,
                kind: EventFm7::from_index(event.index),
            });
        }
        due
    }
}

impl Default for Fm7Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_and_pops_in_fire_order() {
        let mut scheduler = Fm7Scheduler::new();
        scheduler.schedule(EventFm7::Scanline, 100);
        scheduler.schedule(EventFm7::VBlank, 50);

        assert_eq!(scheduler.next_event_cycle(), Some(50));

        let due = scheduler.pop_due_events(100);
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].kind, EventFm7::VBlank);
        assert_eq!(due[1].kind, EventFm7::Scanline);
        assert_eq!(scheduler.next_event_cycle(), None);
    }

    #[test]
    fn cancel_drops_a_pending_event() {
        let mut scheduler = Fm7Scheduler::new();
        scheduler.schedule(EventFm7::TimerIrq, 200);
        scheduler.cancel(EventFm7::TimerIrq);
        assert_eq!(scheduler.next_event_cycle(), None);
    }

    #[test]
    fn pop_leaves_future_events_scheduled() {
        let mut scheduler = Fm7Scheduler::new();
        scheduler.schedule(EventFm7::SubDisplayNmi, 300);
        scheduler.schedule(EventFm7::Scanline, 100);

        let due = scheduler.pop_due_events(150);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].kind, EventFm7::Scanline);
        assert_eq!(scheduler.next_event_cycle(), Some(300));
    }

    #[test]
    fn rescheduling_replaces_the_prior_fire_cycle() {
        let mut scheduler = Fm7Scheduler::new();
        scheduler.schedule(EventFm7::Scanline, 100);
        scheduler.schedule(EventFm7::Scanline, 40);
        assert_eq!(scheduler.next_event_cycle(), Some(40));
    }
}
