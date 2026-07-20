//! Media isolation, mount lifecycle, reset and restore semantics, and filesystem
//! confinement.
//!
//! Writable fixtures are mounted with a RAM backing, so the on-disk baseline
//! must stay byte-identical on every termination path. Byte-level retention of
//! guest writes in RAM is proven by the `device` crate unit tests; the HLE
//! machine here cannot be driven to write a disk within a bounded test, so these
//! tests cover the session orchestration: isolation, the mount lifecycle, the
//! retain-versus-discard distinction between hard reset and restore-startup, and
//! the path-escape rejections.

#[path = "common/harness.rs"]
mod harness;

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

use automation::{
    CommonConfig, MessageProtocol, RunTermination, execute_script, watchdog::CancelHandle,
};
use harness::assert_completed_ok;

/// Builds a minimal one-track, one-sector D88 floppy image.
fn minimal_d88(payload_byte: u8) -> Vec<u8> {
    const HEADER_SIZE: usize = 0x2B0;
    const SECTOR_HEADER_SIZE: usize = 16;
    let mut image = vec![0u8; HEADER_SIZE];
    image[0x1B] = 0x10; // 2DD
    let track_offset = HEADER_SIZE as u32;
    image[0x20..0x24].copy_from_slice(&track_offset.to_le_bytes());

    let mut sector = vec![0u8; SECTOR_HEADER_SIZE];
    sector[0] = 0; // C
    sector[1] = 0; // H
    sector[2] = 1; // R
    sector[3] = 1; // N (256 bytes)
    sector[4..6].copy_from_slice(&1u16.to_le_bytes());
    sector[0x0E..0x10].copy_from_slice(&256u16.to_le_bytes());
    sector.extend(std::iter::repeat_n(payload_byte, 256));
    image.extend_from_slice(&sector);

    let total = image.len() as u32;
    image[0x1C..0x20].copy_from_slice(&total.to_le_bytes());
    image
}

/// A temporary workspace holding a script and its fixtures under one root.
struct Workspace {
    dir: PathBuf,
}

impl Workspace {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = format!(
            "neetan-auto-media-{}-{}-{}",
            std::process::id(),
            tag,
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).expect("create workspace dir");
        Self { dir }
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        let path = self.dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture parent");
        }
        std::fs::write(path, bytes).expect("write fixture");
    }

    fn read(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.dir.join(name)).expect("read fixture")
    }

    fn run(&self, script_name: &str, timeout_seconds: u64) -> RunTermination {
        let mut config = CommonConfig::with_defaults();
        config.timeout_seconds = timeout_seconds;
        config.artifact_root = Some(self.dir.join("artifacts"));

        let (sender, receiver) = mpsc::channel();
        let cancel = CancelHandle::new();
        execute_script(
            self.dir.join(script_name),
            config,
            Vec::new(),
            sender,
            cancel,
        );

        let messages: Vec<MessageProtocol> = receiver.iter().collect();
        match messages.last() {
            Some(MessageProtocol::Finished(termination)) => termination.clone(),
            other => panic!("expected a Finished message, got {other:?}"),
        }
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

#[test]
fn writable_floppy_stays_byte_identical_across_outcomes() {
    let baseline = minimal_d88(0x24);
    let outcomes: &[(&str, &str)] = &[
        ("pass", "(execution-result 'OK)"),
        ("error", "(execution-result 'ERROR \"deliberate failure\")"),
        ("exit", "(exit 0)"),
        ("emergency-exit", "(emergency-exit 0)"),
        (
            "hard-reset",
            "(reset! machine 'hard) (run-frames! machine 2) (execution-result 'OK)",
        ),
        (
            "restore",
            "(restore-startup! machine) (execution-result 'OK)",
        ),
    ];
    for (tag, tail) in outcomes {
        let workspace = Workspace::new(&format!("identical-{tag}"));
        workspace.write("disk.d88", &baseline);
        let script = format!(
            "(import (scheme base) (scheme process-context) (neetan automation 1))\n\
             (call-with-machine '((model . pc9801vm) (media . ((floppy 0 \"disk.d88\"))))\n\
               (lambda (machine)\n\
                 (run-frames! machine 3)\n\
                 {tail}))\n"
        );
        workspace.write("script.scm", script.as_bytes());
        let termination = workspace.run("script.scm", 60);
        assert!(
            !matches!(termination, RunTermination::CompileError(_))
                && !matches!(termination, RunTermination::Internal(_)),
            "{tag}: unexpected termination {termination:?}"
        );
        assert_eq!(
            workspace.read("disk.d88"),
            baseline,
            "{tag}: baseline fixture must stay byte-identical"
        );
    }
}

#[test]
fn writable_floppy_stays_byte_identical_on_timeout() {
    let baseline = minimal_d88(0x37);
    let workspace = Workspace::new("timeout");
    workspace.write("disk.d88", &baseline);
    let script = "(import (scheme base) (neetan automation 1))\n\
         (call-with-machine '((model . pc9801vm) (media . ((floppy 0 \"disk.d88\"))))\n\
           (lambda (machine) (let loop () (loop))))\n";
    workspace.write("script.scm", script.as_bytes());
    let termination = workspace.run("script.scm", 1);
    assert!(
        matches!(termination, RunTermination::Timeout),
        "expected Timeout, got {termination:?}"
    );
    assert_eq!(
        workspace.read("disk.d88"),
        baseline,
        "baseline fixture must stay byte-identical after a timeout"
    );
}

#[test]
fn writable_floppy_stays_byte_identical_on_cancellation() {
    let baseline = minimal_d88(0x58);
    let workspace = Workspace::new("cancellation");
    workspace.write("disk.d88", &baseline);
    let script = "(import (scheme base) (neetan automation 1))\n\
         (call-with-machine '((model . pc9801vm) (media . ((floppy 0 \"disk.d88\"))))\n\
           (lambda (machine) (let loop () (loop))))\n";
    workspace.write("script.scm", script.as_bytes());

    let mut config = CommonConfig::with_defaults();
    config.timeout_seconds = 60;
    config.artifact_root = Some(workspace.dir.join("artifacts"));
    let (sender, receiver) = mpsc::channel();
    let cancel = CancelHandle::new();
    let requester = cancel.clone();
    let request_thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        requester.request_cancel();
    });
    execute_script(
        workspace.dir.join("script.scm"),
        config,
        Vec::new(),
        sender,
        cancel,
    );
    request_thread.join().expect("join cancellation requester");

    let messages: Vec<MessageProtocol> = receiver.iter().collect();
    let termination = match messages.last() {
        Some(MessageProtocol::Finished(termination)) => termination,
        other => panic!("expected a Finished message, got {other:?}"),
    };
    assert!(
        matches!(termination, RunTermination::Cancelled),
        "expected Cancelled, got {termination:?}"
    );
    assert_eq!(
        workspace.read("disk.d88"),
        baseline,
        "baseline fixture must stay byte-identical after cancellation"
    );
}

#[test]
fn media_info_reports_a_mounted_floppy() {
    let workspace = Workspace::new("info");
    workspace.write("disk.d88", &minimal_d88(0x00));
    let script = "(import (scheme base) (neetan automation 1) (neetan test 1))\n\
         (test-suite \"Media information\"\n\
         (test-case \"reports a mounted floppy\"\n\
         (with-machine (machine '((model . pc9801vm)))\n\
         (media-insert! machine 'floppy 0 \"disk.d88\")\n\
         (let ((info (media-info machine 'floppy 0)))\n\
           (if (not (pair? info)) (fail \"floppy 0 should be mounted\"))\n\
           (if (not (eq? 'floppy (cdr (assq 'type info)))) (fail \"type\"))\n\
           (if (not (= 0 (cdr (assq 'slot info)))) (fail \"slot\"))\n\
           (if (not (equal? \"disk.d88\" (cdr (assq 'source info)))) (fail \"source\"))\n\
           (if (cdr (assq 'write-protected info)) (fail \"write-protected\"))\n\
           (if (cdr (assq 'private info)) (fail \"private\"))\n\
           (set-cdr! (assq 'source info) \"corrupted.d88\")\n\
           (if (not (equal? \"disk.d88\"\n\
                           (alist-ref (media-info machine 'floppy 0) 'source)))\n\
               (fail \"media snapshot mutation escaped\")))\n\
         (if (media-info machine 'floppy 1) (fail \"floppy 1 should be empty\")))))\n";
    workspace.write("script.scm", script.as_bytes());
    assert_completed_ok(&workspace.run("script.scm", 60));
}

#[test]
fn runtime_eject_is_discarded_by_restore_startup() {
    let workspace = Workspace::new("restore-set");
    workspace.write("disk.d88", &minimal_d88(0x00));
    // Declare floppy 0 at startup, eject it at runtime, then restore-startup!
    // must bring the declared set back and forget the runtime eject.
    let script = "(import (scheme base) (neetan automation 1) (neetan test 1))\n\
         (test-suite \"Startup media restoration\"\n\
         (test-case \"restores the startup mount set\"\n\
         (with-machine (machine '((model . pc9801vm) (media . ((floppy 0 \"disk.d88\")))))\n\
         (if (not (pair? (media-info machine 'floppy 0))) (fail \"startup floppy missing\"))\n\
         (media-eject! machine 'floppy 0)\n\
         (if (media-info machine 'floppy 0) (fail \"eject should remove the mount\"))\n\
         (restore-startup! machine)\n\
         (if (not (pair? (media-info machine 'floppy 0))) (fail \"restore should replay startup media\")))))\n";
    workspace.write("script.scm", script.as_bytes());
    assert_completed_ok(&workspace.run("script.scm", 60));
}

#[test]
fn runtime_insert_is_kept_across_hard_reset() {
    let workspace = Workspace::new("hard-reset-set");
    workspace.write("disk.d88", &minimal_d88(0x00));
    // A hard reset retains the current mount set, unlike restore-startup!.
    let script = "(import (scheme base) (neetan automation 1) (neetan test 1))\n\
         (test-suite \"Hard reset media\"\n\
         (test-case \"retains runtime mounts\"\n\
         (with-machine (machine '((model . pc9801vm)))\n\
         (media-insert! machine 'floppy 0 \"disk.d88\")\n\
         (reset! machine 'hard)\n\
         (if (not (pair? (media-info machine 'floppy 0))) (fail \"hard reset should keep the mount\")))))\n";
    workspace.write("script.scm", script.as_bytes());
    assert_completed_ok(&workspace.run("script.scm", 60));
}

#[test]
fn parent_escape_source_is_rejected() {
    let workspace = Workspace::new("escape");
    // The fixture lives above the read root; a "../" source must be rejected.
    let script = "(import (scheme base) (neetan automation 1) (neetan test 1))\n\
         (test-suite \"Parent path isolation\"\n\
         (test-case \"rejects parent path escapes\"\n\
         (with-machine (machine '((model . pc9801vm)))\n\
         (guard (condition ((error-object? condition)\n\
                            (if (not (memq 'neetan/path-escape\n\
                                           (error-object-irritants condition)))\n\
                                (fail \"expected a path-escape error\"))))\n\
           (media-insert! machine 'floppy 0 \"../outside.d88\")\n\
           (fail \"expected a path-escape error\")))))\n";
    workspace.write("script.scm", script.as_bytes());
    assert_completed_ok(&workspace.run("script.scm", 60));
}

#[test]
fn symlink_escape_source_is_rejected() {
    let workspace = Workspace::new("symlink");
    // Place a real fixture outside the workspace and a symlink to it inside.
    let outside = workspace.dir.join("outside.d88");
    std::fs::write(&outside, minimal_d88(0x00)).expect("write outside fixture");
    let link = workspace.dir.join("scripts");
    std::fs::create_dir_all(&link).ok();
    let script_dir = link;
    // Symlink scripts/link.d88 -> ../outside.d88, then resolve it from the
    // script directory. The no-follow walk must reject the symlinked component.
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, script_dir.join("link.d88")).expect("create symlink");
    let script = "(import (scheme base) (neetan automation 1) (neetan test 1))\n\
         (test-suite \"Symlink isolation\"\n\
         (test-case \"rejects symlink escapes\"\n\
         (with-machine (machine '((model . pc9801vm)))\n\
         (guard (condition ((error-object? condition)\n\
                            (if (not (memq 'neetan/path-escape\n\
                                           (error-object-irritants condition)))\n\
                                (fail \"expected a path-escape error\"))))\n\
           (media-insert! machine 'floppy 0 \"link.d88\")\n\
           (fail \"expected a path-escape error\")))))\n";
    std::fs::write(script_dir.join("script.scm"), script).expect("write script");

    // Run with the script directory (scripts/) as the read root.
    let mut config = CommonConfig::with_defaults();
    config.timeout_seconds = 60;
    config.artifact_root = Some(workspace.dir.join("artifacts"));
    let (sender, receiver) = mpsc::channel();
    execute_script(
        script_dir.join("script.scm"),
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
    #[cfg(unix)]
    assert_completed_ok(&termination);
    #[cfg(not(unix))]
    let _ = termination;
}

#[test]
fn hard_disk_cannot_be_ejected() {
    let workspace = Workspace::new("hdd-eject");
    let script = "(import (scheme base) (neetan automation 1) (neetan test 1))\n\
         (test-suite \"Hard disk media\"\n\
         (test-case \"rejects hard disk ejection\"\n\
         (with-machine (machine '((model . pc9801vm)))\n\
         (guard (condition ((error-object? condition)\n\
                            (if (not (memq 'neetan/unsupported\n\
                                           (error-object-irritants condition)))\n\
                                (fail \"expected an unsupported error\"))))\n\
           (media-eject! machine 'hdd 0)\n\
           (fail \"expected an unsupported error\")))))\n";
    workspace.write("script.scm", script.as_bytes());
    assert_completed_ok(&workspace.run("script.scm", 60));
}
