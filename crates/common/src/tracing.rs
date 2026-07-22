//! Host-side tracing bridge for automation clients.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fmt,
    num::NonZeroUsize,
    rc::Rc,
};

use crate::{
    OwnedTraceEvent, ProcessorSnapshot, TraceContext, TraceEvent, TraceEventKey, TraceInterest,
    TraceSink,
};

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
    /// Processor snapshot captured when the event was created, when armed.
    pub snapshot: Option<ProcessorSnapshot>,
}

/// Progress of an armed ring capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingCaptureStatus {
    /// No ring capture is armed.
    Idle,
    /// Armed, waiting for the trigger event.
    Armed,
    /// Triggered, collecting post-trigger context.
    Triggered,
    /// The requested post-trigger context is complete.
    Complete,
}

/// The retained events of a disarmed ring capture.
pub struct RingCaptureResult {
    /// Retained envelopes in sequence order, trigger event included.
    pub events: Vec<ApplicationTraceEnvelope>,
    /// Whether the trigger event was seen.
    pub triggered: bool,
    /// Whether the post-trigger context completed.
    pub complete: bool,
    /// Index of the trigger event within `events`, when triggered.
    pub trigger_index: Option<usize>,
}

/// A bounded before-and-after capture window around a trigger event.
struct RingCaptureState {
    capture: Rc<RefCell<Box<dyn TraceMatcher>>>,
    trigger: Rc<RefCell<Box<dyn TraceMatcher>>>,
    before: usize,
    after: usize,
    pre: VecDeque<(ApplicationTraceEnvelope, usize)>,
    trigger_event: Option<ApplicationTraceEnvelope>,
    post: Vec<ApplicationTraceEnvelope>,
    retained_payload_bytes: usize,
    complete: bool,
}

/// One device-interest entry: a device identifier and an optional action.
///
/// An entry with no action covers every action of the device; an entry with an
/// action covers only that action, so sibling high-volume actions of the same
/// device are never built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInterest {
    /// Stable device identifier.
    pub device: String,
    /// Stable action identifier, or `None` for every action of the device.
    pub action: Option<String>,
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
    presentation_yield_target: Option<u64>,
    presentation_boundary_reached: bool,
    armed_snapshot: Option<&'static str>,
    pending_snapshot: Option<ProcessorSnapshot>,
    device_interest: Option<Vec<DeviceInterest>>,
    ring: Option<RingCaptureState>,
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

        let Some(event_payload_bytes) = event.owned_payload_bytes() else {
            self.latch_failure(TraceFailure::EventPayloadTooLarge {
                capacity: self.limits.event_payload_capacity,
            });
            return;
        };
        let snapshot_bytes = self.pending_snapshot.as_ref().map_or(0, |snapshot| {
            snapshot
                .registers
                .len()
                .saturating_mul(core::mem::size_of::<u128>())
        });
        let Some(payload_bytes) = event_payload_bytes.checked_add(snapshot_bytes) else {
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
            schema_version: crate::TRACE_SCHEMA_VERSION,
            sequence: self.next_sequence,
            epoch: self.epoch,
            context,
            event: OwnedTraceEvent::from(event),
            snapshot: self.pending_snapshot.clone(),
        });
        self.next_sequence = next_sequence;
        self.queued_payload_bytes = next_payload_bytes;
        if decision == TraceDecision::RecordAndYield {
            self.matcher_yield_requested = true;
        }
    }

    fn yield_requested(&self) -> bool {
        self.matcher_yield_requested
            || self.failure.is_some()
            || self.presentation_boundary_reached
            || self.ring.as_ref().is_some_and(|ring| ring.complete)
    }

    /// Stores one matched event into the armed ring capture.
    ///
    /// Retention is decided before the owned payload is allocated, so events
    /// that would fall out of an empty pre-trigger window cost nothing. The
    /// retained window is charged against the queue byte capacity, so a ring
    /// capture honours the same advertised limits as the continuous queue.
    fn ring_record(&mut self, context: TraceContext, event: TraceEvent<'_>, is_trigger: bool) {
        let retain = match &self.ring {
            None => return,
            Some(ring) if ring.complete => return,
            Some(ring) => is_trigger || ring.trigger_event.is_some() || ring.before > 0,
        };
        if !retain {
            return;
        }

        let Some(event_payload_bytes) = event.owned_payload_bytes() else {
            self.latch_failure(TraceFailure::EventPayloadTooLarge {
                capacity: self.limits.event_payload_capacity,
            });
            return;
        };
        let snapshot_bytes = self.pending_snapshot.as_ref().map_or(0, |snapshot| {
            snapshot
                .registers
                .len()
                .saturating_mul(core::mem::size_of::<u128>())
        });
        let payload_bytes = event_payload_bytes.saturating_add(snapshot_bytes);
        if payload_bytes > self.limits.event_payload_capacity.get() {
            self.latch_failure(TraceFailure::EventPayloadTooLarge {
                capacity: self.limits.event_payload_capacity,
            });
            return;
        }
        let Some(next_sequence) = self.next_sequence.checked_add(1) else {
            self.latch_failure(TraceFailure::SequenceExhausted);
            return;
        };

        let over_byte_capacity = {
            let ring = self.ring.as_mut().expect("ring capture armed");
            if ring.trigger_event.is_none() && !is_trigger {
                // Evict the oldest pre-window event first, so the sliding
                // window frees its bytes before the new event is charged.
                while ring.pre.len() >= ring.before.max(1) {
                    let Some((_, evicted_bytes)) = ring.pre.pop_front() else {
                        break;
                    };
                    ring.retained_payload_bytes =
                        ring.retained_payload_bytes.saturating_sub(evicted_bytes);
                }
            }
            ring.retained_payload_bytes.saturating_add(payload_bytes)
                > self.limits.byte_capacity.get()
        };
        if over_byte_capacity {
            self.latch_failure(TraceFailure::QueueOverflow {
                event_capacity: self.limits.event_capacity,
                byte_capacity: self.limits.byte_capacity,
            });
            return;
        }

        let envelope = ApplicationTraceEnvelope {
            schema_version: crate::TRACE_SCHEMA_VERSION,
            sequence: self.next_sequence,
            epoch: self.epoch,
            context,
            event: OwnedTraceEvent::from(event),
            snapshot: self.pending_snapshot.clone(),
        };
        self.next_sequence = next_sequence;

        let ring = self.ring.as_mut().expect("ring capture armed");
        ring.retained_payload_bytes = ring.retained_payload_bytes.saturating_add(payload_bytes);
        if is_trigger {
            ring.trigger_event = Some(envelope);
            if ring.after == 0 {
                ring.complete = true;
            }
        } else if ring.trigger_event.is_none() {
            ring.pre.push_back((envelope, payload_bytes));
        } else {
            ring.post.push(envelope);
            if ring.post.len() >= ring.after {
                ring.complete = true;
            }
        }
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
            presentation_yield_target: None,
            presentation_boundary_reached: false,
            armed_snapshot: None,
            pending_snapshot: None,
            device_interest: None,
            ring: None,
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

    /// Feeds one event to the armed ring capture, when one is armed.
    ///
    /// The trigger and capture filters are evaluated on the borrowed event
    /// before any owned payload is allocated.
    fn ring_capture(&self, context: TraceContext, event: TraceEvent<'_>) {
        let (capture, trigger, triggered) = {
            let state = self.state.borrow();
            if state.failure.is_some() {
                return;
            }
            let Some(ring) = &state.ring else {
                return;
            };
            if ring.complete {
                return;
            }
            (
                Rc::clone(&ring.capture),
                Rc::clone(&ring.trigger),
                ring.trigger_event.is_some(),
            )
        };
        let is_trigger =
            !triggered && trigger.borrow_mut().decide(context, event) != TraceDecision::Ignore;
        let is_capture = capture.borrow_mut().decide(context, event) != TraceDecision::Ignore;
        if !is_trigger && !is_capture {
            return;
        }
        self.state
            .borrow_mut()
            .ring_record(context, event, is_trigger);
    }

    /// Arms an exact presentation-boundary stop at absolute epoch `target`.
    ///
    /// The next published frame whose number reaches `target` sets the boundary
    /// flag, so the machine run loop yields right after that scheduler batch.
    pub fn arm_presentation_yield(&self, target: u64) {
        let mut state = self.state.borrow_mut();
        state.presentation_yield_target = Some(target);
        state.presentation_boundary_reached = false;
    }

    /// Disarms any pending presentation-boundary stop.
    pub fn disarm_presentation_yield(&self) {
        let mut state = self.state.borrow_mut();
        state.presentation_yield_target = None;
        state.presentation_boundary_reached = false;
    }

    /// Returns whether the armed presentation boundary has been reached.
    pub fn presentation_boundary_reached(&self) -> bool {
        self.state.borrow().presentation_boundary_reached
    }
}

impl Default for ApplicationTraceSink {
    fn default() -> Self {
        Self::new(TraceLimits::default()).0
    }
}

impl TraceSink for ApplicationTraceSink {
    fn interested(&self, key: TraceEventKey) -> bool {
        if !self.interest.get().contains(key.class()) {
            return false;
        }
        if let TraceEventKey::Device { device, action } = key
            && let Some(entries) = &self.state.borrow().device_interest
        {
            return entries.iter().any(|entry| {
                entry.device == device
                    && entry
                        .action
                        .as_ref()
                        .is_none_or(|interested| interested == action)
            });
        }
        true
    }

    fn trace(&mut self, context: TraceContext, event: TraceEvent<'_>) {
        if let TraceEvent::Presentation(presentation) = event {
            let mut state = self.state.borrow_mut();
            if let Some(target) = state.presentation_yield_target
                && presentation.frame >= target
            {
                state.presentation_boundary_reached = true;
            }
        }
        if self.interested(event.key()) {
            self.record(context, event);
            self.ring_capture(context, event);
        }
    }

    fn yield_requested(&self) -> bool {
        self.state.borrow().yield_requested()
    }

    fn snapshot_request(&self) -> Option<&'static str> {
        self.state.borrow().armed_snapshot
    }

    fn set_pending_snapshot(&mut self, snapshot: ProcessorSnapshot) {
        self.state.borrow_mut().pending_snapshot = Some(snapshot);
    }

    fn clear_pending_snapshot(&mut self) {
        self.state.borrow_mut().pending_snapshot = None;
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
        state.armed_snapshot = None;
        state.pending_snapshot = None;
        state.device_interest = None;
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
        state.armed_snapshot = None;
        state.pending_snapshot = None;
        state.device_interest = None;
        state.ring = None;
        self.interest.set(TraceInterest::NONE);
    }

    /// Arms a bounded ring capture and clears all buffered state.
    ///
    /// Events matching `capture` are retained in a window of at most `before`
    /// events preceding the first `trigger` match and `after` events following
    /// it. Storage is bounded from this call on, and the sink requests a yield
    /// once the post-trigger context is complete.
    pub fn arm_ring_capture<C, T>(
        &self,
        capture: C,
        trigger: T,
        before: usize,
        after: usize,
        interest: TraceInterest,
    ) where
        C: TraceMatcher + 'static,
        T: TraceMatcher + 'static,
    {
        let mut state = self.state.borrow_mut();
        state.matcher = Rc::new(RefCell::new(Box::new(IgnoreAll)));
        state.queue.clear();
        state.queued_payload_bytes = 0;
        state.matcher_yield_requested = false;
        state.failure = None;
        state.armed_snapshot = None;
        state.pending_snapshot = None;
        state.device_interest = None;
        state.ring = Some(RingCaptureState {
            capture: Rc::new(RefCell::new(Box::new(capture))),
            trigger: Rc::new(RefCell::new(Box::new(trigger))),
            before,
            after,
            pre: VecDeque::new(),
            trigger_event: None,
            post: Vec::new(),
            retained_payload_bytes: 0,
            complete: false,
        });
        self.interest.set(interest);
    }

    /// Returns the progress of the armed ring capture.
    pub fn ring_status(&self) -> RingCaptureStatus {
        match &self.state.borrow().ring {
            None => RingCaptureStatus::Idle,
            Some(ring) if ring.complete => RingCaptureStatus::Complete,
            Some(ring) if ring.trigger_event.is_some() => RingCaptureStatus::Triggered,
            Some(_) => RingCaptureStatus::Armed,
        }
    }

    /// Disarms the ring capture and returns its retained events in order.
    pub fn take_ring_capture(&self) -> Option<RingCaptureResult> {
        let mut state = self.state.borrow_mut();
        let ring = state.ring.take()?;
        self.interest.set(TraceInterest::NONE);
        let triggered = ring.trigger_event.is_some();
        let complete = ring.complete;
        let mut events: Vec<ApplicationTraceEnvelope> =
            ring.pre.into_iter().map(|(envelope, _)| envelope).collect();
        let trigger_index = ring.trigger_event.map(|event| {
            events.push(event);
            events.len() - 1
        });
        events.extend(ring.post);
        Some(RingCaptureResult {
            events,
            triggered,
            complete,
            trigger_index,
        })
    }

    /// Restricts device-class interest to the given device and action entries.
    ///
    /// `None` keeps every device the interest classes cover. High-volume
    /// device events are skipped entirely at the emitter's interest check when
    /// their device, or their action within a listed device, is not listed.
    pub fn set_device_interest(&self, entries: Option<Vec<DeviceInterest>>) {
        self.state.borrow_mut().device_interest = entries;
    }

    /// Arms an atomic register snapshot for a processor at each recorded event.
    pub fn arm_snapshot(&self, processor: &'static str) {
        self.state.borrow_mut().armed_snapshot = Some(processor);
    }

    /// Returns a clone of every queued event without draining the queue.
    ///
    /// Used to persist the buffered trace to an artifact while leaving the events
    /// available for a later drain.
    pub fn snapshot_events(&self) -> Vec<ApplicationTraceEnvelope> {
        self.state.borrow().queue.iter().cloned().collect()
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

    /// Arms an exact presentation-boundary stop at absolute epoch `target`.
    pub fn arm_presentation_yield(&self, target: u64) {
        let mut state = self.state.borrow_mut();
        state.presentation_yield_target = Some(target);
        state.presentation_boundary_reached = false;
    }

    /// Disarms any pending presentation-boundary stop.
    pub fn disarm_presentation_yield(&self) {
        let mut state = self.state.borrow_mut();
        state.presentation_yield_target = None;
        state.presentation_boundary_reached = false;
    }

    /// Returns whether the armed presentation boundary has been reached.
    pub fn presentation_boundary_reached(&self) -> bool {
        self.state.borrow().presentation_boundary_reached
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
    use super::*;
    use crate::{
        TraceAccessKind, TraceAccessWidth, TraceAddressSpace, TraceDeviceEvent, TraceEventClass,
        TraceField, TraceValue,
    };

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

    fn device_event(device: &'static str) -> TraceEvent<'static> {
        TraceEvent::Device(TraceDeviceEvent {
            device,
            action: "data",
            fields: &[],
        })
    }

    #[test]
    fn device_interest_narrows_device_events() {
        let (mut sink, handle) = ApplicationTraceSink::new(limits(8, 64, 64));
        handle.set_matcher_with_interest(
            |_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record,
            TraceInterest::only(TraceEventClass::Device),
        );
        handle.set_device_interest(Some(vec![DeviceInterest {
            device: String::from("want.device"),
            action: None,
        }]));
        assert!(sink.interested(TraceEventKey::Device {
            device: "want.device",
            action: "data",
        }));
        assert!(!sink.interested(TraceEventKey::Device {
            device: "other.device",
            action: "data",
        }));
        sink.trace(context(1), device_event("want.device"));
        sink.trace(context(2), device_event("other.device"));
        assert_eq!(handle.queued_len(), 1);
        // Stopping restores interest in every device.
        handle.stop();
        handle.set_matcher_with_interest(
            |_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record,
            TraceInterest::only(TraceEventClass::Device),
        );
        assert!(sink.interested(TraceEventKey::Device {
            device: "other.device",
            action: "data",
        }));
    }

    #[test]
    fn device_interest_narrows_to_the_named_action() {
        let (mut sink, handle) = ApplicationTraceSink::new(limits(8, 64, 64));
        handle.set_matcher_with_interest(
            |_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record,
            TraceInterest::only(TraceEventClass::Device),
        );
        handle.set_device_interest(Some(vec![DeviceInterest {
            device: String::from("want.device"),
            action: Some(String::from("data")),
        }]));
        assert!(sink.interested(TraceEventKey::Device {
            device: "want.device",
            action: "data",
        }));
        // A sibling action of the same device is rejected at the interest
        // check, so its high-volume events are never built.
        assert!(!sink.interested(TraceEventKey::Device {
            device: "want.device",
            action: "other",
        }));
        sink.trace(context(1), device_event("want.device"));
        assert_eq!(handle.queued_len(), 1);
    }

    #[test]
    fn oversized_event_payload_failure_is_sticky() {
        let (mut sink, handle) = ApplicationTraceSink::new(limits(8, 64, 1));
        handle.set_matcher_with_interest(
            |_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record,
            TraceInterest::only(TraceEventClass::Device),
        );
        let fields = [TraceField {
            name: "bytes",
            value: TraceValue::Bytes(&[1, 2, 3]),
        }];
        let oversized = TraceEvent::Device(TraceDeviceEvent {
            device: "test.device",
            action: "data",
            fields: &fields,
        });
        sink.trace(context(1), oversized);
        assert!(matches!(
            handle.failure(),
            Some(TraceFailure::EventPayloadTooLarge { .. })
        ));
        // The failure is sticky: later events are not recorded over it.
        sink.trace(context(2), device_event("test.device"));
        assert_eq!(handle.queued_len(), 0);
        assert!(matches!(
            handle.failure(),
            Some(TraceFailure::EventPayloadTooLarge { .. })
        ));
        handle.take_failure().unwrap();
    }

    #[test]
    fn ring_capture_retains_before_and_after_counts() {
        let (mut sink, handle) = ApplicationTraceSink::new(limits(64, 1024, 64));
        handle.arm_ring_capture(
            |_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record,
            |_: TraceContext, event: TraceEvent<'_>| match event {
                TraceEvent::Access(access) if access.address == 10 => TraceDecision::RecordAndYield,
                _ => TraceDecision::Ignore,
            },
            3,
            2,
            TraceInterest::only(TraceEventClass::Access),
        );
        assert_eq!(handle.ring_status(), RingCaptureStatus::Armed);
        for address in 1..=9 {
            sink.trace(context(address), access(address));
        }
        assert_eq!(handle.ring_status(), RingCaptureStatus::Armed);
        sink.trace(context(10), access(10));
        assert_eq!(handle.ring_status(), RingCaptureStatus::Triggered);
        assert!(!handle.yield_requested());
        sink.trace(context(11), access(11));
        sink.trace(context(12), access(12));
        assert_eq!(handle.ring_status(), RingCaptureStatus::Complete);
        assert!(handle.yield_requested());
        // Later events are not retained once the capture is complete.
        sink.trace(context(13), access(13));

        let result = handle.take_ring_capture().unwrap();
        assert!(result.triggered);
        assert!(result.complete);
        assert_eq!(result.trigger_index, Some(3));
        assert_eq!(result.events.len(), 6);
        let addresses: Vec<u64> = result
            .events
            .iter()
            .map(|envelope| match &envelope.event {
                OwnedTraceEvent::Access(access) => access.address,
                _ => panic!("expected access event"),
            })
            .collect();
        assert_eq!(addresses, [7, 8, 9, 10, 11, 12]);
        assert_eq!(handle.ring_status(), RingCaptureStatus::Idle);
        assert!(!handle.is_active());
    }

    #[test]
    fn ring_capture_charges_retained_bytes_against_the_queue_capacity() {
        // Each device event carries two payload bytes; the queue byte capacity
        // of five holds at most two retained events.
        let (mut sink, handle) = ApplicationTraceSink::new(limits(64, 5, 64));
        handle.arm_ring_capture(
            |_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record,
            |_: TraceContext, _: TraceEvent<'_>| TraceDecision::Ignore,
            2,
            2,
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
        // The sliding pre-window stays within the byte capacity because the
        // evicted event's bytes are freed before the new event is charged.
        for cycle in 1..=4 {
            sink.trace(context(cycle), event);
        }
        assert_eq!(handle.failure(), None);
        let result = handle.take_ring_capture().unwrap();
        assert_eq!(result.events.len(), 2);

        // A post-trigger window that accumulates beyond the byte capacity
        // latches the queue overflow instead of retaining unbounded storage.
        handle.arm_ring_capture(
            |_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record,
            |_: TraceContext, event: TraceEvent<'_>| match event {
                TraceEvent::Access(_) => TraceDecision::RecordAndYield,
                _ => TraceDecision::Ignore,
            },
            0,
            8,
            TraceInterest::only(TraceEventClass::Device)
                .union(TraceInterest::only(TraceEventClass::Access)),
        );
        sink.trace(context(10), access(10));
        for cycle in 11..=14 {
            sink.trace(context(cycle), event);
        }
        assert!(matches!(
            handle.failure(),
            Some(TraceFailure::QueueOverflow { .. })
        ));
    }

    #[test]
    fn ring_capture_with_empty_pre_window_allocates_nothing_before_trigger() {
        let (mut sink, handle) = ApplicationTraceSink::new(limits(64, 1024, 64));
        handle.arm_ring_capture(
            |_: TraceContext, _: TraceEvent<'_>| TraceDecision::Record,
            |_: TraceContext, event: TraceEvent<'_>| match event {
                TraceEvent::Access(access) if access.address == 5 => TraceDecision::RecordAndYield,
                _ => TraceDecision::Ignore,
            },
            0,
            1,
            TraceInterest::only(TraceEventClass::Access),
        );
        sink.trace(context(1), access(1));
        sink.trace(context(5), access(5));
        sink.trace(context(6), access(6));
        let result = handle.take_ring_capture().unwrap();
        assert!(result.complete);
        assert_eq!(result.trigger_index, Some(0));
        assert_eq!(result.events.len(), 2);
    }
}
