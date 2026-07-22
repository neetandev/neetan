//! Generic tracing end to end.
//!
//! Drives the `pc98-trace.scm` fixture through `execute_script`, exercising the
//! full stack: the `(neetan trace 1)` natives, the declarative-filter compiler,
//! continuous collection and drain, exclusive `wait-for-event`, the
//! `neetan/trace-state` guard, up-front filter validation, and bounded-queue
//! overflow reported as `neetan/trace-overflow`. Running the fixture twice under
//! the fixed guest clock also checks that tracing stays deterministic.

#[path = "common/harness.rs"]
mod harness;

use automation::{ExecutionResult, RunTermination};
use harness::run_committed_script;

/// Parses every line of a Scheme trace artifact and returns the datum count.
///
/// Proves the artifact reads back with `read`, covering every serialized value
/// type the events carry.
fn count_artifact_datums(bytes: &[u8]) -> usize {
    let text = String::from_utf8(bytes.to_vec()).expect("trace artifact is UTF-8");
    let mut engine = r7rs::Engine::new(r7rs::EngineConfig::default()).expect("engine");
    let mut reader = engine
        .reader_from_str("trace-artifact", text)
        .expect("reader");
    let mut count = 0;
    while reader.read_next().expect("artifact datum parses").is_some() {
        count += 1;
    }
    count
}

/// Proves the artifact round-trips at value level, not just at parse level.
///
/// Every line is read back with the r7rs reader and re-rendered; the external
/// form must reproduce the written line exactly, so symbols, exact integers,
/// bytevectors, `#f` falseables, and nested alists all survive a read.
fn assert_artifact_round_trips(bytes: &[u8]) {
    let text = String::from_utf8(bytes.to_vec()).expect("trace artifact is UTF-8");
    let mut engine = r7rs::Engine::new(r7rs::EngineConfig::default()).expect("engine");
    for line in text.lines() {
        let mut reader = engine
            .reader_from_str("trace-artifact-line", line.to_owned())
            .expect("reader");
        let datum = reader
            .read_next()
            .expect("artifact datum parses")
            .expect("artifact line holds a datum");
        assert_eq!(
            datum.to_external(),
            line,
            "reading an artifact datum back must preserve every value"
        );
    }
}

#[test]
fn pc98_trace_script_passes() {
    let run = run_committed_script("pc98-trace.scm", 120);
    assert!(
        matches!(
            run.termination,
            RunTermination::Completed(ExecutionResult::Ok)
        ),
        "trace script did not pass: {:?}",
        run.termination
    );
}

#[test]
fn pc98_trace_script_is_deterministic() {
    let first = run_committed_script("pc98-trace.scm", 120);
    let second = run_committed_script("pc98-trace.scm", 120);
    assert!(matches!(
        first.termination,
        RunTermination::Completed(ExecutionResult::Ok)
    ));
    assert!(matches!(
        second.termination,
        RunTermination::Completed(ExecutionResult::Ok)
    ));
    assert_eq!(first.exit_code, second.exit_code);
}

/// Drives the `pc98-console-diagnose.scm` fixture, proving the semantic HLE DOS
/// console events (SGR reverse video, the one-cell two-byte graphic, INT 29h
/// IRET-hook suppression, clear and scroll regions) from structured trace
/// values, an entry snapshot, a decoded text cell, and bounded artifacts, with
/// no emulator source changed for diagnostics.
#[test]
fn pc98_console_diagnose_script_passes() {
    let run = run_committed_script("pc98-console-diagnose.scm", 120);
    assert!(
        matches!(
            run.termination,
            RunTermination::Completed(ExecutionResult::Ok)
        ),
        "console diagnose script did not pass: {:?}",
        run.termination
    );
    // The memory artifact holds the exact guest bytes: the saved byte is the
    // IRET opcode the script installed as the INT 29h handler.
    let iret = std::fs::read(run.artifact_root.join("diagnose-iret.bin"))
        .expect("memory artifact written");
    assert_eq!(iret, [0xCF]);
    // The trace artifact reads back with `read`, one datum per event, and the
    // read-back values reproduce the written external form exactly.
    let trace = std::fs::read(run.artifact_root.join("diagnose-trace.scm"))
        .expect("trace artifact written");
    assert!(count_artifact_datums(&trace) > 0);
    assert_artifact_round_trips(&trace);
    // Spot-check serialized value types against the known fixture events.
    let text = String::from_utf8(trace).expect("trace artifact is UTF-8");
    assert!(
        text.contains("(suppression-reason . int29-iret-hook)"),
        "the suppressed write's symbol field must serialize"
    );
    assert!(
        text.contains("#u8(68 73 65 71)"),
        "the DIAG write's byte field must serialize"
    );
    assert!(
        text.contains("(parameters 7)"),
        "the SGR escape's integer-list field must serialize"
    );
}

/// Drives the `pc98-capture.scm` fixture twice, proving the triggered bounded
/// ring capture end to end: up-front validation, path confinement, the before
/// and after windows, bounded storage without a trigger, and a byte-identical
/// artifact across identical runs.
#[test]
fn pc98_capture_script_passes_and_is_deterministic() {
    let first = run_committed_script("pc98-capture.scm", 120);
    assert!(
        matches!(
            first.termination,
            RunTermination::Completed(ExecutionResult::Ok)
        ),
        "capture script did not pass: {:?}",
        first.termination
    );
    let artifact = first.artifact_root.join("capture.scm");
    let first_bytes = std::fs::read(&artifact).expect("capture artifact written");
    assert_eq!(
        count_artifact_datums(&first_bytes),
        8,
        "artifact must hold one datum per retained event"
    );
    assert!(
        !first.artifact_root.join("untriggered.scm").exists(),
        "an unfired trigger must not write an artifact"
    );
    assert!(
        !first.artifact_root.join("escape.scm").exists()
            && !first
                .artifact_root
                .parent()
                .expect("artifact root parent")
                .join("escape.scm")
                .exists(),
        "a rejected path must not be written"
    );

    let second = run_committed_script("pc98-capture.scm", 120);
    assert!(matches!(
        second.termination,
        RunTermination::Completed(ExecutionResult::Ok)
    ));
    let second_bytes =
        std::fs::read(second.artifact_root.join("capture.scm")).expect("capture artifact written");
    assert_eq!(
        first_bytes, second_bytes,
        "identical runs must produce identical capture artifacts"
    );
}

#[test]
fn pc98_console_diagnose_script_is_deterministic() {
    let first = run_committed_script("pc98-console-diagnose.scm", 120);
    let second = run_committed_script("pc98-console-diagnose.scm", 120);
    assert!(matches!(
        first.termination,
        RunTermination::Completed(ExecutionResult::Ok)
    ));
    assert!(matches!(
        second.termination,
        RunTermination::Completed(ExecutionResult::Ok)
    ));
    assert_eq!(first.exit_code, second.exit_code);
}
