use r7rs::{DatumKind, Engine, EngineConfig, ErrorKind, Number, Real};

fn read(source: &str) -> r7rs::Datum {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .reader_from_str("test.scm", source)
        .unwrap()
        .read_next()
        .unwrap()
        .unwrap()
}

#[test]
fn streams_datums_and_tracks_source() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let mut reader = engine.reader_from_str("many.scm", "one 2 #t").unwrap();
    let first = reader.read_next().unwrap().unwrap();
    assert!(matches!(
        first.kind(first.root()),
        Some(DatumKind::Symbol("one"))
    ));
    let second = reader.read_next().unwrap().unwrap();
    assert!(matches!(
        second.kind(second.root()),
        Some(DatumKind::Number(Number::Real(Real::ExactInteger(2))))
    ));
    let third = reader.read_next().unwrap().unwrap();
    assert!(matches!(
        third.kind(third.root()),
        Some(DatumKind::Boolean(true))
    ));
    assert!(reader.read_next().unwrap().is_none());
}

#[test]
fn reads_compound_data_and_abbreviations() {
    let datum = read("'(a . #(\"x\" #u8(1 2 255)))");
    assert_eq!(datum.to_external(), "(quote (a . #(\"x\" #u8(1 2 255))))");
}

#[test]
fn labels_preserve_cycles_and_print_round_trip_form() {
    let datum = read("#1=(a . #1#)");
    assert_eq!(datum.to_external(), "#0=(a . #0#)");
    let root = datum.root();
    let DatumKind::Pair { cdr, .. } = datum.kind(root).unwrap() else {
        panic!("pair expected");
    };
    assert!(matches!(datum.kind(cdr), Some(DatumKind::Pair { .. })));
}

#[test]
fn datum_comments_do_not_export_label_definitions() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let mut reader = engine
        .reader_from_str("commented-label", "#;#1=(hidden) #1#")
        .unwrap();
    assert_eq!(
        reader.read_next().unwrap_err().kind(),
        ErrorKind::InvalidDatumLabel
    );

    let mut reader = engine
        .reader_from_str("reused-label", "#;#1=(hidden) #1=(visible)")
        .unwrap();
    let datum = reader.read_next().unwrap().unwrap();
    assert_eq!(datum.to_external(), "(visible)");

    let mut reader = engine
        .reader_from_str("nested-commented-label", "(#;#1=(hidden) #1#)")
        .unwrap();
    assert_eq!(
        reader.read_next().unwrap_err().kind(),
        ErrorKind::InvalidDatumLabel
    );

    let datum = read("#1=(a #;#1# b)");
    assert_eq!(datum.to_external(), "(a b)");
}

#[test]
fn parses_numbers_and_exact_decimals() {
    let datum = read("#e1.50");
    assert!(
        matches!(datum.kind(datum.root()), Some(DatumKind::Number(Number::Real(Real::ExactRational(value)))) if value.numerator() == 3 && value.denominator() == 2)
    );
    let datum = read("3-2i");
    assert!(matches!(
        datum.kind(datum.root()),
        Some(DatumKind::Number(Number::Rectangular { .. }))
    ));
}

#[test]
fn reader_reports_invalid_bytes_and_reuses_engine() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        engine.reader_from_bytes("bad", [0xFF]).unwrap_err().kind(),
        ErrorKind::InvalidUtf8
    );
    assert!(
        engine
            .reader_from_str("good", "ok")
            .unwrap()
            .read_next()
            .unwrap()
            .is_some()
    );
}

#[test]
fn comments_directives_and_escapes_work() {
    let datum = read("#!fold-case #| nested #| comment |# |# |MiXeD\\x21;|");
    assert!(matches!(
        datum.kind(datum.root()),
        Some(DatumKind::Symbol("mixed!"))
    ));
    let datum = read("||");
    assert!(matches!(
        datum.kind(datum.root()),
        Some(DatumKind::Symbol(""))
    ));
}

#[test]
fn character_hex_and_string_continuations_follow_reader_rules() {
    let datum = read("#\\x3bb");
    assert!(matches!(
        datum.kind(datum.root()),
        Some(DatumKind::Character('λ'))
    ));
    let datum = read("\"one\\\n  two\"");
    assert!(matches!(
        datum.kind(datum.root()),
        Some(DatumKind::String("onetwo"))
    ));
}

#[test]
fn character_literals_accept_delimiter_characters() {
    // A character literal is `#\` followed by any single character, including
    // delimiters such as brackets, braces, and parentheses. The following
    // delimiter must be taken as the character itself, not end the token early.
    for (source, expected) in [
        ("#\\[", '['),
        ("#\\]", ']'),
        ("#\\{", '{'),
        ("#\\}", '}'),
        ("#\\(", '('),
        ("#\\)", ')'),
        ("#\\|", '|'),
    ] {
        let datum = read(source);
        assert!(
            matches!(datum.kind(datum.root()), Some(DatumKind::Character(c)) if c == expected),
            "{source} should read as the character {expected:?}"
        );
    }
}

#[test]
fn invalid_labels_and_limits_are_structured() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let mut reader = engine.reader_from_str("labels", "#1#").unwrap();
    assert_eq!(
        reader.read_next().unwrap_err().kind(),
        ErrorKind::InvalidDatumLabel
    );

    let limits = r7rs::Limits::default().with_max_token_bytes(2);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let mut reader = engine.reader_from_str("limit", "toolong").unwrap();
    assert_eq!(
        reader.read_next().unwrap_err().kind(),
        ErrorKind::ReaderLimitExceeded
    );
}
