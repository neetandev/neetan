//! Return handling: the return-action dispatch, dynamic-wind, continuation
//! restore, parameter binding, and exit unwinding.

use super::*;

#[allow(clippy::too_many_arguments)]
// Never inlined into `execute` (see `call`): only the `None`-action fast
// return stays in the dispatch loop.
#[inline(never)]
pub(super) fn complete_return(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    destination: usize,
    expected: ExpectedResults,
    action: ReturnAction,
    results: Results,
) -> Result<Option<Results>, Error> {
    match action {
        ReturnAction::Normal => {
            if frames.is_empty() {
                Ok(Some(results))
            } else {
                deliver_results(stack, frames, destination, expected, results)?;
                Ok(None)
            }
        }
        ReturnAction::InvokeConsumer(consumer) => {
            let values = results.into_vec();
            stack.ensure(destination + values.len() + 1);
            stack[destination] = consumer;
            stack[destination + 1..destination + 1 + values.len()].copy_from_slice(&values);
            call(
                heap,
                stack,
                frames,
                globals,
                symbols,
                natives,
                destination,
                values.len(),
                expected,
                false,
                ReturnAction::Normal,
            )
        }
        ReturnAction::StorePromise { promise, flatten } => {
            let values = results.into_vec();
            if flatten {
                let [next] = values.as_slice() else {
                    return Err(Error::plain(
                        ErrorKind::RuntimeError,
                        format!("expected exactly one value, received {}", values.len()),
                    ));
                };
                if heap.promise_state(*next).is_some() {
                    if !heap.set_promise_state(promise, PromiseState::Forward(*next)) {
                        return Err(bad("promise"));
                    }
                    return force_promise(
                        heap,
                        stack,
                        frames,
                        globals,
                        symbols,
                        natives,
                        destination,
                        expected,
                        *next,
                    );
                }
            }
            if !heap.set_promise_state(promise, PromiseState::Done(values.clone())) {
                return Err(bad("promise"));
            }
            complete_return(
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
            )
        }
        ReturnAction::CreateParameter { converter } => {
            let value = exactly_one(results)?;
            let parameter = heap.alloc(Object::Parameter(Box::new(Parameter {
                value,
                converter: Some(converter),
            })))?;
            complete_return(
                heap,
                stack,
                frames,
                globals,
                symbols,
                natives,
                destination,
                expected,
                ReturnAction::Normal,
                Results::One(parameter),
            )
        }
        ReturnAction::RaiseReturned => Err(Error::plain(
            ErrorKind::RuntimeError,
            "exception handler returned from non-continuable raise",
        )),
        ReturnAction::ReinstallHandler(handler) => {
            frames.handlers.push(handler);
            complete_return(
                heap,
                stack,
                frames,
                globals,
                symbols,
                natives,
                destination,
                expected,
                ReturnAction::Normal,
                results,
            )
        }
        ReturnAction::StartWind(data) => {
            let StartWindData { thunk, wind } = *data;
            frames.winds.push(wind.clone());
            stack.ensure(destination + 1);
            stack[destination] = thunk;
            call(
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
                ReturnAction::FinishWind(Box::new(wind)),
            )
        }
        ReturnAction::FinishWind(wind) => {
            let active = frames.winds.pop().ok_or_else(|| bad("wind stack"))?;
            if active.id != wind.id {
                return Err(bad("wind identity"));
            }
            let values = results.into_vec();
            stack.ensure(destination + 1);
            stack[destination] = wind.after;
            call(
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
                ReturnAction::RestoreResults(values),
            )
        }
        ReturnAction::RestoreResults(values) => complete_return(
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
        ),
        ReturnAction::ClosePort(port) => {
            heap.ports_mut().close(port)?;
            complete_return(
                heap,
                stack,
                frames,
                globals,
                symbols,
                natives,
                destination,
                expected,
                ReturnAction::Normal,
                results,
            )
        }
        ReturnAction::RestorePort(data) => {
            let RestorePortData {
                port,
                parameter,
                old,
            } = *data;
            let active = frames
                .parameters
                .pop()
                .ok_or_else(|| bad("parameter stack"))?;
            if active != (parameter, old) || !heap.set_parameter(parameter, old) {
                return Err(bad("current port parameter"));
            }
            heap.ports_mut().close(port)?;
            complete_return(
                heap,
                stack,
                frames,
                globals,
                symbols,
                natives,
                destination,
                expected,
                ReturnAction::Normal,
                results,
            )
        }
        ReturnAction::LoadComplete => complete_return(
            heap,
            stack,
            frames,
            globals,
            symbols,
            natives,
            destination,
            expected,
            ReturnAction::Normal,
            Results::One(Value::unspecified()),
        ),
        ReturnAction::ConvertedParameter(data) => {
            let ConvertedParameterData {
                call_base,
                parameter,
                old,
                remaining,
                mut converted,
            } = *data;
            converted.push((parameter, old, exactly_one(results)?));
            continue_parameter_bindings(
                heap, stack, frames, globals, symbols, natives, call_base, remaining, converted,
            )
        }
        ReturnAction::ContinueTransfer(data) => {
            let ContinueTransferData {
                call_base,
                mut thunks,
                continuation,
                values,
            } = *data;
            if thunks.is_empty() {
                restore_continuation(heap, stack, frames, continuation, values)?;
                return Ok(None);
            }
            let thunk = thunks.remove(0);
            stack.ensure(call_base + 1);
            stack[call_base] = thunk;
            call(
                heap,
                stack,
                frames,
                globals,
                symbols,
                natives,
                call_base,
                0,
                ExpectedResults::Discard,
                false,
                ReturnAction::ContinueTransfer(Box::new(ContinueTransferData {
                    call_base,
                    thunks,
                    continuation,
                    values,
                })),
            )
        }
        ReturnAction::ExitCleanup(data) => {
            let ExitCleanupData {
                call_base,
                remaining,
                status,
            } = *data;
            continue_exit(
                heap, stack, frames, globals, symbols, natives, call_base, remaining, status,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn begin_exit(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    call_base: usize,
    status: crate::ExitStatus,
) -> Result<Option<Results>, Error> {
    let remaining = if status.emergency() {
        Vec::new()
    } else {
        frames.winds.iter().rev().map(|wind| wind.after).collect()
    };
    frames.winds.clear();
    frames.handlers.clear();
    for (parameter, old) in frames.parameters.drain(..).rev() {
        if !heap.set_parameter(parameter, old) {
            return Err(bad("parameter during exit"));
        }
    }
    continue_exit(
        heap, stack, frames, globals, symbols, natives, call_base, remaining, status,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn continue_exit(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    call_base: usize,
    mut remaining: Vec<Value>,
    status: crate::ExitStatus,
) -> Result<Option<Results>, Error> {
    if !remaining.is_empty() {
        let thunk = remaining.remove(0);
        stack.ensure(call_base + 1);
        stack[call_base] = thunk;
        return call(
            heap,
            stack,
            frames,
            globals,
            symbols,
            natives,
            call_base,
            0,
            ExpectedResults::Discard,
            false,
            ReturnAction::ExitCleanup(Box::new(ExitCleanupData {
                call_base,
                remaining,
                status,
            })),
        );
    }
    heap.process_context()?
        .exit(status.code(), status.emergency())
        .map_err(|error| {
            Error::plain(
                ErrorKind::RuntimeError,
                format!("process capability failed: {error}"),
            )
        })?;
    heap.complete_exit(status);
    Ok(None)
}

pub(super) fn restore_continuation(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    continuation: Continuation,
    values: Vec<Value>,
) -> Result<(), Error> {
    for (parameter, old) in frames.parameters.iter().rev() {
        if !heap.set_parameter(*parameter, *old) {
            return Err(bad("parameter"));
        }
    }
    let Continuation {
        frames: mut restored_frames,
        stack: restored_stack,
        handlers,
        parameters,
        parameter_values,
        winds,
        destination,
        expected,
    } = continuation;
    for (parameter, value) in parameter_values {
        if !heap.set_parameter(parameter, value) {
            return Err(bad("parameter"));
        }
    }
    restored_frames.handlers = handlers;
    restored_frames.parameters = parameters;
    restored_frames.winds = winds;
    *frames = restored_frames;
    *stack = restored_stack;
    deliver_results(
        stack,
        frames,
        destination,
        expected,
        results_from_slice(&values),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn continue_parameter_bindings(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    call_base: usize,
    mut remaining: Vec<(Value, Value, Value)>,
    mut converted: Vec<(Value, Value, Value)>,
) -> Result<Option<Results>, Error> {
    while !remaining.is_empty() {
        let (parameter, old, value) = remaining.remove(0);
        let converter = heap
            .parameter_converter(parameter)
            .ok_or_else(|| Error::plain(ErrorKind::TypeError, "expected parameter"))?;
        if let Some(converter) = converter {
            stack.ensure(call_base + 2);
            stack[call_base] = converter;
            stack[call_base + 1] = value;
            return call(
                heap,
                stack,
                frames,
                globals,
                symbols,
                natives,
                call_base,
                1,
                ExpectedResults::One,
                false,
                ReturnAction::ConvertedParameter(Box::new(ConvertedParameterData {
                    call_base,
                    parameter,
                    old,
                    remaining,
                    converted,
                })),
            );
        }
        converted.push((parameter, old, value));
    }
    for (parameter, old, value) in converted {
        if !heap.set_parameter(parameter, value) {
            return Err(bad("parameter"));
        }
        frames.parameters.push((parameter, old));
    }
    Ok(None)
}

pub(super) fn string_path(heap: &Heap, value: Value) -> Result<String, Error> {
    heap.string(value)
        .ok_or_else(|| Error::plain(ErrorKind::TypeError, "expected string path"))
}
