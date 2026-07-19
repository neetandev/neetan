//! Usage examples for the SRFI 27 (Sources of Random Bits) extension. The
//! examples import it through the `(r7rs random-bits)` alias. The canonical
//! `(srfi 27)` name provides the identical library.

use r7rs::{Engine, EngineConfig, Extension, Value};

/// Builds an engine with SRFI 27 installed, the usual first step for any script
/// that needs randomness.
fn engine_with_srfi27() -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Srfi27).unwrap();
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
fn enabling_the_extension_and_drawing() {
    let mut engine = engine_with_srfi27();
    // `random-integer` draws from the default source and stays in `[0, n)`.
    let all_in_range = run(
        &mut engine,
        r#"
        (import (r7rs random-bits))
        (let loop ((i 0) (ok #t))
          (if (= i 10000)
              ok
              (let ((x (random-integer 6)))
                (loop (+ i 1) (and ok (<= 0 x) (< x 6))))))
        "#,
    );
    assert_eq!(all_in_range, Value::boolean(true));

    // `random-real` draws a flonum strictly inside the open interval `(0, 1)`.
    let real_in_unit = run(
        &mut engine,
        r#"
        (import (r7rs random-bits))
        (let loop ((i 0) (ok #t))
          (if (= i 10000)
              ok
              (let ((x (random-real)))
                (loop (+ i 1) (and ok (< 0 x) (< x 1))))))
        "#,
    );
    assert_eq!(real_in_unit, Value::boolean(true));
}

#[test]
fn reproducible_streams_from_a_seed() {
    let mut engine = engine_with_srfi27();
    // Two sources built with the same integer seed replay the same stream. This
    // is how an emulator run can be made deterministic.
    let identical = run(
        &mut engine,
        r#"
        (import (r7rs random-bits))
        (define a (random-source-make-integers (make-random-source 42)))
        (define b (random-source-make-integers (make-random-source 42)))
        (let loop ((i 0) (ok #t))
          (if (= i 1000)
              ok
              (loop (+ i 1) (and ok (= (a 1000000000) (b 1000000000))))))
        "#,
    );
    assert_eq!(identical, Value::boolean(true));

    // Reseeding a source with an explicit seed is likewise deterministic.
    let reseed_matches = run(
        &mut engine,
        r#"
        (import (r7rs random-bits))
        (define s (make-random-source))
        (random-source-randomize! s 7)
        (define g (random-source-make-integers s))
        (define first (g 1000000000))
        (random-source-randomize! s 7)
        (= first (g 1000000000))
        "#,
    );
    assert_eq!(reseed_matches, Value::boolean(true));
}

#[test]
fn negative_seeds_and_stream_indices_address_the_upper_unsigned_range() {
    let mut engine = engine_with_srfi27();
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (srfi 27) (scheme base))
            (define minimum -170141183460469231731687303715884105728)
            (define maximum 170141183460469231731687303715884105727)
            (define seed-a (make-random-source -1))
            (define seed-b (make-random-source -1))
            (define seed-c (make-random-source maximum))
            (define stream-a (make-random-source))
            (define stream-b (make-random-source))
            (define stream-c (make-random-source))
            (random-source-pseudo-randomize! stream-a -1 minimum)
            (random-source-pseudo-randomize! stream-b -1 minimum)
            (random-source-pseudo-randomize! stream-c maximum minimum)
            (and
              (equal? (random-source-state-ref seed-a)
                      (random-source-state-ref seed-b))
              (not (equal? (random-source-state-ref seed-a)
                           (random-source-state-ref seed-c)))
              (equal? (random-source-state-ref stream-a)
                      (random-source-state-ref stream-b))
              (not (equal? (random-source-state-ref stream-a)
                           (random-source-state-ref stream-c))))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn saving_and_restoring_a_source() {
    let mut engine = engine_with_srfi27();
    // A source's state can be captured, then restored to replay from that point.
    let replays = run(
        &mut engine,
        r#"
        (import (r7rs random-bits))
        (define s (make-random-source 7))
        (define g (random-source-make-integers s))
        (g 1000000000)
        (define snapshot (random-source-state-ref s))
        (define expected (g 1000000000))
        (random-source-state-set! s snapshot)
        (= expected (g 1000000000))
        "#,
    );
    assert_eq!(replays, Value::boolean(true));

    // The captured state has a printable external representation.
    let printed = show(
        &mut engine,
        r#"
        (import (r7rs random-bits))
        (random-source-state-ref (make-random-source 1))
        "#,
    );
    assert!(
        printed.starts_with("(squares-state "),
        "unexpected state representation: {printed}"
    );
}

#[test]
fn independent_streams_via_pseudo_randomize() {
    let mut engine = engine_with_srfi27();
    // The same (i, j) indices always select the same stream.
    let same_index_matches = run(
        &mut engine,
        r#"
        (import (r7rs random-bits))
        (define s1 (make-random-source))
        (define s2 (make-random-source))
        (random-source-pseudo-randomize! s1 3 5)
        (random-source-pseudo-randomize! s2 3 5)
        (define g1 (random-source-make-integers s1))
        (define g2 (random-source-make-integers s2))
        (= (g1 1000000000) (g2 1000000000))
        "#,
    );
    assert_eq!(same_index_matches, Value::boolean(true));

    // Different indices select different streams.
    let different_index_diverges = run(
        &mut engine,
        r#"
        (import (r7rs random-bits))
        (define s1 (make-random-source))
        (define s2 (make-random-source))
        (random-source-pseudo-randomize! s1 3 5)
        (random-source-pseudo-randomize! s2 3 6)
        (define g1 (random-source-make-integers s1))
        (define g2 (random-source-make-integers s2))
        (not (= (g1 1000000000) (g2 1000000000)))
        "#,
    );
    assert_eq!(different_index_diverges, Value::boolean(true));
}

#[test]
fn generators_from_a_source() {
    let mut engine = engine_with_srfi27();
    // `random-source-make-integers` returns a procedure of one argument.
    let integers_in_range = run(
        &mut engine,
        r#"
        (import (r7rs random-bits))
        (define g (random-source-make-integers (make-random-source 99)))
        (let loop ((i 0) (ok #t))
          (if (= i 10000)
              ok
              (let ((x (g 100)))
                (loop (+ i 1) (and ok (<= 0 x) (< x 100))))))
        "#,
    );
    assert_eq!(integers_in_range, Value::boolean(true));

    // `random-source-make-reals` with no unit returns reals in `(0, 1)`.
    let reals_in_unit = run(
        &mut engine,
        r#"
        (import (r7rs random-bits))
        (define g (random-source-make-reals (make-random-source 99)))
        (let loop ((i 0) (ok #t))
          (if (= i 10000)
              ok
              (let ((x (g)))
                (loop (+ i 1) (and ok (< 0 x) (< x 1))))))
        "#,
    );
    assert_eq!(reals_in_unit, Value::boolean(true));

    // With an exact unit the results are exact multiples of that unit.
    let quantized = run(
        &mut engine,
        r#"
        (import (r7rs random-bits))
        (define g (random-source-make-reals (make-random-source 5) 1/4))
        (let loop ((i 0) (ok #t))
          (if (= i 10000)
              ok
              (let ((x (g)))
                (loop (+ i 1)
                      (and ok (memv x '(1/4 1/2 3/4)) #t)))))
        "#,
    );
    assert_eq!(quantized, Value::boolean(true));
}

#[test]
fn detecting_availability_from_scheme() {
    let mut engine = engine_with_srfi27();
    // After install, guest code can detect the extension by feature.
    assert_eq!(
        run(
            &mut engine,
            "(cond-expand (srfi-27 'present) (else 'absent))"
        ),
        run(&mut engine, "'present")
    );
    let features_list = show(&mut engine, "(features)");
    assert!(
        features_list.contains("srfi-27"),
        "features should list srfi-27: {features_list}"
    );

    // A fresh engine without the extension cannot import it and reports it
    // absent through cond-expand.
    let mut bare = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(&mut bare, "(cond-expand (srfi-27 'present) (else 'absent))"),
        run(&mut bare, "'absent")
    );
    for program in ["(import (srfi 27)) 1", "(import (r7rs random-bits)) 1"] {
        assert!(
            bare.compile("program.scm", program).is_err(),
            "{program} without the extension should fail"
        );
    }
}

#[test]
fn installing_all_extensions() {
    // Embedders can enable everything this build offers in one loop.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    for extension in Extension::ALL {
        engine.install_extension(*extension).unwrap();
    }
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs random-bits)) (random-integer 1)"
        ),
        Value::integer(0)
    );
}
