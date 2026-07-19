//! Compilers for individual expression forms: constants, lambdas, named lets,
//! applications, `values`, and the cold-instruction call/result emitters.

use super::{
    analysis::{mentions_variable, parameter_usage},
    *,
};

pub(super) fn compile_constant(
    constant: Constant,
    code: &mut Code,
    registers: &mut Registers,
    mode: Mode,
) -> Result<Option<u8>, Error> {
    let Some(destination) = output_register(registers, mode)? else {
        return Ok(Some(0));
    };
    code.load_constant(destination, constant)?;
    finish_value(code, destination, mode)
}

pub(super) fn compile_lambda<'src>(
    required: &'src [String],
    rest: Option<&'src String>,
    body: &'src CoreExpr,
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    mode: Mode,
) -> Result<Option<u8>, Error> {
    let Some(destination) = output_register(registers, mode)? else {
        return Ok(Some(0));
    };
    let mut locals = HashMap::new();
    let param_names = required.iter().chain(rest).collect::<Vec<_>>();
    for (index, name) in param_names.iter().enumerate() {
        if locals
            .insert(
                (*name).clone(),
                register_index(index, "too many parameters")?,
            )
            .is_some()
        {
            return Err(compile_error("duplicate procedure parameter"));
        }
    }
    let arity_count = register_index(required.len(), "too many parameters")?;
    // Only mutated parameters need a heap cell (shared with every closure and
    // continuation that captures them). A parameter that is captured but never
    // `set!` stays a plain register: closures snapshot its value at creation
    // (`CaptureKind::Value`), which is indistinguishable from sharing because
    // the binding never changes. The scan uses the inline-aware model:
    // variables used only by a lambda that `compile_call` will inline are not
    // treated as captured or nested-mutated.
    let boxed = parameter_usage(&param_names, body).mutated;
    let child_env = Environment {
        locals,
        captures: HashMap::new(),
        boxed,
        inline_boxed: HashSet::new(),
        outer: Some(env),
    };
    let arity = if rest.is_some() {
        Arity::AtLeast(arity_count)
    } else {
        Arity::Exact(arity_count)
    };
    let (chunk, captures) = compile_chunk(body, child_env, arity)?;
    let mut sources = Vec::with_capacity(captures.len());
    for (_, name) in captures {
        sources.push(match env.resolve(&name)? {
            Access::Local(index) => CaptureSource::Local(index),
            Access::Capture(index, _) => CaptureSource::Capture(index),
            Access::Global => return Err(compile_error("invalid global capture")),
        });
    }
    let index =
        u32::try_from(code.closures.len()).map_err(|_| compile_error("too many closures"))?;
    code.closures.push(ClosurePrototype {
        chunk: Rc::new(chunk),
        captures: sources,
    });
    code.emit(Word::abx(Opcode::Closure, destination, index)?);
    finish_value(code, destination, mode)
}

/// Lowers a named let to its equivalent closure form and compiles it: a
/// self-recursive procedure bound to the loop name and applied to the initial
/// values. Used when the loop cannot be flattened (its name escapes, a
/// self-call is not in tail position, or a loop variable is captured/mutated).
#[allow(clippy::too_many_arguments)]
fn compile_named_let_closure(
    name: &str,
    params: &[String],
    inits: &[CoreExpr],
    body: &CoreExpr,
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    mode: Mode,
) -> Result<Option<u8>, Error> {
    let loop_lambda = CoreExpr::Lambda {
        params: params.to_vec(),
        body: Box::new(body.clone()),
    };
    let initial_call = CoreExpr::Call {
        procedure: Box::new(CoreExpr::Variable(name.to_owned())),
        arguments: inits.to_vec(),
    };
    let installed = CoreExpr::Begin(vec![
        CoreExpr::Set {
            name: name.to_owned(),
            value: Box::new(loop_lambda),
        },
        initial_call,
    ]);
    let application = CoreExpr::Call {
        procedure: Box::new(CoreExpr::Lambda {
            params: vec![name.to_owned()],
            body: Box::new(installed),
        }),
        arguments: vec![CoreExpr::Literal(Value::unspecified())],
    };
    compile_expression(&application, env, code, registers, mode)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compile_named_let(
    name: &str,
    params: &[String],
    inits: &[CoreExpr],
    body: &CoreExpr,
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    mode: Mode,
) -> Result<Option<u8>, Error> {
    // A multiple-values context keeps the closure path (mirrors the inlining
    // decision, and `scan_usage`'s `all_pos` gate agrees). Otherwise a
    // non-escaping, self-tail-recursive loop is flattened into a register loop.
    if !matches!(mode, Mode::All(_)) && classify_named_let(name, params, body) {
        compile_named_let_loop(name, params, inits, body, env, code, registers, mode)
    } else {
        compile_named_let_closure(name, params, inits, body, env, code, registers, mode)
    }
}

/// Compiles a non-escaping self-tail-recursive named let as a register loop in
/// the enclosing frame: the parameters become a stable register block, the body
/// compiles in place (so its free variables resolve as ordinary enclosing-frame
/// registers rather than boxed captures), and each tail self-call updates the
/// parameter registers and jumps back to the loop header.
#[allow(clippy::too_many_arguments)]
fn compile_named_let_loop(
    name: &str,
    params: &[String],
    inits: &[CoreExpr],
    body: &CoreExpr,
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    mode: Mode,
) -> Result<Option<u8>, Error> {
    let arity = params.len();
    let mark = registers.mark();
    // Stable parameter block: never reset until the whole body is compiled, so
    // the values survive the loop's back-edge.
    let base = registers.acquire_block(arity)?;
    // Initial values are evaluated with the parameters still out of scope
    // (parallel `let` semantics); they cannot reference the loop name.
    for (offset, init) in inits.iter().enumerate() {
        compile_expression(init, env, code, registers, Mode::One(base + offset as u8))?;
    }
    // Bind the parameters as scoped locals, remembering any shadowed entry.
    let mut shadowed = Vec::with_capacity(arity);
    for (offset, parameter) in params.iter().enumerate() {
        let previous = env.locals.insert(parameter.clone(), base + offset as u8);
        shadowed.push((parameter.clone(), previous));
    }
    let header = code.words.len();
    code.loops.push(LoopFrame {
        name: name.to_owned(),
        base,
        arity,
        header,
    });
    let result = compile_expression(body, env, code, registers, mode);
    code.loops.pop();
    // Restore the environment before propagating any body error.
    for (parameter, previous) in shadowed {
        match previous {
            Some(index) => {
                env.locals.insert(parameter, index);
            }
            None => {
                env.locals.remove(&parameter);
            }
        }
    }
    let result = result?;
    if !matches!(mode, Mode::Return) {
        registers.reset(mark);
    }
    Ok(result)
}

/// Emits a flattened tail self-call: the new argument values are evaluated into
/// scratch registers (so a value that reads an old parameter is unaffected),
/// moved into the loop's parameter block, and control jumps back to the loop
/// header. Returns `Ok(None)` like an ordinary tail call.
fn compile_loop_tail_call(
    base: u8,
    arity: usize,
    header: usize,
    arguments: &[CoreExpr],
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
) -> Result<Option<u8>, Error> {
    // Detect a single counter parameter whose new value is `(+ p K)` / `(- p K)`
    // for a small literal step `K`, or `(+ p r)` for a loop-invariant step
    // REGISTER `r`, with `p` the loop parameter at that position. Such a
    // parameter is stepped in place by a fused back-edge (increment +
    // back-edge) instead of the scratch-compute + `Move` + `Jump` trio: the test
    // stays at the loop header, so this fires on every counting loop regardless of
    // its test shape. At most one counter is fused. Any others stay accumulators.
    let counter = (0..arity).find_map(|offset| {
        let register = base + offset as u8;
        counter_step(&arguments[offset], env, base, register).map(|step| (offset, step))
    });
    let counter_pos = counter.map(|(pos, _)| pos);

    // An accumulator whose home register is read by no *other* argument can be
    // written straight into that register: no later argument depends on its old
    // value, so it needs neither a scratch slot nor a Phase-2 `Move`. Arguments
    // whose register is read by another argument stay on the scratch path to
    // preserve parallel-assignment semantics (`mentions_variable` over-approximates
    // reads, which only ever keeps an argument on the safe scratch path). The
    // counter is excluded here. It is stepped in place by `LoopBack`. This is
    // precomputed (owning no borrow of `env`) so Phase 1 can mutably borrow it.
    let direct: Vec<bool> = (0..arity)
        .map(|offset| {
            if Some(offset) == counter_pos {
                return false;
            }
            // Reverse-map the parameter register to its bound name: during the
            // loop body the stable parameter block is bound exactly to these
            // names in `env.locals`.
            let register = base + offset as u8;
            let Some((name, _)) = env.locals.iter().find(|&(_, &index)| index == register) else {
                return false;
            };
            !arguments
                .iter()
                .enumerate()
                .any(|(other, argument)| other != offset && mentions_variable(argument, name))
        })
        .collect();

    // Exactly one cyclic (non-direct) accumulator can still avoid its scratch
    // slot: whichever is evaluated LAST, once every other scratch argument has
    // already read the old parameter values. At that point only scratch slots have
    // been written, so every home register it reads is still the pre-step value
    // (its own included - a home is written only at the very end of its argument's
    // evaluation), and no remaining evaluation needs its old value. Writing it in
    // place therefore preserves parallel-assignment semantics while dropping one
    // `Move`. Pick the highest such offset. `None` when every non-counter parameter
    // is already direct.
    let last_cyclic = (0..arity)
        .rev()
        .find(|&offset| Some(offset) != counter_pos && !direct[offset]);

    let mark = registers.mark();
    let scratch = registers.acquire_block(arity)?;
    // Phase 1: evaluate every non-counter argument except the deferred cyclic one.
    // Independent accumulators go straight to their home register. The rest read
    // the old parameter values into scratch. The counter register is left untouched
    // until its step.
    for (offset, argument) in arguments.iter().enumerate() {
        if Some(offset) == counter_pos || Some(offset) == last_cyclic {
            continue;
        }
        let destination = if direct[offset] {
            base + offset as u8
        } else {
            scratch + offset as u8
        };
        compile_expression(argument, env, code, registers, Mode::One(destination))?;
    }
    // Phase 1b: evaluate the deferred cyclic accumulator straight into its home.
    // Every home it reads is still intact (only scratch slots written so far), and
    // no later step reads its old value.
    if let Some(offset) = last_cyclic {
        compile_expression(
            &arguments[offset],
            env,
            code,
            registers,
            Mode::One(base + offset as u8),
        )?;
    }
    // Phase 2: move the scratch-evaluated values into the parameter block. Direct
    // accumulators and the deferred cyclic one already wrote their home register.
    for offset in 0..arity as u8 {
        if Some(usize::from(offset)) == counter_pos
            || Some(usize::from(offset)) == last_cyclic
            || direct[usize::from(offset)]
        {
            continue;
        }
        code.emit(Word::abc(
            Opcode::Move,
            base + offset,
            scratch + offset,
            0,
            false,
        ));
    }
    registers.reset(mark);
    // Phase 3: step the counter in place (if any), then jump back to the header.
    // The `LoopBack` executor consumes the following `Jump` inline, so the counter
    // is incremented after every accumulator update has read its old value.
    if let Some((pos, step)) = counter {
        // Counted-loop back-edge test fusion. Both families require the loop
        // header to OPEN with the canonical exit test (`words[header]` is the
        // `Test*` word itself, jump-on-false polarity). That anchoring is what
        // makes replication safe: it proves every replicated operand is a
        // k-constant or a named local (parameter home or outer local), never a
        // reusable scratch register, because a scratch operand would need a
        // materialization word at `header`. The fused word re-runs the test on
        // the back-edge: step the counter, replicate the comparison (operands
        // copied verbatim, the compare kind lives in the opcode identity,
        // never an operand swap or negation, which NaN would break), and jump
        // straight to the body start while the loop continues, one dispatch
        // per iteration. The exit path falls through to a `Jump -> header`,
        // where the canonical test re-confirms and falls through to the exit
        // code (a couple of extra dispatches, once per loop, which keeps the
        // correctness argument trivial).
        //
        // Two body layouts, told apart by the header `Jump`'s target. The
        // else-jump of an `if` is patched before its alternate compiles, so at
        // back-edge-emission time an unpatched placeholder (decoding to
        // `header + 2`) means the tail call sits in the CONSEQUENT
        // (fall-into-body: `(if (<= m limit) (begin .. (loop (+ m p))) ..)`),
        // while a patched target past the non-empty consequent
        // (`>= header + 3`) means it sits in the ALTERNATE (jump-to-body:
        // `(if (= i limit) exit (loop .. (+ i 1) ..))`).
        //
        // Jump-to-body loops fuse a literal +1 step with the `=`/`<`/`<=`
        // header test into `LoopBackWhileNot{Equal,Less,LessEqual}` (continue
        // while the exit test is FALSE). Fall-into-body loops fuse a register
        // step whose compare-left is the counter itself into
        // `LoopBackStepWhile{LessEqual,Less}` (continue while the guard is
        // TRUE). `A` doubles as counter and compare-left, freeing `B` for the
        // step register.
        let counter_home = base + pos as u8;
        let test = code.words.get(header).copied();
        let successor = code.words.get(header + 1).copied();
        let fused = match (test, successor) {
            (Some(test), Some(jump))
                if matches!(jump.opcode(), Ok(Opcode::Jump)) && test.a() == 0 =>
            {
                let target = header as isize + 2 + jump.signed_jump();
                let jump_to_body =
                    target >= header as isize + 3 && target <= code.words.len() as isize;
                let fall_into_body = target == header as isize + 2;
                // One comparison operand must be the counter itself: that is
                // the exit-test shape, which keeps the loop running until the
                // final iteration. Exactly when re-running it on the
                // back-edge pays. An incidental leading `if` could branch the
                // other way most iterations and turn the exit detour into a
                // per-iteration cost.
                let counter_is_operand =
                    test.b() == counter_home || (!test.k() && test.c() == counter_home);
                match (test.opcode(), step) {
                    (Ok(Opcode::TestEqual | Opcode::TestEqualFixnum), Step::Literal(1))
                        if jump_to_body && counter_is_operand =>
                    {
                        // A fixnum constant limit takes the specialized word.
                        // The executor then compares raw payloads instead of
                        // re-classifying the constant every iteration.
                        let opcode = if test.k()
                            && code.constants[usize::from(test.c())].as_fixnum().is_some()
                        {
                            Opcode::LoopBackWhileNotEqualFixnum
                        } else {
                            Opcode::LoopBackWhileNotEqual
                        };
                        Some((opcode, test.b(), target))
                    }
                    (Ok(Opcode::TestLess | Opcode::TestLessFixnum), Step::Literal(1))
                        if jump_to_body && counter_is_operand =>
                    {
                        Some((Opcode::LoopBackWhileNotLess, test.b(), target))
                    }
                    (Ok(Opcode::TestLessEqual | Opcode::TestLessEqualFixnum), Step::Literal(1))
                        if jump_to_body && counter_is_operand =>
                    {
                        Some((Opcode::LoopBackWhileNotLessEqual, test.b(), target))
                    }
                    (
                        Ok(Opcode::TestLessEqual | Opcode::TestLessEqualFixnum),
                        Step::Register(step_reg),
                    ) if fall_into_body && test.b() == counter_home => {
                        Some((Opcode::LoopBackStepWhileLessEqual, step_reg, target))
                    }
                    (Ok(Opcode::TestLess | Opcode::TestLessFixnum), Step::Register(step_reg))
                        if fall_into_body && test.b() == counter_home =>
                    {
                        Some((Opcode::LoopBackStepWhileLess, step_reg, target))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if let (Some((opcode, operand_b, body)), Some(test)) = (fused, test) {
            code.emit(Word::abc(
                opcode,
                counter_home,
                operand_b,
                test.c(),
                test.k(),
            ));
            let continue_target = body - code.words.len() as isize - 1;
            code.emit(Word::sj(Opcode::Jump, continue_target)?);
            let exit_target = header as isize - code.words.len() as isize - 1;
            code.emit(Word::sj(Opcode::Jump, exit_target)?);
            return Ok(None);
        }
        match step {
            Step::Literal(step) => {
                code.emit(Word::abc(
                    Opcode::LoopBack,
                    counter_home,
                    0,
                    (step as i8) as u8,
                    false,
                ));
            }
            Step::Register(step_reg) => {
                // Register step without a fusable header: step the counter in
                // place with a plain `Add` (it runs after all staging, so
                // every accumulator already read the old counter value. The
                // same ordering `LoopBack` provides), then fall through to the
                // generic `Jump` back-edge below.
                code.emit(Word::abc(
                    Opcode::Add,
                    counter_home,
                    counter_home,
                    step_reg,
                    false,
                ));
            }
        }
    }
    let target = header as isize - code.words.len() as isize - 1;
    code.emit(Word::sj(Opcode::Jump, target)?);
    Ok(None)
}

/// How a fused back-edge steps its counter parameter: by a literal exact
/// integer fitting the signed 8-bit step field, or by adding a loop-invariant
/// step register (the strided-loop shape, e.g. a sieve's `(+ multiple p)`).
#[derive(Clone, Copy)]
enum Step {
    Literal(i64),
    Register(u8),
}

/// If `arg` is `(+ p K)` / `(+ K p)` / `(- p K)` where `p` is the loop parameter
/// held in `counter_reg` (unshadowed and unboxed) and `K` a literal exact integer
/// that fits the signed 8-bit step field, returns that literal step. If it is
/// `(+ p r)` / `(+ r p)` for a loop-invariant local `r`, returns the step
/// register. These are the counting-loop shapes the fused back-edges handle.
/// Anything else stays on the generic scratch-`Move`-`Jump` back-edge.
fn counter_step(arg: &CoreExpr, env: &Environment<'_>, base: u8, counter_reg: u8) -> Option<Step> {
    // Loop parameters are never boxed (the classifier requires it), but a boxed
    // slot would need `SetLocalBox`, not a raw register write, so guard anyway.
    if env.is_boxed_local(counter_reg) {
        return None;
    }
    let CoreExpr::Call {
        procedure,
        arguments,
    } = arg
    else {
        return None;
    };
    if arguments.len() != 2 {
        return None;
    }
    // Only the unshadowed `(scheme base)` `+`/`-` count as a numeric step.
    let is_counter = |expr: &CoreExpr| {
        matches!(expr, CoreExpr::Variable(name)
            if env.locals.get(name).copied() == Some(counter_reg))
    };
    // A step register must hold the same value at the back-edge that the
    // argument expression would have read: a non-boxed named local BELOW the
    // loop's parameter block is never rewritten while this loop runs (non-boxed
    // locals are written only at their binding, and an enclosing loop's tail
    // call transfers control out of this loop entirely). Same-loop parameters
    // are rejected. The staging phases rewrite them before the back-edge,
    // which would feed the step the NEW value where parallel-assignment
    // semantics require the old one. The counter itself is the one exception
    // (`(+ i i)` doubling): the executor reads the step register before
    // writing the counter, so it sees the pre-step value.
    let step_register = |expr: &CoreExpr| -> Option<u8> {
        let CoreExpr::Variable(name) = expr else {
            return None;
        };
        let home = env.locals.get(name).copied()?;
        if env.is_boxed_local(home) {
            return None;
        }
        (home < base || home == counter_reg).then_some(home)
    };
    match fast_operation(procedure, 2)? {
        Opcode::Add if is_counter(&arguments[0]) => {
            if let Some(step) = literal_integer(&arguments[1]).and_then(fits_step) {
                return Some(Step::Literal(step));
            }
            step_register(&arguments[1]).map(Step::Register)
        }
        Opcode::Add if is_counter(&arguments[1]) => {
            if let Some(step) = literal_integer(&arguments[0]).and_then(fits_step) {
                return Some(Step::Literal(step));
            }
            step_register(&arguments[0]).map(Step::Register)
        }
        Opcode::Subtract if is_counter(&arguments[0]) => literal_integer(&arguments[1])?
            .checked_neg()
            .and_then(fits_step)
            .map(Step::Literal),
        _ => None,
    }
}

/// Two-argument numeric words whose RK constant holds a fixnum take the
/// constant-specialized opcode, so the executor compares or combines raw
/// payloads without re-classifying the constant on every execution.
fn specialize_numeric_fixnum_k(code: &Code, opcode: Opcode, index: u8) -> Opcode {
    if code.constants[usize::from(index)].as_fixnum().is_none() {
        return opcode;
    }
    match opcode {
        Opcode::Add => Opcode::AddFixnumK,
        Opcode::Subtract => Opcode::SubtractFixnumK,
        _ => opcode,
    }
}

/// Comparison `Test*` words whose RK constant holds a fixnum take the
/// specialized opcode. Shared by the condition compiler.
pub(super) fn specialize_test_fixnum_k(code: &Code, opcode: Opcode, index: u8) -> Opcode {
    if code.constants[usize::from(index)].as_fixnum().is_none() {
        return opcode;
    }
    match opcode {
        Opcode::TestLess => Opcode::TestLessFixnum,
        Opcode::TestLessEqual => Opcode::TestLessEqualFixnum,
        Opcode::TestEqual => Opcode::TestEqualFixnum,
        _ => opcode,
    }
}

fn fits_step(value: i128) -> Option<i64> {
    i8::try_from(value).ok().map(i64::from)
}

fn literal_integer(expr: &CoreExpr) -> Option<i128> {
    match expr {
        CoreExpr::NumberLiteral(crate::Number::Real(crate::Real::ExactInteger(value))) => {
            Some(*value)
        }
        CoreExpr::Literal(value) => value.as_fixnum().map(i128::from),
        _ => None,
    }
}

/// Decides whether a named let can be flattened into a register loop: its name
/// must never escape (appear anywhere but as the operator of a tail self-call),
/// every self-call must be in tail position and outside any capture boundary
/// with the right arity, and no loop variable may be `set!`-mutated or captured
/// by a nested escaping lambda (which would require a fresh per-iteration cell).
/// Used identically by `compile_named_let` and `scan_usage`, so they always
/// agree on which loops flatten.
pub(super) fn classify_named_let(name: &str, params: &[String], body: &CoreExpr) -> bool {
    let references = params.iter().collect::<Vec<_>>();
    // Both mutation and capture disqualify flattening: either would require a
    // fresh per-iteration cell, which a flattened register loop cannot provide.
    if !parameter_usage(&references, body).is_empty() {
        return false;
    }
    loop_uses_ok(body, name, params.len(), true, false, false)
}

/// Whether every occurrence of `name` in `node` is a valid tail self-call that
/// the compiler will emit as a back-edge. `tail` tracks tail position relative
/// to the loop body; `boundary` tracks whether `node` sits inside a nested
/// capture boundary (a non-inlined lambda or a promise), where a self-call gets
/// its own frame and cannot be a back-edge; `all_pos` tracks a multiple-values
/// position. The inlining and boundary decisions mirror `compile_call`/
/// `scan_usage` exactly, so the classifier agrees with what the compiler emits.
fn loop_uses_ok(
    node: &CoreExpr,
    name: &str,
    arity: usize,
    tail: bool,
    boundary: bool,
    all_pos: bool,
) -> bool {
    // Non-tail, single-value, boundary-preserving recursion (the common case for
    // operands and effectful sub-expressions).
    let operand = |child: &CoreExpr| loop_uses_ok(child, name, arity, false, boundary, false);
    match node {
        CoreExpr::Literal(_) | CoreExpr::NumberLiteral(_) => true,
        // A bare reference to the loop name (not a call operator) escapes.
        CoreExpr::Variable(value) => value != name,
        CoreExpr::Call {
            procedure,
            arguments,
        } => {
            if let CoreExpr::Variable(value) = procedure.as_ref()
                && value == name
            {
                // A self-call is admissible only as a tail call in this frame
                // with the loop's arity; its arguments are non-tail positions.
                tail && !boundary && arguments.len() == arity && arguments.iter().all(&operand)
            } else if let CoreExpr::Lambda { params, body } = procedure.as_ref()
                && !all_pos
                && can_inline_application(params, arguments)
            {
                // Inlined into the current frame (like `compile_call`): the body
                // is not a boundary and inherits this position.
                arguments.iter().all(&operand)
                    && loop_uses_ok(body, name, arity, tail, boundary, false)
            } else {
                operand(procedure) && arguments.iter().all(&operand)
            }
        }
        CoreExpr::NamedLet {
            name: inner,
            params: inner_params,
            inits,
            body: inner_body,
        } => {
            // A nested loop flattens under the same gate the compiler uses; only
            // then is its body compiled in place, so a cross-loop tail call (this
            // loop invoked from inside the inner one) truly stays in this frame.
            let flattens = !all_pos && classify_named_let(inner, inner_params, inner_body);
            let (inner_tail, inner_boundary) = if flattens {
                (tail, boundary)
            } else {
                (false, true)
            };
            inits.iter().all(&operand)
                && loop_uses_ok(inner_body, name, arity, inner_tail, inner_boundary, false)
        }
        CoreExpr::If(test, consequent, alternate) => {
            operand(test)
                && loop_uses_ok(consequent, name, arity, tail, boundary, all_pos)
                && loop_uses_ok(alternate, name, arity, tail, boundary, all_pos)
        }
        CoreExpr::Begin(items) => match items.split_last() {
            Some((last, leading)) => {
                leading.iter().all(&operand)
                    && loop_uses_ok(last, name, arity, tail, boundary, all_pos)
            }
            None => true,
        },
        CoreExpr::Set {
            name: target,
            value,
        } => {
            // A self-call value is fine; mutating the loop name is not.
            target != name && operand(value)
        }
        CoreExpr::Define { value, .. } => operand(value),
        // Capture boundaries: a `name` use inside cannot be a back-edge here.
        CoreExpr::Lambda { body, .. } | CoreExpr::LambdaRest { body, .. } => {
            loop_uses_ok(body, name, arity, false, true, false)
        }
        CoreExpr::CaseLambda { clauses } => clauses
            .iter()
            .all(|clause| loop_uses_ok(clause, name, arity, false, true, false)),
        CoreExpr::Delay(inner) | CoreExpr::DelayForce(inner) => {
            loop_uses_ok(inner, name, arity, false, true, false)
        }
        CoreExpr::Values(items) => items.iter().all(&operand),
        CoreExpr::CallWithValues { producer, consumer } => operand(producer) && operand(consumer),
        CoreExpr::Force(inner) | CoreExpr::CallWithCurrentContinuation(inner) => operand(inner),
        CoreExpr::WithExceptionHandler { handler, thunk } => operand(handler) && operand(thunk),
        CoreExpr::Raise { object, .. } => operand(object),
        CoreExpr::DynamicWind {
            before,
            thunk,
            after,
        } => operand(before) && operand(thunk) && operand(after),
        CoreExpr::CallWithPort { port, procedure } => operand(port) && operand(procedure),
        CoreExpr::CallWithFile {
            path, procedure, ..
        } => operand(path) && operand(procedure),
        CoreExpr::WithFile { path, thunk, .. } => operand(path) && operand(thunk),
        CoreExpr::Load { path, environment } => {
            operand(path) && environment.as_ref().is_none_or(|inner| operand(inner))
        }
        CoreExpr::Parameterize { bindings, body } => {
            bindings
                .iter()
                .all(|(parameter, value)| operand(parameter) && operand(value))
                // The body is a multiple-values, non-tail dynamic extent.
                && loop_uses_ok(body, name, arity, false, boundary, true)
        }
        CoreExpr::MakeParameter { initial, converter } => {
            operand(initial) && converter.as_ref().is_none_or(|inner| operand(inner))
        }
        CoreExpr::Error { message, irritants } => {
            operand(message) && irritants.iter().all(&operand)
        }
    }
}

pub(super) fn compile_call(
    procedure: &CoreExpr,
    arguments: &[CoreExpr],
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    mode: Mode,
) -> Result<Option<u8>, Error> {
    // A call to an active flattened-loop label is a tail self-call: update the
    // loop's parameter registers and jump back to its header. The classifier
    // guaranteed every such call is in tail position, so no mode check is
    // needed. Loop names are gensyms, so at most one frame matches.
    if let CoreExpr::Variable(name) = procedure
        && let Some((base, arity, header)) = code
            .loops
            .iter()
            .rev()
            .find(|frame| frame.name == *name)
            .map(|frame| (frame.base, frame.arity, frame.header))
    {
        return compile_loop_tail_call(base, arity, header, arguments, env, code, registers);
    }
    // Immediately-applied fixed-arity lambda (`let`/`or`/`and`/`when`/`cond`/...
    // all desugar to this shape): inline it into the current frame as scoped
    // local bindings rather than allocating a fresh closure per evaluation.
    // `Mode::All` (multiple-value context) keeps the closure path, whose
    // destination handling for dynamic result counts is more intricate.
    if let CoreExpr::Lambda { params, body } = procedure
        && !matches!(mode, Mode::All(_))
        && can_inline_application(params, arguments)
    {
        // `scan_usage` uses the same predicate (`can_inline_application` with a
        // non-`All` position) to decide not to box the enclosing frame's
        // variables captured only by this lambda. So inlining must succeed here
        // whenever that holds. A fallback to the closure path would capture
        // those now-unboxed locals. If the flattened frame exceeds the register
        // budget we therefore raise a compile error rather than fall back.
        return compile_inlined_application(params, body, arguments, env, code, registers, mode);
    }
    if let Some(operation) = fast_operation(procedure, arguments.len()) {
        return compile_fast_call(operation, arguments, env, code, registers, mode);
    }
    let mark = registers.mark();
    let base = if let Mode::All(destination) = mode {
        registers.ensure(usize::from(destination) + arguments.len() + 1)?;
        destination
    } else {
        registers.acquire_block(arguments.len() + 1)?
    };
    compile_expression(procedure, env, code, registers, Mode::One(base))?;
    for (offset, argument) in arguments.iter().enumerate() {
        let slot = base + offset as u8 + 1;
        // A generic-call argument leaves its result in its own base register and
        // would otherwise be moved down into `slot`. The argument slots to the
        // right of `slot` are still unwritten scratch, so lower the watermark to
        // `slot` while compiling: the inner call then bases itself at `slot`
        // (result in place, no `Move`) and uses the higher slots as scratch. The
        // full block is restored afterward so the next argument and the `Call`
        // see every slot reserved.
        if let CoreExpr::Call {
            procedure,
            arguments,
        } = argument
            && is_generic_call(procedure, arguments, code)
        {
            let saved = registers.mark();
            registers.reset(u16::from(slot));
            compile_expression(argument, env, code, registers, Mode::One(slot))?;
            registers.reset(saved);
        } else {
            compile_expression(argument, env, code, registers, Mode::One(slot))?;
        }
    }
    if matches!(mode, Mode::Return) {
        code.emit(Word::abc(
            Opcode::TailCall,
            base,
            (arguments.len() + 1) as u8,
            0,
            false,
        ));
        registers.reset(mark);
        return Ok(None);
    }
    emit_call(code, base, arguments.len(), mode);
    let result = finish_call_result(code, base, mode)?;
    registers.reset(mark);
    Ok(result)
}

/// Decides whether `((lambda (params) body) arguments)` can be inlined into the
/// enclosing frame instead of allocating a closure. It requires only an exact
/// argument count and distinct parameter names: a parameter that needs boxing
/// (it is `set!`-mutated or captured by a nested escaping lambda) is still
/// inlined and boxed in place by a `BoxLocal` opcode (see
/// [`compile_inlined_application`]). This is a pure syntactic predicate, so
/// `scan_usage` and `compile_call` agree on which applications are inlined.
pub(super) fn can_inline_application(params: &[String], arguments: &[CoreExpr]) -> bool {
    if arguments.len() != params.len() {
        return false;
    }
    // Distinct names: the scoped insert/restore below keys on the name, so a
    // duplicate would corrupt the environment. `let`/`or`/... never produce
    // these, but user-written `((lambda (x x) ...) ..)` might.
    for (index, name) in params.iter().enumerate() {
        if params[..index].contains(name) {
            return false;
        }
    }
    true
}

/// Lowers an inlinable immediately-applied lambda as a `let`-style binding in the
/// current frame: evaluate the arguments into fresh registers, box any parameter
/// that a nested closure captures or `set!` mutates, bind the parameters to those
/// registers as scoped locals, compile the body in the caller's mode (preserving
/// tail position), then restore the environment.
fn compile_inlined_application(
    params: &[String],
    body: &CoreExpr,
    arguments: &[CoreExpr],
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    mode: Mode,
) -> Result<Option<u8>, Error> {
    let mark = registers.mark();
    let base = registers.acquire_block(params.len())?;
    // Arguments are evaluated with the parameters still out of scope, matching
    // parallel `let`/lambda-call semantics.
    for (offset, argument) in arguments.iter().enumerate() {
        compile_expression(
            argument,
            env,
            code,
            registers,
            Mode::One(base + offset as u8),
        )?;
    }
    // A parameter that is `set!`-mutated anywhere needs a heap `Box` cell
    // (captured-but-immutable ones are snapshotted by value at closure
    // creation instead). Unlike a frame parameter (boxed at entry via
    // `boxed_locals`), an inlined one is boxed in place here, after its argument
    // is in the register, and tracked in `inline_boxed` only while its body
    // compiles (so a sibling scope reusing the register sees a plain local).
    let refs = params.iter().collect::<Vec<_>>();
    let mut boxed = parameter_usage(&refs, body)
        .mutated
        .into_iter()
        .collect::<Vec<_>>();
    boxed.sort_unstable();
    let mut newly_boxed = Vec::new();
    for offset in boxed {
        let register = base + offset;
        code.emit(Word::abc(Opcode::BoxLocal, register, 0, 0, false));
        if env.inline_boxed.insert(register) {
            newly_boxed.push(register);
        }
    }
    // Bind the parameters as scoped locals, remembering any shadowed entry.
    let mut shadowed = Vec::with_capacity(params.len());
    for (offset, name) in params.iter().enumerate() {
        let previous = env.locals.insert(name.clone(), base + offset as u8);
        shadowed.push((name.clone(), previous));
    }
    let result = compile_expression(body, env, code, registers, mode);
    // Restore the environment before propagating any body error.
    for (name, previous) in shadowed {
        match previous {
            Some(index) => {
                env.locals.insert(name, index);
            }
            None => {
                env.locals.remove(&name);
            }
        }
    }
    for register in newly_boxed {
        env.inline_boxed.remove(&register);
    }
    let result = result?;
    if !matches!(mode, Mode::Return) {
        registers.reset(mark);
    }
    Ok(result)
}

fn compile_fast_call(
    opcode: Opcode,
    arguments: &[CoreExpr],
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    mode: Mode,
) -> Result<Option<u8>, Error> {
    if matches!(
        opcode,
        Opcode::Add | Opcode::Subtract | Opcode::Multiply | Opcode::Divide
    ) {
        return compile_fold_call(opcode, arguments, env, code, registers, mode);
    }
    // Unary primitives (`car`, `cdr`, `null?`, `pair?`, `string-length`,
    // `char->integer`): `A = op(B)`. Even a discarded result must execute, as
    // most of these can raise a type error.
    if matches!(
        opcode,
        Opcode::Car
            | Opcode::Cdr
            | Opcode::NullP
            | Opcode::PairP
            | Opcode::StringLength
            | Opcode::CharToInteger
    ) {
        let destination = match output_register(registers, mode)? {
            Some(destination) => destination,
            None => registers.acquire()?,
        };
        let mark = registers.mark();
        let source = operand_register(&arguments[0], env, code, registers)?;
        code.emit(Word::abc(opcode, destination, source, 0, false));
        registers.reset(mark);
        return finish_value(code, destination, mode);
    }
    // `cons`: `A = cons(B, C)` with both operands in registers (the executor
    // reads C as a register, not an RK constant, and never folds a literal).
    if opcode == Opcode::Cons {
        let destination = match output_register(registers, mode)? {
            Some(destination) => destination,
            None => registers.acquire()?,
        };
        let mark = registers.mark();
        let left = operand_register(&arguments[0], env, code, registers)?;
        let right = operand_register(&arguments[1], env, code, registers)?;
        code.emit(Word::abc(opcode, destination, left, right, false));
        registers.reset(mark);
        return finish_value(code, destination, mode);
    }
    // Chained indexing `(vector-ref (vector-ref v i) j)` fuses both fetches
    // into one `VectorRefVectorRef` word (outer index in a consumed
    // `ExtraArg`), the matrix-multiply row-then-element shape. `j` must be a
    // home read: its materialization would otherwise run between the fetches,
    // which the fused word cannot replicate (the miss order must match the
    // unfused pair). The inner operands compile first either way.
    if opcode == Opcode::VectorRef
        && let CoreExpr::Call {
            procedure,
            arguments: inner,
        } = &arguments[0]
        && fast_operation(procedure, inner.len()) == Some(Opcode::VectorRef)
        && home_readable(&arguments[1], env)?
    {
        let destination = match output_register(registers, mode)? {
            Some(destination) => destination,
            None => registers.acquire()?,
        };
        let mark = registers.mark();
        let vector = operand_register(&inner[0], env, code, registers)?;
        let index = operand_register(&inner[1], env, code, registers)?;
        let outer_index = operand_register(&arguments[1], env, code, registers)?;
        code.emit(Word::abc(
            Opcode::VectorRefVectorRef,
            destination,
            vector,
            index,
            false,
        ));
        code.emit(Word::ax(Opcode::ExtraArg, u32::from(outer_index))?);
        registers.reset(mark);
        return finish_value(code, destination, mode);
    }
    // Even a discarded primitive result must execute: vector-set! is
    // effectful, and numeric operations can raise type/overflow errors.
    let destination = match output_register(registers, mode)? {
        Some(destination) => destination,
        None => registers.acquire()?,
    };
    let mark = registers.mark();
    let left = operand_register(&arguments[0], env, code, registers)?;
    // Only immediate-`Value` literals fold into the k-bit constant operand;
    // `NumberLiteral`s have no constant-table representation (they materialize
    // via a cold `LoadNumber`) and take the register-operand path below.
    let literal_right = match (opcode, &arguments[1]) {
        (Opcode::VectorRef | Opcode::VectorSet | Opcode::StringRef, _) => None,
        (_, CoreExpr::Literal(value)) => Some(*value),
        _ => None,
    };
    let constant = literal_right
        .and_then(|value| code.constant(value).ok())
        .filter(|index| *index <= u32::from(u8::MAX));
    let right = if let Some(index) = constant {
        index as u8
    } else if opcode == Opcode::VectorSet {
        // The value operand must sit at register C+1, so the index needs a freshly
        // acquired register with the value acquired immediately after it. A home
        // register would break that adjacency.
        let register = registers.acquire()?;
        compile_expression(&arguments[1], env, code, registers, Mode::One(register))?;
        register
    } else {
        operand_register(&arguments[1], env, code, registers)?
    };
    if opcode == Opcode::VectorSet {
        let value = registers.acquire()?;
        compile_expression(&arguments[2], env, code, registers, Mode::One(value))?;
        code.emit(Word::abc(
            opcode,
            destination,
            left,
            right,
            constant.is_some(),
        ));
        // VectorSet reads its third argument from the register after C.
        debug_assert_eq!(value, right + 1);
    } else {
        let opcode = if constant.is_some() {
            specialize_numeric_fixnum_k(code, opcode, right)
        } else {
            opcode
        };
        code.emit(Word::abc(
            opcode,
            destination,
            left,
            right,
            constant.is_some(),
        ));
    }
    registers.reset(mark);
    finish_value(code, destination, mode)
}

/// Compiles an n-ary numeric primitive (`+ - * /`, arity >= 2) as a left fold of
/// two-argument opcodes: `(op a b c)` becomes `t = op(a, b)` then `op(t, c)`.
/// This mirrors the native operators, which fold left over the same binary
/// steps, while keeping every step on the fast register path (so, e.g.,
/// `(* 2.0 zr zi)` no longer reaches the generic native call). A literal final
/// operand of a step is folded in as an inline constant via the `k` bit.
fn compile_fold_call(
    opcode: Opcode,
    arguments: &[CoreExpr],
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    mode: Mode,
) -> Result<Option<u8>, Error> {
    let destination = match output_register(registers, mode)? {
        Some(destination) => destination,
        None => registers.acquire()?,
    };
    let mark = registers.mark();
    // The wide-literal accumulate `(- (+ v K1) K2)` with both literals inline
    // fixnums and `v` an unboxed local resident in the destination register
    // collapses into one `AddSubFixnumK` word (`rA = (rA + K1) - K2`), the
    // `(loop (- (+ acc K1) K2))` idiom. The home-register requirement makes
    // the in-place read/write sound, and the executor writes `rA` only after
    // both steps succeed, so a raising subtract leaves `v` untouched exactly
    // like the unfused pair (whose intermediate lived in a scratch).
    if opcode == Opcode::Subtract
        && arguments.len() == 2
        && let Some(word) = add_sub_fixnum_operands(arguments, destination, env, code)?
    {
        code.emit(word);
        registers.reset(mark);
        return finish_value(code, destination, mode);
    }
    let mut folded_first_step = false;
    // A two-argument fold has a single step that writes to `destination`, so the
    // accumulator register is only read. A non-boxed local first operand can then
    // be read straight from its home register with no `Move`. Multi-step folds
    // write the accumulator between steps, so they keep a private scratch.
    let accumulator = if arguments.len() == 2 {
        // When the single step's operand fuses into an in-place accumulate word
        // (`A = A op inner(..)`, see `fused_accumulate_operand`), the accumulator
        // must BE `destination`. The common `(+ acc (f ..))` loop idiom aligns
        // naturally (the home read below returns `destination`). Otherwise the
        // left operand can compile straight into `destination`, provided the
        // step operand cannot observe that early write: `destination` must not
        // be a named local's home register (argument expressions only read
        // named homes and their own fresh scratches). A home-readable left
        // operand skips the redirect. Ehe `Move` it would need costs exactly
        // what the fusion saves.
        if fused_accumulate_operand(opcode, &arguments[1], env)?.is_some()
            && !home_readable(&arguments[0], env)?
            && !register_is_named(env, destination)
        {
            compile_expression(&arguments[0], env, code, registers, Mode::One(destination))?;
            destination
        } else {
            operand_register(&arguments[0], env, code, registers)?
        }
    } else {
        let scratch = registers.acquire()?;
        // A commutative fold whose first operand is an immediate-`Value`
        // literal folds that constant into the first step as its RK operand
        // (`scratch = op(arg1, k)`), skipping the `LoadK` that would
        // otherwise re-materialize the constant on every evaluation (the
        // `(* 2.0 zr zi)` shape). Subtraction and division are ordered, so
        // they keep the materializing path.
        let first_literal = if matches!(opcode, Opcode::Add | Opcode::Multiply) {
            match &arguments[0] {
                CoreExpr::Literal(value) => code
                    .constant(*value)
                    .ok()
                    .filter(|index| *index <= u32::from(u8::MAX)),
                _ => None,
            }
        } else {
            None
        };
        if let Some(index) = first_literal {
            let submark = registers.mark();
            let right = operand_register(&arguments[1], env, code, registers)?;
            code.emit(Word::abc(
                specialize_numeric_fixnum_k(code, opcode, index as u8),
                scratch,
                right,
                index as u8,
                true,
            ));
            registers.reset(submark);
            folded_first_step = true;
        } else {
            compile_expression(&arguments[0], env, code, registers, Mode::One(scratch))?;
        }
        scratch
    };
    let last = arguments.len() - 1;
    let first_step = 1 + usize::from(folded_first_step);
    for (offset, argument) in arguments[first_step..].iter().enumerate() {
        let offset = offset + first_step - 1;
        // The final fold step writes straight to `destination`; earlier steps
        // accumulate back into the scratch accumulator register.
        let step_destination = if offset == last - 1 {
            destination
        } else {
            accumulator
        };
        // A step that writes back into its own accumulator register collapses
        // with a fusable nested primitive operand into one accumulate word.
        if step_destination == accumulator
            && let Some(fused) = fused_accumulate_operand(opcode, argument, env)?
        {
            emit_fused_accumulate(fused, step_destination, env, code, registers)?;
            continue;
        }
        // As in `compile_fast_call`: only immediate-`Value` literals fold via
        // the k bit; `NumberLiteral` steps load through a register instead.
        let literal = match argument {
            CoreExpr::Literal(value) => Some(*value),
            _ => None,
        };
        let constant = literal
            .and_then(|value| code.constant(value).ok())
            .filter(|index| *index <= u32::from(u8::MAX));
        if let Some(index) = constant {
            code.emit(Word::abc(
                specialize_numeric_fixnum_k(code, opcode, index as u8),
                step_destination,
                accumulator,
                index as u8,
                true,
            ));
        } else {
            let submark = registers.mark();
            let right = operand_register(argument, env, code, registers)?;
            code.emit(Word::abc(
                opcode,
                step_destination,
                accumulator,
                right,
                false,
            ));
            registers.reset(submark);
        }
    }
    registers.reset(mark);
    finish_value(code, destination, mode)
}

/// A fold-step operand that fuses with an in-place accumulate step into a
/// single word (`A = A op inner(..)`): the fused opcode plus the inner call's
/// operands.
enum FusedOperand<'a> {
    Unary(Opcode, &'a CoreExpr),
    Binary(Opcode, &'a CoreExpr, &'a CoreExpr),
    /// `(+ acc (* (vector-ref v1 i1) (vector-ref v2 i2)))` with all four inner
    /// operands home-readable, stored as `[v1, i1, v2, i2]`.
    DoubleVectorRef([&'a CoreExpr; 4]),
}

/// Recognizes a fold-step operand that pairs with `step` (the fold's opcode)
/// into a fused accumulate word. Only unshadowed `(scheme base)` operators
/// match (via `fast_operation`), so the fused patterns inherit the
/// redefinition guard of the plain fast paths.
fn fused_accumulate_operand<'a>(
    step: Opcode,
    argument: &'a CoreExpr,
    env: &mut Environment<'_>,
) -> Result<Option<FusedOperand<'a>>, Error> {
    let CoreExpr::Call {
        procedure,
        arguments,
    } = argument
    else {
        return Ok(None);
    };
    let Some(inner) = fast_operation(procedure, arguments.len()) else {
        return Ok(None);
    };
    Ok(match (step, inner) {
        (Opcode::Add, Opcode::VectorRef) => Some(FusedOperand::Binary(
            Opcode::AddVectorRef,
            &arguments[0],
            &arguments[1],
        )),
        (Opcode::Add, Opcode::Car) => Some(FusedOperand::Unary(Opcode::AddCar, &arguments[0])),
        // Only two-operand multiplies fuse; wider multiply folds keep their steps.
        (Opcode::Add, Opcode::Multiply) if arguments.len() == 2 => {
            // Both multiply operands being `vector-ref`s of home-readable
            // operands fuse the two element fetches into the accumulate word
            // itself. The home restriction is what preserves the unfused
            // evaluation order: no operand code may run between the fetches,
            // so the fetch misses defer in exactly the unfused error order.
            if let (Some(first), Some(second)) = (
                home_vector_ref_operands(&arguments[0], env)?,
                home_vector_ref_operands(&arguments[1], env)?,
            ) {
                return Ok(Some(FusedOperand::DoubleVectorRef([
                    first.0, first.1, second.0, second.1,
                ])));
            }
            Some(FusedOperand::Binary(
                Opcode::AddMul,
                &arguments[0],
                &arguments[1],
            ))
        }
        (Opcode::Subtract, Opcode::Multiply) if arguments.len() == 2 => Some(FusedOperand::Binary(
            Opcode::SubMul,
            &arguments[0],
            &arguments[1],
        )),
        (Opcode::Add, Opcode::CharToInteger) => {
            let CoreExpr::Call {
                procedure,
                arguments,
            } = &arguments[0]
            else {
                return Ok(None);
            };
            if fast_operation(procedure, arguments.len()) == Some(Opcode::StringRef) {
                Some(FusedOperand::Binary(
                    Opcode::AddStringRefCode,
                    &arguments[0],
                    &arguments[1],
                ))
            } else {
                None
            }
        }
        _ => None,
    })
}

/// The `(vector-ref v i)` shape whose two operands are read straight from
/// local home registers (no code emitted), as the double-fetch fusion
/// requires.
fn home_vector_ref_operands<'a>(
    argument: &'a CoreExpr,
    env: &mut Environment<'_>,
) -> Result<Option<(&'a CoreExpr, &'a CoreExpr)>, Error> {
    let CoreExpr::Call {
        procedure,
        arguments,
    } = argument
    else {
        return Ok(None);
    };
    if fast_operation(procedure, arguments.len()) != Some(Opcode::VectorRef) {
        return Ok(None);
    }
    if home_readable(&arguments[0], env)? && home_readable(&arguments[1], env)? {
        Ok(Some((&arguments[0], &arguments[1])))
    } else {
        Ok(None)
    }
}

/// Emits a fused accumulate word: `destination` is both the accumulator and
/// the target (`A = A op ..`); the inner operands land exactly as the unfused
/// pair's would.
fn emit_fused_accumulate(
    fused: FusedOperand<'_>,
    destination: u8,
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
) -> Result<(), Error> {
    let mark = registers.mark();
    match fused {
        FusedOperand::Unary(fusion, operand) => {
            let source = operand_register(operand, env, code, registers)?;
            code.emit(Word::abc(fusion, destination, source, 0, false));
        }
        FusedOperand::Binary(fusion, left, right) => {
            let left = operand_register(left, env, code, registers)?;
            // `AddMul`/`SubMul` keep the arithmetic RK form on C; the element
            // fusions read C as a register (mirroring `VectorRef`/`StringRef`).
            let constant = match (fusion, right) {
                (Opcode::AddMul | Opcode::SubMul, CoreExpr::Literal(value)) => code
                    .constant(*value)
                    .ok()
                    .filter(|index| *index <= u32::from(u8::MAX)),
                _ => None,
            };
            let (right, k) = match constant {
                Some(index) => (index as u8, true),
                None => (operand_register(right, env, code, registers)?, false),
            };
            code.emit(Word::abc(fusion, destination, left, right, k));
        }
        FusedOperand::DoubleVectorRef([first_vector, first_index, second_vector, second_index]) => {
            // All four operands are home reads (enforced by the recognizer),
            // so these emit no code and the two fetches stay adjacent.
            let first_vector = operand_register(first_vector, env, code, registers)?;
            let first_index = operand_register(first_index, env, code, registers)?;
            let second_vector = operand_register(second_vector, env, code, registers)?;
            let second_index = operand_register(second_index, env, code, registers)?;
            code.emit(Word::abc(
                Opcode::AddMulVectorRef,
                destination,
                first_vector,
                first_index,
                false,
            ));
            code.emit(Word::ax(
                Opcode::ExtraArg,
                (u32::from(second_vector) << 8) | u32::from(second_index),
            )?);
        }
    }
    registers.reset(mark);
    Ok(())
}

/// Recognizes the fused wide-literal accumulate operand pair for
/// `AddSubFixnumK`: the outer subtract's left argument is a two-argument
/// primitive `(+ v K1)` (in either operand order - addition is commutative and
/// a literal cannot raise), its right argument is `K2`, both literals are
/// inline fixnums whose constant-table indexes fit the one-byte `B`/`C`
/// fields, and `v` is an unboxed local whose home register IS `destination`.
/// Returns the fully-formed word, or `None` to keep the unfused pair. `K1` is
/// interned before `K2`, matching the order the unfused spelling would use.
fn add_sub_fixnum_operands(
    arguments: &[CoreExpr],
    destination: u8,
    env: &mut Environment<'_>,
    code: &mut Code,
) -> Result<Option<Word>, Error> {
    let CoreExpr::Literal(second) = &arguments[1] else {
        return Ok(None);
    };
    if second.as_fixnum().is_none() {
        return Ok(None);
    }
    let CoreExpr::Call {
        procedure,
        arguments: inner,
    } = &arguments[0]
    else {
        return Ok(None);
    };
    if inner.len() != 2 || fast_operation(procedure, 2) != Some(Opcode::Add) {
        return Ok(None);
    }
    let (variable, first) = match (&inner[0], &inner[1]) {
        (CoreExpr::Variable(name), CoreExpr::Literal(value)) => (name, value),
        (CoreExpr::Literal(value), CoreExpr::Variable(name)) => (name, value),
        _ => return Ok(None),
    };
    if first.as_fixnum().is_none() {
        return Ok(None);
    }
    match env.resolve(variable)? {
        Access::Local(home) if home == destination && !env.is_boxed_local(home) => {}
        _ => return Ok(None),
    }
    let Some(add_index) = code
        .constant(*first)
        .ok()
        .filter(|index| *index <= u32::from(u8::MAX))
    else {
        return Ok(None);
    };
    let Some(sub_index) = code
        .constant(*second)
        .ok()
        .filter(|index| *index <= u32::from(u8::MAX))
    else {
        return Ok(None);
    };
    Ok(Some(Word::abc(
        Opcode::AddSubFixnumK,
        destination,
        add_index as u8,
        sub_index as u8,
        true,
    )))
}

/// Whether `operand_register` would read `argument` straight from a local home
/// register (emitting no code).
fn home_readable(argument: &CoreExpr, env: &mut Environment<'_>) -> Result<bool, Error> {
    if let CoreExpr::Variable(name) = argument {
        Ok(matches!(env.resolve(name)?, Access::Local(home) if !env.is_boxed_local(home)))
    } else {
        Ok(false)
    }
}

/// Whether any in-scope local of the current frame lives in `register`
/// (compiler temporaries are anonymous, so a `false` proves the register is a
/// scratch slot nothing else can read).
fn register_is_named(env: &Environment<'_>, register: u8) -> bool {
    env.locals.values().any(|&home| home == register)
}

pub(super) fn compile_values(
    values: &[CoreExpr],
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    mode: Mode,
) -> Result<Option<u8>, Error> {
    match mode {
        Mode::Discard => {
            for value in values {
                compile_expression(value, env, code, registers, Mode::Discard)?;
            }
            Ok(Some(0))
        }
        Mode::One(destination) if values.len() == 1 => {
            compile_expression(&values[0], env, code, registers, Mode::One(destination))
        }
        Mode::One(_) => {
            for value in values {
                compile_expression(value, env, code, registers, Mode::Discard)?;
            }
            code.emit_cold(ColdInstruction::ValueCountError {
                expected: 1,
                actual: values.len(),
            })?;
            Ok(None)
        }
        Mode::All(_) | Mode::Return => {
            let first = if let Mode::All(first) = mode {
                first
            } else {
                registers.acquire_block(values.len().max(1))?
            };
            registers.ensure(usize::from(first) + values.len().max(1))?;
            for (offset, value) in values.iter().enumerate() {
                compile_expression(value, env, code, registers, Mode::One(first + offset as u8))?;
            }
            if matches!(mode, Mode::Return) {
                code.emit(Word::abc(
                    Opcode::Return,
                    first,
                    (values.len() + 1) as u8,
                    0,
                    false,
                ));
            }
            Ok(Some(values.len() as u8))
        }
    }
}

pub(super) fn compile_unary_cold<F>(
    value: &CoreExpr,
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    mode: Mode,
    build: F,
) -> Result<Option<u8>, Error>
where
    F: FnOnce(u8, u8, ExpectedResults) -> ColdInstruction,
{
    let mark = registers.mark();
    let source = registers.acquire()?;
    compile_expression(value, env, code, registers, Mode::One(source))?;
    let result = emit_cold_result(code, registers, mode, |destination, expected| {
        build(destination, source, expected)
    })?;
    registers.reset(mark);
    Ok(result)
}

pub(super) fn compile_cold_call<const N: usize, F>(
    inputs: [&CoreExpr; N],
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    mode: Mode,
    build: F,
) -> Result<Option<u8>, Error>
where
    F: FnOnce(u8, [u8; N], ExpectedResults) -> ColdInstruction,
{
    let mark = registers.mark();
    let first = registers.acquire_block(N)?;
    let registers_array = std::array::from_fn(|index| first + index as u8);
    for (input, register) in inputs.into_iter().zip(registers_array) {
        compile_expression(input, env, code, registers, Mode::One(register))?;
    }
    let result = emit_cold_result(code, registers, mode, |destination, expected| {
        build(destination, registers_array, expected)
    })?;
    registers.reset(mark);
    Ok(result)
}

pub(super) fn emit_cold_result<F>(
    code: &mut Code,
    registers: &mut Registers,
    mode: Mode,
    build: F,
) -> Result<Option<u8>, Error>
where
    F: FnOnce(u8, ExpectedResults) -> ColdInstruction,
{
    let destination =
        output_register(registers, mode)?.unwrap_or_else(|| registers.acquire().expect("register"));
    code.emit_cold(build(destination, mode.expected()))?;
    if matches!(mode, Mode::Return) {
        code.emit(Word::abc(Opcode::Return, destination, 0, 0, false));
        Ok(None)
    } else {
        Ok(match mode {
            Mode::Discard => Some(0),
            Mode::One(_) => Some(1),
            Mode::All(_) => None,
            Mode::Return => None,
        })
    }
}

pub(super) fn output_register(registers: &mut Registers, mode: Mode) -> Result<Option<u8>, Error> {
    Ok(match mode {
        Mode::One(register) | Mode::All(register) => {
            registers.ensure(usize::from(register) + 1)?;
            Some(register)
        }
        Mode::Discard => None,
        Mode::Return => Some(registers.acquire()?),
    })
}

pub(super) fn finish_value(code: &mut Code, register: u8, mode: Mode) -> Result<Option<u8>, Error> {
    if matches!(mode, Mode::Return) {
        code.emit(Word::abc(Opcode::Return, register, 2, 0, false));
        Ok(None)
    } else {
        Ok(Some(if matches!(mode, Mode::Discard) { 0 } else { 1 }))
    }
}

pub(super) fn finish_produced(
    code: &mut Code,
    register: u8,
    produced: Option<u8>,
    mode: Mode,
) -> Result<Option<u8>, Error> {
    if matches!(mode, Mode::Return) {
        code.emit(Word::abc(
            Opcode::Return,
            register,
            produced.map_or(0, |count| count + 1),
            0,
            false,
        ));
        Ok(None)
    } else {
        Ok(produced)
    }
}

pub(super) fn emit_call(code: &mut Code, base: u8, arguments: usize, mode: Mode) {
    code.emit(Word::abc(
        Opcode::Call,
        base,
        (arguments + 1) as u8,
        mode.expected().call_field(),
        false,
    ));
}

pub(super) fn finish_call_result(
    code: &mut Code,
    base: u8,
    mode: Mode,
) -> Result<Option<u8>, Error> {
    match mode {
        Mode::One(destination) => {
            if destination != base {
                code.emit(Word::abc(Opcode::Move, destination, base, 0, false));
            }
            Ok(Some(1))
        }
        Mode::Discard => Ok(Some(0)),
        Mode::All(destination) => {
            if destination != base {
                return Err(compile_error(
                    "dynamic results require their call destination",
                ));
            }
            Ok(None)
        }
        Mode::Return => {
            code.emit(Word::abc(Opcode::Return, base, 0, 0, false));
            Ok(None)
        }
    }
}

/// Resolves an operand for a register opcode. When `argument` is a non-boxed
/// local variable it already lives in a stable home register, so that register
/// is returned directly as the operand, eliding the redundant `Move` into a
/// fresh scratch that `compile_expression(.., Mode::One(scratch))` would emit.
///
/// This is sound because a non-boxed local is never mutated (a `set!` forces the
/// variable to be boxed, which this excludes) and a call only ever clobbers the
/// scratch registers above the locals watermark, never a live local's home slot.
/// So the home register still holds the variable's value wherever the operand is
/// read. Any other argument shape is compiled into a freshly acquired register.
pub(super) fn operand_register(
    argument: &CoreExpr,
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
) -> Result<u8, Error> {
    if let CoreExpr::Variable(name) = argument
        && let Access::Local(home) = env.resolve(name)?
        && !env.is_boxed_local(home)
    {
        return Ok(home);
    }
    // A generic (frame-pushing) call leaves its single result in its own base
    // register. If we let that base coincide with the destination the call would
    // otherwise be moved into, the `Move base -> destination` disappears: compile
    // the call with `Mode::One` targeting the current watermark, which is exactly
    // where `compile_call` will place the call block (so `destination == base`).
    // The call restores the watermark on return, so re-acquiring yields the same
    // register, now holding the result and reserved until the caller's reset.
    if let CoreExpr::Call {
        procedure,
        arguments,
    } = argument
        && is_generic_call(procedure, arguments, code)
        && registers.mark() < MAX_REGISTERS as u16
    {
        let landing = registers.mark() as u8;
        compile_expression(argument, env, code, registers, Mode::One(landing))?;
        let register = registers.acquire()?;
        debug_assert_eq!(register, landing);
        return Ok(register);
    }
    let register = registers.acquire()?;
    compile_expression(argument, env, code, registers, Mode::One(register))?;
    Ok(register)
}

/// Whether `(procedure arguments)` compiles through the generic call path. The
/// only path whose result lands in a fresh base register (and thus needs the
/// operand-alignment above to avoid a `Move`). A flattened-loop tail call, an
/// inlinable immediately-applied lambda, and a fast primitive each write their
/// result register directly, so they are excluded. Mirrors the dispatch in
/// [`compile_call`]; operands are never in `Mode::All`, so the inline predicate
/// matches `compile_call`'s decision here.
fn is_generic_call(procedure: &CoreExpr, arguments: &[CoreExpr], code: &Code) -> bool {
    if let CoreExpr::Variable(name) = procedure
        && code.loops.iter().any(|frame| frame.name == *name)
    {
        return false;
    }
    if let CoreExpr::Lambda { params, .. } = procedure
        && can_inline_application(params, arguments)
    {
        return false;
    }
    fast_operation(procedure, arguments.len()).is_none()
}

pub(super) fn fast_operation(procedure: &CoreExpr, arity: usize) -> Option<Opcode> {
    let CoreExpr::Variable(name) = procedure else {
        return None;
    };
    if !name.starts_with("\u{1f}library:(scheme base):") {
        return None;
    }
    let name = name.rsplit(':').next()?;
    // `+ - * /` with two or more arguments compile to a left fold of two-argument
    // opcodes (see `compile_fold_call`); the native operators fold identically.
    // The 0/1-argument identity, negate, and reciprocal forms stay on the generic
    // path, as do the pairwise n-ary comparisons.
    Some(match name {
        "+" if arity >= 2 => Opcode::Add,
        "-" if arity >= 2 => Opcode::Subtract,
        "*" if arity >= 2 => Opcode::Multiply,
        "/" if arity >= 2 => Opcode::Divide,
        "=" if arity == 2 => Opcode::NumericEqual,
        "<" if arity == 2 => Opcode::NumericLess,
        ">" if arity == 2 => Opcode::NumericGreater,
        "<=" if arity == 2 => Opcode::NumericLessEqual,
        ">=" if arity == 2 => Opcode::NumericGreaterEqual,
        "vector-ref" if arity == 2 => Opcode::VectorRef,
        "vector-set!" if arity == 3 => Opcode::VectorSet,
        "cons" if arity == 2 => Opcode::Cons,
        "car" if arity == 1 => Opcode::Car,
        "cdr" if arity == 1 => Opcode::Cdr,
        "null?" if arity == 1 => Opcode::NullP,
        "pair?" if arity == 1 => Opcode::PairP,
        "string-ref" if arity == 2 => Opcode::StringRef,
        "string-length" if arity == 1 => Opcode::StringLength,
        "char->integer" if arity == 1 => Opcode::CharToInteger,
        _ => return None,
    })
}
