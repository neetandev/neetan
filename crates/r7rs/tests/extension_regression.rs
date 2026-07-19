use r7rs::{Engine, EngineConfig, Extension};

/// Builds an engine with a single extension installed.
fn engine_with(extension: Extension) -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(extension).unwrap();
    engine
}

/// Returns whether a program evaluates without error. A malformed program that
/// fails to compile counts as an error too.
fn evaluates(engine: &mut Engine, source: &str) -> bool {
    match engine.compile("program.scm", source) {
        Ok(module) => engine.eval(&module).is_ok(),
        Err(_) => false,
    }
}

/// Fact one: importing `(srfi 1)` binds the specified procedures. A
/// representative slice is checked here, spanning the re-exported R7RS overlap,
/// the native structural primitives, and the Scheme-defined higher-order
/// procedures.
#[test]
fn srfi_1_public_import_exposes_the_specified_bindings() {
    let mut engine = engine_with(Extension::Srfi1);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (srfi 1))
        (and (procedure? cons) (procedure? cadddr)
             (procedure? xcons) (procedure? cons*) (procedure? take)
             (procedure? take-right) (procedure? drop-right) (procedure? last)
             (procedure? length+) (procedure? append-reverse)
             (procedure? circular-list?) (procedure? dotted-list?)
             (procedure? proper-list?) (procedure? null-list?)
             (procedure? iota) (procedure? list-tabulate)
             (procedure? first) (procedure? tenth) (procedure? split-at)
             (procedure? fold) (procedure? fold-right) (procedure? reduce)
             (procedure? unfold) (procedure? pair-fold) (procedure? append-map)
             (procedure? filter-map) (procedure? filter) (procedure? partition)
             (procedure? remove) (procedure? find) (procedure? any)
             (procedure? every) (procedure? count) (procedure? delete)
             (procedure? delete-duplicates) (procedure? alist-cons)
             (procedure? lset-union) (procedure? lset-intersection)
             (procedure? filter!) (procedure? delete-duplicates!))
        "#,
    ));
}

/// Fact two: importing `(srfi 1)` does not leak the wrapper's private helpers.
/// The `%srfi1-` helpers used to implement the higher-order procedures are not
/// exported, so a reference to one through the public import is an
/// unbound-variable error.
#[test]
fn srfi_1_public_import_does_not_leak_internal_helpers() {
    let mut engine = engine_with(Extension::Srfi1);
    for helper in [
        "%srfi1-cars",
        "%srfi1-cdrs",
        "%srfi1-any-null?",
        "%srfi1-member?",
        "%srfi1-subset?",
        "%srfi1-list=2",
    ] {
        let source = format!("(import (srfi 1)) {helper}");
        assert!(
            !evaluates(&mut engine, &source),
            "public import leaks internal helper {helper}"
        );
    }
}

/// Fact three: installing SRFI 1 also registers the `(r7rs lists)` alias, which
/// provides the same library under a discoverable name.
#[test]
fn srfi_1_alias_r7rs_list_provides_the_same_library() {
    let mut engine = engine_with(Extension::Srfi1);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme base) (r7rs lists))
        (unless (= 10 (fold + 0 (iota 5))) (error "alias broken"))
        "#,
    ));
}

/// Fact one: importing `(srfi 2)` binds `and-let*`. It is syntax, not a
/// procedure, so the fact is pinned by using the macro rather than probing with
/// `procedure?`.
#[test]
fn srfi_2_public_import_exposes_the_specified_bindings() {
    let mut engine = engine_with(Extension::Srfi2);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (srfi 2) (scheme base))
        (and (equal? 3 (and-let* ((x 1) (y 2)) (+ x y)))
             (eq? #f (and-let* ((x #f)) x)))
        "#,
    ));
}

/// Fact two: importing `(srfi 2)` binds only `and-let*`. The wrapper has no `%`
/// helpers, so a positive control confirms `and-let*` can be cherry-picked and a
/// negative control confirms an unrelated name is not smuggled in.
#[test]
fn srfi_2_public_import_does_not_bind_unspecified_names() {
    let mut engine = engine_with(Extension::Srfi2);
    assert!(evaluates(
        &mut engine,
        "(import (only (srfi 2) and-let*)) #t"
    ));
    assert!(
        !evaluates(&mut engine, "(import (only (srfi 2) and-let)) #t"),
        "the library binds a name other than and-let*"
    );
}

/// Fact three: installing SRFI 2 also registers the `(r7rs and-let*)` alias,
/// which provides the same library under a discoverable name.
#[test]
fn srfi_2_alias_r7rs_and_let_provides_the_same_library() {
    let mut engine = engine_with(Extension::Srfi2);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme base) (r7rs and-let*))
        (unless (= 3 (and-let* ((x 1) (y 2)) (+ x y))) (error "alias broken"))
        "#,
    ));
}

/// Fact one: importing `(srfi 8)` binds `receive`. It is syntax, not a
/// procedure, so the fact is pinned by using the macro rather than probing with
/// `procedure?`.
#[test]
fn srfi_8_public_import_exposes_the_specified_bindings() {
    let mut engine = engine_with(Extension::Srfi8);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (srfi 8) (scheme base))
        (and (equal? '(1 2) (receive (a b) (values 1 2) (list a b)))
             (equal? '(1 2 3) (receive all (values 1 2 3) all)))
        "#,
    ));
}

/// Fact two: importing `(srfi 8)` binds only `receive`. The wrapper has no `%`
/// helpers, so a positive control confirms `receive` can be cherry-picked and a
/// negative control confirms an unrelated name is not smuggled in.
#[test]
fn srfi_8_public_import_does_not_bind_unspecified_names() {
    let mut engine = engine_with(Extension::Srfi8);
    assert!(evaluates(
        &mut engine,
        "(import (only (srfi 8) receive)) #t"
    ));
    assert!(
        !evaluates(&mut engine, "(import (only (srfi 8) receive-values)) #t"),
        "the library binds a name other than receive"
    );
}

/// Fact three: installing SRFI 8 also registers the `(r7rs receive)` alias,
/// which provides the same library under a discoverable name.
#[test]
fn srfi_8_alias_r7rs_receive_provides_the_same_library() {
    let mut engine = engine_with(Extension::Srfi8);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme base) (r7rs receive))
        (unless (= 3 (receive (a b) (values 1 2) (+ a b))) (error "alias broken"))
        "#,
    ));
}

/// Fact one: importing `(srfi 26)` binds `cut` and `cute`. They are syntax, not
/// procedures, so the fact is pinned by using the macros rather than probing with
/// `procedure?`.
#[test]
fn srfi_26_public_import_exposes_the_specified_bindings() {
    let mut engine = engine_with(Extension::Srfi26);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (srfi 26) (scheme base))
        (and (equal? '(1 2 3 4) ((cut list 1 <> 3 <>) 2 4))
             (equal? '(1 . 2) ((cute cons 1 <>) 2)))
        "#,
    ));
}

/// Fact two: importing `(srfi 26)` binds only `cut` and `cute`. The wrapper has
/// no helper macros, so a positive control cherry-picks both and a negative
/// control confirms an unrelated name is not smuggled in.
#[test]
fn srfi_26_public_import_does_not_bind_unspecified_names() {
    let mut engine = engine_with(Extension::Srfi26);
    assert!(evaluates(
        &mut engine,
        "(import (only (srfi 26) cut cute)) #t"
    ));
    assert!(
        !evaluates(&mut engine, "(import (only (srfi 26) cut*)) #t"),
        "the library binds a name other than cut and cute"
    );
}

/// Fact three: installing SRFI 26 also registers the `(r7rs cut)` alias, which
/// provides the same library under a discoverable name.
#[test]
fn srfi_26_alias_r7rs_cut_provides_the_same_library() {
    let mut engine = engine_with(Extension::Srfi26);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme base) (r7rs cut))
        (unless (= 6 ((cut + 1 <> 3) 2)) (error "alias broken"))
        "#,
    ));
}

/// Fact one: importing `(srfi 27)` binds exactly the names the SRFI specifies,
/// all of them usable. `default-random-source` is a source object, every other
/// export is a procedure.
#[test]
fn srfi_27_public_import_exposes_the_specified_bindings() {
    let mut engine = engine_with(Extension::Srfi27);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (srfi 27))
        (and (random-source? default-random-source)
             (procedure? random-integer)
             (procedure? random-real)
             (procedure? make-random-source)
             (procedure? random-source?)
             (procedure? random-source-state-ref)
             (procedure? random-source-state-set!)
             (procedure? random-source-randomize!)
             (procedure? random-source-pseudo-randomize!)
             (procedure? random-source-make-integers)
             (procedure? random-source-make-reals))
        "#,
    ));
}

/// Fact two: importing `(srfi 27)` does not leak the private native glue. The
/// helpers that live in the undocumented `(srfi 27 native)` library back the
/// public bindings but must not be reachable through the public import, so a
/// reference to any of them is an unbound-variable error.
#[test]
fn srfi_27_public_import_does_not_leak_internal_helpers() {
    let mut engine = engine_with(Extension::Srfi27);
    for helper in [
        "%default-random-source",
        "%random-integer-on",
        "%random-real-on",
    ] {
        let source = format!("(import (srfi 27)) {helper}");
        assert!(
            !evaluates(&mut engine, &source),
            "public import leaks internal helper {helper}"
        );
    }
}

/// Fact three: installing SRFI 27 also registers the `(r7rs random-bits)` alias,
/// which provides the same library under a discoverable name.
#[test]
fn srfi_27_alias_r7rs_random_provides_the_same_library() {
    let mut engine = engine_with(Extension::Srfi27);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme base) (r7rs random-bits))
        (unless (<= 0 (random-integer 6) 5) (error "alias broken"))
        "#,
    ));
}

/// Fact one: importing `(srfi 48)` binds the single specified procedure,
/// `format`, usable with both the SRFI 28 subset and the SRFI 48 additions.
#[test]
fn srfi_48_public_import_exposes_the_specified_bindings() {
    let mut engine = engine_with(Extension::Srfi48);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme base) (srfi 48))
        (unless (and (procedure? format)
                     (string=? "1" (format "~a" 1))
                     (string=? "ff" (format "~x" 255)))
          (error "srfi 48 bindings broken"))
        "#,
    ));
}

/// Fact two: neither the public import nor the compatibility name leaks the
/// private native glue. The template walker is registered under `%format48` in
/// the undocumented `(srfi 48 native)` library, so a reference to it through
/// any public import is an unbound-variable error.
#[test]
fn srfi_48_public_import_does_not_leak_internal_helpers() {
    let mut engine = engine_with(Extension::Srfi48);
    for source in [
        "(import (srfi 48)) %format48",
        "(import (srfi 28)) %format48",
        "(import (r7rs intermediate-format-strings)) %format48",
    ] {
        assert!(
            !evaluates(&mut engine, source),
            "public import leaks internal helper: {source}"
        );
    }
}

/// Fact three: installing SRFI 48 also registers the `(r7rs intermediate-format-strings)` alias,
/// which provides the same library under a discoverable name.
#[test]
fn srfi_48_alias_r7rs_format_provides_the_same_library() {
    let mut engine = engine_with(Extension::Srfi48);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme base) (r7rs intermediate-format-strings))
        (unless (string=? "1-2" (format "~a-~a" 1 2)) (error "alias broken"))
        "#,
    ));
}

/// Fact four: installing SRFI 48 also registers `(srfi 28)` as a compatibility
/// name binding the same `format`, because SRFI 48 is the upward-compatible
/// revision of SRFI 28.
#[test]
fn srfi_48_registers_the_srfi_28_compatibility_name() {
    let mut engine = engine_with(Extension::Srfi48);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme base) (srfi 28))
        (unless (string=? "1-2" (format "~a-~a" 1 2)) (error "compat name broken"))
        "#,
    ));
}

/// Fact one: importing `(srfi 69)` binds the specified procedures. A
/// representative slice is checked here, spanning the table operations and the
/// four hash functions, ending in a real use of the table.
#[test]
fn srfi_69_public_import_exposes_the_specified_bindings() {
    let mut engine = engine_with(Extension::Srfi69);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (srfi 69))
        (and (procedure? make-hash-table) (procedure? hash-table?)
             (procedure? alist->hash-table)
             (procedure? hash-table-equivalence-function)
             (procedure? hash-table-hash-function)
             (procedure? hash-table-ref) (procedure? hash-table-ref/default)
             (procedure? hash-table-set!) (procedure? hash-table-delete!)
             (procedure? hash-table-exists?) (procedure? hash-table-update!)
             (procedure? hash-table-update!/default) (procedure? hash-table-size)
             (procedure? hash-table-keys) (procedure? hash-table-values)
             (procedure? hash-table-walk) (procedure? hash-table-fold)
             (procedure? hash-table->alist) (procedure? hash-table-copy)
             (procedure? hash-table-merge!) (procedure? hash)
             (procedure? string-hash) (procedure? string-ci-hash)
             (procedure? hash-by-identity)
             (= 7 (hash-table-ref/default (make-hash-table) 'k 7)))
        "#,
    ));
}

/// Fact two: importing `(srfi 69)` does not leak the wrapper's private helpers.
/// The `%`-prefixed table internals and the record accessors that are not SRFI
/// names are unexported, so a reference to one through the public import is an
/// unbound-variable error.
#[test]
fn srfi_69_public_import_does_not_leak_internal_helpers() {
    let mut engine = engine_with(Extension::Srfi69);
    for helper in [
        "%make-hash-table",
        "%hash-table-find",
        "%hash-table-add!",
        "%hash-table-remove!",
        "%appropriate-hash-function-for",
        "%hash-table-association-function",
        "%hash-table-entries",
        "%hash-table-set-size!",
        "%hash-table-set-entries!",
        "%hash-node-key",
        "<srfi-69-hash-table>",
    ] {
        let source = format!("(import (srfi 69)) {helper}");
        assert!(
            !evaluates(&mut engine, &source),
            "public import leaks internal helper {helper}"
        );
    }
}

/// Fact three: installing SRFI 69 also registers the `(r7rs basic-hash-table)`
/// alias, which provides the same library under a discoverable name.
#[test]
fn srfi_69_alias_r7rs_hash_table_provides_the_same_library() {
    let mut engine = engine_with(Extension::Srfi69);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme base) (r7rs basic-hash-table))
        (let ((table (make-hash-table)))
          (hash-table-set! table 'answer 42)
          (unless (= 42 (hash-table-ref table 'answer)) (error "alias broken")))
        "#,
    ));
}

/// Fact one: importing `(srfi 132)` binds the specified procedures. A
/// representative slice is checked here, spanning the predicates, the sort and
/// merge families, neighbor-duplicate deletion, the median, and selection,
/// ending in a real sort and merge.
#[test]
fn srfi_132_public_import_exposes_the_specified_bindings() {
    let mut engine = engine_with(Extension::Srfi132);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (srfi 132) (scheme base))
        (and (procedure? list-sorted?) (procedure? vector-sorted?)
             (procedure? list-sort) (procedure? list-stable-sort)
             (procedure? list-sort!) (procedure? list-stable-sort!)
             (procedure? vector-sort) (procedure? vector-stable-sort)
             (procedure? vector-sort!) (procedure? vector-stable-sort!)
             (procedure? list-merge) (procedure? list-merge!)
             (procedure? vector-merge) (procedure? vector-merge!)
             (procedure? list-delete-neighbor-dups)
             (procedure? list-delete-neighbor-dups!)
             (procedure? vector-delete-neighbor-dups)
             (procedure? vector-delete-neighbor-dups!)
             (procedure? vector-find-median) (procedure? vector-find-median!)
             (procedure? vector-select!) (procedure? vector-separate!)
             (equal? '(1 2 3) (list-sort < '(3 1 2)))
             (equal? '(1 2 3 4) (list-merge < '(1 3) '(2 4))))
        "#,
    ));
}

/// Fact two: importing `(srfi 132)` does not leak the wrapper's private helpers.
/// The `%`-prefixed sort, merge, and selection internals are unexported, so a
/// reference to one through the public import is an unbound-variable error.
#[test]
fn srfi_132_public_import_does_not_leak_internal_helpers() {
    let mut engine = engine_with(Extension::Srfi132);
    for helper in [
        "%start",
        "%end",
        "%opt",
        "%append-reverse",
        "%swap!",
        "%merge!",
        "%msort!",
        "%sort-range!",
        "%vmerge!",
        "%default-mean",
        "%median-of-sorted",
        "%partition!",
        "%quickselect!",
    ] {
        let source = format!("(import (srfi 132)) {helper}");
        assert!(
            !evaluates(&mut engine, &source),
            "public import leaks internal helper {helper}"
        );
    }
}

/// Fact three: installing SRFI 132 also registers the `(r7rs sorting)` alias,
/// which provides the same library under a discoverable name.
#[test]
fn srfi_132_alias_r7rs_sorting_provides_the_same_library() {
    let mut engine = engine_with(Extension::Srfi132);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme base) (r7rs sorting))
        (unless (equal? '(1 2 3 5 8) (list-sort < '(5 3 8 1 2)))
          (error "alias broken"))
        "#,
    ));
}

/// Fact one: importing `(srfi 151)` binds the specified procedures. A
/// representative slice is checked here, spanning the native logical, integer,
/// single-bit, and bit-field operations and the Scheme-defined conversions and
/// higher-order procedures, ending in real computations.
#[test]
fn srfi_151_public_import_exposes_the_specified_bindings() {
    let mut engine = engine_with(Extension::Srfi151);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (srfi 151) (scheme base))
        (and (procedure? bitwise-not) (procedure? bitwise-and)
             (procedure? bitwise-ior) (procedure? bitwise-xor)
             (procedure? bitwise-eqv) (procedure? bitwise-nand)
             (procedure? bitwise-nor) (procedure? bitwise-andc1)
             (procedure? bitwise-andc2) (procedure? bitwise-orc1)
             (procedure? bitwise-orc2) (procedure? arithmetic-shift)
             (procedure? bit-count) (procedure? integer-length)
             (procedure? bitwise-if) (procedure? bit-set?)
             (procedure? copy-bit) (procedure? bit-swap)
             (procedure? any-bit-set?) (procedure? every-bit-set?)
             (procedure? first-set-bit) (procedure? bit-field)
             (procedure? bit-field-any?) (procedure? bit-field-every?)
             (procedure? bit-field-clear) (procedure? bit-field-set)
             (procedure? bit-field-replace) (procedure? bit-field-replace-same)
             (procedure? bit-field-rotate) (procedure? bit-field-reverse)
             (procedure? bits->list) (procedure? bits->vector)
             (procedure? list->bits) (procedure? vector->bits)
             (procedure? bits) (procedure? bitwise-fold)
             (procedure? bitwise-for-each) (procedure? bitwise-unfold)
             (procedure? make-bitwise-generator)
             (= 10 (bitwise-and 11 26))
             (equal? '(#t #f #t) (bits->list 5)))
        "#,
    ));
}

/// Fact two: importing `(srfi 151)` does not leak the wrapper's private helper.
/// `%booleans->integer` backs the bits-packing conversions but is not exported,
/// so a reference to it through the public import is an unbound-variable error.
#[test]
fn srfi_151_public_import_does_not_leak_internal_helpers() {
    let mut engine = engine_with(Extension::Srfi151);
    assert!(
        !evaluates(&mut engine, "(import (srfi 151)) %booleans->integer"),
        "public import leaks internal helper %booleans->integer"
    );
}

/// Fact three: installing SRFI 151 also registers the `(r7rs bitwise-operations)` alias,
/// which provides the same library under a discoverable name.
#[test]
fn srfi_151_alias_r7rs_bitwise_provides_the_same_library() {
    let mut engine = engine_with(Extension::Srfi151);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme base) (r7rs bitwise-operations))
        (unless (= 10 (bitwise-and 11 26)) (error "alias broken"))
        "#,
    ));
}

/// Fact one: importing `(scheme bytevector)` binds the specified procedures.
/// A representative slice is checked here, spanning the `endianness` syntax,
/// the re-exported `(scheme base)` overlap, and every native group, ending in
/// real computations.
#[test]
fn bytevector_public_import_exposes_the_specified_bindings() {
    let mut engine = engine_with(Extension::Bytevector);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme bytevector) (scheme base))
        (and (eq? 'little (endianness little))
             (if (memq (native-endianness) '(little big)) #t #f)
             (procedure? bytevector?) (procedure? make-bytevector)
             (procedure? bytevector-length) (procedure? bytevector-copy)
             (procedure? bytevector-copy!) (procedure? bytevector=?)
             (procedure? bytevector-fill!)
             (procedure? bytevector-u8-ref) (procedure? bytevector-u8-set!)
             (procedure? bytevector-s8-ref) (procedure? bytevector-s8-set!)
             (procedure? bytevector->u8-list) (procedure? u8-list->bytevector)
             (procedure? bytevector-uint-ref) (procedure? bytevector-sint-ref)
             (procedure? bytevector-uint-set!) (procedure? bytevector-sint-set!)
             (procedure? bytevector->uint-list) (procedure? bytevector->sint-list)
             (procedure? uint-list->bytevector) (procedure? sint-list->bytevector)
             (procedure? bytevector-u16-ref) (procedure? bytevector-s16-ref)
             (procedure? bytevector-u16-native-ref) (procedure? bytevector-s16-native-ref)
             (procedure? bytevector-u16-set!) (procedure? bytevector-s16-set!)
             (procedure? bytevector-u16-native-set!) (procedure? bytevector-s16-native-set!)
             (procedure? bytevector-u32-ref) (procedure? bytevector-s32-ref)
             (procedure? bytevector-u32-native-ref) (procedure? bytevector-s32-native-ref)
             (procedure? bytevector-u32-set!) (procedure? bytevector-s32-set!)
             (procedure? bytevector-u32-native-set!) (procedure? bytevector-s32-native-set!)
             (procedure? bytevector-u64-ref) (procedure? bytevector-s64-ref)
             (procedure? bytevector-u64-native-ref) (procedure? bytevector-s64-native-ref)
             (procedure? bytevector-u64-set!) (procedure? bytevector-s64-set!)
             (procedure? bytevector-u64-native-set!) (procedure? bytevector-s64-native-set!)
             (procedure? bytevector-ieee-single-ref)
             (procedure? bytevector-ieee-single-native-ref)
             (procedure? bytevector-ieee-single-set!)
             (procedure? bytevector-ieee-single-native-set!)
             (procedure? bytevector-ieee-double-ref)
             (procedure? bytevector-ieee-double-native-ref)
             (procedure? bytevector-ieee-double-set!)
             (procedure? bytevector-ieee-double-native-set!)
             (procedure? string->utf8) (procedure? utf8->string)
             (procedure? string->utf16) (procedure? string->utf32)
             (procedure? utf16->string) (procedure? utf32->string)
             (= 513 (bytevector-u16-ref (u8-list->bytevector '(1 2)) 0
                                        (endianness little)))
             (string=? "A" (utf16->string (string->utf16 "A")
                                          (endianness big))))
        "#,
    ));
}

/// Fact two: importing `(scheme bytevector)` binds only the specified names.
/// The wrapper has no `%` helpers, but two deliberate exclusions are pinned:
/// `bytevector-append` (not part of the R6RS chapter) and the R6RS argument
/// order of `bytevector-copy!` (the re-exported base version keeps the
/// R7RS-small order, so the R6RS five-argument source-first call fails).
#[test]
fn bytevector_public_import_does_not_bind_unspecified_names() {
    let mut engine = engine_with(Extension::Bytevector);
    // A positive control first: an exported name can be cherry-picked.
    assert!(evaluates(
        &mut engine,
        "(import (only (scheme bytevector) bytevector=?)) #t"
    ));
    // bytevector-append is not in the export list, so selecting it fails.
    assert!(
        !evaluates(
            &mut engine,
            "(import (only (scheme bytevector) bytevector-append)) #t"
        ),
        "the library exports bytevector-append, which is not part of it"
    );
    // R6RS order: copying two bytes out of an immutable literal source into a
    // fresh target, which R6RS would allow. Under the R7RS-small signature the
    // first argument is the destination, so the call tries to mutate the
    // literal and raises, proving the small ordering is in effect.
    assert!(
        !evaluates(
            &mut engine,
            r#"
            (import (scheme base) (scheme bytevector))
            (bytevector-copy! #u8(1 2 3 4) 0 (make-bytevector 2 0) 0 2)
            "#,
        ),
        "bytevector-copy! accepted the R6RS argument order"
    );
}

/// Fact three: installing the extension also registers the
/// `(r7rs bytevector)` alias, which provides the same library under a
/// discoverable name, including the re-exported `endianness` macro.
#[test]
fn bytevector_alias_r7rs_bytevector_provides_the_same_library() {
    let mut engine = engine_with(Extension::Bytevector);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme base) (r7rs bytevector))
        (unless (= 258 (bytevector-u16-ref (u8-list->bytevector '(1 2)) 0
                                           (endianness big)))
          (error "alias broken"))
        "#,
    ));
}

/// Fact one: importing `(srfi 152)` binds the specified procedures. A
/// representative slice is checked here, spanning the re-exported R7RS-small
/// procedures, the native scans and builders, and the Scheme-defined predicates,
/// searchers, folders, and splitters, ending in real computations.
#[test]
fn srfi_152_public_import_exposes_the_specified_bindings() {
    let mut engine = engine_with(Extension::Srfi152);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (srfi 152) (scheme base))
        (and (procedure? string-null?) (procedure? string-every)
             (procedure? string-any) (procedure? string-tabulate)
             (procedure? string-unfold) (procedure? string-unfold-right)
             (procedure? reverse-list->string) (procedure? string-take)
             (procedure? string-drop) (procedure? string-take-right)
             (procedure? string-drop-right) (procedure? string-pad)
             (procedure? string-pad-right) (procedure? string-trim)
             (procedure? string-trim-right) (procedure? string-trim-both)
             (procedure? string-replace) (procedure? string-prefix-length)
             (procedure? string-suffix-length) (procedure? string-prefix?)
             (procedure? string-suffix?) (procedure? string-index)
             (procedure? string-index-right) (procedure? string-skip)
             (procedure? string-skip-right) (procedure? string-contains)
             (procedure? string-contains-right) (procedure? string-take-while)
             (procedure? string-take-while-right) (procedure? string-drop-while)
             (procedure? string-drop-while-right) (procedure? string-break)
             (procedure? string-span) (procedure? string-concatenate)
             (procedure? string-concatenate-reverse) (procedure? string-join)
             (procedure? string-fold) (procedure? string-fold-right)
             (procedure? string-count) (procedure? string-filter)
             (procedure? string-remove) (procedure? string-replicate)
             (procedure? string-segment) (procedure? string-split)
             (procedure? string-map) (procedure? string-for-each)
             (procedure? string-ci=?)
             (string=? "cdefab" (string-replicate "abcdef" 2 8))
             (equal? '("a" "b" "c") (string-split "a,b,c" ",")))
        "#,
    ));
}

/// Fact two: importing `(srfi 152)` does not leak the wrapper's private helpers.
/// The `%`-prefixed optional-argument and splitting helpers back the exported
/// procedures but are not exported, so a reference to one is unbound.
#[test]
fn srfi_152_public_import_does_not_leak_internal_helpers() {
    let mut engine = engine_with(Extension::Srfi152);
    for helper in ["%opt", "%->string", "%split-each-char", "%strip-last-empty"] {
        let source = format!("(import (srfi 152)) {helper}");
        assert!(
            !evaluates(&mut engine, &source),
            "public import leaks internal helper {helper}"
        );
    }
}

/// Fact three: installing SRFI 152 also registers the `(r7rs strings)` alias,
/// which provides the same library under a discoverable name.
#[test]
fn srfi_152_alias_r7rs_strings_provides_the_same_library() {
    let mut engine = engine_with(Extension::Srfi152);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme base) (r7rs strings))
        (unless (string=? "foo:bar:baz" (string-join '("foo" "bar" "baz") ":"))
          (error "alias broken"))
        "#,
    ));
}

/// Fact one: importing `(srfi 175)` binds all specified ASCII procedures.
#[test]
fn srfi_175_public_import_exposes_the_specified_bindings() {
    let mut engine = engine_with(Extension::Srfi175);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (srfi 175) (scheme base))
        (and
          (procedure? ascii-codepoint?) (procedure? ascii-bytevector?)
          (procedure? ascii-char?) (procedure? ascii-string?)
          (procedure? ascii-control?) (procedure? ascii-non-control?)
          (procedure? ascii-whitespace?) (procedure? ascii-space-or-tab?)
          (procedure? ascii-other-graphic?) (procedure? ascii-upper-case?)
          (procedure? ascii-lower-case?) (procedure? ascii-alphabetic?)
          (procedure? ascii-alphanumeric?) (procedure? ascii-numeric?)
          (procedure? ascii-digit-value) (procedure? ascii-upper-case-value)
          (procedure? ascii-lower-case-value) (procedure? ascii-nth-digit)
          (procedure? ascii-nth-upper-case) (procedure? ascii-nth-lower-case)
          (procedure? ascii-upcase) (procedure? ascii-downcase)
          (procedure? ascii-control->graphic)
          (procedure? ascii-graphic->control)
          (procedure? ascii-mirror-bracket)
          (procedure? ascii-ci=?) (procedure? ascii-ci<?)
          (procedure? ascii-ci>?) (procedure? ascii-ci<=?)
          (procedure? ascii-ci>=?) (procedure? ascii-string-ci=?)
          (procedure? ascii-string-ci<?) (procedure? ascii-string-ci>?)
          (procedure? ascii-string-ci<=?) (procedure? ascii-string-ci>=?)
          (ascii-ci=? #\A 97)
          (ascii-string? "ASCII"))
        "#,
    ));
}

/// Fact two: importing `(srfi 175)` exposes only the normative names. Native
/// implementation helpers are not public Scheme bindings.
#[test]
fn srfi_175_public_import_does_not_leak_internal_helpers() {
    let mut engine = engine_with(Extension::Srfi175);
    for helper in ["char-fix", "fold-ascii", "compare-strings", "%ascii-ci-cmp"] {
        let source = format!("(import (srfi 175)) {helper}");
        assert!(
            !evaluates(&mut engine, &source),
            "public import leaks internal helper {helper}"
        );
    }
}

/// Fact three: installing SRFI 175 also registers the `(r7rs ascii)` alias,
/// which provides the identical library.
#[test]
fn srfi_175_alias_r7rs_ascii_provides_the_same_library() {
    let mut engine = engine_with(Extension::Srfi175);
    assert!(evaluates(
        &mut engine,
        r#"
        (import (scheme base) (r7rs ascii))
        (unless (and (ascii-string? "fixture")
                     (ascii-ci=? #\A 97)
                     (char=? #\] (ascii-mirror-bracket #\[)))
          (error "alias broken"))
        "#,
    ));
}

/// The `(r7rs ...)` aliases exist only through installation. Without the
/// extension installed, importing an alias is a library-not-found error.
#[test]
fn aliases_are_absent_without_their_extension() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    for alias in [
        "lists",
        "and-let*",
        "receive",
        "cut",
        "random",
        "intermediate-format-strings",
        "hash-table",
        "sorting",
        "bitwise-operations",
        "strings",
        "ascii",
        "bytevector",
    ] {
        let source = format!("(import (r7rs {alias})) #t");
        assert!(
            !evaluates(&mut engine, &source),
            "alias (r7rs {alias}) resolved without its extension installed"
        );
    }
}
