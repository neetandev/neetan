//! Event-driven scheduler for the PC-88VA.

use common::{SchedulerState, StackVec};

/// Number of distinct PC-88VA2 event kinds.
const EVENT88VA_KIND_COUNT: usize = 11;

/// Kinds of scheduled PC-88VA2 events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub(crate) enum Event88Va {
    /// 8253 timer 0 terminal count (raises master IRQ0).
    #[default]
    PitTimer0,
    /// TSP frame loop: alternates the display and VSYNC phases of the screen.
    TspFrame,
    /// System-port-4 VSYNC window and the once-per-frame VSYNC IRQ.
    Sysp4Vsync,
    /// SGP blitter run completion (clears busy, raises the SGP IRQ).
    SgpComplete,
    /// Floppy FDC PIO data-rate byte slot (releases the next DRQ byte).
    FdcDrqByte,
    /// Floppy FDC seek/recalibrate completion (raises the sub-CPU interrupt).
    FdcSeekComplete,
    /// Floppy FDC DMA data-command completion (raises the main-CPU IRQ 11).
    FdcResultComplete,
    /// Floppy FDC terminal-count pulse deassert.
    FdcTcClear,
    /// YM2608 (OPNA) timer A expiry.
    OpnaTimerA,
    /// YM2608 (OPNA) timer B expiry.
    OpnaTimerB,
    /// General-purpose timer 3 (TCU) tick, raising slave IRQ 13 (vector 0x15).
    Timer3,
}

#[cfg(test)]
mod trace_identifier_tests {
    use super::*;

    #[test]
    fn trace_identifiers_match_every_event_variant() {
        assert_eq!(
            Event88Va::ALL.len(),
            common::trace_id::scheduled::pc88va::ALL.len()
        );
        for (event, identifier) in Event88Va::ALL
            .iter()
            .zip(common::trace_id::scheduled::pc88va::ALL)
        {
            assert_eq!(event.trace_name(), *identifier);
        }
    }
}

impl Event88Va {
    pub(crate) const fn trace_name(self) -> &'static str {
        use common::trace_id::scheduled::pc88va;
        match self {
            Self::PitTimer0 => pc88va::PIT_TIMER0,
            Self::TspFrame => pc88va::TSP_FRAME,
            Self::Sysp4Vsync => pc88va::SYSTEM_VSYNC,
            Self::SgpComplete => pc88va::SGP_COMPLETE,
            Self::FdcDrqByte => pc88va::FDC_DRQ,
            Self::FdcSeekComplete => pc88va::FDC_SEEK_COMPLETE,
            Self::FdcResultComplete => pc88va::FDC_RESULT_COMPLETE,
            Self::FdcTcClear => pc88va::FDC_TC_CLEAR,
            Self::OpnaTimerA => pc88va::OPNA_TIMER_A,
            Self::OpnaTimerB => pc88va::OPNA_TIMER_B,
            Self::Timer3 => pc88va::TIMER3,
        }
    }
}

impl Event88Va {
    const ALL: [Event88Va; EVENT88VA_KIND_COUNT] = [
        Event88Va::PitTimer0,
        Event88Va::TspFrame,
        Event88Va::Sysp4Vsync,
        Event88Va::SgpComplete,
        Event88Va::FdcDrqByte,
        Event88Va::FdcSeekComplete,
        Event88Va::FdcResultComplete,
        Event88Va::FdcTcClear,
        Event88Va::OpnaTimerA,
        Event88Va::OpnaTimerB,
        Event88Va::Timer3,
    ];

    const fn from_index(index: usize) -> Self {
        Self::ALL[index]
    }
}

/// Snapshot of a single scheduled PC-88VA2 event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScheduledEvent88Va {
    /// CPU cycle at which this event fires.
    pub(crate) fire_cycle: u64,
    /// The event type.
    pub(crate) kind: Event88Va,
}

/// Snapshot of the scheduler's pending event queue.
///
/// Uses a flat array indexed by [`Event88Va`] discriminant. Each slot holds
/// `Some(fire_cycle)` when an event of that kind is scheduled, or `None`
/// when it is not. At most one event per kind can be active at a time.
pub(crate) type Pc88VaSchedulerState = SchedulerState;

/// Event-driven scheduler for timed PC-88VA2 peripheral events.
pub(crate) struct Pc88VaScheduler {
    /// Embedded state for save/restore.
    pub(crate) state: Pc88VaSchedulerState,
}

impl Default for Pc88VaScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Pc88VaScheduler {
    /// Creates a new empty scheduler.
    pub(crate) fn new() -> Self {
        Self {
            state: SchedulerState::new(EVENT88VA_KIND_COUNT),
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
        save_state::ValidateState::validate_state(&state, &EVENT88VA_KIND_COUNT)?;
        self.state = state;
        Ok(())
    }

    /// Schedules an event to fire at `fire_cycle`. Replaces any existing event
    /// of the same kind.
    pub(crate) fn schedule(&mut self, kind: Event88Va, fire_cycle: u64) {
        self.state.schedule(kind as usize, fire_cycle);
    }

    /// Cancels any scheduled event of the given kind.
    pub(crate) fn cancel(&mut self, kind: Event88Va) {
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
    ) -> StackVec<ScheduledEvent88Va, EVENT88VA_KIND_COUNT> {
        let indexes = self.state.pop_due::<EVENT88VA_KIND_COUNT>(current_cycle);
        let mut due = StackVec::new();
        for event in indexes.iter() {
            due.push(ScheduledEvent88Va {
                fire_cycle: event.fire_cycle,
                kind: Event88Va::from_index(event.index),
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
        let mut scheduler = Pc88VaScheduler::new();
        scheduler.schedule(Event88Va::PitTimer0, 100);
        assert_eq!(scheduler.next_event_cycle(), Some(100));

        let due = scheduler.pop_due_events(50);
        assert_eq!(due.len(), 0);

        let due = scheduler.pop_due_events(150);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].kind, Event88Va::PitTimer0);
        assert_eq!(scheduler.next_event_cycle(), None);
    }
}
