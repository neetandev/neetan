//! Event-driven scheduler for PC-98 machines.

use common::StackVec;
use device::{
    sound_blaster_16::SoundboardSb16Timer, soundboard_14::Soundboard14Timer,
    soundboard_26k::Soundboard26kTimer, soundboard_86::Soundboard86Timer,
};

/// Number of distinct PC-98 event kinds.
pub const EVENT_KIND_COUNT: usize = 23;

/// Kinds of scheduled PC-98 events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum EventKind {
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

impl EventKind {
    const ALL: [Self; EVENT_KIND_COUNT] = [
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
}

impl From<Soundboard86Timer> for EventKind {
    fn from(timer: Soundboard86Timer) -> Self {
        match timer {
            Soundboard86Timer::FmTimerA => Self::FmTimerA,
            Soundboard86Timer::FmTimerB => Self::FmTimerB,
            Soundboard86Timer::Pcm86Irq => Self::Pcm86Irq,
        }
    }
}

impl From<Soundboard14Timer> for EventKind {
    fn from(_: Soundboard14Timer) -> Self {
        Self::MusicGen14Timer
    }
}

impl From<Soundboard26kTimer> for EventKind {
    fn from(timer: Soundboard26kTimer) -> Self {
        match timer {
            Soundboard26kTimer::FmTimerA => Self::FmTimerA,
            Soundboard26kTimer::FmTimerB => Self::FmTimerB,
            Soundboard26kTimer::FmTimer2A => Self::FmTimer2A,
            Soundboard26kTimer::FmTimer2B => Self::FmTimer2B,
        }
    }
}

impl From<SoundboardSb16Timer> for EventKind {
    fn from(timer: SoundboardSb16Timer) -> Self {
        match timer {
            SoundboardSb16Timer::OplTimerA => Self::Sb16OplTimerA,
            SoundboardSb16Timer::OplTimerB => Self::Sb16OplTimerB,
        }
    }
}

/// A scheduled PC-98 event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScheduledEvent {
    /// CPU cycle at which the event fires.
    pub fire_cycle: u64,
    /// Event type.
    pub kind: EventKind,
}

/// Snapshot of the PC-98 scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerState {
    /// Fire cycles indexed by event discriminant.
    pub fire_cycles: [Option<u64>; EVENT_KIND_COUNT],
}

/// Event-driven scheduler for PC-98 peripheral events.
pub struct Scheduler {
    /// Embedded saveable state.
    pub state: SchedulerState,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler {
    /// Creates an empty scheduler.
    pub fn new() -> Self {
        Self {
            state: SchedulerState {
                fire_cycles: [None; EVENT_KIND_COUNT],
            },
        }
    }

    /// Schedules or replaces an event.
    pub fn schedule(&mut self, kind: EventKind, fire_cycle: u64) {
        self.state.fire_cycles[kind as usize] = Some(fire_cycle);
    }

    /// Cancels an event.
    pub fn cancel(&mut self, kind: EventKind) {
        self.state.fire_cycles[kind as usize] = None;
    }

    /// Returns the earliest pending event cycle.
    pub fn next_event_cycle(&self) -> Option<u64> {
        self.state
            .fire_cycles
            .iter()
            .filter_map(|&cycle| cycle)
            .min()
    }

    /// Removes and returns all due events.
    pub fn pop_due_events(
        &mut self,
        current_cycle: u64,
    ) -> StackVec<ScheduledEvent, EVENT_KIND_COUNT> {
        let mut due = StackVec::new();
        for (index, slot) in self.state.fire_cycles.iter_mut().enumerate() {
            if let Some(fire_cycle) = *slot
                && fire_cycle <= current_cycle
            {
                due.push(ScheduledEvent {
                    fire_cycle,
                    kind: EventKind::from_index(index),
                });
                *slot = None;
            }
        }
        due.sort_by_key(|event: &ScheduledEvent| event.fire_cycle);
        due
    }
}
