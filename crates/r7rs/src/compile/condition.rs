//! Boolean-context lowering of conditions: leaf comparison fusion into
//! `Test` + `Jump`, and short-circuiting `and`/`or`/`not`.

use super::{
    analysis::mentions_variable,
    expr::{fast_operation, operand_register},
    *,
};

/// A branch word awaiting a jump target, paired with the register to re-supply
/// when it is a `JumpFalse` (`None` for an unconditional `Jump`, e.g. the
/// successor of a fused `Test`).
pub(super) type PendingJump = (usize, Option<u8>);

/// Patches every pending branch to `target`.
pub(super) fn patch_all(
    code: &mut Code,
    jumps: &[PendingJump],
    target: usize,
) -> Result<(), Error> {
    for &(at, register) in jumps {
        code.patch_jump(at, target, register)?;
    }
    Ok(())
}

/// Compiles `cond` in boolean context. Emits code that transfers control
/// through one of the returned branch words when `cond`'s truth value equals
/// `want`, and falls through to the next instruction otherwise. Leaf numeric
/// comparisons fuse into a `Test` + `Jump` pair. `and`/`or`/`not` short-circuit
/// down to their leaves. Anything else materializes a boolean and branches on
/// it.
pub(super) fn compile_condition(
    cond: &CoreExpr,
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    want: bool,
) -> Result<Vec<PendingJump>, Error> {
    if let CoreExpr::Call {
        procedure,
        arguments,
    } = cond
    {
        // Leaf: a two-argument numeric comparison fuses into `Test` + `Jump`.
        if arguments.len() == 2
            && let Some(operation) = fast_operation(procedure, 2)
            && matches!(
                operation,
                Opcode::NumericEqual
                    | Opcode::NumericLess
                    | Opcode::NumericGreater
                    | Opcode::NumericLessEqual
                    | Opcode::NumericGreaterEqual
            )
        {
            return Ok(vec![compile_test_leaf(
                operation, arguments, want, env, code, registers,
            )?]);
        }
        // Leaf: a `null?`/`pair?` predicate fuses into `Test` + `Jump`.
        if arguments.len() == 1
            && let Some(operation) = fast_operation(procedure, 1)
            && matches!(operation, Opcode::NullP | Opcode::PairP)
        {
            let test = match operation {
                Opcode::NullP => Opcode::TestNull,
                _ => Opcode::TestPair,
            };
            let mark = registers.mark();
            let source = operand_register(&arguments[0], env, code, registers)?;
            // `A` carries the polarity, exactly as in the comparison leaves.
            code.emit(Word::abc(test, u8::from(want), source, 0, false));
            let jump = code.emit(Word::sj(Opcode::Jump, 0)?);
            registers.reset(mark);
            return Ok(vec![(jump, None)]);
        }
        // Leaf: `(vector-ref v i)` as the condition fuses into `TestVectorRef`
        // + `Jump`: the element is fetched and branched on directly (Scheme
        // truthiness - anything but `#f` - exactly `JumpFalse`'s test),
        // replacing the separate `VectorRef` + `JumpFalse` pair. Both operands
        // are registers, mirroring `VectorRef`'s register-only index rule in
        // the fused accumulate family. Errors (non-vector, bad index) defer to
        // the same slow path as the standalone `VectorRef`, so they are
        // identical.
        if arguments.len() == 2 && fast_operation(procedure, 2) == Some(Opcode::VectorRef) {
            let mark = registers.mark();
            let vector = operand_register(&arguments[0], env, code, registers)?;
            let index = operand_register(&arguments[1], env, code, registers)?;
            code.emit(Word::abc(
                Opcode::TestVectorRef,
                u8::from(want),
                vector,
                index,
                false,
            ));
            let jump = code.emit(Word::sj(Opcode::Jump, 0)?);
            registers.reset(mark);
            return Ok(vec![(jump, None)]);
        }
        // `(not x)` inverts the wanted polarity.
        if arguments.len() == 1 && is_primitive(procedure, "not") {
            return compile_condition(&arguments[0], env, code, registers, !want);
        }
        // `(or first rest)` desugars to `((lambda (n) (if n n rest)) first)`.
        if let Some((first, rest)) = match_or_shape(procedure, arguments) {
            return compile_or(first, rest, env, code, registers, want);
        }
    }
    // `(and a b)` desugars to `(if a b #f)`; any `(if a b #f)` has the same
    // truth value as `a && b` in boolean context.
    if let CoreExpr::If(a, b, alternate) = cond
        && is_false_literal(alternate)
    {
        return compile_and(a, b, env, code, registers, want);
    }
    // Fallback: materialize the truth value into a register and branch on it.
    let mark = registers.mark();
    let register = registers.acquire()?;
    compile_expression(cond, env, code, registers, Mode::One(register))?;
    registers.reset(mark);
    if want {
        // Jump when truthy: a `JumpFalse` skips the following `Jump` on falsity,
        // otherwise the `Jump` (patched to the target) fires.
        code.emit(Word::asbx(Opcode::JumpFalse, register, 1)?);
        let jump = code.emit(Word::sj(Opcode::Jump, 0)?);
        Ok(vec![(jump, None)])
    } else {
        let branch = code.emit(Word::asbx(Opcode::JumpFalse, register, 0)?);
        Ok(vec![(branch, Some(register))])
    }
}

/// Emits a fused `Test` + placeholder `Jump` for a two-argument numeric
/// comparison. The right operand may be an inline constant via the `k` bit.
/// `>`/`>=` with a literal right operand keep their own opcodes
/// (`TestGreater`/`TestGreaterEqual`), so the literal lands in the k slot
/// instead of paying a per-evaluation `LoadK`. With a non-literal right
/// operand they reuse `TestLess`/`TestLessEqual` with the operands swapped
/// (sound because every comparison derives from one antisymmetric ordering,
/// and the compare kind stays in opcode identity). The swapped register
/// shape is also what the loop back-edge fusion anchors on, so it must stay.
fn compile_test_leaf(
    operation: Opcode,
    arguments: &[CoreExpr],
    want: bool,
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
) -> Result<PendingJump, Error> {
    let right_is_literal = matches!(&arguments[1], CoreExpr::Literal(_));
    let (test, swap) = match operation {
        Opcode::NumericLess => (Opcode::TestLess, false),
        Opcode::NumericLessEqual => (Opcode::TestLessEqual, false),
        Opcode::NumericEqual => (Opcode::TestEqual, false),
        Opcode::NumericGreater if right_is_literal => (Opcode::TestGreater, false),
        Opcode::NumericGreaterEqual if right_is_literal => (Opcode::TestGreaterEqual, false),
        Opcode::NumericGreater => (Opcode::TestLess, true),
        Opcode::NumericGreaterEqual => (Opcode::TestLessEqual, true),
        _ => return Err(compile_error("non-comparison in boolean context")),
    };
    let (left_argument, right_argument) = if swap {
        (&arguments[1], &arguments[0])
    } else {
        (&arguments[0], &arguments[1])
    };
    let mark = registers.mark();
    let left = operand_register(left_argument, env, code, registers)?;
    // Only immediate-`Value` literals fold into the k-bit operand;
    // `NumberLiteral`s materialize via a cold `LoadNumber` register load.
    let literal_right = match right_argument {
        CoreExpr::Literal(value) => Some(*value),
        _ => None,
    };
    let constant = literal_right
        .and_then(|value| code.constant(value).ok())
        .filter(|index| *index <= u32::from(u8::MAX));
    let right = if let Some(index) = constant {
        index as u8
    } else {
        operand_register(right_argument, env, code, registers)?
    };
    // A fixnum constant takes the specialized test word, letting the executor
    // compare raw payloads without re-classifying the constant.
    let test = if constant.is_some() {
        specialize_test_fixnum_k(code, test, right)
    } else {
        test
    };
    // `A` carries the polarity: the following `Jump` fires when the comparison
    // result equals `want`.
    code.emit(Word::abc(
        test,
        u8::from(want),
        left,
        right,
        constant.is_some(),
    ));
    let jump = code.emit(Word::sj(Opcode::Jump, 0)?);
    registers.reset(mark);
    Ok((jump, None))
}

/// Short-circuit lowering of `a && b` in boolean context.
fn compile_and(
    a: &CoreExpr,
    b: &CoreExpr,
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    want: bool,
) -> Result<Vec<PendingJump>, Error> {
    if want {
        // Jump when both true. If `a` is false the conjunction is false, so
        // route past `b` to the fall-through point.
        let skip = compile_condition(a, env, code, registers, false)?;
        let exits = compile_condition(b, env, code, registers, true)?;
        patch_all(code, &skip, code.words.len())?;
        Ok(exits)
    } else {
        // Jump when either is false.
        let mut exits = compile_condition(a, env, code, registers, false)?;
        exits.extend(compile_condition(b, env, code, registers, false)?);
        Ok(exits)
    }
}

/// Short-circuit lowering of `first || rest` in boolean context.
fn compile_or(
    first: &CoreExpr,
    rest: &CoreExpr,
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    want: bool,
) -> Result<Vec<PendingJump>, Error> {
    if want {
        // Jump when either is true.
        let mut exits = compile_condition(first, env, code, registers, true)?;
        exits.extend(compile_condition(rest, env, code, registers, true)?);
        Ok(exits)
    } else {
        // Jump when both false. If `first` is true the disjunction is true, so
        // route past `rest` to the fall-through point.
        let skip = compile_condition(first, env, code, registers, true)?;
        let exits = compile_condition(rest, env, code, registers, false)?;
        patch_all(code, &skip, code.words.len())?;
        Ok(exits)
    }
}

/// Recognizes the expander's `or` desugaring
/// `((lambda (n) (if n n rest)) first)`, returning `(first, rest)`. The guard
/// that `rest` does not mention `n` keeps the rewrite sound for any `CoreExpr`
/// with this shape (the expander's `n` is a gensym, so it always holds).
fn match_or_shape<'a>(
    procedure: &'a CoreExpr,
    arguments: &'a [CoreExpr],
) -> Option<(&'a CoreExpr, &'a CoreExpr)> {
    let CoreExpr::Lambda { params, body } = procedure else {
        return None;
    };
    if params.len() != 1 || arguments.len() != 1 {
        return None;
    }
    let CoreExpr::If(test, consequent, rest) = body.as_ref() else {
        return None;
    };
    let name = &params[0];
    if !is_variable(test, name) || !is_variable(consequent, name) || mentions_variable(rest, name) {
        return None;
    }
    Some((&arguments[0], rest.as_ref()))
}

fn is_variable(expr: &CoreExpr, name: &str) -> bool {
    matches!(expr, CoreExpr::Variable(value) if value == name)
}

fn is_false_literal(expr: &CoreExpr) -> bool {
    matches!(expr, CoreExpr::Literal(value) if *value == Value::boolean(false))
}

/// Whether `procedure` names the `(scheme base)` primitive `name`, using the
/// same hygienic prefix that `fast_operation` relies on (so a user rebinding of
/// the identifier does not match).
fn is_primitive(procedure: &CoreExpr, name: &str) -> bool {
    let CoreExpr::Variable(value) = procedure else {
        return false;
    };
    value.starts_with("\u{1f}library:(scheme base):") && value.rsplit(':').next() == Some(name)
}
