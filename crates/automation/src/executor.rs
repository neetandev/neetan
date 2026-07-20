//! The shared executor core that runs one script on the calling thread.
//!
//! The engine, session, and message sender never cross a thread boundary. Only
//! the `InterruptToken` inside the `CancelHandle` is shared with the watchdog.
//! `execute_script` emits the ordered `MessageProtocol` stream and a single
//! terminal `Finished` message.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

use r7rs::{Engine, EngineConfig, ErrorKind, EvalOutcome, Extension, Limits};

use crate::{
    capabilities::{
        CapturedOutput, FixedClock, RootedFileSystem, RootedSourceLoader, ScriptProcessContext,
    },
    config::CommonConfig,
    protocol::{ExecutionResult, MessageProtocol, RunTermination},
    scheme::register_libraries,
    session::AutomationSession,
    watchdog::{CancelHandle, Watchdog},
};

/// The instruction fuel backstop against an infinite pure-Scheme loop that the
/// wall-clock watchdog somehow fails to interrupt. The cooperative deadline is
/// the primary bound; fuel is secondary.
const DEFAULT_FUEL: u64 = 50_000_000_000;

/// Runs one script to completion, streaming the message protocol to `events`.
pub fn execute_script(
    script: PathBuf,
    common: CommonConfig,
    arguments: Vec<String>,
    events: Sender<MessageProtocol>,
    cancel: CancelHandle,
) {
    let _ = events.send(MessageProtocol::Started {
        script: script.clone(),
    });
    let termination = run_inner(&script, &common, arguments, &events, &cancel);
    let _ = events.send(MessageProtocol::Finished(termination));
}

fn run_inner(
    script: &Path,
    common: &CommonConfig,
    arguments: Vec<String>,
    events: &Sender<MessageProtocol>,
    cancel: &CancelHandle,
) -> RunTermination {
    let source = match std::fs::read_to_string(script) {
        Ok(source) => source,
        Err(error) => {
            return RunTermination::ConfigError(format!(
                "cannot read script {}: {error}",
                script.display()
            ));
        }
    };

    let script_dir = script
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let artifact_root = common.artifact_root_for(script);

    let config = EngineConfig::default()
        .with_limits(Limits::default().with_fuel(Some(DEFAULT_FUEL)))
        .with_interrupt_token(cancel.interrupt_token());

    let mut engine = match Engine::new(config) {
        Ok(engine) => engine,
        Err(error) => return RunTermination::Internal(format!("engine creation failed: {error}")),
    };

    for extension in Extension::ALL {
        if let Err(error) = engine.install_extension(*extension) {
            return RunTermination::Internal(format!("extension installation failed: {error}"));
        }
    }

    let session = Rc::new(RefCell::new(AutomationSession::new(
        events.clone(),
        cancel.clone(),
        common.clone(),
        common.host_date_time_source(),
        common.audio_sample_rate(),
        script_dir.clone(),
        artifact_root.clone(),
    )));

    let mut command_line = Vec::with_capacity(arguments.len() + 1);
    command_line.push(script.to_string_lossy().into_owned());
    command_line.extend(arguments);

    if let Err(error) = install_capabilities(
        &mut engine,
        events,
        &script_dir,
        &artifact_root,
        command_line,
        &session,
    ) {
        return RunTermination::Internal(error);
    }

    if let Err(error) = register_libraries(&mut engine, &session) {
        return RunTermination::Internal(format!("library registration failed: {error}"));
    }

    let script_name = script.file_name().map_or_else(
        || script.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    );

    let module = match engine.compile_program(script_name, source) {
        Ok(module) => module,
        Err(error) => return RunTermination::CompileError(error.diagnostic().clone()),
    };

    let watchdog = Watchdog::spawn(
        cancel.clone(),
        events.clone(),
        Duration::from_secs(common.timeout_seconds),
        Instant::now(),
    );

    let outcome = engine.eval(&module);
    drop(watchdog);

    // Release every control the script left held, and flush writable media and
    // printer output, on any exit path.
    session.borrow_mut().close_active_machine();

    match outcome {
        Ok(EvalOutcome::Values(_)) | Ok(EvalOutcome::Exited(_)) => {
            match session.borrow().result() {
                Some(ExecutionResult::Ok) => RunTermination::Completed(ExecutionResult::Ok),
                Some(ExecutionResult::Error { message }) => {
                    RunTermination::Completed(ExecutionResult::Error {
                        message: message.clone(),
                    })
                }
                None => RunTermination::NoResult,
            }
        }
        Err(error) if error.kind() == ErrorKind::ExecutionLimitExceeded => {
            if cancel.cancel_requested() && !cancel.deadline_tripped() {
                RunTermination::Cancelled
            } else {
                RunTermination::Timeout
            }
        }
        Err(error) => RunTermination::RuntimeError(error.diagnostic().clone()),
    }
}

fn install_capabilities(
    engine: &mut Engine,
    events: &Sender<MessageProtocol>,
    script_dir: &Path,
    artifact_root: &Path,
    command_line: Vec<String>,
    session: &Rc<RefCell<AutomationSession>>,
) -> Result<(), String> {
    engine
        .set_standard_output(Box::new(CapturedOutput::new(events.clone())))
        .map_err(|error| format!("cannot install output port: {error}"))?;
    engine
        .set_standard_error(Box::new(CapturedOutput::new(events.clone())))
        .map_err(|error| format!("cannot install error port: {error}"))?;
    engine.set_source_loader(Box::new(RootedSourceLoader::new(script_dir.to_path_buf())));
    engine.set_file_system(Box::new(RootedFileSystem::new(
        script_dir.to_path_buf(),
        artifact_root.to_path_buf(),
    )));
    engine.set_clock(Box::new(FixedClock));
    engine.set_process_context(Box::new(ScriptProcessContext::new(
        command_line,
        Rc::clone(session),
    )));
    Ok(())
}
