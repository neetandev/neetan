//! Event-driven scheduler for PC-98 machines.

use common::{SchedulerState, StackVec};
use device::{
    sound_blaster_16::SoundboardSb16Timer, soundboard_14::Soundboard14Timer,
    soundboard_26k::Soundboard26kTimer, soundboard_86::Soundboard86Timer,
};

/// Number of distinct PC-98 event kinds.
const EVENT98_KIND_COUNT: usize = 23;

/// Kinds of scheduled PC-98 events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Event98 {
    /// PIT channel 0 reached terminal count.
    #[default]
    PitTimer0,
    /// GDC vertical sync begins.
    GdcVsync,
    /// GDC active display period begins.
    GdcDisplayStart,
    /// FDC execution phase completes.
    FdcExecution,
    /// FDC interrupt is raised.
    FdcInterrupt,
    /// GDC drawing operation completes.
    GdcDrawingComplete,
    /// Mouse interface timer ticks.
    MouseTimer,
    /// First FM board timer A overflows.
    FmTimerA,
    /// First FM board timer B overflows.
    FmTimerB,
    /// Second FM board timer A overflows.
    FmTimer2A,
    /// Second FM board timer B overflows.
    FmTimer2B,
    /// SASI controller execution completes.
    SasiExecution,
    /// SASI controller interrupt is raised.
    SasiInterrupt,
    /// IDE controller execution completes.
    IdeExecution,
    /// IDE controller interrupt is raised.
    IdeInterrupt,
    /// PCM86 DAC IRQ threshold is checked.
    Pcm86Irq,
    /// SB16 OPL timer A overflows.
    Sb16OplTimerA,
    /// SB16 OPL timer B overflows.
    Sb16OplTimerB,
    /// SB16 DSP DMA batch is transferred.
    Sb16DspDma,
    /// MPU-PC98II timer ticks.
    MpuTimer,
    /// Music Generator board timer expires.
    MusicGen14Timer,
    /// GA-1280A vertical blanking begins.
    GaVsync,
    /// GA-1280A active display begins.
    GaDisplayStart,
}

#[cfg(test)]
mod trace_identifier_tests {
    use super::*;

    #[test]
    fn trace_identifiers_match_every_event_variant() {
        assert_eq!(
            Event98::ALL.len(),
            common::trace_id::scheduled::pc98::ALL.len()
        );
        for (event, identifier) in Event98::ALL
            .iter()
            .zip(common::trace_id::scheduled::pc98::ALL)
        {
            assert_eq!(event.trace_name(), *identifier);
        }
    }

    #[test]
    fn equal_deadline_events_use_frozen_priority_order() {
        let mut scheduler = Pc98Scheduler::new();
        for event in Event98::ALL.into_iter().rev() {
            scheduler.schedule(event, 42);
        }

        let due = scheduler.pop_due_events(42);
        let actual: Vec<_> = due.iter().map(|event| event.kind).collect();

        assert_eq!(actual, Event98::ALL);
    }
}

impl Event98 {
    const ALL: [Self; EVENT98_KIND_COUNT] = [
        Self::PitTimer0,
        Self::GdcVsync,
        Self::GdcDisplayStart,
        Self::FdcExecution,
        Self::FdcInterrupt,
        Self::GdcDrawingComplete,
        Self::MouseTimer,
        Self::FmTimerA,
        Self::FmTimerB,
        Self::FmTimer2A,
        Self::FmTimer2B,
        Self::SasiExecution,
        Self::SasiInterrupt,
        Self::IdeExecution,
        Self::IdeInterrupt,
        Self::Pcm86Irq,
        Self::Sb16OplTimerA,
        Self::Sb16OplTimerB,
        Self::Sb16DspDma,
        Self::MpuTimer,
        Self::MusicGen14Timer,
        Self::GaVsync,
        Self::GaDisplayStart,
    ];

    const fn from_index(index: usize) -> Self {
        Self::ALL[index]
    }

    pub(crate) const fn trace_name(self) -> &'static str {
        use common::trace_id::scheduled::pc98;
        match self {
            Self::PitTimer0 => pc98::PIT_TIMER0,
            Self::GdcVsync => pc98::GDC_VSYNC,
            Self::GdcDisplayStart => pc98::GDC_DISPLAY_START,
            Self::FdcExecution => pc98::FDC_EXECUTION,
            Self::FdcInterrupt => pc98::FDC_INTERRUPT,
            Self::GdcDrawingComplete => pc98::GDC_DRAWING_COMPLETE,
            Self::MouseTimer => pc98::MOUSE_TIMER,
            Self::FmTimerA => pc98::FM_TIMER_A,
            Self::FmTimerB => pc98::FM_TIMER_B,
            Self::FmTimer2A => pc98::FM2_TIMER_A,
            Self::FmTimer2B => pc98::FM2_TIMER_B,
            Self::SasiExecution => pc98::SASI_EXECUTION,
            Self::SasiInterrupt => pc98::SASI_INTERRUPT,
            Self::IdeExecution => pc98::IDE_EXECUTION,
            Self::IdeInterrupt => pc98::IDE_INTERRUPT,
            Self::Pcm86Irq => pc98::PCM86_IRQ,
            Self::Sb16OplTimerA => pc98::SB16_OPL_TIMER_A,
            Self::Sb16OplTimerB => pc98::SB16_OPL_TIMER_B,
            Self::Sb16DspDma => pc98::SB16_DSP_DMA,
            Self::MpuTimer => pc98::MPU_TIMER,
            Self::MusicGen14Timer => pc98::MUSIC_GEN14_TIMER,
            Self::GaVsync => pc98::GA_VSYNC,
            Self::GaDisplayStart => pc98::GA_DISPLAY_START,
        }
    }
}

impl From<Soundboard86Timer> for Event98 {
    fn from(timer: Soundboard86Timer) -> Self {
        match timer {
            Soundboard86Timer::FmTimerA => Self::FmTimerA,
            Soundboard86Timer::FmTimerB => Self::FmTimerB,
            Soundboard86Timer::Pcm86Irq => Self::Pcm86Irq,
        }
    }
}

impl From<Soundboard14Timer> for Event98 {
    fn from(_: Soundboard14Timer) -> Self {
        Self::MusicGen14Timer
    }
}

impl From<Soundboard26kTimer> for Event98 {
    fn from(timer: Soundboard26kTimer) -> Self {
        match timer {
            Soundboard26kTimer::FmTimerA => Self::FmTimerA,
            Soundboard26kTimer::FmTimerB => Self::FmTimerB,
            Soundboard26kTimer::FmTimer2A => Self::FmTimer2A,
            Soundboard26kTimer::FmTimer2B => Self::FmTimer2B,
        }
    }
}

impl From<SoundboardSb16Timer> for Event98 {
    fn from(timer: SoundboardSb16Timer) -> Self {
        match timer {
            SoundboardSb16Timer::OplTimerA => Self::Sb16OplTimerA,
            SoundboardSb16Timer::OplTimerB => Self::Sb16OplTimerB,
        }
    }
}

/// A scheduled PC-98 event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ScheduledEvent98 {
    /// CPU cycle at which the event fires.
    pub(crate) fire_cycle: u64,
    /// Event type.
    pub(crate) kind: Event98,
}

/// Authoritative state of the PC-98 scheduler.
pub type Pc98SchedulerState = SchedulerState;

/// Event-driven scheduler for PC-98 peripheral events.
// savestate: authoritative
pub(crate) struct Pc98Scheduler {
    /// Embedded saveable state.
    pub(crate) state: Pc98SchedulerState,
}

impl Default for Pc98Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Pc98Scheduler {
    /// Creates an empty scheduler.
    pub(crate) fn new() -> Self {
        Self {
            state: SchedulerState::new(EVENT98_KIND_COUNT),
        }
    }

    /// Schedules or replaces an event.
    pub(crate) fn schedule(&mut self, kind: Event98, fire_cycle: u64) {
        self.state.schedule(kind as usize, fire_cycle);
    }

    /// Cancels an event.
    pub(crate) fn cancel(&mut self, kind: Event98) {
        self.state.cancel(kind as usize);
    }

    /// Returns the earliest pending event cycle.
    pub(crate) fn next_event_cycle(&self) -> Option<u64> {
        self.state.next_event_cycle()
    }

    /// Removes and returns all due events.
    pub(crate) fn pop_due_events(
        &mut self,
        current_cycle: u64,
    ) -> StackVec<ScheduledEvent98, EVENT98_KIND_COUNT> {
        let indexes = self.state.pop_due::<EVENT98_KIND_COUNT>(current_cycle);
        let mut due = StackVec::new();
        for event in indexes.iter() {
            due.push(ScheduledEvent98 {
                fire_cycle: event.fire_cycle,
                kind: Event98::from_index(event.index),
            });
        }
        due
    }
}
