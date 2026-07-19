use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
};

use r7rs::{
    Engine, EngineConfig, ErrorKind, Extension, FeatureSet, LibraryName, LibraryNameComponent,
    LoadedSource, SourceLoader, SourceLoaderError, SourceRequest, Value,
};

fn name(parts: &[&str]) -> LibraryName {
    LibraryName::new(
        parts
            .iter()
            .map(|part| LibraryNameComponent::identifier(*part)),
    )
    .unwrap()
}

fn run(engine: &mut Engine, source: &str) -> Value {
    let module = engine.compile("program.scm", source).unwrap();
    engine.eval(&module).unwrap().into_one().unwrap().value()
}

#[test]
fn registered_libraries_initialize_dependencies_and_apply_import_sets() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .register_library_source(
            name(&["example", "math"]),
            "math.sld",
            "(define-library (example math) (export add-one hidden) (import (scheme base)) (begin (define (add-one x) (+ x 1)) (define hidden 9)))",
        )
        .unwrap();
    engine
        .register_library_source(
            name(&["example", "use"]),
            "use.sld",
            "(define-library (example use) (export answer) (import (only (example math) add-one)) (begin (define answer (add-one 41))))",
        )
        .unwrap();
    assert_eq!(
        run(&mut engine, "(import (prefix (example use) u:)) u:answer"),
        Value::integer(42)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (rename (example math) (hidden secret))) secret"
        ),
        Value::integer(9)
    );
}

#[test]
fn default_engine_catches_native_panics_as_errors() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .register_library_fn(
            &name(&["test", "boom"]),
            "boom",
            0..=0,
            |_, _| -> Result<Value, r7rs::Error> { panic!("native exploded") },
        )
        .unwrap();
    let module = engine
        .compile("boom.scm", "(import (test boom)) (boom)")
        .unwrap();
    // Suppress the default panic hook's stderr message for the expected panic.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = engine.eval(&module);
    std::panic::set_hook(previous);
    let error = outcome.expect_err("panicking native must surface an error");
    assert_eq!(error.kind(), ErrorKind::NativePanic);
}

#[test]
fn trusted_natives_execute_directly() {
    // The trusted-natives path skips the catch_unwind guard; a well-behaved
    // native must still run and return correctly.
    let mut engine = Engine::new(EngineConfig::default().with_trusted_natives(true)).unwrap();
    engine
        .register_library_fn(&name(&["test", "inc"]), "inc", 1..=1, |cx, args| {
            let value = cx.to_i128(args[0])? + 1;
            cx.integer(value)
        })
        .unwrap();
    assert_eq!(
        run(&mut engine, "(import (test inc)) (inc 41)"),
        Value::integer(42)
    );
}

#[test]
fn native_libraries_back_scheme_wrappers_without_ambient_bindings() {
    let internal = LibraryName::new([
        LibraryNameComponent::identifier("neetan"),
        LibraryNameComponent::identifier("internal"),
        LibraryNameComponent::number(1),
    ])
    .unwrap();
    let public = name(&["neetan", "public"]);
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .register_library_fn(&internal, "%neetan-inc", 1..=1, |cx, args| {
            let value = cx.to_i128(args[0])? + 1;
            cx.integer(value)
        })
        .unwrap();
    engine
        .register_library_fn(&internal, "%neetan-double", 1..=1, |cx, args| {
            let value = cx.to_i128(args[0])? * 2;
            cx.integer(value)
        })
        .unwrap();
    engine
        .register_library_source(
            public,
            "neetan-public.sld",
            "(define-library (neetan public)
               (export inc-and-double)
               (import
                 (scheme base)
                 (only (neetan internal 1) %neetan-inc %neetan-double))
               (begin
                 (define (inc-and-double value)
                   (%neetan-double (%neetan-inc value)))))",
        )
        .unwrap();

    assert_eq!(
        run(&mut engine, "(import (neetan public)) (inc-and-double 20)"),
        Value::integer(42)
    );

    let ambient = engine
        .compile("native-ambient.scm", "(import (neetan public)) %neetan-inc")
        .unwrap();
    assert_eq!(
        engine.eval(&ambient).unwrap_err().kind(),
        ErrorKind::RuntimeError
    );

    let direct = engine
        .compile(
            "native-arity.scm",
            "(import (neetan internal 1)) (%neetan-inc)",
        )
        .unwrap();
    let arity = engine.eval(&direct).unwrap_err();
    assert_eq!(arity.kind(), ErrorKind::ArityError);
    assert!(arity.diagnostic().message().contains("%neetan-inc"));
    assert!(arity.diagnostic().message().contains("(neetan internal 1)"));
    assert!(!arity.diagnostic().message().contains('\u{1f}'));
}

#[test]
fn apply_spread_arguments_survive_collection_inside_a_native_call() {
    // `apply` spreads its list beyond the caller's register window, and the
    // spread overwrites the register holding the (now otherwise unreachable)
    // list. A collection forced inside the native must still see every
    // argument: each is a distinct heap-backed exact integer produced by
    // arithmetic, so a rooting hole would free them mid-call.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .register_library_fn(
            &name(&["test", "gc"]),
            "sum-all",
            0..=usize::MAX,
            |cx, args| {
                cx.collect_now();
                let mut total: i128 = 0;
                for argument in args {
                    total += cx.to_i128(*argument)?;
                }
                cx.integer(total)
            },
        )
        .unwrap();
    let module = engine
        .compile(
            "apply-gc.scm",
            "(import (scheme base) (test gc))
             (define (build n acc)
               (if (= n 0)
                   acc
                   (build (- n 1) (cons (+ 100000000000000000000 n) acc))))
             (apply sum-all (build 40 '()))",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    // 40 * 10^20 + (1 + 2 + ... + 40)
    assert_eq!(engine.write_root(&value).unwrap(), "4000000000000000000820");
}

#[test]
fn native_allocations_survive_collections_later_in_the_same_call() {
    // A value allocated through the context must stay rooted across a
    // collection triggered later in the same native call.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .register_library_fn(
            &name(&["test", "gc"]),
            "alloc-collect-alloc",
            0..=0,
            |cx, _| {
                let first = cx.string("first".chars())?;
                cx.collect_now();
                let second = cx.string("second".chars())?;
                cx.collect_now();
                cx.pair(first, second)
            },
        )
        .unwrap();
    let module = engine
        .compile(
            "alloc-gc.scm",
            "(import (scheme base) (test gc))
             (let ((p (alloc-collect-alloc)))
               (string-append (car p) \"-\" (cdr p)))",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(engine.write_root(&value).unwrap(), "\"first-second\"");
}

#[test]
fn caller_registers_survive_a_collection_inside_a_native_call() {
    // A heap value that lives only in a caller register (not among the native's
    // arguments) must survive a collection forced inside the native: the
    // collector traces the live register file through the call-site root view.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .register_library_fn(&name(&["test", "gc"]), "force-gc", 0..=0, |cx, _| {
            cx.collect_now();
            Ok(Value::integer(0))
        })
        .unwrap();
    let module = engine
        .compile(
            "registers-gc.scm",
            "(import (scheme base) (test gc))
             (let ((payload (string-append \"keep\" \"me\")))
               (force-gc)
               payload)",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(engine.write_root(&value).unwrap(), "\"keepme\"");
}

#[test]
fn native_library_bindings_are_isolated_per_engine() {
    let native = name(&["example", "native"]);
    let mut first = Engine::new(EngineConfig::default()).unwrap();
    first
        .register_library_fn(&native, "answer", 0..=0, |_, _| Ok(Value::integer(1)))
        .unwrap();
    let mut second = Engine::new(EngineConfig::default()).unwrap();
    second
        .register_library_fn(&native, "answer", 0..=0, |_, _| Ok(Value::integer(2)))
        .unwrap();

    assert_eq!(
        run(&mut first, "(import (example native)) (answer)"),
        Value::integer(1)
    );
    assert_eq!(
        run(&mut second, "(import (example native)) (answer)"),
        Value::integer(2)
    );

    let mut third = Engine::new(EngineConfig::default()).unwrap();
    let missing = third
        .compile("missing-native.scm", "(import (example native)) (answer)")
        .unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::LibraryNotFound);
}

#[test]
fn native_library_registration_validates_before_mutating_state() {
    let native = name(&["example", "validated-native"]);
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    let invalid_arity = engine
        .register_library_fn(
            &native,
            "answer",
            std::ops::RangeInclusive::new(2, 1),
            |_, _| Ok(Value::integer(0)),
        )
        .unwrap_err();
    assert_eq!(invalid_arity.kind(), ErrorKind::RuntimeError);

    engine
        .register_library_fn(&native, "answer", 0..=0, |_, _| Ok(Value::integer(42)))
        .unwrap();
    let duplicate = engine
        .register_library_fn(&native, "answer", 0..=0, |_, _| Ok(Value::integer(0)))
        .unwrap_err();
    assert_eq!(duplicate.kind(), ErrorKind::LibraryError);
    assert_eq!(
        run(&mut engine, "(import (example validated-native)) (answer)"),
        Value::integer(42)
    );

    let sealed = engine
        .register_library_fn(&native, "later", 0..=0, |_, _| Ok(Value::integer(1)))
        .unwrap_err();
    assert_eq!(sealed.kind(), ErrorKind::LibraryError);

    let source_first = name(&["example", "source-first"]);
    engine
        .register_library_source(
            source_first.clone(),
            "source-first.sld",
            "(define-library (example source-first) (export))",
        )
        .unwrap();
    let source_collision = engine
        .register_library_fn(&source_first, "value", 0..=0, |_, _| Ok(Value::integer(1)))
        .unwrap_err();
    assert_eq!(source_collision.kind(), ErrorKind::LibraryError);

    let native_first = name(&["example", "native-first"]);
    engine
        .register_library_fn(&native_first, "value", 0..=0, |_, _| Ok(Value::integer(1)))
        .unwrap();
    let native_collision = engine
        .register_library_source(
            native_first.clone(),
            "native-first.sld",
            "(define-library (example native-first) (export))",
        )
        .unwrap_err();
    assert_eq!(native_collision.kind(), ErrorKind::LibraryError);

    let standard = name(&["scheme", "base"]);
    let standard_collision = engine
        .register_library_fn(&standard, "host-value", 0..=0, |_, _| Ok(Value::integer(1)))
        .unwrap_err();
    assert_eq!(standard_collision.kind(), ErrorKind::LibraryError);
}

#[test]
fn import_errors_are_structured_and_library_cycles_are_retained() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .register_library_source(
            name(&["example", "a"]),
            "a.sld",
            "(define-library (example a) (export a) (import (example b)) (begin (define a b)))",
        )
        .unwrap();
    engine
        .register_library_source(
            name(&["example", "b"]),
            "b.sld",
            "(define-library (example b) (export b) (import (example a)) (begin (define b a)))",
        )
        .unwrap();
    let error = engine
        .compile("cycle.scm", "(import (example a)) a")
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LibraryCycle);
    assert!(
        error
            .diagnostic()
            .message()
            .contains("(example a) -> (example b) -> (example a)")
    );
    let retained = engine
        .compile("cycle-again.scm", "(import (example a)) a")
        .unwrap_err();
    assert_eq!(retained.kind(), ErrorKind::LibraryCycle);
    let missing = engine
        .compile("missing.scm", "(import (only (scheme base) nope)) nope")
        .unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::LibraryError);
}

#[test]
fn cond_expand_uses_only_configured_features() {
    let mut default = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(
            &mut default,
            "(cond-expand ((and r7rs (not made-up)) 7) (else 0))"
        ),
        Value::integer(7)
    );
    assert_eq!(
        run(
            &mut default,
            "(cond-expand ((library (scheme base)) 8) (else 0))"
        ),
        Value::integer(8)
    );
    assert_eq!(
        run(
            &mut default,
            "(equal? (features) '(exact-complex ieee-float r7rs))",
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut default,
            "(let ((f features)) (equal? (f) '(exact-complex ieee-float r7rs)))",
        ),
        Value::boolean(true)
    );
    let mut configured = Engine::new(
        EngineConfig::default().with_features(FeatureSet::default().with_identifier("host-test")),
    )
    .unwrap();
    assert_eq!(
        run(&mut configured, "(cond-expand (host-test 9) (else 0))"),
        Value::integer(9)
    );
}

#[test]
fn cond_expand_library_requirements_see_registered_libraries() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Srfi1).unwrap();
    assert_eq!(
        run(
            &mut engine,
            "(cond-expand ((library (srfi 1)) #t) (else #f))"
        ),
        Value::boolean(true)
    );

    engine
        .register_library_source(
            name(&["example", "library-requirement"]),
            "library-requirement.sld",
            "(define-library (example library-requirement)
               (export value)
               (import (scheme base))
               (cond-expand
                 ((library (srfi 1)) (begin (define value 42)))
                 (else (begin (define value 0)))))",
        )
        .unwrap();
    assert_eq!(
        run(&mut engine, "(import (example library-requirement)) value"),
        Value::integer(42)
    );
}

#[test]
fn cond_expand_validates_clauses_after_the_first_match() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let error = engine
        .compile(
            "invalid-cond-expand.scm",
            "(cond-expand (r7rs 1) (else 2) (made-up 3))",
        )
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ExpandError);

    let error = engine
        .register_library_source(
            name(&["example", "invalid-conditional"]),
            "invalid-conditional.sld",
            "(define-library (example invalid-conditional)
               (export value)
               (cond-expand
                 (r7rs (begin (define value 1)))
                 (else (begin (define value 2)))
                 (made-up (begin (define value 3)))))",
        )
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LibraryError);
}

#[test]
fn standard_library_manifests_are_importable_and_later_phase_bindings_are_explicit() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(&mut engine, "(import (scheme lazy)) (promise? (delay 1))"),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(import (scheme base) (scheme cxr)) (equal? (caaaar '(((((1)))))) '(1))"
        ),
        Value::boolean(true)
    );
    let module = engine
        .compile(
            "file.scm",
            "(import (scheme file)) (file-exists? \"anything\")",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap_err().kind(),
        ErrorKind::CapabilityDenied
    );
    assert_eq!(
        run(&mut engine, "(import (scheme r5rs)) (exact->inexact 3)"),
        Value::float(3.0)
    );
}

#[test]
fn eval_accepts_quoted_expressions_in_an_immutable_base_environment() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(
            &mut engine,
            "(import (scheme eval)) (eval '(* 7 3) (environment '(scheme base)))",
        ),
        Value::integer(21)
    );
    let error = engine
        .compile(
            "immutable-eval.scm",
            "(import (scheme eval)) (eval '(define x 1) (environment '(scheme base)))",
        )
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ExpandError);
    let module = engine
        .compile("environment.scm", "(environment '(scheme base))")
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(
        engine.value_kind(&value).unwrap(),
        r7rs::ValueKind::Environment
    );
}

struct IncludeLoader {
    replies: VecDeque<Result<LoadedSource, SourceLoaderError>>,
    parents: Arc<Mutex<Vec<Option<String>>>>,
}

impl SourceLoader for IncludeLoader {
    fn load(&mut self, request: SourceRequest<'_>) -> Result<LoadedSource, SourceLoaderError> {
        self.parents
            .lock()
            .unwrap()
            .push(request.including_identity().map(str::to_owned));
        self.replies
            .pop_front()
            .unwrap_or_else(|| Err(Box::new(io::Error::other("missing"))))
    }
}

#[test]
fn library_include_uses_the_injected_relative_source_loader() {
    let parents = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_source_loader(Box::new(IncludeLoader {
        replies: VecDeque::from([Ok(LoadedSource::new(
            "pkg/part",
            "part.scm",
            "(define answer 42)",
        ))]),
        parents: parents.clone(),
    }));
    engine
        .register_library_source(
            name(&["example", "included"]),
            "included.sld",
            "(define-library (example included) (export answer) (import (scheme base)) (include \"part.scm\"))",
        )
        .unwrap();
    assert_eq!(
        run(&mut engine, "(import (example included)) answer"),
        Value::integer(42)
    );
    assert_eq!(*parents.lock().unwrap(), vec![None]);
}

#[test]
fn library_declarations_can_be_included_recursively() {
    let parents = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_source_loader(Box::new(IncludeLoader {
        replies: VecDeque::from([
            Ok(LoadedSource::new(
                "pkg/interface",
                "interface.sld",
                "(export answer) (import (scheme base)) (include-library-declarations \"body.sld\")",
            )),
            Ok(LoadedSource::new(
                "pkg/body",
                "body.sld",
                "(cond-expand (r7rs (begin (define answer 42))) (else (begin (define answer 0))))",
            )),
        ]),
        parents: parents.clone(),
    }));
    engine
        .register_library_source(
            name(&["example", "declarations"]),
            "declarations.sld",
            "(define-library (example declarations) (include-library-declarations \"interface.sld\"))",
        )
        .unwrap();
    assert_eq!(
        run(&mut engine, "(import (example declarations)) answer"),
        Value::integer(42)
    );
    assert_eq!(
        *parents.lock().unwrap(),
        vec![None, Some("pkg/interface".to_owned())]
    );
}

#[test]
fn libraries_may_export_no_bindings() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .register_library_source(
            name(&["example", "private"]),
            "private.sld",
            "(define-library (example private) (import (scheme base)) (begin (define hidden 1)))",
        )
        .unwrap();
    engine
        .register_library_source(
            name(&["example", "empty-export"]),
            "empty-export.sld",
            "(define-library (example empty-export) (export) (import (scheme base)))",
        )
        .unwrap();
    assert_eq!(
        run(
            &mut engine,
            "(import (example private) (example empty-export)) 7"
        ),
        Value::integer(7)
    );
}

/// Returns whether a program compiles and evaluates without error.
fn evaluates(engine: &mut Engine, source: &str) -> bool {
    match engine.compile("program.scm", source) {
        Ok(module) => engine.eval(&module).is_ok(),
        Err(_) => false,
    }
}

#[test]
fn define_record_type_bindings_do_not_leak_past_the_export_list() {
    // A define-record-type inside a library body introduces a constructor,
    // predicate, accessors, and mutators. When they are not exported, an
    // importer must not see them, exactly as for a plain unexported define.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .register_library_source(
            name(&["records", "demo"]),
            "records-demo.sld",
            "(define-library (records demo)
               (export make-box unwrap-box)
               (import (scheme base))
               (begin
                 (define-record-type <box> (raw-box value) box? (value box-ref set-box!))
                 (define (make-box value) (raw-box value))
                 (define (unwrap-box b) (box-ref b))))",
        )
        .unwrap();
    // The exported wrapper works, so the record type functions internally.
    assert_eq!(
        run(
            &mut engine,
            "(import (records demo)) (unwrap-box (make-box 42))",
        ),
        Value::integer(42)
    );
    // None of the unexported record bindings leak through the import.
    for leaked in ["raw-box", "box?", "box-ref", "set-box!", "<box>"] {
        let source = format!("(import (records demo)) {leaked}");
        assert!(
            !evaluates(&mut engine, &source),
            "public import leaks unexported record binding {leaked}"
        );
    }
}

#[test]
fn exported_record_bindings_are_usable_by_importers() {
    // The other half of the contract: a library that does export its record
    // bindings exposes them, and they operate on the library's records.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .register_library_source(
            name(&["records", "point"]),
            "records-point.sld",
            "(define-library (records point)
               (export make-point point? point-x point-y set-point-x!)
               (import (scheme base))
               (begin
                 (define-record-type <point>
                   (make-point x y) point?
                   (x point-x set-point-x!)
                   (y point-y))))",
        )
        .unwrap();
    assert_eq!(
        run(
            &mut engine,
            "(import (records point) (scheme base))
             (define p (make-point 3 4))
             (set-point-x! p 30)
             (and (point? p) (not (point? 5))
                  (= (point-x p) 30) (= (point-y p) 4))",
        ),
        Value::boolean(true)
    );
}

#[test]
fn cyclic_library_declaration_includes_are_rejected() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_source_loader(Box::new(IncludeLoader {
        replies: VecDeque::from([
            Ok(LoadedSource::new(
                "pkg/a",
                "a.sld",
                "(include-library-declarations \"b.sld\")",
            )),
            Ok(LoadedSource::new(
                "pkg/b",
                "b.sld",
                "(include-library-declarations \"a.sld\")",
            )),
            Ok(LoadedSource::new(
                "pkg/a",
                "a.sld",
                "(include-library-declarations \"b.sld\")",
            )),
        ]),
        parents: Arc::new(Mutex::new(Vec::new())),
    }));
    let error = engine
        .register_library_source(
            name(&["example", "cycle"]),
            "cycle.sld",
            "(define-library (example cycle) (include-library-declarations \"a.sld\"))",
        )
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LibraryError);
    assert!(error.diagnostic().message().contains("cyclic"));
}

#[test]
fn include_ci_enables_case_folding_for_the_included_forms() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_source_loader(Box::new(IncludeLoader {
        replies: VecDeque::from([Ok(LoadedSource::new(
            "part",
            "part.scm",
            "(define ANSWER 5)",
        ))]),
        parents: Arc::new(Mutex::new(Vec::new())),
    }));
    engine
        .register_library_source(
            name(&["example", "folded"]),
            "folded.sld",
            "(define-library (example folded) (export answer) (import (scheme base)) (include-ci \"part.scm\"))",
        )
        .unwrap();
    assert_eq!(
        run(&mut engine, "(import (example folded)) answer"),
        Value::integer(5)
    );
}

#[test]
fn library_cond_expand_selects_declarations_before_expansion() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .register_library_source(
            name(&["example", "conditional"]),
            "conditional.sld",
            "(define-library (example conditional) (export value) (import (scheme base)) (cond-expand (r7rs (begin (define value 12))) (else (begin (define value 0)))))",
        )
        .unwrap();
    assert_eq!(
        run(&mut engine, "(import (example conditional)) value"),
        Value::integer(12)
    );
}

#[test]
fn imported_syntax_bindings_expand_before_library_body_compilation() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .register_library_source(
            name(&["example", "syntax"]),
            "syntax.sld",
            "(define-library (example syntax) (export twice) (import (scheme base)) (begin (define-syntax twice (syntax-rules () ((twice x) (+ x x))))))",
        )
        .unwrap();
    assert_eq!(
        run(&mut engine, "(import (example syntax)) (twice 21)"),
        Value::integer(42)
    );
}

/// A read-eval-print front-end compiles each input separately. `compile` scopes
/// an `import` to the single call that names it, so a binding imported on one
/// input is gone by the next. `compile_interactive` instead accumulates imports
/// into a persistent environment, the way top-level `define`s already persist,
/// so an imported procedure stays usable on later inputs. Re-importing a library
/// on a later input is a harmless no-op.
#[test]
fn interactive_compiles_carry_imports_across_inputs() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Srfi1).unwrap();

    let import = engine
        .compile_interactive("repl-1", "(import (srfi 1))")
        .unwrap();
    engine.eval(&import).unwrap();
    let module = engine.compile_interactive("repl-2", "(xcons 1 2)").unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(engine.write_root(&value).unwrap(), "(2 . 1)");

    let reimport = engine
        .compile_interactive("repl-3", "(import (srfi 1)) (xcons 3 4)")
        .unwrap();
    let value = engine.eval(&reimport).unwrap().into_one().unwrap();
    assert_eq!(engine.write_root(&value).unwrap(), "(4 . 3)");

    // The batch compile, by contrast, scopes the import to the one call. A free
    // `xcons` in the next call compiles to a plain global reference that is
    // unbound at evaluation, so the split fails where the interactive one worked.
    let mut batch = Engine::new(EngineConfig::default()).unwrap();
    batch.install_extension(Extension::Srfi1).unwrap();
    let import = batch.compile("repl-1", "(import (srfi 1))").unwrap();
    batch.eval(&import).unwrap();
    let module = batch.compile("repl-2", "(xcons 1 2)").unwrap();
    assert!(
        batch.eval(&module).is_err(),
        "a batch import must not carry to the next input"
    );
}

/// Imported macros persist across interactive inputs too, not only procedures:
/// `cut` imported on one input expands on the next.
#[test]
fn interactive_compiles_carry_imported_macros() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.install_extension(Extension::Srfi26).unwrap();

    let import = engine
        .compile_interactive("repl-1", "(import (r7rs cut) (scheme base))")
        .unwrap();
    engine.eval(&import).unwrap();
    let module = engine
        .compile_interactive("repl-2", "((cut list 1 <> 3) 2)")
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(engine.write_root(&value).unwrap(), "(1 2 3)");
}
