//! Event-driven scheduler for the PC-8801.

use common::StackVec;

/// Number of distinct PC-88 event kinds.
pub const EVENT88_KIND_COUNT: usize = 15;

/// Kinds of scheduled PC-8801 events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Event88 {
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
pub struct ScheduledEvent88 {
    /// CPU cycle at which this event fires.
    pub fire_cycle: u64,
    /// The event type.
    pub kind: Event88,
}

/// Snapshot of the scheduler's pending event queue.
///
/// Uses a flat array indexed by [`Event88`] discriminant. Each slot holds
/// `Some(fire_cycle)` when an event of that kind is scheduled, or `None`
/// when it is not. At most one event per kind can be active at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pc8801SchedulerState {
    /// Fire cycle for each event kind, indexed by discriminant.
    pub fire_cycles: [Option<u64>; EVENT88_KIND_COUNT],
}

/// Event-driven scheduler for timed PC-88 peripheral events.
pub struct Pc8801Scheduler {
    /// Embedded state for save/restore.
    pub state: Pc8801SchedulerState,
}

impl Default for Pc8801Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Pc8801Scheduler {
    /// Creates a new empty scheduler.
    pub fn new() -> Self {
        Self {
            state: Pc8801SchedulerState {
                fire_cycles: [None; EVENT88_KIND_COUNT],
            },
        }
    }

    /// Schedules an event to fire at `fire_cycle`. Replaces any existing event
    /// of the same kind.
    pub fn schedule(&mut self, kind: Event88, fire_cycle: u64) {
        self.state.fire_cycles[kind as usize] = Some(fire_cycle);
    }

    /// Cancels any pending event of the given kind.
    pub fn cancel(&mut self, kind: Event88) {
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
    ) -> StackVec<ScheduledEvent88, EVENT88_KIND_COUNT> {
        let mut due = StackVec::new();
        for (index, slot) in self.state.fire_cycles.iter_mut().enumerate() {
            if let Some(fire_cycle) = *slot
                && fire_cycle <= current_cycle
            {
                due.push(ScheduledEvent88 {
                    fire_cycle,
                    kind: Event88::from_index(index),
                });
                *slot = None;
            }
        }
        due.sort_by_key(|event: &ScheduledEvent88| event.fire_cycle);
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
