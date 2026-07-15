//! Whole-machine save-state replay tests for the PC-8801MC.

mod harness;

use common::Machine;

#[test]
fn dual_z80_mailbox_and_dma_replay_exactly() {
    let mut machine = harness::build_machine_with_synthetic_roms(|roms| {
        roms.n88[0] = 0x18;
        roms.n88[1] = 0xFE;
        roms.disk[0] = 0x18;
        roms.disk[1] = 0xFE;
    });
    machine.bus.io_write(0xFF, (4 << 1) | 1);
    machine.bus.io_write(0xFE, 0x01);
    machine.bus.io_write(0x64, 0x00);
    machine.bus.io_write(0x64, 0x80);
    machine.bus.io_write(0x65, 0x1F);
    machine.bus.io_write(0x65, 0x00);
    machine.bus.io_write(0x68, 1 << 2);
    machine.run_for(1_337);

    assert_exact_replay(&mut machine);
}

#[test]
fn corrupt_payload_is_rejected_without_mutation() {
    let mut machine = harness::build_machine_with_synthetic_roms(|roms| {
        roms.n88[0] = 0x18;
        roms.n88[1] = 0xFE;
    });
    machine.run_for(511);
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
}

#[test]
fn rom_identity_mismatch_is_rejected() {
    let source = harness::synthetic_roms();
    let mut source_machine = harness::build_machine_with_roms(&source);
    let snapshot = source_machine.capture_state().expect("capture succeeds");

    let mut different = harness::synthetic_roms();
    different.n88[0x1234] = 1;
    let mut target_machine = harness::build_machine_with_roms(&different);
    assert!(target_machine.restore_state(&snapshot).is_err());
}

fn assert_exact_replay(machine: &mut machine_88::Pc8801Machine) {
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
