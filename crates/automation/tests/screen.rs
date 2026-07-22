//! Display reads, hashing, screenshots, RMSE matching, region matching, and the
//! side-by-side comparison image.
//!
//! Machines are built with ROM-free families (PC-98 HLE and MSX) so no external
//! fixtures are required. Expected PNGs are generated into a temporary read root
//! at test time, so the tests do not depend on committed baselines.

#[path = "common/harness.rs"]
mod harness;

use std::path::Path;

use automation::{
    AutomationSession, ExecutionResult, MessageProtocol, RunTermination, SessionBudgets, screen,
};
use common::RunTarget;
use harness::{build_msx, build_pc98, make_session, run_committed_script, temp_dir};
use sdl3::surface::Surface;

/// Creates a session rooted at the given read and artifact directories, with the
/// message stream discarded.
fn rooted_session(read_root: &Path, artifact_root: &Path) -> AutomationSession {
    let (session, receiver) = make_session(read_root, artifact_root);
    drop(receiver);
    session
}

/// Writes RGBA8 pixels as a PNG under `dir`.
fn write_png(dir: &Path, name: &str, width: u32, height: u32, rgba: &[u8]) {
    let surface = Surface::from_rgba8(width, height, rgba).expect("build surface");
    let bytes = surface.save_png().expect("encode png");
    std::fs::write(dir.join(name), bytes).expect("write png");
}

#[test]
fn pc98_screen_pipeline_end_to_end() {
    let read_root = temp_dir("pc98-read");
    let artifact_root = temp_dir("pc98-artifacts");
    let mut session = rooted_session(&read_root, &artifact_root);

    session.install_machine(build_pc98());
    // The screen is unavailable until the first presentation.
    assert!(!session.screen_available());
    assert!(session.screen_rgba().is_err());

    session
        .run(RunTarget::Frames(30), 50_000_000, 1_000_000)
        .expect("frame run");
    assert!(session.screen_available());

    let (width, height) = session.screen_size().expect("size");
    assert!(width > 0 && height > 0);
    let rgba = session.screen_rgba().expect("rgba");
    assert_eq!(rgba.len(), (width * height * 4) as usize);

    // A pixel read agrees with the raw buffer.
    let (red, green, blue, alpha) = session.screen_pixel(0, 0).expect("pixel");
    assert_eq!([red, green, blue, alpha], rgba[0..4]);

    // The hash is 64 lowercase hex characters and deterministic across sessions.
    let hash = session.screen_hash().expect("hash");
    assert_eq!(hash.len(), 64);
    let mut other = rooted_session(&read_root, &artifact_root);
    other.install_machine(build_pc98());
    other
        .run(RunTarget::Frames(30), 50_000_000, 1_000_000)
        .expect("frame run");
    assert_eq!(hash, other.screen_hash().expect("hash"));

    // A saved screenshot decodes back to the same pixels.
    let written = session
        .save_screenshot("shot.png")
        .expect("save screenshot");
    let decoded =
        Surface::load_png(&std::fs::read(&written).expect("read shot")).expect("decode shot");
    assert_eq!(decoded.dimensions(), (width, height));
    assert_eq!(decoded.to_rgba8().expect("decode pixels"), rgba);

    // Matching against the exact screenshot is a perfect match.
    write_png(&read_root, "expected.png", width, height, &rgba);
    assert!(session.screen_matches("expected.png", 0.0).expect("match"));

    // A clearly different image fails at zero tolerance and passes at full.
    let contrasting: Vec<u8> = rgba.iter().map(|byte| byte ^ 0xFF).collect();
    write_png(&read_root, "other.png", width, height, &contrasting);
    assert!(!session.screen_matches("other.png", 0.0).expect("mismatch"));
    assert!(
        session
            .screen_matches("other.png", 1.0)
            .expect("full tolerance")
    );

    // Dimension mismatch is a hard argument error, not a false result.
    write_png(&read_root, "small.png", 8, 8, &vec![0u8; 8 * 8 * 4]);
    assert!(session.screen_matches("small.png", 0.5).is_err());

    // A region matches its own extracted pixels.
    let region_width = width / 2;
    let region_height = height / 2;
    let region = screen::extract_region(&rgba, width, height, 0, 0, region_width, region_height)
        .expect("region");
    write_png(
        &read_root,
        "region.png",
        region_width,
        region_height,
        &region,
    );
    assert!(
        session
            .screen_region_matches("region.png", 0, 0, region_width, region_height, 0.0)
            .expect("region match")
    );

    // The comparison image is twice as wide as the screen.
    let comparison = session
        .screen_comparison_image("other.png", "compare.png")
        .expect("comparison image");
    let combined = Surface::load_png(&std::fs::read(&comparison).expect("read comparison"))
        .expect("decode comparison");
    assert_eq!(combined.dimensions(), (width * 2, height));
}

#[test]
fn msx_screen_is_available_and_deterministic() {
    let read_root = temp_dir("msx-read");
    let artifact_root = temp_dir("msx-artifacts");
    let mut session = rooted_session(&read_root, &artifact_root);

    session.install_machine(build_msx());
    session
        .run(RunTarget::Frames(20), 50_000_000, 1_000_000)
        .expect("frame run");
    assert!(session.screen_available());

    let (width, height) = session.screen_size().expect("size");
    let rgba = session.screen_rgba().expect("rgba");
    assert_eq!(rgba.len(), (width * height * 4) as usize);

    // A round-trip screenshot matches itself exactly.
    write_png(&read_root, "msx.png", width, height, &rgba);
    assert!(session.screen_matches("msx.png", 0.0).expect("match"));

    // The hash repeats across an identical MSX session.
    let hash = session.screen_hash().expect("hash");
    let mut other = rooted_session(&read_root, &artifact_root);
    other.install_machine(build_msx());
    other
        .run(RunTarget::Frames(20), 50_000_000, 1_000_000)
        .expect("frame run");
    assert_eq!(hash, other.screen_hash().expect("hash"));
}

#[test]
fn wait_for_screen_matches_without_advancing_an_identical_screen() {
    let read_root = temp_dir("wait-immediate-read");
    let artifact_root = temp_dir("wait-immediate-artifacts");
    let mut session = rooted_session(&read_root, &artifact_root);
    session.install_machine(build_pc98());
    session
        .run(RunTarget::Frames(30), 50_000_000, 1_000_000)
        .expect("frame run");
    let (width, height) = session.screen_size().expect("size");
    let rgba = session.screen_rgba().expect("rgba");
    write_png(&read_root, "current.png", width, height, &rgba);
    let before = session.timeline();

    assert!(
        session
            .wait_for_screen("current.png", 0.0, 0, 0)
            .expect("wait")
    );
    assert_eq!(session.timeline(), before);
    assert!(!artifact_root.join("current-compare.png").exists());
}

#[test]
fn wait_for_screen_timeout_writes_and_reports_comparison() {
    let read_root = temp_dir("wait-timeout-read");
    let artifact_root = temp_dir("wait-timeout-artifacts");
    let (mut session, receiver) = make_session(&read_root, &artifact_root);
    session.install_machine(build_pc98());
    session
        .run(RunTarget::Frames(1), 50_000_000, 1_000_000)
        .expect("frame run");
    let (width, height) = session.screen_size().expect("size");
    let contrasting = vec![0xFF; width as usize * height as usize * 4];
    write_png(&read_root, "missing.png", width, height, &contrasting);
    let start_frame = session.timeline().epoch_frames;

    assert!(
        !session
            .wait_for_screen("missing.png", 0.0, 2, 100_000_000)
            .expect("wait")
    );
    assert_eq!(session.timeline().epoch_frames, start_frame + 2);
    let comparison = artifact_root.join("missing-compare.png");
    assert!(comparison.exists());
    let decoded = Surface::load_png(&std::fs::read(comparison).expect("read comparison"))
        .expect("decode comparison");
    assert_eq!(decoded.dimensions(), (width * 2, height));
    assert!(receiver.try_iter().any(|message| matches!(
        message,
        MessageProtocol::Output(text) if text.contains("artifact: missing-compare.png")
    )));
}

#[test]
fn wait_for_screen_tick_bound_returns_false_without_a_presentation() {
    let read_root = temp_dir("wait-tick-read");
    let artifact_root = temp_dir("wait-tick-artifacts");
    let mut session = rooted_session(&read_root, &artifact_root);
    session.install_machine(build_pc98());
    session
        .run(RunTarget::Frames(1), 50_000_000, 1_000_000)
        .expect("frame run");
    let (width, height) = session.screen_size().expect("size");
    write_png(
        &read_root,
        "tick-miss.png",
        width,
        height,
        &vec![0xFF; width as usize * height as usize * 4],
    );
    let start_frame = session.timeline().epoch_frames;

    assert!(
        !session
            .wait_for_screen("tick-miss.png", 0.0, 10, 1)
            .expect("wait")
    );
    assert_eq!(session.timeline().epoch_frames, start_frame);
    assert!(artifact_root.join("tick-miss-compare.png").exists());
}

#[test]
fn wait_for_screen_dimension_mismatch_is_a_miss_with_an_artifact() {
    let read_root = temp_dir("wait-dimension-read");
    let artifact_root = temp_dir("wait-dimension-artifacts");
    let mut session = rooted_session(&read_root, &artifact_root);
    session.install_machine(build_pc98());
    session
        .run(RunTarget::Frames(1), 50_000_000, 1_000_000)
        .expect("frame run");
    let (width, height) = session.screen_size().expect("size");
    write_png(&read_root, "small.png", 8, 8, &vec![0u8; 8 * 8 * 4]);

    assert!(
        !session
            .wait_for_screen("small.png", 0.0, 0, 0)
            .expect("wait")
    );
    let comparison = artifact_root.join("small-compare.png");
    let decoded = Surface::load_png(&std::fs::read(comparison).expect("read comparison"))
        .expect("decode comparison");
    assert_eq!(decoded.dimensions(), (width + 8, height.max(8)));
}

#[test]
fn wait_for_screen_artifact_failure_does_not_replace_false_result() {
    let read_root = temp_dir("wait-budget-read");
    let artifact_root = temp_dir("wait-budget-artifacts");
    let mut session = rooted_session(&read_root, &artifact_root);
    session.install_machine(build_pc98());
    session
        .run(RunTarget::Frames(1), 50_000_000, 1_000_000)
        .expect("frame run");
    let (width, height) = session.screen_size().expect("size");
    write_png(
        &read_root,
        "unmatched.png",
        width,
        height,
        &vec![0xFF; width as usize * height as usize * 4],
    );
    session.set_budgets(SessionBudgets {
        artifact_bytes: Some(0),
        ..SessionBudgets::default()
    });

    assert!(
        !session
            .wait_for_screen("unmatched.png", 0.0, 0, 0)
            .expect("wait")
    );
    assert!(!artifact_root.join("unmatched-compare.png").exists());
}

#[test]
fn pc98_boot_to_title_matches_baseline() {
    let run = run_committed_script("pc98-title.scm", 120);
    assert!(
        matches!(
            run.termination,
            RunTermination::Completed(ExecutionResult::Ok)
        ),
        "expected Completed(Ok), got {:?}",
        run.termination
    );
    // The script's own screenshot artifact is written under the artifact root.
    assert!(run.artifact_root.join("pc98-title-actual.png").exists());
}

#[test]
fn pc98_check_screen_failure_writes_comparison_image() {
    let run = run_committed_script("pc98-mismatch.scm", 120);
    // A failed check-screen ends with a single ERROR result and exit code 1.
    match &run.termination {
        RunTermination::Completed(ExecutionResult::Error { message }) => {
            assert!(
                message.contains("PC-98 mismatch: 1 of 1 test case(s) failed"),
                "summary should report a failure, got {message:?}"
            );
            assert!(
                message.contains("check: (check-screen machine \"expected/pc98-wrong.png\")"),
                "summary should identify the failed check, got {message:?}"
            );
        }
        other => panic!("expected Completed(Error), got {other:?}"),
    }
    assert_eq!(run.exit_code, 1);
    // The side-by-side comparison image is written under the artifact root.
    let comparison = run.artifact_root.join("pc98-wrong-compare.png");
    assert!(comparison.exists(), "comparison image should be written");
    let decoded = Surface::load_png(&std::fs::read(&comparison).expect("read comparison"))
        .expect("decode comparison");
    // Expected (640) on the left and actual (640) on the right.
    assert_eq!(decoded.dimensions(), (1280, 400));
}

#[test]
fn public_wait_for_screen_contract_and_artifact_hold() {
    let run = run_committed_script("wait-for-screen.scm", 120);
    assert!(
        matches!(
            run.termination,
            RunTermination::Completed(ExecutionResult::Ok)
        ),
        "expected Completed(Ok), got {:?}",
        run.termination
    );
    assert!(run.artifact_root.join("pc98-wrong-compare.png").exists());
}
