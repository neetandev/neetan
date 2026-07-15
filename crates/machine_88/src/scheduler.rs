//! Event-driven scheduler for the PC-8801.

use common::{SchedulerState, StackVec};

/// Number of distinct PC-88 event kinds.
const EVENT88_KIND_COUNT: usize = 15;

/// Kinds of scheduled PC-8801 events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub(crate) enum Event88 {
    /// 600 Hz periodic CLOCK interrupt.
    #[default]
    ClockTimer,
    /// uPD3301 per-line display tick (DMA request / blanking timing).
    CrtcVline,
    /// End of the V1S/N CPU bus-request lockout during display.
    CrtcBusRequestEnd,
    /// CRTC vertical sync.
    CrtcVsync,
    /// CRTC active display start.
    CrtcDisplayStart,
    /// OPNA timer A overflow.
    FmTimerA,
    /// OPNA timer B overflow.
    FmTimerB,
    /// uPD765A execution phase complete.
    FdcPhaseComplete,
    /// uPD765A data-transfer byte due (DRQ).
    FdcDrqByte,
    /// uPD765A data lost (overrun/underrun).
    FdcDataLost,
    /// uPD765A result phase ready.
    FdcResult,
    /// uPD765A index pulse.
    FdcIndexPulse,
    /// uPD765A seek complete.
    FdcSeekComplete,
    /// uPD765A terminal-count clear.
    FdcTcClear,
    /// Fixed-tone beeper toggle.
    BeepToggle,
}

#[cfg(test)]
mod trace_identifier_tests {
    use super::*;

    #[test]
    fn trace_identifiers_match_every_event_variant() {
        assert_eq!(
            Event88::ALL.len(),
            common::trace_id::scheduled::pc88::ALL.len()
        );
        for (event, identifier) in Event88::ALL
            .iter()
            .zip(common::trace_id::scheduled::pc88::ALL)
        {
            assert_eq!(event.trace_name(), *identifier);
        }
    }
}

impl Event88 {
    pub(crate) const fn trace_name(self) -> &'static str {
        use common::trace_id::scheduled::pc88;
        match self {
            Self::ClockTimer => pc88::CLOCK_TIMER,
            Self::CrtcVline => pc88::CRTC_VLINE,
            Self::CrtcBusRequestEnd => pc88::CRTC_BUS_REQUEST_END,
            Self::CrtcVsync => pc88::CRTC_VSYNC,
            Self::CrtcDisplayStart => pc88::CRTC_DISPLAY_START,
            Self::FmTimerA => pc88::FM_TIMER_A,
            Self::FmTimerB => pc88::FM_TIMER_B,
            Self::FdcPhaseComplete => pc88::FDC_PHASE_COMPLETE,
            Self::FdcDrqByte => pc88::FDC_DRQ,
            Self::FdcDataLost => pc88::FDC_DATA_LOST,
            Self::FdcResult => pc88::FDC_RESULT,
            Self::FdcIndexPulse => pc88::FDC_INDEX,
            Self::FdcSeekComplete => pc88::FDC_SEEK_COMPLETE,
            Self::FdcTcClear => pc88::FDC_TC_CLEAR,
            Self::BeepToggle => pc88::BEEPER_TOGGLE,
        }
    }
}

impl Event88 {
    const ALL: [Event88; EVENT88_KIND_COUNT] = [
        Event88::ClockTimer,
        Event88::CrtcVline,
        Event88::CrtcBusRequestEnd,
        Event88::CrtcVsync,
        Event88::CrtcDisplayStart,
        Event88::FmTimerA,
        Event88::FmTimerB,
        Event88::FdcPhaseComplete,
        Event88::FdcDrqByte,
        Event88::FdcDataLost,
        Event88::FdcResult,
        Event88::FdcIndexPulse,
        Event88::FdcSeekComplete,
        Event88::FdcTcClear,
        Event88::BeepToggle,
    ];

    const fn from_index(index: usize) -> Self {
        Self::ALL[index]
    }
}

/// Snapshot of a single scheduled PC-88 event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScheduledEvent88 {
    /// CPU cycle at which this event fires.
    pub(crate) fire_cycle: u64,
    /// The event type.
    pub(crate) kind: Event88,
}

/// Snapshot of the scheduler's pending event queue.
///
/// Uses a flat array indexed by [`Event88`] discriminant. Each slot holds
/// `Some(fire_cycle)` when an event of that kind is scheduled, or `None`
/// when it is not. At most one event per kind can be active at a time.
type Pc8801SchedulerState = SchedulerState;

/// Event-driven scheduler for timed PC-88 peripheral events.
pub(crate) struct Pc8801Scheduler {
    /// Embedded state for save/restore.
    state: Pc8801SchedulerState,
}

impl Default for Pc8801Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Pc8801Scheduler {
    /// Creates a new empty scheduler.
    pub(crate) fn new() -> Self {
        Self {
            state: SchedulerState::new(EVENT88_KIND_COUNT),
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
        save_state::ValidateState::validate_state(&state, &EVENT88_KIND_COUNT)?;
        self.state = state;
        Ok(())
    }

    /// Schedules an event to fire at `fire_cycle`. Replaces any existing event
    /// of the same kind.
    pub(crate) fn schedule(&mut self, kind: Event88, fire_cycle: u64) {
        self.state.schedule(kind as usize, fire_cycle);
    }

    /// Cancels any pending event of the given kind.
    pub(crate) fn cancel(&mut self, kind: Event88) {
        self.state.cancel(kind as usize);
    }

    /// Returns the cycle of the earliest scheduled event, if any.
    pub(crate) fn next_event_cycle(&self) -> Option<u64> {
        self.state.next_event_cycle()
    }

    /// Removes and returns all events due at or before `current_cycle`.
    pub(crate) fn pop_due_events(
        &mut self,
        current_cycle: u64,
    ) -> StackVec<ScheduledEvent88, EVENT88_KIND_COUNT> {
        let indexes = self.state.pop_due::<EVENT88_KIND_COUNT>(current_cycle);
        let mut due = StackVec::new();
        for event in indexes.iter() {
            due.push(ScheduledEvent88 {
                fire_cycle: event.fire_cycle,
                kind: Event88::from_index(event.index),
            });
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_and_pops_in_order() {
        let mut scheduler = Pc8801Scheduler::new();
        scheduler.schedule(Event88::FmTimerA, 200);
        scheduler.schedule(Event88::ClockTimer, 100);
        assert_eq!(scheduler.next_event_cycle(), Some(100));

        let due = scheduler.pop_due_events(150);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].kind, Event88::ClockTimer);
        assert_eq!(scheduler.next_event_cycle(), Some(200));
    }

    #[test]
    fn cancel_removes_event() {
        let mut scheduler = Pc8801Scheduler::new();
        scheduler.schedule(Event88::CrtcVsync, 500);
        scheduler.cancel(Event88::CrtcVsync);
        assert_eq!(scheduler.next_event_cycle(), None);
    }
}
