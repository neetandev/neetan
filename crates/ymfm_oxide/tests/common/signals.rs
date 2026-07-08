use ymfm_oxide::{Y8950, Ym2151, Ym2203, Ym2608, Ym3526, Ym3812, Ymf262, YmfmTimerUpdate};

#[derive(Debug, Clone, PartialEq)]
pub enum SignalEvent {
    SetTimer {
        timer_id: u32,
        duration_in_clocks: i32,
    },
    UpdateIrq {
        asserted: bool,
    },
}

pub trait TakeSignals {
    fn take_signals(&mut self) -> Vec<SignalEvent>;
}

fn timer_duration(update: YmfmTimerUpdate) -> i32 {
    match update {
        YmfmTimerUpdate::Cancel => -1,
        YmfmTimerUpdate::Schedule(duration) => duration as i32,
    }
}

macro_rules! impl_take_signals {
    ($chip:ty) => {
        impl TakeSignals for $chip {
            fn take_signals(&mut self) -> Vec<SignalEvent> {
                let mut events = Vec::new();
                for timer_id in 0..=1 {
                    if let Some(update) = self.take_timer_update(timer_id) {
                        events.push(SignalEvent::SetTimer {
                            timer_id: u32::from(timer_id),
                            duration_in_clocks: timer_duration(update),
                        });
                    }
                }
                if let Some(asserted) = self.take_irq_update() {
                    events.push(SignalEvent::UpdateIrq { asserted });
                }
                events
            }
        }
    };
}

impl_take_signals!(Ym2203);
impl_take_signals!(Ym2608);
impl_take_signals!(Ym3526);
impl_take_signals!(Y8950);
impl_take_signals!(Ym3812);
impl_take_signals!(Ymf262);
impl_take_signals!(Ym2151);
