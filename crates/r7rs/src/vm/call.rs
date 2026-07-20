//! Call dispatch: closure/native/continuation/record calls, tail calls, and
//! frame setup.

use super::*;

// Never inlined into `execute`: the generic-call frame machinery would
// otherwise add its register pressure to the dispatch loop.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(super) fn call(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    call_base: usize,
    count: usize,
    expected: ExpectedResults,
    tail: bool,
    return_action: ReturnAction,
) -> Result<Option<Results>, Error> {
    let procedure = read_register(stack, call_base);
    if frames.last().is_some_and(|frame| {
        // A non-accepting count falls through to the classification below
        // rather than erroring here: for a case-lambda the running clause's
        // arity says nothing about the other clauses (`heap.case_lambda`
        // re-selects), and for a plain closure the classified path raises the
        // identical arity error.
        frame.procedure == procedure && frame.chunk.arity.accepts(count)
    }) {
        if tail {
            // Self-recursive tail call (the common loop shape): reuse the running
            // frame's chunk and captures in place rather than cloning both `Rc`s
            // every iteration.
            prepare_tail_self_call(heap, stack, frames, call_base, count)?;
            return Ok(None);
        }
        // Non-tail self-recursive call (e.g. `(+ (fib (- n 1)) (fib (- n 2)))`):
        // push a fresh frame that reuses the running frame's chunk and captures in
        // place, eliding both `Rc` clones on the hot path.
        prepare_self_call(
            heap,
            stack,
            frames,
            call_base,
            count,
            expected,
            return_action,
            procedure,
        )?;
        return Ok(None);
    }
    match heap.callable_kind(procedure) {
        CallableKind::Parameter => {
            if count != 0 {
                return Err(Error::plain(
                    ErrorKind::ArityError,
                    "parameter expected zero arguments",
                ));
            }
            let value = heap.parameter(procedure).ok_or_else(|| bad("parameter"))?;
            finish_immediate_call(
                heap,
                stack,
                frames,
                globals,
                symbols,
                natives,
                call_base,
                expected,
                tail,
                return_action,
                Results::One(value),
            )
        }
        CallableKind::Apply => {
            if count < 2 {
                return Err(Error::plain(
                    ErrorKind::ArityError,
                    "apply expected at least two arguments",
                ));
            }
            let supplied: Vec<Value> = (0..count)
                .map(|offset| read_register(stack, call_base + 1 + offset))
                .collect();
            let target = supplied[0];
            let mut arguments = supplied[1..supplied.len() - 1].to_vec();
            let mut tail_list = supplied[supplied.len() - 1];
            let mut seen = std::collections::HashSet::new();
            while tail_list != Value::nil() {
                let Some(reference) = tail_list.heap_ref() else {
                    return Err(Error::plain(
                        ErrorKind::TypeError,
                        "apply final argument must be a proper list",
                    ));
                };
                if !seen.insert(reference) {
                    return Err(Error::plain(
                        ErrorKind::TypeError,
                        "apply final argument must not be cyclic",
                    ));
                }
                let (car, cdr) = heap.pair(tail_list).ok_or_else(|| {
                    Error::plain(
                        ErrorKind::TypeError,
                        "apply final argument must be a proper list",
                    )
                })?;
                arguments.push(car);
                tail_list = cdr;
            }
            stack.ensure(call_base + arguments.len() + 1);
            stack[call_base] = target;
            stack[call_base + 1..call_base + arguments.len() + 1].copy_from_slice(&arguments);
            call(
                heap,
                stack,
                frames,
                globals,
                symbols,
                natives,
                call_base,
                arguments.len(),
                expected,
                tail,
                return_action,
            )
        }
        CallableKind::Record => {
            let record_procedure = heap
                .record_procedure(procedure)
                .ok_or_else(|| bad("record procedure"))?;
            let arguments: Vec<Value> = (0..count)
                .map(|offset| read_register(stack, call_base + 1 + offset))
                .collect();
            let result = invoke_record(heap, record_procedure, &arguments)?;
            finish_immediate_call(
                heap,
                stack,
                frames,
                globals,
                symbols,
                natives,
                call_base,
                expected,
                tail,
                return_action,
                Results::One(result),
            )
        }
        CallableKind::Native => {
            let (id, _, single_result, may_exit) =
                heap.native_callee(procedure).ok_or_else(|| bad("native"))?;
            let result = if single_result {
                Results::One(invoke_native_one(
                    heap, stack, frames, globals, symbols, natives, id, call_base, count,
                )?)
            } else {
                invoke_native_many(
                    heap, stack, frames, globals, symbols, natives, id, call_base, count,
                )?
                .into_results()
            };
            if may_exit && let Some(status) = heap.take_exit_request() {
                return begin_exit(
                    heap, stack, frames, globals, symbols, natives, call_base, status,
                );
            }
            if tail {
                let completed = frames.pop_frame();
                if frames.is_empty() {
                    return Ok(Some(result));
                }
                let caller = frames.last().expect("caller");
                stack.ensure(caller.base + usize::from(caller.chunk.max_registers));
                complete_return(
                    heap,
                    stack,
                    frames,
                    globals,
                    symbols,
                    natives,
                    completed.return_base,
                    completed.expected,
                    unbox_action(completed.return_action),
                    result,
                )
            } else {
                complete_return(
                    heap,
                    stack,
                    frames,
                    globals,
                    symbols,
                    natives,
                    call_base,
                    expected,
                    return_action,
                    result,
                )
            }
        }
        kind @ (CallableKind::Closure | CallableKind::CaseLambda) => {
            let closure = if kind == CallableKind::Closure {
                heap.closure(procedure).ok_or_else(|| bad("closure"))?
            } else {
                heap.case_lambda(procedure, count).ok_or_else(|| {
                    Error::plain(
                        ErrorKind::ArityError,
                        format!("no case-lambda clause accepts {count} arguments"),
                    )
                })?
            };
            if !closure.chunk.arity.accepts(count) {
                return Err(Error::plain(
                    ErrorKind::ArityError,
                    format!(
                        "procedure expected {:?} arguments, received {count}",
                        closure.chunk.arity
                    ),
                ));
            }
            prepare_closure_call(
                heap,
                stack,
                frames,
                call_base,
                count,
                expected,
                tail,
                return_action,
                procedure,
                closure,
            )?;
            Ok(None)
        }
        CallableKind::Continuation => {
            let continuation = heap
                .continuation(procedure)
                .ok_or_else(|| bad("continuation"))?;
            let values: Vec<Value> = (0..count)
                .map(|offset| read_register(stack, call_base + 1 + offset))
                .collect();
            let common = frames
                .winds
                .iter()
                .zip(&continuation.winds)
                .take_while(|(current, target)| current.id == target.id)
                .count();
            let mut thunks = frames.winds[common..]
                .iter()
                .rev()
                .map(|wind| wind.after)
                .collect::<Vec<_>>();
            thunks.extend(continuation.winds[common..].iter().map(|wind| wind.before));
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
        _ => Err(Error::plain(
            ErrorKind::RuntimeError,
            "attempted to call unsupported callable in register VM",
        )),
    }
}

/// Runs native procedure `id` on the pending arguments in
/// `stack[call_base + 1 ..= call_base + count]`, handing the registry a
/// live-VM root view. The general native path can allocate without bound, and
/// a collection during the call traces the register file (including the
/// pending arguments, which `apply` may have spread above the caller's
/// register window) directly through this view.
#[allow(clippy::too_many_arguments)]
fn invoke_native_one(
    heap: &mut Heap,
    stack: &RegisterStack,
    frames: &FrameStack,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    id: u32,
    call_base: usize,
    count: usize,
) -> Result<Value, Error> {
    let arguments = &stack[call_base + 1..call_base + 1 + count];
    let live_floor = call_base + 1 + count;
    let gather = move |roots: &mut Vec<Value>| {
        gather_vm_roots(stack, frames, live_floor, roots);
    };
    natives.invoke_one(id, heap, symbols, globals, arguments, Some(&gather))
}

#[allow(clippy::too_many_arguments)]
fn invoke_native_many(
    heap: &mut Heap,
    stack: &RegisterStack,
    frames: &FrameStack,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    id: u32,
    call_base: usize,
    count: usize,
) -> Result<crate::native::NativeValues, Error> {
    let arguments = &stack[call_base + 1..call_base + 1 + count];
    let live_floor = call_base + 1 + count;
    let gather = move |roots: &mut Vec<Value>| {
        gather_vm_roots(stack, frames, live_floor, roots);
    };
    natives.invoke_many(id, heap, symbols, globals, arguments, Some(&gather))
}

/// Outcome of [`call_native_inline`], the dispatch loop's native fast path.
pub(super) enum NativeInline {
    /// Results are delivered; execution continues in the same frame activation.
    Continue,
    /// Control left the running activation (exit unwinding began or an error
    /// handler was invoked); mirror the generic call arm's suspension.
    Suspend(Option<Results>),
}

/// Executes a native procedure called by a non-tail `Call` without leaving the
/// inner dispatch loop: no `call` prologue, no frame-activation round trip.
/// Mirrors the `CallableKind::Native` arm of [`call`] for the non-tail shape,
/// including its ordering (exit unwinding precedes result delivery) and its
/// error routing through the active exception handler.
#[allow(clippy::too_many_arguments)]
pub(super) fn call_native_inline(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    id: u32,
    fast: Option<crate::native::FastProcedure>,
    single_result: bool,
    may_exit: bool,
    call_base: usize,
    count: usize,
    expected: ExpectedResults,
) -> Result<NativeInline, Error> {
    // Classified fast path first: the single result arrives through a
    // register-friendly out parameter, with no result packet, rooted region,
    // or arity check. The classification rode along on the callee probe, so
    // an unclassified native costs one predictable branch here. A miss falls
    // through to the general `invoke` below, which re-runs the classification
    // prefix and raises the canonical error. Fast procedures never run host
    // callbacks, so no exit request can arise and the exit check is not
    // needed on this route.
    if let Some(fast_procedure) = fast {
        let mut fast = Value::unspecified();
        if fast_procedure.invoke(
            heap,
            &stack[call_base + 1..call_base + 1 + count],
            &mut fast,
        ) {
            // Same delivery as the single-value hot shape below.
            if expected != ExpectedResults::Discard {
                stack.ensure(call_base + 1);
                stack[call_base] = fast;
                if expected == ExpectedResults::All
                    && let Some(frame) = frames.last_mut()
                {
                    frame.top = call_base + 1;
                }
            }
            return Ok(NativeInline::Continue);
        }
    }
    if single_result {
        let outcome = invoke_native_one(
            heap, stack, frames, globals, symbols, natives, id, call_base, count,
        );
        let pending = match outcome {
            Ok(value) => {
                if may_exit && heap.exit_request_pending() {
                    std::hint::cold_path();
                    let status = heap.take_exit_request().expect("pending exit request");
                    return Ok(NativeInline::Suspend(begin_exit(
                        heap, stack, frames, globals, symbols, natives, call_base, status,
                    )?));
                }
                if expected != ExpectedResults::Discard {
                    stack.ensure(call_base + 1);
                    stack[call_base] = value;
                    if expected == ExpectedResults::All
                        && let Some(frame) = frames.last_mut()
                    {
                        frame.top = call_base + 1;
                    }
                }
                return Ok(NativeInline::Continue);
            }
            Err(error) => Err(error),
        };
        std::hint::cold_path();
        return finish_native_cold(
            heap, stack, frames, globals, symbols, natives, call_base, expected, pending,
        );
    }
    let outcome = invoke_native_many(
        heap, stack, frames, globals, symbols, natives, id, call_base, count,
    );
    let pending = match outcome {
        Ok(values) => {
            if may_exit && heap.exit_request_pending() {
                std::hint::cold_path();
                let status = heap.take_exit_request().expect("pending exit request");
                return Ok(NativeInline::Suspend(begin_exit(
                    heap, stack, frames, globals, symbols, natives, call_base, status,
                )?));
            }
            match values.into_single() {
                // Hot shape: exactly one result, delivered as a single register
                // write (mirrors `deliver_results`' `One` arm).
                Ok(value) => {
                    if expected != ExpectedResults::Discard {
                        stack.ensure(call_base + 1);
                        stack[call_base] = value;
                        if expected == ExpectedResults::All
                            && let Some(frame) = frames.last_mut()
                        {
                            frame.top = call_base + 1;
                        }
                    }
                    return Ok(NativeInline::Continue);
                }
                Err(results) => Ok(results),
            }
        }
        Err(error) => Err(error),
    };
    std::hint::cold_path();
    finish_native_cold(
        heap, stack, frames, globals, symbols, natives, call_base, expected, pending,
    )
}

/// Executes a native procedure called by `TailCall` without the generic
/// `call` prologue. Mirrors the `CallableKind::Native` arm of [`call`] for
/// the tail shape exactly: exit unwinding precedes the frame pop, and the
/// result is delivered through the popped frame's return slot. The caller
/// routes errors through the active exception handler, matching the generic
/// tail-call arm of the dispatch loop.
#[allow(clippy::too_many_arguments)]
pub(super) fn tail_call_native_inline(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    id: u32,
    fast: Option<crate::native::FastProcedure>,
    single_result: bool,
    may_exit: bool,
    call_base: usize,
    count: usize,
) -> Result<Option<Results>, Error> {
    // Classified fast path first, exactly as in `call_native_inline`. The
    // exit check below stays shared: it is one cheap load and this path is
    // colder than the non-tail one.
    let result = {
        let mut out = Value::unspecified();
        if fast.is_some_and(|fast| {
            fast.invoke(heap, &stack[call_base + 1..call_base + 1 + count], &mut out)
        }) {
            Results::One(out)
        } else if single_result {
            Results::One(invoke_native_one(
                heap, stack, frames, globals, symbols, natives, id, call_base, count,
            )?)
        } else {
            invoke_native_many(
                heap, stack, frames, globals, symbols, natives, id, call_base, count,
            )?
            .into_results()
        }
    };
    if may_exit && let Some(status) = heap.take_exit_request() {
        return begin_exit(
            heap, stack, frames, globals, symbols, natives, call_base, status,
        );
    }
    let completed = frames.pop_frame();
    if frames.is_empty() {
        return Ok(Some(result));
    }
    let caller = frames.last().expect("caller");
    stack.ensure(caller.base + usize::from(caller.chunk.max_registers));
    complete_return(
        heap,
        stack,
        frames,
        globals,
        symbols,
        natives,
        completed.return_base,
        completed.expected,
        unbox_action(completed.return_action),
        result,
    )
}

/// Cold tail of [`call_native_inline`]: zero/multi-value delivery and error
/// routing through the active exception handler. Outlined so the fast path
/// inlined into the dispatch loop stays small.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn finish_native_cold(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    call_base: usize,
    expected: ExpectedResults,
    pending: Result<Results, Error>,
) -> Result<NativeInline, Error> {
    let error = match pending {
        Ok(results) => match deliver_results(stack, frames, call_base, expected, results) {
            Ok(()) => return Ok(NativeInline::Continue),
            Err(error) => error,
        },
        Err(error) => error,
    };
    Ok(NativeInline::Suspend(invoke_error_handler(
        heap, stack, frames, globals, symbols, natives, call_base, expected, error,
    )?))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_immediate_call(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    call_base: usize,
    expected: ExpectedResults,
    tail: bool,
    return_action: ReturnAction,
    results: Results,
) -> Result<Option<Results>, Error> {
    if tail {
        let completed = frames.pop_frame();
        if frames.is_empty() {
            return Ok(Some(results));
        }
        let caller = frames.last().expect("caller");
        let caller_end = caller.base + usize::from(caller.chunk.max_registers);
        stack.ensure(caller_end);
        complete_return(
            heap,
            stack,
            frames,
            globals,
            symbols,
            natives,
            completed.return_base,
            completed.expected,
            unbox_action(completed.return_action),
            results,
        )
    } else {
        complete_return(
            heap,
            stack,
            frames,
            globals,
            symbols,
            natives,
            call_base,
            expected,
            return_action,
            results,
        )
    }
}

pub(super) fn invoke_record(
    heap: &mut Heap,
    procedure: crate::heap::RecordProcedure,
    arguments: &[Value],
) -> Result<Value, Error> {
    match procedure {
        crate::heap::RecordProcedure::Constructor {
            record_type,
            fields,
            mapping,
        } => {
            if arguments.len() != mapping.len() {
                return Err(Error::plain(
                    ErrorKind::ArityError,
                    format!(
                        "record constructor expected {} arguments, received {}",
                        mapping.len(),
                        arguments.len()
                    ),
                ));
            }
            let mut values = vec![Value::unspecified(); fields];
            for (field, argument) in mapping.into_iter().zip(arguments.iter().copied()) {
                values[field] = argument;
            }
            heap.alloc(Object::Record(Box::new(crate::heap::Record {
                record_type,
                fields: values,
            })))
        }
        crate::heap::RecordProcedure::Predicate { record_type } => {
            if arguments.len() != 1 {
                return Err(Error::plain(
                    ErrorKind::ArityError,
                    "record predicate expected one argument",
                ));
            }
            Ok(Value::boolean(
                heap.record(arguments[0])
                    .is_some_and(|record| record.record_type == record_type),
            ))
        }
        crate::heap::RecordProcedure::Accessor { record_type, field } => {
            if arguments.len() != 1 {
                return Err(Error::plain(
                    ErrorKind::ArityError,
                    "record accessor expected one argument",
                ));
            }
            let record = heap.record(arguments[0]).ok_or_else(|| {
                Error::plain(ErrorKind::TypeError, "record accessor expected a record")
            })?;
            if record.record_type != record_type {
                return Err(Error::plain(
                    ErrorKind::TypeError,
                    "record has the wrong type",
                ));
            }
            record
                .fields
                .get(field)
                .copied()
                .ok_or_else(|| bad("record field"))
        }
        crate::heap::RecordProcedure::Mutator { record_type, field } => {
            if arguments.len() != 2 {
                return Err(Error::plain(
                    ErrorKind::ArityError,
                    "record mutator expected two arguments",
                ));
            }
            let record = heap.record(arguments[0]).ok_or_else(|| {
                Error::plain(ErrorKind::TypeError, "record mutator expected a record")
            })?;
            if record.record_type != record_type {
                return Err(Error::plain(
                    ErrorKind::TypeError,
                    "record has the wrong type",
                ));
            }
            if !heap.set_record_field(arguments[0], field, arguments[1]) {
                return Err(bad("record field"));
            }
            Ok(Value::unspecified())
        }
    }
}

pub(super) fn outcome(heap: &mut Heap, results: Results) -> crate::EvalOutcome {
    crate::EvalOutcome::Values(Values::new(
        results
            .into_vec()
            .into_iter()
            .map(|value| heap.root(value))
            .collect(),
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_closure_call(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    call_base: usize,
    count: usize,
    expected: ExpectedResults,
    tail: bool,
    return_action: ReturnAction,
    procedure: Value,
    closure: Closure,
) -> Result<(), Error> {
    let required = match closure.chunk.arity {
        Arity::Exact(n) | Arity::AtLeast(n) => usize::from(n),
    };
    if tail {
        let frame = frames.last().expect("frame");
        let destination = frame.base;
        shift_arguments_down(stack, call_base + 1, destination, count);
        if matches!(closure.chunk.arity, Arity::AtLeast(_)) {
            let mut rest = Value::nil();
            for value in stack[destination + required..destination + count]
                .iter()
                .rev()
            {
                // The growing chain survives the repeated allocs without an explicit
                // root: in-VM allocation defers collection to the next safe point, and
                // the finished list lands in the stack before that.
                rest = heap.alloc_pair(*value, rest)?;
            }
            stack[destination + required] = rest;
        }
        let len = destination + usize::from(closure.chunk.max_registers);
        // Grow-only: never shrink the reused frame's backing storage. The new
        // window's dead tail sits above `top` and is never scanned or read.
        stack.ensure(len);
        if !closure.chunk.boxed_locals.is_empty() {
            std::hint::cold_path();
            box_frame_locals(heap, stack, destination, &closure.chunk)?;
        }
        let frame = frames.last_mut().expect("frame");
        frame.chunk = closure.chunk;
        frame.captures = closure.captures;
        frame.procedure = procedure;
        frame.pc = 0;
        frame.top =
            destination + required + usize::from(matches!(frame.chunk.arity, Arity::AtLeast(_)));
        return Ok(());
    }
    let base = call_base + 1;
    if matches!(closure.chunk.arity, Arity::AtLeast(_)) {
        let mut rest = Value::nil();
        for value in stack[base + required..base + count].iter().rev() {
            // The growing chain survives the repeated allocs without an explicit
            // root: in-VM allocation defers collection to the next safe point, and
            // the finished list lands in the stack before that.
            rest = heap.alloc_pair(*value, rest)?;
        }
        stack.ensure(base + required + 1);
        stack[base + required] = rest;
    }
    let len = base + usize::from(closure.chunk.max_registers);
    stack.ensure(len);
    if !closure.chunk.boxed_locals.is_empty() {
        std::hint::cold_path();
        box_frame_locals(heap, stack, base, &closure.chunk)?;
    }
    let has_rest = matches!(closure.chunk.arity, Arity::AtLeast(_));
    let top = base + required + usize::from(has_rest);
    // Write the new frame directly into its (recycled) buffer slot.
    let frame = frames.reserve();
    frame.chunk = closure.chunk;
    frame.pc = 0;
    frame.base = base;
    frame.top = top;
    frame.return_base = call_base;
    frame.expected = expected;
    frame.captures = closure.captures;
    frame.procedure = procedure;
    frame.return_action = boxed_action(return_action);
    Ok(())
}

/// Moves `count` argument registers from `source` down to `destination` for a
/// tail call (`destination <= source` always: the argument window sits above
/// the re-entered frame's base, at `call_base + 1`). Tail calls move only a
/// handful of registers, where `copy_within`'s out-of-line memmove call costs
/// more than the copy itself, so short moves take an explicit loop instead.
/// Safe under overlap because the copy walks upward while every source index
/// sits at or above its destination. Register access is unchecked (see
/// `read_register`): the arguments were just written inside the caller's
/// `ensure`d window and `destination + count <= source + count`.
#[inline(always)]
fn shift_arguments_down(
    stack: &mut RegisterStack,
    source: usize,
    destination: usize,
    count: usize,
) {
    debug_assert!(destination <= source, "tail-call arguments shift upward");
    if count <= 8 {
        for index in 0..count {
            let value = read_register(stack, source + index);
            write_register(stack, destination + index, value);
        }
    } else {
        stack.0.copy_within(source..source + count, destination);
    }
}

/// Re-enters the running frame for a self-recursive tail call. Mirrors the tail
/// branch of [`prepare_closure_call`] but keeps the frame's existing `chunk`,
/// `captures`, `procedure`, `return_base`, `expected`, and `return_action`
/// (all unchanged by definition), so no `Rc` is cloned per iteration.
///
/// Inlined into `execute`'s self-tail-call fast path (every named-let loop
/// iteration that is not LoopBack-fused). Without the hint the out-of-line
/// call costs measurable cycles on tail-call-heavy workloads. `always` because
/// LLVM was observed dropping the plain hint after unrelated `execute` growth.
#[inline(always)]
pub(super) fn prepare_tail_self_call(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    call_base: usize,
    count: usize,
) -> Result<(), Error> {
    let (required, has_rest, max_registers, destination) = {
        let frame = frames.last().expect("frame");
        let required = match frame.chunk.arity {
            Arity::Exact(n) | Arity::AtLeast(n) => usize::from(n),
        };
        (
            required,
            matches!(frame.chunk.arity, Arity::AtLeast(_)),
            usize::from(frame.chunk.max_registers),
            frame.base,
        )
    };
    shift_arguments_down(stack, call_base + 1, destination, count);
    if has_rest {
        let mut rest = Value::nil();
        for value in stack[destination + required..destination + count]
            .iter()
            .rev()
        {
            // The growing chain survives the repeated allocs without an explicit
            // root: in-VM allocation defers collection to the next safe point, and
            // the finished list lands in the stack before that.
            rest = heap.alloc_pair(*value, rest)?;
        }
        stack[destination + required] = rest;
    }
    stack.ensure(destination + max_registers);
    if !frames.last().expect("frame").chunk.boxed_locals.is_empty() {
        std::hint::cold_path();
        box_frame_locals(
            heap,
            stack,
            destination,
            &frames.last().expect("frame").chunk,
        )?;
    }
    let frame = frames.last_mut().expect("frame");
    frame.pc = 0;
    frame.top = destination + required + usize::from(has_rest);
    Ok(())
}

/// Pushes a new frame for a non-tail self-recursive call of the infallible
/// shape, the dispatch loop's hottest call path (every `fib`-style recursion).
/// The caller's guard proved exact matching arity, no rest list, and no boxed
/// locals, so the fat prologue of [`prepare_self_call`] (arity re-reads, rest
/// allocation, local boxing, the fallible `Result`) is unnecessary; the guard
/// also proved the callee is the running procedure, so `procedure` and
/// `max_registers` come from the caller frame / the arm's hoisted chunk rather
/// than being re-derived. The return action is `Normal` by definition and is
/// not written at all: the reserved slot holds `None` by [`FrameStack::reserve`]'s
/// dead-slot invariant, which keeps the per-call `ReturnAction` construction,
/// write, and drop check off this path entirely.
///
/// Inlined into `execute`'s self-call fast path: the slimmed body is small
/// enough that the saved call/spill overhead outweighs the register pressure.
#[inline(always)]
pub(super) fn push_self_frame(
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    call_base: usize,
    count: usize,
    expected: ExpectedResults,
    max_registers: usize,
) {
    let base = call_base + 1;
    stack.ensure(base + max_registers);
    let caller = frames.last().expect("frame");
    let procedure = caller.procedure;
    let caller_chunk_ptr = Rc::as_ptr(&caller.chunk);
    let caller_captures_ptr = Rc::as_ptr(&caller.captures);
    // Write the new frame directly into its recycled buffer slot.
    let frame = frames.reserve();
    frame.pc = 0;
    frame.base = base;
    frame.top = base + count;
    frame.return_base = call_base;
    frame.expected = expected;
    frame.procedure = procedure;
    debug_assert!(
        frame.return_action.is_none(),
        "dead frame slot holds a return action"
    );
    let need_chunk = Rc::as_ptr(&frame.chunk) != caller_chunk_ptr;
    // `captures` is `Rc<[Value]>`; compare only the allocation address
    // (same allocation implies same length), ignoring the slice metadata.
    let need_captures = !std::ptr::addr_eq(Rc::as_ptr(&frame.captures), caller_captures_ptr);
    // Cold: the recycled slot last held a different procedure, so refresh
    // whichever `Rc` no longer matches the caller (== callee).
    if need_chunk || need_captures {
        std::hint::cold_path();
        let slot = frames.depth - 1;
        let caller = slot - 1;
        if need_chunk {
            let chunk = Rc::clone(&frames.buffer[caller].chunk);
            frames.buffer[slot].chunk = chunk;
        }
        if need_captures {
            let captures = Rc::clone(&frames.buffer[caller].captures);
            frames.buffer[slot].captures = captures;
        }
    }
}

/// Pushes a new frame for a non-tail self-recursive call. Mirrors the non-tail
/// branch of [`prepare_closure_call`], but because the caller is the same closure
/// as the callee (the `frame.procedure == procedure` guard), it reuses the
/// running frame's `chunk` and `captures` rather than cloning them into a
/// `Closure`. The recycled frame slot usually already holds those same `Rc`s (the
/// grow path clones them from the caller as filler; steady-state recursion leaves
/// them in place), so the common case pushes a frame with no `Rc` traffic at all.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_self_call(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    call_base: usize,
    count: usize,
    expected: ExpectedResults,
    return_action: ReturnAction,
    procedure: Value,
) -> Result<(), Error> {
    // Read the callee shape and the identities of the caller's chunk/captures
    // (cheap raw pointers, no clone) up front so the reserved slot below can be
    // checked against them.
    let (required, has_rest, max_registers, caller_chunk_ptr, caller_captures_ptr) = {
        let frame = frames.last().expect("frame");
        let required = match frame.chunk.arity {
            Arity::Exact(n) | Arity::AtLeast(n) => usize::from(n),
        };
        (
            required,
            matches!(frame.chunk.arity, Arity::AtLeast(_)),
            usize::from(frame.chunk.max_registers),
            Rc::as_ptr(&frame.chunk),
            Rc::as_ptr(&frame.captures),
        )
    };
    let base = call_base + 1;
    if has_rest {
        let mut rest = Value::nil();
        for value in stack[base + required..base + count].iter().rev() {
            // The growing chain survives the repeated allocs without an explicit
            // root: in-VM allocation defers collection to the next safe point, and
            // the finished list lands in the stack before that.
            rest = heap.alloc_pair(*value, rest)?;
        }
        stack.ensure(base + required + 1);
        stack[base + required] = rest;
    }
    stack.ensure(base + max_registers);
    if !frames.last().expect("frame").chunk.boxed_locals.is_empty() {
        std::hint::cold_path();
        // Clone the chunk `Rc` only on this rare path so `box_frame_locals` can
        // borrow it while `stack` is mutated.
        let chunk = Rc::clone(&frames.last().expect("frame").chunk);
        box_frame_locals(heap, stack, base, &chunk)?;
    }
    let top = base + required + usize::from(has_rest);
    let (need_chunk, need_captures) = {
        // Write the new frame directly into its recycled buffer slot.
        let frame = frames.reserve();
        frame.pc = 0;
        frame.base = base;
        frame.top = top;
        frame.return_base = call_base;
        frame.expected = expected;
        frame.procedure = procedure;
        frame.return_action = boxed_action(return_action);
        (
            Rc::as_ptr(&frame.chunk) != caller_chunk_ptr,
            // `captures` is `Rc<[Value]>`; compare only the allocation address
            // (same allocation implies same length), ignoring the slice metadata.
            !std::ptr::addr_eq(Rc::as_ptr(&frame.captures), caller_captures_ptr),
        )
    };
    // Cold: the recycled slot last held a different procedure, so refresh whichever
    // `Rc` no longer matches the caller (== callee).
    if need_chunk || need_captures {
        std::hint::cold_path();
        let slot = frames.depth - 1;
        let caller = slot - 1;
        if need_chunk {
            let chunk = Rc::clone(&frames.buffer[caller].chunk);
            frames.buffer[slot].chunk = chunk;
        }
        if need_captures {
            let captures = Rc::clone(&frames.buffer[caller].captures);
            frames.buffer[slot].captures = captures;
        }
    }
    Ok(())
}

/// Pushes a new frame for a non-tail call to a plain closure with the
/// infallible shape (exact matching arity, no rest list, no boxed locals).
/// Mirrors the non-tail branch of [`prepare_closure_call`] minus the paths the
/// shape excludes, with [`prepare_self_call`]'s recycled-slot pointer compare
/// keyed to the *callee's* `Rc`s: a loop calling the same procedure finds them
/// already in the slot and pushes a frame with no `Rc` traffic at all.
pub(super) fn prepare_general_call(
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    call_base: usize,
    count: usize,
    expected: ExpectedResults,
    procedure: Value,
    closure: &Closure,
) {
    let base = call_base + 1;
    // Grow-only: never shrink the reused frame's backing storage.
    stack.ensure(base + usize::from(closure.chunk.max_registers));
    // Write the new frame directly into its (recycled) buffer slot.
    let frame = frames.reserve();
    frame.pc = 0;
    frame.base = base;
    frame.top = base + count;
    frame.return_base = call_base;
    frame.expected = expected;
    frame.procedure = procedure;
    // An ordinary call's action is always `Normal` (`None`), which the
    // recycled slot already holds by `reserve`'s dead-slot invariant.
    debug_assert!(
        frame.return_action.is_none(),
        "dead frame slot holds a return action"
    );
    // Cold: the recycled slot last held a different procedure's Rcs.
    if Rc::as_ptr(&frame.chunk) != Rc::as_ptr(&closure.chunk) {
        std::hint::cold_path();
        frame.chunk = Rc::clone(&closure.chunk);
    }
    if !std::ptr::addr_eq(Rc::as_ptr(&frame.captures), Rc::as_ptr(&closure.captures)) {
        std::hint::cold_path();
        frame.captures = Rc::clone(&closure.captures);
    }
}

/// Re-enters the running frame for a tail call to a different plain closure
/// with the infallible shape. Mirrors the tail branch of
/// [`prepare_closure_call`] minus the paths the shape excludes: the frame's
/// `return_base`, `expected`, and `return_action` stay untouched (tail
/// semantics), while `chunk`/`captures`/`procedure` switch to the callee.
/// The pointer compares spare the `Rc` traffic when the callee shares the
/// frame's current chunk or captures (e.g. sibling closures over one lambda).
pub(super) fn prepare_tail_general_call(
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    call_base: usize,
    count: usize,
    procedure: Value,
    closure: &Closure,
) {
    let destination = frames.last().expect("frame").base;
    shift_arguments_down(stack, call_base + 1, destination, count);
    stack.ensure(destination + usize::from(closure.chunk.max_registers));
    let frame = frames.last_mut().expect("frame");
    frame.pc = 0;
    frame.top = destination + count;
    frame.procedure = procedure;
    if Rc::as_ptr(&frame.chunk) != Rc::as_ptr(&closure.chunk) {
        frame.chunk = Rc::clone(&closure.chunk);
    }
    if !std::ptr::addr_eq(Rc::as_ptr(&frame.captures), Rc::as_ptr(&closure.captures)) {
        frame.captures = Rc::clone(&closure.captures);
    }
}

/// Wraps each of a freshly entered frame's boxed parameters in a heap `Box`
/// cell. The slot currently holds the raw argument (or rest list); afterwards
/// it holds a cell that `GetLocalBox`/`SetLocalBox`, captures, and captured
/// continuations all share.
pub(super) fn box_frame_locals(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    base: usize,
    chunk: &Chunk,
) -> Result<(), Error> {
    for &index in chunk.boxed_locals.iter() {
        let slot = base + usize::from(index);
        let raw = stack.get(slot)?;
        let cell = heap.alloc(Object::Box(raw))?;
        // The cell is reachable from `stack[slot]` before the next safe point,
        // so no explicit rooting is required (see [`Heap::enter_vm`]).
        stack[slot] = cell;
    }
    Ok(())
}
