//! Exceptions, promise forcing, and result delivery.

use super::*;

pub(super) fn unhandled_exception(heap: &Heap, object: Value) -> Error {
    let message = heap
        .error_object(object)
        .and_then(|error| heap.string(error.message).map(|text| (text, error)))
        .map(|(text, error)| {
            let mut message = text;
            if !error.irritants.is_empty() {
                let irritants = error
                    .irritants
                    .iter()
                    .map(|value| {
                        crate::printer::write_value(
                            heap,
                            *value,
                            crate::printer::RuntimeWriteMode::Write,
                        )
                        .unwrap_or_else(|_| "#<unprintable>".to_owned())
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                message.push_str(": ");
                message.push_str(&irritants);
            }
            message
        })
        .unwrap_or_else(|| "unhandled Scheme exception".to_owned());
    Error::plain(ErrorKind::RuntimeError, message)
}

#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(super) fn invoke_error_handler(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    destination: usize,
    expected: ExpectedResults,
    error: Error,
) -> Result<Option<Results>, Error> {
    let Some(handler) = frames.handlers.pop() else {
        return Err(error);
    };
    let kind = match error.kind() {
        ErrorKind::FileError => ConditionKind::File,
        ErrorKind::ReadError
        | ErrorKind::InvalidUtf8
        | ErrorKind::InvalidToken
        | ErrorKind::UnexpectedEof
        | ErrorKind::InvalidDatum
        | ErrorKind::InvalidDatumLabel
        | ErrorKind::InvalidNumber
        | ErrorKind::ReaderLimitExceeded => ConditionKind::Read,
        _ => ConditionKind::Error,
    };
    let text = error.diagnostic().message();
    let message = heap.alloc_string(text, text.chars().count())?;
    // `message` stays live across the next alloc without an explicit root:
    // inside the VM, allocation defers collection to the next safe point.
    let object = heap.alloc(Object::Error(Box::new(ErrorObject {
        message,
        irritants: Vec::new(),
        kind,
    })))?;
    stack.ensure(destination + 2);
    stack[destination] = handler;
    stack[destination + 1] = object;
    call(
        heap,
        stack,
        frames,
        globals,
        symbols,
        natives,
        destination,
        1,
        expected,
        false,
        ReturnAction::RaiseReturned,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn force_promise(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    destination: usize,
    expected: ExpectedResults,
    mut promise: Value,
) -> Result<Option<Results>, Error> {
    loop {
        match heap.promise_state(promise) {
            Some(PromiseState::Done(values)) => {
                return complete_return(
                    heap,
                    stack,
                    frames,
                    globals,
                    symbols,
                    natives,
                    destination,
                    expected,
                    ReturnAction::Normal,
                    results_from_slice(&values),
                );
            }
            Some(PromiseState::Forward(next)) => promise = next,
            Some(PromiseState::Pending { thunk, flatten })
            | Some(PromiseState::Forcing { thunk, flatten }) => {
                if !heap.set_promise_state(promise, PromiseState::Forcing { thunk, flatten }) {
                    return Err(bad("promise"));
                }
                stack.ensure(destination + 1);
                stack[destination] = thunk;
                return call(
                    heap,
                    stack,
                    frames,
                    globals,
                    symbols,
                    natives,
                    destination,
                    0,
                    expected,
                    false,
                    ReturnAction::StorePromise { promise, flatten },
                );
            }
            None => return Err(Error::plain(ErrorKind::TypeError, "expected promise")),
        }
    }
}

pub(super) fn deliver_results(
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    destination: usize,
    expected: ExpectedResults,
    results: Results,
) -> Result<(), Error> {
    // Match on `results` alone (rather than a `(expected, results)` tuple) so the
    // 24-byte `Results` scrutinee is not materialized to the stack on every return.
    // This delivery runs on the hot path of every call that returns.
    let count = match results {
        Results::Zero => {
            if expected == ExpectedResults::One {
                return Err(Error::plain(
                    ErrorKind::RuntimeError,
                    "expected exactly one value, received 0".to_string(),
                ));
            }
            0
        }
        Results::One(value) => {
            if expected == ExpectedResults::Discard {
                0
            } else {
                stack.ensure(destination + 1);
                stack[destination] = value;
                1
            }
        }
        Results::Many(values) => {
            if expected == ExpectedResults::One && values.len() != 1 {
                return Err(Error::plain(
                    ErrorKind::RuntimeError,
                    format!("expected exactly one value, received {}", values.len()),
                ));
            }
            if expected == ExpectedResults::Discard {
                0
            } else {
                stack.ensure(destination + values.len());
                stack[destination..destination + values.len()].copy_from_slice(&values);
                values.len()
            }
        }
    };
    if expected == ExpectedResults::All
        && let Some(frame) = frames.last_mut()
    {
        frame.top = destination + count;
    }
    Ok(())
}

/// Delivers a `Normal` return's results straight from the callee's register
/// window (`stack[first..first + count]`) into the caller, without materializing
/// an intermediate `Results` or `ReturnAction`. This is the hot-path equivalent
/// of `complete_return(.., ReturnAction::Normal, results_from_slice(..))` for a
/// non-empty frame stack.
///
/// Inlined into `execute`'s `Return` arm: it runs once per return, and the
/// out-of-line call was measurable on call-heavy workloads.
#[inline(always)]
pub(super) fn deliver_return_fast(
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    destination: usize,
    expected: ExpectedResults,
    first: usize,
    count: usize,
) -> Result<(), Error> {
    let delivered = if expected == ExpectedResults::Discard {
        0
    } else if count == 0 {
        if expected == ExpectedResults::One {
            return Err(Error::plain(
                ErrorKind::RuntimeError,
                "expected exactly one value, received 0".to_string(),
            ));
        }
        0
    } else if count == 1 {
        // `expected` is `One` or `All`; both deliver the single value. No
        // `ensure` here: the sole caller (the `Return` arm) just re-extended
        // the caller frame's window to `base + max_registers`, and
        // `destination` is `return_base` = `base + a` of the originating
        // `Call` word, whose `a` the verifier bounds below `max_registers`.
        let value = read_register(stack, first);
        debug_assert!(destination < stack.0.len(), "return destination in window");
        write_register(stack, destination, value);
        1
    } else {
        if expected == ExpectedResults::One {
            return Err(Error::plain(
                ErrorKind::RuntimeError,
                format!("expected exactly one value, received {count}"),
            ));
        }
        // `expected` is `All`. Move the values down into the caller's window;
        // `copy_within` is memmove-correct should the ranges ever overlap.
        stack.ensure(destination + count);
        stack.0.copy_within(first..first + count, destination);
        count
    };
    if expected == ExpectedResults::All
        && let Some(frame) = frames.last_mut()
    {
        frame.top = destination + delivered;
    }
    Ok(())
}

pub(super) fn results_from_slice(values: &[Value]) -> Results {
    match values {
        [] => Results::Zero,
        [value] => Results::One(*value),
        values => Results::Many(values.to_vec()),
    }
}

pub(super) fn exactly_one(results: Results) -> Result<Value, Error> {
    match results {
        Results::One(value) => Ok(value),
        Results::Zero => Err(Error::plain(
            ErrorKind::RuntimeError,
            "expected exactly one value, received 0",
        )),
        Results::Many(values) => Err(Error::plain(
            ErrorKind::RuntimeError,
            format!("expected exactly one value, received {}", values.len()),
        )),
    }
}
