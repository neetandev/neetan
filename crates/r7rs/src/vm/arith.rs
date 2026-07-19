//! Register/constant primitives and fast-path arithmetic.

use super::*;

#[inline(always)]
pub(super) fn fetch_word(code: &[Word], pc: usize) -> Word {
    debug_assert!(pc < code.len(), "instruction fetch out of bounds");
    // SAFETY: the bytecode verifier proves every pc the dispatch loop fetches
    // is in bounds: chunks are non-empty (entry pc 0), no instruction may fall
    // off the end of the code (the fall-through rule, which also covers the
    // consumed `ExtraArg`/`Jump` successor words of `LoadKx`/`Test*`/
    // `LoopBack`), every jump target is bounds-checked (`jump_target`), and a
    // resumed `frame.pc` is always a saved fall-through pc.
    #[allow(unsafe_code)]
    unsafe {
        *code.get_unchecked(pc)
    }
}

#[inline(always)]
pub(super) fn read_constant(constants: &[Value], index: usize) -> Value {
    debug_assert!(index < constants.len(), "constant read out of bounds");
    // SAFETY: every `index` reaching this helper is a `LoadK`/`LoadKx`/RK
    // constant operand, and the bytecode verifier proves each one is
    // `< chunk.constants.len()`.
    #[allow(unsafe_code)]
    unsafe {
        *constants.get_unchecked(index)
    }
}

#[inline(always)]
pub(super) fn read_register(stack: &RegisterStack, index: usize) -> Value {
    debug_assert!(index < stack.0.len(), "register read out of bounds");
    // SAFETY: every `index` reaching this helper is `frame.base + operand`, and
    // the bytecode verifier proves `operand < chunk.max_registers` while each
    // frame is `ensure`d to `base + max_registers <= stack.len()` on entry (the
    // register file is grow-only, never truncated).
    #[allow(unsafe_code)]
    unsafe {
        *stack.0.get_unchecked(index)
    }
}

#[inline(always)]
pub(super) fn write_register(stack: &mut RegisterStack, index: usize, value: Value) {
    debug_assert!(index < stack.0.len(), "register write out of bounds");
    // SAFETY: identical invariant to `read_register`. `index` is a
    // verifier-bounded register operand added to a frame base whose window is
    // `ensure`d to fit within the grow-only register file.
    #[allow(unsafe_code)]
    unsafe {
        *stack.0.get_unchecked_mut(index) = value;
    }
}

/// Inlined monomorphic front-end for the arithmetic/comparison arms. Handles only
/// the two overwhelmingly common shapes, both operands inline `f64`, or both inline
/// fixnums, and returns `None` for everything else so the caller defers to the out-of-line
/// [`register_numeric`]. Kept tiny and `inline(always)` so the ~90% path lands
/// directly in the dispatch loop while the full numeric tower stays out-of-line
/// (protecting I-cache). Any case this misses is handled identically by
/// `register_numeric`, so it is purely a fast front-end with no behavior change.
#[inline(always)]
pub(super) fn numeric_fast(opcode: Opcode, left: Value, right: Value) -> Option<Value> {
    // Both operands are classified by one combined scalar (`pair_key`), so
    // the dominant all-fixnum case costs a single compare and the all-float
    // case exactly one more. Mixed or heap-backed operands (and fixnum
    // overflow) fall to the out-of-line tower.
    let key = Value::pair_key(left, right);
    if key == Value::PAIR_BOTH_FIXNUM {
        let (left, right) = (left.fixnum_payload(), right.fixnum_payload());
        return match opcode {
            Opcode::Add => left.checked_add(right).map(Value::integer),
            Opcode::Subtract => left.checked_sub(right).map(Value::integer),
            Opcode::Multiply => left.checked_mul(right).map(Value::integer),
            Opcode::NumericEqual => Some(Value::boolean(left == right)),
            Opcode::NumericLess => Some(Value::boolean(left < right)),
            Opcode::NumericGreater => Some(Value::boolean(left > right)),
            Opcode::NumericLessEqual => Some(Value::boolean(left <= right)),
            Opcode::NumericGreaterEqual => Some(Value::boolean(left >= right)),
            // Fixnum division (exactness/zero checks) stays on the general path.
            _ => None,
        };
    }
    if key == Value::PAIR_BOTH_FLOAT {
        return f64_numeric(opcode, left.float_payload(), right.float_payload());
    }
    // Mixed fixnum/float add/subtract/multiply promote the fixnum inline,
    // exactly as `register_numeric` would (`as f64` then a raw rebox). Kept
    // deliberately lean: comparisons (which need the exact-conversion guard)
    // and division (zero/exactness handling) defer to the identical
    // out-of-line chain, so this adds no code to those paths.
    let (l, r) = if key == Value::PAIR_FIXNUM_FLOAT {
        (left.fixnum_payload() as f64, right.float_payload())
    } else if key == Value::PAIR_FLOAT_FIXNUM {
        (left.float_payload(), right.fixnum_payload() as f64)
    } else {
        return None;
    };
    match opcode {
        Opcode::Add => Some(Value::float_raw(l + r)),
        Opcode::Subtract => Some(Value::float_raw(l - r)),
        Opcode::Multiply => Some(Value::float_raw(l * r)),
        _ => None,
    }
}

/// Out-of-line tail of the arithmetic/comparison arms: the full mixed
/// fixnum/float tower, then the heap-backed numeric native. Outlined so the
/// dispatch arms carry only the tiny [`numeric_fast`] front-end and none of
/// this path's register pressure. Deliberately NOT `#[cold]`: mixed
/// fixnum/float operands land here legitimately.
#[inline(never)]
pub(super) fn numeric_slow(
    opcode: Opcode,
    heap: &mut Heap,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    left: Value,
    right: Value,
) -> Result<Value, Error> {
    if let Some(value) = register_numeric(opcode, left, right) {
        return Ok(value);
    }
    exactly_one(invoke_register_operation(
        opcode,
        heap,
        globals,
        symbols,
        natives,
        &[left, right],
    )?)
}

/// Out-of-line miss path of the fused `AddSubFixnumK` word: re-runs the whole
/// word as the unfused `AddFixnumK` + `SubtractFixnumK` pair would, in order
/// (checked fixnum add, else the numeric tower, then the checked subtract on
/// the intermediate, else the tower again). Reads are pure, so the re-run is
/// unobservable, and the caller writes the accumulator register only on
/// success, so a raising step leaves it untouched exactly like the unfused
/// pair (whose intermediate lived in a scratch register). Not `#[cold]` for
/// the same reason as [`numeric_slow`]: a float accumulator lands here
/// legitimately on every iteration.
#[inline(never)]
pub(super) fn fused_add_sub_fixnum_k(
    heap: &mut Heap,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    value: Value,
    add: i64,
    subtract: i64,
) -> Result<Value, Error> {
    let sum = match value
        .as_fixnum()
        .and_then(|current| current.checked_add(add))
    {
        Some(sum) => Value::integer(sum),
        None => numeric_slow(
            Opcode::Add,
            heap,
            globals,
            symbols,
            natives,
            value,
            Value::integer(add),
        )?,
    };
    match sum
        .as_fixnum()
        .and_then(|current| current.checked_sub(subtract))
    {
        Some(result) => Ok(Value::integer(result)),
        None => numeric_slow(
            Opcode::Subtract,
            heap,
            globals,
            symbols,
            natives,
            sum,
            Value::integer(subtract),
        ),
    }
}

/// Cold rescue for the inline pair/vector/loop-counter fast paths: defers to
/// the generic native, which either computes the rare valid case (heap-backed
/// numbers, exotic representations) or raises the proper error.
#[cold]
#[inline(never)]
pub(super) fn register_operation_slow(
    opcode: Opcode,
    heap: &mut Heap,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    arguments: &[Value],
) -> Result<Value, Error> {
    exactly_one(invoke_register_operation(
        opcode, heap, globals, symbols, natives, arguments,
    )?)
}

/// Shared accumulate tail of the fused opcodes: `rA = rA op element` with the
/// standalone arithmetic arm's fast/slow split.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn fused_accumulate_step(
    step: Opcode,
    stack: &mut RegisterStack,
    heap: &mut Heap,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    target: usize,
    element: Value,
) -> Result<(), Error> {
    let accumulator = read_register(stack, target);
    let value = match numeric_fast(step, accumulator, element) {
        Some(value) => value,
        None => numeric_slow(step, heap, globals, symbols, natives, accumulator, element)?,
    };
    write_register(stack, target, value);
    Ok(())
}

/// Out-of-line executor for `AddVectorRef`: `rA = rA + (vector-ref rB rC)`.
///
/// The fused bodies stay out of `execute` deliberately: inlined, their live
/// ranges cost the dispatch loop a register (measured as a per-dispatch spill
/// of the retirement counter and a uniform instruction ripple on every
/// untouched loop shape). A fused word already saves a whole dispatch, so the
/// call is well amortized. Each body runs the constituent fast paths in the
/// unfused source order, and every miss defers to the same native the unfused
/// word pair would have invoked, so errors are identical.
#[inline(never)]
pub(super) fn fused_add_vector_ref(
    stack: &mut RegisterStack,
    heap: &mut Heap,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    word: Word,
    base: usize,
) -> Result<(), Error> {
    let vector = read_register(stack, base + usize::from(word.b()));
    let index = read_register(stack, base + usize::from(word.c()));
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
    let target = base + usize::from(word.a());
    fused_accumulate_step(
        Opcode::Add,
        stack,
        heap,
        globals,
        symbols,
        natives,
        target,
        element,
    )
}

/// Out-of-line executor for `AddCar`: `rA = rA + (car rB)`.
/// See [`fused_add_vector_ref`] for the outlining rationale.
#[inline(never)]
pub(super) fn fused_add_car(
    stack: &mut RegisterStack,
    heap: &mut Heap,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    word: Word,
    base: usize,
) -> Result<(), Error> {
    let pair = read_register(stack, base + usize::from(word.b()));
    let element = match heap.pair(pair) {
        Some((car, _)) => car,
        None => register_operation_slow(Opcode::Car, heap, globals, symbols, natives, &[pair])?,
    };
    let target = base + usize::from(word.a());
    fused_accumulate_step(
        Opcode::Add,
        stack,
        heap,
        globals,
        symbols,
        natives,
        target,
        element,
    )
}

/// Out-of-line executor for `AddStringRefCode`:
/// `rA = rA + (char->integer (string-ref rB rC))`.
/// See [`fused_add_vector_ref`] for the outlining rationale.
#[inline(never)]
pub(super) fn fused_add_string_ref_code(
    stack: &mut RegisterStack,
    heap: &mut Heap,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    word: Word,
    base: usize,
) -> Result<(), Error> {
    let string = read_register(stack, base + usize::from(word.b()));
    let index = read_register(stack, base + usize::from(word.c()));
    let element = match index.as_fixnum() {
        Some(index) if index >= 0 => heap.string_ref(string, index as usize),
        _ => None,
    };
    let element = match element {
        Some(character) => Value::integer(i64::from(character as u32)),
        None => {
            // Decomposed miss in source order: the `string-ref` native raises
            // its range/type error first; whatever it returns then flows
            // through `char->integer`.
            let character = register_operation_slow(
                Opcode::StringRef,
                heap,
                globals,
                symbols,
                natives,
                &[string, index],
            )?;
            match character.decode() {
                crate::value::ValueRepr::Character(character) => {
                    Value::integer(i64::from(character as u32))
                }
                _ => register_operation_slow(
                    Opcode::CharToInteger,
                    heap,
                    globals,
                    symbols,
                    natives,
                    &[character],
                )?,
            }
        }
    };
    let target = base + usize::from(word.a());
    fused_accumulate_step(
        Opcode::Add,
        stack,
        heap,
        globals,
        symbols,
        natives,
        target,
        element,
    )
}

/// One `vector-ref` element fetch over two register slots, with the standalone
/// arm's fast/slow split (a miss defers to the `vector-ref` native, raising
/// the identical error).
#[inline(always)]
fn fetch_vector_element(
    stack: &RegisterStack,
    heap: &mut Heap,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    vector_slot: usize,
    index_slot: usize,
) -> Result<Value, Error> {
    let vector = read_register(stack, vector_slot);
    let index = read_register(stack, index_slot);
    let element = match index.as_fixnum() {
        Some(index) if index >= 0 => heap.vector_ref(vector, index as usize),
        _ => None,
    };
    match element {
        Some(value) => Ok(value),
        None => register_operation_slow(
            Opcode::VectorRef,
            heap,
            globals,
            symbols,
            natives,
            &[vector, index],
        ),
    }
}

/// Out-of-line miss path for `AddMulVectorRef`:
/// `rA = rA + (vector-ref rB rC) * (vector-ref rB2 rC2)`, the second operand
/// pair packed as `(b2 << 8) | c2` in the consumed `ExtraArg` word.
/// The dispatch arm inlines the hit case and re-runs the whole word here on
/// any miss (the fetches are pure reads, so the re-run is unobservable). The
/// fetches, the product, and the accumulate run in unfused source order, each
/// deferring its miss to the same helper as the standalone words.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(super) fn fused_add_mul_vector_ref(
    stack: &mut RegisterStack,
    heap: &mut Heap,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    word: Word,
    extra: Word,
    base: usize,
) -> Result<(), Error> {
    let first = fetch_vector_element(
        stack,
        heap,
        globals,
        symbols,
        natives,
        base + usize::from(word.b()),
        base + usize::from(word.c()),
    )?;
    let packed = extra.ax_value();
    let second = fetch_vector_element(
        stack,
        heap,
        globals,
        symbols,
        natives,
        base + (packed >> 8) as usize,
        base + (packed & 0xFF) as usize,
    )?;
    // All-float chain (the float_dot shape): product and accumulate stay in
    // `f64` end to end, one raw rebox. Infallible for three floats, so error
    // identity is untouched; any other shape falls to the decomposed sequence.
    let target = base + usize::from(word.a());
    let accumulator = read_register(stack, target);
    if let (Some(l), Some(r), Some(acc)) =
        (first.as_float(), second.as_float(), accumulator.as_float())
    {
        write_register(stack, target, Value::float_raw(acc + l * r));
        return Ok(());
    }
    let product = match numeric_fast(Opcode::Multiply, first, second) {
        Some(value) => value,
        None => numeric_slow(
            Opcode::Multiply,
            heap,
            globals,
            symbols,
            natives,
            first,
            second,
        )?,
    };
    fused_accumulate_step(
        Opcode::Add,
        stack,
        heap,
        globals,
        symbols,
        natives,
        target,
        product,
    )
}

/// Out-of-line miss path for `VectorRefVectorRef`:
/// `rA = (vector-ref (vector-ref rB rC) rX)` with the outer index register X
/// in the consumed `ExtraArg` word. The dispatch arm inlines the hit case and
/// re-runs the whole word here on any miss (pure reads, so the re-run is
/// unobservable). Both fetches defer misses to the `vector-ref` native in
/// unfused order.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(super) fn fused_vector_ref_vector_ref(
    stack: &mut RegisterStack,
    heap: &mut Heap,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    word: Word,
    extra: Word,
    base: usize,
) -> Result<(), Error> {
    let inner = fetch_vector_element(
        stack,
        heap,
        globals,
        symbols,
        natives,
        base + usize::from(word.b()),
        base + usize::from(word.c()),
    )?;
    let index = read_register(stack, base + extra.ax_value() as usize);
    let element = match index.as_fixnum() {
        Some(index) if index >= 0 => heap.vector_ref(inner, index as usize),
        _ => None,
    };
    let value = match element {
        Some(value) => value,
        None => register_operation_slow(
            Opcode::VectorRef,
            heap,
            globals,
            symbols,
            natives,
            &[inner, index],
        )?,
    };
    write_register(stack, base + usize::from(word.a()), value);
    Ok(())
}

/// Out-of-line executor for `AddMul`/`SubMul`: `rA = rA ± (rB * RK(C))`.
/// See [`fused_add_vector_ref`] for the outlining rationale.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(super) fn fused_mul_step(
    opcode: Opcode,
    stack: &mut RegisterStack,
    heap: &mut Heap,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    constants: &[Value],
    word: Word,
    base: usize,
) -> Result<(), Error> {
    let left = read_register(stack, base + usize::from(word.b()));
    let right = if word.k() {
        read_constant(constants, word.c() as usize)
    } else {
        read_register(stack, base + usize::from(word.c()))
    };
    // All-float chain (the mandelbrot shape): keep the product and the
    // accumulate step in `f64` end to end - one raw rebox instead of a
    // product `Value` materialization plus a second two-operand
    // classification. Infallible for three floats, so error identity is
    // untouched; any other shape falls to the decomposed sequence below.
    let accumulator = read_register(stack, base + usize::from(word.a()));
    if let (Some(l), Some(r), Some(acc)) =
        (left.as_float(), right.as_float(), accumulator.as_float())
    {
        let result = if opcode == Opcode::AddMul {
            acc + l * r
        } else {
            acc - l * r
        };
        write_register(
            stack,
            base + usize::from(word.a()),
            Value::float_raw(result),
        );
        return Ok(());
    }
    let product = match numeric_fast(Opcode::Multiply, left, right) {
        Some(value) => value,
        None => numeric_slow(
            Opcode::Multiply,
            heap,
            globals,
            symbols,
            natives,
            left,
            right,
        )?,
    };
    let step = if opcode == Opcode::AddMul {
        Opcode::Add
    } else {
        Opcode::Subtract
    };
    let target = base + usize::from(word.a());
    fused_accumulate_step(
        step, stack, heap, globals, symbols, natives, target, product,
    )
}

#[inline]
pub(super) fn register_numeric(opcode: Opcode, left: Value, right: Value) -> Option<Value> {
    use crate::value::ValueRepr::{Fixnum, Float};

    // Monomorphic f64 fast path: when both operands are already inexact reals they are
    // stored inline in the tagged `Value`, so `as_float` is a single tag-compare each.
    if let (Some(left), Some(right)) = (left.as_float(), right.as_float()) {
        return f64_numeric(opcode, left, right);
    }

    match opcode {
        Opcode::Add | Opcode::Subtract | Opcode::Multiply => {
            match (left.decode(), right.decode()) {
                (Fixnum(left), Fixnum(right)) => {
                    let value = match opcode {
                        Opcode::Add => left.checked_add(right),
                        Opcode::Subtract => left.checked_sub(right),
                        Opcode::Multiply => left.checked_mul(right),
                        _ => unreachable!(),
                    }?;
                    // `None` only on real i64 overflow; the caller then folds to
                    // the heap-backed exact-integer path.
                    Some(Value::integer(value))
                }
                (Fixnum(left), Float(right)) => Some(Value::float_raw(float_arithmetic(
                    opcode,
                    left as f64,
                    right,
                ))),
                (Float(left), Fixnum(right)) => Some(Value::float_raw(float_arithmetic(
                    opcode,
                    left,
                    right as f64,
                ))),
                (Float(left), Float(right)) => {
                    Some(Value::float_raw(float_arithmetic(opcode, left, right)))
                }
                _ => None,
            }
        }
        Opcode::Divide => match (left.decode(), right.decode()) {
            (Fixnum(left), Fixnum(right)) if right != 0 && left % right == 0 => {
                left.checked_div(right).map(Value::integer)
            }
            (Fixnum(left), Float(right)) => Some(Value::float_raw(left as f64 / right)),
            (Float(left), Fixnum(right)) => Some(Value::float_raw(left / right as f64)),
            (Float(left), Float(right)) => Some(Value::float_raw(left / right)),
            _ => None,
        },
        Opcode::NumericEqual
        | Opcode::NumericLess
        | Opcode::NumericGreater
        | Opcode::NumericLessEqual
        | Opcode::NumericGreaterEqual => {
            let ordering = match (left.decode(), right.decode()) {
                (Fixnum(left), Fixnum(right)) => left.cmp(&right),
                (Fixnum(left), Float(right)) if left.unsigned_abs() <= (1_u64 << 53) => {
                    (left as f64).partial_cmp(&right)?
                }
                (Float(left), Fixnum(right)) if right.unsigned_abs() <= (1_u64 << 53) => {
                    left.partial_cmp(&(right as f64))?
                }
                (Float(left), Float(right)) => left.partial_cmp(&right)?,
                _ => return None,
            };
            Some(Value::boolean(match opcode {
                Opcode::NumericEqual => ordering.is_eq(),
                Opcode::NumericLess => ordering.is_lt(),
                Opcode::NumericGreater => ordering.is_gt(),
                Opcode::NumericLessEqual => ordering.is_le(),
                Opcode::NumericGreaterEqual => ordering.is_ge(),
                _ => unreachable!(),
            }))
        }
        _ => None,
    }
}

/// Both operands are inline `f64`s. Mirrors the `(Float, Float)` arms of
/// [`register_numeric`] exactly: arithmetic yields an inline float, comparisons
/// a boolean, and a NaN comparison (`partial_cmp` -> `None`) falls through to
/// `None` so the caller defers to the general numeric native.
#[inline(always)]
fn f64_numeric(opcode: Opcode, left: f64, right: f64) -> Option<Value> {
    match opcode {
        // Raw (non-canonicalizing) reboxes: NaN normalization happens at the
        // observation sites, not per arithmetic result.
        Opcode::Add => Some(Value::float_raw(left + right)),
        Opcode::Subtract => Some(Value::float_raw(left - right)),
        Opcode::Multiply => Some(Value::float_raw(left * right)),
        Opcode::Divide => Some(Value::float_raw(left / right)),
        Opcode::NumericEqual
        | Opcode::NumericLess
        | Opcode::NumericGreater
        | Opcode::NumericLessEqual
        | Opcode::NumericGreaterEqual => {
            let ordering = left.partial_cmp(&right)?;
            Some(Value::boolean(match opcode {
                Opcode::NumericEqual => ordering.is_eq(),
                Opcode::NumericLess => ordering.is_lt(),
                Opcode::NumericGreater => ordering.is_gt(),
                Opcode::NumericLessEqual => ordering.is_le(),
                Opcode::NumericGreaterEqual => ordering.is_ge(),
                _ => unreachable!(),
            }))
        }
        _ => None,
    }
}

#[inline(always)]
pub(super) fn float_arithmetic(opcode: Opcode, left: f64, right: f64) -> f64 {
    match opcode {
        Opcode::Add => left + right,
        Opcode::Subtract => left - right,
        Opcode::Multiply => left * right,
        _ => unreachable!(),
    }
}

pub(super) fn invoke_register_operation(
    opcode: Opcode,
    heap: &mut Heap,
    globals: &crate::global::GlobalStore,
    symbols: &mut HashMap<String, Value>,
    natives: &crate::native::NativeRegistry,
    arguments: &[Value],
) -> Result<Results, Error> {
    let procedure = match opcode {
        Opcode::Add => crate::native::FastProcedure::Add,
        Opcode::Subtract => crate::native::FastProcedure::Subtract,
        Opcode::Multiply => crate::native::FastProcedure::Multiply,
        Opcode::Divide => crate::native::FastProcedure::Divide,
        Opcode::NumericEqual => crate::native::FastProcedure::Equal,
        Opcode::NumericLess => crate::native::FastProcedure::Less,
        Opcode::NumericGreater => crate::native::FastProcedure::Greater,
        Opcode::NumericLessEqual => crate::native::FastProcedure::LessEqual,
        Opcode::NumericGreaterEqual => crate::native::FastProcedure::GreaterEqual,
        Opcode::VectorRef => crate::native::FastProcedure::VectorRef,
        Opcode::VectorSet => crate::native::FastProcedure::VectorSet,
        Opcode::Car => crate::native::FastProcedure::Car,
        Opcode::Cdr => crate::native::FastProcedure::Cdr,
        Opcode::StringRef => crate::native::FastProcedure::StringRef,
        Opcode::StringLength => crate::native::FastProcedure::StringLength,
        Opcode::CharToInteger => crate::native::FastProcedure::CharToInteger,
        _ => return Err(bad("register operation")),
    };
    let mut fast = Value::unspecified();
    if procedure.invoke(heap, arguments, &mut fast) {
        return Ok(Results::One(fast));
    }
    let name = format!("\u{1f}library:(scheme base):{}", procedure.name());
    let value = globals
        .get(&name)
        .copied()
        .ok_or_else(|| bad("numeric fallback"))?;
    let id = heap.native(value).ok_or_else(|| bad("numeric fallback"))?;
    // No register view is available here, so `None`. Sound because this path
    // runs under VM-managed rooting (no rooted region is entered): any `alloc`
    // inside the built-in defers collection to the next safe point rather than
    // collecting mid-call.
    Ok(natives
        .invoke(id, heap, symbols, globals, arguments, None)?
        .into_results())
}

pub(super) fn execute_cold(
    instruction: ColdInstruction,
    _config: &crate::EngineConfig,
) -> Result<(), Error> {
    match instruction {
        ColdInstruction::ValueCountError { expected, actual } => Err(Error::plain(
            ErrorKind::RuntimeError,
            format!("expected exactly {expected} value, received {actual}"),
        )),
        _ => Err(Error::plain(
            ErrorKind::ImplementationRestriction,
            "cold register opcode is not connected yet",
        )),
    }
}
