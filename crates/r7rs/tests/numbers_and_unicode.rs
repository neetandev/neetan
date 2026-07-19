use r7rs::{Engine, EngineConfig, ErrorKind, Value};

fn run(engine: &mut Engine, source: &str) -> Value {
    let module = engine.compile("numbers_and_unicode.scm", source).unwrap();
    engine.eval(&module).unwrap().into_one().unwrap().value()
}

#[test]
fn expt_handles_huge_exact_exponents_without_a_linear_native_loop() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(
            &mut engine,
            "(and (= (expt 1 170141183460469231731687303715884105727) 1)
                  (= (expt -1 170141183460469231731687303715884105727) -1)
                  (= (expt -1 170141183460469231731687303715884105726) 1))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn fixnum_boundary_and_float_edges_round_trip() {
    // Guards the tagged `Value`: the inline fixnum range is the full i64, so
    // exact integers of larger magnitude must remain correct via the heap
    // `i128` path, and float edge cases (signed zero, NaN canonicalization,
    // infinities) must preserve IEEE semantics.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // Inline fixnum boundary (FIXNUM_MAX = 2^63 - 1, FIXNUM_MIN = -2^63).
    assert_eq!(
        run(&mut engine, "(= (- (expt 2 63) 1) 9223372036854775807)"),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(= (- 0 (expt 2 63)) -9223372036854775808)"),
        Value::boolean(true)
    );

    // Just past the inline range: heap-backed i128, still exact.
    assert_eq!(
        run(&mut engine, "(= (expt 2 64) 18446744073709551616)"),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(exact? (expt 2 100))"),
        Value::boolean(true)
    );

    // Arithmetic that crosses the i64 boundary spills to the heap and stays exact.
    assert_eq!(
        run(&mut engine, "(= (+ (expt 2 62) (expt 2 62)) (expt 2 63))"),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(= (* (expt 2 62) 4) (expt 2 64))"),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(= (* 1000000000 1000000000) 1000000000000000000)"
        ),
        Value::boolean(true)
    );

    // A value fitting i64 (and now inline) round-trips through number->string.
    assert_eq!(
        run(
            &mut engine,
            "(string=? (number->string 1000000000000000) \"1000000000000000\")"
        ),
        Value::boolean(true)
    );

    // Signed zero: eqv? distinguishes, = does not.
    assert_eq!(run(&mut engine, "(eqv? 0.0 -0.0)"), Value::boolean(false));
    assert_eq!(run(&mut engine, "(= 0.0 -0.0)"), Value::boolean(true));

    // NaN: never = to itself, canonicalized so its bits are stable, produced by
    // several routes.
    assert_eq!(run(&mut engine, "(= +nan.0 +nan.0)"), Value::boolean(false));
    assert_eq!(
        run(&mut engine, "(eqv? +nan.0 +nan.0)"),
        Value::boolean(true)
    );
    assert_eq!(run(&mut engine, "(nan? +nan.0)"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(nan? (- +nan.0))"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(nan? (/ 0.0 0.0))"), Value::boolean(true));

    // Infinities are ordinary (non-boxed) floats and order correctly.
    assert_eq!(
        run(&mut engine, "(= +inf.0 (/ 1.0 0.0))"),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(< -inf.0 0.0 +inf.0)"),
        Value::boolean(true)
    );
    assert_eq!(run(&mut engine, "(infinite? -inf.0)"), Value::boolean(true));

    // An ordinary inline float decodes exactly.
    assert_eq!(run(&mut engine, "(= 3.14 3.14)"), Value::boolean(true));
}

#[test]
fn eqv_distinguishes_exactness_per_r7rs() {
    // R7RS: eqv? returns #f when one operand is exact and the other inexact,
    // even though they may be numerically = ; same-exactness operands compare
    // by exact numeric equality or by inexact bit-identity.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // Exactness mismatch => #f (but = still reports numeric equality).
    assert_eq!(run(&mut engine, "(eqv? 5 5.0)"), Value::boolean(false));
    assert_eq!(run(&mut engine, "(eqv? 5.0 5)"), Value::boolean(false));
    assert_eq!(run(&mut engine, "(= 5 5.0)"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(eqv? 1/2 0.5)"), Value::boolean(false));
    assert_eq!(
        run(&mut engine, "(eqv? (expt 2 60) (inexact (expt 2 60)))"),
        Value::boolean(false)
    );

    // Same exactness => normal comparison.
    assert_eq!(run(&mut engine, "(eqv? 2 2)"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(eqv? 2.0 2.0)"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(eqv? 1/3 1/3)"), Value::boolean(true));

    // Inexact bit rule: signed zero distinct, canonicalized NaN eqv.
    assert_eq!(run(&mut engine, "(eqv? 0.0 -0.0)"), Value::boolean(false));
    assert_eq!(
        run(&mut engine, "(eqv? +nan.0 +nan.0)"),
        Value::boolean(true)
    );

    // Complex numbers respect exactness component-wise.
    assert_eq!(run(&mut engine, "(eqv? 1+2i 1+2i)"), Value::boolean(true));
    assert_eq!(
        run(
            &mut engine,
            "(eqv? (make-rectangular 1.0 2.0) (make-rectangular 1.0 2.0))"
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(eqv? 1+2i (make-rectangular 1.0 2.0))"),
        Value::boolean(false)
    );

    // eqv? drives memv: an inexact key must not match exact list members.
    assert_eq!(
        run(&mut engine, "(memv 5.0 '(1 2 5))"),
        Value::boolean(false)
    );
    assert_eq!(
        run(&mut engine, "(pair? (memv 5 '(1 2 5)))"),
        Value::boolean(true)
    );
}

#[test]
fn eqv_identifies_values_beyond_the_identity_fast_path() {
    // Guards the bit-identity fast prefix in eqv_value: values that are NOT
    // bit-identical tagged words but are still eqv? must keep reaching the
    // numeric tower, and unequal wide fixnums must stay distinct.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // Two separately computed heap-backed exact integers are distinct objects
    // with the same value.
    assert_eq!(
        run(&mut engine, "(eqv? (expt 2 100) (expt 2 100))"),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(eqv? (expt 2 100) (+ (expt 2 100) 1))"),
        Value::boolean(false)
    );

    // Full-width inline fixnums compare by value.
    assert_eq!(
        run(&mut engine, "(eqv? 9223372036854775807 (- (expt 2 63) 1))"),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(eqv? 9223372036854775807 9223372036854775806)"
        ),
        Value::boolean(false)
    );

    // A fixnum against a non-number with different bits stays #f, and assv
    // keyed by a fixnum still finds its entry.
    assert_eq!(run(&mut engine, "(eqv? 1 'a)"), Value::boolean(false));
    assert_eq!(
        run(
            &mut engine,
            "(cdr (assv 4 (list (cons 1 10) (cons 2 20) (cons 4 40))))"
        ),
        Value::integer(40)
    );
}

#[test]
fn exact_rationals_and_complex_literals_execute() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(run(&mut engine, "(= (/ 3 2) 3/2)"), Value::boolean(true));
    assert_eq!(
        run(&mut engine, "(= (+ 1/3 1/6) 1/2)"),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(= (* 1+2i 1-2i) 5)"),
        Value::boolean(true)
    );
    assert_eq!(run(&mut engine, "(real? 3+0i)"), Value::boolean(true));
    assert_eq!(
        run(&mut engine, "(= 9007199254740992.0 9007199254740993)"),
        Value::boolean(false)
    );
}

#[test]
fn complex_infinities_have_readable_external_form() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(
            &mut engine,
            "(let ((port (open-output-string))) (write (make-rectangular +inf.0 +inf.0) port) (string=? (get-output-string port) \"+inf.0+inf.0i\"))",
        ),
        Value::boolean(true)
    );
}

#[test]
fn numeric_input_output_and_components_round_trip() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(&mut engine, "(= (string->number \"ff\" 16) 255)"),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(string=? (number->string 255 16) \"ff\")"),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(= (imag-part 1+2i) 2)"),
        Value::boolean(true)
    );
    assert_eq!(run(&mut engine, "(finite? +inf.0)"), Value::boolean(false));
    assert_eq!(
        run(&mut engine, "(= (floor-quotient -5 2) -3)",),
        Value::boolean(true)
    );
}

#[test]
fn i128_integer_boundaries_parse_print_and_calculate_exactly() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    for (radix, digits) in [
        (2, format!("{:b}", i128::MAX)),
        (8, format!("{:o}", i128::MAX)),
        (16, format!("{:x}", i128::MAX)),
    ] {
        let source = format!(
            "(= (string->number \"{digits}\" {radix}) 170141183460469231731687303715884105727)"
        );
        assert_eq!(run(&mut engine, &source), Value::boolean(true));
    }
    assert_eq!(
        run(
            &mut engine,
            "(and (string=? (number->string 170141183460469231731687303715884105727) \"170141183460469231731687303715884105727\") (string=? (number->string -170141183460469231731687303715884105728 16) \"-80000000000000000000000000000000\"))"
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(= (+ 18446744073709551615 1) 18446744073709551616)"
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(and (= #x-80000000000000000000000000000000 -170141183460469231731687303715884105728) (= (truncate-quotient -170141183460469231731687303715884105728 2) -85070591730234615865843651857942052864) (= (gcd 18446744073709551615 4294967295) 4294967295) (= (lcm 4294967296 4294967295) 18446744069414584320))"
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(= (* 18446744073709551615 2/3) 12297829382473034410)"
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(let-values (((root remainder) (exact-integer-sqrt (expt 2 121)))) (= (+ (* root root) remainder) (expt 2 121)))"
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(and (> 170141183460469231731687303715884105727 9223372036854775807/2) (< -170141183460469231731687303715884105728 -9223372036854775807/2))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn i128_overflow_and_narrow_rational_results_are_rejected() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    for source in [
        "(+ 170141183460469231731687303715884105727 1)",
        "(abs -170141183460469231731687303715884105728)",
        "(truncate-quotient -170141183460469231731687303715884105728 -1)",
        "(/ 18446744073709551615 2)",
    ] {
        let module = engine.compile("i128-overflow.scm", source).unwrap();
        assert_eq!(
            engine.eval(&module).unwrap_err().kind(),
            ErrorKind::ImplementationRestriction
        );
    }
    for source in [
        "170141183460469231731687303715884105728",
        "-170141183460469231731687303715884105729",
        "#x80000000000000000000000000000000",
    ] {
        assert!(engine.compile("i128-literal-overflow.scm", source).is_err());
    }
}

#[test]
fn exact_rational_rounding_does_not_lose_low_bits() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(
            &mut engine,
            r#"
            (and (= (floor 9223372036854775807/2) 4611686018427387903)
                 (= (ceiling -9223372036854775807/2) -4611686018427387903)
                 (= (truncate 9223372036854775807/2) 4611686018427387903)
                 (= (truncate -9223372036854775807/2) -4611686018427387903)
                 (= (round 9223372036854775807/2) 4611686018427387904)
                 (= (round -9223372036854775807/2) -4611686018427387904))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn min_and_max_propagate_inexactness_from_every_argument() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(
            &mut engine,
            "(and (inexact? (max 1.0 2))
                  (= (max 1.0 2) 2.0)
                  (inexact? (min 1 2.0))
                  (= (min 1 2.0) 1.0))",
        ),
        Value::boolean(true)
    );
}

#[test]
fn float_fast_path_arithmetic_and_comparisons_match_r7rs_semantics() {
    // Guards the monomorphic f64 fast path in `register_numeric`: when both
    // operands are inline inexact reals the VM shortcuts the general numeric
    // dispatch, and its results must stay bit-for-bit equivalent, including
    // NaN comparisons yielding #f and mixed exact/inexact operands (which
    // deliberately bypass the fast path) staying correct.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // Arithmetic on two floats (fast path).
    assert_eq!(
        run(&mut engine, "(= (+ 2.0 3.0) 5.0)"),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(= (- 5.0 2.0) 3.0)"),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(= (* 2.0 3.0) 6.0)"),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(= (/ 7.0 2.0) 3.5)"),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(infinite? (/ 1.0 0.0))"),
        Value::boolean(true)
    );

    // All six comparisons on two floats (fast path).
    assert_eq!(run(&mut engine, "(< 1.0 2.0)"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(< 2.0 1.0)"), Value::boolean(false));
    assert_eq!(run(&mut engine, "(<= 2.0 2.0)"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(> 3.0 2.0)"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(>= 2.0 2.0)"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(= 2.0 2.0)"), Value::boolean(true));

    // NaN compares false under every operator (fast path must fall through to
    // the same answer as the general native, not fabricate an ordering).
    for op in ["<", "<=", ">", ">=", "="] {
        assert_eq!(
            run(&mut engine, &format!("({op} +nan.0 1.0)")),
            Value::boolean(false),
            "NaN comparison with {op} must be #f"
        );
        assert_eq!(
            run(&mut engine, &format!("({op} 1.0 +nan.0)")),
            Value::boolean(false),
            "NaN comparison with {op} must be #f"
        );
    }
    assert_eq!(run(&mut engine, "(= +nan.0 +nan.0)"), Value::boolean(false));

    // Mixed exact/inexact operands bypass the f64 fast path but stay correct.
    assert_eq!(run(&mut engine, "(= (+ 2 3.0) 5.0)"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(< 2 3.0)"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(< 2.0 3)"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(= 5 5.0)"), Value::boolean(true));
}

#[test]
fn character_and_string_case_operations_are_available() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(&mut engine, "(import (scheme char)) (char-ci=? #\\A #\\a)"),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(import (scheme char)) (digit-value #\\x0664)"),
        Value::integer(4)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (scheme char)) (string-ci=? \"Straße\" \"STRASSE\")"
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(char->integer #\\λ)"),
        Value::integer(0x3BB)
    );
}

#[test]
fn heap_number_literals_work_in_every_operand_position() {
    // Literals with no inline representation (rationals, beyond-i64 exact
    // integers) materialize through a cold `LoadNumber` instruction instead of
    // the hot constant table, so every syntactic position that used to fold
    // them as inline constants must still compute correctly.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // Plain literal load.
    assert_eq!(
        run(&mut engine, "(= 18446744073709551616 (expt 2 64))"),
        Value::boolean(true)
    );
    // Rational literal as the right operand of fast-path arithmetic.
    assert_eq!(
        run(&mut engine, "(let ((x 1/2)) (= (+ x 1/2) 1))"),
        Value::boolean(true)
    );
    // Beyond-i64 literal as the right operand (previously an RK constant).
    assert_eq!(
        run(
            &mut engine,
            "(let ((x (expt 2 64))) (= (+ x 18446744073709551616) (expt 2 65)))"
        ),
        Value::boolean(true)
    );
    // Fused comparison against a rational literal in boolean context.
    assert_eq!(
        run(&mut engine, "(let ((x 1/4)) (if (< x 1/2) 'yes 'no))"),
        run(&mut engine, "'yes")
    );
    // Rational literal inside an n-ary fold step.
    assert_eq!(run(&mut engine, "(= (* 2 3 1/3) 2)"), Value::boolean(true));
    // A heap literal in a loop body: the cold load runs on every iteration.
    assert_eq!(
        run(
            &mut engine,
            "(let loop ((i 0) (acc 0))
               (if (= i 8) (= acc 4) (loop (+ i 1) (+ acc 1/2))))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn numeric_fast_path_reordering_preserves_the_tower_semantics() {
    // The dispatch fast path tests the all-fixnum shape before the all-float
    // shape; every mixed, overflowing, NaN, and non-numeric case must still
    // reach the identical general tower.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // Mixed fixnum/float promotes to float, both operand orders.
    assert_eq!(run(&mut engine, "(+ 1 2.5)"), Value::float(3.5));
    assert_eq!(run(&mut engine, "(+ 2.5 1)"), Value::float(3.5));
    assert_eq!(run(&mut engine, "(< 1 1.5)"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(< 1.5 2)"), Value::boolean(true));

    // NaN comparisons are false, including self-comparison.
    assert_eq!(run(&mut engine, "(= +nan.0 +nan.0)"), Value::boolean(false));
    assert_eq!(run(&mut engine, "(< +nan.0 1.0)"), Value::boolean(false));

    // Fixnum overflow spills to the heap-backed exact path, staying exact.
    assert_eq!(
        run(
            &mut engine,
            "(= (+ 9223372036854775807 1) 9223372036854775808)"
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(exact? (* 9223372036854775807 2))"),
        Value::boolean(true)
    );

    // Division keeps its exactness rules (never on the inline fixnum path).
    assert_eq!(run(&mut engine, "(/ 6 3)"), Value::integer(2));
    assert_eq!(run(&mut engine, "(= (/ 1 2) 1/2)"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(/ 1.0 2)"), Value::float(0.5));

    // Non-numbers raise a type error through the standalone compare and the
    // fused test-and-branch shape alike.
    for source in ["(+ 'a 1)", "(if (< 'a 1) 1 2)"] {
        let module = engine.compile("numbers_and_unicode.scm", source).unwrap();
        assert!(engine.eval(&module).is_err(), "{source} should raise");
    }

    // The 2^53 exact-comparison precision gate is untouched: an i64 just past
    // the float's exact range must not compare equal to the nearest double.
    assert_eq!(
        run(&mut engine, "(= 9007199254740993 9007199254740992.0)"),
        Value::boolean(false)
    );
}

#[test]
fn nan_identity_printing_and_predicates_are_bit_pattern_independent() {
    // Pins the NaN contract across the arithmetic fast paths: `eqv?` (and
    // everything funneling through it - `equal?`, `memv`, `case`) treats any
    // two NaNs as identical, every NaN prints as `+nan.0` regardless of sign
    // or payload bits, and `nan?` recognizes NaNs from every producing
    // operation. Runtime float results skip per-operation canonicalization,
    // so these observation sites carry the normalization instead.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    for (source, expected) in [
        ("(let ((x (/ 0. 0.))) (eqv? x x))", "#t"),
        ("(eqv? (/ 0. 0.) (- (/ 0. 0.)))", "#t"),
        ("(eqv? (/ 0. 0.) (* +inf.0 0.))", "#t"),
        ("(eqv? +nan.0 (- +nan.0))", "#t"),
        (
            "(equal? (vector (/ 0. 0.)) (vector (* -1. (/ 0. 0.))))",
            "#t",
        ),
        (
            "(if (memv (/ 0. 0.) (list 1. (- (/ 0. 0.)))) 'hit 'miss)",
            "hit",
        ),
        ("(case (* +inf.0 0.) ((+nan.0) 'hit) (else 'miss))", "hit"),
        ("(number->string (/ 0. 0.))", "\"+nan.0\""),
        ("(number->string (- (/ 0. 0.)))", "\"+nan.0\""),
        ("(number->string (* -1. (/ 0. 0.)))", "\"+nan.0\""),
        ("(nan? (* +inf.0 0.))", "#t"),
        ("(nan? (- (/ 0. 0.)))", "#t"),
        ("(finite? (/ 0. 0.))", "#f"),
        ("(= (/ 0. 0.) (/ 0. 0.))", "#f"),
        ("(< (/ 0. 0.) 1.0)", "#f"),
    ] {
        let module = engine.compile("nan.scm", source).unwrap();
        let root = engine.eval(&module).unwrap().into_one().unwrap();
        assert_eq!(
            engine.write_root(&root).unwrap(),
            expected,
            "source: {source}"
        );
    }
}

#[test]
fn string_append_concatenates_and_always_returns_a_fresh_mutable_string() {
    // Guards the native string-append that replaced the prelude definition.
    // R7RS requires a newly allocated string in every case, including the
    // zero-argument and one-argument forms.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    assert_eq!(
        run(
            &mut engine,
            "(string=? (string-append \"foo\" \"bar\" \"baz\") \"foobarbaz\")"
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(string=? (string-append) \"\")"),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(string-length (string-append \"ab\" \"\" \"c\"))"
        ),
        Value::integer(3)
    );

    // One-argument form returns a fresh copy, independent of the input.
    assert_eq!(
        run(
            &mut engine,
            "(let* ((s (string #\\a #\\b)) (t (string-append s)))
               (string-set! t 0 #\\z)
               (and (string=? s \"ab\") (string=? t \"zb\")))"
        ),
        Value::boolean(true)
    );

    // Results built from literal operands are mutable.
    assert_eq!(
        run(
            &mut engine,
            "(let ((t (string-append \"ab\" \"cd\")))
               (string-set! t 3 #\\z)
               (string=? t \"abcz\"))"
        ),
        Value::boolean(true)
    );

    // Non-string operands raise the canonical type error. The prelude version
    // surfaced an incidental string-length error instead, so this message is
    // asserted deliberately for the native.
    let module = engine
        .compile("string-append-type.scm", "(string-append \"a\" 5)")
        .unwrap();
    let error = engine.eval(&module).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::TypeError);
}

#[test]
fn width_changing_string_set_preserves_contents_and_indexing() {
    // Guards the UTF-8 string backing: string-set! with a replacement of a
    // different UTF-8 width shifts the tail, and indexed access before, at,
    // and after the mutation point must stay correct in both directions.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // Widening: ASCII -> 2-byte -> astral, length is unchanged throughout.
    assert_eq!(
        run(
            &mut engine,
            "(let ((s (string #\\a #\\b #\\c #\\d #\\e)))
               (string-set! s 2 #\\λ)
               (string-set! s 2 #\\x1F700)
               (and (= (string-length s) 5)
                    (char=? (string-ref s 0) #\\a)
                    (char=? (string-ref s 1) #\\b)
                    (char=? (string-ref s 2) #\\x1F700)
                    (char=? (string-ref s 3) #\\d)
                    (char=? (string-ref s 4) #\\e)))"
        ),
        Value::boolean(true)
    );

    // Narrowing: a wide char replaced by ASCII heals the string back to a
    // pure byte-indexed form, and the tail shifts left correctly.
    assert_eq!(
        run(
            &mut engine,
            "(let ((s (string #\\a #\\λ #\\x1F700 #\\d)))
               (string-set! s 1 #\\b)
               (string-set! s 2 #\\c)
               (and (= (string-length s) 4)
                    (string=? s \"abcd\")))"
        ),
        Value::boolean(true)
    );

    // Round-trip through string->list after mixed-width mutations.
    assert_eq!(
        run(
            &mut engine,
            "(let ((s (make-string 4 #\\x)))
               (string-set! s 0 #\\α)
               (string-set! s 3 #\\x10000)
               (equal? (string->list s) (list #\\α #\\x #\\x #\\x10000)))"
        ),
        Value::boolean(true)
    );

    // Out-of-range and immutability behavior is unchanged by width handling.
    let module = engine
        .compile(
            "string-set-range.scm",
            "(let ((s (string #\\a #\\λ))) (string-set! s 2 #\\b))",
        )
        .unwrap();
    let error = engine.eval(&module).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::RangeError);
}

#[test]
fn multibyte_strings_support_indexed_loops_in_both_directions() {
    // Guards the heap-side access cursors: ascending loops, descending loops
    // (string->list walks indices downward), far-apart index jumps, and loops
    // resumed after a width-changing mutation must all read correct chars.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // Ascending accumulation over a multibyte string.
    assert_eq!(
        run(
            &mut engine,
            "(let ((s (make-string 100 #\\λ)))
               (let loop ((i 0) (acc 0))
                 (if (= i (string-length s))
                     acc
                     (loop (+ i 1) (+ acc (char->integer (string-ref s i)))))))"
        ),
        Value::integer(100 * i64::from(0x03BB))
    );

    // Descending walk via string->list on mixed-width contents.
    assert_eq!(
        run(
            &mut engine,
            "(let* ((s (string #\\a #\\λ #\\x1F700 #\\z))
                    (l (string->list s)))
               (and (char=? (car l) #\\a)
                    (char=? (cadr l) #\\λ)
                    (char=? (caddr l) #\\x1F700)
                    (char=? (cadddr l) #\\z)))"
        ),
        Value::boolean(true)
    );

    // Far-apart alternating indices defeat the cursor and must fall back to
    // the nearest end anchor without losing correctness.
    assert_eq!(
        run(
            &mut engine,
            "(let ((s (string-append (make-string 50 #\\α) (make-string 50 #\\ω))))
               (let loop ((i 0) (ok #t))
                 (if (= i 50)
                     ok
                     (loop (+ i 1)
                           (and ok
                                (char=? (string-ref s i) #\\α)
                                (char=? (string-ref s (- 99 i)) #\\ω))))))"
        ),
        Value::boolean(true)
    );

    // A width-changing mutation mid-loop must not leave a stale cursor:
    // reads after the write see the shifted tail.
    assert_eq!(
        run(
            &mut engine,
            "(let ((s (string #\\α #\\β #\\γ #\\δ #\\ε)))
               (let loop ((i 0) (ok #t))
                 (if (= i (string-length s))
                     ok
                     (begin
                       (when (= i 2) (string-set! s 3 #\\d))
                       (loop (+ i 1)
                             (and ok (char=? (string-ref s i)
                                             (case i
                                               ((0) #\\α) ((1) #\\β) ((2) #\\γ)
                                               ((3) #\\d) ((4) #\\ε)
                                               (else #\\?)))))))))"
        ),
        Value::boolean(true)
    );

    // string-copy! (prelude ref+set loop) over two multibyte strings, plus
    // an overlapping same-string copy.
    assert_eq!(
        run(
            &mut engine,
            "(let ((from (string #\\λ #\\μ #\\ν))
                   (to (make-string 5 #\\x1F700)))
               (string-copy! to 1 from)
               (and (char=? (string-ref to 0) #\\x1F700)
                    (char=? (string-ref to 1) #\\λ)
                    (char=? (string-ref to 2) #\\μ)
                    (char=? (string-ref to 3) #\\ν)
                    (char=? (string-ref to 4) #\\x1F700)))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn string_char_ranges_map_to_correct_utf8_slices() {
    // Guards the char-index to byte-range conversion used by string->utf8
    // and write-string, the two sites where a char/byte confusion could hide.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // string->utf8 bounds are char indices, not byte offsets.
    assert_eq!(
        run(
            &mut engine,
            "(bytevector-length (string->utf8 (string #\\a #\\λ #\\x1F700) 1 3))"
        ),
        Value::integer(2 + 4)
    );
    assert_eq!(
        run(
            &mut engine,
            "(equal? (string->utf8 (string #\\α #\\b) 1) (bytevector 98))"
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(string=? (utf8->string (string->utf8 \"αβγ\" 1 2)) \"β\")"
        ),
        Value::boolean(true)
    );

    // Range errors are still char-counted.
    let module = engine
        .compile(
            "string-utf8-range.scm",
            "(string->utf8 (string #\\λ #\\μ) 1 3)",
        )
        .unwrap();
    let error = engine.eval(&module).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::RangeError);

    // write-string honors char-index start/end on multibyte contents.
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-output-string)))
               (write-string (string #\\a #\\λ #\\x1F700 #\\z) p 1 3)
               (string=? (get-output-string p) (string #\\λ #\\x1F700)))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn mixed_width_strings_compare_by_code_point() {
    // Guards string comparison and equal? on the UTF-8 backing: byte-wise
    // UTF-8 ordering equals code-point ordering, and equality must be
    // content-based across strings whose byte lengths differ from their
    // char counts.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    assert_eq!(
        run(
            &mut engine,
            "(and (equal? (string #\\a #\\λ) (string #\\a #\\λ))
                  (not (equal? (string #\\a #\\λ) (string #\\a #\\μ)))
                  (string=? \"αβ\" \"αβ\")
                  (string<? \"abc\" \"αβ\")
                  (string<? \"z\" \"α\" \"\\x1F700;\")
                  (not (string<? \"αβ\" \"αα\")))"
        ),
        Value::boolean(true)
    );

    // A string mutated to equal another compares equal afterward.
    assert_eq!(
        run(
            &mut engine,
            "(let ((s (string #\\a #\\b)))
               (string-set! s 0 #\\λ)
               (equal? s (string #\\λ #\\b)))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn write_control_character_escapes_round_trip() {
    // Regression test for the printer writing control-character escapes with
    // a decimal code point after the `\x` marker instead of the hexadecimal
    // one R7RS specifies, which made `write` output read back as a different
    // value.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // A string holding ESC (U+001B) survives a write/read round trip.
    assert_eq!(
        run(
            &mut engine,
            "(let* ((s (string #\\x1b))
                    (p (open-output-string)))
               (write s p)
               (equal? (read (open-input-string (get-output-string p))) s))"
        ),
        Value::boolean(true)
    );

    // A character constant survives a write/read round trip.
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-output-string)))
               (write #\\x1b p)
               (eqv? (read (open-input-string (get-output-string p))) #\\x1b))"
        ),
        Value::boolean(true)
    );

    // A pipe-quoted symbol holding ESC survives a write/read round trip.
    assert_eq!(
        run(
            &mut engine,
            "(let* ((sym (string->symbol (string #\\x1b)))
                    (p (open-output-string)))
               (write sym p)
               (eq? (read (open-input-string (get-output-string p))) sym))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn mixed_fixnum_float_arithmetic_matches_the_general_path() {
    // Guards the inline mixed fixnum/float arms in numeric_fast: results and
    // comparison outcomes must be identical to the out-of-line tower,
    // including the 2^53 comparison guard and the excluded division shape.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    assert_eq!(run(&mut engine, "(* 0.5 3)"), Value::float(1.5));
    assert_eq!(run(&mut engine, "(* 3 0.5)"), Value::float(1.5));
    assert_eq!(run(&mut engine, "(+ 1 2.5)"), Value::float(3.5));
    assert_eq!(run(&mut engine, "(- 2.5 1)"), Value::float(1.5));
    assert_eq!(run(&mut engine, "(/ 1 2.0)"), Value::float(0.5));
    assert_eq!(run(&mut engine, "(/ 3.0 2)"), Value::float(1.5));

    // Comparisons around the exact f64 range: 2^53 + 1 is not representable,
    // so the fast path must defer and the tower must still answer correctly.
    assert_eq!(
        run(&mut engine, "(< 9007199254740993 9007199254740992.0)"),
        Value::boolean(false)
    );
    assert_eq!(
        run(&mut engine, "(> 9007199254740993 9007199254740992.0)"),
        Value::boolean(true)
    );
    assert_eq!(
        run(&mut engine, "(= 9223372036854775807 9223372036854775807.0)"),
        Value::boolean(false)
    );
    assert_eq!(run(&mut engine, "(< 3 3.5)"), Value::boolean(true));
    assert_eq!(run(&mut engine, "(<= 4.0 4)"), Value::boolean(true));

    // NaN comparisons stay false in both operand orders.
    assert_eq!(run(&mut engine, "(< 1 +nan.0)"), Value::boolean(false));
    assert_eq!(run(&mut engine, "(> +nan.0 1)"), Value::boolean(false));
}
