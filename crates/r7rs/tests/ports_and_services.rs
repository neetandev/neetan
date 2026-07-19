use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use r7rs::{
    Clock, Engine, EngineConfig, ErrorKind, FileSystem, HostIoError, Limits, LoadedSource,
    PortResource, ProcessContext, SourceLoader, SourceLoaderError, SourceRequest, Value,
};

fn run(engine: &mut Engine, source: &str) -> Value {
    let module = engine.compile("ports_and_services.scm", source).unwrap();
    engine.eval(&module).unwrap().into_one().unwrap().value()
}

#[test]
fn in_memory_text_and_binary_ports_round_trip() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-output-string))) (write-string \"hello\" p) (newline p) (string=? (get-output-string p) \"hello\\n\"))"
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-input-bytevector #u8(1 2)))) (and (= (peek-u8 p) 1) (= (read-u8 p) 1) (= (read-u8 p) 2) (eof-object? (read-u8 p))))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn empty_binary_input_does_not_preallocate_the_requested_read_count() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let source = format!(
        "(eof-object?
           (read-bytevector {} (open-input-bytevector #u8())))",
        usize::MAX
    );
    assert_eq!(run(&mut engine, &source), Value::boolean(true));
}

#[test]
fn read_and_write_use_ports() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-input-string \"(1 . 2) #t\"))) (and (equal? (read p) '(1 . 2)) (eq? (read p) #t) (eof-object? (read p))))"
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-output-string))) (write '(a 2) p) (string=? (get-output-string p) \"(a 2)\"))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn port_combinators_preserve_values_and_rebind_defaults() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-output-string))) (call-with-values (lambda () (call-with-port p (lambda (q) (values 4 5)))) (lambda (a b) (and (= a 4) (= b 5) (eq? (output-port-open? p) #f)))))",
        ),
        Value::boolean(true)
    );
}

#[test]
fn runtime_writer_uses_datum_labels_for_cycles_and_sharing() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (cons 1 '())) (out (open-output-string))) (set-cdr! p p) (write p out) (string=? (get-output-string out) \"#0=(1 . #0#)\"))",
        ),
        Value::boolean(true)
    );
    assert_eq!(
        run(
            &mut engine,
            "(let* ((p (cons 1 '())) (v (vector p p)) (out (open-output-string))) (write-shared v out) (string=? (get-output-string out) \"#(#0=(1) #0#)\"))",
        ),
        Value::boolean(true)
    );
}

#[test]
fn read_and_file_failures_are_catchable_conditions() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(
            &mut engine,
            "(guard (condition ((read-error? condition) 'caught)) (read (open-input-string \"(\")))",
        ),
        run(&mut engine, "'caught")
    );
    engine.set_file_system(Box::new(Files));
    assert_eq!(
        run(
            &mut engine,
            "(guard (condition ((file-error? condition) 'caught)) (open-output-file \"fixture\"))",
        ),
        run(&mut engine, "'caught")
    );
    assert_eq!(
        run(
            &mut engine,
            "(guard (condition ((file-error? condition) 'caught)) (call-with-output-file \"fixture\" (lambda (port) #f)))",
        ),
        run(&mut engine, "'caught")
    );
}

#[test]
fn file_operations_are_denied_without_a_capability() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let module = engine
        .compile("ports-and-services.scm", "(open-input-file \"denied\")")
        .unwrap();
    assert!(engine.eval(&module).is_err());
}

struct TextFile {
    characters: Vec<char>,
    position: usize,
}

impl PortResource for TextFile {
    fn read_char(&mut self) -> Result<Option<char>, HostIoError> {
        let value = self.characters.get(self.position).copied();
        self.position += usize::from(value.is_some());
        Ok(value)
    }

    fn read_u8(&mut self) -> Result<Option<u8>, HostIoError> {
        Err(HostIoError::new("not a binary port"))
    }

    fn write_char(&mut self, _: char) -> Result<(), HostIoError> {
        Ok(())
    }

    fn write_u8(&mut self, _: u8) -> Result<(), HostIoError> {
        Err(HostIoError::new("not a binary port"))
    }

    fn char_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(true)
    }

    fn u8_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(false)
    }

    fn flush(&mut self) -> Result<(), HostIoError> {
        Ok(())
    }

    fn close(&mut self) -> Result<(), HostIoError> {
        Ok(())
    }
}

struct Files;

struct LoadSources;

struct TrackingFile {
    closes: Arc<AtomicUsize>,
}

impl PortResource for TrackingFile {
    fn read_char(&mut self) -> Result<Option<char>, HostIoError> {
        Ok(None)
    }

    fn read_u8(&mut self) -> Result<Option<u8>, HostIoError> {
        Ok(None)
    }

    fn write_char(&mut self, _: char) -> Result<(), HostIoError> {
        Ok(())
    }

    fn write_u8(&mut self, _: u8) -> Result<(), HostIoError> {
        Ok(())
    }

    fn char_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(true)
    }

    fn u8_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(true)
    }

    fn flush(&mut self) -> Result<(), HostIoError> {
        Ok(())
    }

    fn close(&mut self) -> Result<(), HostIoError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl SourceLoader for LoadSources {
    fn load(&mut self, request: SourceRequest<'_>) -> Result<LoadedSource, SourceLoaderError> {
        Ok(LoadedSource::new(
            request.requested(),
            request.requested(),
            "(define loaded-value 42)",
        ))
    }
}

struct StaticSource(&'static str);

impl SourceLoader for StaticSource {
    fn load(&mut self, request: SourceRequest<'_>) -> Result<LoadedSource, SourceLoaderError> {
        Ok(LoadedSource::new(
            request.requested(),
            request.requested(),
            self.0,
        ))
    }
}

impl FileSystem for Files {
    fn open_input(&mut self, _: &str, _: bool) -> Result<Box<dyn PortResource>, HostIoError> {
        Ok(Box::new(TextFile {
            characters: "ok".chars().collect(),
            position: 0,
        }))
    }

    fn open_output(&mut self, _: &str, _: bool) -> Result<Box<dyn PortResource>, HostIoError> {
        Err(HostIoError::new("not used"))
    }

    fn exists(&mut self, path: &str) -> Result<bool, HostIoError> {
        Ok(path == "fixture")
    }

    fn delete(&mut self, _: &str) -> Result<(), HostIoError> {
        Ok(())
    }
}

#[test]
fn installed_file_capability_creates_host_backed_ports() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_file_system(Box::new(Files));
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-input-file \"fixture\"))) (and (file-exists? \"fixture\") (char=? (read-char p) #\\o) (char=? (read-char p) #\\k)))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn failed_port_object_allocation_closes_the_host_resource() {
    let closes = Arc::new(AtomicUsize::new(0));
    let limits = Limits::default()
        .with_initial_gc_threshold(64)
        .with_max_heap_slots(512);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    let exhaust = engine
        .compile(
            "exhaust.scm",
            "(define kept '())
             (let loop ((i 0))
               (set! kept (cons i kept))
               (loop (+ i 1)))",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&exhaust).unwrap_err().kind(),
        ErrorKind::HeapLimitExceeded
    );
    assert_eq!(
        engine
            .set_standard_input(Box::new(TrackingFile {
                closes: closes.clone(),
            }))
            .unwrap_err()
            .kind(),
        ErrorKind::HeapLimitExceeded
    );
    assert_eq!(closes.load(Ordering::SeqCst), 1);
}

#[test]
fn read_and_peek_work_on_host_backed_ports() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_file_system(Box::new(Files));
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-input-file \"fixture\"))) (and (char=? (peek-char p) #\\o) (eq? (read p) 'ok)))",
        ),
        Value::boolean(true)
    );
}

#[test]
fn file_port_combinators_use_vm_cleanup_paths() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_file_system(Box::new(Files));
    assert_eq!(
        run(
            &mut engine,
            "(and (eq? (call-with-input-file \"fixture\" (lambda (p) (read p))) 'ok) (eq? (with-input-from-file \"fixture\" (lambda () (read))) 'ok))",
        ),
        Value::boolean(true)
    );
}

#[test]
fn load_compiles_into_the_requested_runtime_environment() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_source_loader(Box::new(LoadSources));
    assert_eq!(
        run(&mut engine, "(begin (load \"loaded.scm\") loaded-value)"),
        Value::integer(42)
    );
    assert_eq!(
        run(
            &mut engine,
            "(begin (load \"loaded.scm\" (interaction-environment)) loaded-value)",
        ),
        Value::integer(42)
    );
}

#[test]
fn runtime_load_enforces_source_limits() {
    let limits = r7rs::Limits::default()
        .with_max_source_bytes(20)
        .with_max_token_bytes(20);
    let mut engine = Engine::new(EngineConfig::default().with_limits(limits)).unwrap();
    engine.set_source_loader(Box::new(StaticSource("                     ")));
    let module = engine.compile("main.scm", "(load \"large.scm\")").unwrap();
    assert_eq!(
        engine.eval(&module).unwrap_err().kind(),
        ErrorKind::SourceTooLarge
    );
}

#[test]
fn runtime_load_diagnostics_name_the_loaded_source() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_source_loader(Box::new(StaticSource("(define value")));
    let module = engine.compile("main.scm", "(load \"broken.scm\")").unwrap();
    let error = engine.eval(&module).unwrap_err();
    assert!(engine.render_error(&error).contains("broken.scm"));
}

struct Process;

impl ProcessContext for Process {
    fn command_line(&mut self) -> Result<Vec<String>, HostIoError> {
        Ok(vec!["r7rs".to_owned(), "fixture".to_owned()])
    }
    fn environment_variable(&mut self, name: &str) -> Result<Option<String>, HostIoError> {
        Ok((name == "KEY").then_some("value".to_owned()))
    }
    fn environment_variables(&mut self) -> Result<Vec<(String, String)>, HostIoError> {
        Ok(vec![("KEY".to_owned(), "value".to_owned())])
    }
    fn exit(&mut self, _: Option<i64>, _: bool) -> Result<(), HostIoError> {
        Ok(())
    }
}

type ExitCalls = Arc<Mutex<Vec<(Option<i64>, bool)>>>;

struct RecordingProcess {
    exits: ExitCalls,
}

impl ProcessContext for RecordingProcess {
    fn command_line(&mut self) -> Result<Vec<String>, HostIoError> {
        Ok(Vec::new())
    }

    fn environment_variable(&mut self, _: &str) -> Result<Option<String>, HostIoError> {
        Ok(None)
    }

    fn environment_variables(&mut self) -> Result<Vec<(String, String)>, HostIoError> {
        Ok(Vec::new())
    }

    fn exit(&mut self, value: Option<i64>, emergency: bool) -> Result<(), HostIoError> {
        self.exits.lock().unwrap().push((value, emergency));
        Ok(())
    }
}

struct TestClock;

impl Clock for TestClock {
    fn current_second(&mut self) -> Result<f64, HostIoError> {
        Ok(12.5)
    }
    fn current_jiffy(&mut self) -> Result<i64, HostIoError> {
        Ok(125)
    }
    fn jiffies_per_second(&mut self) -> Result<i64, HostIoError> {
        Ok(10)
    }
}

#[test]
fn process_and_clock_capabilities_are_engine_local() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_process_context(Box::new(Process));
    engine.set_clock(Box::new(TestClock));
    assert_eq!(
        run(
            &mut engine,
            "(and (equal? (command-line) '(\"r7rs\" \"fixture\")) (string=? (get-environment-variable \"KEY\") \"value\") (= (current-jiffy) 125) (= (jiffies-per-second) 10) (= (current-second) 12.5))"
        ),
        Value::boolean(true)
    );
}

#[test]
fn exit_returns_a_typed_outcome_after_dynamic_wind_cleanup() {
    let exits = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_process_context(Box::new(RecordingProcess {
        exits: exits.clone(),
    }));
    let module = engine
        .compile(
            "exit.scm",
            "(define cleanup 0) (dynamic-wind (lambda () #f) (lambda () (exit 7)) (lambda () (set! cleanup (+ cleanup 1))))",
        )
        .unwrap();
    let outcome = engine.eval(&module).unwrap();
    let status = outcome.exit_status().unwrap();
    assert_eq!(status.code(), Some(7));
    assert!(!status.emergency());
    assert_eq!(run(&mut engine, "cleanup"), Value::integer(1));
    assert_eq!(*exits.lock().unwrap(), vec![(Some(7), false)]);
}

#[test]
fn emergency_exit_skips_dynamic_wind_cleanup() {
    let exits = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_process_context(Box::new(RecordingProcess {
        exits: exits.clone(),
    }));
    let module = engine
        .compile(
            "emergency-exit.scm",
            "(define cleanup 0) (dynamic-wind (lambda () #f) (lambda () (emergency-exit 9)) (lambda () (set! cleanup (+ cleanup 1))))",
        )
        .unwrap();
    let status = engine.eval(&module).unwrap().exit_status().unwrap();
    assert_eq!(status.code(), Some(9));
    assert!(status.emergency());
    assert_eq!(run(&mut engine, "cleanup"), Value::integer(0));
    assert_eq!(*exits.lock().unwrap(), vec![(Some(9), true)]);
}

#[test]
fn host_provided_values_stay_write_protected() {
    // Host-derived strings and pairs (environment data) are marked immutable on
    // construction. The guarded mutators resolve the slot once (immutability
    // check fused into the same lookup), and every refusal surfaces as the
    // dedicated immutability error.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine.set_process_context(Box::new(Process));

    let module = engine
        .compile(
            "immutable-env-string.scm",
            "(string-set! (get-environment-variable \"KEY\") 0 #\\z)",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap_err().kind(),
        ErrorKind::RuntimeError,
        "in-range write to an immutable string must still be refused",
    );
    assert_eq!(
        run(
            &mut engine,
            "(string=? (get-environment-variable \"KEY\") \"value\")"
        ),
        Value::boolean(true),
    );

    let module = engine
        .compile(
            "immutable-env-pair.scm",
            "(set-car! (car (get-environment-variables)) \"X\")",
        )
        .unwrap();
    assert_eq!(
        engine.eval(&module).unwrap_err().kind(),
        ErrorKind::RuntimeError,
        "write to an immutable pair must still be refused",
    );
}

#[test]
fn string_ports_handle_multibyte_contents_by_char() {
    // Guards the UTF-8 port buffers: positions are byte offsets internally,
    // but every Scheme-visible operation stays char-based.
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // read-char/peek-char step whole chars of every width.
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-input-string (string #\\a #\\λ #\\x1F700 #\\z))))
               (and (char=? (peek-char p) #\\a)
                    (char=? (read-char p) #\\a)
                    (char=? (peek-char p) #\\λ)
                    (char=? (read-char p) #\\λ)
                    (char=? (read-char p) #\\x1F700)
                    (char=? (read-char p) #\\z)
                    (eof-object? (read-char p))))"
        ),
        Value::boolean(true)
    );

    // read-string counts chars, not bytes.
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-input-string \"αβγδε\")))
               (and (string=? (read-string 2 p) \"αβ\")
                    (string=? (read-string 10 p) \"γδε\")
                    (eof-object? (read-string 1 p))))"
        ),
        Value::boolean(true)
    );

    // read-line returns multibyte lines intact.
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-input-string \"höhe\\nλμν\")))
               (and (string=? (read-line p) \"höhe\")
                    (string=? (read-line p) \"λμν\")
                    (eof-object? (read-line p))))"
        ),
        Value::boolean(true)
    );

    // Datum reads and char reads interleave on the same multibyte buffer.
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-input-string \"(α β) γ\")))
               (and (equal? (read p) (list (string->symbol \"α\") (string->symbol \"β\")))
                    (char=? (read-char p) #\\space)
                    (char=? (read-char p) #\\γ)
                    (eof-object? (read-char p))))"
        ),
        Value::boolean(true)
    );

    // Output ports accumulate mixed-width chars and report them back intact.
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-output-string)))
               (write-char #\\a p)
               (write-char #\\λ p)
               (write-char #\\x1F700 p)
               (write-string \"xß\" p)
               (let ((s (get-output-string p)))
                 (and (= (string-length s) 5)
                      (string=? s (string #\\a #\\λ #\\x1F700 #\\x #\\ß)))))"
        ),
        Value::boolean(true)
    );

    // display/write of multibyte strings through a port round-trips.
    assert_eq!(
        run(
            &mut engine,
            "(let ((p (open-output-string)))
               (display \"αβ\" p)
               (write \"γ\" p)
               (string=? (get-output-string p) \"αβ\\\"γ\\\"\"))"
        ),
        Value::boolean(true)
    );
}
