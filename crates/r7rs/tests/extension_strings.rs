//! Usage examples for the SRFI 152 (String Library) extension. The examples
//! import it through the `(r7rs strings)` alias. The canonical `(srfi 152)` name
//! provides the identical library.
//!
//! SRFI 152 is a reduced version of the classic string SRFIs. It layers a large
//! set of string-processing procedures on top of the R7RS-small string
//! procedures (which it re-exports unchanged), so a single import brings the
//! whole vocabulary into scope: predicates, constructors, selection, padding and
//! trimming, comparison, prefix and suffix tests, searching, concatenation,
//! folding and mapping, and splitting and joining.

use r7rs::{Engine, EngineConfig, ErrorKind, Extension, Value};

/// Builds an engine with SRFI 152 installed, the usual first step for any script
/// that needs the string library.
fn engine_with_srfi152() -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Srfi152).unwrap();
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

/// Evaluates a program and returns the error kind it raises.
fn error_kind(engine: &mut Engine, source: &str) -> ErrorKind {
    let module = engine.compile("program.scm", source).unwrap();
    engine.eval(&module).unwrap_err().kind()
}

#[test]
fn predicates_test_emptiness_and_scan_with_a_predicate() {
    let mut engine = engine_with_srfi152();
    // string-null? is true only for the empty string.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (list (string-null? "") (string-null? "x"))
            "#,
        ),
        "(#t #f)"
    );
    // string-every returns the last true value; string-any returns the first.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (list (string-every char-alphabetic? "abc")
                  (string-any char-numeric? "ab3cd"))
            "#,
        ),
        "(#t #t)"
    );
    // string-every on an empty range is true; string-any that never matches is #f.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (list (string-every char-numeric? "")
                  (string-any char-numeric? "abc"))
            "#,
        ),
        "(#t #f)"
    );
}

#[test]
fn constructors_tabulate_and_unfold() {
    let mut engine = engine_with_srfi152();
    // string-tabulate calls proc on each index to build the string.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (string-tabulate (lambda (i) (integer->char (+ 65 i))) 5)
            "#,
        ),
        "\"ABCDE\""
    );
    // string-unfold builds left to right; here it just copies a list of chars.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (string-unfold null? car cdr '(#\a #\b #\c))
            "#,
        ),
        "\"abc\""
    );
    // string-unfold-right assembles right to left, with a base and a make-final.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (string-unfold-right null?
                                 (lambda (x) (string #\[ (car x) #\]))
                                 cdr
                                 '(#\a #\b #\c))
            "#,
        ),
        "\"[c][b][a]\""
    );
}

#[test]
fn reverse_list_to_string_builds_from_a_reversed_char_list() {
    let mut engine = engine_with_srfi152();
    // A common idiom at the end of a loop that accumulates chars in reverse.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (reverse-list->string '(#\a #\B #\c))
            "#,
        ),
        "\"cBa\""
    );
}

#[test]
fn selection_takes_drops_pads_and_trims() {
    let mut engine = engine_with_srfi152();
    // take/drop from either end.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (list (string-take "Pete Szilagyi" 6)
                  (string-drop "Pete Szilagyi" 6)
                  (string-take-right "Beta rules" 5)
                  (string-drop-right "Beta rules" 5))
            "#,
        ),
        "(\"Pete S\" \"zilagyi\" \"rules\" \"Beta \")"
    );
    // pad on the left; longer input is truncated on the left.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (list (string-pad "325" 5) (string-pad "8871325" 5))
            "#,
        ),
        "(\"  325\" \"71325\")"
    );
    // trim strips whitespace by default; a predicate customizes what to strip.
    assert_eq!(
        show(
            &mut engine,
            "(import (r7rs strings) (scheme base)) (string-trim-both \"  hi  \")",
        ),
        "\"hi\""
    );
}

#[test]
fn replacement_splices_one_string_into_another() {
    let mut engine = engine_with_srfi152();
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (string-replace "It's easy to code it up in Scheme." "lots of fun" 5 9)
            "#,
        ),
        "\"It's lots of fun to code it up in Scheme.\""
    );
}

#[test]
fn prefixes_and_suffixes_measure_and_test() {
    let mut engine = engine_with_srfi152();
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (list (string-prefix-length "cool" "court")
                  (string-suffix-length "place" "space")
                  (string-prefix? "abc" "abcdef")
                  (string-suffix? "def" "abcdef"))
            "#,
        ),
        "(2 3 #t #t)"
    );
}

#[test]
fn searching_by_predicate_and_by_substring() {
    let mut engine = engine_with_srfi152();
    // index/skip find the first char that does/does not satisfy the predicate.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (list (string-index "  abc" char-alphabetic?)
                  (string-skip "  abc" char-whitespace?)
                  (string-index-right "abc  " char-alphabetic?))
            "#,
        ),
        "(2 2 2)"
    );
    // string-contains returns the match index (searching a substring here).
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (string-contains "eek -- what a geek." "ee" 12 18)
            "#,
        ),
        Value::integer(15)
    );
    // take-while/drop-while and span/break split at the predicate boundary.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (list (string-take-while "123abc" char-numeric?)
                  (string-drop-while "123abc" char-numeric?)
                  (let-values (((a b) (string-span "123abc" char-numeric?)))
                    (list a b)))
            "#,
        ),
        "(\"123\" \"abc\" (\"123\" \"abc\"))"
    );
}

#[test]
fn concatenation_joins_lists_and_delimits() {
    let mut engine = engine_with_srfi152();
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (string-concatenate '("foo" "bar" "baz"))
            "#,
        ),
        "\"foobarbaz\""
    );
    // string-concatenate-reverse conses final onto the list, reverses, joins.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (string-concatenate-reverse '(" must be" "Hello, I") " going.XXXX" 7)
            "#,
        ),
        "\"Hello, I must be going.\""
    );
    // string-join pastes with a delimiter; the grammar controls the placement.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (list (string-join '("foo" "bar" "baz"))
                  (string-join '("foo" "bar" "baz") ":")
                  (string-join '("foo" "bar" "baz") ":" 'suffix))
            "#,
        ),
        "(\"foo bar baz\" \"foo:bar:baz\" \"foo:bar:baz:\")"
    );
}

#[test]
fn folding_mapping_and_filtering() {
    let mut engine = engine_with_srfi152();
    // string-fold-right with cons rebuilds the list of chars.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (string-fold-right cons '() "abc")
            "#,
        ),
        "(#\\a #\\b #\\c)"
    );
    // count, filter, and remove select by predicate.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (list (string-count "a1b2c3" char-numeric?)
                  (string-filter char-alphabetic? "a1b2c3")
                  (string-remove char-alphabetic? "a1b2c3"))
            "#,
        ),
        "(3 \"abc\" \"123\")"
    );
    // string-map is the R7RS-small one, re-exported unchanged.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (string-map char-upcase "hello")
            "#,
        ),
        "\"HELLO\""
    );
}

#[test]
fn replication_and_splitting() {
    let mut engine = engine_with_srfi152();
    // string-replicate rotates and repeats through a replicated index space.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (list (string-replicate "abcdef" 2 8)
                  (string-replicate "abcdef" -2 4)
                  (string-replicate "abc" 0 7))
            "#,
        ),
        "(\"cdefab\" \"efabcd\" \"abcabca\")"
    );
    // string-segment chops into fixed-length pieces; the last may be shorter.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (string-segment "abcdefg" 3)
            "#,
        ),
        "(\"abc\" \"def\" \"g\")"
    );
    // string-split cuts on a delimiter string; the grammar controls empty edges.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (list (string-split "a,b,c" ",")
                  (string-split "a,,c" ",")
                  (string-split "a,b,c,d" "," 'infix 2))
            "#,
        ),
        "((\"a\" \"b\" \"c\") (\"a\" \"\" \"c\") (\"a\" \"b\" \"c,d\"))"
    );
}

#[test]
fn out_of_range_selection_raises() {
    let mut engine = engine_with_srfi152();
    // Taking more characters than the string holds is an error.
    assert_eq!(
        error_kind(
            &mut engine,
            "(import (r7rs strings) (scheme base)) (string-take \"foo\" 37)",
        ),
        ErrorKind::RangeError
    );
    // An empty string with the strict-infix grammar is rejected by string-split.
    assert_eq!(
        error_kind(
            &mut engine,
            "(import (r7rs strings) (scheme base)) (string-split \"\" \",\" 'strict-infix)",
        ),
        ErrorKind::RuntimeError
    );
}

#[test]
fn the_extension_advertises_its_feature() {
    let mut engine = engine_with_srfi152();
    // Installing the extension enables the srfi-152 cond-expand feature.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs strings) (scheme base))
            (cond-expand (srfi-152 #t) (else #f))
            "#,
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs strings) (scheme base)) (if (memq 'srfi-152 (features)) #t #f)",
        ),
        Value::boolean(true)
    );
}

#[test]
fn a_bare_engine_cannot_import_the_extension() {
    // Without install_extension neither library name is available.
    let mut bare = Engine::new(EngineConfig::default()).unwrap();
    for program in ["(import (srfi 152)) 1", "(import (r7rs strings)) 1"] {
        assert!(
            bare.compile("program.scm", program).is_err(),
            "{program} without the extension should fail"
        );
    }
}
