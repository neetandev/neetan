//! Event-driven scheduler for the X68000.

use common::StackVec;

/// Number of distinct X68000 event kinds.
const EVENT_X68K_KIND_COUNT: usize = 14;

/// Kinds of scheduled X68000 events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub(crate) enum EventX68k {
    /// CRTC reaches a signal or raster boundary.
    #[default]
    Crtc,
    /// MC68901 MFP reaches a timer or serial boundary.
    Mfp,
    /// RP5C15 RTC reaches a clock output or calendar boundary.
    Rtc,
    /// Keyboard reaches a repeat or output boundary.
    Keyboard,
    /// HD63450 DMAC reaches a scheduled transfer boundary.
    Dmac,
    /// uPD72065 FDC execution-phase byte cadence.
    Fdc,
    /// uPD72065 FDC delayed interrupt delivery.
    FdcInterrupt,
    /// YM2151 OPM timer A overflow.
    OpmTimerA,
    /// YM2151 OPM timer B overflow.
    OpmTimerB,
    /// MSM6258 ADPCM playback byte cadence.
    Adpcm,
    /// SASI hard-disk controller reaches a scheduled boundary.
    Hdc,
    /// Internal MB89352 SCSI controller reaches a scheduled boundary.
    Spc,
    /// CZ-6BM1 YM3802 reaches a transmit or timer boundary.
    Midi,
    /// Z8530 SCC finishes serializing a mouse packet byte.
    SccMouse,
}

impl EventX68k {
    const ALL: [EventX68k; EVENT_X68K_KIND_COUNT] = [
        EventX68k::Crtc,
        EventX68k::Mfp,
        EventX68k::Rtc,
        EventX68k::Keyboard,
        EventX68k::Dmac,
        EventX68k::Fdc,
        EventX68k::FdcInterrupt,
        EventX68k::OpmTimerA,
        EventX68k::OpmTimerB,
        EventX68k::Adpcm,
        EventX68k::Hdc,
        EventX68k::Spc,
        EventX68k::Midi,
        EventX68k::SccMouse,
    ];

    const fn from_index(index: usize) -> Self {
        Self::ALL[index]
    }
}

/// Snapshot of a single scheduled X68000 event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScheduledEventX68k {
    /// CPU cycle at which this event fires.
    pub(crate) fire_cycle: u64,
    /// The event type.
    pub(crate) kind: EventX68k,
}

/// Event-driven scheduler for timed X68000 peripheral events.
///
/// Uses a flat array indexed by [`EventX68k`] discriminant. Each slot holds
/// `Some(fire_cycle)` when an event of that kind is scheduled; at most one
/// event per kind is active at a time.
pub(crate) struct X68kScheduler {
    fire_cycles: [Option<u64>; EVENT_X68K_KIND_COUNT],
}

impl Default for X68kScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl X68kScheduler {
    /// Creates a new empty scheduler.
    pub(crate) fn new() -> Self {
        Self {
            fire_cycles: [None; EVENT_X68K_KIND_COUNT],
        }
    }

    /// Schedules an event to fire at `fire_cycle`, replacing any existing
    /// event of the same kind.
    pub(crate) fn schedule(&mut self, kind: EventX68k, fire_cycle: u64) {
        self.fire_cycles[kind as usize] = Some(fire_cycle);
    }

    /// Cancels any scheduled event of the given kind.
    pub(crate) fn cancel(&mut self, kind: EventX68k) {
        self.fire_cycles[kind as usize] = None;
    }

    /// Returns the scheduled cycle for one event kind, if armed.
    pub(crate) fn event_cycle(&self, kind: EventX68k) -> Option<u64> {
        self.fire_cycles[kind as usize]
    }

    /// Returns the cycle of the earliest scheduled event, if any.
    pub(crate) fn next_event_cycle(&self) -> Option<u64> {
        self.fire_cycles.iter().filter_map(|&cycle| cycle).min()
    }

    /// Removes and returns all events due at or before `current_cycle`,
    /// sorted by fire cycle.
    pub(crate) fn pop_due_events(
        &mut self,
        current_cycle: u64,
    ) -> StackVec<ScheduledEventX68k, EVENT_X68K_KIND_COUNT> {
        let mut due = StackVec::new();
        for (index, slot) in self.fire_cycles.iter_mut().enumerate() {
            if let Some(fire_cycle) = *slot
                && fire_cycle <= current_cycle
            {
                due.push(ScheduledEventX68k {
                    fire_cycle,
                    kind: EventX68k::from_index(index),
                });
                *slot = None;
            }
        }
        due.sort_by_key(|event: &ScheduledEventX68k| event.fire_cycle);
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_and_pops_in_order() {
        let mut scheduler = X68kScheduler::new();
        scheduler.schedule(EventX68k::Fdc, 100);
        scheduler.schedule(EventX68k::Hdc, 60);
        assert_eq!(scheduler.next_event_cycle(), Some(60));
        assert_eq!(scheduler.pop_due_events(50).len(), 0);

        let due = scheduler.pop_due_events(150);
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].kind, EventX68k::Hdc);
        assert_eq!(due[1].kind, EventX68k::Fdc);
        assert_eq!(scheduler.next_event_cycle(), None);
    }

    #[test]
    fn cancel_removes_a_pending_event() {
        let mut scheduler = X68kScheduler::new();
        scheduler.schedule(EventX68k::Spc, 100);
        scheduler.cancel(EventX68k::Spc);
        assert_eq!(scheduler.next_event_cycle(), None);
    }
}
