//! Direct execution of fixed-width register bytecode.

use std::{collections::HashMap, rc::Rc};

use crate::{
    CompiledModule, Error, ErrorKind, InterruptToken, Limits, Value, Values,
    bytecode::{Arity, CaptureSource, Chunk, ColdInstruction, ExpectedResults, Opcode, Word},
    heap::{
        CallableKind, Callee, ConditionKind, ErrorObject, Heap, Object, Parameter, Promise,
        PromiseState,
    },
};

mod arith;
mod call;
mod control;
mod frame;
mod gc;
mod ret;

use arith::*;
use call::*;
use control::*;
use frame::*;
pub(crate) use frame::{Closure, Continuation, Frame, RegisterStack, Results};
use gc::*;
use ret::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute(
    module: &CompiledModule,
    heap: &mut Heap,
    stack: &mut RegisterStack,
    globals: &mut crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    limits: &Limits,
    interrupt: &InterruptToken,
    config: &crate::EngineConfig,
    source_loader: &mut Option<Box<dyn crate::SourceLoader>>,
    sources: &mut crate::source::SourceMap,
) -> Result<crate::EvalOutcome, Error> {
    // Refresh the cached engine roots only when the tables changed since the
    // last refresh. Rebuilding unconditionally here cost a full copy of every
    // global and interned symbol per eval, a measurable tax on short programs.
    heap.sync_engine_roots(globals, symbols);
    // Hand rooting to the VM's precise safe-point scans for the duration of this
    // call; the guard reverts to host/native rooting on every return path.
    let _vm_guard = heap.enter_vm();
    let _scope = heap.scope();

    // Reset the reused register file to zero length, so no value
    // from a prior run is observed.
    stack.clear();
    stack.ensure(usize::from(module.entry.max_registers));

    let mut frames = FrameStack {
        buffer: vec![Frame {
            chunk: module.entry.clone(),
            pc: 0,
            base: 0,
            top: 0,
            return_base: 0,
            expected: ExpectedResults::All,
            captures: Rc::from([]),
            // The `undefined` sentinel is unreachable from user code (it only
            // exists as the global store's empty-slot marker and is filtered on
            // load), so no callee register can ever compare equal to it. Any
            // user-visible value here (e.g. `unspecified`) would let a
            // top-level call of that value pass the self-call guards and
            // re-enter the entry chunk instead of raising a call error.
            procedure: Value::undefined(),
            return_action: None,
        }],
        depth: 1,
        ..FrameStack::default()
    };
    let mut fuel = limits.fuel();
    // Instructions retired since the last cold safe point. Charged to `fuel` in
    // bulk when the cold handler runs, keeping fuel accounting accurate to
    // within an overshoot bounded by `COLD_INTERVAL` plus one inter-safe-point
    // segment. `retired` lives in a register (incremented every dispatch), so
    // the periodic check below is a compare against an immediate.
    let mut retired: u64 = 0;
    // How many retired instructions may pass between cold safe points. Bounds
    // interrupt latency and fuel overshoot. At ~5GHz this is on the order of a
    // microsecond of latency. Small enough that the (~tens of instructions)
    // cold handler amortizes to noise.
    const COLD_INTERVAL: u64 = 8192;
    // Run the cold handler once before the first instruction: an interrupt
    // raised (or an exit completed) before execution starts is observed
    // immediately, and a deferred collection from a previous eval is serviced.
    if let Some(status) = cold_safe_point(
        heap, globals, symbols, &*stack, &frames, interrupt, &mut fuel, 0,
    )? {
        return Ok(crate::EvalOutcome::Exited(status));
    }

    // A VM safe point: the fast path is one compare of the register-resident
    // `retired` counter against an immediate (the periodic cold entry, bounding
    // interrupt latency and fuel overshoot) plus one independent load of the
    // heap's fused trap flag (deferred collection, completed exit - serviced at
    // the very next safe point). Deliberately no read-modify-write here: a
    // memory counter would serialize consecutive safe points through
    // store-to-load forwarding on the call-heavy paths (measured on tak).
    // The cold handler services all conditions: completed exit, deferred
    // collection against a refreshed precise root snapshot, interrupt poll,
    // and the fuel charge accumulated since the previous cold entry.
    // This runs only at control-transfer sites (the outer re-establish below,
    // reached after every Call/TailCall/Return/Cold) and on backward branches
    // (loop back-edges), rather than before every instruction.
    // Collection is non-moving and never restructures `frames`/`stack`, so the
    // hoisted `chunk`/`code`/`base`/`pc` locals stay valid across an inline
    // back-edge safe point. Every loop crosses one of these sites each
    // iteration (it either tail-calls or takes a backward jump), so
    // GC/interrupt/fuel liveness is preserved with bounded latency.
    macro_rules! safe_point {
        () => {{
            if retired >= COLD_INTERVAL || heap.trap_pending() {
                if let Some(status) = cold_safe_point(
                    heap, globals, symbols, &*stack, &frames, interrupt, &mut fuel, retired,
                )? {
                    return Ok(crate::EvalOutcome::Exited(status));
                }
                retired = 0;
            }
        }};
    }

    // Two-level dispatch (Lua-style). The outer loop hoists the hot execution
    // state - the program counter, register base, and pointers to the running
    // chunk's code/constants - into loop locals. The inner loop then
    // dispatches every opcode using only those locals, keeping `pc` in a
    // register; the call/tail-call/return fast paths rebind the locals in
    // place and stay in the inner loop, so the outer loop only runs on entry
    // and after the residual slow transfers (generic calls, cold returns,
    // cold instructions, suspending natives).
    // Only `chunk` is hoisted with an `Rc::clone`, refreshed (pointer-compared
    // first) whenever the activation switches chunks. `captures` is read by
    // only the three capture opcodes below, so it is fetched lazily from the
    // (structurally stable) top frame there rather than cloned per call.
    let mut chunk = Rc::clone(&frames.last().expect("entry frame").chunk);
    // The hot `code`/`constants` views are raw pointer/length pairs rather
    // than borrows of `chunk`, so the in-loop call/return fast paths can
    // reassign `chunk` without fighting a long-lived borrow. `rebind_chunk!`
    // re-derives them at every reassignment; the pointers are only ever read
    // through the slices re-materialized at the top of the inner loop, while
    // the owning `chunk` Rc is live.
    let mut code_ptr: *const Word;
    let mut code_len: usize;
    let mut constants_ptr: *const Value;
    let mut constants_len: usize;
    macro_rules! rebind_chunk {
        () => {{
            code_ptr = chunk.code.as_ptr();
            code_len = chunk.code.len();
            constants_ptr = chunk.constants.as_ptr();
            constants_len = chunk.constants.len();
        }};
    }
    loop {
        // Control-transfer safe point: reached on entry and after every slow
        // control transfer (each breaks out of the inner loop to here); the
        // in-loop fast paths poll at their own transfer sites instead.
        safe_point!();
        let (mut base, mut pc) = {
            let frame = frames.last().expect("entry frame");
            if !Rc::ptr_eq(&chunk, &frame.chunk) {
                chunk = Rc::clone(&frame.chunk);
            }
            (frame.base, frame.pc)
        };
        rebind_chunk!();

        // Every frame's window is `ensure`d to `base + max_registers` on entry
        // and the register file is grow-only (never truncated), so the frame
        // always fits. Per-access reads/writes still bounds-check as a backstop.
        debug_assert!(
            base + usize::from(chunk.max_registers) <= stack.0.len(),
            "register frame exceeds stack length"
        );
        'inner: loop {
            // SAFETY: the pointers were derived (by `rebind_chunk!`) from the
            // `Chunk` owned by the live `chunk` Rc local. Rc heap data is
            // address-stable, a `Chunk` is never mutated after compilation,
            // and the pointers are re-derived at every `chunk` reassignment,
            // so they always view the current chunk's live allocations.
            #[allow(unsafe_code)]
            let code = unsafe { std::slice::from_raw_parts(code_ptr, code_len) };
            #[allow(unsafe_code)]
            let constants = unsafe { std::slice::from_raw_parts(constants_ptr, constants_len) };
            // Unchecked: the verifier proves every reachable pc is in bounds
            // (see `fetch_word`).
            let word = fetch_word(code, pc);
            pc += 1;
            retired += 1;
            let opcode = word.opcode_verified();
            // Monomorphized arithmetic/comparison arm body. Every per-opcode
            // match arm below expands its own copy with `$op` a compile-time
            // constant.
            macro_rules! numeric_arm {
                ($op:expr) => {{
                    let left = read_register(&*stack, base + usize::from(word.b()));
                    let right = if word.k() {
                        read_constant(constants, word.c() as usize)
                    } else {
                        read_register(&*stack, base + usize::from(word.c()))
                    };
                    let value = if let Some(value) = numeric_fast($op, left, right) {
                        value
                    } else {
                        numeric_slow($op, heap, globals, symbols, natives, left, right)?
                    };
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }};
            }
            // Fused compare-and-branch body, monomorphized the same way. The
            // comparison is delegated to the same numeric helpers as the
            // standalone `Numeric*` opcodes, so exact/inexact/NaN semantics
            // are identical. On a match the following `Jump` (guaranteed
            // present by the verifier) is skipped. Otherwise, it executes on
            // the next dispatch.
            macro_rules! test_arm {
                ($op:expr) => {{
                    let left = read_register(&*stack, base + usize::from(word.b()));
                    let right = if word.k() {
                        read_constant(constants, word.c() as usize)
                    } else {
                        read_register(&*stack, base + usize::from(word.c()))
                    };
                    let truth = if let Some(value) = numeric_fast($op, left, right) {
                        value
                    } else {
                        numeric_slow($op, heap, globals, symbols, natives, left, right)?
                    };
                    let taken = (truth == Value::boolean(true)) == (word.a() != 0);
                    // `pc` currently points at the following `Jump`, which the
                    // verifier guarantees exists.
                    let jump_word = fetch_word(code, pc);
                    pc += 1;
                    if taken {
                        let offset = jump_word.signed_jump();
                        // Verified in bounds; see the `Jump` arm.
                        pc = pc.wrapping_add_signed(offset);
                        if offset < 0 {
                            safe_point!();
                        }
                    }
                }};
            }
            // Fully fused counted-loop back-edge (jump-to-body layout), one
            // monomorphized expansion per compare kind: step the counter in
            // place (+1), re-run the loop header's exit test over B/RK(C)
            // (replicated verbatim by the emitter), and take the following
            // `Jump` straight back to the body start while the exit test stays
            // FALSE. One dispatch per iteration. On exit control falls through
            // to the `Jump -> header` emitted after the pair. The canonical
            // header test then re-confirms and falls through to the exit code
            // (a couple of extra dispatches, once per loop). Both miss paths
            // defer to the same helpers as the unfused `LoopBack`/`Test*`
            // words, so semantics are identical.
            macro_rules! loop_back_while_not_arm {
                ($op:expr) => {{
                    let counter_reg = base + usize::from(word.a());
                    let counter = read_register(&*stack, counter_reg);
                    let next = match counter
                        .as_fixnum()
                        .and_then(|current| current.checked_add(1))
                        .map(Value::integer)
                    {
                        Some(value) => value,
                        None => register_operation_slow(
                            Opcode::Add,
                            heap,
                            globals,
                            symbols,
                            natives,
                            &[counter, Value::integer(1)],
                        )?,
                    };
                    write_register(&mut *stack, counter_reg, next);
                    let left = read_register(&*stack, base + usize::from(word.b()));
                    let right = if word.k() {
                        read_constant(constants, word.c() as usize)
                    } else {
                        read_register(&*stack, base + usize::from(word.c()))
                    };
                    let truth = if let Some(value) = numeric_fast($op, left, right) {
                        value
                    } else {
                        numeric_slow($op, heap, globals, symbols, natives, left, right)?
                    };
                    let jump_word = fetch_word(code, pc);
                    pc += 1;
                    if truth != Value::boolean(true) {
                        // Loop continues: the body jump is always backward, so
                        // run the per-iteration safe point here.
                        pc = pc.wrapping_add_signed(jump_word.signed_jump());
                        safe_point!();
                    }
                }};
            }
            // Fused strided back-edge (fall-into-body layout): step the
            // counter in place by the step REGISTER `B` (the general add - the
            // stride may be any numeric), then re-run the loop guard with the
            // stepped counter as the left operand (`A` doubles as counter and
            // compare-left) and take the following `Jump` back to the body
            // start while the guard stays TRUE. The step reads `B` before the
            // counter write, so a `(+ i i)` doubling step sees the pre-step
            // value, exactly like the unfused argument evaluation.
            macro_rules! loop_back_step_while_arm {
                ($op:expr) => {{
                    let counter_reg = base + usize::from(word.a());
                    let counter = read_register(&*stack, counter_reg);
                    let step = read_register(&*stack, base + usize::from(word.b()));
                    let next = if let Some(value) = numeric_fast(Opcode::Add, counter, step) {
                        value
                    } else {
                        numeric_slow(Opcode::Add, heap, globals, symbols, natives, counter, step)?
                    };
                    write_register(&mut *stack, counter_reg, next);
                    let right = if word.k() {
                        read_constant(constants, word.c() as usize)
                    } else {
                        read_register(&*stack, base + usize::from(word.c()))
                    };
                    let truth = if let Some(value) = numeric_fast($op, next, right) {
                        value
                    } else {
                        numeric_slow($op, heap, globals, symbols, natives, next, right)?
                    };
                    let jump_word = fetch_word(code, pc);
                    pc += 1;
                    if truth == Value::boolean(true) {
                        // Loop continues: the body jump is always backward, so
                        // run the per-iteration safe point here.
                        pc = pc.wrapping_add_signed(jump_word.signed_jump());
                        safe_point!();
                    }
                }};
            }
            // Arithmetic specialized for a fixnum constant operand (proved by
            // the verifier): one tag check on the register side and a checked
            // payload operation. Misses defer to the same helper as the
            // unfused word (`numeric_fast` would miss a mixed pair anyway), so
            // values and errors are identical.
            macro_rules! numeric_fixnum_k_arm {
                ($op:expr) => {{
                    let left = read_register(&*stack, base + usize::from(word.b()));
                    let constant = read_constant(constants, word.c() as usize).fixnum_payload();
                    let checked = left.as_fixnum().and_then(|value| {
                        if $op == Opcode::Add {
                            value.checked_add(constant)
                        } else {
                            value.checked_sub(constant)
                        }
                    });
                    let value = match checked {
                        Some(value) => Value::integer(value),
                        None => numeric_slow(
                            $op,
                            heap,
                            globals,
                            symbols,
                            natives,
                            left,
                            Value::integer(constant),
                        )?,
                    };
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }};
            }
            // Comparison-and-branch specialized for a fixnum constant operand
            // (proved by the verifier): one tag check on the register side and
            // a raw payload compare. Misses defer to the same numeric helpers
            // as the unfused comparison, so semantics are identical.
            macro_rules! test_fixnum_arm {
                ($op:expr) => {{
                    let left = read_register(&*stack, base + usize::from(word.b()));
                    let constant = read_constant(constants, word.c() as usize).fixnum_payload();
                    let truth = match left.as_fixnum() {
                        Some(value) => {
                            if $op == Opcode::NumericLess {
                                value < constant
                            } else if $op == Opcode::NumericLessEqual {
                                value <= constant
                            } else {
                                value == constant
                            }
                        }
                        None => {
                            let truth = numeric_slow(
                                $op,
                                heap,
                                globals,
                                symbols,
                                natives,
                                left,
                                Value::integer(constant),
                            )?;
                            truth == Value::boolean(true)
                        }
                    };
                    let taken = truth == (word.a() != 0);
                    // `pc` currently points at the following `Jump`, which the
                    // verifier guarantees exists.
                    let jump_word = fetch_word(code, pc);
                    pc += 1;
                    if taken {
                        let offset = jump_word.signed_jump();
                        // Verified in bounds; see the `Jump` arm.
                        pc = pc.wrapping_add_signed(offset);
                        if offset < 0 {
                            safe_point!();
                        }
                    }
                }};
            }
            match opcode {
                // Data movement and constants.
                Opcode::Move => {
                    let value = read_register(&*stack, base + usize::from(word.b()));
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                Opcode::LoadK | Opcode::LoadKx => {
                    let index = if opcode == Opcode::LoadK {
                        word.bx()
                    } else {
                        // The verifier guarantees the `ExtraArg` successor.
                        let extra = fetch_word(code, pc);
                        pc += 1;
                        extra.ax_value()
                    };
                    let value = read_constant(constants, index as usize);
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                Opcode::ExtraArg => return Err(bad("orphan EXTRAARG")),
                // Global access.
                Opcode::GetGlobal => {
                    let operand = chunk
                        .global_operands
                        .get(word.bx() as usize)
                        .ok_or_else(|| bad("global"))?;
                    let id = operand.resolve(globals)?;
                    let value = globals
                        .load(id)
                        .ok_or_else(|| unbound_variable(&operand.name))?;
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                Opcode::SetGlobal => {
                    let operand = chunk
                        .global_operands
                        .get(word.bx() as usize)
                        .ok_or_else(|| bad("global"))?;
                    let id = operand.resolve(globals)?;
                    let value = read_register(&*stack, base + usize::from(word.a()));
                    // `store` records the mutation in the global store's own
                    // dirty flag, which the next engine-roots sync drains.
                    if !globals.store(id, value) {
                        return Err(bad("global slot"));
                    }
                }
                // Capture access.
                Opcode::GetCapture => {
                    let cell = *frames
                        .last()
                        .expect("frame")
                        .captures
                        .get(word.b() as usize)
                        .ok_or_else(|| bad("capture"))?;
                    let value = heap.boxed(cell).ok_or_else(|| bad("capture cell"))?;
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                Opcode::SetCapture => {
                    let value = read_register(&*stack, base + usize::from(word.a()));
                    let cell = *frames
                        .last()
                        .expect("frame")
                        .captures
                        .get(word.b() as usize)
                        .ok_or_else(|| bad("capture"))?;
                    if !heap.set_boxed(cell, value) {
                        return Err(bad("capture cell"));
                    }
                }
                Opcode::GetCaptureValue => {
                    // A never-mutated capture holds the raw value directly: one
                    // slice read, no heap cell dereference.
                    let value = *frames
                        .last()
                        .expect("frame")
                        .captures
                        .get(word.b() as usize)
                        .ok_or_else(|| bad("capture"))?;
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                // Local box (mutable cell) access.
                Opcode::GetLocalBox => {
                    // Read the value held by a boxed local's heap cell.
                    let cell = read_register(&*stack, base + usize::from(word.b()));
                    let value = heap.boxed(cell).ok_or_else(|| bad("local box cell"))?;
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                Opcode::SetLocalBox => {
                    // Store into a boxed local's heap cell, shared with any closures
                    // and continuations that captured it.
                    let value = read_register(&*stack, base + usize::from(word.b()));
                    let cell = read_register(&*stack, base + usize::from(word.a()));
                    if !heap.set_boxed(cell, value) {
                        return Err(bad("local box cell"));
                    }
                }
                Opcode::BoxLocal => {
                    // Wrap an inlined boxed local's current value in a fresh heap
                    // `Box` cell, in place. This is the mid-body analogue of
                    // `box_frame_locals`: an immediately-applied lambda's parameter
                    // that a nested closure captures (or that `set!` mutates) is
                    // boxed here, at the binding point, rather than at frame entry.
                    let slot = base + usize::from(word.a());
                    let raw = read_register(&*stack, slot);
                    let cell = heap.alloc(Object::Box(raw))?;
                    // Reachable from `stack[slot]` before the next safe point (the
                    // slot still holds `raw` across the alloc), so no explicit root.
                    write_register(&mut *stack, slot, cell);
                }
                // Closures.
                Opcode::Closure => {
                    let closure = create_closure(heap, &*stack, &frames, &chunk, word, base)?;
                    write_register(&mut *stack, base + usize::from(word.a()), closure);
                }
                Opcode::CaseLambda => {
                    let first = base + usize::from(word.b());
                    let procedure = create_case_lambda(heap, &*stack, first, word.c())?;
                    write_register(&mut *stack, base + usize::from(word.a()), procedure);
                }
                // Control flow and calls.
                Opcode::Jump => {
                    let offset = word.signed_jump();
                    // Exact, not merely wrapping: `jump_target` proves the
                    // target is computed overflow-free and lands in bounds.
                    pc = pc.wrapping_add_signed(offset);
                    // Backward jump = loop back-edge: run a safe point so a
                    // straight-line jump loop still honors GC/interrupt/fuel.
                    if offset < 0 {
                        safe_point!();
                    }
                }
                Opcode::JumpFalse => {
                    let value = read_register(&*stack, base + usize::from(word.a()));
                    if value == Value::boolean(false) {
                        let offset = word.sbx();
                        // Verified in bounds; see the `Jump` arm.
                        pc = pc.wrapping_add_signed(offset);
                        if offset < 0 {
                            safe_point!();
                        }
                    }
                }
                Opcode::Call => {
                    let call_base = base + usize::from(word.a());
                    let arguments = usize::from(word.b().saturating_sub(1));
                    // Verified decode: the verifier ran the fallible decoder on
                    // every `Call` word at load time.
                    let expected = ExpectedResults::from_call_field_verified(word.c());
                    frames.last_mut().expect("frame").pc = pc;
                    let procedure = read_register(&*stack, call_base);
                    // Self-call fast path first (the recursive-call shape): a
                    // pure frame-local compare, so the hottest call kind pays
                    // no heap classification at all. The callee is the running
                    // procedure, so the chunk, and with it the hoisted
                    // `code`/`constants` borrows, is unchanged by definition;
                    // the entry frame's `undefined` sentinel keeps this guard
                    // unreachable for non-procedure callees. Push the callee
                    // frame and continue in the inner loop, skipping the
                    // generic dispatch prologue and the outer-loop
                    // re-establish. Guarded to the infallible frame shape
                    // (exact matching arity, no rest list, no boxed locals).
                    // Anything else takes the paths below, which also own
                    // error routing.
                    if frames.last().expect("frame").procedure == procedure
                        && matches!(chunk.arity, Arity::Exact(_))
                        && chunk.arity.accepts(arguments)
                        && chunk.boxed_locals.is_empty()
                    {
                        push_self_frame(
                            &mut *stack,
                            &mut frames,
                            call_base,
                            arguments,
                            expected,
                            usize::from(chunk.max_registers),
                        );
                        base = call_base + 1;
                        pc = 0;
                        debug_assert!(
                            base + usize::from(chunk.max_registers) <= stack.0.len(),
                            "register frame exceeds stack length"
                        );
                        safe_point!();
                        continue 'inner;
                    }
                    // Everything else classifies the callee with one arena
                    // probe (closure and native both resolve here, instead of
                    // paying an always-miss native probe before the closure
                    // one on every Scheme-to-Scheme call).
                    match heap.callee(procedure) {
                        // General closure fast path: any plain closure with
                        // the same infallible shape (exact matching arity, no
                        // rest list, no boxed locals). Under this guard the
                        // generic path below is infallible and does nothing
                        // beyond what happens inline here, no `Closure`
                        // clone, and the recycled frame slot usually spares
                        // the `Rc` traffic entirely. The callee usually runs a
                        // different chunk, so rebind the hoisted chunk view in
                        // place and continue in the inner loop rather than
                        // paying the outer re-establish.
                        Callee::Closure(closure)
                            if matches!(closure.chunk.arity, Arity::Exact(_))
                                && closure.chunk.arity.accepts(arguments)
                                && closure.chunk.boxed_locals.is_empty() =>
                        {
                            prepare_general_call(
                                &mut *stack,
                                &mut frames,
                                call_base,
                                arguments,
                                expected,
                                procedure,
                                closure,
                            );
                            let callee_chunk = &frames.last().expect("frame").chunk;
                            if !Rc::ptr_eq(&chunk, callee_chunk) {
                                chunk = Rc::clone(callee_chunk);
                                rebind_chunk!();
                            }
                            base = call_base + 1;
                            pc = 0;
                            debug_assert!(
                                base + usize::from(chunk.max_registers) <= stack.0.len(),
                                "register frame exceeds stack length"
                            );
                            safe_point!();
                            continue 'inner;
                        }
                        // Native fast path: a native call never pushes a
                        // frame, so run it right here and stay in this
                        // activation. Skipping the generic dispatch prologue,
                        // the return delivery machinery, and the outer-loop
                        // re-establish entirely.
                        Callee::Native { id, fast } => {
                            match call_native_inline(
                                heap,
                                &mut *stack,
                                &mut frames,
                                globals,
                                symbols,
                                natives,
                                id,
                                fast,
                                call_base,
                                arguments,
                                expected,
                            )? {
                                NativeInline::Continue => continue,
                                NativeInline::Suspend(completed) => {
                                    if let Some(results) = completed {
                                        return Ok(outcome(heap, results));
                                    }
                                    break 'inner;
                                }
                            }
                        }
                        // Fallible closure shapes and the cold callable kinds
                        // fall through to the generic path below.
                        Callee::Closure(_) | Callee::Other => {}
                    }
                    let completed = match call(
                        heap,
                        &mut *stack,
                        &mut frames,
                        globals,
                        symbols,
                        natives,
                        call_base,
                        arguments,
                        expected,
                        false,
                        ReturnAction::Normal,
                    ) {
                        Ok(completed) => completed,
                        Err(error) => invoke_error_handler(
                            heap,
                            &mut *stack,
                            &mut frames,
                            globals,
                            symbols,
                            natives,
                            call_base,
                            expected,
                            error,
                        )?,
                    };
                    if let Some(results) = completed {
                        return Ok(outcome(heap, results));
                    }
                    break 'inner;
                }
                Opcode::TailCall => {
                    let call_base = base + usize::from(word.a());
                    let arguments = usize::from(word.b().saturating_sub(1));
                    frames.last_mut().expect("frame").pc = pc;
                    let procedure = read_register(&*stack, call_base);
                    // Self tail call (every named-let loop that is not
                    // LoopBack-fused): re-enter the running frame in place and
                    // continue in the inner loop; `base` and the chunk are
                    // unchanged by definition. Same infallible-shape guard as
                    // the `Call` fast path above.
                    if frames.last().expect("frame").procedure == procedure
                        && matches!(chunk.arity, Arity::Exact(_))
                        && chunk.arity.accepts(arguments)
                        && chunk.boxed_locals.is_empty()
                    {
                        prepare_tail_self_call(
                            heap,
                            &mut *stack,
                            &mut frames,
                            call_base,
                            arguments,
                        )?;
                        pc = 0;
                        safe_point!();
                        continue 'inner;
                    }
                    // General closure tail-call fast path: mirrors the `Call`
                    // fast path above (same infallible-shape guard, callee
                    // read in place, no `Closure` clone), re-entering the
                    // running frame with the callee's chunk and captures and
                    // rebinding the hoisted chunk view in place; `base` is
                    // unchanged (the frame is reused).
                    if let Some(closure) = heap.closure_ref(procedure)
                        && matches!(closure.chunk.arity, Arity::Exact(_))
                        && closure.chunk.arity.accepts(arguments)
                        && closure.chunk.boxed_locals.is_empty()
                    {
                        prepare_tail_general_call(
                            &mut *stack,
                            &mut frames,
                            call_base,
                            arguments,
                            procedure,
                            closure,
                        );
                        let callee = &frames.last().expect("frame").chunk;
                        if !Rc::ptr_eq(&chunk, callee) {
                            chunk = Rc::clone(callee);
                            rebind_chunk!();
                        }
                        pc = 0;
                        safe_point!();
                        continue 'inner;
                    }
                    // Native tail-call fast path: skip the generic `call`
                    // prologue and deliver through the popped frame's return
                    // slot. The frame changes, so control re-establishes
                    // through the outer loop like the generic path below.
                    if let Some((id, fast)) = heap.native_callee(procedure) {
                        let completed = match tail_call_native_inline(
                            heap,
                            &mut *stack,
                            &mut frames,
                            globals,
                            symbols,
                            natives,
                            id,
                            fast,
                            call_base,
                            arguments,
                        ) {
                            Ok(completed) => completed,
                            Err(error) => invoke_error_handler(
                                heap,
                                &mut *stack,
                                &mut frames,
                                globals,
                                symbols,
                                natives,
                                call_base,
                                ExpectedResults::All,
                                error,
                            )?,
                        };
                        if let Some(results) = completed {
                            return Ok(outcome(heap, results));
                        }
                        break 'inner;
                    }
                    let completed = match call(
                        heap,
                        &mut *stack,
                        &mut frames,
                        globals,
                        symbols,
                        natives,
                        call_base,
                        arguments,
                        ExpectedResults::All,
                        true,
                        ReturnAction::Normal,
                    ) {
                        Ok(completed) => completed,
                        Err(error) => invoke_error_handler(
                            heap,
                            &mut *stack,
                            &mut frames,
                            globals,
                            symbols,
                            natives,
                            call_base,
                            ExpectedResults::All,
                            error,
                        )?,
                    };
                    if let Some(results) = completed {
                        return Ok(outcome(heap, results));
                    }
                    break 'inner;
                }
                Opcode::Return => {
                    let first = base + usize::from(word.a());
                    let count = if word.b() == 0 {
                        frames.last().expect("frame").top.saturating_sub(first)
                    } else {
                        usize::from(word.b() - 1)
                    };
                    let completed = frames.pop_frame();
                    if frames.is_empty() {
                        return Ok(crate::EvalOutcome::Values(Values::new(
                            results_from_slice(&stack[first..first + count])
                                .into_vec()
                                .into_iter()
                                .map(|value| heap.root(value))
                                .collect(),
                        )));
                    }
                    let caller = frames.last().expect("caller");
                    // Grow-only: re-extend the caller's window (a no-op unless a
                    // continuation restore left the stack short) but never shrink,
                    // avoiding per-return resize churn. The precise root scan reads
                    // only live windows, so the dead tail is never observed.
                    stack.ensure(caller.base + usize::from(caller.chunk.max_registers));
                    match completed.return_action {
                        // Common case: an ordinary (`Normal`) return.
                        None => {
                            deliver_return_fast(
                                &mut *stack,
                                &mut frames,
                                completed.return_base,
                                completed.expected,
                                first,
                                count,
                            )?;
                            // Rebind the frame locals - and, for a cross-chunk
                            // caller, the hoisted chunk view - and continue in
                            // the inner loop; an ordinary return never needs
                            // the outer re-establish.
                            let caller = frames.last().expect("caller");
                            if !Rc::ptr_eq(&caller.chunk, &chunk) {
                                chunk = Rc::clone(&caller.chunk);
                                rebind_chunk!();
                            }
                            base = caller.base;
                            pc = caller.pc;
                            safe_point!();
                            continue 'inner;
                        }
                        // Cold return actions (multiple-value consumers, dynamic-wind,
                        // promises, parameterize, exceptions, continuation transfer).
                        Some(action) => {
                            let results = results_from_slice(&stack[first..first + count]);
                            if let Some(results) = complete_return(
                                heap,
                                &mut *stack,
                                &mut frames,
                                globals,
                                symbols,
                                natives,
                                completed.return_base,
                                completed.expected,
                                *action,
                                results,
                            )? {
                                return Ok(outcome(heap, results));
                            }
                        }
                    }
                    break 'inner;
                }
                Opcode::Cold => {
                    // The resume pc must be visible to the frame before any cold
                    // handling: `CaptureContinuation` snapshots the frame stack and
                    // the snapshot must record the post-instruction pc.
                    frames.last_mut().expect("frame").pc = pc;
                    if let Some(results) = execute_cold_instruction(
                        heap,
                        &mut *stack,
                        &mut frames,
                        globals,
                        symbols,
                        natives,
                        config,
                        source_loader,
                        sources,
                        &chunk,
                        word,
                        base,
                    )? {
                        return Ok(outcome(heap, results));
                    }
                    break 'inner;
                }
                // Arithmetic.
                Opcode::Add => numeric_arm!(Opcode::Add),
                Opcode::Subtract => numeric_arm!(Opcode::Subtract),
                Opcode::Multiply => numeric_arm!(Opcode::Multiply),
                Opcode::Divide => numeric_arm!(Opcode::Divide),
                // Numeric comparison.
                Opcode::NumericEqual => numeric_arm!(Opcode::NumericEqual),
                Opcode::NumericLess => numeric_arm!(Opcode::NumericLess),
                Opcode::NumericLessEqual => numeric_arm!(Opcode::NumericLessEqual),
                Opcode::NumericGreater => numeric_arm!(Opcode::NumericGreater),
                Opcode::NumericGreaterEqual => numeric_arm!(Opcode::NumericGreaterEqual),
                // Pair and list primitives.
                Opcode::Cons => {
                    // Inline `cons`: `alloc` never collects inline while the VM
                    // manages roots (it only arms a deferred collection), and the
                    // new pair is written straight into a register before the next
                    // safe point, so the unrooted operands stay reachable. A heap
                    // limit yields the identical `HeapLimitExceeded` as the native.
                    let car = read_register(&*stack, base + usize::from(word.b()));
                    let cdr = read_register(&*stack, base + usize::from(word.c()));
                    let value = heap.alloc_pair(car, cdr)?;
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                Opcode::Car => {
                    // Inline fast path; a non-pair defers to the generic `car`
                    // native, which raises the same type error.
                    let target = read_register(&*stack, base + usize::from(word.b()));
                    let value = match heap.pair(target) {
                        Some((car, _)) => car,
                        None => register_operation_slow(
                            opcode,
                            heap,
                            globals,
                            symbols,
                            natives,
                            &[target],
                        )?,
                    };
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                Opcode::Cdr => {
                    let target = read_register(&*stack, base + usize::from(word.b()));
                    let value = match heap.pair(target) {
                        Some((_, cdr)) => cdr,
                        None => register_operation_slow(
                            opcode,
                            heap,
                            globals,
                            symbols,
                            natives,
                            &[target],
                        )?,
                    };
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                Opcode::NullP => {
                    // `null?`/`pair?` never error, so they compute fully inline.
                    let target = read_register(&*stack, base + usize::from(word.b()));
                    let value = Value::boolean(target == Value::nil());
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                Opcode::PairP => {
                    let target = read_register(&*stack, base + usize::from(word.b()));
                    let value = Value::boolean(heap.pair(target).is_some());
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                // Vector primitives.
                Opcode::VectorRef => {
                    let vector = read_register(&*stack, base + usize::from(word.b()));
                    let index = read_register(&*stack, base + usize::from(word.c()));
                    let value = match index.as_fixnum() {
                        Some(index) if index >= 0 => heap.vector_ref(vector, index as usize),
                        _ => None,
                    };
                    let value = match value {
                        Some(value) => value,
                        None => register_operation_slow(
                            opcode,
                            heap,
                            globals,
                            symbols,
                            natives,
                            &[vector, index],
                        )?,
                    };
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                Opcode::VectorSet => {
                    let vector = read_register(&*stack, base + usize::from(word.b()));
                    let index = read_register(&*stack, base + usize::from(word.c()));
                    let value = read_register(&*stack, base + usize::from(word.c()) + 1);

                    // Inline fast path mirroring `VectorRef`: `heap.vector_set`
                    // already refuses (returns `false`, without mutating) on an
                    // immutable slot, a non-vector, or an out-of-range index, so a
                    // hit here is exactly the success case. Any miss (refused, or a
                    // non-fixnum/negative index) defers to the generic
                    // `vector-set!` native, which raises the appropriate error.
                    let result = match index.as_fixnum() {
                        Some(index)
                            if index >= 0 && heap.vector_set(vector, index as usize, value) =>
                        {
                            Value::unspecified()
                        }
                        _ => register_operation_slow(
                            opcode,
                            heap,
                            globals,
                            symbols,
                            natives,
                            &[vector, index, value],
                        )?,
                    };
                    write_register(&mut *stack, base + usize::from(word.a()), result);
                }
                // String and char primitives.
                Opcode::StringRef => {
                    // Inline fast path mirroring `VectorRef`. Any miss (non-string,
                    // out-of-range, or non-fixnum/negative index) defers to the
                    // generic `string-ref` native, which raises the same error.
                    let string = read_register(&*stack, base + usize::from(word.b()));
                    let index = read_register(&*stack, base + usize::from(word.c()));
                    let value = match index.as_fixnum() {
                        Some(index) if index >= 0 => heap.string_ref(string, index as usize),
                        _ => None,
                    };
                    let value = match value {
                        Some(character) => Value::character(character),
                        None => register_operation_slow(
                            opcode,
                            heap,
                            globals,
                            symbols,
                            natives,
                            &[string, index],
                        )?,
                    };
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                Opcode::StringLength => {
                    // The `i64::try_from` mirrors the native's `length` conversion,
                    // so absurd lengths take the same fallback error path.
                    let target = read_register(&*stack, base + usize::from(word.b()));
                    let length = heap
                        .string_len(target)
                        .and_then(|length| i64::try_from(length).ok());
                    let value = match length {
                        Some(length) => Value::integer(length),
                        None => register_operation_slow(
                            opcode,
                            heap,
                            globals,
                            symbols,
                            natives,
                            &[target],
                        )?,
                    };
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                Opcode::CharToInteger => {
                    // Characters are immediate values: a hit never touches the heap.
                    let target = read_register(&*stack, base + usize::from(word.b()));
                    let value = match target.decode() {
                        crate::value::ValueRepr::Character(character) => {
                            Value::integer(i64::from(character as u32))
                        }
                        _ => register_operation_slow(
                            opcode,
                            heap,
                            globals,
                            symbols,
                            natives,
                            &[target],
                        )?,
                    };
                    write_register(&mut *stack, base + usize::from(word.a()), value);
                }
                // Compare-and-branch.
                Opcode::TestEqual => test_arm!(Opcode::NumericEqual),
                Opcode::TestLess => test_arm!(Opcode::NumericLess),
                Opcode::TestLessEqual => test_arm!(Opcode::NumericLessEqual),
                Opcode::TestGreater => test_arm!(Opcode::NumericGreater),
                Opcode::TestGreaterEqual => test_arm!(Opcode::NumericGreaterEqual),
                Opcode::TestNull | Opcode::TestPair => {
                    // Fused predicate-and-branch: `null?`/`pair?` never error, so
                    // the truth value computes fully inline (same checks as the
                    // standalone `NullP`/`PairP` arms). The following `Jump`
                    // (guaranteed present by the verifier) is consumed like in
                    // the comparison `Test*` family.
                    let target = read_register(&*stack, base + usize::from(word.b()));
                    let truth = match opcode {
                        Opcode::TestNull => target == Value::nil(),
                        _ => heap.pair(target).is_some(),
                    };
                    let taken = truth == (word.a() != 0);
                    let jump_word = fetch_word(code, pc);
                    pc += 1;
                    if taken {
                        let offset = jump_word.signed_jump();
                        // Verified in bounds; see the `Jump` arm.
                        pc = pc.wrapping_add_signed(offset);
                        if offset < 0 {
                            safe_point!();
                        }
                    }
                }
                Opcode::TestVectorRef => {
                    // Fused `(vector-ref v i)`-as-condition branch: fetch the
                    // element with the same fast path as the standalone
                    // `VectorRef` arm (identical errors via the same slow
                    // path), then branch on Scheme truthiness (anything but
                    // `#f` - exactly `JumpFalse`'s test), consuming the
                    // following `Jump` like the rest of the `Test*` family.
                    let vector = read_register(&*stack, base + usize::from(word.b()));
                    let index = read_register(&*stack, base + usize::from(word.c()));
                    let element = match index.as_fixnum() {
                        Some(index) if index >= 0 => heap.vector_ref(vector, index as usize),
                        _ => None,
                    };
                    let element = match element {
                        Some(value) => value,
                        None => register_operation_slow(
                            Opcode::VectorRef,
                            heap,
                            globals,
                            symbols,
                            natives,
                            &[vector, index],
                        )?,
                    };
                    let truth = element != Value::boolean(false);
                    let taken = truth == (word.a() != 0);
                    let jump_word = fetch_word(code, pc);
                    pc += 1;
                    if taken {
                        let offset = jump_word.signed_jump();
                        // Verified in bounds; see the `Jump` arm.
                        pc = pc.wrapping_add_signed(offset);
                        if offset < 0 {
                            safe_point!();
                        }
                    }
                }
                // Fused loop back-edges.
                Opcode::LoopBack => {
                    // Fused loop counter step + back-edge for a flattened counting
                    // loop: increment the counter register in place by the signed
                    // step in `C`, then take the mandatory following `Jump` (always
                    // a backward branch). The accumulator parameters are already
                    // updated by the preceding `Move`s. The counter is stepped last
                    // so those updates still read its old value.
                    let counter_reg = base + usize::from(word.a());
                    let step = word.c() as i8 as i64;
                    let counter = read_register(&*stack, counter_reg);
                    let next = match counter
                        .as_fixnum()
                        .and_then(|current| current.checked_add(step))
                        .map(Value::integer)
                    {
                        Some(value) => value,
                        // Counter is a heap number or the increment overflows
                        // i64: defer to the general add so it becomes a
                        // heap-backed exact integer (or raises on i128 overflow).
                        None => register_operation_slow(
                            Opcode::Add,
                            heap,
                            globals,
                            symbols,
                            natives,
                            &[counter, Value::integer(step)],
                        )?,
                    };
                    write_register(&mut *stack, counter_reg, next);
                    // Consume the following `Jump` inline (guaranteed present by
                    // the verifier); the back-edge is always backward, so run a
                    // safe point (one per loop iteration).
                    let jump_word = fetch_word(code, pc);
                    pc += 1;
                    let offset = jump_word.signed_jump();
                    // Verified in bounds; see the `Jump` arm.
                    pc = pc.wrapping_add_signed(offset);
                    safe_point!();
                }
                Opcode::LoopBackWhileNotEqual => {
                    loop_back_while_not_arm!(Opcode::NumericEqual)
                }
                Opcode::LoopBackWhileNotLess => loop_back_while_not_arm!(Opcode::NumericLess),
                Opcode::LoopBackWhileNotLessEqual => {
                    loop_back_while_not_arm!(Opcode::NumericLessEqual)
                }
                Opcode::LoopBackStepWhileLess => loop_back_step_while_arm!(Opcode::NumericLess),
                Opcode::LoopBackStepWhileLessEqual => {
                    loop_back_step_while_arm!(Opcode::NumericLessEqual)
                }
                // Fused accumulates.
                // The fused accumulate bodies live out-of-line (see
                // `fused_add_vector_ref` for the rationale: inlined, their
                // live ranges cost the dispatch loop a register).
                Opcode::AddVectorRef => {
                    fused_add_vector_ref(&mut *stack, heap, globals, symbols, natives, word, base)?;
                }
                Opcode::AddCar => {
                    fused_add_car(&mut *stack, heap, globals, symbols, natives, word, base)?;
                }
                Opcode::AddStringRefCode => {
                    fused_add_string_ref_code(
                        &mut *stack,
                        heap,
                        globals,
                        symbols,
                        natives,
                        word,
                        base,
                    )?;
                }
                Opcode::AddMul | Opcode::SubMul => {
                    // Inline all-float fast path (the mandelbrot steady state,
                    // where two out-of-line calls per iteration eat the fused
                    // dispatch win): `rA = rA +- (rB * RK(C))` computed in
                    // `f64` end to end with one raw rebox. Any other shape
                    // defers to the decomposed helper, which re-reads the
                    // operands (pure register/constant reads, so the re-run is
                    // unobservable) and owns error identity.
                    let left = read_register(&*stack, base + usize::from(word.b()));
                    let right = if word.k() {
                        read_constant(constants, word.c() as usize)
                    } else {
                        read_register(&*stack, base + usize::from(word.c()))
                    };
                    let accumulator = read_register(&*stack, base + usize::from(word.a()));
                    let key = Value::pair_key(left, right);
                    let fused = if key == Value::PAIR_BOTH_FLOAT {
                        accumulator.as_float().map(|acc| {
                            let product = left.float_payload() * right.float_payload();
                            Value::float_raw(if opcode == Opcode::AddMul {
                                acc + product
                            } else {
                                acc - product
                            })
                        })
                    } else if key == Value::PAIR_BOTH_FIXNUM {
                        // The all-fixnum chain (the matrix steady state):
                        // checked multiply then checked accumulate, deferring
                        // to the helper on overflow, which recomputes through
                        // the identical wide chain.
                        accumulator.as_fixnum().and_then(|acc| {
                            let product =
                                left.fixnum_payload().checked_mul(right.fixnum_payload())?;
                            if opcode == Opcode::AddMul {
                                acc.checked_add(product)
                            } else {
                                acc.checked_sub(product)
                            }
                            .map(Value::integer)
                        })
                    } else {
                        None
                    };
                    if let Some(result) = fused {
                        write_register(&mut *stack, base + usize::from(word.a()), result);
                    } else {
                        fused_mul_step(
                            opcode,
                            &mut *stack,
                            heap,
                            globals,
                            symbols,
                            natives,
                            constants,
                            word,
                            base,
                        )?;
                    }
                }
                Opcode::AddMulVectorRef => {
                    // The verifier guarantees the `ExtraArg` successor.
                    let extra = fetch_word(code, pc);
                    pc += 1;
                    // Inline fast path (the float_dot/matrix steady states,
                    // where an out-of-line call per back-edge would eat the
                    // fused dispatch win): both element fetches, then the
                    // all-float chain or the two-step fixnum chain. Any miss
                    // re-runs the whole word in the decomposed helper, whose
                    // steps defer to the same natives as the unfused words in
                    // the same order (the fetches are pure reads, so the
                    // re-run is unobservable).
                    let first_vector = read_register(&*stack, base + usize::from(word.b()));
                    let first_index = read_register(&*stack, base + usize::from(word.c()));
                    let packed = extra.ax_value();
                    let second_vector = read_register(&*stack, base + (packed >> 8) as usize);
                    let second_index = read_register(&*stack, base + (packed & 0xFF) as usize);
                    let elements = match (first_index.as_fixnum(), second_index.as_fixnum()) {
                        (Some(i), Some(j)) if i >= 0 && j >= 0 => {
                            match heap.vector_ref(first_vector, i as usize) {
                                Some(first) => heap
                                    .vector_ref(second_vector, j as usize)
                                    .map(|second| (first, second)),
                                None => None,
                            }
                        }
                        _ => None,
                    };
                    let target = base + usize::from(word.a());
                    let value = elements.and_then(|(first, second)| {
                        let accumulator = read_register(&*stack, target);
                        if let (Some(l), Some(r), Some(acc)) =
                            (first.as_float(), second.as_float(), accumulator.as_float())
                        {
                            return Some(Value::float_raw(acc + l * r));
                        }
                        numeric_fast(Opcode::Multiply, first, second)
                            .and_then(|product| numeric_fast(Opcode::Add, accumulator, product))
                    });
                    match value {
                        Some(value) => write_register(&mut *stack, target, value),
                        None => fused_add_mul_vector_ref(
                            &mut *stack,
                            heap,
                            globals,
                            symbols,
                            natives,
                            word,
                            extra,
                            base,
                        )?,
                    }
                }
                Opcode::VectorRefVectorRef => {
                    // The verifier guarantees the `ExtraArg` successor.
                    let extra = fetch_word(code, pc);
                    pc += 1;
                    // Inline fast path mirroring the standalone `VectorRef`
                    // arm twice. Any miss re-runs the word in the decomposed
                    // helper (pure reads, so the re-run is unobservable).
                    let vector = read_register(&*stack, base + usize::from(word.b()));
                    let index = read_register(&*stack, base + usize::from(word.c()));
                    let outer_index = read_register(&*stack, base + extra.ax_value() as usize);
                    let element = match (index.as_fixnum(), outer_index.as_fixnum()) {
                        (Some(i), Some(j)) if i >= 0 && j >= 0 => heap
                            .vector_ref(vector, i as usize)
                            .and_then(|row| heap.vector_ref(row, j as usize)),
                        _ => None,
                    };
                    match element {
                        Some(value) => {
                            write_register(&mut *stack, base + usize::from(word.a()), value);
                        }
                        None => fused_vector_ref_vector_ref(
                            &mut *stack,
                            heap,
                            globals,
                            symbols,
                            natives,
                            word,
                            extra,
                            base,
                        )?,
                    }
                }
                // Fixnum-constant specializations.
                Opcode::AddFixnumK => numeric_fixnum_k_arm!(Opcode::Add),
                Opcode::SubtractFixnumK => numeric_fixnum_k_arm!(Opcode::Subtract),
                Opcode::AddSubFixnumK => {
                    // The fused wide-literal accumulate `rA = (rA + K[B]) -
                    // K[C]`, both constants proved inline fixnums by the
                    // verifier. The register is written once, after both steps
                    // succeed. A miss on either checked step defers the whole
                    // word to the out-of-line helper, which re-runs the exact
                    // unfused chain in order (reads are pure, so the re-run is
                    // unobservable).
                    let register = base + usize::from(word.a());
                    let value = read_register(&*stack, register);
                    let add = read_constant(constants, word.b() as usize).fixnum_payload();
                    let subtract = read_constant(constants, word.c() as usize).fixnum_payload();
                    let result = match value
                        .as_fixnum()
                        .and_then(|current| current.checked_add(add))
                        .and_then(|sum| sum.checked_sub(subtract))
                    {
                        Some(result) => Value::integer(result),
                        None => fused_add_sub_fixnum_k(
                            heap, globals, symbols, natives, value, add, subtract,
                        )?,
                    };
                    write_register(&mut *stack, register, result);
                }
                Opcode::TestLessFixnum => test_fixnum_arm!(Opcode::NumericLess),
                Opcode::TestLessEqualFixnum => test_fixnum_arm!(Opcode::NumericLessEqual),
                Opcode::TestEqualFixnum => test_fixnum_arm!(Opcode::NumericEqual),
                Opcode::LoopBackWhileNotEqualFixnum => {
                    // `LoopBackWhileNotEqual` specialized for a fixnum constant
                    // limit (proved by the verifier). The exit test needs no
                    // constant re-classification: one tag check on the counter
                    // side and a raw payload compare. A non-fixnum left operand
                    // defers to the same numeric equality as the unfused test
                    // (`numeric_fast` would miss the mixed pair anyway, so the
                    // semantics are bit-identical).
                    let counter_reg = base + usize::from(word.a());
                    let counter = read_register(&*stack, counter_reg);
                    let next = match counter
                        .as_fixnum()
                        .and_then(|current| current.checked_add(1))
                        .map(Value::integer)
                    {
                        Some(value) => value,
                        None => register_operation_slow(
                            Opcode::Add,
                            heap,
                            globals,
                            symbols,
                            natives,
                            &[counter, Value::integer(1)],
                        )?,
                    };
                    write_register(&mut *stack, counter_reg, next);
                    let left = read_register(&*stack, base + usize::from(word.b()));
                    let limit = read_constant(constants, word.c() as usize).fixnum_payload();
                    let continues = match left.as_fixnum() {
                        Some(value) => value != limit,
                        None => {
                            let truth = numeric_slow(
                                Opcode::NumericEqual,
                                heap,
                                globals,
                                symbols,
                                natives,
                                left,
                                Value::integer(limit),
                            )?;
                            truth != Value::boolean(true)
                        }
                    };
                    let jump_word = fetch_word(code, pc);
                    pc += 1;
                    if continues {
                        // Loop continues: the body jump is always backward, so
                        // run the per-iteration safe point here.
                        pc = pc.wrapping_add_signed(jump_word.signed_jump());
                        safe_point!();
                    }
                }
            }
        }
    }
}

/// Builds a closure object for the `Closure` opcode. Outlined so the capture
/// vector and allocation machinery stay out of the dispatch loop.
#[inline(never)]
fn create_closure(
    heap: &mut Heap,
    stack: &RegisterStack,
    frames: &FrameStack,
    chunk: &Chunk,
    word: Word,
    base: usize,
) -> Result<Value, Error> {
    let prototype = chunk
        .closures
        .get(word.bx() as usize)
        .cloned()
        .ok_or_else(|| bad("closure"))?;
    let mut new_captures = Vec::with_capacity(prototype.captures.len());
    for source in prototype.captures {
        new_captures.push(match source {
            CaptureSource::Capture(index) => *frames
                .last()
                .expect("frame")
                .captures
                .get(index as usize)
                .ok_or_else(|| bad("capture"))?,
            CaptureSource::Local(index) => {
                // The slot holds whatever representation the binding's kind
                // dictates: a shared heap cell for mutated locals (boxed at
                // frame entry or by `BoxLocal`), or the raw value for
                // immutable ones. Either is captured verbatim.
                stack.get(base + usize::from(index))?
            }
        });
    }
    heap.alloc(Object::Closure(Closure {
        chunk: prototype.chunk,
        captures: Rc::from(new_captures),
    }))
}

/// Builds a case-lambda dispatcher for the `CaseLambda` opcode; outlined for
/// the same reason as [`create_closure`].
#[inline(never)]
fn create_case_lambda(
    heap: &mut Heap,
    stack: &RegisterStack,
    first: usize,
    count: u8,
) -> Result<Value, Error> {
    let clauses = stack[first..first + usize::from(count)].to_vec();
    heap.alloc(Object::CaseLambda(clauses))
}

/// Executes one `Cold` instruction out of line. Cold instructions are the
/// rare, heavy operations (continuations, dynamic-wind, parameterize, ports,
/// `load`, heap number literals, ...); keeping their code and register
/// pressure out of `execute` lets the dispatch loop keep its hot state (pc,
/// base, code pointer) in registers. Returns `Ok(Some(results))` when
/// execution completed with final results and `Ok(None)` when the dispatch
/// loop should re-establish its frame state and continue.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn execute_cold_instruction(
    heap: &mut Heap,
    stack: &mut RegisterStack,
    frames: &mut FrameStack,
    globals: &mut crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    config: &crate::EngineConfig,
    source_loader: &mut Option<Box<dyn crate::SourceLoader>>,
    sources: &mut crate::source::SourceMap,
    chunk: &Chunk,
    word: Word,
    base: usize,
) -> Result<Option<Results>, Error> {
    let instruction = chunk
        .cold
        .get(word.bx() as usize)
        .cloned()
        .ok_or_else(|| bad("cold operand"))?;
    let completed = match instruction {
        ColdInstruction::CallWithValues {
            destination,
            producer,
            consumer,
            expected,
        } => {
            let destination = base + usize::from(destination);
            let producer = read_register(stack, base + usize::from(producer));
            let consumer = read_register(stack, base + usize::from(consumer));
            stack.ensure(destination + 1);
            stack[destination] = producer;
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
                ReturnAction::InvokeConsumer(consumer),
            )?
        }
        ColdInstruction::MakePromise {
            destination,
            thunk,
            flatten,
        } => {
            let thunk = read_register(stack, base + usize::from(thunk));
            let promise = heap.alloc(Object::Promise(Promise {
                state: PromiseState::Pending { thunk, flatten },
            }))?;
            write_register(stack, base + usize::from(destination), promise);
            frames.last_mut().expect("frame").top = base + usize::from(destination) + 1;
            None
        }
        ColdInstruction::Force {
            destination,
            promise,
            expected,
        } => {
            let promise = read_register(stack, base + usize::from(promise));
            force_promise(
                heap,
                stack,
                frames,
                globals,
                symbols,
                natives,
                base + usize::from(destination),
                expected,
                promise,
            )?
        }
        ColdInstruction::CaptureContinuation {
            destination,
            procedure,
            expected,
        } => {
            let destination = base + usize::from(destination);
            let procedure = read_register(stack, base + usize::from(procedure));
            // Snapshot only the live prefix; returns re-extend each
            // caller's window on the way back out, so the dead tail
            // (and any deeper frames' windows) need not be copied.
            let live_top = live_register_top(frames);
            let continuation = heap.alloc(Object::Continuation(Box::new(Continuation {
                frames: frames.snapshot(),
                stack: stack.snapshot(live_top),
                handlers: frames.handlers.clone(),
                parameters: frames.parameters.clone(),
                parameter_values: frames
                    .parameters
                    .iter()
                    .map(|(parameter, _)| {
                        heap.parameter(*parameter)
                            .map(|value| (*parameter, value))
                            .ok_or_else(|| bad("parameter"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                winds: frames.winds.clone(),
                destination,
                expected,
            })))?;
            stack.ensure(destination + 2);
            stack[destination] = procedure;
            stack[destination + 1] = continuation;
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
                ReturnAction::Normal,
            )?
        }
        ColdInstruction::MakeParameter {
            destination,
            initial,
            converter,
        } => {
            let destination = base + usize::from(destination);
            let initial = read_register(stack, base + usize::from(initial));
            if let Some(converter) = converter {
                let converter = read_register(stack, base + usize::from(converter));
                stack.ensure(destination + 2);
                stack[destination] = converter;
                stack[destination + 1] = initial;
                call(
                    heap,
                    stack,
                    frames,
                    globals,
                    symbols,
                    natives,
                    destination,
                    1,
                    ExpectedResults::One,
                    false,
                    ReturnAction::CreateParameter { converter },
                )?
            } else {
                let parameter = heap.alloc(Object::Parameter(Box::new(Parameter {
                    value: initial,
                    converter: None,
                })))?;
                write_register(stack, destination, parameter);
                frames.last_mut().expect("frame").top = destination + 1;
                None
            }
        }
        ColdInstruction::PushParameters { first, count } => {
            let call_base = base + usize::from(first);
            let mut bindings = Vec::with_capacity(usize::from(count));
            for offset in 0..usize::from(count) {
                let parameter = read_register(stack, base + usize::from(first) + offset * 2);
                let value = read_register(stack, base + usize::from(first) + offset * 2 + 1);
                let old = heap
                    .parameter(parameter)
                    .ok_or_else(|| Error::plain(ErrorKind::TypeError, "expected parameter"))?;
                bindings.push((parameter, old, value));
            }
            continue_parameter_bindings(
                heap,
                stack,
                frames,
                globals,
                symbols,
                natives,
                call_base,
                bindings,
                Vec::new(),
            )?
        }
        ColdInstruction::PopParameters { count } => {
            for _ in 0..count {
                let (parameter, old) = frames
                    .parameters
                    .pop()
                    .ok_or_else(|| bad("parameter stack"))?;
                if !heap.set_parameter(parameter, old) {
                    return Err(bad("parameter"));
                }
            }
            None
        }
        ColdInstruction::MakeError {
            destination,
            message,
            first_irritant,
            count,
        } => {
            let message = read_register(stack, base + usize::from(message));
            if heap.string_slice(message).is_none() {
                return Err(Error::plain(
                    ErrorKind::TypeError,
                    "error message must be a string",
                ));
            }
            let irritants: Vec<Value> = (0..usize::from(count))
                .map(|offset| read_register(stack, base + usize::from(first_irritant) + offset))
                .collect();
            let error = heap.alloc(Object::Error(Box::new(ErrorObject {
                message,
                irritants,
                kind: ConditionKind::Error,
            })))?;
            write_register(stack, base + usize::from(destination), error);
            None
        }
        ColdInstruction::PushHandler { handler } => {
            let handler = read_register(stack, base + usize::from(handler));
            // `frames.handlers` is itself a precise GC root (`gather_vm_roots`).
            frames.handlers.push(handler);
            None
        }
        ColdInstruction::PopHandler => {
            frames
                .handlers
                .pop()
                .ok_or_else(|| bad("exception handler stack"))?;
            None
        }
        ColdInstruction::Raise {
            destination,
            object,
            continuable,
            expected,
        } => {
            let object = read_register(stack, base + usize::from(object));
            let handler = frames
                .handlers
                .pop()
                .ok_or_else(|| unhandled_exception(heap, object))?;
            let destination = base + usize::from(destination);
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
                if continuable {
                    ReturnAction::ReinstallHandler(handler)
                } else {
                    ReturnAction::RaiseReturned
                },
            )?
        }
        ColdInstruction::DynamicWind {
            destination,
            before,
            thunk,
            after,
            expected,
        } => {
            let destination = base + usize::from(destination);
            let before = read_register(stack, base + usize::from(before));
            let thunk = read_register(stack, base + usize::from(thunk));
            let after = read_register(stack, base + usize::from(after));
            let wind = Wind {
                id: frames.next_wind,
                before,
                after,
            };
            frames.next_wind = frames.next_wind.wrapping_add(1);
            stack.ensure(destination + 1);
            stack[destination] = before;
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
                ReturnAction::StartWind(Box::new(StartWindData { thunk, wind })),
            )?
        }
        ColdInstruction::CallWithPort {
            destination,
            port,
            procedure,
            expected,
        } => {
            let destination = base + usize::from(destination);
            let port = read_register(stack, base + usize::from(port));
            let id = heap
                .port(port)
                .ok_or_else(|| Error::plain(ErrorKind::TypeError, "expected port"))?;
            let procedure = read_register(stack, base + usize::from(procedure));
            stack.ensure(destination + 2);
            stack[destination] = procedure;
            stack[destination + 1] = port;
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
                ReturnAction::ClosePort(id),
            )?
        }
        ColdInstruction::CallWithFile {
            destination,
            path,
            procedure,
            input,
            expected,
        } => {
            let destination = base + usize::from(destination);
            let path = string_path(heap, read_register(stack, base + usize::from(path)))?;
            let id = match heap.open_file(&path, input, false) {
                Ok(id) => id,
                Err(error) => {
                    return invoke_error_handler(
                        heap,
                        stack,
                        frames,
                        globals,
                        symbols,
                        natives,
                        destination,
                        expected,
                        error,
                    );
                }
            };
            let port = heap.alloc(Object::Port(crate::port::PortObject { id }))?;
            let procedure = read_register(stack, base + usize::from(procedure));
            stack.ensure(destination + 2);
            stack[destination] = procedure;
            stack[destination + 1] = port;
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
                ReturnAction::ClosePort(id),
            )?
        }
        ColdInstruction::WithFile {
            destination,
            path,
            thunk,
            input,
            expected,
        } => {
            let destination = base + usize::from(destination);
            let path = string_path(heap, read_register(stack, base + usize::from(path)))?;
            let id = match heap.open_file(&path, input, false) {
                Ok(id) => id,
                Err(error) => {
                    return invoke_error_handler(
                        heap,
                        stack,
                        frames,
                        globals,
                        symbols,
                        natives,
                        destination,
                        expected,
                        error,
                    );
                }
            };
            let port = heap.alloc(Object::Port(crate::port::PortObject { id }))?;
            let thunk = read_register(stack, base + usize::from(thunk));
            let name = if input {
                "current-input-port"
            } else {
                "current-output-port"
            };
            let parameter = globals
                .get(name)
                .copied()
                .ok_or_else(|| bad("current port parameter"))?;
            let old = heap
                .parameter(parameter)
                .ok_or_else(|| bad("current port parameter"))?;
            if !heap.set_parameter(parameter, port) {
                return Err(bad("current port parameter"));
            }
            frames.parameters.push((parameter, old));
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
                ReturnAction::RestorePort(Box::new(RestorePortData {
                    port: id,
                    parameter,
                    old,
                })),
            )?
        }
        ColdInstruction::Load {
            destination,
            path,
            environment,
            expected,
        } => {
            let destination = base + usize::from(destination);
            let path = string_path(heap, read_register(stack, base + usize::from(path)))?;
            let mutable = match environment {
                Some(environment) => heap
                    .environment_mutable(read_register(stack, base + usize::from(environment)))
                    .ok_or_else(|| {
                        Error::plain(ErrorKind::TypeError, "load requires an environment")
                    })?,
                None => true,
            };
            let loader = source_loader.as_mut().ok_or_else(|| {
                Error::plain(
                    ErrorKind::FileError,
                    "source loading is denied because no loader is installed",
                )
            })?;
            let loaded = loader
                .load(crate::SourceRequest::new(&path, None))
                .map_err(|error| {
                    Error::plain(
                        ErrorKind::FileError,
                        format!("failed to load source '{path}': {error}"),
                    )
                })?;
            let maximum = config.limits().max_source_bytes();
            if loaded.text().len() > maximum {
                return Err(Error::plain(
                    ErrorKind::SourceTooLarge,
                    format!(
                        "source contains {} bytes, exceeding the {maximum}-byte limit",
                        loaded.text().len()
                    ),
                ));
            }
            let source = sources.add(
                loaded.display_name().to_owned(),
                Some(loaded.canonical_identity().to_owned()),
                loaded.text().to_owned(),
            )?;
            let mut reader = crate::Reader::new(source, loaded.text().to_owned(), config);
            let forms = crate::frontend::read_forms(&mut reader)?;
            if !mutable
                && (!crate::library::definition_names(&forms).is_empty()
                    || !crate::library::syntax_definition_names(&forms).is_empty())
            {
                return Err(Error::plain(
                    ErrorKind::RuntimeError,
                    "cannot define in an immutable environment",
                ));
            }
            let expression = crate::expand::expand_forms_with_features(
                &forms,
                config.limits(),
                HashMap::new(),
                config.features(),
            )?;
            let module = crate::compile::compile(&expression, config.limits())?;
            stack.ensure(destination + usize::from(module.entry.max_registers));
            let frame = frames.reserve();
            frame.chunk = module.entry;
            frame.pc = 0;
            frame.base = destination;
            frame.top = destination;
            frame.return_base = destination;
            frame.expected = expected;
            frame.captures = Rc::from([]);
            frame.procedure = Value::unspecified();
            frame.return_action = Some(Box::new(ReturnAction::LoadComplete));
            None
        }
        ColdInstruction::LoadNumber {
            destination,
            number,
        } => {
            // A numeric literal with no inline representation
            // (heap-backed exact integer or rational). The value
            // is reachable from its register before the next
            // safe point, so no explicit root is needed.
            let value = heap.alloc(Object::Number(Box::new(
                crate::number::RuntimeNumber::from_literal(number),
            )))?;
            write_register(stack, base + usize::from(destination), value);
            None
        }
        instruction => {
            execute_cold(instruction, config)?;
            None
        }
    };
    Ok(completed)
}

/// The out-of-line safe-point body: honors a completed exit, runs any deferred
/// collection against a refreshed precise root snapshot, polls the interrupt
/// token, and charges the instruction fuel retired since the previous cold
/// entry. Returns `Ok(Some(status))` when a completed exit ends execution.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn cold_safe_point(
    heap: &mut Heap,
    globals: &crate::global::GlobalStore,
    symbols: &HashMap<String, Value>,
    stack: &RegisterStack,
    frames: &FrameStack,
    interrupt: &InterruptToken,
    fuel: &mut Option<u64>,
    retired: u64,
) -> Result<Option<crate::ExitStatus>, Error> {
    if let Some(status) = heap.take_completed_exit() {
        return Ok(Some(status));
    }
    if heap.needs_collection() {
        sync_engine_roots(heap, globals, symbols);
        heap.collect_with(&|roots| gather_vm_roots(stack, frames, 0, roots));
    }
    if interrupt.is_interrupted() {
        return Err(Error::plain(
            ErrorKind::ExecutionLimitExceeded,
            "execution interrupted",
        ));
    }
    if let Some(remaining) = fuel.as_mut() {
        if retired > *remaining {
            return Err(Error::plain(
                ErrorKind::ExecutionLimitExceeded,
                "instruction fuel exhausted",
            ));
        }
        *remaining -= retired;
    }
    Ok(None)
}
