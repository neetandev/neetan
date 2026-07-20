use r7rs::{
    CoreExpr, Engine, EngineConfig, ErrorKind, LibraryName, LibraryNameComponent, Limits,
    NativeValues, Value, ValueKind,
};

fn library_name(parts: &[&str]) -> LibraryName {
    LibraryName::new(
        parts
            .iter()
            .map(|part| LibraryNameComponent::identifier(*part)),
    )
    .unwrap()
}

fn literal(value: Value) -> CoreExpr {
    CoreExpr::literal(value)
}

fn variable(name: &str) -> CoreExpr {
    CoreExpr::variable(name)
}

fn call(name: &str, arguments: Vec<CoreExpr>) -> CoreExpr {
    CoreExpr::Call {
        procedure: Box::new(variable(name)),
        arguments,
    }
}

fn eval(engine: &mut Engine, expression: CoreExpr) -> r7rs::Root {
    let module = engine.compile_core(&expression).unwrap();
    engine.eval(&module).unwrap().into_one().unwrap()
}

#[test]
fn pairs_and_sequences_are_mutable_through_primitive_globals() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let pair = eval(
        &mut engine,
        call(
            "cons",
            vec![literal(Value::integer(1)), literal(Value::integer(2))],
        ),
    );
    assert_eq!(engine.value_kind(&pair).unwrap(), ValueKind::Pair);
    let (car, cdr) = engine.pair_values(&pair).unwrap();
    assert_eq!(car.value(), Value::integer(1));
    assert_eq!(cdr.value(), Value::integer(2));
    engine.set_pair_car(&pair, &cdr).unwrap();
    assert_eq!(
        engine.pair_values(&pair).unwrap().0.value(),
        Value::integer(2)
    );

    let vector = eval(
        &mut engine,
        CoreExpr::Begin(vec![
            CoreExpr::Define {
                name: "v".into(),
                value: Box::new(call(
                    "vector",
                    vec![literal(Value::integer(1)), literal(Value::integer(2))],
                )),
            },
            call(
                "vector-set!",
                vec![
                    variable("v"),
                    literal(Value::integer(1)),
                    literal(Value::integer(9)),
                ],
            ),
            call(
                "vector-ref",
                vec![variable("v"), literal(Value::integer(1))],
            ),
        ]),
    );
    assert_eq!(vector.value(), Value::integer(9));

    let string = eval(
        &mut engine,
        CoreExpr::Begin(vec![
            CoreExpr::Define {
                name: "s".into(),
                value: Box::new(call(
                    "string",
                    vec![
                        literal(Value::character('a')),
                        literal(Value::character('b')),
                    ],
                )),
            },
            call(
                "string-set!",
                vec![
                    variable("s"),
                    literal(Value::integer(0)),
                    literal(Value::character('λ')),
                ],
            ),
            call(
                "string-ref",
                vec![variable("s"), literal(Value::integer(0))],
            ),
        ]),
    );
    assert_eq!(string.value(), Value::character('λ'));

    let byte = eval(
        &mut engine,
        CoreExpr::Begin(vec![
            CoreExpr::Define {
                name: "b".into(),
                value: Box::new(call(
                    "bytevector",
                    vec![literal(Value::integer(1)), literal(Value::integer(2))],
                )),
            },
            call(
                "bytevector-u8-set!",
                vec![
                    variable("b"),
                    literal(Value::integer(1)),
                    literal(Value::integer(255)),
                ],
            ),
            call(
                "bytevector-u8-ref",
                vec![variable("b"), literal(Value::integer(1))],
            ),
        ]),
    );
    assert_eq!(byte.value(), Value::integer(255));
}

#[test]
fn symbols_equality_and_cycles_terminate() {
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let symbol_equal = eval(
        &mut engine,
        call(
            "eq?",
            vec![
                call(
                    "string->symbol",
                    vec![call("string", vec![literal(Value::character('a'))])],
                ),
                call(
                    "string->symbol",
                    vec![call("string", vec![literal(Value::character('a'))])],
                ),
            ],
        ),
    );
    assert_eq!(symbol_equal.value(), Value::boolean(true));

    let result = eval(
        &mut engine,
        CoreExpr::Begin(vec![
            CoreExpr::Define {
                name: "a".into(),
                value: Box::new(call(
                    "cons",
                    vec![literal(Value::integer(1)), literal(Value::nil())],
                )),
            },
            CoreExpr::Define {
                name: "b".into(),
                value: Box::new(call(
                    "cons",
                    vec![literal(Value::integer(1)), literal(Value::nil())],
                )),
            },
            call("set-cdr!", vec![variable("a"), variable("a")]),
            call("set-cdr!", vec![variable("b"), variable("b")]),
            call("equal?", vec![variable("a"), variable("b")]),
        ]),
    );
    assert_eq!(result.value(), Value::boolean(true));
    let list = eval(
        &mut engine,
        CoreExpr::Begin(vec![
            CoreExpr::Define {
                name: "c".into(),
                value: Box::new(call(
                    "cons",
                    vec![literal(Value::integer(1)), literal(Value::nil())],
                )),
            },
            call("set-cdr!", vec![variable("c"), variable("c")]),
            call("list?", vec![variable("c")]),
        ]),
    );
    assert_eq!(list.value(), Value::boolean(false));
}

#[test]
fn numeric_core_checks_types_and_exact_overflow() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        eval(
            &mut engine,
            call(
                "+",
                vec![literal(Value::integer(2)), literal(Value::integer(3))]
            )
        )
        .value(),
        Value::integer(5)
    );
    assert_eq!(
        eval(
            &mut engine,
            call(
                "/",
                vec![literal(Value::integer(8)), literal(Value::integer(2))]
            )
        )
        .value(),
        Value::integer(4)
    );
    assert_eq!(
        eval(
            &mut engine,
            call("inexact", vec![literal(Value::integer(2))])
        )
        .value(),
        Value::float(2.0)
    );

    let module = engine
        .compile(
            "overflow.scm",
            "(import (scheme base)) (+ 170141183460469231731687303715884105727 1)",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap_err().kind(),
        ErrorKind::ImplementationRestriction
    );
    let module = engine
        .compile_core(&call(
            "vector-ref",
            vec![call("vector", vec![]), literal(Value::integer(0))],
        ))
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap_err().kind(),
        ErrorKind::RangeError
    );
}

#[test]
fn native_callbacks_keep_arguments_and_results_alive_during_collection() {
    let limits = Limits::default().with_initial_gc_threshold(1);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let native = library_name(&["test", "native", "gc"]);
    engine
        .register_library_fn(&native, "host-pair", 2..=2, |cx, args| {
            let first = cx.to_i128(args[0])?;
            cx.collect_now();
            let first = cx.integer(first)?;
            cx.pair(first, args[1])
        })
        .unwrap();
    engine
        .register_library_fn(
            &native,
            "host-panic",
            0..=0,
            |cx, _| -> Result<Value, r7rs::Error> {
                let _temporary = cx.pair(Value::integer(1), Value::nil())?;
                panic!("test")
            },
        )
        .unwrap();
    let module = engine
        .compile(
            "native-gc.scm",
            "(import (scheme base) (test native gc)) (host-pair 18446744073709551615 8)",
        )
        .unwrap();
    let result = engine.eval(&module).unwrap().into_one().unwrap();
    let (first, second) = engine.pair_values(&result).unwrap();
    assert_eq!(engine.write_root(&first).unwrap(), u64::MAX.to_string());
    assert_eq!(second.value(), Value::integer(8));
    engine.collect_now();
    let first = engine.pair_values(&result).unwrap().0;
    assert_eq!(engine.write_root(&first).unwrap(), u64::MAX.to_string());

    let module = engine
        .compile(
            "native-panic.scm",
            "(import (scheme base) (test native gc)) (host-panic)",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap_err().kind(),
        ErrorKind::NativePanic
    );
    assert_eq!(
        eval(
            &mut engine,
            call(
                "+",
                vec![literal(Value::integer(1)), literal(Value::integer(2))]
            )
        )
        .value(),
        Value::integer(3)
    );
}

#[test]
fn native_callbacks_can_deliver_multiple_values() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let native = library_name(&["test", "native", "values"]);
    engine
        .register_library_fn(&native, "host-values", 0..=0, |_, _| {
            Ok(NativeValues::many([Value::integer(4), Value::integer(5)]))
        })
        .unwrap();
    engine
        .register_library_fn(&native, "host-noop", 0..=0, |_, _| Ok(Value::unspecified()))
        .unwrap();
    let module = engine
        .compile(
            "native-values.scm",
            "(import (scheme base) (test native values)) (call-with-values host-values (lambda (a b) (+ a b)))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(9)
    );
    let module = engine
        .compile(
            "native-wind.scm",
            "(import (scheme base) (test native values)) (dynamic-wind host-noop (lambda () 42) host-noop)",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap().into_one().unwrap().value(),
        Value::integer(42)
    );
}

#[test]
fn roots_cannot_be_mixed_between_engines() {
    let mut first = Engine::new(EngineConfig::default()).unwrap();
    let mut second = Engine::new(EngineConfig::default()).unwrap();
    let one = eval(
        &mut first,
        call(
            "cons",
            vec![literal(Value::integer(1)), literal(Value::nil())],
        ),
    );
    let two = eval(
        &mut second,
        call(
            "cons",
            vec![literal(Value::integer(2)), literal(Value::nil())],
        ),
    );
    assert_eq!(
        first.make_pair(&one, &two).unwrap_err().kind(),
        ErrorKind::WrongEngine
    );
}
