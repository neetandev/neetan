use r7rs::{Engine, EngineConfig, ErrorKind, Limits, Value};

fn run(source: &str) -> Value {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine.compile("syntax.scm", source).unwrap();
    engine.eval(&module).unwrap().into_one().unwrap().value()
}

#[test]
fn source_forms_compile_and_execute() {
    assert_eq!(
        run("(begin (define (inc x) (+ x 1)) (inc 41))"),
        Value::integer(42)
    );
    assert_eq!(
        run("(let* ((x 2) (y (+ x 3))) (* x y))"),
        Value::integer(10)
    );
    assert_eq!(run("(and #t (or #f 7))"), Value::integer(7));
    assert_eq!(
        run("(cond ((= 1 2) 0) ((= 2 2) => (lambda (x) 9)) (else 1))"),
        Value::integer(9)
    );
    assert_eq!(
        run("(case 2 ((1) 0) ((2 3) 8) (else 9))"),
        Value::integer(8)
    );
}

#[test]
fn nested_quasiquote_decrements_unquote_splicing_depth() {
    assert_eq!(
        run(r#"
            (let ((x 7))
              (equal?
                (quasiquote
                  (quasiquote ((unquote-splicing (unquote x)))))
                '(quasiquote ((unquote-splicing 7)))))
            "#),
        Value::boolean(true)
    );
}

#[test]
fn named_let_internal_definitions_local_syntax_do_and_records_work() {
    assert_eq!(
        run("(let loop ((n 100) (sum 0)) (if (= n 0) sum (loop (- n 1) (+ sum n))))"),
        Value::integer(5050)
    );
    // Variadic self-recursion in tail position exercises the rest-list rebuild
    // on the reused-frame fast path.
    assert_eq!(
        run("(begin
               (define (down . args)
                 (let ((n (car args)) (acc (cadr args)))
                   (if (= n 0) acc (down (- n 1) (+ acc n)))))
               (down 100 0))"),
        Value::integer(5050)
    );
    assert_eq!(
        run("((lambda (x) (define (twice y) (+ y y)) (twice x)) 21)"),
        Value::integer(42)
    );
    assert_eq!(
        run("(let-syntax ((twice (syntax-rules () ((twice x) (+ x x))))) (twice 21))"),
        Value::integer(42)
    );
    assert_eq!(
        run("(do ((i 0 (+ i 1)) (sum 0 (+ sum i))) ((= i 10) sum))"),
        Value::integer(45)
    );
    assert_eq!(
        run("(letrec* ((p (lambda (x) (+ 1 (q (- x 1)))))
                       (q (lambda (y) (if (zero? y) 0 (+ 1 (p (- y 1))))))
                       (x (p 5))
                       (y x))
               y)"),
        Value::integer(5)
    );
    assert_eq!(
        run("(begin
               (define-record-type <point>
                 (make-point x y) point?
                 (x point-x set-point-x!)
                 (y point-y))
               (define point (make-point 2 3))
               (set-point-x! point 39)
               (if (point? point) (+ (point-x point) (point-y point)) 0))"),
        Value::integer(42)
    );
}

#[test]
fn template_generated_empty_list_matches_literal_nil_pattern() {
    // A recursive macro peels one element per step and recurses on the tail
    // `(rest ...)`. When the input is exhausted that ellipsis expands to zero
    // elements, so the recursive call passes an empty list built by the
    // template. That template-generated `()` must match the literal `()` pattern
    // the base case expects, the same way a reader-written `()` does. Regression
    // test for the two empty-list representations diverging in pattern matching.
    assert_eq!(
        run("(begin
               (define-syntax peel
                 (syntax-rules ()
                   ((_ () acc) acc)
                   ((_ (x rest ...) acc) (peel (rest ...) (+ acc x)))))
               (peel (1 2 3 4) 0))"),
        Value::integer(10)
    );
    // The same empty list, once matched, is usable as a self-quoting empty-list
    // expression, matching how a reader `()` lowers.
    assert_eq!(
        run("(begin
               (define-syntax tail-of
                 (syntax-rules ()
                   ((_ (x rest ...)) (rest ...))))
               (null? (tail-of (1))))"),
        Value::boolean(true)
    );
}

#[test]
fn quote_builds_runtime_data_under_forced_gc() {
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile(
            "quote.scm",
            "(equal? '(a 1 #u8(2 3)) (list 'a 1 (bytevector 2 3)))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::boolean(true)
    );
    assert_eq!(
        run("(letrec-syntax
                 ((aux (syntax-rules () ((_ value) value)))
                  (classify-aux
                    (syntax-rules (aux)
                      ((_ aux) 'same)
                      ((_ value) 'different))))
               (equal?
                 (list (classify-aux aux)
                       (let-syntax ((aux (syntax-rules () ((_ value) value))))
                         (classify-aux aux)))
                 '(same different)))"),
        Value::boolean(true)
    );
}

#[test]
fn compiling_cyclic_source_data_returns_a_structured_error() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let error = engine
        .compile("cyclic-quote.scm", "(quote #1=(a . #1#))")
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ExpandError);
}

#[test]
fn syntax_rules_expands_simple_and_repeated_patterns() {
    assert_eq!(
        run("(begin (define-syntax twice (syntax-rules () ((twice x) (+ x x)))) (twice 21))"),
        Value::integer(42)
    );
    assert_eq!(
        run("(begin (define-syntax sum (syntax-rules () ((sum x ...) (+ x ...)))) (sum 1 2 3))"),
        Value::integer(6)
    );
    assert_eq!(
        run("(begin
               (define helper 10)
               (define-syntax use-helper
                 (syntax-rules () ((_ value) (+ helper value))))
               ((lambda (helper) (use-helper 2)) 40))"),
        Value::integer(12)
    );
    assert_eq!(
        run("(begin
               (define-syntax choose
                 (syntax-rules (otherwise)
                   ((_ otherwise value) value)
                   ((_ ignored value) 0)))
               (choose otherwise 42))"),
        Value::integer(42)
    );
    assert_eq!(
        run("(begin
               (define-syntax be-like-begin
                 (syntax-rules ()
                   ((_ name)
                    (define-syntax name
                      (... (syntax-rules ()
                             ((name expression ...)
                              (begin expression ...))))))))
               (be-like-begin sequence)
               (sequence 1 2 3 42))"),
        Value::integer(42)
    );
}

#[test]
fn syntax_rules_freshens_introduced_binders_per_expansion() {
    // A recursive helper accumulates one introduced formal per `<>` slot. Each
    // recursive expansion introduces its own `x`, so the collected formals must
    // be distinct. Before per-expansion freshness the two `x`s collapsed to the
    // same name and the generated `(lambda (x x) ...)` was rejected as a
    // duplicate formal. This is the exact shape SRFI 26's `cut` relies on.
    let program = "(begin
        (define-syntax collect
          (syntax-rules ()
            ((collect proc arg ...) (collect-internal (proc) () arg ...))))
        (define-syntax collect-internal
          (syntax-rules (<>)
            ((collect-internal (call ...) (formal ...))
             (lambda (formal ...) (call ...)))
            ((collect-internal (call ...) (formal ...) <> rest ...)
             (collect-internal (call ... x) (formal ... x) rest ...))
            ((collect-internal (call ...) (formal ...) other rest ...)
             (collect-internal (call ... other) (formal ...) rest ...))))
        _BODY_)";
    // Two slots produce a two-argument procedure with distinct formals.
    assert_eq!(
        run(&program.replace("_BODY_", "(equal? ((collect list <> <>) 1 2) '(1 2))")),
        Value::boolean(true)
    );
    // An introduced formal does not capture a user identifier of the same source
    // name: the non-slot `x` here is the outer binding, not the slot variable.
    assert_eq!(
        run(&program.replace(
            "_BODY_",
            "(let ((x 99)) (equal? ((collect list <> x) 1) '(1 99)))"
        )),
        Value::boolean(true)
    );
}

#[test]
fn syntax_rules_supports_binding_aware_literals_and_nested_templates() {
    assert_eq!(
        run("(begin
               (define-syntax classify
                 (syntax-rules (otherwise)
                   ((_ otherwise) 'literal)
                   ((_ value) 'other)))
               (equal? (list (classify otherwise)
                             (let ((otherwise #f)) (classify otherwise)))
                       '(literal other)))"),
        Value::boolean(true)
    );
    assert_eq!(
        run("(begin
               (define-syntax rows
                 (syntax-rules ()
                   ((_ ((x y) ...)) (list (list x y) ...))))
               (equal? (rows ((1 2) (3 4))) '((1 2) (3 4))))"),
        Value::boolean(true)
    );
    assert_eq!(
        run("(begin
               (define-syntax nested
                 (syntax-rules ()
                   ((_ ((x ...) ...)) (list (list x ...) ...))))
               (equal? (nested ((1 2) (3 4 5))) '((1 2) (3 4 5))))"),
        Value::boolean(true)
    );
    assert_eq!(
        run("(begin
               (define-syntax vector-elements
                 (syntax-rules ()
                   ((_ #(x ...)) (list x ...))))
               (equal? (vector-elements #(1 2 3)) '(1 2 3)))"),
        Value::boolean(true)
    );
}

#[test]
fn syntax_rules_rejects_invalid_patterns() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let duplicate = engine
        .compile(
            "duplicate-pattern-variable.scm",
            "(define-syntax invalid (syntax-rules () ((_ x x) x)))",
        )
        .unwrap_err();
    assert_eq!(duplicate.kind(), ErrorKind::ExpandError);

    let misplaced = engine
        .compile(
            "misplaced-ellipsis.scm",
            "(define-syntax invalid (syntax-rules () ((_ ... x) x)))",
        )
        .unwrap_err();
    assert_eq!(misplaced.kind(), ErrorKind::ExpandError);
}

#[test]
fn recursive_macros_respect_the_expansion_depth_limit() {
    let limits = Limits::default().with_max_expansion_depth(4);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let error = engine
        .compile(
            "macro-depth.scm",
            r#"
            (define-syntax peel
              (syntax-rules ()
                ((_ ()) 0)
                ((_ (head tail ...)) (peel (tail ...)))))
            (peel (1 2 3 4 5 6 7 8))
            "#,
        )
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ExpansionLimitExceeded);
}

#[test]
fn multiple_values_are_rooted_and_single_value_contexts_reject_them() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine.compile("values.scm", "(values 1 2 3)").unwrap();
    let results = engine.eval(&module).unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results.as_slice()[0].value(), Value::integer(1));
    assert_eq!(results.as_slice()[2].value(), Value::integer(3));
    let bad = engine.compile("bad.scm", "(+ (values 1 2) 3)").unwrap();
    assert_eq!(
        engine.eval(&bad).unwrap_err().kind(),
        ErrorKind::RuntimeError
    );
    let module = engine.compile("good.scm", "(+ 1 2)").unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(3)
    );
}

#[test]
fn call_with_values_forwards_the_full_result_packet() {
    assert_eq!(
        run("(call-with-values (lambda () (values 4 5)) (lambda (a b) (+ a b)))"),
        Value::integer(9)
    );
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "forward.scm",
            "(call-with-values (lambda () (values 4 5)) (lambda (a b) (values b a)))",
        )
        .unwrap();
    let results = engine.eval(&module).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results.as_slice()[0].value(), Value::integer(5));
    let module = engine
        .compile(
            "first-class-values.scm",
            "(call-with-values (lambda () (values 1 2)) values)",
        )
        .unwrap();
    let results = engine.eval(&module).unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn sequential_value_bindings_and_definitions_receive_all_values() {
    assert_eq!(
        run("(let ((x 10)) (let-values (((a) (values x)) ((b) (values x))) (+ a b)))"),
        Value::integer(20)
    );
    assert_eq!(
        run("(let*-values (((a b) (values 2 3)) ((c) (values (+ a b)))) (+ b c))"),
        Value::integer(8)
    );
    assert_eq!(
        run("(begin (define-values (a b) (values 6 7)) (+ a b))"),
        Value::integer(13)
    );
}

#[test]
fn delayed_computations_are_memoized() {
    assert_eq!(
        run("(let* ((n 0) (p (delay (begin (set! n (+ n 1)) n)))) (force p) (force p) n)"),
        Value::integer(1)
    );
    assert_eq!(run("(promise? (delay-force 1))"), Value::boolean(true));
    assert_eq!(run("(force (make-promise 12))"), Value::integer(12));
    assert_eq!(
        run("(begin
               (define count 0)
               (define p
                 (delay
                   (if (> count 5)
                       count
                       (begin (set! count (+ count 1)) (force p)))))
               (force p))"),
        Value::integer(6)
    );
}

#[test]
fn delay_force_flattens_deep_promise_chains() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile(
            "delay-force.scm",
            "(begin
               (define (chain n)
                 (if (= n 0)
                     (delay 42)
                     (delay-force (chain (- n 1)))))
               (force (chain 1000)))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(42)
    );
}

#[test]
fn parameters_are_callable_dynamic_values() {
    assert_eq!(run("(let ((p (make-parameter 7))) (p))"), Value::integer(7));
    assert_eq!(
        run("(let ((p (make-parameter 7))) (+ (parameterize ((p 9)) (p)) (p)))"),
        Value::integer(16)
    );
    assert_eq!(
        run("(let ((p (make-parameter 2 (lambda (x) (* x 10)))))
               (+ (p) (parameterize ((p 3)) (p)) (p)))"),
        Value::integer(70)
    );
}

#[test]
fn case_lambda_single_clause_uses_the_regular_closure_path() {
    assert_eq!(run("((case-lambda ((x) (+ x 1))) 41)"), Value::integer(42));
    assert_eq!(
        run(
            "(+ ((case-lambda (() 10) ((x) x) ((x y . rest) (+ x y (car rest)))) 1)
                 ((case-lambda (() 10) ((x) x) ((x y . rest) (+ x y (car rest)))) 1 2 3))"
        ),
        Value::integer(7)
    );
    assert_eq!(run("((lambda args (car args)) 8 9)"), Value::integer(8));
    assert_eq!(
        run("((lambda (x . rest) (+ x (car rest))) 2 3)"),
        Value::integer(5)
    );
    assert_eq!(
        run("(begin (define (sum first . rest) (+ first (car rest))) (sum 4 5))"),
        Value::integer(9)
    );
}

#[test]
fn exception_handlers_receive_raised_values() {
    assert_eq!(
        run("(with-exception-handler (lambda (x) (+ x 1)) (lambda () (raise-continuable 4)))"),
        Value::integer(5)
    );
}

#[test]
fn guard_dispatches_error_objects_and_reraises_unmatched_conditions() {
    assert_eq!(
        run("(guard (condition
                ((error-object? condition)
                 (car (error-object-irritants condition))))
               (error \"bad\" 42))"),
        Value::integer(42)
    );
    assert_eq!(
        run("(with-exception-handler
               (lambda (condition) 9)
               (lambda ()
                 (guard (condition (#f 0))
                   (raise-continuable 2))))"),
        Value::integer(9)
    );
    assert_eq!(
        run("(let ((n 0))
               (guard (condition (#t 0))
                 (dynamic-wind
                   (lambda () (set! n (+ n 1)))
                   (lambda () (error \"boom\"))
                   (lambda () (set! n (+ n 10)))))
               n)"),
        Value::integer(11)
    );
}

#[test]
fn continuations_restore_the_captured_execution_state() {
    assert_eq!(
        run("(+ 1 (call/cc (lambda (escape) (escape 41) 0)))"),
        Value::integer(42)
    );
}

#[test]
fn continuations_are_multi_shot_and_accept_multiple_values() {
    assert_eq!(
        run("(call/cc (lambda (k) (procedure? k)))"),
        Value::boolean(true)
    );
    assert_eq!(run("(procedure? (make-parameter 1))"), Value::boolean(true));
    assert_eq!(
        run("(let ((saved #f) (count 0))
               (+ (call/cc (lambda (k) (set! saved k) 1))
                  (if (< count 2)
                      (begin (set! count (+ count 1)) (saved 10))
                      0)))"),
        Value::integer(10)
    );
    assert_eq!(
        run("(call-with-values
               (lambda () (call/cc (lambda (k) (k 4 5))))
               (lambda (a b) (+ a b)))"),
        Value::integer(9)
    );
}

#[test]
fn control_objects_survive_collection_on_every_allocation() {
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let module = engine
        .compile(
            "gc-control.scm",
            "(let ((p (delay (call/cc (lambda (k) (k 42)))))) (force p))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(42)
    );
}

#[test]
fn dynamic_wind_runs_before_and_after_around_the_thunk() {
    assert_eq!(
        run(
            "(let ((n 0)) (dynamic-wind (lambda () (set! n (+ n 1))) (lambda () (set! n (+ n 10)) n) (lambda () (set! n (+ n 100)))) n)"
        ),
        Value::integer(111)
    );
}

#[test]
fn loop_closures_capture_distinct_per_iteration_cells() {
    // Each iteration of the named let re-enters the loop frame, so its captured
    // parameter `i` must be boxed afresh on entry; otherwise every closure would
    // alias one cell and observe the final value. Expect 0 + 1 + 2 = 3.
    assert_eq!(
        run("(begin
               (define (sum-thunks fns)
                 (if (null? fns) 0 (+ ((car fns)) (sum-thunks (cdr fns)))))
               (let loop ((i 0) (fns '()))
                 (if (= i 3)
                     (sum-thunks fns)
                     (loop (+ i 1) (cons (lambda () i) fns)))))"),
        Value::integer(3)
    );
    // Two closures over the same binding share one cell: a `set!` through the
    // first is observed by the second (independent of argument evaluation order).
    assert_eq!(
        run("(begin
               (define (make)
                 (let ((n 0))
                   (cons (lambda () (set! n (+ n 1)))
                         (lambda () n))))
               (let ((c (make)))
                 ((car c)) ((car c))
                 ((cdr c))))"),
        Value::integer(2)
    );
}

#[test]
fn continuation_transfer_runs_winders_and_restores_parameters() {
    assert_eq!(
        run("(let ((p (make-parameter 1)) (saved #f) (n 0))
               (dynamic-wind
                 (lambda () (set! n (+ n 1)))
                 (lambda ()
                   (parameterize ((p 9))
                     (call/cc (lambda (k) (set! saved k) 0))))
                 (lambda () (set! n (+ n 10))))
               (if saved
                   (let ((k saved)) (set! saved #f) (k 0))
                   (+ n (p))))"),
        Value::integer(23)
    );
}
