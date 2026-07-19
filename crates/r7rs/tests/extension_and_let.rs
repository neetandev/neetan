//! Usage examples for the SRFI 2 (`AND-LET*`) extension. The examples import it
//! through the `(r7rs and-let*)` alias. The canonical `(srfi 2)` name provides
//! the identical library.
//!
//! `and-let*` is a short-circuiting `and` whose clauses can bind their non-`#f`
//! results for use in later clauses and the body. A clause is one of
//! `(variable expression)` to bind, `(expression)` to test without binding, or a
//! bare `bound-variable` to test an existing binding. The first `#f` result
//! stops evaluation and makes the whole form `#f`.

use r7rs::{Engine, EngineConfig, Extension, Value};

/// Builds an engine with SRFI 2 installed, the usual first step for any script
/// that needs `and-let*`.
fn engine_with_srfi2() -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Srfi2).unwrap();
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
fn later_clauses_see_the_earlier_bindings() {
    let mut engine = engine_with_srfi2();
    // Each (variable expression) clause binds its value for the rest of the
    // clauses and the body, exactly like let*.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs and-let*) (scheme base))
            (and-let* ((x 1) (y (+ x 1))) (+ x y))
            "#,
        ),
        Value::integer(3)
    );
}

#[test]
fn a_false_clause_short_circuits_the_whole_form() {
    let mut engine = engine_with_srfi2();
    // The first clause that yields #f stops evaluation and makes the form #f,
    // whether it binds a variable or is a bare test.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs and-let*) (scheme base))
            (and (eq? #f (and-let* ((x #f) (y 2)) (+ x y)))
                 (eq? #f (and-let* ((x 1) ((> x 10))) x)))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn a_test_only_clause_guards_without_binding() {
    let mut engine = engine_with_srfi2();
    // The classic use: bind a lookup, then guard on a property of it before the
    // body runs. A (expression) clause tests without introducing a name.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs and-let*) (scheme base))
            (define (look-up key alist)
              (and-let* ((pair (assq key alist))) (cdr pair)))
            (list (look-up 'b '((a . 1) (b . 2)))
                  (look-up 'z '((a . 1) (b . 2))))
            "#,
        ),
        "(2 #f)"
    );
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs and-let*) (scheme base))
            (and-let* ((lst '(1 2 3)) ((not (null? lst)))) (car lst))
            "#,
        ),
        Value::integer(1)
    );
}

#[test]
fn a_bare_variable_clause_tests_an_existing_binding() {
    let mut engine = engine_with_srfi2();
    // A clause that is just an identifier tests a variable already in scope. It
    // stops the form when that variable is #f, and otherwise falls through to
    // the body.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs and-let*) (scheme base))
            (let ((n 4)) (and-let* (n ((> n 0))) (* n n)))
            "#,
        ),
        Value::integer(16)
    );
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs and-let*) (scheme base))
            (let ((n -4)) (and-let* (n ((> n 0))) (* n n)))
            "#,
        ),
        Value::boolean(false)
    );
}

#[test]
fn a_trailing_clause_with_no_body_yields_its_value() {
    let mut engine = engine_with_srfi2();
    // With no body, the form returns the value of the last clause rather than a
    // plain #t. An empty clause list yields #t, and an empty clause list with a
    // body runs the body like let.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs and-let*) (scheme base))
            (and (equal? 5 (and-let* ((x 5))))
                 (eq? #f (and-let* ((x #f))))
                 (eq? #t (and-let* ()))
                 (equal? 99 (and-let* () 99)))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn the_body_is_a_let_body_with_internal_definitions() {
    let mut engine = engine_with_srfi2();
    // Once every clause passes, the body behaves like a let* body: multiple
    // forms run in order and may open with internal definitions.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs and-let*) (scheme base))
            (and-let* ((x 3) (y 4))
              (define hypotenuse-squared (+ (* x x) (* y y)))
              hypotenuse-squared)
            "#,
        ),
        Value::integer(25)
    );
}

#[test]
fn the_extension_advertises_its_feature() {
    let mut engine = engine_with_srfi2();
    // Installing the extension enables the srfi-2 cond-expand feature.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs and-let*) (scheme base))
            (cond-expand (srfi-2 #t) (else #f))
            "#,
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs and-let*) (scheme base)) (if (memq 'srfi-2 (features)) #t #f)"
        ),
        Value::boolean(true)
    );
}

#[test]
fn a_bare_engine_cannot_import_the_extension() {
    // Without install_extension neither library name is available.
    let mut bare = Engine::new(EngineConfig::default()).unwrap();
    for program in ["(import (srfi 2)) 1", "(import (r7rs and-let*)) 1"] {
        assert!(
            bare.compile("program.scm", program).is_err(),
            "{program} without the extension should fail"
        );
    }
}
