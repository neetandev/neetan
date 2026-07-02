//! Event-driven scheduler for the FM Towns.

use common::StackVec;

/// Number of distinct FM Towns event kinds.
pub const EVENT_TOWNS_KIND_COUNT: usize = 11;

/// Kinds of scheduled FM Towns events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum EventTowns {
    /// Interval-timer channel 0 output edge (drives the timer IRQ 0 when armed).
    #[default]
    TimerChannel0,
    /// Interval-timer channel 1 output edge (drives the timer IRQ 0 when armed).
    TimerChannel1,
    /// Keyboard receive-FIFO service tick (delivers the next queued scancode).
    KeyboardReady,
    /// Start of vertical sync: raises IRQ 11 and renders the frame.
    VsyncStart,
    /// End of vertical sync: lowers the VSYNC display-status bit.
    VsyncEnd,
    /// CD-ROM controller task: command sequencing, sector reads, CD-DA polling.
    CdTask,
    /// OPN2 FM timer A expiry (drives the shared sound IRQ 13).
    FmTimerA,
    /// OPN2 FM timer B expiry (drives the shared sound IRQ 13).
    FmTimerB,
    /// Sprite transfer completion: clears the busy flag and paints the page.
    SpriteFinish,
    /// MB8877 FDC command completion: performs the DMA transfer and raises IRQ 6.
    FdcTask,
    /// SCSI SPC command completion: performs the DMA transfer and raises IRQ 8.
    ScsiTask,
}

impl EventTowns {
    const ALL: [EventTowns; EVENT_TOWNS_KIND_COUNT] = [
        EventTowns::TimerChannel0,
        EventTowns::TimerChannel1,
        EventTowns::KeyboardReady,
        EventTowns::VsyncStart,
        EventTowns::VsyncEnd,
        EventTowns::CdTask,
        EventTowns::FmTimerA,
        EventTowns::FmTimerB,
        EventTowns::SpriteFinish,
        EventTowns::FdcTask,
        EventTowns::ScsiTask,
    ];

    const fn from_index(index: usize) -> Self {
        Self::ALL[index]
    }
}

/// Snapshot of a single scheduled FM Towns event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledEventTowns {
    /// CPU cycle at which this event fires.
    pub fire_cycle: u64,
    /// The event type.
    pub kind: EventTowns,
}

/// Snapshot of the scheduler's pending event queue.
///
/// Uses a flat array indexed by [`EventTowns`] discriminant. Each slot holds
/// `Some(fire_cycle)` when an event of that kind is scheduled, or `None` when it
/// is not. At most one event per kind can be active at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TownsSchedulerState {
    /// Fire cycle for each event kind, indexed by discriminant.
    pub fire_cycles: [Option<u64>; EVENT_TOWNS_KIND_COUNT],
}

/// Event-driven scheduler for timed FM Towns peripheral events.
pub struct TownsScheduler {
    /// Embedded state for save/restore.
    pub state: TownsSchedulerState,
}

impl Default for TownsScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl TownsScheduler {
    /// Creates a new empty scheduler.
    pub fn new() -> Self {
        Self {
            state: TownsSchedulerState {
                fire_cycles: [None; EVENT_TOWNS_KIND_COUNT],
            },
        }
    }

    /// Schedules an event to fire at `fire_cycle`. Replaces any existing event of
    /// the same kind.
    pub fn schedule(&mut self, kind: EventTowns, fire_cycle: u64) {
        self.state.fire_cycles[kind as usize] = Some(fire_cycle);
    }

    /// Cancels any scheduled event of the given kind.
    pub fn cancel(&mut self, kind: EventTowns) {
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
    ) -> StackVec<ScheduledEventTowns, EVENT_TOWNS_KIND_COUNT> {
        let mut due = StackVec::new();
        for (index, slot) in self.state.fire_cycles.iter_mut().enumerate() {
            if let Some(fire_cycle) = *slot
                && fire_cycle <= current_cycle
            {
                due.push(ScheduledEventTowns {
                    fire_cycle,
                    kind: EventTowns::from_index(index),
                });
                *slot = None;
            }
        }
        due.sort_by_key(|event: &ScheduledEventTowns| event.fire_cycle);
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_and_pops_in_order() {
        let mut scheduler = TownsScheduler::new();
        scheduler.schedule(EventTowns::TimerChannel0, 100);
        assert_eq!(scheduler.next_event_cycle(), Some(100));
        assert_eq!(scheduler.pop_due_events(50).len(), 0);

        let due = scheduler.pop_due_events(150);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].kind, EventTowns::TimerChannel0);
        assert_eq!(scheduler.next_event_cycle(), None);
    }
}
