//! Whole-machine save-state replay tests for both FM-7 models.

mod harness;

use common::Machine;
use machine_fm7::{BootMode, Fm7Machine};

#[test]
fn fm7_dual_cpu_handshake_replays_exactly() {
    let mut machine = harness::build_machine_with_synthetic_roms(BootMode::Basic, |roms| {
        harness::park_main_cpu(roms);
        harness::park_sub_cpu(roms);
    });
    exercise_handshake(&mut machine);
    assert_exact_replay(&mut machine);
}

#[test]
fn fm77av_dual_cpu_handshake_and_latched_video_replay_exactly() {
    let mut machine = harness::build_av_machine_with_synthetic_roms(BootMode::Basic, |roms| {
        harness::park_main_cpu_av(roms);
        harness::park_sub_cpu(roms);
    });
    exercise_handshake(&mut machine);
    machine.bus.write_byte(0xFD12, 0x40);
    assert_exact_replay(&mut machine);
}

#[test]
fn corrupt_payload_and_model_mismatch_are_rejected_transactionally() {
    let mut machine = harness::build_machine_with_synthetic_roms(BootMode::Basic, |roms| {
        harness::park_main_cpu(roms);
        harness::park_sub_cpu(roms);
    });
    machine.run_for(257);
    let valid = machine.capture_state().expect("capture succeeds");
    let before = valid.payload().to_vec();
    let corrupt = valid
        .with_payload(before[..before.len() / 2].to_vec())
        .expect("corrupt blob container is valid");
    assert!(machine.restore_state(&corrupt).is_err());
    assert_eq!(
        machine
            .capture_state()
            .expect("recapture succeeds")
            .payload(),
        before
    );

    let mut av_machine = harness::build_av_machine_with_synthetic_roms(BootMode::Basic, |_| {});
    assert!(av_machine.restore_state(&valid).is_err());
}

fn exercise_handshake(machine: &mut Fm7Machine) {
    machine.bus.write_byte(0xFD05, 0x80);
    machine.run_for(401);
    machine.bus.write_byte(0xFC80, 0x5A);
    machine.bus.write_byte(0xFD05, 0x00);
    machine.run_for(137);
}

fn assert_exact_replay(machine: &mut Fm7Machine) {
    let snapshot = machine.capture_state().expect("capture succeeds");
    machine.run_for(2_003);
    let mut expected_audio = vec![0.0; 384];
    machine.generate_audio_samples(0.5, &mut expected_audio);
    let expected = machine.capture_state().expect("expected capture succeeds");

    machine.restore_state(&snapshot).expect("restore succeeds");
    machine.run_for(2_003);
    let mut actual_audio = vec![0.0; 384];
    machine.generate_audio_samples(0.5, &mut actual_audio);
    let actual = machine.capture_state().expect("actual capture succeeds");

    assert_eq!(actual_audio, expected_audio);
    assert_eq!(actual.payload(), expected.payload());
}
