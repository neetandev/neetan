//! Event-driven scheduler for the PC-88VA.

use common::StackVec;

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pc88VaSchedulerState {
    /// Fire cycle for each event kind, indexed by discriminant.
    pub(crate) fire_cycles: [Option<u64>; EVENT88VA_KIND_COUNT],
}

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
            state: Pc88VaSchedulerState {
                fire_cycles: [None; EVENT88VA_KIND_COUNT],
            },
        }
    }

    /// Schedules an event to fire at `fire_cycle`. Replaces any existing event
    /// of the same kind.
    pub(crate) fn schedule(&mut self, kind: Event88Va, fire_cycle: u64) {
        self.state.fire_cycles[kind as usize] = Some(fire_cycle);
    }

    /// Cancels any scheduled event of the given kind.
    pub(crate) fn cancel(&mut self, kind: Event88Va) {
        self.state.fire_cycles[kind as usize] = None;
    }

    /// Returns the cycle of the earliest scheduled event, if any.
    pub(crate) fn next_event_cycle(&self) -> Option<u64> {
        self.state.fire_cycles.iter().filter_map(|&c| c).min()
    }

    /// Removes and returns all events due at or before `current_cycle`.
    pub(crate) fn pop_due_events(
        &mut self,
        current_cycle: u64,
    ) -> StackVec<ScheduledEvent88Va, EVENT88VA_KIND_COUNT> {
        let mut due = StackVec::new();
        for (index, slot) in self.state.fire_cycles.iter_mut().enumerate() {
            if let Some(fire_cycle) = *slot
                && fire_cycle <= current_cycle
            {
                due.push(ScheduledEvent88Va {
                    fire_cycle,
                    kind: Event88Va::from_index(index),
                });
                *slot = None;
            }
        }
        due.sort_by_key(|event: &ScheduledEvent88Va| event.fire_cycle);
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
