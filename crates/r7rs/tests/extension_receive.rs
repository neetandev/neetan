//! Usage examples for the SRFI 8 (`receive`) extension. The examples import it
//! through the `(r7rs receive)` alias. The canonical `(srfi 8)` name provides
//! the identical library.
//!
//! `receive` is a concise syntax for binding the multiple values of an
//! expression before evaluating a body. `(receive <formals> <expression>
//! <body> ...)` evaluates `<expression>`, binds its values according to
//! `<formals>`, and runs `<body>` in that scope. `<formals>` takes any lambda
//! formals shape: a proper list binds each value, a bare identifier collects
//! every value into a list, and a dotted list binds the leading values and
//! gathers the rest.

use r7rs::{Engine, EngineConfig, Extension, Value};

/// Builds an engine with SRFI 8 installed, the usual first step for any script
/// that needs `receive`.
fn engine_with_srfi8() -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Srfi8).unwrap();
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
fn a_fixed_formals_list_binds_each_value() {
    let mut engine = engine_with_srfi8();
    // The common case: a proper list of variables binds the values one to one,
    // then the body runs with those names in scope.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs receive) (scheme base))
            (receive (a b) (values 1 2) (+ a b))
            "#,
        ),
        Value::integer(3)
    );
}

#[test]
fn a_bare_identifier_collects_every_value_into_a_list() {
    let mut engine = engine_with_srfi8();
    // With a single identifier as the formals, every value produced is gathered
    // into a freshly allocated list bound to that name.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs receive) (scheme base))
            (receive all (values 1 2 3) all)
            "#,
        ),
        "(1 2 3)"
    );
}

#[test]
fn a_dotted_formals_list_binds_leading_values_and_gathers_the_rest() {
    let mut engine = engine_with_srfi8();
    // A dotted formals list binds the leading values by position and puts any
    // remaining values into a list bound to the tail variable.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs receive) (scheme base))
            (receive (first . rest) (values 1 2 3) (list first rest))
            "#,
        ),
        "(1 (2 3))"
    );
}

#[test]
fn the_body_runs_like_a_lambda_body() {
    let mut engine = engine_with_srfi8();
    // Once the values are bound the body behaves like a lambda body: multiple
    // forms run in order and may open with internal definitions.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs receive) (scheme base))
            (receive (x y) (values 3 4)
              (define hypotenuse-squared (+ (* x x) (* y y)))
              hypotenuse-squared)
            "#,
        ),
        Value::integer(25)
    );
}

#[test]
fn the_extension_advertises_its_feature() {
    let mut engine = engine_with_srfi8();
    // Installing the extension enables the srfi-8 cond-expand feature.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs receive) (scheme base))
            (cond-expand (srfi-8 #t) (else #f))
            "#,
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs receive) (scheme base)) (if (memq 'srfi-8 (features)) #t #f)"
        ),
        Value::boolean(true)
    );
}

#[test]
fn a_bare_engine_cannot_import_the_extension() {
    // Without install_extension neither library name is available.
    let mut bare = Engine::new(EngineConfig::default()).unwrap();
    for program in ["(import (srfi 8)) 1", "(import (r7rs receive)) 1"] {
        assert!(
            bare.compile("program.scm", program).is_err(),
            "{program} without the extension should fail"
        );
    }
}
