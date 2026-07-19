//! Usage examples for the SRFI 69 (Basic Hash Tables) extension. The examples
//! import it through the `(r7rs basic-hash-table)` alias. The canonical `(srfi 69)`
//! name provides the identical library.

use r7rs::{Engine, EngineConfig, Extension, Value};

/// Builds an engine with SRFI 69 installed, the usual first step for any script
/// that needs hash tables.
fn engine_with_srfi69() -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Srfi69).unwrap();
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
fn setting_and_reading_back_a_value() {
    let mut engine = engine_with_srfi69();
    // A fresh table maps keys to values with hash-table-set! and hash-table-ref.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs basic-hash-table))
            (define ages (make-hash-table))
            (hash-table-set! ages 'alice 30)
            (hash-table-set! ages 'bob 41)
            (hash-table-ref ages 'bob)
            "#,
        ),
        Value::integer(41)
    );
}

#[test]
fn omitted_and_non_positive_size_hints_use_the_default_bucket_count() {
    let mut engine = engine_with_srfi69();
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs basic-hash-table) (scheme base))
            (define observed-bound #f)
            (define (observing-hash key bound)
              (set! observed-bound bound)
              0)

            (define omitted (make-hash-table equal? observing-hash))
            (hash-table-set! omitted 'first 1)
            (define omitted-bound observed-bound)

            (set! observed-bound #f)
            (define zero (make-hash-table equal? observing-hash 0))
            (hash-table-set! zero 'second 2)
            (define zero-bound observed-bound)

            (set! observed-bound #f)
            (define negative (make-hash-table equal? observing-hash -10))
            (hash-table-set! negative 'third 3)

            (list omitted-bound zero-bound observed-bound
                  (hash-table-size omitted)
                  (hash-table-size zero)
                  (hash-table-size negative))
            "#,
        ),
        "(64 64 64 1 1 1)"
    );
}

#[test]
fn a_missing_key_uses_the_default_or_the_thunk() {
    let mut engine = engine_with_srfi69();
    // hash-table-ref/default returns the default for an absent key, and
    // hash-table-ref calls the thunk when one is given.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs basic-hash-table) (scheme base))
            (define t (make-hash-table))
            (list (hash-table-ref/default t 'missing 'none)
                  (hash-table-ref t 'missing (lambda () 'computed))
                  (hash-table-exists? t 'missing))
            "#,
        ),
        "(none computed #f)"
    );
}

#[test]
fn counting_occurrences_with_update() {
    let mut engine = engine_with_srfi69();
    // hash-table-update!/default reads the current count, or the default 0, and
    // stores the incremented value back.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs basic-hash-table) (scheme base))
            (define counts (make-hash-table))
            (for-each
              (lambda (word)
                (hash-table-update!/default counts word (lambda (n) (+ n 1)) 0))
              '(a b a c a b))
            (hash-table-ref counts 'a)
            "#,
        ),
        Value::integer(3)
    );
}

#[test]
fn deleting_a_key_shrinks_the_table() {
    let mut engine = engine_with_srfi69();
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs basic-hash-table) (scheme base))
            (define t (make-hash-table))
            (hash-table-set! t 'x 1)
            (hash-table-set! t 'y 2)
            (hash-table-delete! t 'x)
            (list (hash-table-size t) (hash-table-exists? t 'x) (hash-table-exists? t 'y))
            "#,
        ),
        "(1 #f #t)"
    );
}

#[test]
fn keys_values_and_alist_round_trip() {
    let mut engine = engine_with_srfi69();
    // The order of keys and values is unspecified, so check membership and size
    // rather than a fixed order.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs basic-hash-table) (scheme base))
            (define t (make-hash-table))
            (hash-table-set! t 1 'one)
            (hash-table-set! t 2 'two)
            (hash-table-set! t 3 'three)
            (define keys (hash-table-keys t))
            (define vals (hash-table-values t))
            (and (= 3 (hash-table-size t))
                 (= 3 (length keys) (length vals) (length (hash-table->alist t)))
                 (and (memv 1 keys) (memv 2 keys) (memv 3 keys) #t)
                 (and (memq 'one vals) (memq 'two vals) (memq 'three vals) #t))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn folding_and_walking_visit_every_association() {
    let mut engine = engine_with_srfi69();
    // hash-table-fold accumulates over every association, and hash-table-walk
    // visits each one for effect.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs basic-hash-table) (scheme base))
            (define t (make-hash-table))
            (hash-table-set! t 'a 10)
            (hash-table-set! t 'b 20)
            (hash-table-set! t 'c 30)
            (define sum-of-values
              (hash-table-fold t (lambda (key value acc) (+ value acc)) 0))
            (define total 0)
            (hash-table-walk t (lambda (key value) (set! total (+ total value))))
            (and (= sum-of-values 60) (= total 60) sum-of-values)
            "#,
        ),
        Value::integer(60)
    );
}

#[test]
fn alist_to_hash_table_keeps_the_first_association() {
    let mut engine = engine_with_srfi69();
    // When a key repeats, the first association wins.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs basic-hash-table))
            (define t (alist->hash-table '((a . 1) (b . 2) (a . 99))))
            (hash-table-ref t 'a)
            "#,
        ),
        Value::integer(1)
    );
}

#[test]
fn a_string_table_uses_string_equality() {
    let mut engine = engine_with_srfi69();
    // With string=? two equal strings that are not the same object still find
    // the same association.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs basic-hash-table) (scheme base))
            (define t (make-hash-table string=?))
            (hash-table-set! t (string #\k #\e #\y) 42)
            (hash-table-ref t (string #\k #\e #\y))
            "#,
        ),
        Value::integer(42)
    );
}

#[test]
fn copy_is_independent_and_merge_combines() {
    let mut engine = engine_with_srfi69();
    // A copy does not see later mutations of the original, and merge! folds one
    // table's associations into another.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs basic-hash-table) (scheme base))
            (define a (make-hash-table))
            (hash-table-set! a 'x 1)
            (define b (hash-table-copy a))
            (hash-table-set! a 'x 100)
            (define c (make-hash-table))
            (hash-table-set! c 'y 2)
            (hash-table-merge! c a)
            (list (hash-table-ref b 'x)
                  (hash-table-ref c 'x)
                  (hash-table-ref c 'y))
            "#,
        ),
        "(1 100 2)"
    );
}

#[test]
fn reflective_queries_return_the_installed_procedures() {
    let mut engine = engine_with_srfi69();
    // The equivalence and hash functions a table was made with are recoverable.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs basic-hash-table) (scheme base))
            (define t (make-hash-table eq?))
            (and (eq? (hash-table-equivalence-function t) eq?)
                 (procedure? (hash-table-hash-function t)))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn the_hash_functions_stay_within_a_bound() {
    let mut engine = engine_with_srfi69();
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs basic-hash-table) (scheme base))
            (define (in-range? n) (and (exact-integer? n) (<= 0 n) (< n 64)))
            (and (in-range? (hash '(a b c) 64))
                 (in-range? (string-hash "text" 64))
                 (in-range? (string-ci-hash "TEXT" 64))
                 (in-range? (hash-by-identity 'symbol 64)))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn the_extension_advertises_its_feature() {
    let mut engine = engine_with_srfi69();
    // Installing the extension enables the srfi-69 cond-expand feature.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs basic-hash-table) (scheme base))
            (cond-expand (srfi-69 #t) (else #f))
            "#,
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs basic-hash-table) (scheme base)) (if (memq 'srfi-69 (features)) #t #f)"
        ),
        Value::boolean(true)
    );
}

#[test]
fn a_bare_engine_cannot_import_the_extension() {
    // Without install_extension neither library name is available.
    let mut bare = Engine::new(EngineConfig::default()).unwrap();
    for program in ["(import (srfi 69)) 1", "(import (r7rs basic-hash-table)) 1"] {
        assert!(
            bare.compile("program.scm", program).is_err(),
            "{program} without the extension should fail"
        );
    }
}
