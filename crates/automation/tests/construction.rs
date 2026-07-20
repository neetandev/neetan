//! Scoped machine construction and explicit handle validation.

#[path = "common/harness.rs"]
mod harness;

use automation::RunTermination;
use common::AutomatedMachine;
use harness::{
    assert_completed_ok, build_fm7, build_msx, build_pc98, build_x68k, machine_ready, make_session,
    run_committed_script, temp_dir,
};

#[test]
fn pc98_build_script_constructs_and_runs() {
    let run = run_committed_script("pc98-build.scm", 60);
    let identity = machine_ready(&run).expect("a MachineReady message must be emitted");
    assert_eq!(identity.target, "pc98");
    assert_eq!(identity.model, "pc9801vm");
    assert_completed_ok(&run.termination);
}

#[test]
fn machine_operation_requires_a_handle() {
    let run = run_committed_script("no-machine.scm", 60);
    match &run.termination {
        RunTermination::RuntimeError(diagnostic) => {
            assert!(
                diagnostic.message().contains("expected a machine"),
                "diagnostic should mention the missing handle, got {:?}",
                diagnostic.message()
            );
        }
        other => panic!("expected RuntimeError, got {other:?}"),
    }
    assert_eq!(run.exit_code, 3);
    assert!(
        machine_ready(&run).is_none(),
        "no machine should be constructed"
    );
}

#[test]
fn opaque_handles_are_scoped_and_non_forgeable() {
    let run = run_committed_script("handles.scm", 60);
    assert_completed_ok(&run.termination);
    let ready_count = run
        .messages
        .iter()
        .filter(|message| matches!(message, automation::MessageProtocol::MachineReady { .. }))
        .count();
    assert_eq!(
        ready_count, 3,
        "each successful sequential scope must emit MachineReady"
    );
}

#[test]
fn every_machine_operation_validates_resource_handle() {
    let run = run_committed_script("handle-conformance.scm", 60);
    assert_completed_ok(&run.termination);
    let ready_count = run
        .messages
        .iter()
        .filter(|message| matches!(message, automation::MessageProtocol::MachineReady { .. }))
        .count();
    assert_eq!(ready_count, 2);
}

#[test]
fn machine_scope_unwinds_and_construction_is_transactional() {
    let run = run_committed_script("scope-unwind.scm", 60);
    assert_completed_ok(&run.termination);
    let ready_count = run
        .messages
        .iter()
        .filter(|message| matches!(message, automation::MessageProtocol::MachineReady { .. }))
        .count();
    assert_eq!(ready_count, 5);
}

#[test]
fn logical_machine_lifecycle_is_backend_independent() {
    let root = temp_dir("cross-backend-lifecycle");
    let (mut session, _receiver) = make_session(&root, &root);
    let builders = [
        ("pc98", build_pc98 as fn() -> Box<dyn AutomatedMachine>),
        ("msx", build_msx),
        ("fm7", build_fm7),
        ("x68k", build_x68k),
    ];

    for (target, build) in builders {
        session.install_machine(build());
        let descriptor = session.descriptor().expect("installed descriptor");
        assert_eq!(descriptor.target, target);
        assert_eq!(session.timeline().epoch, 0);
        session.close_active_machine();
        assert!(session.descriptor().is_none());
        assert_eq!(session.timeline(), common::AutomationTimeline::default());
    }
}
