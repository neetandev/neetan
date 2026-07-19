use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
};

use r7rs::{
    Engine, EngineConfig, ErrorKind, InterruptToken, Limits, LoadedSource, SourceLoader,
    SourceLoaderError, SourceRequest, SourceRetention, Span,
};

#[test]
fn engines_have_independent_state() {
    let mut first = Engine::new(EngineConfig::default()).unwrap();
    let mut second = Engine::new(EngineConfig::default()).unwrap();

    first.interrupt_token().interrupt();
    assert!(first.interrupt_token().is_interrupted());
    assert!(!second.interrupt_token().is_interrupted());

    let first_source = first.add_source("same.scm", "α").unwrap();
    let second_source = second.add_source("same.scm", "abcdef").unwrap();
    let first_location = first
        .source_location(Span::new(first_source, 2, 2).unwrap())
        .unwrap();
    let second_location = second
        .source_location(Span::new(second_source, 2, 2).unwrap())
        .unwrap();
    assert_eq!(first_location.column(), 2);
    assert_eq!(second_location.column(), 3);
}

#[test]
fn roots_preserve_the_full_exact_integer_range() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let minimum = engine.root_integer(i128::MIN).unwrap();
    let unsigned_max = engine.root_integer(i128::from(u64::MAX)).unwrap();
    assert_eq!(engine.write_root(&minimum).unwrap(), i128::MIN.to_string());
    assert_eq!(
        engine.write_root(&unsigned_max).unwrap(),
        u64::MAX.to_string()
    );
}

#[test]
fn compiled_modules_cannot_cross_engine_boundaries() {
    let mut first = Engine::new(EngineConfig::default()).unwrap();
    let mut second = Engine::new(EngineConfig::default()).unwrap();
    let module = first.compile("owned.scm", "42").unwrap();
    assert_eq!(
        second.eval(&module).unwrap_err().kind(),
        ErrorKind::WrongEngine
    );
    assert_eq!(
        first.eval(&module).unwrap().into_one().unwrap().value(),
        r7rs::Value::integer(42)
    );
}

#[test]
fn host_can_share_an_interrupt_token_deliberately() {
    let token = InterruptToken::new();
    let engine = Engine::new(EngineConfig::default().with_interrupt_token(token.clone())).unwrap();
    token.interrupt();
    assert!(engine.interrupt_token().is_interrupted());
    engine.interrupt_token().reset();
    assert!(!token.is_interrupted());
}

#[test]
fn configuration_is_validated() {
    let error = Engine::new(
        EngineConfig::default().with_limits(Limits::default().with_max_source_bytes(0)),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidConfiguration);

    let error = Engine::new(
        EngineConfig::default().with_limits(
            Limits::default()
                .with_max_source_bytes(10)
                .with_max_token_bytes(11),
        ),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidConfiguration);

    let error =
        Engine::new(EngineConfig::default().with_limits(Limits::default().with_fuel(Some(0))))
            .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidConfiguration);
}

#[test]
fn source_limit_failures_leave_engine_reusable() {
    let limits = Limits::default()
        .with_max_source_bytes(4)
        .with_max_token_bytes(4);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();

    let error = engine.add_source("large.scm", "12345").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::SourceTooLarge);

    let source = engine.add_source("small.scm", "1234").unwrap();
    let location = engine
        .source_location(Span::new(source, 4, 4).unwrap())
        .unwrap();
    assert_eq!((location.line(), location.column()), (1, 5));
}

#[test]
fn locations_use_bytes_for_spans_and_unicode_scalars_for_columns() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let source = engine.add_source("lines.scm", "αβ\r\nx🙂\n").unwrap();

    let location = engine
        .source_location(Span::new(source, 2, 2).unwrap())
        .unwrap();
    assert_eq!((location.line(), location.column()), (1, 2));

    let location = engine
        .source_location(Span::new(source, 7, 7).unwrap())
        .unwrap();
    assert_eq!((location.line(), location.column()), (2, 2));

    let error = engine
        .source_location(Span::new(source, 1, 1).unwrap())
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidSpan);
}

#[test]
fn metadata_retention_preserves_unicode_locations() {
    let mut engine =
        Engine::new(EngineConfig::default().with_source_retention(SourceRetention::Metadata))
            .unwrap();
    let source = engine.add_source("meta.scm", "λx\n🙂z").unwrap();
    let location = engine
        .source_location(Span::new(source, 8, 8).unwrap())
        .unwrap();
    assert_eq!((location.line(), location.column()), (2, 2));
}

type RecordedRequests = Arc<Mutex<Vec<(String, Option<String>)>>>;

struct QueueLoader {
    replies: VecDeque<Result<LoadedSource, SourceLoaderError>>,
    requests: RecordedRequests,
}

impl SourceLoader for QueueLoader {
    fn load(&mut self, request: SourceRequest<'_>) -> Result<LoadedSource, SourceLoaderError> {
        self.requests.lock().unwrap().push((
            request.requested().to_owned(),
            request.including_identity().map(str::to_owned),
        ));
        self.replies.pop_front().unwrap()
    }
}

#[test]
fn source_loading_is_denied_by_default() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let error = engine.load_source("file.scm", None).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::SourceLoadingDenied);
}

#[test]
fn loader_receives_canonical_parent_and_caches_identity() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let replies = VecDeque::from([
        Ok(LoadedSource::new("pkg/root", "root.scm", "(root)")),
        Ok(LoadedSource::new("pkg/child", "child.scm", "(child)")),
        Ok(LoadedSource::new("pkg/child", "alternate.scm", "(child)")),
    ]);
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_source_loader(Box::new(QueueLoader {
        replies,
        requests: requests.clone(),
    }));

    let root = engine.load_source("root.scm", None).unwrap();
    let child = engine.load_source("child.scm", Some(root)).unwrap();
    let cached = engine.load_source("./child.scm", Some(root)).unwrap();
    assert_eq!(child, cached);
    assert_eq!(
        *requests.lock().unwrap(),
        vec![
            ("root.scm".into(), None),
            ("child.scm".into(), Some("pkg/root".into())),
            ("./child.scm".into(), Some("pkg/root".into())),
        ]
    );
}

#[test]
fn loader_errors_and_identity_conflicts_are_structured() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let replies = VecDeque::from([
        Err(Box::new(io::Error::other("offline")) as SourceLoaderError),
        Ok(LoadedSource::new("same", "a.scm", "a")),
        Ok(LoadedSource::new("same", "b.scm", "b")),
    ]);
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_source_loader(Box::new(QueueLoader { replies, requests }));

    let error = engine.load_source("missing.scm", None).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::SourceLoadFailed);
    assert_eq!(error.diagnostic().cause(), Some("offline"));

    engine.load_source("a.scm", None).unwrap();
    let error = engine.load_source("b.scm", None).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ConflictingSourceIdentity);

    engine.clear_source_loader();
    assert_eq!(
        engine.load_source("again.scm", None).unwrap_err().kind(),
        ErrorKind::SourceLoadingDenied
    );
}
