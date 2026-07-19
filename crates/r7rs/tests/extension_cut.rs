//! Usage examples for the SRFI 26 (`cut`/`cute`) extension. The examples import
//! it through the `(r7rs cut)` alias. The canonical `(srfi 26)` name provides the
//! identical library.
//!
//! `cut` and `cute` are a compact notation for specializing (partially applying)
//! some of a procedure's arguments without writing a `lambda`. In a `cut` form
//! each `<>` marks a slot that becomes a formal of the resulting procedure, and
//! everything else is passed through in place. A trailing `<...>` (the rest-slot)
//! forwards any extra arguments. `cute` is `cut` with evaluated non-slots: it
//! evaluates the non-slot expressions once when the procedure is built rather
//! than on every call.

use r7rs::{Engine, EngineConfig, Extension, Value};

/// Builds an engine with SRFI 26 installed, the usual first step for any script
/// that needs `cut` or `cute`.
fn engine_with_srfi26() -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Srfi26).unwrap();
    engine
}

/// Evaluates a program and returns its single result value.
fn run(engine: &mut Engine, source: &str) -> Value {
    let module = engine.compile("program.scm", source).unwrap();
    engine.eval(&module).unwrap().into_one().unwrap().value()
}

/// Evaluates a program and returns the external representation of its result.
fn show(engine: &mut Engine, source: &str) -> String {
    let module = engine.compile("program.scm", source).unwrap();
    let root = engine.eval(&module).unwrap().into_one().unwrap();
    engine.write_root(&root).unwrap()
}

#[test]
fn a_single_slot_becomes_the_only_formal() {
    let mut engine = engine_with_srfi26();
    // `(cut cons 1 <>)` is `(lambda (x) (cons 1 x))`: the one slot becomes the
    // one argument, and the `1` is passed through unchanged.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs cut) (scheme base))
            ((cut cons 1 <>) 2)
            "#,
        ),
        "(1 . 2)"
    );
}

#[test]
fn several_slots_become_formals_in_order() {
    let mut engine = engine_with_srfi26();
    // Each `<>` is a distinct formal, filled left to right from the call. The
    // non-slot positions stay fixed.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs cut) (scheme base))
            ((cut list 1 <> 3 <>) 2 4)
            "#,
        ),
        "(1 2 3 4)"
    );
}

#[test]
fn the_operator_position_can_be_a_slot() {
    let mut engine = engine_with_srfi26();
    // The very first `<slot-or-expr>` may itself be a slot, so the procedure to
    // call is supplied at call time.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs cut) (scheme base))
            ((cut <> 2 3) +)
            "#,
        ),
        Value::integer(5)
    );
}

#[test]
fn a_form_with_no_slots_is_a_thunk() {
    let mut engine = engine_with_srfi26();
    // With no slots the result is a zero-argument procedure that runs the call
    // when invoked.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs cut) (scheme base))
            ((cut list))
            "#,
        ),
        "()"
    );
}

#[test]
fn a_rest_slot_forwards_the_remaining_arguments() {
    let mut engine = engine_with_srfi26();
    // A trailing `<...>` makes the procedure variable arity: every extra argument
    // is forwarded to the call after the fixed and slot arguments.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs cut) (scheme base))
            ((cut list 1 <> 3 <...>) 2 4 5)
            "#,
        ),
        "(1 2 3 4 5)"
    );
}

#[test]
fn cut_is_handy_with_map() {
    let mut engine = engine_with_srfi26();
    // The idiomatic use: a slot per element the higher-order procedure supplies.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs cut) (scheme base))
            (map (cut * 2 <>) '(1 2 3))
            "#,
        ),
        "(2 4 6)"
    );
}

#[test]
fn cut_re_evaluates_non_slots_on_every_call() {
    let mut engine = engine_with_srfi26();
    // `cut` leaves the non-slot expression `(bump!)` inside the lambda body, so it
    // runs once per call. Three calls bump the counter three times.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs cut) (scheme base))
            (define calls 0)
            (define (bump!) (set! calls (+ calls 1)) 10)
            (define f (cut + (bump!) <>))
            (f 1)
            (f 1)
            (f 1)
            calls
            "#,
        ),
        Value::integer(3)
    );
}

#[test]
fn cute_evaluates_non_slots_once_at_specialization() {
    let mut engine = engine_with_srfi26();
    // `cute` binds the non-slot expression `(bump!)` in an enclosing `let`, so it
    // runs a single time when the procedure is built. Three calls do not bump it
    // again.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs cut) (scheme base))
            (define calls 0)
            (define (bump!) (set! calls (+ calls 1)) 10)
            (define g (cute + (bump!) <>))
            (g 1)
            (g 1)
            (g 1)
            calls
            "#,
        ),
        Value::integer(1)
    );
}

#[test]
fn the_extension_advertises_its_feature() {
    let mut engine = engine_with_srfi26();
    // Installing the extension enables the srfi-26 cond-expand feature.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs cut) (scheme base))
            (cond-expand (srfi-26 #t) (else #f))
            "#,
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs cut) (scheme base)) (if (memq 'srfi-26 (features)) #t #f)"
        ),
        Value::boolean(true)
    );
}

#[test]
fn a_bare_engine_cannot_import_the_extension() {
    // Without install_extension neither library name is available.
    let mut bare = Engine::new(EngineConfig::default()).unwrap();
    for program in ["(import (srfi 26)) 1", "(import (r7rs cut)) 1"] {
        assert!(
            bare.compile("program.scm", program).is_err(),
            "{program} without the extension should fail"
        );
    }
}
