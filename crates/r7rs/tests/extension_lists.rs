//! Usage examples for the SRFI 1 (List Library) extension. The examples import
//! it through the `(r7rs lists)` alias. The canonical `(srfi 1)` name provides
//! the identical library.

use r7rs::{Engine, EngineConfig, ErrorKind, Extension, Value};

/// Builds an engine with SRFI 1 installed, the usual first step for any script
/// that needs the extended list vocabulary.
fn engine_with_srfi1() -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Srfi1).unwrap();
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
fn constructing_lists_with_iota_and_tabulate() {
    let mut engine = engine_with_srfi1();
    // `iota` counts from a start by a step, tower-correct for exact and inexact.
    assert_eq!(
        show(&mut engine, "(import (r7rs lists)) (iota 5)"),
        "(0 1 2 3 4)"
    );
    assert_eq!(
        show(&mut engine, "(import (r7rs lists)) (iota 4 1 2)"),
        "(1 3 5 7)"
    );
    // `list-tabulate` fills a list from a function of the index.
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (list-tabulate 4 (lambda (i) (* i i)))"
        ),
        "(0 1 4 9)"
    );
    // `cons*` conses a run of elements onto a final tail.
    assert_eq!(
        show(&mut engine, "(import (r7rs lists)) (cons* 1 2 3 '(4 5))"),
        "(1 2 3 4 5)"
    );
}

#[test]
fn taking_and_dropping_prefixes_and_suffixes() {
    let mut engine = engine_with_srfi1();
    assert_eq!(
        show(&mut engine, "(import (r7rs lists)) (take '(a b c d e) 2)"),
        "(a b)"
    );
    assert_eq!(
        show(&mut engine, "(import (r7rs lists)) (drop '(a b c d e) 2)"),
        "(c d e)"
    );
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (take-right '(a b c d e) 2)"
        ),
        "(d e)"
    );
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (drop-right '(a b c d e) 2)"
        ),
        "(a b c)"
    );
    // `split-at` returns both halves as two values, joined here with a list.
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (call-with-values (lambda () (split-at '(a b c d) 2)) list)"
        ),
        "((a b) (c d))"
    );
    assert_eq!(
        show(&mut engine, "(import (r7rs lists)) (last '(a b c))"),
        "c"
    );
}

#[test]
fn take_checks_the_input_before_allocating_for_a_huge_count() {
    let mut engine = engine_with_srfi1();
    let module = engine
        .compile(
            "huge_take.scm",
            "(import (srfi 1)) (take '() 18446744073709551615)",
        )
        .unwrap();
    let error = engine.eval(&module).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::TypeError);
}

#[test]
fn folding_and_reducing() {
    let mut engine = engine_with_srfi1();
    // `fold` passes each element then the accumulator: this reverses the list.
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (fold cons '() '(1 2 3))"
        ),
        "(3 2 1)"
    );
    // `fold-right` preserves order.
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (fold-right cons '() '(1 2 3))"
        ),
        "(1 2 3)"
    );
    // `fold` over several lists stops at the shortest.
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs lists)) (fold (lambda (a b acc) (+ acc (* a b))) 0 '(1 2 3) '(4 5 6 7))"
        ),
        Value::integer(32)
    );
    // `reduce` needs no seed for a non-empty list.
    assert_eq!(
        run(&mut engine, "(import (r7rs lists)) (reduce + 0 '(1 2 3 4))"),
        Value::integer(10)
    );
}

#[test]
fn mapping_variants() {
    let mut engine = engine_with_srfi1();
    // `append-map` maps then concatenates.
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (append-map (lambda (x) (list x x)) '(1 2 3))"
        ),
        "(1 1 2 2 3 3)"
    );
    // `filter-map` keeps only the non-#f results.
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (filter-map (lambda (x) (and (odd? x) (* x x))) '(1 2 3 4 5))"
        ),
        "(1 9 25)"
    );
    // `map` still applies across several lists, stopping at the shortest.
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (map + '(1 2 3) '(10 20 30))"
        ),
        "(11 22 33)"
    );
}

#[test]
fn filtering_partitioning_and_searching() {
    let mut engine = engine_with_srfi1();
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (filter odd? '(1 2 3 4 5))"
        ),
        "(1 3 5)"
    );
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (remove odd? '(1 2 3 4 5))"
        ),
        "(2 4)"
    );
    // `partition` returns the kept and rejected elements as two values.
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (call-with-values (lambda () (partition odd? '(1 2 3 4 5))) list)"
        ),
        "((1 3 5) (2 4))"
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs lists)) (find even? '(3 1 4 1 5))"
        ),
        Value::integer(4)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs lists)) (any (lambda (x) (> x 4)) '(1 2 3 5))"
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs lists)) (every (lambda (x) (> x 0)) '(1 2 3))"
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs lists)) (count even? '(1 2 3 4 5 6))"
        ),
        Value::integer(3)
    );
    // `take-while` stops at the first element failing the predicate.
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (take-while odd? '(1 3 5 2 7))"
        ),
        "(1 3 5)"
    );
}

#[test]
fn deleting_and_deduplicating() {
    let mut engine = engine_with_srfi1();
    assert_eq!(
        show(&mut engine, "(import (r7rs lists)) (delete 2 '(1 2 3 2 4))"),
        "(1 3 4)"
    );
    // `delete-duplicates` keeps the first occurrence of each element.
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (delete-duplicates '(a b a c b a))"
        ),
        "(a b c)"
    );
    // A custom comparator is accepted as the last argument.
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (delete-duplicates '(1 2 -1 3 -2) (lambda (a b) (= (abs a) (abs b))))"
        ),
        "(1 2 3)"
    );
}

#[test]
fn association_list_helpers() {
    let mut engine = engine_with_srfi1();
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (alist-cons 'a 1 '((b . 2)))"
        ),
        "((a . 1) (b . 2))"
    );
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (alist-delete 'b '((a . 1) (b . 2) (c . 3)))"
        ),
        "((a . 1) (c . 3))"
    );
}

#[test]
fn set_operations_on_lists() {
    let mut engine = engine_with_srfi1();
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (lset-intersection eqv? '(a b c d) '(b d e))"
        ),
        "(b d)"
    );
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (lset-difference eqv? '(a b c d) '(b d))"
        ),
        "(a c)"
    );
    // `lset-union` keeps the first list and adds the new members it lacks.
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs lists)) (length (lset-union eqv? '(a b c) '(b c d e)))"
        ),
        Value::integer(5)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs lists)) (lset<= eqv? '(a) '(a b) '(a b c))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn zipping_and_unzipping() {
    let mut engine = engine_with_srfi1();
    assert_eq!(
        show(&mut engine, "(import (r7rs lists)) (zip '(1 2 3) '(a b c))"),
        "((1 a) (2 b) (3 c))"
    );
    // `unzip2` splits a list of pairs back into two parallel lists.
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs lists)) (call-with-values (lambda () (unzip2 '((1 a) (2 b) (3 c)))) list)"
        ),
        "((1 2 3) (a b c))"
    );
}

#[test]
fn the_extension_advertises_its_feature() {
    let mut engine = engine_with_srfi1();
    // Installing the extension enables the `srfi-1` cond-expand feature.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs lists) (scheme base))
            (cond-expand (srfi-1 #t) (else #f))
            "#,
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs lists) (scheme base)) (if (memq 'srfi-1 (features)) #t #f)"
        ),
        Value::boolean(true)
    );
}

#[test]
fn a_bare_engine_cannot_import_the_extension() {
    // Without install_extension neither library name is available.
    let mut bare = Engine::new(EngineConfig::default()).unwrap();
    for program in ["(import (srfi 1)) 1", "(import (r7rs lists)) 1"] {
        assert!(
            bare.compile("program.scm", program).is_err(),
            "{program} without the extension should fail"
        );
    }
}
