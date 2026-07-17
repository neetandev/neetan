//! Event scheduler for the MSX machine.

use common::{SchedulerState, StackVec};

/// Scheduled MSX event kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum EventMsx {
    /// HBlank latch point of the next physical video scanline.
    Scanline,
    /// End of the active display.
    VBlank,
    /// Programmable V9938 scanline interrupt.
    LineInterrupt,
    /// WD2793 command task.
    FdcTask,
    /// WD2793 PIO request or lost-data deadline.
    FdcPio,
    /// Y8950 timer A.
    Y8950TimerA,
    /// Y8950 timer B.
    Y8950TimerB,
}

impl EventMsx {
    /// Number of scheduled MSX event kinds.
    const EVENT_COUNT: usize = 7;
    /// Scheduled MSX event kinds in slot order.
    const ALL: [Self; Self::EVENT_COUNT] = [
        Self::Scanline,
        Self::VBlank,
        Self::LineInterrupt,
        Self::FdcTask,
        Self::FdcPio,
        Self::Y8950TimerA,
        Self::Y8950TimerB,
    ];

    /// Stable trace identifier for this event.
    pub(crate) const fn trace_name(self) -> &'static str {
        match self {
            Self::Scanline => common::trace_id::scheduled::msx::VIDEO_SCANLINE,
            Self::VBlank => common::trace_id::scheduled::msx::VIDEO_VBLANK,
            Self::LineInterrupt => common::trace_id::scheduled::msx::VIDEO_LINE_INTERRUPT,
            Self::FdcTask => common::trace_id::scheduled::msx::FDC_TASK,
            Self::FdcPio => common::trace_id::scheduled::msx::FDC_PIO,
            Self::Y8950TimerA => "msx.y8950.timer_a",
            Self::Y8950TimerB => "msx.y8950.timer_b",
        }
    }

    /// Scheduler slot index for this event.
    const fn index(self) -> usize {
        self as usize
    }

    /// Converts a scheduler slot back to its event kind.
    const fn from_index(index: usize) -> Self {
        Self::ALL[index]
    }
}

/// One due MSX event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScheduledEventMsx {
    pub(crate) fire_cycle: u64,
    pub(crate) kind: EventMsx,
}

/// Tracks the next fire cycle of each MSX event.
pub(crate) struct MsxScheduler {
    state: SchedulerState,
}

impl MsxScheduler {
    /// Creates a scheduler with no pending events.
    pub(crate) fn new() -> Self {
        Self {
            state: SchedulerState::new(EventMsx::ALL.len()),
        }
    }

    /// Schedules an event, replacing its prior schedule.
    pub(crate) fn schedule(&mut self, kind: EventMsx, fire_cycle: u64) {
        self.state.schedule(kind.index(), fire_cycle);
    }

    /// Cancels the pending event of one kind.
    pub(crate) fn cancel(&mut self, kind: EventMsx) {
        self.state.cancel(kind.index());
    }

    /// Returns the earliest scheduled cycle.
    pub(crate) fn next_event_cycle(&self) -> Option<u64> {
        self.state.next_event_cycle()
    }

    /// Removes all events due at or before `current_cycle`.
    pub(crate) fn pop_due_events(
        &mut self,
        current_cycle: u64,
    ) -> StackVec<ScheduledEventMsx, { EventMsx::EVENT_COUNT }> {
        let indexes = self
            .state
            .pop_due::<{ EventMsx::EVENT_COUNT }>(current_cycle);
        let mut due = StackVec::new();
        for event in indexes.iter() {
            due.push(ScheduledEventMsx {
                fire_cycle: event.fire_cycle,
                kind: EventMsx::from_index(event.index),
            });
        }
        due
    }

    /// Captures every scheduled event slot.
    /// Captures every event slot.
    pub(crate) fn capture_state(&self) -> SchedulerState {
        self.state.clone()
    }

    /// Restores every scheduled event slot.
    /// Restores every event slot after count validation.
    pub(crate) fn restore_state(
        &mut self,
        state: SchedulerState,
    ) -> Result<(), save_state::StateValidationError> {
        save_state::ValidateState::validate_state(&state, &EventMsx::EVENT_COUNT)?;
        self.state = state;
        Ok(())
    }
}

impl Default for MsxScheduler {
    /// Creates a scheduler with no pending events.
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Rescheduling one kind replaces its prior cycle.
    fn scanline_event_reschedules_by_slot() {
        let mut scheduler = MsxScheduler::new();
        scheduler.schedule(EventMsx::Scanline, 228);
        scheduler.schedule(EventMsx::Scanline, 456);
        assert_eq!(scheduler.next_event_cycle(), Some(456));
        let due = scheduler.pop_due_events(456);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].kind, EventMsx::Scanline);
    }

    #[test]
    /// Scanline and VBlank events retain independent scheduler slots.
    fn scanline_and_vblank_are_independent_slots() {
        let mut scheduler = MsxScheduler::new();
        scheduler.schedule(EventMsx::Scanline, 100);
        scheduler.schedule(EventMsx::VBlank, 50);
        let due = scheduler.pop_due_events(100);
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].kind, EventMsx::VBlank);
        assert_eq!(due[1].kind, EventMsx::Scanline);
    }
}
