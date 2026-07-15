//! Whole-machine FM Towns save-state replay tests.

#[path = "common/harness.rs"]
mod harness;

use common::{Bus, Machine};
use device::{disk::HddImage, scsi::command::opcode};
use harness::{machine_base, machine_cx, machine_mx, program_dma_channel};

fn capture(machine: &mut impl Machine) -> common::MachineStateBlob {
    machine.capture_state().expect("capture must succeed")
}

fn immediate_round_trip(machine: &mut impl Machine) {
    let saved = capture(machine);
    machine.restore_state(&saved).expect("restore must succeed");
    assert_eq!(saved.payload(), capture(machine).payload());
}

fn exercise(
    machine: &mut machine_towns::TownsMachine<{ cpu::CPU_MODEL_486_DX }>,
) -> (Vec<u32>, Vec<u8>) {
    machine.bus.write_byte(0x5000, 0xA5);
    machine.bus.io_write_byte(0x0440, 0x00);
    machine.bus.io_write_byte(0x0442, 0x31);
    machine.bus.io_write_byte(0x0450, 0x01);
    machine.bus.io_write_byte(0x0452, 0x80);
    program_dma_channel(&mut machine.bus, 3, 0x6000, 2048);
    machine.push_keyboard_scancode(0x1E);
    machine.push_mouse_delta(-6, 9);
    machine.set_mouse_buttons(true, true, false);
    machine.run_for(200_000);
    let mut audio = vec![0.0f32; 512];
    machine.generate_audio_samples(0.75, &mut audio);
    let state = capture(machine);
    (
        audio.into_iter().map(f32::to_bits).collect(),
        state.payload().to_vec(),
    )
}

#[test]
fn all_three_models_restore_immediately_without_mutation() {
    immediate_round_trip(&mut machine_base());
    immediate_round_trip(&mut machine_cx());
    immediate_round_trip(&mut machine_mx());
}

#[test]
fn cpu_dma_video_sprite_and_audio_replay_exactly() {
    let mut machine = machine_mx();
    machine.run_for(20_000);
    let saved = capture(&mut machine);

    let expected = exercise(&mut machine);
    machine.restore_state(&saved).expect("restore must succeed");
    let actual = exercise(&mut machine);

    assert_eq!(expected, actual);
}

#[test]
fn corrupt_payload_is_rejected_transactionally() {
    let mut machine = machine_mx();
    let saved = capture(&mut machine);
    let corrupt = saved
        .with_payload(saved.payload()[..saved.payload().len() - 1].to_vec())
        .unwrap();

    assert!(machine.restore_state(&corrupt).is_err());
    assert_eq!(saved.payload(), capture(&mut machine).payload());
}

#[test]
fn model_mismatch_is_rejected() {
    let mut source = machine_mx();
    let saved = capture(&mut source);
    let mut target = machine_cx();

    assert!(target.restore_state(&saved).is_err());
}

#[test]
fn partial_scsi_command_and_dma_replay_exactly() {
    let mut machine = machine_mx();
    let mut disk = vec![0u8; 128 * 1024];
    for (index, byte) in disk[3 * 512..4 * 512].iter_mut().enumerate() {
        *byte = (index as u8) ^ 0x5A;
    }
    machine.insert_hdd(0, HddImage::from_raw_flat(disk).unwrap(), None);
    let destination = 0x7000;
    program_dma_channel(&mut machine.bus, 1, destination, 512);
    machine.bus.io_write_byte(0x0C30, (1 << 0) | (1 << 7));
    machine.bus.io_write_byte(0x0C32, 0x04);
    machine.bus.io_write_byte(0x0C32, 0x00);
    let command = [opcode::READ10, 0, 0, 0, 0, 3, 0, 0, 1, 0];
    for byte in &command[..5] {
        machine.bus.io_write_byte(0x0C30, *byte);
    }
    let saved = capture(&mut machine);

    let complete = |machine: &mut machine_towns::TownsMachine<{ cpu::CPU_MODEL_486_DX }>| {
        for byte in &command[5..] {
            machine.bus.io_write_byte(0x0C30, *byte);
        }
        let deadline = machine.bus.current_cycle() + 1_000_000;
        machine.bus.set_current_cycle(deadline);
        let bytes: Vec<_> = (0..512)
            .map(|index| machine.bus.read_byte(destination + index))
            .collect();
        (bytes, capture(machine).payload().to_vec())
    };
    let expected = complete(&mut machine);
    machine.restore_state(&saved).unwrap();
    let actual = complete(&mut machine);

    assert_eq!(expected, actual);
}
