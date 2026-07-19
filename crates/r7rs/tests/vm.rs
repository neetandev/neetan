use r7rs::{CoreExpr, Engine, EngineConfig, ErrorKind, Limits, Value};

fn literal(value: i64) -> CoreExpr {
    CoreExpr::literal(Value::integer(value))
}

fn variable(name: &str) -> CoreExpr {
    CoreExpr::variable(name)
}

fn call(procedure: CoreExpr, arguments: Vec<CoreExpr>) -> CoreExpr {
    CoreExpr::Call {
        procedure: Box::new(procedure),
        arguments,
    }
}

#[test]
fn compiler_and_vm_execute_branches_variables_and_closure_calls() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let program = CoreExpr::Begin(vec![
        CoreExpr::Define {
            name: "identity".into(),
            value: Box::new(CoreExpr::Lambda {
                params: vec!["x".into()],
                body: Box::new(CoreExpr::If(
                    Box::new(CoreExpr::literal(Value::boolean(false))),
                    Box::new(literal(0)),
                    Box::new(variable("x")),
                )),
            }),
        },
        call(variable("identity"), vec![literal(42)]),
    ]);
    let module = engine.compile_core(&program).unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(42)
    );
}

#[test]
fn compile_core_rejects_malformed_binding_forms() {
    let engine = Engine::new(EngineConfig::default()).unwrap();
    let malformed = [
        CoreExpr::NamedLet {
            name: "loop".into(),
            params: vec!["x".into()],
            inits: vec![],
            body: Box::new(literal(0)),
        },
        CoreExpr::NamedLet {
            name: "loop".into(),
            params: vec!["x".into(), "x".into()],
            inits: vec![literal(1), literal(2)],
            body: Box::new(literal(0)),
        },
        CoreExpr::CaseLambda {
            clauses: vec![literal(0)],
        },
    ];
    for expression in malformed {
        let error = engine.compile_core(&expression).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::CompileError);
    }
}

#[test]
fn value_count_errors_report_counts_larger_than_one_byte() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let values = std::iter::repeat_n("1", 256).collect::<Vec<_>>().join(" ");
    let module = engine
        .compile("many_values.scm", format!("(+ (values {values}) 1)"))
        .unwrap();
    let error = engine.eval(&module).unwrap_err();
    assert!(
        error.to_string().contains("received 256"),
        "unexpected error: {error}"
    );
}

#[test]
fn captures_share_mutable_cells_across_closure_calls_and_forced_gc() {
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let maker = CoreExpr::Lambda {
        params: vec!["x".into()],
        body: Box::new(CoreExpr::Lambda {
            params: vec![],
            body: Box::new(CoreExpr::Begin(vec![
                CoreExpr::Set {
                    name: "x".into(),
                    value: Box::new(literal(9)),
                },
                variable("x"),
            ])),
        }),
    };
    let program = CoreExpr::Begin(vec![
        CoreExpr::Define {
            name: "make".into(),
            value: Box::new(maker),
        },
        CoreExpr::Define {
            name: "saved".into(),
            value: Box::new(call(variable("make"), vec![literal(1)])),
        },
        call(variable("saved"), vec![]),
    ]);
    let module = engine.compile_core(&program).unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(9)
    );
}

#[test]
fn nested_closures_propagate_captures_through_intermediate_scopes() {
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let program = call(
        call(
            call(
                CoreExpr::Lambda {
                    params: vec!["x".into()],
                    body: Box::new(CoreExpr::Lambda {
                        params: vec!["ignored".into()],
                        body: Box::new(CoreExpr::Lambda {
                            params: vec![],
                            body: Box::new(variable("x")),
                        }),
                    }),
                },
                vec![literal(17)],
            ),
            vec![literal(0)],
        ),
        vec![],
    );
    let module = engine.compile_core(&program).unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(17)
    );
}

#[test]
fn tail_call_reuses_the_entry_frame() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let program = call(
        CoreExpr::Lambda {
            params: vec!["value".into()],
            body: Box::new(variable("value")),
        },
        vec![literal(7)],
    );
    let module = engine.compile_core(&program).unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(7)
    );
}

#[test]
fn a_procedure_whose_whole_body_raises_compiles_and_raises() {
    // `(error ...)` as an entire body compiles to a chunk ending in cold
    // MakeError/Raise instructions plus an unreachable `Return` emitted purely
    // to satisfy the verifier's no-fall-through rule. The raise must still
    // propagate as a catchable error object with its message and irritants.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "raise_only_body.scm",
            "(define (boom x) (error \"kaput\" x 2))
             (guard (condition
                     ((error-object? condition)
                      (list (error-object-message condition)
                            (error-object-irritants condition))))
               (boom 1))
             ",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(engine.write_root(&value).unwrap(), "(\"kaput\" (1 2))");
}

#[test]
fn calling_the_unspecified_value_raises_instead_of_reentering_the_entry_chunk() {
    // Regression: the entry frame's `procedure` sentinel used to be the
    // user-reachable unspecified value, so a top-level call of that value
    // passed the self-call guards (`frame.procedure == procedure`) and
    // re-entered the whole program chunk, looping forever for a zero-argument
    // call and raising a bogus entry-chunk arity error otherwise. The fuel
    // limit turns any regression back into a fast failure instead of a hang.
    for source in [
        "((if #f #f))",
        "(define x (if #f #f)) (x)",
        "((if #f #f) 1)",
    ] {
        let limits = Limits::default().with_fuel(Some(1_000_000));
        let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
        let module = engine.compile("call_unspecified.scm", source).unwrap();
        let error = engine.eval(&module).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::RuntimeError, "{source}");
        assert!(
            error.to_string().contains("unsupported callable"),
            "{source}: {error}"
        );
    }
}

#[test]
fn self_call_fast_path_preserves_return_actions_across_recycled_frames() {
    // The self-call fast path relies on the frame stack's dead-slot invariant
    // (recycled slots hold no return action) instead of writing one per call.
    // Interleave recursion that plants real boxed actions at depth
    // (dynamic-wind, parameterize, promises), a continuation captured
    // mid-recursion and re-invoked (wholesale frame-stack replacement), and
    // plain deep recursion reusing those same slots afterwards.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "recycled_frames.scm",
            "(define order '())
             (define (note tag) (set! order (cons tag order)))
             (define p (make-parameter 10))
             (define (wind n acc)
               (if (= n 0)
                   acc
                   (dynamic-wind
                     (lambda () (note 'in))
                     (lambda () (wind (- n 1) (+ acc (p))))
                     (lambda () (note 'out)))))
             (define wound (wind 4 0))
             (define deep-p
               (parameterize ((p 1))
                 (let recur ((n 6)) (if (= n 0) (p) (+ (p) (recur (- n 1)))))))
             (define lazy
               (let make ((n 5))
                 (if (= n 0)
                     (delay 7)
                     (delay (force (make (- n 1)))))))
             (define saved #f)
             (define invoked 0)
             (define (probe n)
               (if (= n 0)
                   (call-with-current-continuation
                     (lambda (k) (set! saved k) 1))
                   (+ 1 (probe (- n 1)))))
             (define captured (probe 10))
             (set! invoked (+ invoked 1))
             (if (= invoked 1) (saved 90) #f)
             (define (spin n) (if (= n 0) 0 (+ 1 (spin (- n 1)))))
             (list wound deep-p (force lazy) captured (spin 40)
                   (length order))",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(
        engine.write_root(&value).unwrap(),
        // wind: 4 iterations of (p)=10; deep-p: 6*(p=1)+(p=1)=7; lazy chain
        // forces to 7; captured: first pass 1+10, after (saved 90) => 100;
        // spin unchanged; 8 wind notes (4 in + 4 out).
        "(40 7 7 100 40 8)"
    );
}

#[test]
fn fallible_call_shapes_take_the_generic_path_with_identical_errors() {
    // Regression (case-lambda clause re-selection): a self-recursive
    // case-lambda call whose count doesn't match the *running* clause used to
    // raise that clause's arity error from the generic path's self-call guard
    // instead of selecting the accepting clause.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "generic_shapes.scm",
            "(define f
               (case-lambda
                 ((n) (+ 1 (f n 0)))
                 ((n acc) (if (= n 0) acc (f (- n 1) (+ acc 2))))))
             (define (h n . rest)
               (if (= n 0) (length rest) (+ 1 (h (- n 1) 'x))))
             (define (arity-miss n) (if (= n 0) 0 (arity-miss)))
             (list (f 5)
                   (h 3)
                   (guard (e (#t 'arity)) (arity-miss 1))
                   (guard (e (#t 'not-callable)) ((vector 1 2)))
                   (guard (e (#t 'native-error)) (vector-ref (vector) 5)))",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(
        engine.write_root(&value).unwrap(),
        "(11 4 arity not-callable native-error)"
    );
    // A count no clause accepts raises a proper arity error, both through the
    // self-call route (recursive `g` calling itself with three arguments) and
    // the plain route (top-level `(g 1 2 3)`).
    for source in [
        "(define g (case-lambda ((n) (g n 0 0)) ((n acc) acc))) (g 1)",
        "(define g (case-lambda ((n) n) ((n acc) acc))) (g 1 2 3)",
    ] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine.compile("no_clause.scm", source).unwrap();
        let error = engine.eval(&module).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ArityError, "{source}");
        assert!(
            error.to_string().contains("no case-lambda clause"),
            "{source}: {error}"
        );
    }
}

#[test]
fn tail_call_argument_shuffle_is_correct_for_all_small_counts() {
    // Tail calls shift the argument window down to the frame base with an
    // explicit small-copy loop (falling back to `copy_within` above 8
    // registers). Cover every count through both the self-tail and the
    // general-tail paths, including the loop/memmove boundary at 8/9.
    for count in 1..=9usize {
        let params: Vec<String> = (0..count).map(|i| format!("p{i}")).collect();
        let params_list = params.join(" ");
        let rest_args = params[1..].join(" ");
        let sum_terms = params.join(" ");
        // Self tail call: decrement p0, keep the rest; the final sum proves
        // every argument landed in its home register.
        let initial: Vec<String> = (0..count).map(|i| format!("{}", (i + 1) * 10)).collect();
        let expected: i64 = (2..=count as i64).map(|i| i * 10).sum();
        let source = format!(
            "(define (self {params_list})
               (if (= p0 0) (+ 0 {sum_terms}) (self (- p0 1) {rest_args})))
             (define (general {params_list})
               (if (= p0 0) (+ 0 {sum_terms}) (self (- p0 1) {rest_args})))
             (list (self {args}) (general {args}))",
            args = initial.join(" "),
        );
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine.compile("shuffle.scm", &source).unwrap();
        let value = engine.eval(&module).unwrap().into_one().unwrap();
        assert_eq!(
            engine.write_root(&value).unwrap(),
            format!("({expected} {expected})"),
            "count {count}"
        );
    }
    // Zero-argument general tail call.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "shuffle0.scm",
            "(define (leaf) 42)
             (define (loop n) (if (= n 0) (leaf) (loop (- n 1))))
             (loop 3)",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(engine.write_root(&value).unwrap(), "42");
}

#[test]
fn cxr_accessors_compose_car_and_cdr() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "cxr.scm",
            "(define x '((1 2 3) (4 5 6) (7 8 9)))
             (list (caar x) (cadr x) (cddr x) (caddr x) (cadadr x) (caddar x))",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(
        engine.write_root(&value).unwrap(),
        "(1 (4 5 6) ((7 8 9)) (7 8 9) 5 3)"
    );
}

#[test]
fn cxr_on_non_pair_raises_the_same_type_error_as_car() {
    // A non-pair at any depth must raise car's or cdr's pair type error, not a
    // different kind, so the native accessors are drop-in for the old nested
    // car/cdr definitions.
    for source in ["(cadr 5)", "(caar '(1))", "(cddr '(1))", "(caddr '(1 2))"] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine.compile("cxr_err.scm", source).unwrap();
        let error = engine.eval(&module).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TypeError, "{source}");
    }
}

#[test]
fn list_core_natives_match_scheme_semantics() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "list_core.scm",
            "(define original (list 1 2 3))
             (define appended (append '(1 2) '(3 4) '(5)))
             (define shared-tail 99)
             (define with-tail (append '(1 2) shared-tail))
             (list (length '(a b c d))
                   (reverse '(1 2 3))
                   appended
                   with-tail
                   (list-tail '(a b c d e) 2)
                   (list-ref '(a b c d e) 3)
                   (make-list 3 'x)
                   (list-copy original)
                   (eq? (list-copy original) original))",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(
        engine.write_root(&value).unwrap(),
        "(4 (3 2 1) (1 2 3 4 5) (1 2 . 99) (c d e) d (x x x) (1 2 3) #f)"
    );
}

#[test]
fn append_shares_the_last_argument_without_copying() {
    // The final argument is returned as-is, so mutating it through the appended
    // structure is visible in the original. Earlier arguments are fresh copies.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "append_share.scm",
            "(define tail (list 3 4))
             (define whole (append (list 1 2) tail))
             (set-car! (cddr whole) 99)
             (list whole tail (car tail))",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(engine.write_root(&value).unwrap(), "((1 2 99 4) (99 4) 99)");
}

#[test]
fn list_set_mutates_in_place() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "list_set.scm",
            "(define l (list 'a 'b 'c)) (list-set! l 1 'z) l",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(engine.write_root(&value).unwrap(), "(a z c)");
}

#[test]
fn list_core_natives_raise_on_bad_arguments() {
    for (source, kind) in [
        ("(length '(1 2 . 3))", ErrorKind::TypeError),
        ("(length 5)", ErrorKind::TypeError),
        ("(reverse '(1 . 2))", ErrorKind::TypeError),
        ("(list-tail '(1 2) 5)", ErrorKind::TypeError),
        ("(list-ref '(1 2) 5)", ErrorKind::TypeError),
        ("(list-tail '(1 2) -1)", ErrorKind::RangeError),
        ("(append '(1 . 2) '(3))", ErrorKind::TypeError),
    ] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine.compile("list_err.scm", source).unwrap();
        let error = engine.eval(&module).unwrap_err();
        assert_eq!(error.kind(), kind, "{source}");
    }
}

#[test]
fn fast_path_natives_with_wrong_arity_raise_the_canonical_error() {
    // A wrong-arity call to a classified fast native misses the register
    // fast path (the argument count does not match its shape), so the call
    // must fall through to the general path and raise the canonical arity
    // error. Covers both the plain call and the tail-call route.
    for source in ["(car 1 2)", "(cons 1)", "((lambda () (car 1 2)))"] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine.compile("fast_arity.scm", source).unwrap();
        let error = engine.eval(&module).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ArityError, "{source}");
    }
}

#[test]
fn list_natives_raise_on_circular_lists_instead_of_hanging() {
    // A native Rust loop never reaches a fuel safe point, so a circular list must
    // be detected structurally. The fuel limit turns any regression into a fast
    // failure rather than a hang.
    for source in [
        "(define l (list 1 2 3)) (set-cdr! (cddr l) l) (length l)",
        "(define l (list 1 2 3)) (set-cdr! (cddr l) l) (reverse l)",
        "(define l (list 1 2 3)) (set-cdr! (cddr l) l) (list-copy l)",
        "(define l (list 1 2 3)) (set-cdr! (cddr l) l) (append l '(9))",
    ] {
        let limits = Limits::default().with_fuel(Some(5_000_000));
        let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
        let module = engine.compile("circular.scm", source).unwrap();
        let error = engine.eval(&module).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TypeError, "{source}");
        assert!(error.to_string().contains("circular"), "{source}: {error}");
    }
}

#[test]
fn search_natives_match_scheme_semantics() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "search.scm",
            "(list (memq 'c '(a b c d))
                   (memv 2 '(1 2 3))
                   (memq 'x '(a b c))
                   (member \"bb\" '(\"aa\" \"bb\" \"cc\"))
                   (assq 'b '((a . 1) (b . 2) (c . 3)))
                   (assv 2 '((1 . 10) (2 . 20)))
                   (assoc \"k\" '((\"j\" . 1) (\"k\" . 2)))
                   (assq 'z '((a . 1))))",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(
        engine.write_root(&value).unwrap(),
        "((c d) (2 3) #f (\"bb\" \"cc\") (b . 2) (2 . 20) (\"k\" . 2) #f)"
    );
}

#[test]
fn member_and_assoc_honor_a_custom_comparator() {
    // The custom-comparator path stays in Scheme and must keep calling the
    // supplied procedure. Here a case-insensitive-ish comparator matches on a
    // computed key.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "custom_compare.scm",
            "(define (close? a b) (< (abs (- a b)) 2))
             (list (member 5 '(1 10 6 20) close?)
                   (assoc 5 '((1 . a) (6 . b) (10 . c)) close?))",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(engine.write_root(&value).unwrap(), "((6 20) (6 . b))");
}

#[test]
fn member_with_a_failing_comparator_propagates_the_error() {
    // A comparator that raises must surface its error, proving the Scheme
    // dispatch still runs arbitrary user code rather than swallowing it.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "failing_compare.scm",
            "(member 1 '(1 2 3) (lambda (a b) (car 5)))",
        )
        .unwrap();
    let error = engine.eval(&module).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::TypeError);
}

#[test]
fn search_natives_raise_on_circular_lists_instead_of_hanging() {
    for source in [
        "(define l (list 1 2 3)) (set-cdr! (cddr l) l) (memq 'x l)",
        "(define l (list (cons 1 1) (cons 2 2))) (set-cdr! (cdr l) l) (assq 'x l)",
    ] {
        let limits = Limits::default().with_fuel(Some(5_000_000));
        let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
        let module = engine.compile("circular_search.scm", source).unwrap();
        let error = engine.eval(&module).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TypeError, "{source}");
        assert!(error.to_string().contains("circular"), "{source}: {error}");
    }
}

#[test]
fn string_and_vector_conversions_and_copies() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "seq_ops.scm",
            "(list (string->list \"abc\")
                   (list->string '(#\\x #\\y))
                   (vector->list #(1 2 3) 1)
                   (list->vector '(a b))
                   (string->vector \"hi\")
                   (vector->string #(#\\o #\\k))
                   (vector-append #(1 2) #(3) #())
                   (string-copy \"abcde\" 1 3)
                   (substring \"abcde\" 2 4)
                   (vector-copy #(1 2 3 4) 1 3))",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(
        engine.write_root(&value).unwrap(),
        "((#\\a #\\b #\\c) \"xy\" (2 3) #(a b) #(#\\h #\\i) \"ok\" #(1 2 3) \"bc\" \"cd\" #(2 3))"
    );
}

#[test]
fn in_place_fills_and_copies_mutate_the_target() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "seq_mut.scm",
            "(define s (make-string 5 #\\x))
             (string-fill! s #\\- 1 3)
             (define v (vector 1 2 3 4 5))
             (vector-fill! v 'z 2 4)
             (define s2 (string-copy \"abcde\"))
             (string-copy! s2 1 \"12345\" 0 2)
             (define v2 (vector 1 2 3 4 5))
             (vector-copy! v2 1 #(9 8 7) 0 2)
             (list s v s2 v2)",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(
        engine.write_root(&value).unwrap(),
        "(\"x--xx\" #(1 2 z z 5) \"a12de\" #(1 9 8 4 5))"
    );
}

#[test]
fn copy_bang_is_memmove_safe_for_overlapping_source_and_destination() {
    // The old Scheme snapshotted the source range before writing. The natives
    // must preserve that: an overlapping self-copy reads pre-write values.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "overlap.scm",
            "(define s (string-copy \"abcde\"))
             (string-copy! s 1 s 0 2)
             (define v (vector 1 2 3 4 5))
             (vector-copy! v 3 v 0 2)
             (list s v)",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(
        engine.write_root(&value).unwrap(),
        "(\"aabde\" #(1 2 3 1 2))"
    );
}

#[test]
fn sequence_ops_raise_on_bad_bounds_and_elements() {
    for (source, kind) in [
        ("(string-copy \"abc\" 0 5)", ErrorKind::RangeError),
        ("(vector-copy #(1 2 3) 0 9)", ErrorKind::RangeError),
        ("(string->list 5)", ErrorKind::TypeError),
        ("(vector->string #(1 2))", ErrorKind::TypeError),
        (
            "(string-copy! (make-string 2) 1 \"xyz\")",
            ErrorKind::RangeError,
        ),
        ("(string-copy! 1 0 \"\")", ErrorKind::TypeError),
        ("(vector-copy! 1 0 #())", ErrorKind::TypeError),
        (
            "(string-copy! (make-string 2) 3 \"\")",
            ErrorKind::RangeError,
        ),
        (
            "(vector-copy! (make-vector 2) 3 #())",
            ErrorKind::RangeError,
        ),
    ] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine.compile("seq_err.scm", source).unwrap();
        let error = engine.eval(&module).unwrap_err();
        assert_eq!(error.kind(), kind, "{source}");
    }
}

#[test]
fn huge_sequence_constructors_report_heap_limits_without_native_panics() {
    for source in [
        format!("(make-bytevector {})", usize::MAX),
        format!("(make-string {} #\\x)", usize::MAX),
    ] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine.compile("huge_sequence.scm", source).unwrap();
        let error = engine.eval(&module).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::HeapLimitExceeded, "{error}");
    }
}

#[test]
fn not_and_equality_predicates_match_scheme_semantics() {
    // `not` is false only for #f; every other value including () and 0 is truthy.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "trivia.scm",
            "(list (not #f) (not #t) (not '()) (not 0)
                   (boolean=? #t #t #t) (boolean=? #t #t #f)
                   (symbol=? 'a 'a 'a) (symbol=? 'a 'a 'b))",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(
        engine.write_root(&value).unwrap(),
        "(#t #f #f #f #t #f #t #f)"
    );
}

#[test]
fn homogeneous_equality_predicates_reject_other_types() {
    for source in ["(boolean=? 1 1)", "(symbol=? 1 1)"] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine.compile("typed_equality.scm", source).unwrap();
        let error = engine.eval(&module).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TypeError, "{source}");
    }
}

#[test]
fn literal_occurrences_share_one_hoisted_object() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    // A literal occurrence evaluates to the same object every time, and equal
    // literal datums in one unit share one hidden content-named definition.
    let module = engine
        .compile(
            "literal_identity.scm",
            "(define (f) \"abc\") (list (eq? (f) (f)) (eq? \"abc\" \"abc\") (eq? '(1 2) '(1 2)))",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(engine.write_root(&value).unwrap(), "(#t #t #t)");
}

#[test]
fn mutating_a_literal_raises_a_runtime_error() {
    for source in [
        "(string-set! \"abc\" 0 #\\x)",
        "(string-fill! \"abc\" #\\x)",
        "(string-copy! \"abc\" 0 \"zz\")",
        "(vector-set! #(1 2 3) 0 9)",
        "(vector-fill! #(1 2 3) 9)",
        "(vector-copy! #(1 2 3) 0 #(9))",
        "(bytevector-u8-set! #u8(1 2 3) 0 9)",
        "(bytevector-copy! #u8(1 2 3) 0 #u8(9))",
        "(read-bytevector! #u8(0) (open-input-bytevector #u8(9)))",
        "(set-car! '(1 2) 9)",
        "(set-cdr! '(1 2) 9)",
        "(list-set! '(1 2) 0 9)",
    ] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine.compile("literal_mutation.scm", source).unwrap();
        let error = engine.eval(&module).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::RuntimeError, "{source}");
        assert!(error.to_string().contains("immutable"), "{source}: {error}");
    }
}

#[test]
fn mutation_of_literal_copies_still_succeeds() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "literal_copies.scm",
            "(let ((s (string-copy \"ab\"))
                   (v (vector-copy #(1 2)))
                   (b (bytevector-copy #u8(1 2)))
                   (l (list-copy '(1 2))))
               (string-set! s 0 #\\x)
               (vector-set! v 0 9)
               (bytevector-u8-set! b 0 9)
               (set-car! l 9)
               (list s v b l))",
        )
        .unwrap();
    let value = engine.eval(&module).unwrap().into_one().unwrap();
    assert_eq!(
        engine.write_root(&value).unwrap(),
        "(\"xb\" #(9 2) #u8(9 2) (9 2))"
    );
}

#[test]
fn out_of_range_mutation_still_reports_a_range_error() {
    for source in [
        "(vector-set! (vector-copy #(1 2)) 5 9)",
        "(string-set! (string-copy \"ab\") 5 #\\x)",
        "(bytevector-u8-set! (bytevector-copy #u8(1 2)) 5 9)",
    ] {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        let module = engine.compile("literal_range.scm", source).unwrap();
        let error = engine.eval(&module).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::RangeError, "{source}");
    }
}

#[test]
fn globals_defined_across_evals_survive_later_collections() {
    // Regression test for the dirty-flag engine-root refresh: a global defined
    // in one eval must stay rooted through collections triggered by later
    // evals and by host-side collection requests, even though the root vector
    // is no longer rebuilt eagerly on every eval.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let define = engine
        .compile(
            "keeper.scm",
            "(define keeper (make-vector 4 42)) (define (touch) (vector-ref keeper 0))",
        )
        .unwrap();
    engine.eval(&define).unwrap();
    engine.collect_now();
    let churn = engine
        .compile(
            "churn.scm",
            "(let loop ((i 0) (acc '()))
               (if (= i 20000) (length acc) (loop (+ i 1) (cons (make-string 8 #\\a) acc))))",
        )
        .unwrap();
    for _ in 0..5 {
        engine.eval(&churn).unwrap();
        engine.collect_now();
    }
    let read = engine.compile("read.scm", "(touch)").unwrap();
    let value = engine.eval(&read).unwrap().into_one().unwrap().value();
    assert_eq!(value, Value::integer(42));
}
