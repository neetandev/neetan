//! Usage examples for the SRFI 132 (Sort Libraries) extension. The examples
//! import it through the `(r7rs sorting)` alias. The canonical `(srfi 132)`
//! name provides the identical library.
//!
//! Every procedure takes the ordering or equality predicate as its first
//! argument, before the data, following the R6RS convention.

use r7rs::{Engine, EngineConfig, Extension, Value};

/// Builds an engine with SRFI 132 installed, the usual first step for any script
/// that needs the sort toolkit.
fn engine_with_srfi132() -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Srfi132).unwrap();
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
fn list_and_vector_sort_order_their_input() {
    let mut engine = engine_with_srfi132();
    // list-sort and vector-sort return freshly ordered data without disturbing
    // their argument. The ordering predicate comes first.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (list-sort < '(3 1 4 1 5 9 2 6))
            "#,
        ),
        "(1 1 2 3 4 5 6 9)"
    );
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (vector-sort < #(3 1 4 1 5 9 2 6))
            "#,
        ),
        "#(1 1 2 3 4 5 6 9)"
    );
}

#[test]
fn the_non_destructive_sorts_leave_their_input_alone() {
    let mut engine = engine_with_srfi132();
    // vector-sort allocates a fresh result, so the source vector is unchanged.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (define v (vector 3 1 2))
            (vector-sort < v)
            v
            "#,
        ),
        "#(3 1 2)"
    );
}

#[test]
fn stable_sort_preserves_the_order_of_equal_elements() {
    let mut engine = engine_with_srfi132();
    // A stable sort keeps equal-comparing elements in their original order.
    // Sorting by absolute value, 3 stays ahead of -3 and 1 ahead of -1.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (define (abs< x y) (< (abs x) (abs y)))
            (list-stable-sort abs< '(3 -3 1 -1 2 -2))
            "#,
        ),
        "(1 -1 2 -2 3 -3)"
    );
}

#[test]
fn the_destructive_sort_aliases_return_the_same_ordering() {
    let mut engine = engine_with_srfi132();
    // The ! variants are permitted to reuse storage; here they return an
    // ordered list and vector just like their non-destructive counterparts.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (list-stable-sort! < '(5 2 8 1))
            "#,
        ),
        "(1 2 5 8)"
    );
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (define v (vector 5 2 8 1))
            (vector-sort! < v)
            v
            "#,
        ),
        "#(1 2 5 8)"
    );
}

#[test]
fn the_sorted_predicates_test_adjacent_order() {
    let mut engine = engine_with_srfi132();
    // list-sorted? and vector-sorted? are true unless some element is strictly
    // less than the one before it. Equal neighbours are allowed.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (and (list-sorted? < '(1 2 2 3))
                 (not (list-sorted? < '(1 3 2)))
                 (vector-sorted? < #(1 2 2 3))
                 (not (vector-sorted? < #(3 2 1))))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn vector_operations_accept_a_subrange() {
    let mut engine = engine_with_srfi132();
    // The optional start and end arguments restrict a vector operation to the
    // half-open range [start, end). Here only indices 1 through 3 are examined.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (vector-sorted? < #(9 1 2 3) 1 4)
            "#,
        ),
        Value::boolean(true)
    );
    // vector-sort over a subrange returns only that many elements.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (vector-sort < #(9 3 1 2 8) 1 4)
            "#,
        ),
        "#(1 2 3)"
    );
}

#[test]
fn merging_is_stable_and_favours_the_first_data_set() {
    let mut engine = engine_with_srfi132();
    // All merges are stable: an element of the first list precedes an
    // equal-comparing element of the second. Comparing by absolute value, the 4
    // of the first list lands before the -4 of the second.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (define (abs< x y) (< (abs x) (abs y)))
            (list-merge abs< '(0 -2 4 8 -10) '(-1 3 -4 7))
            "#,
        ),
        "(0 -1 -2 3 4 -4 7 8 -10)"
    );
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (vector-merge < #(1 4 8) #(2 3 9))
            "#,
        ),
        "#(1 2 3 4 8 9)"
    );
}

#[test]
fn destructive_list_merge_reuses_input_pairs() {
    let mut engine = engine_with_srfi132();
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (let* ((first (list 1 3))
                   (first-tail (cdr first))
                   (second (list 2 4))
                   (second-tail (cdr second))
                   (result (list-merge! < first second)))
              (and (eq? result first)
                   (eq? (cdr result) second)
                   (eq? (cddr result) first-tail)
                   (eq? (cdddr result) second-tail)
                   (equal? result '(1 2 3 4))))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn vector_merge_writes_into_a_target_vector() {
    let mut engine = engine_with_srfi132();
    // vector-merge! writes the merged result into the caller's `to` vector
    // starting at index 0 and returns an unspecified value.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (define to (make-vector 6 0))
            (vector-merge! < to #(1 4 8) #(2 3 9))
            to
            "#,
        ),
        "#(1 2 3 4 8 9)"
    );
}

#[test]
fn delete_neighbor_dups_keeps_the_first_of_each_run() {
    let mut engine = engine_with_srfi132();
    // Adjacent equal elements collapse to their first occurrence. The list is
    // not otherwise reordered, so non-adjacent equal elements both survive.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (list-delete-neighbor-dups = '(1 1 2 7 7 7 0 -2 -2))
            "#,
        ),
        "(1 2 7 0 -2)"
    );
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (vector-delete-neighbor-dups = #(1 1 2 7 7 7 0 -2 -2))
            "#,
        ),
        "#(1 2 7 0 -2)"
    );
}

#[test]
fn destructive_vector_delete_neighbor_dups_packs_and_returns_the_new_end() {
    let mut engine = engine_with_srfi132();
    // The ! variant packs the survivors into the front of the range and returns
    // the exact index one past the last survivor. Elements past that index are
    // whatever the packing left behind.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (define v (vector 1 1 2 7 7 7 0 -2 -2))
            (define newend (vector-delete-neighbor-dups! = v))
            (list newend (vector-copy v 0 newend))
            "#,
        ),
        "(5 #(1 2 7 0 -2))"
    );
}

#[test]
fn destructive_list_delete_neighbor_dups_reuses_input_pairs() {
    let mut engine = engine_with_srfi132();
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (let* ((input (list 1 1 2))
                   (last (cddr input))
                   (result (list-delete-neighbor-dups! = input)))
              (and (eq? result input)
                   (eq? (cdr result) last)
                   (equal? result '(1 2))))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn find_median_handles_odd_even_and_empty_vectors() {
    let mut engine = engine_with_srfi132();
    // With an odd count the middlemost sorted element is returned.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (vector-find-median < #(3 1 2) 'none)
            "#,
        ),
        Value::integer(2)
    );
    // With an even count the default mean averages the two middlemost.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (vector-find-median < #(4 1 3 2) 'none)
            "#,
        ),
        "5/2"
    );
    // An empty vector yields the supplied knil.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (vector-find-median < #() 'none)
            "#,
        ),
        "none"
    );
    // A custom mean replaces the default averaging procedure.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (vector-find-median < #(1 2 3 4) 'none (lambda (a b) (list a b)))
            "#,
        ),
        "(2 3)"
    );
}

#[test]
fn find_median_destructively_leaves_the_input_sorted() {
    let mut engine = engine_with_srfi132();
    // vector-find-median! sorts its input in place while computing the median.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (define v (vector 5 4 3 2 1))
            (vector-find-median! < v 'none)
            v
            "#,
        ),
        "#(1 2 3 4 5)"
    );
}

#[test]
fn vector_select_returns_the_kth_smallest_element() {
    let mut engine = engine_with_srfi132();
    // vector-select! returns the kth smallest element of the range, counting
    // from zero, so k of 0 is the minimum and the last index is the maximum.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (list (vector-select! < (vector 7 3 9 1 5) 0)
                  (vector-select! < (vector 7 3 9 1 5) 2)
                  (vector-select! < (vector 7 3 9 1 5) 4))
            "#,
        ),
        "(1 5 9)"
    );
}

#[test]
fn vector_separate_moves_the_smallest_k_to_the_front() {
    let mut engine = engine_with_srfi132();
    // vector-separate! places the smallest k elements into the first k positions
    // of the range, in no particular order. Sorting that prefix shows which
    // elements were selected.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (define v (vector 7 3 9 1 5))
            (vector-separate! < v 2)
            (vector-sort < (vector-copy v 0 2))
            "#,
        ),
        "#(1 3)"
    );
}

#[test]
fn the_extension_advertises_its_feature() {
    let mut engine = engine_with_srfi132();
    // Installing the extension enables the srfi-132 cond-expand feature.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs sorting) (scheme base))
            (cond-expand (srfi-132 #t) (else #f))
            "#,
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs sorting) (scheme base)) (if (memq 'srfi-132 (features)) #t #f)"
        ),
        Value::boolean(true)
    );
}

#[test]
fn a_bare_engine_cannot_import_the_extension() {
    // Without install_extension neither library name is available.
    let mut bare = Engine::new(EngineConfig::default()).unwrap();
    for program in ["(import (srfi 132)) 1", "(import (r7rs sorting)) 1"] {
        assert!(
            bare.compile("program.scm", program).is_err(),
            "{program} without the extension should fail"
        );
    }
}
