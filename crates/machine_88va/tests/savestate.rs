//! Whole-machine save-state replay tests for the PC-88VA2.

#[path = "common/harness.rs"]
mod harness;

use common::{Bus, Machine};

#[test]
fn v30_z80_mailbox_and_graphics_replay_exactly() {
    let mut roms = harness::synthetic_roms();
    roms.rom1.fill(0x90);
    roms.subsys[0] = 0x18;
    roms.subsys[1] = 0xFE;
    let mut machine = harness::machine_from_roms(roms);
    machine.bus.io_write_byte(0xFF, (4 << 1) | 1);
    machine.bus.io_write_byte(0xFE, 0x01);
    machine.bus.io_write_byte(0x153, 0x51);
    machine.bus.io_write_byte(0x580, 0x18);
    machine.bus.io_write_byte(0x590, 0xA5);
    machine.bus.io_write_byte(0x591, 0x5A);
    machine.bus.io_write_byte(0x160, 0x02);
    machine.bus.io_write_byte(0x161, 0x02);
    machine.bus.io_write_byte(0x162, 0xFF);
    machine.bus.io_write_byte(0x163, 0x03);
    machine.bus.io_write_byte(0x164, 0x00);
    machine.bus.io_write_byte(0x165, 0x40);
    machine.bus.io_write_byte(0x16F, 0x0B);
    machine.bus.io_write_byte(0x16E, 1 << 2);
    machine.run_for(1_337);

    let snapshot = machine.capture_state().expect("capture succeeds");
    machine.run_for(2_003);
    let mut expected_audio = vec![0.0; 384];
    machine.generate_audio_samples(0.5, &mut expected_audio);
    let expected = machine.capture_state().expect("expected capture succeeds");

    machine.restore_state(&snapshot).expect("restore succeeds");
    let immediate = machine.capture_state().expect("immediate capture succeeds");
    assert_eq!(
        first_difference(immediate.payload(), snapshot.payload()),
        None,
        "restore itself changed the payload"
    );
    machine.run_for(2_003);
    let mut actual_audio = vec![0.0; 384];
    machine.generate_audio_samples(0.5, &mut actual_audio);
    let actual = machine.capture_state().expect("actual capture succeeds");

    assert_eq!(actual_audio, expected_audio);
    let first_difference = first_difference(actual.payload(), expected.payload());
    assert_eq!(
        first_difference,
        None,
        "payload lengths are {} and {}",
        actual.payload().len(),
        expected.payload().len()
    );
}

#[test]
fn corrupt_payload_and_rom_mismatch_are_rejected_transactionally() {
    let mut machine = harness::machine();
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

    let mut different_roms = harness::synthetic_roms();
    different_roms.rom00[0x1234] ^= 1;
    let mut different_machine = harness::machine_from_roms(different_roms);
    assert!(different_machine.restore_state(&valid).is_err());
}

fn first_difference(actual: &[u8], expected: &[u8]) -> Option<usize> {
    actual
        .iter()
        .zip(expected)
        .position(|(actual, expected)| actual != expected)
}
