//! Usage examples for the SRFI 48 (Intermediate Format Strings) extension. The
//! examples import it through the `(r7rs intermediate-format-strings)` alias. The canonical
//! `(srfi 48)` name and the `(srfi 28)` compatibility name provide the
//! identical library.

use r7rs::{Engine, EngineConfig, ErrorKind, Extension, Value};

/// Builds an engine with SRFI 48 installed, the usual first step for any script
/// that needs `format`.
fn engine_with_srfi48() -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Srfi48).unwrap();
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
fn display_and_write_directives_match_the_spec_example() {
    let mut engine = engine_with_srfi48();
    // `~a` renders like display, `~s` like write, in argument order.
    assert_eq!(
        show(
            &mut engine,
            r#"(import (r7rs intermediate-format-strings)) (format "~a ~s ~a ~s" 'this 'is "a" "test")"#
        ),
        "\"this is a \\\"test\\\"\""
    );
    // `~%` is a newline and `~~` a literal tilde. Directives are
    // case-insensitive.
    assert_eq!(
        show(
            &mut engine,
            r#"(import (r7rs intermediate-format-strings)) (format "~A~~~%" 1)"#
        ),
        "\"1~\\n\""
    );
}

#[test]
fn radix_directives_render_numbers() {
    let mut engine = engine_with_srfi48();
    assert_eq!(
        show(
            &mut engine,
            r##"(import (r7rs intermediate-format-strings)) (format "#d~d #x~x #o~o #b~b" 32 32 32 32)"##
        ),
        "\"#d32 #x20 #o40 #b100000\""
    );
    // The typical emulator diagnostic: registers and addresses in hex.
    assert_eq!(
        show(
            &mut engine,
            r#"(import (r7rs intermediate-format-strings)) (format "PC=~x SP=~x" 65500 255)"#
        ),
        "\"PC=ffdc SP=ff\""
    );
}

#[test]
fn character_tab_and_space_directives() {
    let mut engine = engine_with_srfi48();
    // `~c` outputs the character itself, `~t` a tab, `~_` a space.
    assert_eq!(
        show(
            &mut engine,
            r#"(import (r7rs intermediate-format-strings)) (format "~c~t~_~c" #\a #\b)"#
        ),
        "\"a\\t b\""
    );
}

#[test]
fn freshline_emits_a_newline_only_when_needed() {
    let mut engine = engine_with_srfi48();
    // The spec example: consecutive `~&` collapse into one newline.
    assert_eq!(
        show(
            &mut engine,
            r#"(import (r7rs intermediate-format-strings)) (format #f "~&1~&~&2~&~&~&3~%")"#
        ),
        "\"\\n1\\n2\\n3\\n\""
    );
    // A `~&` right after an argument that ended with a newline emits nothing.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (scheme base) (r7rs intermediate-format-strings))
            (format "~a~a~&" (list->string (list #\newline)) "")
            "#
        ),
        "\"\\n\""
    );
}

#[test]
fn write_circular_handles_recursive_structure() {
    let mut engine = engine_with_srfi48();
    // `~w` labels shared structure instead of looping forever. This engine
    // numbers datum labels from 0.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (scheme base) (r7rs intermediate-format-strings))
            (format "~w" (let ((c (list 'a 'b 'c))) (set-cdr! (cddr c) c) c))
            "#
        ),
        "\"#0=(a b c . #0#)\""
    );
}

#[test]
fn yuppify_pretty_prints_as_write() {
    let mut engine = engine_with_srfi48();
    // This implementation deliberately pretty-prints with plain write, which
    // the SRFI permits.
    assert_eq!(
        show(
            &mut engine,
            r#"(import (r7rs intermediate-format-strings)) (format "~y" '(1 "two" 3))"#
        ),
        "\"(1 \\\"two\\\" 3)\""
    );
}

#[test]
fn indirection_formats_a_nested_template() {
    let mut engine = engine_with_srfi48();
    // `~?` consumes a template and a list of its arguments, `~k` is a synonym.
    assert_eq!(
        show(
            &mut engine,
            r#"(import (r7rs intermediate-format-strings)) (format "~a ~? ~a" 'a "~s" '(new) 'test)"#
        ),
        "\"a new test\""
    );
    assert_eq!(
        show(
            &mut engine,
            r#"(import (r7rs intermediate-format-strings)) (format "~k" "~a+~a" '(1 2))"#
        ),
        "\"1+2\""
    );
}

#[test]
fn fixed_format_pads_numbers_and_strings() {
    let mut engine = engine_with_srfi48();
    // The spec examples: width pads on the left, digits pad on the right with
    // zeros, and a too-wide rendering is emitted whole.
    for (template, argument, expected) in [
        ("~8,2F", "32", "   32.00"),
        ("~6F", "32", "    32"),
        ("~1,2F", "4321", "4321.00"),
        ("~6,3F", "1/3", " 0.333"),
        ("~8,3F", "123.3456", " 123.346"),
        ("~4F", "12", "  12"),
        ("~8,3F", "\"foo\"", "     foo"),
    ] {
        let source = format!(
            r#"(import (scheme base) (r7rs intermediate-format-strings)) (format "{template}" {argument})"#
        );
        assert_eq!(
            show(&mut engine, &source),
            format!("{expected:?}"),
            "template {template} on {argument}"
        );
    }
    // With digits specified, both parts of a complex number are fixed.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (scheme complex) (r7rs intermediate-format-strings))
            (format "~1,2F" (make-rectangular 1.0 -2.0))
            "#
        ),
        "\"1.00-2.00i\""
    );
}

#[test]
fn help_directive_documents_the_directives() {
    let mut engine = engine_with_srfi48();
    // `~h` expands to the multi-line synopsis, one line per directive.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme base) (r7rs intermediate-format-strings))
            (let ((help (format "~h")))
              (and (string? help) (> (string-length help) 500)))
            "#
        ),
        Value::boolean(true)
    );
}

#[test]
fn destination_argument_selects_string_or_port_output() {
    let mut engine = engine_with_srfi48();
    // `#f` (or no destination) returns the string.
    assert_eq!(
        show(
            &mut engine,
            r#"(import (r7rs intermediate-format-strings)) (format #f "~a" 1)"#
        ),
        "\"1\""
    );
    // An output port receives the text directly.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (scheme base) (r7rs intermediate-format-strings))
            (let ((port (open-output-string)))
              (format port "~a!" 5)
              (get-output-string port))
            "#
        ),
        "\"5!\""
    );
    // `#t` writes to the current output port.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (scheme base) (r7rs intermediate-format-strings))
            (let ((port (open-output-string)))
              (parameterize ((current-output-port port))
                (format #t "~a" 7))
              (get-output-string port))
            "#
        ),
        "\"7\""
    );
}

#[test]
fn argument_count_must_match_the_template() {
    let mut engine = engine_with_srfi48();
    // SRFI 48 makes both missing and leftover arguments an error.
    assert_eq!(
        error_kind(
            &mut engine,
            r#"(import (r7rs intermediate-format-strings)) (format "~a ~a" 1)"#
        ),
        ErrorKind::ArityError
    );
    assert_eq!(
        error_kind(
            &mut engine,
            r#"(import (r7rs intermediate-format-strings)) (format "~a" 1 2)"#
        ),
        ErrorKind::ArityError
    );
}

#[test]
fn invalid_templates_and_arguments_raise_clear_errors() {
    let mut engine = engine_with_srfi48();
    assert_eq!(
        error_kind(
            &mut engine,
            r#"(import (r7rs intermediate-format-strings)) (format "~q" 1)"#
        ),
        ErrorKind::RuntimeError
    );
    assert_eq!(
        error_kind(
            &mut engine,
            r#"(import (r7rs intermediate-format-strings)) (format "~d" "12")"#
        ),
        ErrorKind::TypeError
    );
    assert_eq!(
        error_kind(
            &mut engine,
            r#"(import (r7rs intermediate-format-strings)) (format "~c" 12)"#
        ),
        ErrorKind::TypeError
    );
}

#[test]
fn the_srfi_28_compatibility_name_provides_the_same_format() {
    let mut engine = engine_with_srfi48();
    // A script written for SRFI 28 keeps working unchanged, and the richer
    // directives are available through the compatibility name too.
    assert_eq!(
        show(
            &mut engine,
            r#"(import (srfi 28)) (format "Hello, ~a" "World!")"#
        ),
        "\"Hello, World!\""
    );
    assert_eq!(
        show(&mut engine, r#"(import (srfi 28)) (format "~x" 255)"#),
        "\"ff\""
    );
}

#[test]
fn the_extension_advertises_its_features() {
    let mut engine = engine_with_srfi48();
    // Installing the extension enables the `srfi-48` cond-expand feature and,
    // because the compatibility name satisfies SRFI 28, `srfi-28` as well.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (srfi 48) (scheme base))
            (cond-expand ((and srfi-48 srfi-28) #t) (else #f))
            "#,
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (srfi 48) (scheme base))
            (if (and (memq 'srfi-48 (features)) (memq 'srfi-28 (features))) #t #f)
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn a_bare_engine_cannot_import_the_extension() {
    // Without install_extension none of the library names are available.
    let mut bare = Engine::new(EngineConfig::default()).unwrap();
    for program in [
        "(import (srfi 48)) 1",
        "(import (srfi 28)) 1",
        "(import (r7rs intermediate-format-strings)) 1",
    ] {
        assert!(
            bare.compile("program.scm", program).is_err(),
            "{program} without the extension should fail"
        );
    }
}
