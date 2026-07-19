use r7rs::{Engine, EngineConfig, ErrorKind, InterruptToken, Limits, Value};

fn evaluate(source: &str) -> Value {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine.compile("optimization.scm", source).unwrap();
    engine.eval(&module).unwrap().into_one().unwrap().value()
}

#[test]
fn linked_global_slots_preserve_mutation_and_heap_values() {
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile(
            "global-slots.scm",
            "(begin
               (define retained (vector 1))
               (define counter 0)
               (let loop ((n 100))
                 (if (= n 0)
                     (begin (vector-set! retained 0 counter)
                            (vector-ref retained 0))
                     (begin (set! counter (+ counter 1))
                            (cons n n)
                            (loop (- n 1))))))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(100)
    );
}

#[test]
fn vector_set_fast_path_stores_and_defers_out_of_range_to_the_error_path() {
    // The `VectorSet` opcode takes an inline `heap.vector_set` fast path and only
    // falls back to the generic native when that refuses. A hit must store and
    // yield the written value; a miss (out-of-range or negative index) must defer
    // to the native so the same `RangeError` is still raised.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((v (make-vector 3 0)))
               (vector-set! v 2 42)
               (vector-ref v 2))",
        ),
        Value::integer(42)
    );

    for index in ["5", "-1"] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine
            .compile(
                "vector-set-oob.scm",
                format!("(import (scheme base)) (vector-set! (make-vector 3 0) {index} 1)"),
            )
            .unwrap();
        assert_eq!(
            engine.eval(&module).unwrap_err().kind(),
            ErrorKind::RangeError,
            "index {index} should raise RangeError via the fallback path",
        );
    }
}

#[test]
fn pure_primitive_fast_paths_store_and_defer_errors_to_the_native() {
    // cons/car/cdr/null?/pair?/string-ref/char->integer/string-length compile
    // to dedicated opcodes whose arms take an inline fast path that skips the
    // arity check, root scope, and panic guard. A hit must match the generic
    // semantics exactly. A miss (wrong type / out-of-range index) must defer to
    // the native so the same error is still raised.

    // cons + car + cdr round-trip, including nested pairs and the empty list.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((p (cons 1 (cons 2 '()))))
               (+ (car p) (car (cdr p))))",
        ),
        Value::integer(3)
    );

    // null? / pair? classification.
    assert_eq!(
        evaluate("(import (scheme base)) (null? '())"),
        Value::boolean(true)
    );
    assert_eq!(
        evaluate("(import (scheme base)) (null? (cons 1 2))"),
        Value::boolean(false)
    );
    assert_eq!(
        evaluate("(import (scheme base)) (null? 0)"),
        Value::boolean(false)
    );
    assert_eq!(
        evaluate("(import (scheme base)) (pair? (cons 1 2))"),
        Value::boolean(true)
    );
    assert_eq!(
        evaluate("(import (scheme base)) (pair? '())"),
        Value::boolean(false)
    );
    assert_eq!(
        evaluate("(import (scheme base)) (pair? 5)"),
        Value::boolean(false)
    );

    // string-ref / char->integer / string-length on the hit path.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((s (make-string 3 #\\z)))
               (+ (char->integer (string-ref s 0)) (string-length s)))",
        ),
        Value::integer(122 + 3) // #\z is code point 122.
    );

    // car / cdr of a non-pair defer to the native's TypeError.
    for source in ["(car 5)", "(cdr 5)"] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine
            .compile("non-pair.scm", format!("(import (scheme base)) {source}"))
            .unwrap();
        assert_eq!(
            engine.eval(&module).unwrap_err().kind(),
            ErrorKind::TypeError,
            "{source} should raise TypeError via the fallback path",
        );
    }

    // string-ref out-of-range defers to the native's RangeError.
    for index in ["5", "-1"] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine
            .compile(
                "string-ref-oob.scm",
                format!("(import (scheme base)) (string-ref (make-string 3 #\\a) {index})"),
            )
            .unwrap();
        assert_eq!(
            engine.eval(&module).unwrap_err().kind(),
            ErrorKind::RangeError,
            "index {index} should raise RangeError via the fallback path",
        );
    }

    // Wrong argument types defer so the native still raises a TypeError.
    for source in ["(string-ref 5 0)", "(char->integer 5)", "(string-length 5)"] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine
            .compile("wrong-type.scm", format!("(import (scheme base)) {source}"))
            .unwrap();
        assert_eq!(
            engine.eval(&module).unwrap_err().kind(),
            ErrorKind::TypeError,
            "{source} should raise TypeError via the fallback path",
        );
    }
}

#[test]
fn loop_back_counter_fusion_matches_generic_semantics() {
    // A flattened counting loop steps its counter with a fused `LoopBack`
    // (increment + back-edge) while accumulators keep the generic scratch+move
    // path. Each shape below must match its unfused meaning exactly.

    // Ascending +1 counter with an accumulator (rows/nested shape).
    assert_eq!(
        evaluate(
            "(let loop ((i 0) (acc 0))
               (if (= i 1000) acc (loop (+ i 1) (+ acc i))))",
        ),
        Value::integer(499500)
    );

    // Descending -1 counter (native/`(= r 0)` shape).
    assert_eq!(
        evaluate(
            "(let loop ((r 5) (v 0))
               (if (= r 0) v (loop (- r 1) (+ v r))))",
        ),
        Value::integer(15)
    );

    // Step != 1, counter written as `(+ K i)` (literal on the left).
    assert_eq!(
        evaluate(
            "(let loop ((i 0) (acc 0))
               (if (= i 10) acc (loop (+ 2 i) (+ acc i))))",
        ),
        Value::integer(20) // 0+2+4+6+8
    );

    // Single-counter loop with a non-comparison-of-the-counter test still fuses
    // the step (the test stays at the header, untouched by `LoopBack`).
    assert_eq!(
        evaluate("(let loop ((i 0)) (if (= (* i i) 25) i (loop (+ i 1))))"),
        Value::integer(5)
    );

    // Accumulators that swap must still read their old values (the counter is
    // stepped last), so the two-phase scratch+move ordering is preserved.
    assert_eq!(
        evaluate(
            "(let loop ((a 1) (b 2) (n 0))
               (if (= n 3) (+ a b) (loop b a (+ n 1))))",
        ),
        Value::integer(3)
    );

    // A variable step (`(+ m s)`, not a literal) does NOT fuse and must still run
    // correctly through the generic back-edge.
    assert_eq!(
        evaluate(
            "(let loop ((m 0) (s 5) (n 0))
               (if (= n 2) m (loop (+ m s) s (+ n 1))))",
        ),
        Value::integer(10)
    );

    // Counter crossing the inline i64 range mid-loop must become a heap-backed
    // exact integer, exactly as `(+ i 1)` would (no wrap). Start at FIXNUM_MAX
    // (i64::MAX) so the very first increment overflows i64 and promotes to a
    // heap-backed i128, then subtract the base back to a small fixnum.
    assert_eq!(
        evaluate(
            "(let loop ((i 9223372036854775807) (n 0))
               (if (= n 3) (- i 9223372036854775807) (loop (+ i 1) (+ n 1))))",
        ),
        Value::integer(3)
    );
}

#[test]
fn loop_parallel_assignment_direct_writes_preserve_old_reads() {
    // A flattened loop's tail call assigns its parameters in parallel. An
    // accumulator whose register is read by no other argument is written straight
    // into its home register (no scratch + `Move`). Any parameter that another
    // argument reads must still stay on the scratch path so it is read with its
    // OLD value. These shapes exercise both branches of that decision.

    // Independent accumulator (only reads itself) and the direct-write case.
    assert_eq!(
        evaluate(
            "(let loop ((i 0) (acc 1))
               (if (= i 5) acc (loop (+ i 1) (* acc 2))))",
        ),
        Value::integer(32),
    );

    // The directly-written accumulator reads ANOTHER parameter (`lst`) whose own
    // update overwrites it: `acc` must observe the pre-step `lst`, so `lst` stays
    // on the scratch path even though `acc` is written in place.
    assert_eq!(
        evaluate(
            "(let loop ((lst '(1 2 3 4 5)) (acc 0))
               (if (null? lst) acc (loop (cdr lst) (+ acc (car lst)))))",
        ),
        Value::integer(15),
    );

    // Every parameter reads another (a rotation/dependency cycle). One cyclic
    // accumulator, the one evaluated last, is written straight into its home
    // once the others are already in scratch. The rest keep their scratch slot.
    // Fibonacci-style `(loop b (+ a b) ...)`: new `a` is old `b`, new `b` is old
    // `a` + old `b`. The in-place write must still read the pre-step values.
    assert_eq!(
        evaluate(
            "(let loop ((a 0) (b 1) (n 0))
               (if (= n 10) a (loop b (+ a b) (+ n 1))))",
        ),
        Value::integer(55),
    );

    // A pure swap `(loop b a ...)` is the tightest 2-cycle (the mandelbrot
    // `zr`/`zi` shape). `b` is written in place reading old `a`; `a` moves from
    // scratch holding old `b`. Three swaps of `(1 . 2)` leave `(2 . 1)`.
    assert_eq!(
        evaluate(
            "(let loop ((a 1) (b 2) (n 0))
               (if (= n 3) (- a b) (loop b a (+ n 1))))",
        ),
        Value::integer(1),
    );

    // A no-counter loop whose sole parameter updates from a non-literal step
    // (`(+ p q)`, `q` a captured constant) is written in place and must still read
    // its own old value.
    assert_eq!(
        evaluate(
            "(let ((step 3))
               (let loop ((p 0) (k 0))
                 (if (= k 4) p (loop (+ p step) (+ k 1)))))",
        ),
        Value::integer(12),
    );
}

#[test]
fn if_with_empty_alternate_drops_dead_skip_jump() {
    // When the alternate of an `if` compiles to zero instructions (here the else
    // branch `count` is a non-boxed local already in its home register, so the
    // read is a no-op), the skip jump emitted after the consequent would target
    // the immediately-following instruction: a dead jump. It is dropped and the
    // false branch lands on the fall-through. Both branches must still yield the
    // correct value.
    assert_eq!(
        evaluate(
            "(let loop ((n 0) (count 0))
               (if (< n 10) (loop (+ n 1) (+ count 1)) count))",
        ),
        Value::integer(10),
    );
    // Taking the else branch on the very first iteration (consequent never runs)
    // must return the accumulator's initial value through the fall-through.
    assert_eq!(
        evaluate(
            "(let loop ((n 10) (count 42))
               (if (< n 10) (loop (+ n 1) (+ count 1)) count))",
        ),
        Value::integer(42),
    );
}

#[test]
fn capture_access_reads_the_live_frame_after_lazy_hoisting() {
    // The dispatch loop no longer clones the frame's `captures` `Rc` per
    // activation; the capture opcodes read it straight from the top frame. These
    // shapes exercise each of those opcodes and must keep exact closure semantics.

    // GetCapture + SetCapture: a counter closure mutating a captured (boxed) local.
    assert_eq!(
        evaluate(
            "(let ((count 0))
               (let ((inc (lambda () (set! count (+ count 1)) count)))
                 (inc)
                 (inc)
                 (inc)))",
        ),
        Value::integer(3),
    );

    // Closure opcode's capture-of-capture path: the innermost lambda captures `a`
    // and `b`, which are themselves captures in the intermediate frames.
    assert_eq!(
        evaluate("((((lambda (a) (lambda (b) (lambda (c) (+ a b c)))) 100) 20) 3)"),
        Value::integer(123),
    );

    // Two independent counters must not share a cell (distinct capture bindings),
    // and their reads must track each frame's own captures.
    assert_eq!(
        evaluate(
            "(letrec ((make (lambda ()
                              (let ((n 0)) (lambda () (set! n (+ n 1)) n)))))
               (let ((a (make)) (b (make)))
                 (a) (a) (b)
                 (+ (* 10 (a)) (b))))",
        ),
        Value::integer(32), // a called 3× -> 3, b called 2× -> 2: 10*3 + 2
    );
}

#[test]
fn n_ary_arithmetic_folds_left_into_two_argument_opcodes() {
    // Exact integer folds.
    assert_eq!(evaluate("(+ 1 2 3 4)"), Value::integer(10));
    assert_eq!(evaluate("(* 2 3 4)"), Value::integer(24));
    // Non-commutative operators must fold strictly left-to-right.
    assert_eq!(evaluate("(- 3 4 5)"), Value::integer(-6));
    assert_eq!(evaluate("(/ 24 2 3)"), Value::integer(4));
    // Inexact, including the mandelbrot three-argument multiply shape.
    assert_eq!(evaluate("(* 2.0 3.0 4.0)"), Value::float(24.0));
    assert_eq!(
        evaluate("(let ((zr 1.5) (zi 2.0)) (* 2.0 zr zi))"),
        Value::float(6.0)
    );
    // Exact/inexact contagion survives the fold.
    assert_eq!(evaluate("(+ 1 2.0 3)"), Value::float(6.0));
    // Nested folds compose, and the 0/1-argument forms stay on the generic path.
    assert_eq!(evaluate("(+ (* 2 3 4) (- 10 1 2))"), Value::integer(31));
    assert_eq!(evaluate("(*)"), Value::integer(1));
    assert_eq!(evaluate("(- 5)"), Value::integer(-5));
}

#[test]
fn direct_local_promotes_to_shared_cell_when_captured() {
    assert_eq!(
        evaluate(
            "(let ((x 1))
               (let ((read (lambda () x))
                     (write (lambda (value) (set! x value))))
                 (write 42)
                 (read)))",
        ),
        Value::integer(42)
    );
}

#[test]
fn continuation_snapshot_shares_uncaptured_mutable_locals() {
    assert_eq!(
        evaluate(
            "(let ((saved #f) (count 0))
               (+ (call/cc (lambda (k) (set! saved k) 1))
                  (if (< count 2)
                      (begin (set! count (+ count 1)) (saved 10))
                      0)))",
        ),
        Value::integer(10)
    );
}

#[test]
fn inline_packets_preserve_zero_one_and_multiple_values() {
    assert_eq!(
        evaluate(
            "(+ (call-with-values (lambda () (values)) (lambda () 10))
                (call-with-values (lambda () 2) (lambda (x) x))
                (call-with-values (lambda () (values 3 4)) +))",
        ),
        Value::integer(19)
    );
}

#[test]
fn recursive_call_specialization_respects_later_mutation() {
    assert_eq!(
        evaluate(
            "(letrec ((f (lambda (n)
                         (if (= n 0)
                             0
                             (begin
                               (set! f (lambda (ignored) 99))
                               (f (- n 1)))))))
               (f 1))",
        ),
        Value::integer(99)
    );
}

#[test]
fn batched_polling_observes_an_existing_interrupt() {
    let token = InterruptToken::new();
    let mut engine =
        Engine::new(EngineConfig::default().with_interrupt_token(token.clone())).unwrap();
    let module = engine
        .compile("interrupt.scm", "(let loop () (loop))")
        .unwrap();
    token.interrupt();
    assert_eq!(
        engine.eval(&module).unwrap_err().kind(),
        ErrorKind::ExecutionLimitExceeded
    );
}

#[test]
fn instruction_fuel_terminates_a_runaway_loop() {
    // Safe points are batched to back-edges and calls, so fuel is charged in bulk
    // at each safe point rather than per instruction. A runaway loop must still
    // exhaust the budget and raise `ExecutionLimitExceeded`.
    let limits = Limits::default().with_fuel(Some(10_000));
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile("fuel.scm", "(let loop ((n 0)) (loop (+ n 1)))")
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap_err().kind(),
        ErrorKind::ExecutionLimitExceeded
    );
}

#[test]
fn instruction_fuel_permits_a_bounded_computation() {
    // The batched fuel accounting must not spuriously kill a program that stays
    // within its budget; a short loop with ample fuel runs to completion.
    let limits = Limits::default().with_fuel(Some(1_000_000));
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile(
            "fuel-ok.scm",
            "(let loop ((n 0) (acc 0)) (if (= n 100) acc (loop (+ n 1) (+ acc n))))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(4950)
    );
}

#[test]
fn immediately_applied_lambdas_inline_without_changing_semantics() {
    // `or`/`and`/`when`/`cond` desugar to `((lambda ...) ...)`, which the
    // compiler now inlines into the enclosing frame. Results and short-circuit
    // behaviour must be identical to the closure path.
    assert_eq!(evaluate("(or #f 5)"), Value::integer(5));
    assert_eq!(evaluate("(or 3 (error \"unreached\"))"), Value::integer(3));
    assert_eq!(evaluate("(and 1 2 3)"), Value::integer(3));
    assert_eq!(
        evaluate("(and 1 #f (error \"unreached\"))"),
        Value::boolean(false)
    );
    assert_eq!(evaluate("(when #t 1 2 3)"), Value::integer(3));
    assert_eq!(
        evaluate("(cond (#f 1) (7 => (lambda (x) (* x 10))) (else 99))"),
        Value::integer(70)
    );
}

#[test]
fn inlined_let_preserves_parallel_binding_and_ordering() {
    // A plain `let` binds in parallel: the inner initializers see the OUTER
    // binding, so `(* x 2)` and `(+ x 1)` both read `x = 5`.
    assert_eq!(
        evaluate("(let ((x 5)) (let ((x (* x 2)) (y (+ x 1))) (- x y)))"),
        Value::integer(4)
    );
    // `let*` is sequential: the second initializer sees the first inner binding.
    assert_eq!(
        evaluate("(let ((x 5)) (let* ((x (* x 2)) (y (+ x 1))) (- x y)))"),
        Value::integer(-1)
    );
    // Arguments evaluate left to right before the body runs.
    assert_eq!(
        evaluate(
            "(let ((n 0))
               (define (bump) (set! n (+ n 1)) n)
               (let ((a (bump)) (b (bump)) (c (bump)))
                 (+ (* a 100) (* b 10) c)))"
        ),
        Value::integer(123)
    );
}

#[test]
fn inlined_binding_hot_loop_matches_the_closure_path() {
    // `let`/`and`/`or` inside a tight loop (the mandelbrot allocation shape).
    // Sum over 0..999 of `2*i` when `i` is even, else 0 = 4 * (0+1+...+499).
    assert_eq!(
        evaluate(
            "(let loop ((i 0) (acc 0))
               (if (= i 1000)
                   acc
                   (loop (+ i 1)
                         (+ acc (let ((t (* i 2)))
                                  (or (and (even? i) t) 0))))))"
        ),
        Value::integer(499_000)
    );
}

#[test]
fn parameterize_body_let_boxes_captured_frame_locals() {
    // The inner `let` sits in a `parameterize` body, which the compiler lowers
    // in `Mode::All`; it is therefore NOT inlined but becomes a closure that
    // captures `x`. The boxing pre-pass must box `x` accordingly, or the
    // capture would read an unboxed slot as if it were a box cell.
    assert_eq!(
        evaluate(
            "(begin
               (define p (make-parameter 10))
               (define (f x)
                 (parameterize ((p 1))
                   (let ((y (+ x 100)))
                     (+ y x (p)))))
               (f 5))"
        ),
        Value::integer(111)
    );
    // The same position with a multi-value body exercises the `Mode::All`
    // fallback (never flattened into the parent frame, so no register overlap).
    assert_eq!(
        evaluate(
            "(begin
               (define q (make-parameter 0))
               (call-with-values
                 (lambda ()
                   (parameterize ((q 1))
                     (let ((a 10) (b 20)) (values a b (q)))))
                 (lambda (a b c) (+ (* a 100) (* b 10) c))))"
        ),
        Value::integer(1201)
    );
}

#[test]
fn mutated_or_captured_let_binding_stays_correct() {
    // A `set!`-mutated binding must not be inlined as a bare register.
    assert_eq!(
        evaluate("(let ((x 1)) (let ((y 10)) (set! y (+ y 5)) (+ x y)))"),
        Value::integer(16)
    );
    // A captured, mutated binding keeps an independent shared cell per instance.
    assert_eq!(
        evaluate(
            "(let ((make (lambda ()
                           (let ((c 0))
                             (lambda () (set! c (+ c 1)) c)))))
               (let ((f (make)) (g (make)))
                 (f) (f)
                 (+ (* (f) 10) (g))))"
        ),
        Value::integer(31)
    );
}

#[test]
fn deeply_nested_inlined_let_star_compiles_and_evaluates() {
    let mut source = String::from("(let* (");
    for index in 0..50 {
        if index == 0 {
            source.push_str("(x0 0)");
        } else {
            source.push_str(&format!("(x{index} (+ x{} 1))", index - 1));
        }
    }
    source.push_str(") x49)");
    assert_eq!(evaluate(&source), Value::integer(49));
}

#[test]
fn inlining_beyond_the_register_budget_is_a_clean_compile_error() {
    // Inlining a `let` binds its parameters as locals of the enclosing frame.
    // A `let` with more bindings than the 255-register budget must surface as a
    // structured compile error, never a panic or an unsound fallback to the
    // closure path (`scan_boxed` already decided not to box these locals).
    let mut source = String::from("(let (");
    for index in 0..300 {
        source.push_str(&format!("(x{index} {index}) "));
    }
    source.push_str(") x0)");
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        engine.compile("wide.scm", &source).unwrap_err().kind(),
        ErrorKind::CompileError
    );
}

#[test]
fn fused_comparisons_branch_correctly_across_operators() {
    // Each comparison directly testing an `if` fuses into a `Test` + `Jump`
    // (1 = consequent taken, 0 = alternate taken).
    assert_eq!(evaluate("(if (< 1 2) 1 0)"), Value::integer(1));
    assert_eq!(evaluate("(if (< 2 1) 1 0)"), Value::integer(0));
    assert_eq!(evaluate("(if (<= 2 2) 1 0)"), Value::integer(1));
    assert_eq!(evaluate("(if (<= 3 2) 1 0)"), Value::integer(0));
    assert_eq!(evaluate("(if (= 2 2) 1 0)"), Value::integer(1));
    assert_eq!(evaluate("(if (= 2 3) 1 0)"), Value::integer(0));
    // `>`/`>=` with a literal right operand take TestGreater/TestGreaterEqual,
    // with a register right operand they swap onto TestLess/TestLessEqual.
    assert_eq!(evaluate("(if (> 3 2) 1 0)"), Value::integer(1));
    assert_eq!(evaluate("(if (> 2 3) 1 0)"), Value::integer(0));
    assert_eq!(evaluate("(if (>= 2 2) 1 0)"), Value::integer(1));
    assert_eq!(evaluate("(if (>= 1 2) 1 0)"), Value::integer(0));
    // Immediate (RK) right operand.
    assert_eq!(
        evaluate("(let ((n 5)) (if (< n 40) 1 0))"),
        Value::integer(1)
    );
    assert_eq!(
        evaluate("(let ((n 99)) (if (< n 40) 1 0))"),
        Value::integer(0)
    );
}

#[test]
fn fused_comparisons_match_ieee_and_swap_semantics_on_nan() {
    // NaN makes every ordering false; the operand swap for `>`/`>=` must agree.
    assert_eq!(evaluate("(if (> +nan.0 1.0) 1 0)"), Value::integer(0));
    assert_eq!(evaluate("(if (< +nan.0 1.0) 1 0)"), Value::integer(0));
    assert_eq!(evaluate("(if (>= 1.0 +nan.0) 1 0)"), Value::integer(0));
    assert_eq!(evaluate("(if (= +nan.0 +nan.0) 1 0)"), Value::integer(0));
    // Float/fixnum mixed operands through the fused path.
    assert_eq!(evaluate("(if (< 3 3.5) 1 0)"), Value::integer(1));
    assert_eq!(evaluate("(if (> 3.5 3) 1 0)"), Value::integer(1));
}

#[test]
fn fused_comparisons_match_the_exact_generic_path_beyond_inline_range() {
    // Operands outside the inline fixnum/float comparison fast path fall back to
    // the generic numeric comparison; the `>`/`>=` swap must still agree with
    // the non-swapped `<`/`<=`.
    assert_eq!(
        evaluate("(if (> 10000000000000000000 1) 1 0)"),
        Value::integer(1)
    );
    assert_eq!(
        evaluate("(if (< 1 10000000000000000000) 1 0)"),
        Value::integer(1)
    );
    assert_eq!(evaluate("(if (>= 1/2 1/3) 1 0)"), Value::integer(1));
    assert_eq!(evaluate("(if (< 1/3 1/2) 1 0)"), Value::integer(1));
}

#[test]
fn boolean_context_lowering_reaches_comparisons_under_and_or_not() {
    // The mandelbrot inner-test shape: `or` of two comparisons.
    assert_eq!(
        evaluate("(let ((i 5) (n 40) (x 9.0)) (if (or (= i n) (> x 4.0)) 1 0))"),
        Value::integer(1)
    );
    assert_eq!(
        evaluate("(let ((i 5) (n 40) (x 1.0)) (if (or (= i n) (> x 4.0)) 1 0))"),
        Value::integer(0)
    );
    // `and` short-circuits down to leaf comparisons.
    assert_eq!(
        evaluate("(if (and (< 1 2) (< 2 3)) 1 0)"),
        Value::integer(1)
    );
    assert_eq!(
        evaluate("(if (and (< 1 2) (< 3 2)) 1 0)"),
        Value::integer(0)
    );
    // `not` inverts polarity.
    assert_eq!(evaluate("(if (not (< 1 2)) 1 0)"), Value::integer(0));
    assert_eq!(evaluate("(if (not (< 2 1)) 1 0)"), Value::integer(1));
    // Nested and/or.
    assert_eq!(
        evaluate("(if (or (and (< 1 2) (< 3 2)) (= 7 7)) 1 0)"),
        Value::integer(1)
    );
    assert_eq!(
        evaluate("(if (and (or (< 3 2) (= 1 1)) (> 5 4)) 1 0)"),
        Value::integer(1)
    );
    // Short-circuit must not evaluate the second operand when the first decides.
    assert_eq!(
        evaluate("(let ((hits 0)) (if (or (< 1 2) (begin (set! hits (+ hits 1)) #t)) hits 99))"),
        Value::integer(0)
    );
}

#[test]
fn comparisons_used_as_values_stay_on_the_unfused_path() {
    // A comparison whose boolean result is used (not a branch test) must still
    // materialize an ordinary boolean.
    assert_eq!(evaluate("(let ((p (< 1 2))) p)"), Value::boolean(true));
    assert_eq!(evaluate("(let ((p (> 1 2))) p)"), Value::boolean(false));
    assert_eq!(
        evaluate("(list (< 1 2) (> 1 2) (= 3 3))"),
        evaluate("(list #t #f #t)")
    );
    // `and`/`or` returning their operand value (not in test position).
    assert_eq!(evaluate("(or #f 7)"), Value::integer(7));
    assert_eq!(evaluate("(and 1 2 3)"), Value::integer(3));
}

#[test]
fn fused_condition_preserves_tail_and_nested_branches() {
    // Fusion in tail position (recursive countdown) and nested ifs.
    assert_eq!(
        evaluate("(let loop ((n 1000) (a 0)) (if (= n 0) a (loop (- n 1) (+ a 1))))"),
        Value::integer(1000)
    );
    assert_eq!(
        evaluate("(let ((x 5)) (if (< x 3) 1 (if (< x 7) 2 3)))"),
        Value::integer(2)
    );
}

#[test]
fn flattened_named_let_loops_compute_correctly() {
    // Simple counting loop (self-tail-recursive, non-escaping -> flattened).
    assert_eq!(
        evaluate("(let loop ((i 0) (a 0)) (if (= i 100) a (loop (+ i 1) (+ a i))))"),
        Value::integer(4950)
    );
    // Nested loops that capture outer loop variables (the mandelbrot shape:
    // `bs`, `cs`, `i` become plain registers, not boxed captures).
    assert_eq!(
        evaluate(
            "(let a ((i 0) (acc 0))
               (if (= i 20) acc
                 (a (+ i 1)
                    (let b ((j 0) (bs 0))
                      (if (= j 20) (+ acc bs)
                        (b (+ j 1)
                           (let c ((k 0) (cs 0))
                             (if (= k 25) (+ bs cs)
                               (let ((tmp (+ cs i))) (c (+ k 1) tmp))))))))))"
        ),
        Value::integer(95000)
    );
}

#[test]
fn flattened_loop_tail_call_uses_parallel_assignment() {
    // New parameter values read the *old* parameters; the evaluate-into-scratch-
    // then-move sequence must not clobber operands mid-update. Fibonacci swaps.
    assert_eq!(
        evaluate("(let loop ((a 0) (b 1) (n 20)) (if (= n 0) a (loop b (+ a b) (- n 1))))"),
        Value::integer(6765)
    );
    // A three-way rotation stresses aliasing across all parameter registers.
    assert_eq!(
        evaluate(
            "(let loop ((x 1) (y 2) (z 3) (n 3)) (if (= n 0) (+ (* x 100) (* y 10) z) (loop y z x (- n 1))))"
        ),
        Value::integer(123)
    );
}

#[test]
fn non_tail_self_call_falls_back_to_closure() {
    // The self-call is an operand of `+`, not a tail call -> not flattenable, but
    // must still evaluate correctly via the closure lowering.
    assert_eq!(
        evaluate("(let loop ((i 0)) (if (= i 5) 0 (+ 1 (loop (+ i 1)))))"),
        Value::integer(5)
    );
}

#[test]
fn self_recursive_calls_compute_across_frame_shapes() {
    // Non-tail self-recursion (fib): every call and return stays within one
    // chunk, exercising the in-loop self-call and same-chunk-return fast paths.
    assert_eq!(
        evaluate("(let fib ((n 15)) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))"),
        Value::integer(610)
    );
    // Rest-arg self-recursion allocates a rest list per call, so it must take
    // the generic (fallible) call path, not the in-loop fast path.
    assert_eq!(
        evaluate(
            "(begin
               (define (sum-all acc . rest)
                 (if (null? rest) acc (sum-all (+ acc (car rest)))))
               (sum-all 0 1))"
        ),
        Value::integer(1)
    );
    // Deep non-tail self-recursion grows the frame arena and register file far
    // past their initial sizes while staying inside the inner dispatch loop.
    assert_eq!(
        evaluate("(let deep ((n 20000)) (if (= n 0) 0 (+ 1 (deep (- n 1)))))"),
        Value::integer(20000)
    );
}

#[test]
fn continuations_and_handlers_reach_across_self_recursive_frames() {
    // A continuation captured deep inside self-recursion escapes, is re-entered
    // after the recursion returned, and must resume the (snapshotted) frames
    // that the in-loop self-call fast path pushed.
    assert_eq!(
        evaluate(
            "(let ((k #f))
               (let ((first (+ 100 (let rec ((n 3))
                                     (if (= n 0)
                                         (call/cc (lambda (c) (set! k c) 0))
                                         (+ 1 (rec (- n 1))))))))
                 (if (< first 200) (k 100) first)))"
        ),
        Value::integer(203)
    );
    // An error raised deep inside self-recursion unwinds through frames pushed
    // by the fast path and reaches the installed handler with its payload.
    assert_eq!(
        evaluate(
            "(guard (e (#t (string=? (error-object-message e) \"bottom\")))
               (let rec ((n 5))
                 (if (= n 0) (error \"bottom\") (+ 1 (rec (- n 1))))))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn gc_pressure_during_self_recursion_preserves_frames() {
    // With a collection threshold of one slot, every allocation defers a
    // collection to the next safe point, which the self-call and same-chunk
    // return fast paths must still reach on every transfer.
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile(
            "gc-self-recursion.scm",
            "(let rec ((n 200))
               (if (= n 0)
                   0
                   (+ (car (cons 1 '())) (rec (- n 1)))))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(200)
    );
}

#[test]
fn instruction_fuel_terminates_a_runaway_self_tail_call() {
    // A self tail call never leaves the inner dispatch loop, so fuel and
    // interrupts must be enforced by the per-transfer safe point alone.
    let limits = Limits::default().with_fuel(Some(100_000));
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile("runaway-self-call.scm", "(begin (define (f) (f)) (f))")
        .unwrap();
    let error = engine.eval(&module).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ExecutionLimitExceeded);
}

#[test]
fn closures_mix_value_and_cell_captures_independently() {
    // `a` is captured read-only (value capture); `b` is mutated through the
    // closure (cell capture). Two instances must keep independent state, and
    // the value capture must still be visible alongside the cell.
    assert_eq!(
        evaluate(
            "(begin
               (define (make a b) (lambda () (set! b (+ b a)) b))
               (define f (make 10 0))
               (define g (make 100 0))
               (f) (f) (g)
               (equal? (list (f) (g)) '(30 200)))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn transitive_captures_keep_the_origin_binding_semantics() {
    // A value capture threaded through two closure levels reads the original.
    assert_eq!(
        evaluate("((((lambda (n) (lambda () (lambda () n))) 42)))"),
        Value::integer(42)
    );
    // A mutated binding stays a shared cell through the same chain: the
    // deeply nested reader observes the writer's updates.
    assert_eq!(
        evaluate(
            "(let ((n 5))
               (let ((write! (lambda () (set! n (+ n 1))))
                     (read (lambda () (lambda () n))))
                 (let ((reader (read)))
                   (write!)
                   (reader))))"
        ),
        Value::integer(6)
    );
}

#[test]
fn loop_closures_snapshot_per_iteration_values() {
    // The loop variable is captured (so the loop cannot flatten); each created
    // closure must remember its own iteration's value, not the final one.
    assert_eq!(
        evaluate(
            "(equal? '(0 1 2)
                     (let loop ((i 0) (acc '()))
                       (if (= i 3)
                           (map (lambda (f) (f)) (reverse acc))
                           (loop (+ i 1) (cons (lambda () i) acc)))))"
        ),
        Value::boolean(true)
    );
    // Same shape via `do`.
    assert_eq!(
        evaluate(
            "(equal? '(0 1 2)
                     (let ((fns '()))
                       (do ((i 0 (+ i 1)))
                           ((= i 3) (map (lambda (f) (f)) (reverse fns)))
                         (set! fns (cons (lambda () i) fns)))))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn delayed_expressions_capture_by_the_right_kind() {
    // An immutable capture inside `delay` snapshots the value; forcing twice
    // memoizes.
    assert_eq!(
        evaluate(
            "(begin
               (define (make-p x) (delay (* x 2)))
               (define p (make-p 21))
               (+ (force p) (force p)))"
        ),
        Value::integer(84)
    );
    // A binding mutated after the delay is created must be seen through its
    // cell when the promise is first forced; later mutations do not re-run the
    // memoized body.
    assert_eq!(
        evaluate(
            "(let ((x 1))
               (let ((p (delay x)))
                 (set! x 2)
                 (let ((first (force p)))
                   (set! x 3)
                   (equal? (list first (force p)) '(2 2)))))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn case_lambda_clauses_share_the_origin_binding() {
    // `n` is mutated by one clause and read by another: both clauses must see
    // the same cell.
    assert_eq!(
        evaluate(
            "(begin
               (define (mk n) (case-lambda (() n) ((x) (set! n x) n)))
               (define c (mk 1))
               (c 5)
               (c))"
        ),
        Value::integer(5)
    );
    // A read-only case-lambda capture stays correct across clauses.
    assert_eq!(
        evaluate(
            "(begin
               (define (mk n) (case-lambda (() n) ((x) (+ n x))))
               (define c (mk 7))
               (equal? (list (c) (c 3)) '(7 10)))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn continuation_reentry_preserves_value_captures() {
    // Re-entering a continuation captured inside a closure over an immutable
    // binding must observe the same captured value both times.
    assert_eq!(
        evaluate(
            "(let ((k #f) (log '()))
               (define (observe n)
                 (lambda ()
                   (set! log (cons (+ n (call/cc (lambda (c) (set! k c) 0))) log))
                   log))
               (let ((f (observe 10)))
                 (f)
                 (if (< (length log) 2) (k 5) (equal? (reverse log) '(10 15)))))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn escaping_loop_name_falls_back_to_closure() {
    // The loop name is used as a value (returned in a list) -> escapes -> closure
    // lowering; calling the escaped procedure still works.
    assert_eq!(
        evaluate(
            "(let ((fns (let loop ((i 0)) (if (= i 2) (list loop) (loop (+ i 1))))))
               (procedure? (car fns)))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn mutated_loop_variable_falls_back_to_closure() {
    // A `set!` of a loop variable disqualifies flattening; still correct.
    assert_eq!(
        evaluate(
            "(let loop ((i 0) (acc 0)) (set! acc (+ acc i)) (if (= i 10) acc (loop (+ i 1) acc)))"
        ),
        Value::integer(55)
    );
}

#[test]
fn captured_loop_variable_keeps_per_iteration_cells() {
    // A loop variable captured by an escaping lambda disqualifies flattening and
    // must preserve R7RS fresh-binding: each thunk sees its own iteration's `i`.
    // Sum of captured values is 0+1+2 = 3 with fresh cells; a single shared cell
    // would instead yield 2+2+2 = 6.
    assert_eq!(
        evaluate(
            "(let loop ((i 0) (acc '()))
               (if (= i 3)
                   (apply + (map (lambda (f) (f)) acc))
                   (loop (+ i 1) (cons (lambda () i) acc))))"
        ),
        Value::integer(3)
    );
}

#[test]
fn call_cc_escapes_a_flattened_loop() {
    // A continuation captured inside a flattened loop and invoked to escape it
    // works (the loop is ordinary frame registers, captured by the continuation).
    assert_eq!(
        evaluate(
            "(+ 1 (call-with-current-continuation
                    (lambda (k)
                      (let loop ((i 0))
                        (if (= i 3) (k 10) (loop (+ i 1)))))))"
        ),
        Value::integer(11)
    );
}

#[test]
fn do_loops_flatten_and_stay_correct() {
    // `do` desugars to a named let, so it reaches the same flattening path.
    assert_eq!(
        evaluate("(do ((i 0 (+ i 1)) (acc 0 (+ acc i))) ((= i 100) acc))"),
        Value::integer(4950)
    );
    // `do` with a body that captures the loop variable via a nested let.
    assert_eq!(
        evaluate("(do ((i 0 (+ i 1)) (s 0 (+ s (let ((d (* i 2))) d)))) ((= i 5) s))"),
        Value::integer(20)
    );
}

#[test]
fn pair_opcodes_respect_shadowing_and_stay_correct_under_gc() {
    // The inline pair/list opcodes (Cons/Car/Cdr/NullP/PairP) only fire when the
    // operator resolves to the unshadowed `(scheme base)` binding, and `cons`
    // allocates, so this covers both the redefinition-safety guard and the
    // opcode's root safety across collections.

    // list_sum shape: build a 200-cell list with `cons`, fold with car/cdr/null?.
    // A tiny GC threshold forces collections while the pairs are still live, so a
    // missing root would surface as a wrong sum or a crash.
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile(
            "pair-loop.scm",
            "(import (scheme base))
             (letrec ((build (lambda (n acc)
                               (if (= n 0) acc (build (- n 1) (cons n acc)))))
                      (sum (lambda (lst acc)
                             (if (null? lst) acc (sum (cdr lst) (+ acc (car lst)))))))
               (sum (build 200 '()) 0))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(200 * 201 / 2),
    );

    // A locally shadowed operator must win over the opcode.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((car (lambda (x) 42))) (car (cons 1 2)))",
        ),
        Value::integer(42),
    );
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((cons (lambda (a b) (+ a b)))
                   (null? (lambda (x) 7)))
               (+ (cons 3 4) (null? '())))",
        ),
        Value::integer(3 + 4 + 7),
    );

    // Used as a first-class value (not operator position), `car` still works via
    // the ordinary procedure path.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((f car)) (f (cons 9 10)))",
        ),
        Value::integer(9),
    );

    // `car`/`cdr` of a non-pair still raise the native TypeError from the opcode's
    // fallback, even inside a hot loop position.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "car-nonpair.scm",
            "(import (scheme base)) (car (cdr (cons 1 2)))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap_err().kind(),
        ErrorKind::TypeError,
    );
}

#[test]
fn variable_operands_read_home_register_without_redundant_moves() {
    // Non-boxed local operands of register opcodes (arith, comparisons, pair ops)
    // are read straight from their home register instead of being copied into a
    // scratch first. These cases would misbehave if that elision ever clobbered a
    // still-live variable or bypassed a boxed local's cell.

    // A variable used as an operand twice, and across a nested call, must keep its
    // value: (+ (- n 1) (- n 2)) with n=5 is 4 + 3 = 7.
    assert_eq!(
        evaluate("(import (scheme base)) (let ((n 5)) (+ (- n 1) (- n 2)))"),
        Value::integer(7),
    );

    // Same operand on both sides of a two-argument fold: (* n n) with n=6 is 36
    // (the accumulator reads n's home register; it must not be overwritten).
    assert_eq!(
        evaluate("(import (scheme base)) (let ((n 6)) (* n n))"),
        Value::integer(36),
    );

    // A `set!`-mutated (hence boxed) local must still read through its cell, not a
    // stale home-register slot.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((n 5)) (set! n (- n 1)) (set! n (* n 3)) n)",
        ),
        Value::integer(12),
    );

    // Comparison operand from a variable, used as a branch and then reused.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((n 4)) (if (< n 10) (+ n n) 0))",
        ),
        Value::integer(8),
    );

    // car/cdr/cons operands from variables (the pair opcodes) round-trip.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((p (cons 11 22))) (- (cdr p) (car p)))",
        ),
        Value::integer(11),
    );
}

#[test]
fn tail_returned_local_skips_the_result_move() {
    // A non-boxed local returned in tail position emits `Return home` directly,
    // with no `Move home -> scratch` beforehand. These would break if the elision
    // ever returned the wrong register or bypassed a boxed local's cell.

    // Plain tail return of a parameter.
    assert_eq!(
        evaluate("(import (scheme base)) ((lambda (x) x) 42)"),
        Value::integer(42),
    );

    // The recursion base case (`(if (= n 0) n ...)`) returns the local directly;
    // the whole fold must still compute 1+2+3+4+5 = 15.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (letrec ((sum (lambda (n) (if (= n 0) 0 (+ n (sum (- n 1)))))))
               (sum 5))",
        ),
        Value::integer(15),
    );

    // A `set!`-mutated (boxed) local returned in tail position must read through
    // its cell, not a raw register: n ends at 7.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             ((lambda (n) (set! n (+ n 1)) n) 6)",
        ),
        Value::integer(7),
    );

    // Both branches tail-return a distinct local; the value must track the branch.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((a 1) (b 2)) (if (< a b) b a))",
        ),
        Value::integer(2),
    );
}

#[test]
fn generic_call_results_feed_operands_and_arguments_without_moves() {
    // A generic (frame-pushing) call leaves its result in its own base register.
    // When that result feeds an arithmetic operand or an enclosing call's argument
    // slot, the compiler bases the call at the destination so no `Move` follows.
    // These exercises would misbehave if an inner call clobbered an already-live
    // sibling result, or if the aligned base collided with a live register.

    // Two generic-call results as the operands of `+` (the fibonacci shape). Every
    // fib(n) for n>=2 sums two recursive results read straight from their bases.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (letrec ((fib (lambda (n)
                             (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))))
               (fib 10))",
        ),
        Value::integer(55),
    );

    // Three generic-call results as the arguments of an enclosing call (the tak
    // shape): each inner call bases at its argument slot, and the earlier results
    // must survive the later inner calls' scratch use.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (letrec ((tak (lambda (x y z)
                             (if (< y x)
                                 (tak (tak (- x 1) y z)
                                      (tak (- y 1) z x)
                                      (tak (- z 1) x y))
                                 z))))
               (tak 18 12 6))",
        ),
        Value::integer(7),
    );

    // Mixed: a generic-call result and a live local as the two operands, where the
    // local sits below the call's aligned base and must not be overwritten.
    // g(x) = x*10; (+ n (g n)) with n=4 is 4 + 40 = 44.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (letrec ((g (lambda (x) (* x 10))))
               (let ((n 4)) (+ n (g n))))",
        ),
        Value::integer(44),
    );

    // A generic-call result as the first argument of a call whose remaining
    // arguments are leaves that read live locals: build (list (id a) b c).
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (letrec ((id (lambda (v) v)))
               (let ((a 1) (b 2) (c 3))
                 (let ((lst (list (id a) b c)))
                   (+ (car lst) (cadr lst) (car (cddr lst))))))",
        ),
        Value::integer(6),
    );
}

#[test]
fn general_closure_call_fast_path_matches_generic_semantics() {
    // The in-loop general-closure fast path (exact arity, no rest list, no
    // boxed locals) must behave exactly like the generic `call` path for every
    // shape it accepts, and every shape it rejects must still work through the
    // generic path.

    // Mutual recursion deep enough to grow the frame arena past its initial
    // high-water mark, alternating between two chunks every call.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (letrec ((even? (lambda (n) (if (= n 0) #t (odd? (- n 1)))))
                      (odd? (lambda (n) (if (= n 0) #f (even? (- n 1))))))
               (even? 10001))",
        ),
        Value::boolean(false),
    );

    // Non-tail mutual recursion: the callee frames stack up rather than being
    // reused, exercising the non-tail fast path with alternating chunks.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (letrec ((a (lambda (n) (if (= n 0) 0 (+ 1 (b (- n 1))))))
                      (b (lambda (n) (if (= n 0) 0 (+ 1 (a (- n 1)))))))
               (a 2000))",
        ),
        Value::integer(2000),
    );

    // Two distinct closures over one prototype chunk: the recycled frame slot
    // hits the chunk pointer compare but must still swap the captures.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((make-adder (lambda (n) (lambda (x) (+ x n)))))
               (let ((add3 (make-adder 3)) (add7 (make-adder 7)))
                 (let loop ((i 0) (acc 0))
                   (if (= i 100) acc (loop (+ i 1) (add7 (add3 acc)))))))",
        ),
        Value::integer(1000),
    );

    // Variadic callee: fails the exact-arity guard and must still build its
    // rest list through the generic path.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((f (lambda args (apply + args))))
               (let loop ((i 0) (acc 0))
                 (if (= i 10) acc (loop (+ i 1) (f acc 1 2)))))",
        ),
        Value::integer(30),
    );

    // Boxed-locals callee: `set!` on a parameter forces boxing, which the fast
    // path must reject so the cell is still created.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((f (lambda (x) (set! x (+ x 1)) x)))
               (let loop ((i 0) (acc 0))
                 (if (= i 10) acc (loop (+ i 1) (f acc)))))",
        ),
        Value::integer(10),
    );

    // Multiple values across a cross-chunk return.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((two (lambda (x) (values x (* x 2)))))
               (call-with-values (lambda () (two 21)) +))",
        ),
        Value::integer(63),
    );

    // A continuation captured inside a fast-path callee escapes and re-enters.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((saved #f))
               (let ((f (lambda (x)
                          (+ 100 (call/cc (lambda (k) (set! saved k) x))))))
                 (let ((first (f 1)))
                   (if saved
                       (let ((k saved))
                         (set! saved #f)
                         (k 41))
                       first))))",
        ),
        Value::integer(141),
    );

    // A raise inside a fast-path callee reaches the installed handler.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((f (lambda (x) (raise x))))
               (call/cc (lambda (k)
                 (with-exception-handler
                   (lambda (e) (k (+ e 1)))
                   (lambda () (f 41))))))",
        ),
        Value::integer(42),
    );
}

#[test]
fn general_closure_call_fast_path_survives_gc_pressure() {
    // A call-heavy loop that allocates in the callee, with a tiny collection
    // threshold: the frames written by the fast path must be precise roots so
    // the callee's freshly consed values survive the deferred collections.
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile(
            "gc-calls.scm",
            "(import (scheme base))
             (let ((make-pair (lambda (a b) (cons a b))))
               (let loop ((i 0) (acc '()))
                 (if (= i 500)
                     (let count ((lst acc) (n 0))
                       (if (null? lst) n (count (cdr lst) (+ n 1))))
                     (loop (+ i 1) (make-pair i acc)))))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(500)
    );
}

#[test]
fn allocation_fast_path_survives_repeated_build_and_drop_cycles() {
    // Repeated list build/drop cycles at the smallest collection threshold:
    // every cons crosses the soft threshold, so the deferred-collection trap,
    // the free-list reuse path, and the sweep run constantly while the live
    // list must survive precisely.
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile(
            "churn.scm",
            "(import (scheme base))
             (let outer ((round 0) (checksum 0))
               (if (= round 20)
                   checksum
                   (let build ((n 50) (acc '()))
                     (if (= n 0)
                         (let sum ((lst acc) (a 0))
                           (if (null? lst)
                               (outer (+ round 1) (+ checksum a))
                               (sum (cdr lst) (+ a (car lst)))))
                         (build (- n 1) (cons n acc))))))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(20 * 1275)
    );
}

#[test]
fn string_opcodes_respect_shadowing_and_fall_back_identically() {
    // StringRef/StringLength/CharToInteger only fire when the operator resolves
    // to the unshadowed `(scheme base)` binding. Shadowed and first-class uses
    // must take the generic call path, and every fast-path miss must raise the
    // same error as the native.

    // strscan shape under GC pressure: StringLength feeds the fused TestEqual
    // every iteration while StringRef + conses keep the collector busy. The
    // string literal stays rooted in a register across collections.
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile(
            "string-loop.scm",
            "(import (scheme base))
             (let ((s (make-string 200 #\\a)))
               (let loop ((i 0) (acc 0))
                 (if (= i (string-length s))
                     acc
                     (begin
                       (cons i i)
                       (loop (+ i 1) (+ acc (char->integer (string-ref s i))))))))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(200 * 97), // #\a is code point 97.
    );

    // Locally shadowed operators must win over the opcodes.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((string-length (lambda (s) 42)))
               (string-length \"abc\"))",
        ),
        Value::integer(42)
    );
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((string-ref (lambda (s i) #\\z))
                   (char->integer (lambda (c) 7)))
               (char->integer (string-ref \"abc\" 0)))",
        ),
        Value::integer(7)
    );

    // First-class references compile as plain closures over the natives.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((f string-ref) (g string-length))
               (+ (char->integer (f \"abc\" 1)) (g \"abc\")))",
        ),
        Value::integer(98 + 3) // #\b is code point 98.
    );

    // A non-fixnum index misses the opcode fast path and must raise the same
    // error the native raises.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "string-ref-float-index.scm",
            "(import (scheme base)) (string-ref \"abc\" 1.0)",
        )
        .unwrap();
    let opcode_error = engine.eval(&module).unwrap_err().kind();
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "string-ref-float-index-native.scm",
            "(import (scheme base)) (let ((f string-ref)) (f \"abc\" 1.0))",
        )
        .unwrap();
    assert_eq!(
        opcode_error,
        engine.eval(&module).unwrap_err().kind(),
        "opcode fallback and generic call must raise the same error kind",
    );
}

#[test]
fn predicate_test_fusion_preserves_polarity_shadowing_and_gc_safety() {
    // `null?`/`pair?` in boolean context compile to the fused TestNull/TestPair
    // branch opcodes. Cover both polarities (direct and under `not`), both
    // predicates on list and non-list values, short-circuit combinations, the
    // shadowing/first-class fallbacks that must bypass the fusion, and a
    // list-walking loop under GC pressure.

    // Both branch directions for both predicates, plus `not` inversion.
    for (source, expected) in [
        ("(if (null? '()) 1 2)", 1),
        ("(if (null? '(1)) 1 2)", 2),
        ("(if (null? 5) 1 2)", 2),
        ("(if (not (null? '())) 1 2)", 2),
        ("(if (not (null? '(1))) 1 2)", 1),
        ("(if (pair? '(1)) 1 2)", 1),
        ("(if (pair? '()) 1 2)", 2),
        ("(if (pair? 5) 1 2)", 2),
        ("(if (not (pair? '(1))) 1 2)", 2),
        // Short-circuit forms route through the same fused leaves with both
        // wanted polarities.
        ("(if (and (pair? '(1 2)) (null? (cdr '(1)))) 1 2)", 1),
        ("(if (and (pair? '()) (null? '())) 1 2)", 2),
        ("(if (or (null? '(1)) (pair? '(1))) 1 2)", 1),
        ("(if (or (null? '(1)) (pair? '())) 1 2)", 2),
    ] {
        assert_eq!(
            evaluate(&format!("(import (scheme base)) {source}")),
            Value::integer(expected),
            "{source}",
        );
    }

    // A locally shadowed predicate must win over the fusion, and a first-class
    // reference goes through the ordinary procedure path.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((null? (lambda (x) #f))) (if (null? '()) 1 2))",
        ),
        Value::integer(2),
    );
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((f pair?)) (if (f '(1)) 1 2))",
        ),
        Value::integer(1),
    );

    // A list-walking loop whose exit condition is the fused predicate branch,
    // with collections forced while the list is live.
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile(
            "predicate-loop.scm",
            "(import (scheme base))
             (letrec ((build (lambda (n acc)
                               (if (= n 0) acc (build (- n 1) (cons n acc)))))
                      (sum (lambda (lst acc)
                             (if (pair? lst) (sum (cdr lst) (+ acc (car lst))) acc))))
               (sum (build 200 '()) 0))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(200 * 201 / 2),
    );
}

#[test]
fn accumulate_fusion_preserves_results_and_error_paths() {
    // `(+ acc (vector-ref v i))`, `(+ acc (car l))`,
    // `(+ acc (char->integer (string-ref s i)))`, and `(± acc (* x y))` compile
    // to fused accumulate opcodes (AddVectorRef/AddCar/AddStringRefCode/
    // AddMul/SubMul). Cover exact results under GC pressure, float bit-identity
    // against the unfused spelling, literal RK multipliers, error-kind
    // equivalence on every decomposed miss path, overflow promotion, and the
    // shadowing guard.

    // Element-accumulate loops under a tiny GC threshold.
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile(
            "fused-accumulate.scm",
            "(import (scheme base))
             (let ((v (make-vector 100 3)) (s (make-string 50 #\\a)))
               (+ (let loop ((i 0) (acc 0))
                    (if (= i 100) acc (loop (+ i 1) (+ acc (vector-ref v i)))))
                  (let loop ((i 0) (acc 0))
                    (if (= i 50)
                        acc
                        (loop (+ i 1) (+ acc (char->integer (string-ref s i))))))
                  (let loop ((l '(1 2 3 4 5)) (acc 0))
                    (if (null? l) acc (loop (cdr l) (+ acc (car l)))))
                  (let loop ((i 0) (acc 0))
                    (if (= i 20) acc (loop (+ i 1) (+ acc (* i 3)))))))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(100 * 3 + 50 * 97 + 15 + 3 * 190),
    );

    // Mandelbrot-shaped float updates: the fused `SubMul`/`AddMul` words must
    // produce bit-identical results to the unfused let-temporary spelling
    // (same operations in the same order).
    let fused = evaluate(
        "(import (scheme base))
         (let loop ((zr 1.5) (zi 0.5) (n 0))
           (if (= n 10)
               (+ zr zi)
               (loop (+ (- (* zr zr) (* zi zi)) 0.25)
                     (+ (* 2.0 zr zi) 0.1)
                     (+ n 1))))",
    );
    let unfused = evaluate(
        "(import (scheme base))
         (let loop ((zr 1.5) (zi 0.5) (n 0))
           (if (= n 10)
               (+ zr zi)
               (let* ((a (* zr zr)) (b (* zi zi)) (c (* 2.0 zr zi)))
                 (loop (+ (- a b) 0.25) (+ c 0.1) (+ n 1)))))",
    );
    assert_eq!(fused, unfused);

    // A product that overflows i64 promotes through the same slow path as a
    // standalone multiply; the differential spelling proves it.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((a 4611686018427387904) (b 4))
               (if (= (+ 1 (* a b)) (let ((t (* a b))) (+ 1 t))) 1 0))",
        ),
        Value::integer(1),
    );

    // Error-kind equivalence between the fused word and the unfused spelling,
    // per decomposed miss path.
    let kinds = |fused_source: &str, unfused_source: &str| {
        let kind = |source: &str| {
            let mut engine = Engine::new(EngineConfig::default()).unwrap();
            let module = engine.compile("fused-error.scm", source).unwrap();
            engine.eval(&module).unwrap_err().kind()
        };
        (kind(fused_source), kind(unfused_source))
    };
    for (fused_source, unfused_source) in [
        // vector-ref out of range inside the fused word.
        (
            "(import (scheme base)) (+ 1 (vector-ref (vector 1 2) 9))",
            "(import (scheme base)) (let ((t (vector-ref (vector 1 2) 9))) (+ 1 t))",
        ),
        // car of a non-pair.
        (
            "(import (scheme base)) (+ 1 (car 5))",
            "(import (scheme base)) (let ((t (car 5))) (+ 1 t))",
        ),
        // string-ref out of range feeding char->integer.
        (
            "(import (scheme base)) (+ 1 (char->integer (string-ref \"ab\" 9)))",
            "(import (scheme base))
             (let ((t (char->integer (string-ref \"ab\" 9)))) (+ 1 t))",
        ),
        // non-numeric accumulator: the multiply succeeds, the add step raises.
        (
            "(import (scheme base)) (+ \"x\" (* 2 3))",
            "(import (scheme base)) (let ((t (* 2 3))) (+ \"x\" t))",
        ),
        // non-numeric multiplicand inside the fused multiply.
        (
            "(import (scheme base)) (- 1 (* \"x\" 3))",
            "(import (scheme base)) (let ((t (* \"x\" 3))) (- 1 t))",
        ),
    ] {
        let (fused_kind, unfused_kind) = kinds(fused_source, unfused_source);
        assert_eq!(fused_kind, unfused_kind, "{fused_source}");
    }

    // Shadowed operators bypass the fusion entirely.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((+ (lambda (a b) 99))) (+ 5 (vector-ref (vector 1) 0)))",
        ),
        Value::integer(99),
    );
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((vector-ref (lambda (v i) 7))) (+ 1 (vector-ref (vector 1) 0)))",
        ),
        Value::integer(8),
    );
}

#[test]
fn counted_loop_back_edge_test_fusion_preserves_boundaries_and_falls_back() {
    // The canonical counting loop `(if (= i limit) exit (loop .. (+ i 1) ..))`
    // fuses its back-edge into `LoopBackWhileNotEqual` (step + exit test +
    // branch in one word). Cover exact trip counts including the boundaries,
    // register and constant limits, a limit that changes every iteration
    // (parallel assignment), inexact and beyond-fixnum equality through the
    // slow paths, GC pressure, and the layouts/shadowings that must NOT fuse.

    // Trip counts: zero iterations, one iteration, and a register limit.
    for (source, expected) in [
        (
            "(let loop ((i 0) (acc 0)) (if (= i 0) acc (loop (+ i 1) (+ acc 1))))",
            0,
        ),
        (
            "(let loop ((i 0) (acc 0)) (if (= i 1) acc (loop (+ i 1) (+ acc 1))))",
            1,
        ),
        (
            "(let ((n 1000))
               (let loop ((i 0) (acc 0)) (if (= i n) acc (loop (+ i 1) (+ acc i)))))",
            999 * 1000 / 2,
        ),
        // The counter on the right of the comparison fuses too.
        (
            "(let loop ((i 0) (acc 0)) (if (= 100 i) acc (loop (+ i 1) (+ acc 1))))",
            100,
        ),
        // A limit that changes every iteration: the fused test reads the
        // freshly written parameter registers, exactly like the header test.
        (
            "(let loop ((i 0) (n 10)) (if (= i n) i (loop (+ i 1) (- n 1))))",
            5,
        ),
        // An inexact limit exits through the same numeric equality.
        (
            "(let loop ((i 0) (acc 0)) (if (= i 10.0) acc (loop (+ i 1) (+ acc 1))))",
            10,
        ),
        // Body-in-consequent layout (`<` header) stays on the unfused path.
        (
            "(let loop ((i 0) (acc 0)) (if (< i 10) (loop (+ i 1) (+ acc i)) acc))",
            45,
        ),
    ] {
        assert_eq!(
            evaluate(&format!("(import (scheme base)) {source}")),
            Value::integer(expected),
            "{source}",
        );
    }

    // Counter overflow past i64 promotes through the slow add, and the exit
    // equality against the heap-backed limit resolves through the slow
    // comparison; both inside the fused word.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((limit (+ 9223372036854775807 1)))
               (let loop ((i 9223372036854775805) (steps 0))
                 (if (= i limit) steps (loop (+ i 1) (+ steps 1)))))",
        ),
        Value::integer(3),
    );

    // A shadowed `=` compiles to a generic call header (no TestEqual), so the
    // back-edge must not fuse; the loop still terminates by the shadow's rule.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((= (lambda (a b) (>= a b))))
               (let loop ((i 0)) (if (= i 5) i (loop (+ i 1)))))",
        ),
        Value::integer(5),
    );

    // Counting loop allocating every iteration under a tiny GC threshold: the
    // fused back-edge still runs its per-iteration safe point.
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile(
            "fused-back-edge-gc.scm",
            "(import (scheme base))
             (let loop ((i 0) (acc '()))
               (if (= i 100) (length acc) (loop (+ i 1) (cons i acc))))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(100),
    );
}

#[test]
fn ordered_back_edge_fusion_preserves_boundaries_overflow_and_slow_paths() {
    // `LoopBackWhileNotLess` / `LoopBackWhileNotLessEqual`: `>`/`>=` exit
    // guards (compiled as swapped TestLess/TestLessEqual with the counter on
    // the right) re-run on the back-edge. Cover exact trip counts including
    // zero iterations, both guard spellings, an i64-overflowing counter that
    // promotes to a heap number through the same slow path as the unfused
    // words, and an inexact limit.
    for (source, expected) in [
        // `>` guard: runs for i = 0..=5, sum 15.
        (
            "(let ((limit 5))
               (let loop ((i 0) (acc 0)) (if (> i limit) acc (loop (+ i 1) (+ acc i)))))",
            15,
        ),
        // Zero iterations: guard true immediately.
        (
            "(let ((limit -1))
               (let loop ((i 0) (acc 0)) (if (> i limit) acc (loop (+ i 1) (+ acc 1)))))",
            0,
        ),
        // `>=` guard: runs for i = 0..=4, sum 10.
        (
            "(let ((limit 5))
               (let loop ((i 0) (acc 0)) (if (>= i limit) acc (loop (+ i 1) (+ acc i)))))",
            10,
        ),
        // Counter crosses i64::MAX: the +1 step promotes to a heap-backed
        // exact integer, and the replicated `>` guard compares it exactly.
        (
            "(let ((limit 9223372036854775807))
               (let loop ((i 9223372036854775805) (acc 0))
                 (if (> i limit) acc (loop (+ i 1) (+ acc 1)))))",
            3,
        ),
        // Inexact limit exits through the same numeric comparison tower.
        (
            "(let ((limit 4.5))
               (let loop ((i 0) (acc 0)) (if (> i limit) acc (loop (+ i 1) (+ acc 1)))))",
            5,
        ),
    ] {
        assert_eq!(evaluate(source), Value::integer(expected), "{source}");
    }
}

#[test]
fn strided_back_edge_fusion_steps_by_register_and_preserves_slow_paths() {
    // `LoopBackStepWhile{LessEqual,Less}`: the fall-into-body strided loop
    // (`(if (<= m limit) (begin .. (loop (+ m p))) ..)`) steps by a register
    // and re-runs the guard on the back-edge, with the stepped counter as the
    // left operand. Cover both guard kinds, the `(+ i i)` doubling step (the
    // step register IS the counter; the executor must read it before the
    // write), i64 overflow through the general add, a float stride, and a
    // multi-parameter accumulator riding alongside.
    for (source, expected) in [
        // `<=` guard, stride 3 from 9 to limit 20: m = 9,12,15,18; sum 54.
        (
            "(let ((p 3) (limit 20))
               (let loop ((m 9) (acc 0))
                 (if (<= m limit) (loop (+ m p) (+ acc m)) acc)))",
            54,
        ),
        // `<` guard: m = 9,12,15,18 with 18 < 20 still entering; sum 54.
        (
            "(let ((p 3) (limit 19))
               (let loop ((m 9) (acc 0))
                 (if (< m limit) (loop (+ m p) (+ acc m)) acc)))",
            54,
        ),
        // Zero iterations.
        (
            "(let ((p 3) (limit 5))
               (let loop ((m 9) (acc 0))
                 (if (<= m limit) (loop (+ m p) (+ acc m)) acc)))",
            0,
        ),
        // Doubling: the step register is the counter itself; i = 1,2,4,..,64.
        (
            "(let loop ((i 1) (acc 0))
               (if (<= i 100) (loop (+ i i) (+ acc i)) acc))",
            127,
        ),
        // Stride crosses i64::MAX: promoted exactly, guard exits.
        (
            "(let ((p 3) (limit 9223372036854775806))
               (let loop ((m 9223372036854775803) (acc 0))
                 (if (<= m limit) (loop (+ m p) (+ acc 1)) acc)))",
            2,
        ),
        // Float stride through the mixed numeric tower.
        (
            "(let ((p 0.5) (limit 3.0))
               (let loop ((m 1.0) (acc 0))
                 (if (<= m limit) (loop (+ m p) (+ acc 1)) acc)))",
            5,
        ),
    ] {
        assert_eq!(evaluate(source), Value::integer(expected), "{source}");
    }

    // The full sieve (the benchmark program) exercises both new back-edge
    // families and TestVectorRef together.
    let sieve = "(let* ((limit 2000)
                        (prime? (make-vector (+ limit 1) #t)))
                   (vector-set! prime? 0 #f)
                   (vector-set! prime? 1 #f)
                   (let mark-primes ((p 2))
                     (if (> (* p p) limit)
                         (let count-primes ((i 2) (count 0))
                           (if (> i limit)
                               count
                               (count-primes (+ i 1)
                                             (if (vector-ref prime? i)
                                                 (+ count 1)
                                                 count))))
                         (begin
                           (if (vector-ref prime? p)
                               (let mark-multiples ((multiple (* p p)))
                                 (if (<= multiple limit)
                                     (begin
                                       (vector-set! prime? multiple #f)
                                       (mark-multiples (+ multiple p)))))
                               #f)
                           (mark-primes (+ p 1))))))";
    assert_eq!(evaluate(sieve), Value::integer(303));
}

#[test]
fn constant_first_folds_skip_materialization_with_identical_semantics() {
    // A commutative n-ary fold whose first operand is a literal folds it into
    // the first step's RK slot instead of loading it into a register. Values
    // and errors must match the materializing path.
    for (source, expected) in [
        ("(* 2.0 3.0 4.0)", "24.0"),
        ("(* 2 3 4)", "24"),
        ("(+ 5 6 7 8)", "26"),
        ("(exact (round (* 2.0 (+ 1 2) 4)))", "24"),
    ] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine.compile("fold.scm", source).unwrap();
        let root = engine.eval(&module).unwrap().into_one().unwrap();
        assert_eq!(engine.write_root(&root).unwrap(), expected, "{source}");
    }
    // A non-numeric later operand raises the same error kind as the
    // materializing spelling.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile("fold-err.scm", "(* 2.0 (if #f #f) 4.0)")
        .unwrap();
    let folded = engine.eval(&module).unwrap_err();
    let module = engine
        .compile("fold-err2.scm", "(let ((two 2.0)) (* two (if #f #f) 4.0))")
        .unwrap();
    let materialized = engine.eval(&module).unwrap_err();
    assert_eq!(folded.kind(), materialized.kind());
}

#[test]
fn fixnum_constant_arithmetic_specialization_preserves_slow_paths() {
    // `AddFixnumK`/`SubtractFixnumK` compare raw payloads on the fast path.
    // A non-fixnum left operand and an i64 overflow must route through the
    // same numeric tower as the unfused words.
    for (source, expected) in [
        // Float left operand through the mixed tower.
        ("(exact (round (+ 1.5 10)))", "12"),
        // Overflow promotes to a heap-backed exact integer.
        ("(+ 9223372036854775800 100)", "9223372036854775900"),
        ("(- -9223372036854775800 100)", "-9223372036854775900"),
        // The k constant on the right of a subtraction chain.
        ("(- 1000000000000000 999999999999993)", "7"),
    ] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine.compile("fixk.scm", source).unwrap();
        let root = engine.eval(&module).unwrap().into_one().unwrap();
        assert_eq!(engine.write_root(&root).unwrap(), expected, "{source}");
    }
}

#[test]
fn fused_back_edges_stay_live_under_fuel_and_nan_guards() {
    // A NaN limit makes the replicated `=` exit test false forever (NaN
    // equals nothing); the fused back-edge must still hit safe points so the
    // fuel budget raises instead of hanging - identical to the unfused
    // spelling's behavior.
    let limits = Limits::default().with_fuel(Some(10_000));
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile(
            "nan-limit.scm",
            "(let ((limit (/ 0. 0.)))
               (let loop ((i 0) (acc 0)) (if (= i limit) acc (loop (+ i 1) (+ acc 1)))))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap_err().kind(),
        ErrorKind::ExecutionLimitExceeded
    );
}

#[test]
fn test_vector_ref_fusion_branches_on_truthiness_with_identical_errors() {
    // `(vector-ref v i)` as an `if` condition fuses into TestVectorRef; the
    // element is branched on with Scheme truthiness (only #f is false), and
    // every failure defers to the same slow path as the standalone VectorRef.
    for (source, expected) in [
        // #f element takes the alternate; 0 (a truthy value!) the consequent.
        (
            "(let ((v (vector #f 0 7)))
               (+ (if (vector-ref v 0) 100 1)
                  (if (vector-ref v 1) 20 2)
                  (if (vector-ref v 2) 300 3)))",
            321,
        ),
        // Negated polarity through `not`.
        (
            "(let ((v (vector #f 7)))
               (+ (if (not (vector-ref v 0)) 10 1)
                  (if (not (vector-ref v 1)) 200 2)))",
            12,
        ),
        // A literal (immutable) vector reads identically.
        ("(if (vector-ref '#(#f #t) 1) 5 6)", 5),
    ] {
        assert_eq!(evaluate(source), Value::integer(expected), "{source}");
    }

    // Error identity with the unfused word: out-of-range index and non-vector
    // raise exactly what the standalone `vector-ref` raises.
    for (fused, unfused) in [
        (
            "(let ((v (vector 1))) (if (vector-ref v 5) 1 2))",
            "(let ((v (vector 1))) (vector-ref v 5))",
        ),
        (
            "(let ((v 42)) (if (vector-ref v 0) 1 2))",
            "(let ((v 42)) (vector-ref v 0))",
        ),
        (
            "(let ((v (vector 1))) (if (vector-ref v -1) 1 2))",
            "(let ((v (vector 1))) (vector-ref v -1))",
        ),
    ] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let fused_error = {
            let module = engine.compile("fused.scm", fused).unwrap();
            engine.eval(&module).unwrap_err()
        };
        let unfused_error = {
            let module = engine.compile("unfused.scm", unfused).unwrap();
            engine.eval(&module).unwrap_err()
        };
        assert_eq!(fused_error.kind(), unfused_error.kind(), "{fused}");
        assert_eq!(
            format!("{fused_error}"),
            format!("{unfused_error}"),
            "{fused}"
        );
    }
}

#[test]
fn scratch_operand_loop_headers_swap_correctly_through_the_unfused_back_edge() {
    // A comparison operand that needs materialization every header evaluation
    // (`(string-length s)` into a scratch register) keeps the loop on the
    // unfused LoopBack + header-re-run path, because the tail call's argument
    // staging reuses that scratch register for the a/b swap. If back-edge test
    // fusion ever fired here it would compare the counter against the swapped
    // parameter instead of the string length and never terminate. The fuel
    // bound turns that regression into a clean failure instead of a hang.
    let limits = Limits::default().with_fuel(Some(1_000_000));
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile(
            "scratch-header.scm",
            "(let ((s (make-string 3 #\\a)))
               (let loop ((a 1) (b 2) (i 0))
                 (if (= i (string-length s)) (list a b) (loop b a (+ i 1)))))",
        )
        .unwrap();
    let root = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(engine.write_root(&root).unwrap(), "(2 1)");
}

#[test]
fn fused_mul_step_float_chain_matches_decomposed_shapes_and_propagates_nan() {
    // The AddMul/SubMul all-float chain computes `acc ± l * r` in f64 with a
    // single raw rebox. Every mixed shape must fall back to the decomposed
    // multiply-then-accumulate sequence with identical results, and a NaN
    // produced inside the chain must stay one `eqv?`/print equivalence class.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    for (source, expected) in [
        // All-float chain (mandelbrot shape), both polarities.
        ("(let ((a 1.5) (b 2.0) (c 3.0)) (+ a (* b c)))", "7.5"),
        ("(let ((a 1.5) (b 2.0) (c 3.0)) (- a (* b c)))", "-4.5"),
        // Mixed shapes: float acc/int product, int acc/float product,
        // int acc/int product - all decomposed.
        ("(let ((a 1.5) (b 2) (c 3)) (+ a (* b c)))", "7.5"),
        ("(let ((a 1) (b 2.0) (c 3.0)) (+ a (* b c)))", "7.0"),
        ("(let ((a 1) (b 2) (c 3)) (- a (* b c)))", "-5"),
        // Chain-produced NaN and infinity behave canonically downstream.
        (
            "(let ((a +inf.0) (b -1.0) (c +inf.0))
               (let ((x (+ a (* b c))))
                 (list (nan? x) (eqv? x (/ 0. 0.)) (number->string x))))",
            "(#t #t \"+nan.0\")",
        ),
        (
            "(let ((a 0.0) (b 1e308) (c 1e308)) (+ a (* b c)))",
            "+inf.0",
        ),
    ] {
        let module = engine.compile("mul_chain.scm", source).unwrap();
        let root = engine.eval(&module).unwrap().into_one().unwrap();
        assert_eq!(
            engine.write_root(&root).unwrap(),
            expected,
            "source: {source}"
        );
    }
}

#[test]
fn double_vector_ref_fusion_preserves_results_and_error_paths() {
    // `(+ acc (* (vector-ref v1 i1) (vector-ref v2 i2)))` with all four inner
    // operands home-readable fuses into `AddMulVectorRef`. Cover float
    // bit-identity against the unfused let-temporary spelling, integer results
    // with overflow promotion, error-kind equivalence on every decomposed miss
    // path, and the shadowing guard.

    // Float dot product: bit-identical to the unfused spelling.
    let fused = evaluate(
        "(import (scheme base))
         (let ((v (vector 1.5 2.25 3.125 4.0625)) (n 4))
           (let loop ((i 0) (sum 0.0))
             (if (= i n) sum
                 (loop (+ i 1) (+ sum (* (vector-ref v i) (vector-ref v i)))))))",
    );
    let unfused = evaluate(
        "(import (scheme base))
         (let ((v (vector 1.5 2.25 3.125 4.0625)) (n 4))
           (let loop ((i 0) (sum 0.0))
             (if (= i n) sum
                 (let* ((a (vector-ref v i)) (b (vector-ref v i)))
                   (loop (+ i 1) (+ sum (* a b)))))))",
    );
    assert_eq!(fused, unfused);

    // Distinct vectors and indexes, integer elements.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((v (vector 2 3)) (w (vector 5 7)) (i 0) (j 1) (acc 100))
               (+ acc (* (vector-ref v i) (vector-ref w j))))",
        ),
        Value::integer(100 + 2 * 7),
    );

    // A product that overflows i64 promotes through the same slow path as the
    // standalone words.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((v (vector 4611686018427387904 4)) (i 0) (j 1))
               (if (= (+ 1 (* (vector-ref v i) (vector-ref v j)))
                      (let* ((a (vector-ref v i)) (b (vector-ref v j)) (t (* a b)))
                        (+ 1 t)))
                   1 0))",
        ),
        Value::integer(1),
    );

    // Error-kind equivalence per decomposed miss path.
    let kinds = |fused_source: &str, unfused_source: &str| {
        let kind = |source: &str| {
            let mut engine = Engine::new(EngineConfig::default()).unwrap();
            let module = engine.compile("fused-error.scm", source).unwrap();
            engine.eval(&module).unwrap_err().kind()
        };
        (kind(fused_source), kind(unfused_source))
    };
    for (fused_source, unfused_source) in [
        // First fetch out of range.
        (
            "(import (scheme base))
             (let ((v (vector 1)) (w (vector 1)) (i 9) (j 0))
               (+ 1 (* (vector-ref v i) (vector-ref w j))))",
            "(import (scheme base))
             (let ((v (vector 1)) (w (vector 1)) (i 9) (j 0))
               (let* ((a (vector-ref v i)) (b (vector-ref w j))) (+ 1 (* a b))))",
        ),
        // Second fetch out of range.
        (
            "(import (scheme base))
             (let ((v (vector 1)) (w (vector 1)) (i 0) (j 9))
               (+ 1 (* (vector-ref v i) (vector-ref w j))))",
            "(import (scheme base))
             (let ((v (vector 1)) (w (vector 1)) (i 0) (j 9))
               (let* ((a (vector-ref v i)) (b (vector-ref w j))) (+ 1 (* a b))))",
        ),
        // First operand is not a vector.
        (
            "(import (scheme base))
             (let ((v 5) (w (vector 1)) (i 0) (j 0))
               (+ 1 (* (vector-ref v i) (vector-ref w j))))",
            "(import (scheme base))
             (let ((v 5) (w (vector 1)) (i 0) (j 0))
               (let* ((a (vector-ref v i)) (b (vector-ref w j))) (+ 1 (* a b))))",
        ),
        // Non-numeric elements: the fetches succeed, the multiply raises.
        (
            "(import (scheme base))
             (let ((v (vector \"x\")) (w (vector 2)) (i 0) (j 0))
               (+ 1 (* (vector-ref v i) (vector-ref w j))))",
            "(import (scheme base))
             (let ((v (vector \"x\")) (w (vector 2)) (i 0) (j 0))
               (let* ((a (vector-ref v i)) (b (vector-ref w j))) (+ 1 (* a b))))",
        ),
        // Non-numeric accumulator: the multiply succeeds, the add raises.
        (
            "(import (scheme base))
             (let ((v (vector 2)) (w (vector 3)) (i 0) (j 0))
               (+ \"x\" (* (vector-ref v i) (vector-ref w j))))",
            "(import (scheme base))
             (let ((v (vector 2)) (w (vector 3)) (i 0) (j 0))
               (let* ((a (vector-ref v i)) (b (vector-ref w j))) (+ \"x\" (* a b))))",
        ),
    ] {
        let (fused_kind, unfused_kind) = kinds(fused_source, unfused_source);
        assert_eq!(fused_kind, unfused_kind, "{fused_source}");
    }

    // Shadowed operators bypass the fusion entirely.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((vector-ref (lambda (v i) 7)) (v (vector 1)) (i 0))
               (+ 1 (* (vector-ref v i) (vector-ref v i))))",
        ),
        Value::integer(50),
    );
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((* (lambda (a b) 9)) (v (vector 1)) (i 0))
               (+ 1 (* (vector-ref v i) (vector-ref v i))))",
        ),
        Value::integer(10),
    );
}

#[test]
fn chained_vector_ref_fusion_preserves_results_and_error_paths() {
    // `(vector-ref (vector-ref m k) j)` with a home-readable outer index fuses
    // into `VectorRefVectorRef`. Cover the row-then-element result, error-kind
    // equivalence on both fetch misses, a non-vector inner result, and the
    // shadowing guard.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((m (vector (vector 1 2) (vector 3 4))) (k 1) (j 0))
               (vector-ref (vector-ref m k) j))",
        ),
        Value::integer(3),
    );

    let kinds = |fused_source: &str, unfused_source: &str| {
        let kind = |source: &str| {
            let mut engine = Engine::new(EngineConfig::default()).unwrap();
            let module = engine.compile("chained-error.scm", source).unwrap();
            engine.eval(&module).unwrap_err().kind()
        };
        (kind(fused_source), kind(unfused_source))
    };
    for (fused_source, unfused_source) in [
        // Inner fetch out of range.
        (
            "(import (scheme base))
             (let ((m (vector (vector 1))) (k 9) (j 0))
               (vector-ref (vector-ref m k) j))",
            "(import (scheme base))
             (let ((m (vector (vector 1))) (k 9) (j 0))
               (let ((row (vector-ref m k))) (vector-ref row j)))",
        ),
        // Outer fetch out of range.
        (
            "(import (scheme base))
             (let ((m (vector (vector 1))) (k 0) (j 9))
               (vector-ref (vector-ref m k) j))",
            "(import (scheme base))
             (let ((m (vector (vector 1))) (k 0) (j 9))
               (let ((row (vector-ref m k))) (vector-ref row j)))",
        ),
        // The inner element is not a vector.
        (
            "(import (scheme base))
             (let ((m (vector 5)) (k 0) (j 0))
               (vector-ref (vector-ref m k) j))",
            "(import (scheme base))
             (let ((m (vector 5)) (k 0) (j 0))
               (let ((row (vector-ref m k))) (vector-ref row j)))",
        ),
    ] {
        let (fused_kind, unfused_kind) = kinds(fused_source, unfused_source);
        assert_eq!(fused_kind, unfused_kind, "{fused_source}");
    }

    // A shadowed vector-ref bypasses the fusion.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((vector-ref (lambda (v i) 7)) (m (vector 1)) (k 0) (j 0))
               (vector-ref (vector-ref m k) j))",
        ),
        Value::integer(7),
    );
}

#[test]
fn greater_test_fusion_preserves_semantics_errors_and_shadowing() {
    // `(> x LITERAL)` and `(>= x LITERAL)` in test position keep their own
    // opcodes (`TestGreater`/`TestGreaterEqual`) with the literal in the k
    // slot instead of the old swap-plus-LoadK lowering. Cover truth values in
    // both branch polarities, NaN on either side, error-kind identity against
    // the value-position spelling, and the shadowing guard.
    assert_eq!(
        evaluate("(let ((x 5)) (if (> x 4) 1 0))"),
        Value::integer(1)
    );
    assert_eq!(
        evaluate("(let ((x 4)) (if (> x 4) 1 0))"),
        Value::integer(0)
    );
    assert_eq!(
        evaluate("(let ((x 4)) (if (>= x 4) 1 0))"),
        Value::integer(1)
    );
    assert_eq!(
        evaluate("(let ((x 3)) (if (>= x 4) 1 0))"),
        Value::integer(0)
    );
    // Inverted polarity through `not`.
    assert_eq!(
        evaluate("(let ((x 5)) (if (not (> x 4)) 1 0))"),
        Value::integer(0)
    );
    assert_eq!(
        evaluate("(let ((x 3)) (if (not (>= x 4)) 1 0))"),
        Value::integer(1)
    );
    // A fixnum left operand against a float constant goes through the generic
    // numeric helpers (exact/inexact comparison stays correct).
    assert_eq!(
        evaluate("(let ((x 5)) (if (> x 4.0) 1 0))"),
        Value::integer(1)
    );
    assert_eq!(
        evaluate("(let ((x 4)) (if (>= x 4.5) 1 0))"),
        Value::integer(0)
    );
    // Float constant in the k slot (the mandelbrot escape-test shape).
    assert_eq!(
        evaluate("(let ((x 9.0)) (if (> x 4.0) 1 0))"),
        Value::integer(1)
    );
    assert_eq!(
        evaluate("(let ((x 1.0)) (if (> x 4.0) 1 0))"),
        Value::integer(0)
    );
    // NaN makes every ordering false, in both polarities, whether NaN is the
    // register operand or the folded constant.
    assert_eq!(evaluate("(if (> +nan.0 1.0) 1 0)"), Value::integer(0));
    assert_eq!(evaluate("(if (>= +nan.0 1.0) 1 0)"), Value::integer(0));
    assert_eq!(evaluate("(if (not (> +nan.0 1.0)) 1 0)"), Value::integer(1));
    assert_eq!(
        evaluate("(let ((x 1.0)) (if (> x +nan.0) 1 0))"),
        Value::integer(0)
    );
    assert_eq!(
        evaluate("(let ((x 1.0)) (if (>= x +nan.0) 1 0))"),
        Value::integer(0)
    );

    // A non-numeric left operand raises the same error kind as the
    // value-position spelling of the identical comparison.
    let kind = |source: &str| {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine.compile("greater-error.scm", source).unwrap();
        engine.eval(&module).unwrap_err().kind()
    };
    for (fused_source, unfused_source) in [
        (
            "(let ((x \"s\")) (if (> x 4) 1 0))",
            "(let ((x \"s\")) (let ((p (> x 4))) (if p 1 0)))",
        ),
        (
            "(let ((x \"s\")) (if (>= x 4.0) 1 0))",
            "(let ((x \"s\")) (let ((p (>= x 4.0))) (if p 1 0)))",
        ),
    ] {
        assert_eq!(kind(fused_source), kind(unfused_source), "{fused_source}");
    }

    // A shadowed `>` wins over the fusion.
    assert_eq!(
        evaluate("(let ((> (lambda (a b) #f))) (if (> 5 1) 1 0))"),
        Value::integer(0)
    );
    assert_eq!(
        evaluate("(let ((>= (lambda (a b) 42))) (if (>= 1 5) 1 0))"),
        Value::integer(1)
    );
}

#[test]
fn add_sub_fixnum_fusion_preserves_results_and_error_paths() {
    // The wide-literal accumulate `(- (+ acc K1) K2)` written back to the
    // accumulator's own register collapses into one `AddSubFixnumK` word.
    // Cover result identity against the unfused let-temporary spelling,
    // overflow promotion at the intermediate, error-kind identity on every
    // decomposed miss path, and the shadowing guard.
    let fused = evaluate(
        "(let loop ((i 0) (acc 1000000000000000))
           (if (= i 1000) acc
               (loop (+ i 1) (- (+ acc 1000000000000000) 999999999999993))))",
    );
    let unfused = evaluate(
        "(let loop ((i 0) (acc 1000000000000000))
           (if (= i 1000) acc
               (let ((t (+ acc 1000000000000000)))
                 (loop (+ i 1) (- t 999999999999993)))))",
    );
    assert_eq!(fused, unfused);

    // The commuted inner add `(+ K1 acc)` fuses to the same word.
    assert_eq!(
        evaluate(
            "(let loop ((i 0) (acc 10))
               (if (= i 3) acc (loop (+ i 1) (- (+ 7 acc) 5))))",
        ),
        Value::integer(16),
    );

    // An i64 overflow at the intermediate only (the net result is back in
    // range) promotes through the same slow path as the unfused pair, which
    // also proves the two constants are not folded together at compile time.
    let fused = evaluate(
        "(let loop ((i 0) (acc 9223372036854775800))
           (if (= i 1) acc (loop (+ i 1) (- (+ acc 100) 200))))",
    );
    let unfused = evaluate(
        "(let loop ((i 0) (acc 9223372036854775800))
           (if (= i 1) acc
               (let ((t (+ acc 100))) (loop (+ i 1) (- t 200)))))",
    );
    assert_eq!(fused, unfused);

    // A float accumulator misses the fixnum fast path every iteration and
    // must be bit-identical to the unfused spelling.
    let fused = evaluate(
        "(let loop ((i 0) (acc 0.5))
           (if (= i 3) acc (loop (+ i 1) (- (+ acc 3) 1))))",
    );
    let unfused = evaluate(
        "(let loop ((i 0) (acc 0.5))
           (if (= i 3) acc (let ((t (+ acc 3))) (loop (+ i 1) (- t 1)))))",
    );
    assert_eq!(fused, unfused);

    // Error-kind identity per decomposed miss path.
    let kind = |source: &str| {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine.compile("add-sub-error.scm", source).unwrap();
        engine.eval(&module).unwrap_err().kind()
    };
    for (fused_source, unfused_source) in [
        // A non-numeric accumulator: the add raises first.
        (
            "(let loop ((i 0) (acc \"x\"))
               (if (= i 1) acc (loop (+ i 1) (- (+ acc 100) 200))))",
            "(let loop ((i 0) (acc \"x\"))
               (if (= i 1) acc
                   (let ((t (+ acc 100))) (loop (+ i 1) (- t 200)))))",
        ),
        // An i128 overflow at the intermediate raises the implementation
        // restriction from the add step.
        (
            "(let loop ((i 0) (acc 170141183460469231731687303715884105727))
               (if (= i 1) acc (loop (+ i 1) (- (+ acc 100) 200))))",
            "(let loop ((i 0) (acc 170141183460469231731687303715884105727))
               (if (= i 1) acc
                   (let ((t (+ acc 100))) (loop (+ i 1) (- t 200)))))",
        ),
    ] {
        assert_eq!(kind(fused_source), kind(unfused_source), "{fused_source}");
    }

    // Shadowed operators bypass the fusion entirely.
    assert_eq!(
        evaluate(
            "(let ((+ (lambda (a b) 100)))
               (let loop ((i 0) (acc 5))
                 (if (= i 0) (- (+ acc 1) 2) (loop i acc))))",
        ),
        Value::integer(98),
    );
    assert_eq!(
        evaluate(
            "(let ((- (lambda (a b) 100)))
               (let loop ((i 1) (acc 5))
                 (if (= i 1) (- (+ acc 1) 2) (loop i acc))))",
        ),
        Value::integer(100),
    );

    // A different variable's home register is not the destination, so the
    // unfused pair is kept and stays correct.
    assert_eq!(
        evaluate(
            "(let loop ((i 0) (acc 5) (other 7))
               (if (= i 3) acc (loop (+ i 1) (- (+ other 1) 2) other)))",
        ),
        Value::integer(6),
    );
}

#[test]
fn list_scan_fast_paths_keep_identity_and_defer_errors_to_the_native() {
    // assq/assv/memq/memv and the equal-based assoc/member wrappers dispatch
    // through a bounded fast scan classified at registration. A hit must
    // return the stored entry or sublist itself (eq? identity, never a copy)
    // and a proper-list miss returns #f. Any other shape defers to the
    // canonical native so the identical error is raised.

    // Hit identity for the eqv-based scans.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((al (list (cons 'a 1) (cons 'b 2) (cons 'c 3))))
               (and (eq? (assq 'a al) (car al))
                    (eq? (assv 'c al) (car (cddr al)))))",
        ),
        Value::boolean(true)
    );
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((l (list 'a 'b 'c)))
               (and (eq? (memq 'c l) (cddr l))
                    (eq? (memv 'a l) l)))",
        ),
        Value::boolean(true)
    );

    // Hit identity for the equal-based scans (string keys force equal?).
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((al (list (cons \"aa\" 1) (cons \"bb\" 2))))
               (eq? (assoc \"bb\" al) (cadr al)))",
        ),
        Value::boolean(true)
    );
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((l (list \"x\" \"y\")))
               (eq? (member \"y\" l) (cdr l)))",
        ),
        Value::boolean(true)
    );

    // A proper-list miss stays #f on the fast path.
    assert_eq!(
        evaluate("(import (scheme base)) (assq 'x (list (cons 'a 1)))"),
        Value::boolean(false)
    );
    assert_eq!(
        evaluate("(import (scheme base)) (memq 'x (list 'a 'b))"),
        Value::boolean(false)
    );

    // Hits past the fast-scan bound cross into the deferred general path and
    // still answer correctly.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let loop ((n 200) (al '()))
               (if (= n 0)
                   (cdr (assv 150 al))
                   (loop (- n 1) (cons (cons n (* n 10)) al))))",
        ),
        Value::integer(1500)
    );
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let loop ((n 200) (l '()))
               (if (= n 0)
                   (car (memv 150 l))
                   (loop (- n 1) (cons n l))))",
        ),
        Value::integer(150)
    );

    // Error identity per defer path: an improper tail, a non-pair alist
    // entry, and a circular list raise the same TypeError as the canonical
    // scan.
    let failing = [
        "(memq 'x (cons 1 2))",
        "(assq 'x (list 1 2))",
        "(let ((l (list 'a 'b))) (set-cdr! (cdr l) l) (memq 'x l))",
        "(let ((al (list (cons 'a 1) (cons 'b 2))))
           (set-cdr! (cdr al) al)
           (assq 'x al))",
    ];
    for source in failing {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine
            .compile(
                "list-scan-error.scm",
                format!("(import (scheme base)) {source}"),
            )
            .unwrap();
        assert_eq!(
            engine.eval(&module).unwrap_err().kind(),
            ErrorKind::TypeError,
            "{source} should raise TypeError via the deferred path",
        );
    }
}

#[test]
fn list_walk_fast_paths_match_values_and_defer_errors_to_the_native() {
    // length, list-ref, and list-tail dispatch through bounded fast walks. A
    // hit must match the canonical value, including list-tail's non-pair
    // result after exactly k steps. Non-fixnum or negative counts, walks past
    // the end, and circular lists defer to the canonical native for the
    // identical error.

    // Hits on the fast path.
    assert_eq!(
        evaluate("(import (scheme base)) (length (list 1 2 3))"),
        Value::integer(3)
    );
    assert_eq!(
        evaluate("(import (scheme base)) (length '())"),
        Value::integer(0)
    );
    assert_eq!(
        evaluate("(import (scheme base)) (list-ref (list 10 20 30) 2)"),
        Value::integer(30)
    );
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((l (list 1 2 3)))
               (eq? (list-tail l 1) (cdr l)))",
        ),
        Value::boolean(true)
    );
    // list-tail may land on a non-pair after k steps, including the improper
    // tail itself.
    assert_eq!(
        evaluate("(import (scheme base)) (list-tail (cons 1 2) 1)"),
        Value::integer(2)
    );
    assert_eq!(
        evaluate("(import (scheme base)) (null? (list-tail (list 1) 1))"),
        Value::boolean(true)
    );

    // Walks past the fast bound cross into the deferred general path.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let loop ((n 200) (l '()))
               (if (= n 0)
                   (+ (length l) (list-ref l 150) (car (list-tail l 199)))
                   (loop (- n 1) (cons n l))))",
        ),
        Value::integer(551)
    );

    // Error identity per defer path.
    let failing = [
        ("(length (cons 1 2))", ErrorKind::TypeError),
        (
            "(let ((l (list 1 2))) (set-cdr! (cdr l) l) (length l))",
            ErrorKind::TypeError,
        ),
        ("(list-ref (list 1 2) 5)", ErrorKind::TypeError),
        ("(list-ref (list 1 2) -1)", ErrorKind::RangeError),
        ("(list-ref (list 1 2) 'x)", ErrorKind::TypeError),
        ("(list-tail (list 1 2) 3)", ErrorKind::TypeError),
    ];
    for (source, kind) in failing {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine
            .compile(
                "list-walk-error.scm",
                format!("(import (scheme base)) {source}"),
            )
            .unwrap();
        assert_eq!(
            engine.eval(&module).unwrap_err().kind(),
            kind,
            "{source} should raise {kind:?} via the deferred path",
        );
    }
}

#[test]
fn reverse_and_append_fast_paths_share_tails_and_defer_errors() {
    // reverse and two-argument append dispatch through bounded allocating
    // fast arms. Values must match the canonical natives, including append's
    // R7RS tail sharing: the last argument is returned uncopied. Other
    // arities, longer lists, improper shapes, and cycles defer to the
    // canonical native for the identical value or error.

    // Hits on the fast path.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (equal? (reverse (list 1 2 3)) '(3 2 1))",
        ),
        Value::boolean(true)
    );
    assert_eq!(
        evaluate("(import (scheme base)) (null? (reverse '()))"),
        Value::boolean(true)
    );
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (equal? (append (list 1 2) (list 3)) '(1 2 3))",
        ),
        Value::boolean(true)
    );

    // The second argument is shared, never copied.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let ((t (list 1 2)))
               (eq? (list-tail (append (list 9) t) 1) t))",
        ),
        Value::boolean(true)
    );

    // The second argument may be any object, matching the last-argument rule.
    assert_eq!(
        evaluate("(import (scheme base)) (cdr (append (list 1) 5))"),
        Value::integer(5)
    );

    // Other arities defer and stay correct.
    assert_eq!(
        evaluate("(import (scheme base)) (null? (append))"),
        Value::boolean(true)
    );
    assert_eq!(
        evaluate("(import (scheme base)) (car (append (list 7)))"),
        Value::integer(7)
    );
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (equal? (append (list 1) (list 2) (list 3)) '(1 2 3))",
        ),
        Value::boolean(true)
    );

    // Lists past the fast bound cross into the deferred general path.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (let loop ((n 200) (l '()))
               (if (= n 0)
                   (+ (car (reverse l)) (length (append l l)))
                   (loop (- n 1) (cons n l))))",
        ),
        Value::integer(600)
    );

    // Error identity per defer path.
    let failing = [
        "(reverse (cons 1 2))",
        "(let ((l (list 1 2))) (set-cdr! (cdr l) l) (reverse l))",
        "(append (cons 1 2) (list 3))",
        "(let ((l (list 1 2))) (set-cdr! (cdr l) l) (append l (list 3)))",
    ];
    for source in failing {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine
            .compile(
                "reverse-append-error.scm",
                format!("(import (scheme base)) {source}"),
            )
            .unwrap();
        assert_eq!(
            engine.eval(&module).unwrap_err().kind(),
            ErrorKind::TypeError,
            "{source} should raise TypeError via the deferred path",
        );
    }
}

#[test]
fn equal_prefix_matches_the_recursive_compare() {
    // equal? runs a non-allocating prefix (eqv, then flat string and
    // bytevector compares) before the recursive worklist. Results must be
    // identical across flat, mixed, deep, and cyclic shapes.
    let cases = [
        // Flat strings and bytevectors on the prefix.
        ("(equal? \"abc\" (string-copy \"abc\"))", true),
        ("(equal? \"abc\" \"abd\")", false),
        ("(equal? (bytevector 1 2 3) (bytevector 1 2 3))", true),
        ("(equal? (bytevector 1 2) (bytevector 1 3))", false),
        // Numbers keep eqv? semantics: exactness matters.
        ("(equal? 2 2.0)", false),
        ("(equal? 2.0 2.0)", true),
        // Mixed shapes never recurse and are #f.
        ("(equal? \"a\" 'a)", false),
        ("(equal? \"abc\" (bytevector 97 98 99))", false),
        ("(equal? (list 1) (vector 1))", false),
        ("(equal? (list 1) \"x\")", false),
        // Deep structures still reach the worklist.
        (
            "(equal? (list 1 (vector 2 \"x\") (bytevector 3))
                     (list 1 (vector 2 \"x\") (bytevector 3)))",
            true,
        ),
        (
            "(equal? (list 1 (vector 2 \"x\"))
                     (list 1 (vector 2 \"y\")))",
            false,
        ),
        // Cyclic structures terminate via the seen set.
        (
            "(let ((a (list 1 2)) (b (list 1 2)))
               (set-cdr! (cdr a) a)
               (set-cdr! (cdr b) b)
               (equal? a b))",
            true,
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(
            evaluate(&format!("(import (scheme base)) {source}")),
            Value::boolean(expected),
            "{source}",
        );
    }
}

#[test]
fn tail_called_natives_deliver_values_and_route_errors() {
    // A closure body ending in a native call takes the TailCall native fast
    // path: the frame pops and the result delivers through the popped
    // frame's return slot. Single, multiple, and zero values and error
    // routing must match the generic tail-call path.

    // Single value through a popped frame.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (define (f l) (reverse l))
             (car (f (list 1 2 3)))",
        ),
        Value::integer(3)
    );

    // Multiple values from a tail-called native.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (define (f x) (values x (+ x 1)))
             (call-with-values (lambda () (f 5)) (lambda (a b) (+ a b)))",
        ),
        Value::integer(11)
    );

    // Zero values from a tail-called native.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (define (g) (values))
             (call-with-values g (lambda () 42))",
        ),
        Value::integer(42)
    );

    // A native error raised in tail position reaches the active handler.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (eq? 'caught
                  (call-with-current-continuation
                    (lambda (k)
                      (with-exception-handler
                        (lambda (e) (k 'caught))
                        (lambda () ((lambda (l) (length l)) (cons 1 2)))))))",
        ),
        Value::boolean(true)
    );

    // A deep chain of frames each tail-calling into the next still delivers
    // to the original caller.
    assert_eq!(
        evaluate(
            "(import (scheme base))
             (define (wrap l n)
               (if (= n 0) (length l) (wrap l (- n 1))))
             (wrap (list 1 2 3 4) 10)",
        ),
        Value::integer(4)
    );
}
