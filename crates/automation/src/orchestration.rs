//! Bounded parallel orchestration for repository Scheme tests.

use std::{
    collections::VecDeque,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{
    CommonConfig, ExecutionResult, MessageProtocol, RunTermination, TestCaseOutcome,
    execute_script, watchdog::CancelHandle,
};

const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_MESSAGES_PER_POLL: usize = 256;

/// An orchestration failure that prevents an aggregate test result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrchestrationError {
    /// The command or discovered test tree is invalid.
    Configuration(String),
    /// The coordinator cannot preserve its reporting contract.
    Internal(String),
}

impl OrchestrationError {
    /// Returns the process exit code for this orchestration failure.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            Self::Configuration(_) => 2,
            Self::Internal(_) => 4,
        }
    }
}

impl std::fmt::Display for OrchestrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Configuration(message) | Self::Internal(message) => formatter.write_str(message),
        }
    }
}

/// Stable identity assigned to a discovered script.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ScriptId(pub usize);

/// The aggregate outcome of one script.
#[derive(Clone, Debug)]
pub enum ScriptOutcome {
    /// The script completed successfully and every reported case passed.
    Passed,
    /// The script completed with a test failure.
    Failed(String),
    /// The script did not complete normally.
    Errored(String),
}

/// One event understood by the orchestration reporter.
#[derive(Clone, Debug)]
pub enum OrchestrationProtocol {
    /// Test discovery completed.
    Discovered { total: usize },
    /// One script began execution.
    ScriptStarted { id: ScriptId, path: PathBuf },
    /// One Scheme test case completed.
    TestCaseFinished {
        id: ScriptId,
        suite: String,
        test_case: String,
        outcome: TestCaseOutcome,
    },
    /// One script completed.
    ScriptFinished {
        id: ScriptId,
        path: PathBuf,
        outcome: ScriptOutcome,
        duration: Duration,
    },
}

#[derive(Debug)]
struct DiscoveredScript {
    id: ScriptId,
    path: PathBuf,
    relative: PathBuf,
}

struct ActiveScript {
    script: DiscoveredScript,
    receiver: Receiver<MessageProtocol>,
    worker: Option<JoinHandle<()>>,
    cancel: CancelHandle,
    output: String,
    case_failures: Vec<String>,
    passed_cases: usize,
    failed_cases: usize,
    termination: Option<RunTermination>,
}

impl Drop for ActiveScript {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.cancel.request_cancel();
            let _ = worker.join();
        }
    }
}

#[derive(Debug)]
struct FailedScript {
    id: ScriptId,
    path: PathBuf,
    output: String,
    details: Vec<String>,
}

/// Runs every Scheme script below `directory` and writes the test report.
pub fn orchestrate(
    directory: &Path,
    common: CommonConfig,
    jobs: usize,
    output: &mut dyn Write,
) -> Result<i32, OrchestrationError> {
    orchestrate_with_interval(directory, common, jobs, output, PROGRESS_INTERVAL)
}

fn orchestrate_with_interval(
    directory: &Path,
    common: CommonConfig,
    jobs: usize,
    output: &mut dyn Write,
    progress_interval: Duration,
) -> Result<i32, OrchestrationError> {
    if jobs == 0 {
        return Err(OrchestrationError::Configuration(
            "jobs must be greater than zero".to_owned(),
        ));
    }
    let scripts = discover_scripts(directory).map_err(OrchestrationError::Configuration)?;
    if scripts.is_empty() {
        return Err(OrchestrationError::Configuration(format!(
            "no .scm tests found beneath {}",
            directory.display()
        )));
    }

    let total = scripts.len();
    let mut queue: VecDeque<DiscoveredScript> = scripts.into();
    let mut active = Vec::new();
    let mut failures = Vec::new();
    let mut finished = 0;
    let mut failed_files = 0;
    let mut passed_cases = 0;
    let mut failed_cases = 0;
    let mut last_progress = Instant::now();

    while !queue.is_empty() || !active.is_empty() {
        while active.len() < jobs {
            let Some(script) = queue.pop_front() else {
                break;
            };
            active.push(start_script(script, &common));
        }

        let mut received_message = false;
        let mut index = 0;
        while index < active.len() {
            let mut disconnected = false;
            for _ in 0..MAX_MESSAGES_PER_POLL {
                match active[index].receiver.try_recv() {
                    Ok(message) => {
                        received_message = true;
                        handle_message(&mut active[index], message, output)?;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }

            if active[index].termination.is_some() || disconnected {
                let completed = active.swap_remove(index);
                let (passed_script, failure, script_passed_cases, script_failed_cases) =
                    finish_script(completed, output)?;
                finished += 1;
                if !passed_script {
                    failed_files += 1;
                }
                passed_cases += script_passed_cases;
                failed_cases += script_failed_cases;
                if let Some(failure) = failure {
                    failures.push(failure);
                }
            } else {
                index += 1;
            }
        }

        if finished < total && last_progress.elapsed() >= progress_interval {
            writeln!(output, "Finished {finished} of {total} test files")
                .map_err(report_write_error)?;
            output.flush().map_err(report_write_error)?;
            last_progress = Instant::now();
        }

        if !received_message && !active.is_empty() {
            thread::sleep(IDLE_POLL_INTERVAL);
        }
    }

    writeln!(output, "Finished {finished} of {total} test files").map_err(report_write_error)?;
    failures.sort_by_key(|failure| failure.id);
    if !failures.is_empty() {
        writeln!(output, "\nfailures:\n").map_err(report_write_error)?;
        for failure in &failures {
            writeln!(output, "---- {} output ----", failure.path.display())
                .map_err(report_write_error)?;
            if !failure.output.is_empty() {
                write!(output, "{}", failure.output).map_err(report_write_error)?;
                if !failure.output.ends_with('\n') {
                    writeln!(output).map_err(report_write_error)?;
                }
            }
            for detail in &failure.details {
                writeln!(output, "{detail}").map_err(report_write_error)?;
            }
            writeln!(output).map_err(report_write_error)?;
        }
    }
    writeln!(
        output,
        "test result: {}. {passed_cases} passed; {failed_cases} failed; {} total",
        if failed_files == 0 {
            "SUCCESS"
        } else {
            "FAILURE"
        },
        passed_cases + failed_cases
    )
    .map_err(report_write_error)?;
    output.flush().map_err(report_write_error)?;

    Ok(i32::from(failed_files != 0))
}

fn discover_scripts(directory: &Path) -> Result<Vec<DiscoveredScript>, String> {
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        format!(
            "cannot inspect test directory {}: {error}",
            directory.display()
        )
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!(
            "test path is not a directory: {}",
            directory.display()
        ));
    }

    let mut paths = Vec::new();
    discover_below(directory, &mut paths)?;
    paths.sort();
    Ok(paths
        .into_iter()
        .enumerate()
        .map(|(index, path)| DiscoveredScript {
            id: ScriptId(index),
            relative: path
                .strip_prefix(directory)
                .expect("discovered path must remain below its root")
                .to_path_buf(),
            path,
        })
        .collect())
}

fn discover_below(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(directory).map_err(|error| {
        format!(
            "cannot read test directory {}: {error}",
            directory.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "cannot read an entry beneath {}: {error}",
                directory.display()
            )
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect test path {}: {error}", path.display()))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            discover_below(&path, paths)?;
        } else if file_type.is_file()
            && path.extension().is_some_and(|extension| extension == "scm")
        {
            paths.push(path);
        }
    }
    Ok(())
}

fn start_script(script: DiscoveredScript, common: &CommonConfig) -> ActiveScript {
    let mut script_common = common.clone();
    if let Some(root) = &common.artifact_root {
        let mut relative = script.relative.clone();
        relative.set_extension("");
        script_common.artifact_root = Some(root.join(relative));
    }

    let (sender, receiver) = mpsc::channel();
    let cancel = CancelHandle::new();
    let worker_cancel = cancel.clone();
    let path = script.path.clone();
    let worker = thread::spawn(move || {
        execute_script(path, script_common, Vec::new(), sender, worker_cancel);
    });

    ActiveScript {
        script,
        receiver,
        worker: Some(worker),
        cancel,
        output: String::new(),
        case_failures: Vec::new(),
        passed_cases: 0,
        failed_cases: 0,
        termination: None,
    }
}

fn handle_message(
    active: &mut ActiveScript,
    message: MessageProtocol,
    output: &mut dyn Write,
) -> Result<(), OrchestrationError> {
    match message {
        MessageProtocol::Output(text) => active.output.push_str(&text),
        MessageProtocol::TestCaseFinished {
            suite,
            test_case,
            outcome,
        } => {
            match outcome {
                TestCaseOutcome::Success => {
                    active.passed_cases += 1;
                    writeln!(output, "{suite} - {test_case} ...... SUCCESS")
                        .map_err(report_write_error)?;
                }
                TestCaseOutcome::Failure { kind, message } => {
                    active.failed_cases += 1;
                    writeln!(output, "{suite} - {test_case} ...... FAILURE")
                        .map_err(report_write_error)?;
                    active
                        .case_failures
                        .push(format!("{suite} - {test_case}: {kind}: {message}"));
                }
            }
            output.flush().map_err(report_write_error)?;
        }
        MessageProtocol::Finished(termination) => active.termination = Some(termination),
        MessageProtocol::Started { .. }
        | MessageProtocol::MachineReady { .. }
        | MessageProtocol::Progress(_)
        | MessageProtocol::Result(_) => {}
    }
    Ok(())
}

fn finish_script(
    mut active: ActiveScript,
    output: &mut dyn Write,
) -> Result<(bool, Option<FailedScript>, usize, usize), OrchestrationError> {
    let joined = active
        .worker
        .take()
        .expect("active script must own its worker")
        .join();
    if joined.is_err() {
        return Err(OrchestrationError::Internal(format!(
            "executor thread panicked while running {}",
            active.script.relative.display()
        )));
    }
    let termination = active.termination.take().ok_or_else(|| {
        OrchestrationError::Internal(format!(
            "executor channel closed without Finished for {}",
            active.script.relative.display()
        ))
    })?;

    let terminal_failure = termination_failure(&termination);
    let passed = terminal_failure.is_none() && active.case_failures.is_empty();
    let abnormal_termination = !matches!(termination, RunTermination::Completed(_));
    let case_count = active.passed_cases + active.failed_cases;
    if case_count == 0 || abnormal_termination {
        writeln!(
            output,
            "{} ...... {}",
            active.script.relative.display(),
            if passed { "SUCCESS" } else { "FAILURE" }
        )
        .map_err(report_write_error)?;
        output.flush().map_err(report_write_error)?;
        if passed {
            active.passed_cases += 1;
        } else {
            active.failed_cases += 1;
        }
    }

    if passed {
        return Ok((true, None, active.passed_cases, active.failed_cases));
    }

    if let Some(detail) = terminal_failure {
        active.case_failures.push(detail);
    }
    let failure = FailedScript {
        id: active.script.id,
        path: active.script.relative.clone(),
        output: std::mem::take(&mut active.output),
        details: std::mem::take(&mut active.case_failures),
    };
    Ok((
        false,
        Some(failure),
        active.passed_cases,
        active.failed_cases,
    ))
}

fn termination_failure(termination: &RunTermination) -> Option<String> {
    match termination {
        RunTermination::Completed(ExecutionResult::Ok) => None,
        RunTermination::Completed(ExecutionResult::Error { message }) => {
            Some(format!("test failure: {message}"))
        }
        RunTermination::NoResult => Some("script ended without an execution result".to_owned()),
        RunTermination::Timeout => Some("script timed out".to_owned()),
        RunTermination::Cancelled => Some("script was cancelled".to_owned()),
        RunTermination::ConfigError(message) => Some(format!("configuration error: {message}")),
        RunTermination::CompileError(diagnostic) => {
            Some(format!("compile error: {}", diagnostic.message()))
        }
        RunTermination::RuntimeError(diagnostic) => {
            Some(format!("runtime error: {}", diagnostic.message()))
        }
        RunTermination::Internal(message) => Some(format!("internal error: {message}")),
    }
}

fn report_write_error(error: io::Error) -> OrchestrationError {
    OrchestrationError::Internal(format!("cannot write orchestration report: {error}"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    use super::{discover_scripts, orchestrate_with_interval};
    use crate::CommonConfig;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    fn temporary_directory(name: &str) -> PathBuf {
        let identifier = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir()
            .join("neetan-orchestration-tests")
            .join(format!("{name}-{}-{identifier}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create test directory");
        path
    }

    fn write_script(directory: &Path, name: &str, body: &str) {
        let path = directory.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create script parent");
        }
        fs::write(path, body).expect("write Scheme test");
    }

    #[test]
    fn discovery_requires_a_directory() {
        let error = discover_scripts(std::path::Path::new("definitely-missing-test-directory"))
            .expect_err("missing directory must fail");
        assert!(error.contains("cannot inspect test directory"));
    }

    #[test]
    fn discovery_is_recursive_sorted_and_unicode_safe() {
        let directory = temporary_directory("discovery");
        write_script(&directory, "z last.scm", "(display \"z\")");
        let unicode_name = format!("nested/{} test.scm", '\u{65e5}');
        write_script(&directory, &unicode_name, "(display \"unicode\")");
        write_script(&directory, "nested/ignored.txt", "not Scheme");

        let scripts = discover_scripts(&directory).expect("discover scripts");
        let relative: Vec<PathBuf> = scripts
            .iter()
            .map(|script| script.relative.clone())
            .collect();
        assert_eq!(
            relative,
            [PathBuf::from(unicode_name), PathBuf::from("z last.scm")]
        );
    }

    #[test]
    fn report_prints_case_outcomes_and_only_failed_output() {
        let directory = temporary_directory("report");
        write_script(
            &directory,
            "pass.scm",
            "(import (scheme base) (neetan test 1))\n\
             (test-suite \"Passing Suite\"\n\
               (test-case \"passing case\"\n\
                 (note \"successful output must be discarded\")\n\
                 (check-true #t)))\n",
        );
        write_script(
            &directory,
            "fail.scm",
            "(import (scheme base) (neetan test 1))\n\
             (test-suite \"Failing Suite\"\n\
               (test-case \"failing case\"\n\
                 (note \"captured failing output\")\n\
                 (fail \"deliberate failure\")))\n",
        );

        let mut report = Vec::new();
        let exit_code = orchestrate_with_interval(
            &directory,
            CommonConfig::with_defaults(),
            2,
            &mut report,
            Duration::from_secs(60),
        )
        .expect("run orchestration");
        let report = String::from_utf8(report).expect("UTF-8 report");

        assert_eq!(exit_code, 1);
        assert!(report.contains("Passing Suite - passing case ...... SUCCESS"));
        assert!(report.contains("Failing Suite - failing case ...... FAILURE"));
        assert!(report.contains("Finished 2 of 2 test files"));
        assert!(report.contains("test result: FAILURE. 1 passed; 1 failed; 2 total"));
        assert!(report.contains("captured failing output"));
        assert!(!report.contains("successful output must be discarded"));
        let failure_section = report.find("failures:").expect("failure section");
        let captured_output = report
            .find("captured failing output")
            .expect("captured failure output");
        assert!(captured_output > failure_section);
        assert!(!directory.join("output.log").exists());
    }

    #[test]
    fn progress_uses_discovered_file_count() {
        let directory = temporary_directory("progress");
        write_script(
            &directory,
            "hang.scm",
            "(import (scheme base))\n(let loop () (loop))\n",
        );
        let mut config = CommonConfig::with_defaults();
        config.timeout_seconds = 1;
        let mut report = Vec::new();
        let exit_code = orchestrate_with_interval(
            &directory,
            config,
            1,
            &mut report,
            Duration::from_millis(100),
        )
        .expect("run orchestration");
        let report = String::from_utf8(report).expect("UTF-8 report");

        assert_eq!(exit_code, 1);
        assert!(report.contains("Finished 0 of 1 test files"));
        assert!(report.contains("Finished 1 of 1 test files"));
        assert!(report.contains("test result: FAILURE. 0 passed; 1 failed; 1 total"));
    }
}
