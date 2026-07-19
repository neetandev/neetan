//! Usage examples for the R6RS Bytevectors extension. The examples import it
//! through its canonical R7RS-large name `(scheme bytevector)` or the
//! `(r7rs bytevector)` alias, which provides the identical library. The
//! library re-exports the overlapping `(scheme base)` procedures, so importing
//! both needs no renames, and `bytevector-copy!` keeps the R7RS-small argument
//! order.

use r7rs::{Engine, EngineConfig, ErrorKind, Extension, Value};

/// Builds an engine with the bytevector extension installed, the usual first
/// step for any script that needs binary data access.
fn engine_with_bytevector() -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Bytevector).unwrap();
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

/// Evaluates a program that must raise and returns the error kind.
fn error_kind(engine: &mut Engine, source: &str) -> ErrorKind {
    let module = engine.compile("program.scm", source).unwrap();
    engine.eval(&module).unwrap_err().kind()
}

#[test]
fn the_endianness_syntax_and_native_endianness() {
    let mut engine = engine_with_bytevector();
    // (endianness little) and (endianness big) evaluate to the symbols the
    // accessors accept. native-endianness reports the host byte order.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (and (eq? (endianness little) 'little)
                 (eq? (endianness big) 'big)
                 (if (memq (native-endianness) '(little big)) #t #f))
            "#,
        ),
        Value::boolean(true)
    );
    // An unknown endianness symbol is rejected at expansion time.
    assert!(
        engine
            .compile(
                "program.scm",
                "(import (scheme bytevector)) (endianness middle)"
            )
            .is_err()
    );
    // The plain symbol spelling works at runtime too, but an unsupported
    // symbol raises when the accessor runs.
    assert_eq!(
        error_kind(
            &mut engine,
            r#"
            (import (scheme bytevector))
            (bytevector-u16-ref (u8-list->bytevector '(1 2)) 0 'middle)
            "#,
        ),
        ErrorKind::TypeError
    );
}

#[test]
fn make_bytevector_and_fill_accept_the_signed_range() {
    let mut engine = engine_with_bytevector();
    // The R6RS fill argument spans -128 through 255. A negative fill stores
    // its two's complement.
    assert_eq!(
        show(
            &mut engine,
            "(import (scheme bytevector)) (make-bytevector 4 -1)"
        ),
        "#u8(255 255 255 255)"
    );
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (let ((b (make-bytevector 3 0)))
              (bytevector-fill! b -128)
              b)
            "#,
        ),
        "#u8(128 128 128)"
    );
    // The widened fill is a conformant extension of (scheme base) itself.
    assert_eq!(
        run(
            &mut engine,
            "(import (scheme base)) (bytevector-u8-ref (make-bytevector 1 -128) 0)"
        ),
        Value::integer(128)
    );
    // Out-of-range fills raise.
    for program in [
        "(import (scheme bytevector)) (make-bytevector 1 -129)",
        "(import (scheme bytevector)) (make-bytevector 1 256)",
        "(import (scheme bytevector) (scheme base)) (bytevector-fill! (make-bytevector 1) 256)",
    ] {
        assert_eq!(error_kind(&mut engine, program), ErrorKind::RangeError);
    }
}

#[test]
fn bytevector_equality_compares_length_and_content() {
    let mut engine = engine_with_bytevector();
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (and (bytevector=? (bytevector 1 2 3) (bytevector 1 2 3))
                 (not (bytevector=? (bytevector 1 2 3) (bytevector 1 2)))
                 (not (bytevector=? (bytevector 1 2 3) (bytevector 1 2 4))))
            "#,
        ),
        Value::boolean(true)
    );
    assert_eq!(
        error_kind(
            &mut engine,
            "(import (scheme bytevector)) (bytevector=? 1 (u8-list->bytevector '()))"
        ),
        ErrorKind::TypeError
    );
}

#[test]
fn signed_and_unsigned_bytes_share_the_same_storage() {
    let mut engine = engine_with_bytevector();
    // The R6RS examples: a byte is read either as an octet or as its signed
    // two's-complement interpretation.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (let ((b1 (make-bytevector 16 -127))
                  (b2 (make-bytevector 16 255)))
              (bytevector-s8-set! b2 0 -126)
              (bytevector-u8-set! b2 1 246)
              (list (bytevector-s8-ref b1 0)
                    (bytevector-u8-ref b1 0)
                    (bytevector-s8-ref b2 0)
                    (bytevector-u8-ref b2 0)
                    (bytevector-s8-ref b2 1)
                    (bytevector-u8-ref b2 1)))
            "#,
        ),
        "(-127 129 -126 130 -10 246)"
    );
    // A signed byte outside -128..127 raises.
    assert_eq!(
        error_kind(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (bytevector-s8-set! (make-bytevector 1) 0 128)
            "#,
        ),
        ErrorKind::RangeError
    );
}

#[test]
fn converting_between_bytevectors_and_lists() {
    let mut engine = engine_with_bytevector();
    // u8-list->bytevector and bytevector->u8-list are inverses. The sized
    // conversions decode fixed-width integers, signed or unsigned, in the
    // requested byte order. The examples come from the R6RS chapter.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (let ((b (u8-list->bytevector '(1 2 3 255 1 2 1 2))))
              (and (equal? (bytevector->u8-list b) '(1 2 3 255 1 2 1 2))
                   (equal? (bytevector->sint-list b (endianness little) 2)
                           '(513 -253 513 513))
                   (equal? (bytevector->uint-list b (endianness little) 2)
                           '(513 65283 513 513))
                   (bytevector=? b (sint-list->bytevector
                                     '(513 -253 513 513) (endianness little) 2))
                   (bytevector=? b (uint-list->bytevector
                                     '(513 65283 513 513) (endianness little) 2))))
            "#,
        ),
        Value::boolean(true)
    );
    // The bytevector length must divide evenly into elements.
    assert_eq!(
        error_kind(
            &mut engine,
            r#"
            (import (scheme bytevector))
            (bytevector->uint-list (u8-list->bytevector '(1 2 3)) 'little 2)
            "#,
        ),
        ErrorKind::RangeError
    );
    // An element outside the width raises.
    assert_eq!(
        error_kind(
            &mut engine,
            "(import (scheme bytevector)) (uint-list->bytevector '(65536) 'little 2)"
        ),
        ErrorKind::RangeError
    );
    // A circular list raises instead of spinning.
    assert_eq!(
        error_kind(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (define ring (list 1 2 3))
            (set-cdr! (cddr ring) ring)
            (u8-list->bytevector ring)
            "#,
        ),
        ErrorKind::TypeError
    );
}

#[test]
fn arbitrary_size_integers_up_to_the_i128_window() {
    let mut engine = engine_with_bytevector();
    // Sixteen-byte encodings work for every value that fits the engine's
    // i128 exact integers. Storing -3 reproduces the byte pattern of the
    // R6RS example, whose unsigned reading is 2^128 - 3.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (let ((b (make-bytevector 16 -127)))
              (bytevector-sint-set! b 0 -3 (endianness little) 16)
              (bytevector->u8-list b))
            "#,
        ),
        "(253 255 255 255 255 255 255 255 255 255 255 255 255 255 255 255)"
    );
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (let ((b (make-bytevector 16 0)))
              (bytevector-sint-set! b 0 -3 (endianness big) 16)
              (and (= (bytevector-sint-ref b 0 (endianness big) 16) -3)
                   ;; Redundant leading zero bytes keep a wide unsigned read
                   ;; representable.
                   (= (bytevector-uint-ref (make-bytevector 16 0)
                                           0 (endianness big) 16)
                      0)))
            "#,
        ),
        Value::boolean(true)
    );
    // The unsigned reading of that pattern is 2^128 - 3, which is outside
    // the exact integer range and raises an implementation restriction.
    assert_eq!(
        error_kind(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (let ((b (make-bytevector 16 0)))
              (bytevector-sint-set! b 0 -3 (endianness little) 16)
              (bytevector-uint-ref b 0 (endianness little) 16))
            "#,
        ),
        ErrorKind::ImplementationRestriction
    );
    // A zero size and a range past the end raise.
    assert_eq!(
        error_kind(
            &mut engine,
            "(import (scheme bytevector)) (bytevector-uint-ref (u8-list->bytevector '(1)) 0 'little 0)"
        ),
        ErrorKind::RangeError
    );
    assert_eq!(
        error_kind(
            &mut engine,
            "(import (scheme bytevector)) (bytevector-uint-ref (u8-list->bytevector '(1)) 0 'little 2)"
        ),
        ErrorKind::RangeError
    );
}

#[test]
fn fixed_width_integers_in_both_byte_orders() {
    let mut engine = engine_with_bytevector();
    // The R6RS 16, 32, and 64 bit examples over the byte pattern of the
    // big-endian encoding of 2^128 - 3 (stored here as signed -3).
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (let ((b (make-bytevector 16 -127)))
              (bytevector-sint-set! b 0 -3 (endianness big) 16)
              (and (= (bytevector-u16-ref b 14 (endianness little)) 65023)
                   (= (bytevector-s16-ref b 14 (endianness little)) -513)
                   (= (bytevector-u16-ref b 14 (endianness big)) 65533)
                   (= (bytevector-s16-ref b 14 (endianness big)) -3)
                   (= (bytevector-u32-ref b 12 (endianness little)) 4261412863)
                   (= (bytevector-s32-ref b 12 (endianness little)) -33554433)
                   (= (bytevector-u32-ref b 12 (endianness big)) 4294967293)
                   (= (bytevector-s32-ref b 12 (endianness big)) -3)
                   (= (bytevector-u64-ref b 8 (endianness little))
                      18302628885633695743)
                   (= (bytevector-s64-ref b 8 (endianness little))
                      -144115188075855873)
                   (= (bytevector-u64-ref b 8 (endianness big))
                      18446744073709551613)
                   (= (bytevector-s64-ref b 8 (endianness big)) -3)))
            "#,
        ),
        Value::boolean(true)
    );
    // set! and ref roundtrip in both byte orders.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (let ((b (make-bytevector 8 0)))
              (bytevector-u16-set! b 0 44034 (endianness little))
              (bytevector-s16-set! b 2 -1000 (endianness big))
              (bytevector-u32-set! b 4 3000000000 (endianness little))
              (and (= (bytevector-u16-ref b 0 (endianness little)) 44034)
                   (= (bytevector-s16-ref b 2 (endianness big)) -1000)
                   (= (bytevector-u32-ref b 4 (endianness little)) 3000000000)))
            "#,
        ),
        Value::boolean(true)
    );
    // A value outside the representable range of the width raises.
    for program in [
        "(import (scheme bytevector) (scheme base)) (bytevector-u16-set! (make-bytevector 2) 0 65536 'little)",
        "(import (scheme bytevector) (scheme base)) (bytevector-s16-set! (make-bytevector 2) 0 32768 'little)",
        "(import (scheme bytevector) (scheme base)) (bytevector-u64-set! (make-bytevector 8) 0 -1 'little)",
        "(import (scheme bytevector) (scheme base)) (bytevector-s64-set! (make-bytevector 8) 0 (expt 2 63) 'little)",
    ] {
        assert_eq!(error_kind(&mut engine, program), ErrorKind::RangeError);
    }
}

#[test]
fn native_forms_use_the_host_byte_order_and_require_alignment() {
    let mut engine = engine_with_bytevector();
    // The -native- accessors agree with the explicit form under
    // (native-endianness) and demand an index aligned to the width.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (let ((b (make-bytevector 16 0)))
              (bytevector-u16-native-set! b 2 513)
              (bytevector-u32-native-set! b 4 66051)
              (bytevector-u64-native-set! b 8 72057594037927941)
              (and (= (bytevector-u16-ref b 2 (native-endianness)) 513)
                   (= (bytevector-u16-native-ref b 2) 513)
                   (= (bytevector-u32-native-ref b 4) 66051)
                   (= (bytevector-u64-native-ref b 8) 72057594037927941)))
            "#,
        ),
        Value::boolean(true)
    );
    for program in [
        "(import (scheme bytevector) (scheme base)) (bytevector-u16-native-ref (make-bytevector 4) 1)",
        "(import (scheme bytevector) (scheme base)) (bytevector-u32-native-ref (make-bytevector 8) 2)",
        "(import (scheme bytevector) (scheme base)) (bytevector-u64-native-set! (make-bytevector 16) 4 0)",
        "(import (scheme bytevector) (scheme base)) (bytevector-ieee-double-native-ref (make-bytevector 16) 4)",
    ] {
        assert_eq!(error_kind(&mut engine, program), ErrorKind::RangeError);
    }
}

#[test]
fn ieee_754_accessors_store_singles_and_doubles() {
    let mut engine = engine_with_bytevector();
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (let ((b (make-bytevector 16 0)))
              (bytevector-ieee-double-set! b 0 1.5 (endianness little))
              (bytevector-ieee-double-set! b 8 -0.0 (endianness big))
              (and (= (bytevector-ieee-double-ref b 0 (endianness little)) 1.5)
                   (= (bytevector-ieee-double-ref b 8 (endianness big)) 0.0)
                   (eqv? (bytevector-ieee-double-ref b 8 (endianness big)) -0.0)))
            "#,
        ),
        Value::boolean(true)
    );
    // Singles roundtrip representable values exactly and round others. An
    // exact argument is accepted and converted. Infinities survive.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (let ((b (make-bytevector 8 0)))
              (bytevector-ieee-single-set! b 0 1.5 (endianness big))
              (bytevector-ieee-single-set! b 4 +inf.0 (endianness little))
              (bytevector-ieee-single-native-set! b 0 (bytevector-ieee-single-ref b 0 (endianness big)))
              (and (= (bytevector-ieee-single-native-ref b 0) 1.5)
                   (= (bytevector-ieee-single-ref b 4 (endianness little)) +inf.0)))
            "#,
        ),
        Value::boolean(true)
    );
    // NaN survives as NaN. The engine canonicalizes NaN payload bits, so
    // only nan-ness is preserved, not the exact bit pattern.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base) (scheme inexact))
            (let ((b (make-bytevector 8 0)))
              (bytevector-ieee-double-set! b 0 +nan.0 (endianness little))
              (nan? (bytevector-ieee-double-ref b 0 (endianness little))))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn utf16_and_utf32_transcoding() {
    let mut engine = engine_with_bytevector();
    // string->utf16 defaults to big-endian and never emits a byte order
    // mark. A character outside the basic plane becomes a surrogate pair.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (and (equal? (bytevector->u8-list (string->utf16 "A")) '(0 65))
                 (equal? (bytevector->u8-list (string->utf16 "A" (endianness little)))
                         '(65 0))
                 (equal? (bytevector->u8-list (string->utf16 "\x1F600;"))
                         '(216 61 222 0))
                 (equal? (bytevector->u8-list (string->utf32 "A")) '(0 0 0 65))
                 (equal? (bytevector->u8-list (string->utf32 "A" (endianness little)))
                         '(65 0 0 0)))
            "#,
        ),
        Value::boolean(true)
    );
    // Decoding honors a byte order mark unless the mandatory flag is set,
    // in which case the mark decodes as a regular character.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (let ((bom-le (u8-list->bytevector '(255 254 65 0)))
                  (bom-be (u8-list->bytevector '(254 255 0 65))))
              (and (string=? (utf16->string bom-le (endianness big)) "A")
                   (string=? (utf16->string bom-be (endianness little)) "A")
                   (= (string-length (utf16->string bom-be (endianness big) #t)) 2)
                   (string=? (utf32->string
                               (u8-list->bytevector '(255 254 0 0 65 0 0 0))
                               (endianness big))
                             "A")))
            "#,
        ),
        Value::boolean(true)
    );
    // Invalid or incomplete encodings decode to U+FFFD replacement
    // characters: a lone surrogate, a trailing odd byte, and an out-of-range
    // UTF-32 scalar.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (and (string=? (utf16->string (u8-list->bytevector '(216 61))
                                          (endianness big) #t)
                           "\xFFFD;")
                 (string=? (utf16->string (u8-list->bytevector '(0 65 99))
                                          (endianness big) #t)
                           "A\xFFFD;")
                 (string=? (utf32->string (u8-list->bytevector '(0 17 0 0))
                                          (endianness big) #t)
                           "\xFFFD;"))
            "#,
        ),
        Value::boolean(true)
    );
    // The UTF-8 conversions of (scheme base) are re-exported unchanged.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (string=? (utf8->string (string->utf8 "grüße")) "grüße")
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn literal_bytevectors_are_immutable() {
    let mut engine = engine_with_bytevector();
    for program in [
        "(import (scheme bytevector) (scheme base)) (bytevector-u16-set! #u8(1 2) 0 5 (endianness little))",
        "(import (scheme bytevector)) (bytevector-fill! #u8(1 2) 3)",
        "(import (scheme bytevector)) (bytevector-s8-set! #u8(1 2) 0 -1)",
    ] {
        assert_eq!(error_kind(&mut engine, program), ErrorKind::RuntimeError);
    }
}

#[test]
fn joint_import_with_scheme_base_needs_no_renames() {
    let mut engine = engine_with_bytevector();
    // The overlap is re-exported from (scheme base), so both libraries can
    // be imported together. bytevector-copy! keeps the R7RS-small argument
    // order (to at from start end).
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme base) (scheme bytevector))
            (let ((b (make-bytevector 4 0)))
              (bytevector-copy! b 2 (u8-list->bytevector '(1 2)) 0 2)
              (bytevector-u16-ref b 2 (endianness little)))
            "#,
        ),
        Value::integer(513)
    );
}

#[test]
fn the_extension_advertises_its_feature() {
    let mut engine = engine_with_bytevector();
    // Installing the extension enables the scheme-bytevector cond-expand
    // feature.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (scheme bytevector) (scheme base))
            (cond-expand (scheme-bytevector #t) (else #f))
            "#,
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (scheme base)) (if (memq 'scheme-bytevector (features)) #t #f)"
        ),
        Value::boolean(true)
    );
}

#[test]
fn a_bare_engine_cannot_import_the_extension() {
    // Without install_extension neither library name is available.
    let mut bare = Engine::new(EngineConfig::default()).unwrap();
    for program in [
        "(import (scheme bytevector)) 1",
        "(import (r7rs bytevector)) 1",
    ] {
        assert!(
            bare.compile("program.scm", program).is_err(),
            "{program} without the extension should fail"
        );
    }
}
