//! Event-driven scheduler for the PC-88VA.

use common::StackVec;

/// Number of distinct PC-88VA2 event kinds.
pub const EVENT_VA_KIND_COUNT: usize = 11;

/// Kinds of scheduled PC-88VA2 events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum EventVA {
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

impl EventVA {
    const ALL: [EventVA; EVENT_VA_KIND_COUNT] = [
        EventVA::PitTimer0,
        EventVA::TspFrame,
        EventVA::Sysp4Vsync,
        EventVA::SgpComplete,
        EventVA::FdcDrqByte,
        EventVA::FdcSeekComplete,
        EventVA::FdcResultComplete,
        EventVA::FdcTcClear,
        EventVA::OpnaTimerA,
        EventVA::OpnaTimerB,
        EventVA::Timer3,
    ];

    const fn from_index(index: usize) -> Self {
        Self::ALL[index]
    }
}

/// Snapshot of a single scheduled PC-88VA2 event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledEventVA {
    /// CPU cycle at which this event fires.
    pub fire_cycle: u64,
    /// The event type.
    pub kind: EventVA,
}

/// Snapshot of the scheduler's pending event queue.
///
/// Uses a flat array indexed by [`EventVA`] discriminant. Each slot holds
/// `Some(fire_cycle)` when an event of that kind is scheduled, or `None`
/// when it is not. At most one event per kind can be active at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pc88VaSchedulerState {
    /// Fire cycle for each event kind, indexed by discriminant.
    pub fire_cycles: [Option<u64>; EVENT_VA_KIND_COUNT],
}

/// Event-driven scheduler for timed PC-88VA2 peripheral events.
pub struct Pc88VaScheduler {
    /// Embedded state for save/restore.
    pub state: Pc88VaSchedulerState,
}

impl Default for Pc88VaScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Pc88VaScheduler {
    /// Creates a new empty scheduler.
    pub fn new() -> Self {
        Self {
            state: Pc88VaSchedulerState {
                fire_cycles: [None; EVENT_VA_KIND_COUNT],
            },
        }
    }

    /// Schedules an event to fire at `fire_cycle`. Replaces any existing event
    /// of the same kind.
    pub fn schedule(&mut self, kind: EventVA, fire_cycle: u64) {
        self.state.fire_cycles[kind as usize] = Some(fire_cycle);
    }

    /// Cancels any scheduled event of the given kind.
    pub fn cancel(&mut self, kind: EventVA) {
        self.state.fire_cycles[kind as usize] = None;
    }

    /// Returns the cycle of the earliest scheduled event, if any.
    pub fn next_event_cycle(&self) -> Option<u64> {
        self.state.fire_cycles.iter().filter_map(|&c| c).min()
    }

    /// Removes and returns all events due at or before `current_cycle`.
    pub fn pop_due_events(
        &mut self,
        current_cycle: u64,
    ) -> StackVec<ScheduledEventVA, EVENT_VA_KIND_COUNT> {
        let mut due = StackVec::new();
        for (index, slot) in self.state.fire_cycles.iter_mut().enumerate() {
            if let Some(fire_cycle) = *slot
                && fire_cycle <= current_cycle
            {
                due.push(ScheduledEventVA {
                    fire_cycle,
                    kind: EventVA::from_index(index),
                });
                *slot = None;
            }
        }
        due.sort_by_key(|event: &ScheduledEventVA| event.fire_cycle);
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_and_pops_in_order() {
        let mut scheduler = Pc88VaScheduler::new();
        scheduler.schedule(EventVA::PitTimer0, 100);
        assert_eq!(scheduler.next_event_cycle(), Some(100));

        let due = scheduler.pop_due_events(50);
        assert_eq!(due.len(), 0);

        let due = scheduler.pop_due_events(150);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].kind, EventVA::PitTimer0);
        assert_eq!(scheduler.next_event_cycle(), None);
    }
}
