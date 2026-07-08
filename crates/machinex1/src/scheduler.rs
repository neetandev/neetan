//! Event scheduler.
//!
//! A single monotonic cycle counter (in main-clock units) lives on the bus; this
//! scheduler tracks the fire cycle of each event kind in a flat array.

use common::StackVec;

/// Kinds of scheduled events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EventX1 {
    /// Start of vertical blanking.
    VBlank,
    /// Vertical sync pulse.
    VSync,
    /// Start of a display scanline.
    Scanline,
    /// Zero count of Z80 CTC channel 0.
    CtcChannel0,
    /// Zero count of Z80 CTC channel 1.
    CtcChannel1,
    /// Zero count of Z80 CTC channel 2.
    CtcChannel2,
    /// Zero count of Z80 CTC channel 3.
    CtcChannel3,
    /// DMA pacing tick at the FDC's next data-request slot.
    DmaTick,
    /// FDC seek / command completion.
    FdcSeekComplete,
    /// Periodic keyboard matrix scan.
    KeyScan,
    /// Cassette demodulated-byte tick.
    CassetteByte,
    /// Periodic sub-CPU mailbox step.
    SubPoll,
    /// Z80 SIO channel 0 transmit-bit clock (turbo, RS-232C).
    SioTxCh0,
    /// Z80 SIO channel 0 receive-bit clock (turbo, RS-232C).
    SioRxCh0,
    /// Z80 SIO channel 1 transmit-bit clock (turbo, mouse).
    SioTxCh1,
    /// Z80 SIO channel 1 receive-bit clock (turbo, mouse).
    SioRxCh1,
    /// Zero count of the sound-board Z80 CTC channel 0 (CZ-8BS1 FM board).
    SoundCtcChannel0,
    /// Zero count of the sound-board Z80 CTC channel 1.
    SoundCtcChannel1,
    /// Zero count of the sound-board Z80 CTC channel 2.
    SoundCtcChannel2,
    /// Zero count of the sound-board Z80 CTC channel 3.
    SoundCtcChannel3,
    /// OPM (YM2151) timer A overflow.
    FmTimerA,
    /// OPM (YM2151) timer B overflow.
    FmTimerB,
}

const EVENT_COUNT: usize = 22;

impl EventX1 {
    const ALL: [EventX1; EVENT_COUNT] = [
        EventX1::VBlank,
        EventX1::VSync,
        EventX1::Scanline,
        EventX1::CtcChannel0,
        EventX1::CtcChannel1,
        EventX1::CtcChannel2,
        EventX1::CtcChannel3,
        EventX1::DmaTick,
        EventX1::FdcSeekComplete,
        EventX1::KeyScan,
        EventX1::CassetteByte,
        EventX1::SubPoll,
        EventX1::SioTxCh0,
        EventX1::SioRxCh0,
        EventX1::SioTxCh1,
        EventX1::SioRxCh1,
        EventX1::SoundCtcChannel0,
        EventX1::SoundCtcChannel1,
        EventX1::SoundCtcChannel2,
        EventX1::SoundCtcChannel3,
        EventX1::FmTimerA,
        EventX1::FmTimerB,
    ];

    const fn index(self) -> usize {
        self as usize
    }

    const fn from_index(index: usize) -> Self {
        Self::ALL[index]
    }

    /// The scheduler event for main CTC channel `channel` (0..=3).
    pub const fn ctc_channel(channel: usize) -> Self {
        match channel {
            0 => EventX1::CtcChannel0,
            1 => EventX1::CtcChannel1,
            2 => EventX1::CtcChannel2,
            _ => EventX1::CtcChannel3,
        }
    }

    /// The scheduler event for sound-board CTC channel `channel` (0..=3).
    pub const fn sound_ctc_channel(channel: usize) -> Self {
        match channel {
            0 => EventX1::SoundCtcChannel0,
            1 => EventX1::SoundCtcChannel1,
            2 => EventX1::SoundCtcChannel2,
            _ => EventX1::SoundCtcChannel3,
        }
    }

    /// The scheduler event for OPM timer `timer_id` (0 = A, 1 = B).
    pub const fn fm_timer(timer_id: u8) -> Self {
        match timer_id {
            0 => EventX1::FmTimerA,
            _ => EventX1::FmTimerB,
        }
    }
}

/// A scheduled event together with the cycle it fires at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledEventX1 {
    /// Cycle at which the event fires.
    pub fire_cycle: u64,
    /// The event kind.
    pub kind: EventX1,
}

/// Tracks the next fire cycle of each event kind.
pub struct X1Scheduler {
    fire_cycles: [Option<u64>; EVENT_COUNT],
}

impl X1Scheduler {
    /// Creates a scheduler with no events pending.
    pub fn new() -> Self {
        Self {
            fire_cycles: [None; EVENT_COUNT],
        }
    }

    /// Schedules `kind` to fire at `fire_cycle`, replacing any prior schedule.
    pub fn schedule(&mut self, kind: EventX1, fire_cycle: u64) {
        self.fire_cycles[kind.index()] = Some(fire_cycle);
    }

    /// Cancels any pending schedule for `kind`.
    pub fn cancel(&mut self, kind: EventX1) {
        self.fire_cycles[kind.index()] = None;
    }

    /// Returns the earliest scheduled fire cycle, if any event is pending.
    pub fn next_event_cycle(&self) -> Option<u64> {
        self.fire_cycles.iter().flatten().copied().min()
    }

    /// Removes and returns all events due at or before `current_cycle`, ordered
    /// by fire cycle.
    pub fn pop_due_events(
        &mut self,
        current_cycle: u64,
    ) -> StackVec<ScheduledEventX1, EVENT_COUNT> {
        let mut due = StackVec::new();
        for (index, slot) in self.fire_cycles.iter_mut().enumerate() {
            if let Some(fire_cycle) = *slot
                && fire_cycle <= current_cycle
            {
                due.push(ScheduledEventX1 {
                    fire_cycle,
                    kind: EventX1::from_index(index),
                });
                *slot = None;
            }
        }
        due.sort_by_key(|event: &ScheduledEventX1| event.fire_cycle);
        due
    }
}

impl Default for X1Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_and_pops_in_fire_order() {
        let mut scheduler = X1Scheduler::new();
        scheduler.schedule(EventX1::Scanline, 100);
        scheduler.schedule(EventX1::VBlank, 50);

        assert_eq!(scheduler.next_event_cycle(), Some(50));

        let due = scheduler.pop_due_events(100);
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].kind, EventX1::VBlank);
        assert_eq!(due[1].kind, EventX1::Scanline);
        assert_eq!(scheduler.next_event_cycle(), None);
    }

    #[test]
    fn cancel_drops_a_pending_event() {
        let mut scheduler = X1Scheduler::new();
        scheduler.schedule(EventX1::VSync, 200);
        scheduler.cancel(EventX1::VSync);
        assert_eq!(scheduler.next_event_cycle(), None);
    }

    #[test]
    fn pop_leaves_future_events_scheduled() {
        let mut scheduler = X1Scheduler::new();
        scheduler.schedule(EventX1::VSync, 300);
        scheduler.schedule(EventX1::Scanline, 100);

        let due = scheduler.pop_due_events(150);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].kind, EventX1::Scanline);
        assert_eq!(scheduler.next_event_cycle(), Some(300));
    }

    #[test]
    fn rescheduling_replaces_the_prior_fire_cycle() {
        let mut scheduler = X1Scheduler::new();
        scheduler.schedule(EventX1::Scanline, 100);
        scheduler.schedule(EventX1::Scanline, 40);
        assert_eq!(scheduler.next_event_cycle(), Some(40));
    }
}
