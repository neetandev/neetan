//! End-to-end tests driving `execute_script` and checking the message stream,
//! terminal `RunTermination`, and mapped exit code for representative scripts.

#[path = "common/harness.rs"]
mod harness;

use automation::{ExecutionResult, MessageProtocol, RunTermination, TestCaseOutcome};
use harness::{assert_started_then_finished, output_text, run_committed_script};

#[test]
fn passing_script_reports_ok() {
    let run = run_committed_script("pass.scm", 30);
    assert_started_then_finished(&run);
    assert!(output_text(&run).contains("pass script running"));
    assert!(
        run.messages
            .iter()
            .any(|message| matches!(message, MessageProtocol::Result(ExecutionResult::Ok))),
        "a Result(Ok) message must be emitted"
    );
    assert!(matches!(
        run.termination,
        RunTermination::Completed(ExecutionResult::Ok)
    ));
    assert_eq!(run.exit_code, 0);
}

#[test]
fn failing_script_reports_error() {
    let run = run_committed_script("fail.scm", 30);
    assert_started_then_finished(&run);
    assert!(output_text(&run).contains("fail script running"));
    match &run.termination {
        RunTermination::Completed(ExecutionResult::Error { message }) => {
            assert_eq!(message, "boom");
        }
        other => panic!("expected Completed(Error), got {other:?}"),
    }
    assert_eq!(run.exit_code, 1);
}

#[test]
fn script_without_result_reports_no_result() {
    let run = run_committed_script("noresult.scm", 30);
    assert_started_then_finished(&run);
    assert!(
        !run.messages
            .iter()
            .any(|message| matches!(message, MessageProtocol::Result(_))),
        "no Result message must be emitted"
    );
    assert!(matches!(run.termination, RunTermination::NoResult));
    assert_eq!(run.exit_code, 4);
}

#[test]
fn hung_script_times_out() {
    let run = run_committed_script("hang.scm", 1);
    assert_started_then_finished(&run);
    assert!(matches!(run.termination, RunTermination::Timeout));
    assert_eq!(run.exit_code, 124);
}

#[test]
fn test_library_sets_one_result() {
    let run = run_committed_script("test-lib.scm", 30);
    assert!(matches!(
        run.termination,
        RunTermination::Completed(ExecutionResult::Ok)
    ));
    assert_eq!(run.exit_code, 0);
    assert_eq!(
        run.messages
            .iter()
            .filter(|message| matches!(message, MessageProtocol::Result(_)))
            .count(),
        1
    );
    assert!(run.messages.iter().any(|message| matches!(
        message,
        MessageProtocol::TestCaseFinished {
            suite,
            test_case,
            outcome: TestCaseOutcome::Success,
        } if suite == "test library" && test_case == "passes a successful case"
    )));
}

#[test]
fn failed_case_is_named_and_later_cases_continue() {
    let run = run_committed_script("test-suite-fail.scm", 30);
    let output = output_text(&run);
    assert!(output.contains("later case ran"));
    assert!(output.contains("(kind . assertion)"));
    assert!(output.contains("(test-case . \"assertion case\")"));
    assert_eq!(
        run.messages
            .iter()
            .filter(|message| matches!(message, MessageProtocol::Result(_)))
            .count(),
        1
    );
    match &run.termination {
        RunTermination::Completed(ExecutionResult::Error { message }) => {
            assert!(message.contains("failure suite: 1 of 2 test case(s) failed"));
            assert!(message.contains("assertion case: check-true failed"));
        }
        other => panic!("expected Completed(Error), got {other:?}"),
    }
}

#[test]
fn failed_checks_report_their_source_forms() {
    let run = run_committed_script("test-suite-check-details.scm", 30);
    let expected_messages = [
        (
            "check-true detail",
            "check-true failed: value was false; check: (check-true (begin (set! evaluations (+ evaluations 1)) #f))",
        ),
        (
            "check-false detail",
            "check-false failed: value was true; check: (check-false (> 2 1))",
        ),
        (
            "check-equal detail",
            "check-equal failed: values are not equal; check: (check-equal (+ 1 1) 3)",
        ),
        (
            "check-near detail",
            "check-near failed: |1.0 - 2.0| > 0.1; check: (check-near 1.0 2.0 0.1)",
        ),
    ];

    for (test_case, expected_message) in expected_messages {
        assert!(run.messages.iter().any(|message| matches!(
            message,
            MessageProtocol::TestCaseFinished {
                test_case: actual_test_case,
                outcome: TestCaseOutcome::Failure { message, .. },
                ..
            } if actual_test_case == test_case && message == expected_message
        )));
    }

    match &run.termination {
        RunTermination::Completed(ExecutionResult::Error { message }) => {
            assert!(message.contains("check detail suite: 4 of 5 test case(s) failed"));
            for (test_case, expected_message) in expected_messages {
                assert!(message.contains(&format!("{test_case}: {expected_message}")));
            }
        }
        other => panic!("expected Completed(Error), got {other:?}"),
    }
}

#[test]
fn catchable_case_error_is_recorded_and_later_cases_continue() {
    let run = run_committed_script("test-suite-error.scm", 30);
    let output = output_text(&run);
    assert!(output.contains("later error case ran"));
    assert!(output.contains("(kind . error)"));
    assert!(output.contains("(test-case . \"error case\")"));
    assert_eq!(
        run.messages
            .iter()
            .filter(|message| matches!(message, MessageProtocol::Result(_)))
            .count(),
        1
    );
    match &run.termination {
        RunTermination::Completed(ExecutionResult::Error { message }) => {
            assert!(message.contains("error suite: 1 of 2 test case(s) failed"));
            assert!(message.contains("error case: unexpected case error"));
        }
        other => panic!("expected Completed(Error), got {other:?}"),
    }
}

#[test]
fn test_suite_returns_a_structured_summary() {
    let run = run_committed_script("test-suite-summary.scm", 30);
    assert!(matches!(
        run.termination,
        RunTermination::Completed(ExecutionResult::Ok)
    ));
    let output = output_text(&run);
    assert!(output.contains("(suite . \"summary suite\")"));
    assert!(output.contains("(passed . #t)"));
    assert!(output.contains("(test-count . 1)"));
    assert!(output.contains("(passed-count . 1)"));
    assert!(output.contains("(failure-count . 0)"));
    assert!(output.contains("(failures)"));
    assert!(output.contains("(summary . \"summary suite: 1 test case(s) passed\")"));
}

#[test]
fn suite_details_are_ordered_and_fail_aborts_its_case() {
    let run = run_committed_script("test-suite-details.scm", 30);
    let output = output_text(&run);
    assert!(!output.contains("unreachable assertion tail"));
    assert!(output.contains("(passed . #f)"));
    assert!(output.contains("(test-count . 6)"));
    assert!(output.contains("(passed-count . 3)"));
    assert!(output.contains("(failure-count . 3)"));
    assert!(output.contains("(kind . assertion)"));
    assert!(output.contains("(kind . error)"));
    assert!(output.contains("test case raised a non-error condition"));
    match &run.termination {
        RunTermination::Completed(ExecutionResult::Error { message }) => assert_eq!(
            message,
            "detailed suite: 3 of 6 test case(s) failed\n\
             assertion case: deliberate assertion\n\
             error case: deliberate error\n\
             raised value case: test case raised a non-error condition"
        ),
        other => panic!("expected Completed(Error), got {other:?}"),
    }
}

#[test]
fn empty_suite_reports_test_state_without_a_result() {
    let run = run_committed_script("test-suite-empty.scm", 30);
    assert!(
        !run.messages
            .iter()
            .any(|message| matches!(message, MessageProtocol::Result(_)))
    );
    match &run.termination {
        RunTermination::RuntimeError(diagnostic) => {
            assert!(diagnostic.message().contains("at least one test-case"));
        }
        other => panic!("expected RuntimeError, got {other:?}"),
    }
}

#[test]
fn check_outside_case_reports_test_state_without_a_result() {
    let run = run_committed_script("test-suite-unscoped-check.scm", 30);
    assert!(
        !run.messages
            .iter()
            .any(|message| matches!(message, MessageProtocol::Result(_)))
    );
    match &run.termination {
        RunTermination::RuntimeError(diagnostic) => {
            assert!(
                diagnostic
                    .message()
                    .contains("requires an active test-case")
            );
        }
        other => panic!("expected RuntimeError, got {other:?}"),
    }
}

#[test]
fn fail_checks_case_scope_before_its_argument() {
    let run = run_committed_script("test-fail-unscoped-invalid.scm", 30);
    assert!(
        !run.messages
            .iter()
            .any(|message| matches!(message, MessageProtocol::Result(_)))
    );
    match &run.termination {
        RunTermination::RuntimeError(diagnostic) => {
            assert!(
                diagnostic
                    .message()
                    .contains("requires an active test-case")
            );
            assert!(diagnostic.message().contains("neetan/test-state"));
        }
        other => panic!("expected RuntimeError, got {other:?}"),
    }
}

#[test]
fn case_outside_suite_reports_test_state_without_a_result() {
    let run = run_committed_script("test-case-outside-suite.scm", 30);
    assert!(
        !run.messages
            .iter()
            .any(|message| matches!(message, MessageProtocol::Result(_)))
    );
    match &run.termination {
        RunTermination::RuntimeError(diagnostic) => {
            assert!(
                diagnostic
                    .message()
                    .contains("requires an active test-suite")
            );
        }
        other => panic!("expected RuntimeError, got {other:?}"),
    }
}

#[test]
fn nested_suite_reports_test_state_without_a_result() {
    let run = run_committed_script("test-suite-nested.scm", 30);
    assert!(
        !run.messages
            .iter()
            .any(|message| matches!(message, MessageProtocol::Result(_)))
    );
    match &run.termination {
        RunTermination::RuntimeError(diagnostic) => {
            assert!(
                diagnostic
                    .message()
                    .contains("test-suite may not be nested")
            );
        }
        other => panic!("expected RuntimeError, got {other:?}"),
    }
}

#[test]
fn second_root_suite_is_rejected_before_its_body() {
    let run = run_committed_script("test-suite-multiple.scm", 30);
    assert!(!output_text(&run).contains("second root case ran"));
    assert_eq!(
        run.messages
            .iter()
            .filter(|message| matches!(message, MessageProtocol::Result(_)))
            .count(),
        1
    );
    match &run.termination {
        RunTermination::RuntimeError(diagnostic) => {
            assert!(diagnostic.message().contains("root may only appear once"));
            assert!(diagnostic.message().contains("neetan/test-state"));
        }
        other => panic!("expected RuntimeError, got {other:?}"),
    }
}

#[test]
fn suite_and_case_names_must_be_strings() {
    for script in ["test-suite-name-invalid.scm", "test-case-name-invalid.scm"] {
        let run = run_committed_script(script, 30);
        assert!(
            !run.messages
                .iter()
                .any(|message| matches!(message, MessageProtocol::Result(_)))
        );
        match &run.termination {
            RunTermination::RuntimeError(diagnostic) => {
                assert!(diagnostic.message().contains("name must be a string"));
                assert!(diagnostic.message().contains("neetan/argument"));
            }
            other => panic!("expected RuntimeError, got {other:?}"),
        }
    }
}

#[test]
fn nested_case_is_recorded_as_a_case_error() {
    let run = run_committed_script("test-case-nested.scm", 30);
    match &run.termination {
        RunTermination::Completed(ExecutionResult::Error { message }) => {
            assert!(message.contains("outer case: test-case may not be nested"));
        }
        other => panic!("expected Completed(Error), got {other:?}"),
    }
}

#[test]
fn compile_error_reports_location() {
    let run = run_committed_script("compile-error.scm", 30);
    match &run.termination {
        RunTermination::CompileError(diagnostic) => {
            assert!(!diagnostic.message().is_empty());
        }
        other => panic!("expected CompileError, got {other:?}"),
    }
    assert_eq!(run.exit_code, 3);
}

#[test]
fn double_result_raises_result_state() {
    let run = run_committed_script("double-result.scm", 30);
    // The first result is still emitted before the second call raises.
    assert!(
        run.messages
            .iter()
            .any(|message| matches!(message, MessageProtocol::Result(ExecutionResult::Ok))),
        "the first result must be emitted"
    );
    match &run.termination {
        RunTermination::RuntimeError(diagnostic) => {
            assert!(diagnostic.message().contains("already set"));
        }
        other => panic!("expected RuntimeError, got {other:?}"),
    }
    assert_eq!(run.exit_code, 3);
}
