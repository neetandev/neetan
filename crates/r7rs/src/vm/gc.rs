//! Precise GC rooting and small VM utilities.

use super::*;

/// The number of register slots that can hold live values: everything below the
/// current (deepest) frame's window top. Every suspended caller allocates its
/// pending call at the top of its own registers, so its live registers all lie
/// below the callee's base; the union is therefore the prefix `[0, top)`.
pub(super) fn live_register_top(frames: &FrameStack) -> usize {
    frames.last().map_or(0, |frame| {
        frame.base + usize::from(frame.chunk.max_registers)
    })
}

/// Appends every root the live VM state holds: registers below the deepest
/// frame's window top, frame captures/procedures/pending return actions,
/// handlers, parameter shadows, and dynamic winds. Passed to
/// [`Heap::collect_with`] so a collection traces the register file directly.
/// No eager snapshot is ever materialized. Over-approximating (rooting extra
/// values) is always safe; only missing a live value would be unsound, so this
/// enumerates a superset of every reachable edge.
///
/// `live_floor` extends the scanned register prefix beyond the deepest frame's
/// window: `apply` spreads a pending call's arguments above the caller's
/// `max_registers`, so the native call site passes the end of that argument
/// block to keep the arguments alive across the call.
pub(super) fn gather_vm_roots(
    stack: &RegisterStack,
    frames: &FrameStack,
    live_floor: usize,
    roots: &mut Vec<Value>,
) {
    let live_top = live_register_top(frames).max(live_floor).min(stack.len());
    roots.extend(stack.0[..live_top].iter().copied());
    for frame in frames.iter() {
        roots.extend(frame.captures.iter().copied());
        roots.push(frame.procedure);
        if let Some(action) = &frame.return_action {
            action.trace(roots);
        }
    }
    roots.extend(frames.handlers.iter().copied());
    for (parameter, old) in frames.parameters.iter() {
        roots.push(*parameter);
        roots.push(*old);
    }
    for wind in frames.winds.iter() {
        roots.push(wind.before);
        roots.push(wind.after);
    }
}

/// Refreshes `engine_roots` (globals and interned symbols) from the live tables
/// when they have changed since the last refresh. Cheap when clean (two flag
/// checks), so it is safe to call before every collection point without
/// rescanning the tables on every native call.
pub(super) fn sync_engine_roots(
    heap: &mut Heap,
    globals: &crate::global::GlobalStore,
    symbols: &HashMap<String, Value>,
) {
    heap.sync_engine_roots(globals, symbols);
}

/// Cold and outlined: `bad(...)` calls sit on hot dispatch arms' failure
/// edges, and the `format!` machinery must not be inlined into them.
#[cold]
#[inline(never)]
pub(super) fn bad(what: &str) -> Error {
    Error::plain(ErrorKind::InvalidBytecode, format!("invalid {what}"))
}

/// Cold and outlined for the same reason as [`bad`]: this sits on the
/// `GetGlobal` arm's failure edge.
#[cold]
#[inline(never)]
pub(super) fn unbound_variable(name: &str) -> Error {
    Error::plain(
        ErrorKind::RuntimeError,
        format!("unbound variable '{name}'"),
    )
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::{
        bytecode::{Arity, Chunk, ExpectedResults},
        heap::{Heap, Object},
    };

    /// Builds a minimal one-frame continuation.
    fn make_continuation(
        procedure: Value,
        return_action: Option<Box<ReturnAction>>,
    ) -> Continuation {
        let chunk = Rc::new(Chunk {
            code: Vec::new(),
            constants: Vec::new(),
            global_operands: Vec::new(),
            closures: Vec::new(),
            cold: Vec::new(),
            arity: Arity::Exact(0),
            max_registers: 0,
            capture_kinds: Box::new([]),
            boxed_locals: Box::new([]),
        });
        let frame = Frame {
            chunk,
            pc: 0,
            base: 0,
            top: 0,
            return_base: 0,
            expected: ExpectedResults::One,
            captures: Rc::from(Vec::new()),
            procedure,
            return_action,
        };
        Continuation {
            frames: FrameStack {
                buffer: vec![frame],
                depth: 1,
                ..Default::default()
            },
            stack: RegisterStack(Vec::new()),
            handlers: Vec::new(),
            parameters: Vec::new(),
            parameter_values: Vec::new(),
            winds: Vec::new(),
            destination: 0,
            expected: ExpectedResults::One,
        }
    }

    /// Builds a minimal one-frame continuation, stores it on the heap, and
    /// returns the rooted continuation value.
    fn alloc_test_continuation(
        heap: &mut Heap,
        procedure: Value,
        return_action: Option<Box<ReturnAction>>,
    ) -> crate::Root {
        let continuation = make_continuation(procedure, return_action);
        let continuation_value = heap
            .alloc(Object::Continuation(Box::new(continuation)))
            .unwrap();
        heap.root(continuation_value)
    }

    /// A continuation reachable only through its heap object must keep the
    /// procedure objects its frames reference alive across a collection.
    /// Regression test for a heap-side tracer that skipped `frame.procedure`
    /// before `Continuation::trace` unified the two tracers.
    #[test]
    fn heap_continuation_keeps_frame_procedures_alive() {
        let limits = crate::Limits::default();
        let mut heap = Heap::new(&limits);

        let procedure = heap.alloc_vector(&[Value::integer(42)]).unwrap();
        let root = alloc_test_continuation(&mut heap, procedure, None);

        heap.collect();

        assert_eq!(
            heap.vector(procedure),
            Some(vec![Value::integer(42)]),
            "frame.procedure of a heap-stored continuation was collected"
        );
        drop(root);
    }

    /// A captured frame keeps its pending return action (`FrameStack::snapshot`
    /// preserves it, and `force` installs `StorePromise` on the thunk frame, so
    /// `(force (delay (call/cc ...)))` captures exactly this shape). The heap
    /// values such an action holds must survive a collection, exactly like
    /// [`gather_vm_roots`] keeps them alive for live frames. Regression test
    /// for continuation tracers that skipped `frame.return_action` before
    /// `Continuation::trace` unified them.
    #[test]
    fn heap_continuation_keeps_return_action_values_alive() {
        let limits = crate::Limits::default();
        let mut heap = Heap::new(&limits);

        let promise = heap.alloc_vector(&[Value::integer(7)]).unwrap();
        let action = Some(Box::new(ReturnAction::StorePromise {
            promise,
            flatten: false,
        }));
        let root = alloc_test_continuation(&mut heap, Value::integer(0), action);

        heap.collect();

        assert_eq!(
            heap.vector(promise),
            Some(vec![Value::integer(7)]),
            "a return-action value of a heap-stored continuation was collected"
        );
        drop(root);
    }

    /// A `ContinueTransfer` return action embeds a whole inner continuation,
    /// so [`Continuation::trace`] must recurse into it. Heap values held by
    /// the inner frames' pending actions must survive a collection of the
    /// outer heap-stored continuation.
    #[test]
    fn heap_continuation_traces_nested_transfer_continuations() {
        let limits = crate::Limits::default();
        let mut heap = Heap::new(&limits);

        let promise = heap.alloc_vector(&[Value::integer(9)]).unwrap();
        let inner = make_continuation(
            Value::integer(0),
            Some(Box::new(ReturnAction::StorePromise {
                promise,
                flatten: false,
            })),
        );
        let transfer = ReturnAction::ContinueTransfer(Box::new(ContinueTransferData {
            call_base: 0,
            thunks: Vec::new(),
            continuation: inner,
            values: Vec::new(),
        }));
        let root = alloc_test_continuation(&mut heap, Value::integer(0), Some(Box::new(transfer)));

        heap.collect();

        assert_eq!(
            heap.vector(promise),
            Some(vec![Value::integer(9)]),
            "a nested transfer continuation's return-action value was collected"
        );
        drop(root);
    }
}
