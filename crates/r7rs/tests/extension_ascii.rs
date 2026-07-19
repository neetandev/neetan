//! Usage and conformance tests for the SRFI 175 ASCII character library.
//! Examples import the `(r7rs ascii)` alias. The canonical `(srfi 175)` name
//! provides the identical library.

use r7rs::{Engine, EngineConfig, ErrorKind, Extension, Value};

/// Builds an engine with SRFI 175 installed.
fn engine_with_srfi175() -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Srfi175).unwrap();
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
fn object_predicates_recognize_only_ascii_values() {
    let mut engine = engine_with_srfi175();
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs ascii) (scheme base))
            (list
              (map ascii-codepoint? '(-1 0 127 128 #\A 1.0))
              (map ascii-char? (list #\null #\delete #\λ 65))
              (map ascii-string? '("" "ASCII" "λ" 42))
              (map ascii-bytevector?
                   (list #u8() #u8(0 127) #u8(0 128) "not a bytevector")))
            "#,
        ),
        "((#f #t #t #f #f #f) (#t #t #f #f) (#t #t #f #f) (#t #t #f #f))"
    );
}

#[test]
fn character_classes_cover_every_ascii_codepoint() {
    let mut engine = engine_with_srfi175();
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs ascii) (scheme base))
            (define (in? n lo hi) (and (<= lo n) (<= n hi)))
            (define (expected-control? n) (or (in? n 0 31) (= n 127)))
            (define (expected-other? n)
              (or (in? n 33 47) (in? n 58 64)
                  (in? n 91 96) (in? n 123 126)))
            (define predicates
              (list ascii-control? ascii-non-control? ascii-whitespace?
                    ascii-space-or-tab? ascii-other-graphic?
                    ascii-upper-case? ascii-lower-case? ascii-alphabetic?
                    ascii-alphanumeric? ascii-numeric?))
            (define (same-representation-results? predicates integer character)
              (or (null? predicates)
                  (and (eq? ((car predicates) integer)
                            ((car predicates) character))
                       (same-representation-results?
                         (cdr predicates) integer character))))
            (let loop ((n 0))
              (if (= n 128)
                  #t
                  (let ((ch (integer->char n)))
                    (and
                      (same-representation-results? predicates n ch)
                      (eq? (ascii-control? n) (expected-control? n))
                      (eq? (ascii-non-control? n) (in? n 32 126))
                      (eq? (ascii-whitespace? n)
                           (or (in? n 9 13) (= n 32)))
                      (eq? (ascii-space-or-tab? n) (or (= n 9) (= n 32)))
                      (eq? (ascii-other-graphic? n) (expected-other? n))
                      (eq? (ascii-upper-case? n) (in? n 65 90))
                      (eq? (ascii-lower-case? n) (in? n 97 122))
                      (eq? (ascii-alphabetic? n)
                           (or (in? n 65 90) (in? n 97 122)))
                      (eq? (ascii-alphanumeric? n)
                           (or (in? n 48 57) (in? n 65 90)
                               (in? n 97 122)))
                      (eq? (ascii-numeric? n) (in? n 48 57))
                      (loop (+ n 1))))))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn character_classes_reject_non_ascii_but_accept_character_or_integer_inputs() {
    let mut engine = engine_with_srfi175();
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs ascii) (scheme base))
            (list
              (ascii-control? -1) (ascii-control? 128)
              (ascii-non-control? #\λ)
              (ascii-alphabetic? #\é)
              (ascii-numeric? #\９)
              (ascii-whitespace? #\x2003))
            "#,
        ),
        "(#f #f #f #f #f #f)"
    );
}

#[test]
fn case_conversion_preserves_the_input_representation() {
    let mut engine = engine_with_srfi175();
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs ascii) (scheme base))
            (list (ascii-upcase #\a) (ascii-upcase 97)
                  (ascii-downcase #\Z) (ascii-downcase 90)
                  (ascii-upcase #\λ) (ascii-downcase #\λ)
                  (ascii-upcase 170141183460469231731687303715884105727)
                  (ascii-downcase -170141183460469231731687303715884105728))
            "#,
        ),
        "(#\\A 65 #\\z 122 #\\λ #\\λ 170141183460469231731687303715884105727 -170141183460469231731687303715884105728)"
    );
}

#[test]
fn control_and_graphic_conversions_round_trip() {
    let mut engine = engine_with_srfi175();
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs ascii) (scheme base))
            (define (round-trips? value)
              (and (= value
                      (ascii-graphic->control
                        (ascii-control->graphic value)))
                   (char=?
                     (integer->char value)
                     (ascii-graphic->control
                       (ascii-control->graphic (integer->char value))))))
            (and
              (let loop ((n 0))
                (or (= n 32) (and (round-trips? n) (loop (+ n 1)))))
              (round-trips? 127)
              (eq? #f (ascii-control->graphic 32))
              (eq? #f (ascii-graphic->control #\a))
              (= 127 (ascii-graphic->control 63)))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn bracket_mirroring_handles_all_pairs_and_preserves_representation() {
    let mut engine = engine_with_srfi175();
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs ascii) (scheme base))
            (list
              (map ascii-mirror-bracket
                   '(#\( #\) #\[ #\] #\{ #\} #\< #\> #\A))
              (map ascii-mirror-bracket
                   '(40 41 91 93 123 125 60 62 65)))
            "#,
        ),
        "((#\\) #\\( #\\] #\\[ #\\} #\\{ #\\> #\\< #f) (41 40 93 91 125 123 62 60 #f))"
    );
}

#[test]
fn numeric_transformations_follow_limits_offsets_and_rotation() {
    let mut engine = engine_with_srfi175();
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs ascii) (scheme base))
            (list
              (ascii-digit-value #\0 10)
              (ascii-digit-value #\7 8)
              (ascii-digit-value #\7 7)
              (ascii-digit-value #\0 0)
              (ascii-upper-case-value #\F 10 6)
              (ascii-upper-case-value #\G 10 6)
              (ascii-lower-case-value #\b 9223372036854775808 2)
              (map ascii-nth-digit '(-1 0 9 10))
              (map ascii-nth-upper-case '(-1 0 25 26))
              (map ascii-nth-lower-case '(-1 0 25 26)))
            "#,
        ),
        "(0 7 #f #f 15 #f 9223372036854775809 (#f #\\0 #\\9 #f) (#\\Z #\\A #\\Z #\\A) (#\\z #\\a #\\z #\\a))"
    );
}

#[test]
fn alphabet_rotation_handles_the_full_exact_integer_range() {
    let mut engine = engine_with_srfi175();
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs ascii) (scheme base))
            (define minimum -170141183460469231731687303715884105728)
            (define maximum 170141183460469231731687303715884105727)
            (and
              (char=? (ascii-nth-upper-case minimum)
                      (ascii-nth-upper-case (modulo minimum 26)))
              (char=? (ascii-nth-lower-case maximum)
                      (ascii-nth-lower-case (modulo maximum 26))))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn character_comparisons_fold_ascii_only_and_allow_mixed_inputs() {
    let mut engine = engine_with_srfi175();
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs ascii) (scheme base))
            (and (ascii-ci=? #\A 97)
                 (ascii-ci<? #\a #\B)
                 (ascii-ci>? #\A #\_)
                 (ascii-ci<=? #\a 65)
                 (ascii-ci>=? 90 #\z)
                 (not (ascii-ci=? #\É #\é))
                 (ascii-ci<? #\É #\é)
                 (ascii-ci<? -170141183460469231731687303715884105728
                             #\A)
                 (ascii-ci>? 170141183460469231731687303715884105727
                             #\z))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn string_comparisons_fold_ascii_and_compare_prefixes() {
    let mut engine = engine_with_srfi175();
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs ascii) (scheme base))
            (and (ascii-string-ci=? "" "")
                 (ascii-string-ci=? "Alpha" "aLPHA")
                 (ascii-string-ci<? "a" "aa")
                 (ascii-string-ci>? "baa" "Ba")
                 (ascii-string-ci<=? "same" "SAME")
                 (ascii-string-ci>=? "z" "Y")
                 (not (ascii-string-ci=? "É" "é"))
                 (ascii-string-ci<? "É" "é"))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn invalid_argument_types_raise_type_errors() {
    let mut engine = engine_with_srfi175();
    for source in [
        r#"(import (r7rs ascii)) (ascii-control? "x")"#,
        r#"(import (r7rs ascii)) (ascii-upcase 1.0)"#,
        r#"(import (r7rs ascii)) (ascii-nth-digit #\1)"#,
        r#"(import (r7rs ascii)) (ascii-digit-value #\1 10.0)"#,
        r#"(import (r7rs ascii)) (ascii-string-ci=? "a" #\a)"#,
    ] {
        assert_eq!(error_kind(&mut engine, source), ErrorKind::TypeError);
    }
}

#[test]
fn offset_limit_arithmetic_never_wraps() {
    let mut engine = engine_with_srfi175();
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs ascii))
            (ascii-upper-case-value #\A
              170141183460469231731687303715884105727
              1)
            "#,
        ),
        "170141183460469231731687303715884105727"
    );
    assert_eq!(
        error_kind(
            &mut engine,
            r#"
            (import (r7rs ascii))
            (ascii-upper-case-value #\A
              170141183460469231731687303715884105727
              2)
            "#,
        ),
        ErrorKind::ImplementationRestriction
    );
    assert_eq!(
        error_kind(
            &mut engine,
            r#"
            (import (r7rs ascii))
            (ascii-lower-case-value #\z
              -170141183460469231731687303715884105728
              0)
            "#,
        ),
        ErrorKind::ImplementationRestriction
    );
}

#[test]
fn the_extension_advertises_its_feature() {
    let mut engine = engine_with_srfi175();
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs ascii) (scheme base))
            (and (cond-expand (srfi-175 #t) (else #f))
                 (if (memq 'srfi-175 (features)) #t #f))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn a_bare_engine_cannot_import_the_extension() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    for source in ["(import (srfi 175)) #t", "(import (r7rs ascii)) #t"] {
        assert!(
            engine.compile("program.scm", source).is_err(),
            "{source} without installation should fail"
        );
    }
}
