//! Event scheduler.
//!
//! A single monotonic cycle counter (in main-clock units) lives on the bus; this
//! scheduler tracks the fire cycle of each event kind in a flat array.

use common::StackVec;

/// Kinds of scheduled events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Event60 {
    /// Periodic timer interrupt (the main BASIC tick).
    TimerIrq,
    /// Vertical retrace interrupt.
    Vrtc,
    /// Next demodulated cassette byte ready.
    CassetteByte,
    /// Periodic keyboard matrix scan.
    KeyScan,
    /// uPD765A non-DMA data-rate byte pacing tick.
    FdcDrqByte,
    /// uPD765A seek/recalibrate completion interrupt.
    FdcSeekComplete,
    /// YM2203 FM timer A expiry (SR).
    FmTimerA,
    /// YM2203 FM timer B expiry (SR).
    FmTimerB,
    /// Start of a display scanline (drives the VRAM bus-request stall).
    Scanline,
    /// End of the per-scanline VRAM bus-request window.
    BusReqEnd,
    /// uPD7752 is ready to request its next voice parameter frame.
    VoiceRequest,
}

#[cfg(test)]
mod trace_identifier_tests {
    use super::*;

    #[test]
    fn trace_identifiers_match_every_event_variant() {
        assert_eq!(
            Event60::ALL.len(),
            common::trace_id::scheduled::pc60::ALL.len()
        );
        for (event, identifier) in Event60::ALL
            .iter()
            .zip(common::trace_id::scheduled::pc60::ALL)
        {
            assert_eq!(event.trace_name(), *identifier);
        }
    }
}

impl Event60 {
    pub(crate) const fn trace_name(self) -> &'static str {
        use common::trace_id::scheduled::pc60;
        match self {
            Self::TimerIrq => pc60::TIMER_IRQ,
            Self::Vrtc => pc60::VIDEO_VRTC,
            Self::CassetteByte => pc60::CASSETTE_BYTE,
            Self::KeyScan => pc60::KEYBOARD_SCAN,
            Self::FdcDrqByte => pc60::FDC_DRQ,
            Self::FdcSeekComplete => pc60::FDC_SEEK_COMPLETE,
            Self::FmTimerA => pc60::FM_TIMER_A,
            Self::FmTimerB => pc60::FM_TIMER_B,
            Self::Scanline => pc60::VIDEO_SCANLINE,
            Self::BusReqEnd => pc60::VIDEO_BUS_REQUEST_END,
            Self::VoiceRequest => pc60::VOICE_REQUEST,
        }
    }
}

const EVENT_COUNT: usize = 11;

impl Event60 {
    const ALL: [Event60; EVENT_COUNT] = [
        Event60::TimerIrq,
        Event60::Vrtc,
        Event60::CassetteByte,
        Event60::KeyScan,
        Event60::FdcDrqByte,
        Event60::FdcSeekComplete,
        Event60::FmTimerA,
        Event60::FmTimerB,
        Event60::Scanline,
        Event60::BusReqEnd,
        Event60::VoiceRequest,
    ];

    const fn index(self) -> usize {
        self as usize
    }

    const fn from_index(index: usize) -> Self {
        Self::ALL[index]
    }
}

/// A scheduled event together with the cycle it fires at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScheduledEvent60 {
    /// Cycle at which the event fires.
    pub(crate) fire_cycle: u64,
    /// The event kind.
    pub(crate) kind: Event60,
}

/// Tracks the next fire cycle of each event kind.
pub(crate) struct Pc6000Scheduler {
    fire_cycles: [Option<u64>; EVENT_COUNT],
}

impl Pc6000Scheduler {
    /// Creates a scheduler with no events pending.
    pub(crate) fn new() -> Self {
        Self {
            fire_cycles: [None; EVENT_COUNT],
        }
    }

    /// Schedules `kind` to fire at `fire_cycle`, replacing any prior schedule.
    pub(crate) fn schedule(&mut self, kind: Event60, fire_cycle: u64) {
        self.fire_cycles[kind.index()] = Some(fire_cycle);
    }

    /// Cancels any pending schedule for `kind`.
    pub(crate) fn cancel(&mut self, kind: Event60) {
        self.fire_cycles[kind.index()] = None;
    }

    /// Returns the earliest scheduled fire cycle, if any event is pending.
    pub(crate) fn next_event_cycle(&self) -> Option<u64> {
        self.fire_cycles.iter().flatten().copied().min()
    }

    /// Removes and returns all events due at or before `current_cycle`,
    /// ordered by fire cycle.
    pub(crate) fn pop_due_events(
        &mut self,
        current_cycle: u64,
    ) -> StackVec<ScheduledEvent60, EVENT_COUNT> {
        let mut due = StackVec::new();
        for (index, slot) in self.fire_cycles.iter_mut().enumerate() {
            if let Some(fire_cycle) = *slot
                && fire_cycle <= current_cycle
            {
                due.push(ScheduledEvent60 {
                    fire_cycle,
                    kind: Event60::from_index(index),
                });
                *slot = None;
            }
        }
        due.sort_by_key(|event: &ScheduledEvent60| event.fire_cycle);
        due
    }
}

impl Default for Pc6000Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_and_pops_the_fm_timers() {
        let mut scheduler = Pc6000Scheduler::new();
        scheduler.schedule(Event60::FmTimerA, 100);
        scheduler.schedule(Event60::FmTimerB, 50);

        assert_eq!(scheduler.next_event_cycle(), Some(50));

        // Both are due by cycle 100 and come back in fire-cycle order.
        let due = scheduler.pop_due_events(100);
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].kind, Event60::FmTimerB);
        assert_eq!(due[1].kind, Event60::FmTimerA);
        assert_eq!(scheduler.next_event_cycle(), None);
    }

    #[test]
    fn cancel_drops_a_pending_fm_timer() {
        let mut scheduler = Pc6000Scheduler::new();
        scheduler.schedule(Event60::FmTimerA, 200);
        scheduler.cancel(Event60::FmTimerA);
        assert_eq!(scheduler.next_event_cycle(), None);
    }

    #[test]
    fn pop_leaves_events_scheduled_in_the_future() {
        let mut scheduler = Pc6000Scheduler::new();
        scheduler.schedule(Event60::Vrtc, 300);
        scheduler.schedule(Event60::FmTimerA, 100);

        let due = scheduler.pop_due_events(150);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].kind, Event60::FmTimerA);
        // The future Vrtc event is still pending.
        assert_eq!(scheduler.next_event_cycle(), Some(300));
    }

    #[test]
    fn rescheduling_replaces_the_prior_fire_cycle() {
        let mut scheduler = Pc6000Scheduler::new();
        scheduler.schedule(Event60::FmTimerB, 100);
        scheduler.schedule(Event60::FmTimerB, 40);
        assert_eq!(scheduler.next_event_cycle(), Some(40));
    }
}
