//! Event-driven scheduler for the PC/AT.

use common::StackVec;

/// Number of distinct PC/AT event kinds.
const EVENT_AT_KIND_COUNT: usize = 18;

/// Kinds of scheduled PC/AT events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub(crate) enum EventAt {
    /// PIT channel 0 output edge (drives the timer IRQ 0).
    #[default]
    PitChannel0,
    /// PIT channel 0 falling edge (clears timer IRQ 0).
    PitChannel0Low,
    /// MC146818 one-second update tick (drives IRQ 8 when enabled).
    RtcUpdate,
    /// MC146818 periodic tick (drives IRQ 8 when enabled).
    RtcPeriodic,
    /// 8042 output-buffer delivery tick (loads the next queued byte).
    KbcDeliver,
    /// AT keyboard typematic repeat tick.
    KeyboardTypematic,
    /// VGA frame tick at vertical sync start (renders and re-arms).
    VgaFrame,
    /// FDC command execution (DMA transfer and result assembly).
    FdcExecution,
    /// FDC completion interrupt delivery (seek end, reset polling, results).
    FdcInterrupt,
    /// IDE primary-channel command execution completion.
    IdeExecution,
    /// IDE primary-channel completion interrupt delivery.
    IdeInterrupt,
    /// IDE secondary-channel (ATAPI) command execution completion.
    IdeSecondaryExecution,
    /// IDE secondary-channel (ATAPI) completion interrupt delivery.
    IdeSecondaryInterrupt,
    /// Sound Blaster 16 OPL3 timer A expiry.
    Sb16OplTimerA,
    /// Sound Blaster 16 OPL3 timer B expiry.
    Sb16OplTimerB,
    /// Sound Blaster 16 DSP DMA batch transfer.
    Sb16DspDma,
    /// MPU-401 intelligent-mode timing tick.
    MpuTimer,
    /// COM1 UART received-byte release (paces the serial mouse packet).
    UartRx,
}

#[cfg(test)]
mod trace_identifier_tests {
    use super::*;

    #[test]
    fn trace_identifiers_match_every_event_variant() {
        assert_eq!(
            EventAt::ALL.len(),
            common::trace_id::scheduled::at::ALL.len()
        );
        for (event, identifier) in EventAt::ALL
            .iter()
            .zip(common::trace_id::scheduled::at::ALL)
        {
            assert_eq!(event.trace_name(), *identifier);
        }
    }
}

impl EventAt {
    const ALL: [EventAt; EVENT_AT_KIND_COUNT] = [
        EventAt::PitChannel0,
        EventAt::PitChannel0Low,
        EventAt::RtcUpdate,
        EventAt::RtcPeriodic,
        EventAt::KbcDeliver,
        EventAt::KeyboardTypematic,
        EventAt::VgaFrame,
        EventAt::FdcExecution,
        EventAt::FdcInterrupt,
        EventAt::IdeExecution,
        EventAt::IdeInterrupt,
        EventAt::IdeSecondaryExecution,
        EventAt::IdeSecondaryInterrupt,
        EventAt::Sb16OplTimerA,
        EventAt::Sb16OplTimerB,
        EventAt::Sb16DspDma,
        EventAt::MpuTimer,
        EventAt::UartRx,
    ];

    const fn from_index(index: usize) -> Self {
        Self::ALL[index]
    }

    pub(crate) const fn trace_name(self) -> &'static str {
        use common::trace_id::scheduled::at;
        match self {
            Self::PitChannel0 => at::PIT_CHANNEL0,
            Self::PitChannel0Low => at::PIT_CHANNEL0_LOW,
            Self::RtcUpdate => at::RTC_UPDATE,
            Self::RtcPeriodic => at::RTC_PERIODIC,
            Self::KbcDeliver => at::KBC_DELIVER,
            Self::KeyboardTypematic => at::KEYBOARD_TYPEMATIC,
            Self::VgaFrame => at::VGA_FRAME,
            Self::FdcExecution => at::FDC_EXECUTION,
            Self::FdcInterrupt => at::FDC_INTERRUPT,
            Self::IdeExecution => at::IDE_EXECUTION,
            Self::IdeInterrupt => at::IDE_INTERRUPT,
            Self::IdeSecondaryExecution => at::IDE_SECONDARY_EXECUTION,
            Self::IdeSecondaryInterrupt => at::IDE_SECONDARY_INTERRUPT,
            Self::Sb16OplTimerA => at::SB16_OPL_TIMER_A,
            Self::Sb16OplTimerB => at::SB16_OPL_TIMER_B,
            Self::Sb16DspDma => at::SB16_DSP_DMA,
            Self::MpuTimer => at::MPU_TIMER,
            Self::UartRx => at::UART_RX,
        }
    }
}

impl From<device::sound_blaster_16::SoundboardSb16Timer> for EventAt {
    fn from(timer: device::sound_blaster_16::SoundboardSb16Timer) -> Self {
        match timer {
            device::sound_blaster_16::SoundboardSb16Timer::OplTimerA => EventAt::Sb16OplTimerA,
            device::sound_blaster_16::SoundboardSb16Timer::OplTimerB => EventAt::Sb16OplTimerB,
        }
    }
}

/// Snapshot of a single scheduled PC/AT event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScheduledEventAt {
    /// CPU cycle at which this event fires.
    pub(crate) fire_cycle: u64,
    /// The event type.
    pub(crate) kind: EventAt,
}

/// Snapshot of the scheduler's pending event queue.
///
/// Uses a flat array indexed by [`EventAt`] discriminant; each slot holds
/// `Some(fire_cycle)` when scheduled. At most one event per kind is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AtSchedulerState {
    /// Fire cycle for each event kind, indexed by discriminant.
    pub(crate) fire_cycles: [Option<u64>; EVENT_AT_KIND_COUNT],
}

/// Event-driven scheduler for timed PC/AT peripheral events.
pub(crate) struct AtScheduler {
    /// Embedded state for save/restore.
    pub(crate) state: AtSchedulerState,
}

impl Default for AtScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl AtScheduler {
    /// Creates a new empty scheduler.
    pub(crate) fn new() -> Self {
        Self {
            state: AtSchedulerState {
                fire_cycles: [None; EVENT_AT_KIND_COUNT],
            },
        }
    }

    /// Schedules an event to fire at `fire_cycle`, replacing any existing event
    /// of the same kind.
    pub(crate) fn schedule(&mut self, kind: EventAt, fire_cycle: u64) {
        self.state.fire_cycles[kind as usize] = Some(fire_cycle);
    }

    /// Cancels any scheduled event of the given kind.
    pub(crate) fn cancel(&mut self, kind: EventAt) {
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
    ) -> StackVec<ScheduledEventAt, EVENT_AT_KIND_COUNT> {
        let mut due = StackVec::new();
        for (index, slot) in self.state.fire_cycles.iter_mut().enumerate() {
            if let Some(fire_cycle) = *slot
                && fire_cycle <= current_cycle
            {
                due.push(ScheduledEventAt {
                    fire_cycle,
                    kind: EventAt::from_index(index),
                });
                *slot = None;
            }
        }
        due.sort_by_key(|event: &ScheduledEventAt| event.fire_cycle);
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_and_pops_in_order() {
        let mut scheduler = AtScheduler::new();
        scheduler.schedule(EventAt::PitChannel0, 100);
        scheduler.schedule(EventAt::RtcUpdate, 50);
        assert_eq!(scheduler.next_event_cycle(), Some(50));

        let due = scheduler.pop_due_events(60);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].kind, EventAt::RtcUpdate);
        assert_eq!(scheduler.next_event_cycle(), Some(100));
    }
}
