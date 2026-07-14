//! Host-side tracing bridge for automation clients.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fmt,
    num::NonZeroUsize,
    rc::Rc,
};

use common::{OwnedTraceEvent, TraceContext, TraceEvent, TraceEventKey, TraceInterest, TraceSink};

/// Default number of owned events retained by an application trace queue.
pub const DEFAULT_TRACE_QUEUE_EVENT_CAPACITY: usize = 16_384;
/// Default number of variable payload bytes retained by a trace queue.
pub const DEFAULT_TRACE_QUEUE_BYTE_CAPACITY: usize = 8 * 1024 * 1024;
/// Default maximum variable payload bytes accepted from one trace event.
pub const DEFAULT_TRACE_EVENT_PAYLOAD_CAPACITY: usize = 1024 * 1024;

/// Action selected by a trace matcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceDecision {
    /// Discard the event without allocating an owned payload.
    Ignore,
    /// Add the event to the owned queue.
    Record,
    /// Add the event and yield at the end of the current instruction.
    RecordAndYield,
}

/// Rust-side event filter used before an event is copied into the queue.
///
/// A matcher may use its [`TraceHandle`] while deciding, including replacing
/// or clearing itself. It must not recursively run the emulator.
pub trait TraceMatcher {
    /// Selects how the sink handles one borrowed event.
    fn decide(&mut self, context: TraceContext, event: TraceEvent<'_>) -> TraceDecision;
}

impl<F> TraceMatcher for F
where
    F: for<'a> FnMut(TraceContext, TraceEvent<'a>) -> TraceDecision,
{
    fn decide(&mut self, context: TraceContext, event: TraceEvent<'_>) -> TraceDecision {
        self(context, event)
    }
}

struct IgnoreAll;

impl TraceMatcher for IgnoreAll {
    fn decide(&mut self, _context: TraceContext, _event: TraceEvent<'_>) -> TraceDecision {
        TraceDecision::Ignore
    }
}

/// Hard limits for an application trace queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceLimits {
    /// Maximum number of retained events.
    pub event_capacity: NonZeroUsize,
    /// Maximum total bytes copied from text and byte fields.
    pub byte_capacity: NonZeroUsize,
    /// Maximum bytes copied from one event.
    pub event_payload_capacity: NonZeroUsize,
}

impl Default for TraceLimits {
    fn default() -> Self {
        Self {
            event_capacity: NonZeroUsize::new(DEFAULT_TRACE_QUEUE_EVENT_CAPACITY).unwrap(),
            byte_capacity: NonZeroUsize::new(DEFAULT_TRACE_QUEUE_BYTE_CAPACITY).unwrap(),
            event_payload_capacity: NonZeroUsize::new(DEFAULT_TRACE_EVENT_PAYLOAD_CAPACITY)
                .unwrap(),
        }
    }
}

/// Sticky failure that stops trace collection and requests a machine yield.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceFailure {
    /// The bounded event or payload-byte queue was exhausted.
    QueueOverflow {
        /// Maximum retained events.
        event_capacity: NonZeroUsize,
        /// Maximum retained variable payload bytes.
        byte_capacity: NonZeroUsize,
    },
    /// One event exceeded the per-event payload bound.
    EventPayloadTooLarge {
        /// Maximum variable payload bytes accepted for one event.
        capacity: NonZeroUsize,
    },
    /// The event sequence counter could not advance.
    SequenceExhausted,
}

impl fmt::Display for TraceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueOverflow {
                event_capacity,
                byte_capacity,
            } => write!(
                formatter,
                "trace queue limits of {event_capacity} events and {byte_capacity} payload bytes were exceeded"
            ),
            Self::EventPayloadTooLarge { capacity } => write!(
                formatter,
                "trace event exceeded the payload limit of {capacity} bytes"
            ),
            Self::SequenceExhausted => write!(formatter, "trace event sequence was exhausted"),
        }
    }
}

impl std::error::Error for TraceFailure {}

/// An owned trace event with automation epoch and ordering metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationTraceEnvelope {
    /// Event schema version.
    pub schema_version: u16,
    /// Monotonic sequence assigned in observation order.
    pub sequence: u64,
    /// Automation reconstruction epoch active when the event was recorded.
    pub epoch: u64,
    /// Origin and timestamp of the event.
    pub context: TraceContext,
    /// Owned event payload.
    pub event: OwnedTraceEvent,
}

struct TraceState {
    matcher: Rc<RefCell<Box<dyn TraceMatcher>>>,
    queue: VecDeque<ApplicationTraceEnvelope>,
    limits: TraceLimits,
    queued_payload_bytes: usize,
    next_sequence: u64,
    epoch: u64,
    matcher_yield_requested: bool,
    failure: Option<TraceFailure>,
}

impl TraceState {
    fn latch_failure(&mut self, failure: TraceFailure) {
        if self.failure.is_none() {
            self.failure = Some(failure);
        }
    }

    fn record(&mut self, context: TraceContext, event: TraceEvent<'_>, decision: TraceDecision) {
        if decision == TraceDecision::Ignore || self.failure.is_some() {
            return;
        }

        let Some(payload_bytes) = event.owned_payload_bytes() else {
            self.latch_failure(TraceFailure::EventPayloadTooLarge {
                capacity: self.limits.event_payload_capacity,
            });
            return;
        };
        if payload_bytes > self.limits.event_payload_capacity.get() {
            self.latch_failure(TraceFailure::EventPayloadTooLarge {
                capacity: self.limits.event_payload_capacity,
            });
            return;
        }
        let Some(next_payload_bytes) = self.queued_payload_bytes.checked_add(payload_bytes) else {
            self.latch_failure(TraceFailure::QueueOverflow {
                event_capacity: self.limits.event_capacity,
                byte_capacity: self.limits.byte_capacity,
            });
            return;
        };
        if self.queue.len() >= self.limits.event_capacity.get()
            || next_payload_bytes > self.limits.byte_capacity.get()
        {
            self.latch_failure(TraceFailure::QueueOverflow {
                event_capacity: self.limits.event_capacity,
                byte_capacity: self.limits.byte_capacity,
            });
            return;
        }
        let Some(next_sequence) = self.next_sequence.checked_add(1) else {
            self.latch_failure(TraceFailure::SequenceExhausted);
            return;
        };

        self.queue.push_back(ApplicationTraceEnvelope {
            schema_version: common::TRACE_SCHEMA_VERSION,
            sequence: self.next_sequence,
            epoch: self.epoch,
            context,
            event: OwnedTraceEvent::from(event),
        });
        self.next_sequence = next_sequence;
        self.queued_payload_bytes = next_payload_bytes;
        if decision == TraceDecision::RecordAndYield {
            self.matcher_yield_requested = true;
        }
    }

    fn yield_requested(&self) -> bool {
        self.matcher_yield_requested || self.failure.is_some()
    }
}

/// Application-facing trace sink for explicitly traced machines.
#[derive(Clone)]
pub struct ApplicationTraceSink {
    state: Rc<RefCell<TraceState>>,
    interest: Rc<Cell<TraceInterest>>,
}

impl ApplicationTraceSink {
    /// Creates a sink and its external control handle.
    pub fn new(limits: TraceLimits) -> (Self, TraceHandle) {
        let state = Rc::new(RefCell::new(TraceState {
            matcher: Rc::new(RefCell::new(Box::new(IgnoreAll))),
            queue: VecDeque::new(),
            limits,
            queued_payload_bytes: 0,
            next_sequence: 0,
            epoch: 0,
            matcher_yield_requested: false,
            failure: None,
        }));
        let interest = Rc::new(Cell::new(TraceInterest::NONE));
        (
            Self {
                state: Rc::clone(&state),
                interest: Rc::clone(&interest),
            },
            TraceHandle { state, interest },
        )
    }

    fn record(&mut self, context: TraceContext, event: TraceEvent<'_>) {
        let matcher = {
            let state = self.state.borrow();
            if state.failure.is_some() {
                return;
            }
            Rc::clone(&state.matcher)
        };
        let decision = matcher.borrow_mut().decide(context, event);
        self.state.borrow_mut().record(context, event, decision);
    }
}

impl Default for ApplicationTraceSink {
    fn default() -> Self {
        Self::new(TraceLimits::default()).0
    }
}

impl TraceSink for ApplicationTraceSink {
    fn interested(&self, key: TraceEventKey) -> bool {
        self.interest.get().contains(key.class())
    }

    fn trace(&mut self, context: TraceContext, event: TraceEvent<'_>) {
        if self.interested(event.key()) {
            self.record(context, event);
        }
    }

    fn yield_requested(&self) -> bool {
        self.state.borrow().yield_requested()
    }
}

/// External control and event-consumption handle for an application trace sink.
#[derive(Clone)]
pub struct TraceHandle {
    state: Rc<RefCell<TraceState>>,
    interest: Rc<Cell<TraceInterest>>,
}

impl TraceHandle {
    /// Returns whether any event class is currently collected.
    pub fn is_active(&self) -> bool {
        !self.interest.get().is_empty()
    }

    /// Starts a new trace and clears all buffered state.
    pub fn start<M>(&self, matcher: M, interest: TraceInterest)
    where
        M: TraceMatcher + 'static,
    {
        let mut state = self.state.borrow_mut();
        state.matcher = Rc::new(RefCell::new(Box::new(matcher)));
        state.queue.clear();
        state.queued_payload_bytes = 0;
        state.matcher_yield_requested = false;
        state.failure = None;
        self.interest.set(interest);
    }

    /// Installs a matcher without changing buffered state.
    pub fn set_matcher<M>(&self, matcher: M)
    where
        M: TraceMatcher + 'static,
    {
        self.set_matcher_with_interest(matcher, TraceInterest::ALL);
    }

    /// Installs a matcher with explicit event-class interest.
    pub fn set_matcher_with_interest<M>(&self, matcher: M, interest: TraceInterest)
    where
        M: TraceMatcher + 'static,
    {
        self.state.borrow_mut().matcher = Rc::new(RefCell::new(Box::new(matcher)));
        self.interest.set(interest);
    }

    /// Stops collection without discarding buffered events.
    pub fn stop(&self) {
        let mut state = self.state.borrow_mut();
        state.matcher = Rc::new(RefCell::new(Box::new(IgnoreAll)));
        state.matcher_yield_requested = false;
        self.interest.set(TraceInterest::NONE);
    }

    /// Restores the default matcher that ignores every event.
    pub fn clear_matcher(&self) {
        self.stop();
    }

    /// Sets the automation epoch for subsequently recorded events.
    pub fn set_epoch(&self, epoch: u64) {
        self.state.borrow_mut().epoch = epoch;
    }

    /// Returns the current automation epoch.
    pub fn epoch(&self) -> u64 {
        self.state.borrow().epoch
    }

    /// Drains all queued events in sequence order.
    pub fn drain(&self) -> Vec<ApplicationTraceEnvelope> {
        let mut state = self.state.borrow_mut();
        state.queued_payload_bytes = 0;
        state.queue.drain(..).collect()
    }

    /// Returns whether the sink requested an instruction-boundary yield.
    pub fn yield_requested(&self) -> bool {
        self.state.borrow().yield_requested()
    }

    /// Returns the sticky trace failure without acknowledging it.
    pub fn failure(&self) -> Option<TraceFailure> {
        self.state.borrow().failure
    }

    /// Takes and acknowledges a sticky trace failure.
    pub fn take_failure(&self) -> Option<TraceFailure> {
        self.state.borrow_mut().failure.take()
    }

    /// Clears a matcher yield after any trace failure has been acknowledged.
    pub fn resume(&self) -> Result<(), TraceFailure> {
        let mut state = self.state.borrow_mut();
        if let Some(failure) = state.failure {
            return Err(failure);
        }
        state.matcher_yield_requested = false;
        Ok(())
    }

    /// Returns the number of queued events.
    pub fn queued_len(&self) -> usize {
        self.state.borrow().queue.len()
    }

    /// Returns the queued variable payload bytes.
    pub fn queued_payload_bytes(&self) -> usize {
        self.state.borrow().queued_payload_bytes
    }
}

#[cfg(test)]
mod tests {
    use common::{
        TraceAccessKind, TraceAccessWidth, TraceAddressSpace, TraceDeviceEvent, TraceEventClass,
        TraceField, TraceValue,
    };

    use super::*;

    fn access(address: u64) -> TraceEvent<'static> {
        TraceEvent::access(
            TraceAddressSpace::MAIN_MEMORY,
            TraceAccessKind::Read,
            address,
            TraceAccessWidth::Byte,
            Some(address),
            true,
        )
    }

    fn context(cycle: u64) -> TraceContext {
        TraceContext::main_cpu(cycle, Some(1))
    }

    fn limits(events: usize, bytes: usize, event_bytes: usize) -> TraceLimits {
        TraceLimits {
            event_capacity: NonZeroUsize::new(events).unwrap(),
            byte_capacity: NonZeroUsize::new(bytes).unwrap(),
            event_payload_capacity: NonZeroUsize::new(event_bytes).unwrap(),
        }
    }

    #[test]
    fn inactive_sink_rejects_interest_without_queueing() {
        let (mut sink, handle) = ApplicationTraceSink::new(limits(1, 1, 1));
        assert_eq!(sink.state.borrow().queue.capacity(), 0);
        assert!(!handle.is_active());
        assert!(!sink.interested(access(1).key()));
        sink.trace(context(1), access(1));
        assert_eq!(handle.queued_len(), 0);
        assert!(!handle.yield_requested());
    }

    #[test]
    fn records_epoch_order_and_yields_on_match() {
        let (mut sink, handle) = ApplicationTraceSink::new(limits(2, 1, 1));
        handle.set_epoch(7);
        handle.set_matcher(|_: TraceContext, event: TraceEvent<'_>| match event {
            TraceEvent::Access(access) if access.address == 2 => TraceDecision::RecordAndYield,
            _ => TraceDecision::Record,
        });
        sink.trace(context(1), access(1));
        sink.trace(context(2), access(2));
        assert!(handle.yield_requested());
        let events = handle.drain();
        assert_eq!(events[0].sequence, 0);
        assert_eq!(events[1].sequence, 1);
        assert!(events.iter().all(|event| event.epoch == 7));
        handle.resume().unwrap();
        assert!(!handle.yield_requested());
    }

    #[test]
    fn event_overflow_is_sticky_and_requires_acknowledgement() {
        let (mut sink, handle) = ApplicationTraceSink::new(limits(1, 1, 1));
        handle.set_matcher(|_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record);
        sink.trace(context(1), access(1));
        sink.trace(context(2), access(2));
        assert!(handle.yield_requested());
        assert!(matches!(
            handle.failure(),
            Some(TraceFailure::QueueOverflow { .. })
        ));
        assert!(handle.resume().is_err());
        assert_eq!(handle.drain().len(), 1);
        handle.take_failure().unwrap();
        handle.resume().unwrap();
    }

    #[test]
    fn payload_limits_bound_owned_bytes() {
        let (mut sink, handle) = ApplicationTraceSink::new(limits(2, 3, 3));
        handle.set_matcher_with_interest(
            |_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record,
            TraceInterest::only(TraceEventClass::Device),
        );
        let fields = [TraceField {
            name: "bytes",
            value: TraceValue::Bytes(&[1, 2]),
        }];
        let event = TraceEvent::Device(TraceDeviceEvent {
            device: "test.device",
            action: "data",
            fields: &fields,
        });
        sink.trace(context(1), event);
        sink.trace(context(2), event);
        assert_eq!(handle.queued_len(), 1);
        assert_eq!(handle.queued_payload_bytes(), 2);
        assert!(matches!(
            handle.failure(),
            Some(TraceFailure::QueueOverflow { .. })
        ));
    }

    #[test]
    fn starting_a_trace_clears_buffer_failure_and_yield() {
        let (mut sink, handle) = ApplicationTraceSink::new(limits(1, 1, 1));
        handle.set_matcher(|_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record);
        sink.trace(context(1), access(1));
        sink.trace(context(2), access(2));
        handle.start(
            |_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record,
            TraceInterest::only(TraceEventClass::Access),
        );
        assert!(handle.is_active());
        assert_eq!(handle.queued_len(), 0);
        assert_eq!(handle.queued_payload_bytes(), 0);
        assert_eq!(handle.failure(), None);
        assert!(!handle.yield_requested());
    }

    #[test]
    fn start_keeps_sequence_monotonic_and_stop_preserves_events() {
        let (mut sink, handle) = ApplicationTraceSink::new(limits(2, 1, 1));
        handle.start(
            |_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record,
            TraceInterest::only(TraceEventClass::Access),
        );
        sink.trace(context(1), access(1));
        assert_eq!(handle.drain()[0].sequence, 0);

        handle.start(
            |_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record,
            TraceInterest::only(TraceEventClass::Access),
        );
        sink.trace(context(2), access(2));
        handle.stop();
        assert!(!handle.is_active());
        let events = handle.drain();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sequence, 1);
    }

    #[test]
    fn oversized_event_is_rejected_without_partial_storage() {
        let (mut sink, handle) = ApplicationTraceSink::new(limits(2, 8, 1));
        handle.set_matcher_with_interest(
            |_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record,
            TraceInterest::only(TraceEventClass::Device),
        );
        let fields = [TraceField {
            name: "bytes",
            value: TraceValue::Bytes(&[1, 2]),
        }];
        sink.trace(
            context(1),
            TraceEvent::Device(TraceDeviceEvent {
                device: "test.device",
                action: "data",
                fields: &fields,
            }),
        );
        assert_eq!(handle.queued_len(), 0);
        assert_eq!(handle.queued_payload_bytes(), 0);
        assert!(matches!(
            handle.failure(),
            Some(TraceFailure::EventPayloadTooLarge { .. })
        ));
    }

    #[test]
    fn matcher_can_replace_and_clear_itself() {
        let (mut sink, handle) = ApplicationTraceSink::new(limits(2, 1, 1));
        let matcher_handle = handle.clone();
        handle.set_matcher(move |_: TraceContext, _: TraceEvent<'_>| {
            matcher_handle
                .set_matcher(|_: TraceContext, _: TraceEvent<'_>| TraceDecision::RecordAndYield);
            TraceDecision::Ignore
        });
        sink.trace(context(1), access(1));
        sink.trace(context(2), access(2));
        assert_eq!(handle.drain().len(), 1);
        assert!(handle.yield_requested());

        handle.clear_matcher();
        sink.trace(context(3), access(3));
        assert_eq!(handle.queued_len(), 0);
        assert!(!handle.yield_requested());
    }
}
