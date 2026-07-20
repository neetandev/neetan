//! Bounded machine execution: tick and frame targets driven in sub-chunks so
//! cancellation and total-session budgets are checked at fine granularity.

use common::{RunOutcome, RunRequest, RunTarget, StopReason};

use super::{AutomationSession, INPUT_DRAIN_INTERVAL_TICKS, PUBLIC_INTEGER_MAX, RunError};

/// Frames advanced per machine call so cancellation and budgets are checked at
/// presentation granularity.
const FRAME_SUBCHUNK: u64 = 1;

/// Ticks advanced per machine call for a tick target, bounding the work done
/// between cancellation and budget checks.
const TICK_SUBCHUNK: u64 = 1_000_000;

impl AutomationSession {
    /// Advances the machine through one bounded run request.
    ///
    /// The request is issued to the machine in bounded sub-chunks so the session
    /// checks cancellation and total-session budgets at frame or tick
    /// granularity. Absolute session-frame targets are lowered to epoch frames.
    pub fn run(
        &mut self,
        target: RunTarget,
        max_ticks: u64,
        audio_drain_interval_ticks: u64,
    ) -> Result<RunOutcome, RunError> {
        if self.active.is_none() {
            return Err(RunError::NoMachine);
        }
        self.guard_request_range(target, max_ticks)?;

        let start = self.timeline();
        let stop_reason = match target {
            RunTarget::Ticks(count) => {
                self.run_ticks(start.epoch_ticks, count, audio_drain_interval_ticks)
            }
            RunTarget::Frames(count) => {
                self.run_frames(count, max_ticks, audio_drain_interval_ticks)
            }
            RunTarget::UntilSessionFrame(absolute) => {
                let count = u64::try_from(absolute.saturating_sub(start.session_frames))
                    .unwrap_or(u64::MAX);
                self.run_frames(count, max_ticks, audio_drain_interval_ticks)
            }
        };

        // A continuous trace collector that overflowed during this run is a
        // yield reason the session consumes here: the active run raises
        // neetan/trace-overflow rather than reporting an ordinary outcome.
        if self.consume_trace_overflow() {
            return Err(RunError::TraceOverflow);
        }

        let end = self.timeline();
        let ticks =
            u64::try_from(end.epoch_ticks.saturating_sub(start.epoch_ticks)).unwrap_or(u64::MAX);
        let frames =
            u64::try_from(end.epoch_frames.saturating_sub(start.epoch_frames)).unwrap_or(u64::MAX);
        let overshoot_ticks = match target {
            RunTarget::Ticks(count) => u64::try_from(
                end.epoch_ticks
                    .saturating_sub(start.epoch_ticks.saturating_add(count as u128)),
            )
            .unwrap_or(u64::MAX),
            RunTarget::Frames(_) | RunTarget::UntilSessionFrame(_) => 0,
        };

        // An advancing run may have let the guest write to a mounted disk. There
        // is no machine-side dirty query, so mark every writable mount as
        // possibly written since the last flush.
        if ticks > 0 || frames > 0 {
            for mount in self
                .active
                .as_mut()
                .expect("machine present")
                .mounts
                .values_mut()
            {
                if mount.kind.writable() {
                    mount.dirty = true;
                }
            }
        }

        Ok(RunOutcome {
            stop_reason,
            ticks,
            frames,
            overshoot_ticks,
            timeline: end,
        })
    }

    /// Advances the machine by `count` epoch ticks, bounded by the same count.
    pub fn advance_ticks(&mut self, count: u64) -> Result<RunOutcome, RunError> {
        self.run(RunTarget::Ticks(count), count, INPUT_DRAIN_INTERVAL_TICKS)
    }

    /// Advances the machine by `count` presented frames or `max_ticks` ticks.
    pub fn advance_frames(&mut self, count: u64, max_ticks: u64) -> Result<RunOutcome, RunError> {
        self.run(
            RunTarget::Frames(count),
            max_ticks,
            INPUT_DRAIN_INTERVAL_TICKS,
        )
    }

    /// Advances the machine until absolute session `frame` or `max_ticks` ticks.
    pub fn advance_until_frame(
        &mut self,
        frame: u128,
        max_ticks: u64,
    ) -> Result<RunOutcome, RunError> {
        self.run(
            RunTarget::UntilSessionFrame(frame),
            max_ticks,
            INPUT_DRAIN_INTERVAL_TICKS,
        )
    }

    /// Rejects a request whose resulting public value would overflow the signed
    /// 128-bit range before any machine execution.
    fn guard_request_range(&self, target: RunTarget, max_ticks: u64) -> Result<(), RunError> {
        let timeline = self.timeline();
        let tick_ceiling = timeline
            .session_ticks
            .checked_add(u128::from(max_ticks))
            .ok_or(RunError::Range)?;
        if tick_ceiling > PUBLIC_INTEGER_MAX {
            return Err(RunError::Range);
        }
        let projected_frames = match target {
            RunTarget::Ticks(_) => timeline.session_frames,
            RunTarget::Frames(count) => timeline
                .session_frames
                .checked_add(u128::from(count))
                .ok_or(RunError::Range)?,
            RunTarget::UntilSessionFrame(absolute) => absolute,
        };
        if projected_frames > PUBLIC_INTEGER_MAX {
            return Err(RunError::Range);
        }
        Ok(())
    }

    /// Drives a tick target in bounded sub-chunks, returning the stop reason.
    fn run_ticks(&mut self, start_ticks: u128, count: u64, drain_interval: u64) -> StopReason {
        let final_target = start_ticks.saturating_add(u128::from(count));
        loop {
            let current = self.timeline().epoch_ticks;
            if current >= final_target {
                return StopReason::TargetReached;
            }
            if self.is_stopped() {
                return StopReason::Cancelled;
            }
            if self.tick_budget_exhausted() {
                return StopReason::CounterExhausted;
            }
            let chunk = u64::try_from((final_target - current).min(u128::from(TICK_SUBCHUNK)))
                .unwrap_or(TICK_SUBCHUNK);
            let request = RunRequest {
                target: RunTarget::Ticks(chunk),
                max_ticks: chunk,
                audio_drain_interval_ticks: drain_interval,
            };
            let outcome = self
                .active
                .as_mut()
                .expect("machine present")
                .machine
                .run_automation(request);
            self.consume_budget(&outcome);
            if outcome.stop_reason == StopReason::GuestShutdown {
                return StopReason::GuestShutdown;
            }
            if outcome.ticks == 0 && self.timeline().epoch_ticks == current {
                return StopReason::TargetReached;
            }
        }
    }

    /// Drives a frame target in single-frame sub-chunks, returning the stop
    /// reason. The whole request shares one tick fallback bound.
    fn run_frames(&mut self, count: u64, max_ticks: u64, drain_interval: u64) -> StopReason {
        let mut presented = 0u64;
        let mut remaining_ticks = max_ticks;
        while presented < count {
            if self.is_stopped() {
                return StopReason::Cancelled;
            }
            if self.tick_budget_exhausted() || self.frame_budget_exhausted() {
                return StopReason::CounterExhausted;
            }
            if remaining_ticks == 0 {
                return StopReason::TickLimit;
            }
            let request = RunRequest {
                target: RunTarget::Frames(FRAME_SUBCHUNK.min(count - presented)),
                max_ticks: remaining_ticks,
                audio_drain_interval_ticks: drain_interval,
            };
            let outcome = self
                .active
                .as_mut()
                .expect("machine present")
                .machine
                .run_automation(request);
            self.consume_budget(&outcome);
            remaining_ticks = remaining_ticks.saturating_sub(outcome.ticks);
            presented = presented.saturating_add(outcome.frames);
            match outcome.stop_reason {
                StopReason::TargetReached => {}
                other => return other,
            }
        }
        StopReason::TargetReached
    }

    /// Decrements the tick and frame budgets by a completed sub-chunk.
    pub(super) fn consume_budget(&mut self, outcome: &RunOutcome) {
        if let Some(remaining) = self.budgets.ticks.as_mut() {
            *remaining = remaining.saturating_sub(u128::from(outcome.ticks));
        }
        if let Some(remaining) = self.budgets.frames.as_mut() {
            *remaining = remaining.saturating_sub(u128::from(outcome.frames));
        }
    }

    /// Returns whether the total tick budget is exhausted.
    pub(super) fn tick_budget_exhausted(&self) -> bool {
        matches!(self.budgets.ticks, Some(0))
    }

    /// Returns whether the total frame budget is exhausted.
    pub(super) fn frame_budget_exhausted(&self) -> bool {
        matches!(self.budgets.frames, Some(0))
    }
}
