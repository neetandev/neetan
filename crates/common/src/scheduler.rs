//! Shared authoritative event scheduler state.

use alloc::vec;

use save_state::{StateValidationError, ValidateState};

use crate::StackVec;

/// One due scheduler slot and its exact deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledEventIndex {
    /// Slot index whose event became due.
    pub index: usize,
    /// Absolute emulated cycle at which the event fires.
    pub fire_cycle: u64,
}

save_state::runtime_state! {
    /// Authoritative deadlines for a fixed set of event kinds.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SchedulerState {
        /// Fire cycles indexed by the machine's frozen event priority order.
        pub fire_cycles: alloc::vec::Vec<Option<u64>>,
    }
}

impl SchedulerState {
    /// Creates an empty scheduler state with one slot per event kind.
    pub fn new(event_count: usize) -> Self {
        Self {
            fire_cycles: vec![None; event_count],
        }
    }

    /// Schedules or replaces the event at `index`.
    pub fn schedule(&mut self, index: usize, fire_cycle: u64) {
        self.fire_cycles[index] = Some(fire_cycle);
    }

    /// Cancels the event at `index`.
    pub fn cancel(&mut self, index: usize) {
        self.fire_cycles[index] = None;
    }

    /// Returns whether the event at `index` is scheduled.
    pub fn is_scheduled(&self, index: usize) -> bool {
        self.fire_cycles[index].is_some()
    }

    /// Returns the earliest pending deadline.
    pub fn next_event_cycle(&self) -> Option<u64> {
        self.fire_cycles.iter().flatten().copied().min()
    }

    /// Removes due slots ordered by deadline and frozen slot priority.
    pub fn pop_due<const CAPACITY: usize>(
        &mut self,
        current_cycle: u64,
    ) -> StackVec<ScheduledEventIndex, CAPACITY> {
        assert!(
            self.fire_cycles.len() <= CAPACITY,
            "scheduler output capacity is smaller than the event count"
        );

        let mut due = StackVec::new();
        for (index, slot) in self.fire_cycles.iter_mut().enumerate() {
            if let Some(fire_cycle) = *slot
                && fire_cycle <= current_cycle
            {
                due.push(ScheduledEventIndex { index, fire_cycle });
                *slot = None;
            }
        }
        due.sort_by_key(|event: &ScheduledEventIndex| (event.fire_cycle, event.index));
        due
    }
}

impl ValidateState<usize> for SchedulerState {
    fn validate_state(&self, expected_event_count: &usize) -> Result<(), StateValidationError> {
        if self.fire_cycles.len() != *expected_event_count {
            return Err(StateValidationError::new(alloc::format!(
                "scheduler event count differs: expected {}, decoded {}",
                expected_event_count,
                self.fire_cycles.len()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_after_state_replacement_is_exact() {
        fn assert_runtime_state<State: save_state::RuntimeState>() {}
        assert_runtime_state::<SchedulerState>();

        let mut scheduler = SchedulerState::new(4);
        scheduler.schedule(3, 80);
        scheduler.schedule(2, 40);
        scheduler.schedule(0, 40);
        let captured = scheduler.clone();

        let first = scheduler.pop_due::<4>(80);
        scheduler = captured;
        let replay = scheduler.pop_due::<4>(80);

        assert_eq!(&*first, &*replay);
        assert_eq!(first[0].index, 0);
        assert_eq!(first[1].index, 2);
        assert_eq!(first[2].index, 3);

        let bytes = save_state::encode_runtime_state(&scheduler);
        let decoded: SchedulerState = save_state::decode_runtime_state(&bytes, 4).unwrap();
        assert_eq!(decoded, scheduler);
    }

    #[test]
    fn decoded_event_count_is_validated() {
        let state = SchedulerState::new(3);
        assert!(state.validate_state(&3).is_ok());
        assert!(state.validate_state(&4).is_err());
    }
}
