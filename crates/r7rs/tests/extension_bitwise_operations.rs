//! Usage examples for the SRFI 151 (Bitwise Operations) extension. The
//! examples import it through the `(r7rs bitwise-operations)` alias. The canonical
//! `(srfi 151)` name provides the identical library.

use r7rs::{Engine, EngineConfig, Extension, Value};

/// Builds an engine with SRFI 151 installed, the usual first step for any script
/// that needs bitwise operations.
fn engine_with_srfi151() -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Srfi151).unwrap();
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
fn the_logical_family_lifts_boolean_functions_over_bits() {
    let mut engine = engine_with_srfi151();
    // bitwise-and, bitwise-ior, bitwise-xor, and bitwise-eqv are associative and
    // n-ary. With no arguments they return their identity element.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs bitwise-operations) (scheme base))
            (and (= (bitwise-not 10) -11)
                 (= (bitwise-and 11 26) 10)
                 (= (bitwise-ior 3 10) 11)
                 (= (bitwise-xor 3 10) 9)
                 (= (bitwise-eqv 37 12) -42)
                 (= (bitwise-and 255 15 3) 3)
                 (= (bitwise-and) -1)
                 (= (bitwise-ior) 0)
                 (= (bitwise-xor) 0))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn the_non_associative_dyadic_operators() {
    let mut engine = engine_with_srfi151();
    // The nand, nor, and and-with-complement operators take exactly two
    // arguments.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs bitwise-operations) (scheme base))
            (and (= (bitwise-nand 11 26) -11)
                 (= (bitwise-nor 11 26) -28)
                 (= (bitwise-andc1 11 26) 16)
                 (= (bitwise-andc2 11 26) 1)
                 (= (bitwise-orc1 11 26) -2)
                 (= (bitwise-orc2 11 26) -17))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn arithmetic_shift_moves_bits_in_either_direction() {
    let mut engine = engine_with_srfi151();
    // A positive count shifts left, a negative count shifts right while keeping
    // the sign. The large-negative example comes straight from the SRFI.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs bitwise-operations) (scheme base))
            (and (= (arithmetic-shift 8 2) 32)
                 (= (arithmetic-shift 4 0) 4)
                 (= (arithmetic-shift 8 -1) 4)
                 (= (arithmetic-shift -100000000000000000000000000000000 -100) -79))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn counting_and_measuring_bits() {
    let mut engine = engine_with_srfi151();
    // bit-count counts ones for non-negative inputs and zeros for negative ones.
    // integer-length reports the bits needed without the sign, and first-set-bit
    // finds the lowest set bit or -1 for zero. All work on heap-sized integers.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs bitwise-operations) (scheme base))
            (and (= (bit-count 13) 3)
                 (= (bit-count -13) 2)
                 (= (bit-count (expt 2 100)) 1)
                 (= (bit-count (- (expt 2 100))) 100)
                 (= (integer-length 7) 3)
                 (= (integer-length -8) 3)
                 (= (first-set-bit 40) 3)
                 (= (first-set-bit -28) 2)
                 (= (first-set-bit 0) -1))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn merging_and_testing_single_bits() {
    let mut engine = engine_with_srfi151();
    // bitwise-if selects each result bit from i or j according to mask; the
    // single-bit operators read, copy, swap, and test individual bits.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs bitwise-operations) (scheme base))
            (and (= (bitwise-if #b00111100 #b11110000 #b00001111) #b00110011)
                 (eq? (bit-set? 0 6) #f)
                 (eq? (bit-set? 2 6) #t)
                 (= (copy-bit 2 #b1111 #f) #b1011)
                 (= (bit-swap 0 2 4) 1)
                 (eq? (any-bit-set? 3 6) #t)
                 (eq? (any-bit-set? 3 12) #f)
                 (eq? (every-bit-set? 4 6) #t)
                 (eq? (every-bit-set? 7 6) #f))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn extracting_and_editing_bit_fields() {
    let mut engine = engine_with_srfi151();
    // A field is the half-open range [start, end). bit-field extracts it, and the
    // clear/set/replace/rotate/reverse operators edit it in place.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs bitwise-operations) (scheme base))
            (and (= (bit-field #b1101101010 0 4) #b1010)
                 (= (bit-field #b1101101010 3 9) #b101101)
                 (= (bit-field 6 2 999) 1)
                 (eq? (bit-field-any? #b1001001 1 6) #t)
                 (eq? (bit-field-every? #b1011110 1 5) #t)
                 (= (bit-field-clear #b101010 1 4) #b100000)
                 (= (bit-field-set #b101010 1 4) #b101110)
                 (= (bit-field-replace #b101010 #b010 1 4) #b100100)
                 (= (bit-field-replace-same #b1111 #b0000 1 3) #b1001)
                 (= (bit-field-rotate #b110 1 2 4) #b1010)
                 (= (bit-field-reverse 6 1 4) 12)
                 (= (bit-field-reverse 1 0 32) #x80000000)
                 (= (bit-field-set 0 126 127) (expt 2 126)))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn converting_between_integers_and_boolean_sequences() {
    let mut engine = engine_with_srfi151();
    // bits->list and bits->vector expand an integer into little-endian booleans,
    // and list->bits, vector->bits, and bits pack them back. bits->list and
    // list->bits are inverses for positive integers.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs bitwise-operations) (scheme base))
            (bits->list #b1110101)
            "#,
        ),
        "(#t #f #t #f #t #t #t)"
    );
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs bitwise-operations) (scheme base))
            (and (equal? (bits->list 3 5) '(#t #t #f #f #f))
                 (equal? (bits->vector #b1110101) #(#t #f #t #f #t #t #t))
                 (= (list->bits '(#t #f #t #f #t #t #t)) #b1110101)
                 (= (vector->bits #(#f #t #t)) 6)
                 (= (bits #t #f #t #f #t #t #t) #b1110101)
                 (= (list->bits (bits->list 6)) 6))
            "#,
        ),
        Value::boolean(true)
    );
}

#[test]
fn bit_inputs_reject_non_booleans() {
    let mut engine = engine_with_srfi151();
    for source in [
        "(import (r7rs bitwise-operations)) (copy-bit 0 0 1)",
        "(import (r7rs bitwise-operations)) (list->bits '(#t 1))",
    ] {
        let module = engine.compile("program.scm", source).unwrap();
        assert!(
            engine.eval(&module).is_err(),
            "non-boolean bit input should raise: {source}"
        );
    }
}

#[test]
fn folding_and_iterating_over_bits() {
    let mut engine = engine_with_srfi151();
    // bitwise-fold threads an accumulator through the bits from bit 0 up, and
    // bitwise-for-each applies a procedure for its side effects.
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs bitwise-operations) (scheme base))
            (bitwise-fold cons '() #b1010111)
            "#,
        ),
        "(#t #f #t #f #t #t #t)"
    );
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs bitwise-operations) (scheme base))
            (let ((count 0))
              (bitwise-for-each (lambda (b) (if b (set! count (+ count 1))))
                                #b1010111)
              count)
            "#,
        ),
        Value::integer(5)
    );
}

#[test]
fn unfolding_and_generating_bits() {
    let mut engine = engine_with_srfi151();
    // bitwise-unfold builds an integer bit by bit from a state, and
    // make-bitwise-generator returns a thunk yielding successive bits.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs bitwise-operations) (scheme base))
            (bitwise-unfold (lambda (i) (= i 10))
                            even?
                            (lambda (i) (+ i 1))
                            0)
            "#,
        ),
        Value::integer(0b101010101)
    );
    assert_eq!(
        show(
            &mut engine,
            r#"
            (import (r7rs bitwise-operations) (scheme base))
            (let ((g (make-bitwise-generator #b110)))
              (list (g) (g) (g) (g)))
            "#,
        ),
        "(#f #t #t #f)"
    );
}

#[test]
fn a_result_beyond_the_integer_range_is_refused() {
    let mut engine = engine_with_srfi151();
    // Exact integers here are bounded by i128, so a shift whose result would not
    // fit raises rather than wrapping.
    let module = engine
        .compile(
            "program.scm",
            "(import (r7rs bitwise-operations)) (arithmetic-shift 1 200)",
        )
        .unwrap();
    assert!(
        engine.eval(&module).is_err(),
        "an overflowing shift should raise"
    );
}

#[test]
fn the_extension_advertises_its_feature() {
    let mut engine = engine_with_srfi151();
    // Installing the extension enables the srfi-151 cond-expand feature.
    assert_eq!(
        run(
            &mut engine,
            r#"
            (import (r7rs bitwise-operations) (scheme base))
            (cond-expand (srfi-151 #t) (else #f))
            "#,
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (r7rs bitwise-operations) (scheme base)) (if (memq 'srfi-151 (features)) #t #f)"
        ),
        Value::boolean(true)
    );
}

#[test]
fn a_bare_engine_cannot_import_the_extension() {
    // Without install_extension neither library name is available.
    let mut bare = Engine::new(EngineConfig::default()).unwrap();
    for program in [
        "(import (srfi 151)) 1",
        "(import (r7rs bitwise-operations)) 1",
    ] {
        assert!(
            bare.compile("program.scm", program).is_err(),
            "{program} without the extension should fail"
        );
    }
}
