//! Shared value marshalling helpers for the native library callbacks.
//!
//! These build Scheme values (alists, lists, tagged error values) and convert
//! integer arguments, and are used across every native group.

use std::{cell::RefCell, rc::Rc};

use common::{RunOutcome, StopReason};
use r7rs::{Error, NativeContext, Value};

use crate::session::{AutomationSession, OpError, RunError};

/// Builds a proper list from key and value pairs, preserving the given order.
pub(super) fn make_alist(
    context: &mut NativeContext,
    entries: Vec<(&str, Value)>,
) -> Result<Value, Error> {
    let mut list = Value::nil();
    for (key, value) in entries.into_iter().rev() {
        let key_symbol = context.intern_symbol(key)?;
        let pair = context.pair(key_symbol, value)?;
        list = context.pair(pair, list)?;
    }
    Ok(list)
}

/// Builds the artifact alist `((path . "...") (bytes . n))` for a written file.
pub(super) fn artifact_alist(
    context: &mut NativeContext,
    path: &str,
    bytes: Option<usize>,
) -> Result<Value, Error> {
    let path_value = context.string_utf8(path.to_owned())?;
    let mut entries = vec![("path", path_value)];
    if let Some(bytes) = bytes {
        let bytes_value = context.integer(i128::try_from(bytes).unwrap_or(i128::MAX))?;
        entries.push(("bytes", bytes_value));
    }
    make_alist(context, entries)
}

/// Returns the on-disk byte length of a written artifact, or 0 when unknown.
pub(super) fn written_len(path: &std::path::Path) -> usize {
    std::fs::metadata(path)
        .map(|metadata| usize::try_from(metadata.len()).unwrap_or(usize::MAX))
        .unwrap_or(0)
}

/// Builds a proper list from values, preserving the given order.
pub(super) fn make_list(context: &mut NativeContext, values: Vec<Value>) -> Result<Value, Error> {
    let mut list = Value::nil();
    for value in values.into_iter().rev() {
        list = context.pair(value, list)?;
    }
    Ok(list)
}

/// Builds the tagged error value `(%error neetan/SYM "message")`.
pub(super) fn error_value(
    context: &mut NativeContext,
    symbol: &str,
    message: &str,
) -> Result<Value, Error> {
    let tag = context.intern_symbol("%error")?;
    let symbol = context.intern_symbol(symbol)?;
    let message = context.string_utf8(message.to_owned())?;
    let tail = context.pair(message, Value::nil())?;
    let middle = context.pair(symbol, tail)?;
    context.pair(tag, middle)
}

/// Builds the tagged error value for an operation error.
pub(super) fn op_error_value(context: &mut NativeContext, error: &OpError) -> Result<Value, Error> {
    error_value(context, error.symbol(), &error.message())
}

/// Parses and validates the private machine token at a native boundary.
pub(super) fn machine_id(
    context: &mut NativeContext,
    session: &Rc<RefCell<AutomationSession>>,
    value: Value,
) -> Result<Result<u64, Value>, Error> {
    let id = match u64::try_from(context.to_i128(value)?) {
        Ok(id) => id,
        Err(_) => {
            return Ok(Err(error_value(
                context,
                "neetan/stale-handle",
                "machine handle is no longer active",
            )?));
        }
    };
    match session.borrow().validate_machine_handle(id) {
        Ok(()) => Ok(Ok(id)),
        Err(error) => Ok(Err(op_error_value(context, &error)?)),
    }
}

/// Returns whether a callback argument is a truthy Scheme value.
pub(super) fn is_truthy(value: Value) -> bool {
    value != Value::boolean(false)
}

/// Returns the stable symbol name of a run stop reason.
pub(super) fn stop_reason_name(reason: StopReason) -> &'static str {
    match reason {
        StopReason::TargetReached => "target-reached",
        StopReason::TickLimit => "tick-limit",
        StopReason::GuestShutdown => "guest-shutdown",
        StopReason::Cancelled => "cancelled",
        StopReason::CounterExhausted => "counter-exhausted",
        StopReason::MachineError => "machine-error",
    }
}

/// Builds the run-outcome alist required by the manifest.
pub(super) fn run_outcome_value(
    context: &mut NativeContext,
    session: &Rc<RefCell<AutomationSession>>,
    outcome: &RunOutcome,
) -> Result<Value, Error> {
    let timeline = session.borrow().timeline();
    let stop_reason = context.intern_symbol(stop_reason_name(outcome.stop_reason))?;
    let ticks = context.integer(i128::from(outcome.ticks))?;
    let frames = context.integer(i128::from(outcome.frames))?;
    let overshoot = context.integer(i128::from(outcome.overshoot_ticks))?;
    let epoch = context.integer(i128::from(timeline.epoch))?;
    let current_tick = context.integer(timeline.session_ticks as i128)?;
    let current_frame = context.integer(timeline.session_frames as i128)?;
    make_alist(
        context,
        vec![
            ("stop-reason", stop_reason),
            ("ticks", ticks),
            ("frames", frames),
            ("overshoot-ticks", overshoot),
            ("epoch", epoch),
            ("current-tick", current_tick),
            ("current-frame", current_frame),
        ],
    )
}

/// Maps a bounded-run result to a run-outcome alist or a tagged error value.
pub(super) fn run_result_value(
    context: &mut NativeContext,
    session: &Rc<RefCell<AutomationSession>>,
    result: Result<RunOutcome, RunError>,
) -> Result<Value, Error> {
    match result {
        Err(RunError::NoMachine) => error_value(
            context,
            "neetan/no-machine",
            "no machine has been constructed",
        ),
        Err(RunError::Range) => error_value(context, "neetan/range", "run request is out of range"),
        Err(RunError::TraceOverflow) => error_value(
            context,
            "neetan/trace-overflow",
            "trace collector exhausted its bounded queue",
        ),
        Ok(outcome) => match outcome.stop_reason {
            StopReason::TickLimit => error_value(
                context,
                "neetan/timeout",
                "tick limit reached before target",
            ),
            StopReason::GuestShutdown => {
                error_value(context, "neetan/guest-shutdown", "guest requested shutdown")
            }
            StopReason::TargetReached
            | StopReason::Cancelled
            | StopReason::CounterExhausted
            | StopReason::MachineError => run_outcome_value(context, session, &outcome),
        },
    }
}

/// Converts an exact-integer argument to a non-negative `u64` count.
pub(super) fn to_count(
    context: &mut NativeContext,
    value: Value,
) -> Result<Result<u64, Value>, Error> {
    let raw = context.to_i128(value)?;
    match u64::try_from(raw) {
        Ok(count) => Ok(Ok(count)),
        Err(_) => Ok(Err(error_value(
            context,
            "neetan/range",
            "value is out of range",
        )?)),
    }
}

/// Converts an exact-integer argument to a non-negative `u32`.
pub(super) fn to_u32(
    context: &mut NativeContext,
    value: Value,
) -> Result<Result<u32, Value>, Error> {
    let raw = context.to_i128(value)?;
    match u32::try_from(raw) {
        Ok(number) => Ok(Ok(number)),
        Err(_) => Ok(Err(error_value(
            context,
            "neetan/range",
            "value is out of range",
        )?)),
    }
}

/// Converts an exact-integer argument to a non-negative `u128`.
pub(super) fn to_u128(
    context: &mut NativeContext,
    value: Value,
) -> Result<Result<u128, Value>, Error> {
    let raw = context.to_i128(value)?;
    match u128::try_from(raw) {
        Ok(number) => Ok(Ok(number)),
        Err(_) => Ok(Err(error_value(
            context,
            "neetan/range",
            "value is out of range",
        )?)),
    }
}

/// Converts an inspected unsigned value to an exact-integer Scheme value.
///
/// Every inspected value fits the signed 128-bit range; the guard clamps
/// defensively rather than wrapping.
pub(super) fn inspected_integer(context: &mut NativeContext, value: u128) -> Result<Value, Error> {
    context.integer(i128::try_from(value).unwrap_or(i128::MAX))
}
