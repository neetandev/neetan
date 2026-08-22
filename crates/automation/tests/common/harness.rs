//! Shared helpers for the automation crate integration tests.
//!
//! Covers the two ways the tests drive the crate: running a committed `.scm`
//! script through `execute_script` and inspecting its message stream, and
//! building an automated machine directly to exercise the session API. Machine
//! builders use ROM-free or synthetic-ROM families so no external fixtures are
//! required.

#![allow(dead_code)]

use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
};

use automation::{
    AutomationSession, CancelHandle, CommonConfig, ExecutionResult, MachineIdentity,
    MessageProtocol, RunTermination, execute_script,
};
use common::{
    AutomatedMachine, CpuMode, FixedHostDateTime, HostDateTime, MachineModel,
    tracing::{ApplicationTraceSink, TraceLimits},
};

/// The fixed guest real-time-clock value shared by every automated machine.
pub fn fixed_date_time() -> HostDateTime {
    HostDateTime {
        year: 2000,
        month: 1,
        day: 1,
        day_of_week: 6,
        hour: 0,
        minute: 0,
        second: 0,
    }
}

/// A fresh application trace sink for one machine construction.
pub fn trace_sink() -> ApplicationTraceSink {
    ApplicationTraceSink::new(TraceLimits::default()).0
}

/// Builds an automated PC-98 machine (HLE, ROM-free) with the fixed RTC.
#[cfg(feature = "pc98")]
pub fn build_pc98() -> Box<dyn AutomatedMachine> {
    let bus = machine_98::Pc9801Bus::new_with_trace_sink(
        MachineModel::PC9801VM,
        CpuMode::High,
        48000,
        trace_sink(),
    );
    let mut machine = machine_98::build_automated_machine(MachineModel::PC9801VM, bus);
    machine.set_host_date_time_source(Arc::new(FixedHostDateTime(fixed_date_time())));
    machine
}

/// Builds an automated MSX machine with the fixed RTC.
#[cfg(feature = "msx")]
pub fn build_msx() -> Box<dyn AutomatedMachine> {
    let bus =
        machine_msx::MsxBus::new_with_trace_sink(machine_msx::MsxModel::Msx, 48000, trace_sink());
    let mut machine = machine_msx::build_automated_machine(bus);
    machine.set_host_date_time_source(Arc::new(FixedHostDateTime(fixed_date_time())));
    machine
}

/// Builds an automated X68000 machine with synthetic ROMs and the fixed RTC.
#[cfg(feature = "x68k")]
pub fn build_x68k() -> Box<dyn AutomatedMachine> {
    let mut ipl = vec![0u8; 0x20000];
    // A synthetic reset vector so the real CPU bus is exercised on construction.
    ipl[0x10000..0x10008].copy_from_slice(&[0, 0xBF, 0xF0, 0, 0, 0xFF, 0, 8]);
    let roms = machine_x68k::LoadedRoms {
        model: machine_x68k::X68kModel::X68000,
        cgrom: vec![0; 0xC0000],
        ipl,
        internal_scsi: None,
        uses_compatibility_scsi: false,
    };
    let bus = machine_x68k::X68kBus::new_with_trace_sink(
        machine_x68k::X68kModel::X68000,
        CpuMode::High,
        roms,
        48000,
        trace_sink(),
    )
    .expect("build x68k bus");
    let mut machine =
        machine_x68k::build_automated_machine(machine_x68k::X68kModel::X68000, CpuMode::High, bus);
    machine.set_host_date_time_source(Arc::new(FixedHostDateTime(fixed_date_time())));
    machine
}

/// Builds an automated FM-7 machine with synthetic ROMs and the fixed RTC.
#[cfg(feature = "fm7")]
pub fn build_fm7() -> Box<dyn AutomatedMachine> {
    let model = machine_fm7::Fm7Model::Fm7;
    let mut bus = machine_fm7::Fm7Bus::new_with_trace_sink(
        model,
        machine_fm7::BootMode::Basic,
        48000,
        trace_sink(),
    );
    let roms = machine_fm7::LoadedRoms {
        model,
        fbasic: vec![0; 0x7C00],
        subsys_c: vec![0; 0x2800],
        kanji: None,
        boot_bas: Some(vec![0; 0x0200]),
        boot_dos: Some(vec![0; 0x0200]),
        initiate: None,
        subsys_a: None,
        subsys_b: None,
        subsyscg: None,
    };
    bus.load_roms(&roms);
    let mut machine = machine_fm7::build_automated_machine(model, bus);
    machine.set_host_date_time_source(Arc::new(FixedHostDateTime(fixed_date_time())));
    machine
}

/// The absolute path to a committed test script under `tests/scripts`.
pub fn script_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("scripts")
        .join(name)
}

/// The outcome of running one script through `execute_script`.
pub struct Run {
    /// Every message emitted, in order.
    pub messages: Vec<MessageProtocol>,
    /// The terminal `Finished` termination.
    pub termination: RunTermination,
    /// The process exit code mapped from the termination.
    pub exit_code: i32,
    /// The artifact root the script wrote under.
    pub artifact_root: PathBuf,
}

/// Runs a committed script through the executor with a private artifact root and
/// collects its message stream.
pub fn run_committed_script(name: &str, timeout_seconds: u64) -> Run {
    // Keep artifacts out of the source tree, and give every invocation its own
    // clean root so concurrently running tests of the same script never clear
    // or read each other's artifacts.
    static INVOCATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let invocation = INVOCATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let artifact_root = std::env::temp_dir().join("neetan-auto-tests").join(format!(
        "{}-{}-{invocation}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&artifact_root);
    let mut config = CommonConfig::with_defaults();
    config.timeout_seconds = timeout_seconds;
    config.artifact_root = Some(artifact_root.clone());

    let (sender, receiver) = mpsc::channel();
    execute_script(
        script_path(name),
        config,
        Vec::new(),
        sender,
        CancelHandle::new(),
    );

    let messages: Vec<MessageProtocol> = receiver.iter().collect();
    let termination = match messages.last() {
        Some(MessageProtocol::Finished(termination)) => termination.clone(),
        other => panic!("expected a Finished message, got {other:?}"),
    };
    let exit_code = termination.exit_code();
    Run {
        messages,
        termination,
        exit_code,
        artifact_root,
    }
}

/// Returns the machine identity from the first `MachineReady` message, if any.
pub fn machine_ready(run: &Run) -> Option<&MachineIdentity> {
    run.messages.iter().find_map(|message| match message {
        MessageProtocol::MachineReady { identity } => Some(identity),
        _ => None,
    })
}

/// Concatenates every captured `Output` chunk.
pub fn output_text(run: &Run) -> String {
    run.messages
        .iter()
        .filter_map(|message| match message {
            MessageProtocol::Output(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// Asserts the stream begins with `Started` and ends with `Finished`.
pub fn assert_started_then_finished(run: &Run) {
    assert!(
        matches!(run.messages.first(), Some(MessageProtocol::Started { .. })),
        "first message must be Started"
    );
    assert!(
        matches!(run.messages.last(), Some(MessageProtocol::Finished(_))),
        "last message must be Finished"
    );
}

/// Asserts a termination is `Completed(Ok)` with exit code zero.
pub fn assert_completed_ok(termination: &RunTermination) {
    assert!(
        matches!(termination, RunTermination::Completed(ExecutionResult::Ok)),
        "expected Completed(Ok), got {termination:?}"
    );
    assert_eq!(termination.exit_code(), 0);
}

/// A fresh, empty temporary directory for one test.
pub fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("neetan-auto-tests").join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Creates a session rooted at the given read and artifact directories, returning
/// the session and its live message receiver. Callers that ignore the stream can
/// drop the receiver, which turns later sends into no-ops.
pub fn make_session(
    read_root: &Path,
    artifact_root: &Path,
) -> (AutomationSession, mpsc::Receiver<MessageProtocol>) {
    let (sender, receiver) = mpsc::channel();
    let common = CommonConfig::with_defaults();
    let factory_rtc = common.host_date_time_source();
    let sample_rate = common.audio_sample_rate();
    let session = AutomationSession::new(
        sender,
        CancelHandle::new(),
        common,
        factory_rtc,
        sample_rate,
        read_root.to_path_buf(),
        artifact_root.to_path_buf(),
    );
    (session, receiver)
}
