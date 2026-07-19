#![cfg(feature = "host-capabilities")]

use std::{
    cell::RefCell,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use r7rs::{Engine, EngineConfig, HostIoError, PortResource, Value};

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

fn run(engine: &mut Engine, source: &str) -> Value {
    let module = engine.compile("host-capabilities.scm", source).unwrap();
    engine.eval(&module).unwrap().into_one().unwrap().value()
}

fn temporary_path(extension: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "r7rs-host-{}-{}.{extension}",
        std::process::id(),
        NEXT_PATH.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn standalone_profile_installs_file_source_process_and_clock_capabilities() {
    let data = temporary_path("txt");
    let source = temporary_path("scm");
    std::fs::write(&source, "(define loaded-value 42)").unwrap();
    let mut engine = Engine::new(EngineConfig::standalone()).unwrap();
    let program = format!(
        "(begin
           (call-with-output-file {data:?} (lambda (port) (write-string \"hello\" port)))
           (define text (call-with-input-file {data:?} read-line))
           (if (and (string=? text \"hello\")
                    (pair? (command-line))
                    (number? (current-second))
                    (begin (load {source:?}) (= loaded-value 42)))
               42
               0))"
    );
    assert_eq!(run(&mut engine, &program), Value::integer(42));
    std::fs::remove_file(data).unwrap();
    std::fs::remove_file(source).unwrap();
}

/// A textual output resource recording every write and flush, standing in for
/// a host stream such as process stdout.
struct RecordingOutput {
    text: Rc<RefCell<String>>,
    flushes: Rc<RefCell<u32>>,
}

impl PortResource for RecordingOutput {
    fn read_char(&mut self) -> Result<Option<char>, HostIoError> {
        Err(HostIoError::new("port is not textual input"))
    }

    fn read_u8(&mut self) -> Result<Option<u8>, HostIoError> {
        Err(HostIoError::new("port is not binary input"))
    }

    fn write_char(&mut self, value: char) -> Result<(), HostIoError> {
        self.text.borrow_mut().push(value);
        Ok(())
    }

    fn write_u8(&mut self, _value: u8) -> Result<(), HostIoError> {
        Err(HostIoError::new("port is not binary output"))
    }

    fn char_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(false)
    }

    fn u8_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(false)
    }

    fn flush(&mut self) -> Result<(), HostIoError> {
        *self.flushes.borrow_mut() += 1;
        Ok(())
    }

    fn close(&mut self) -> Result<(), HostIoError> {
        Ok(())
    }
}

/// A textual input resource over a fixed string, standing in for a host
/// stream such as process stdin.
struct FixedInput {
    data: String,
    position: usize,
}

impl PortResource for FixedInput {
    fn read_char(&mut self) -> Result<Option<char>, HostIoError> {
        let value = self.data[self.position..].chars().next();
        if let Some(value) = value {
            self.position += value.len_utf8();
        }
        Ok(value)
    }

    fn read_u8(&mut self) -> Result<Option<u8>, HostIoError> {
        Err(HostIoError::new("port is not binary input"))
    }

    fn write_char(&mut self, _value: char) -> Result<(), HostIoError> {
        Err(HostIoError::new("port is not textual output"))
    }

    fn write_u8(&mut self, _value: u8) -> Result<(), HostIoError> {
        Err(HostIoError::new("port is not binary output"))
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

/// Installs a recording resource as standard output and returns the engine
/// plus the shared text and flush-count cells.
fn engine_with_recorded_output() -> (Engine, Rc<RefCell<String>>, Rc<RefCell<u32>>) {
    let text = Rc::new(RefCell::new(String::new()));
    let flushes = Rc::new(RefCell::new(0));
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .set_standard_output(Box::new(RecordingOutput {
            text: Rc::clone(&text),
            flushes: Rc::clone(&flushes),
        }))
        .unwrap();
    (engine, text, flushes)
}

/// Regression: display output used to accumulate invisibly in the default
/// in-memory buffer with no way to reach the host. An installed standard
/// output resource receives it, and `flush-output-port` reaches its flush.
#[test]
fn installed_standard_output_receives_display_output() {
    let (mut engine, text, flushes) = engine_with_recorded_output();
    assert_eq!(
        run(
            &mut engine,
            r#"(begin (display "x") (newline) (flush-output-port) 1)"#
        ),
        Value::integer(1)
    );
    assert_eq!(text.borrow().as_str(), "x\n");
    assert_eq!(*flushes.borrow(), 1);
}

/// The installed resource only replaces the parameter's base value, so
/// `parameterize` still redirects output away from it and back.
#[test]
fn parameterize_still_overrides_an_installed_standard_output() {
    let (mut engine, text, _flushes) = engine_with_recorded_output();
    let inner = run(
        &mut engine,
        r#"
        (let ((port (open-output-string)))
          (parameterize ((current-output-port port))
            (display "inner"))
          (display "outer")
          (string=? "inner" (get-output-string port)))
        "#,
    );
    assert_eq!(inner, Value::boolean(true));
    assert_eq!(text.borrow().as_str(), "outer");
}

/// An installed standard input resource feeds the ordinary read procedures
/// through `current-input-port`.
#[test]
fn installed_standard_input_feeds_read_line() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .set_standard_input(Box::new(FixedInput {
            data: "hello\nrest".to_owned(),
            position: 0,
        }))
        .unwrap();
    assert_eq!(
        run(&mut engine, r#"(string=? "hello" (read-line))"#),
        Value::boolean(true)
    );
}

/// An installed standard error resource receives `current-error-port` output.
#[test]
fn installed_standard_error_receives_error_port_output() {
    let text = Rc::new(RefCell::new(String::new()));
    let flushes = Rc::new(RefCell::new(0));
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    engine
        .set_standard_error(Box::new(RecordingOutput {
            text: Rc::clone(&text),
            flushes: Rc::clone(&flushes),
        }))
        .unwrap();
    run(
        &mut engine,
        r#"(begin (write-string "oops" (current-error-port)) 1)"#,
    );
    assert_eq!(text.borrow().as_str(), "oops");
}

/// Without an installed resource, the sandboxed default keeps its silent
/// engine-local buffers: display succeeds and grants no host authority.
#[test]
fn default_engine_output_stays_engine_local() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        run(&mut engine, r#"(begin (display "captured") 1)"#),
        Value::integer(1)
    );
}
