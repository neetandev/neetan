//! Whole-machine X68000 save-state replay and rejection tests.

#[path = "common/harness.rs"]
mod harness;
#[path = "common/spc.rs"]
mod spc;

use common::Machine;
use harness::{
    machine, machine_from_roms, patterned_sasi_hdf, patterned_scsi_hdf, read_byte, test_roms,
    write_byte,
};
use machine_x68k::{X68kMachine, X68kModel};

const SASI_DATA: u32 = 0xE96001;
const SASI_STATUS: u32 = 0xE96003;
const SASI_SELECT: u32 = 0xE96007;
const MIDI_CONTROL: u32 = 0xEAFA03;
const MIDI_RATE: u32 = 0xEAFA09;
const MIDI_ENABLE: u32 = 0xEAFA0B;
const MIDI_DATA: u32 = 0xEAFA0D;

fn capture(machine: &mut impl Machine) -> common::MachineStateBlob {
    machine.capture_state().expect("capture must succeed")
}

fn exercise(machine: &mut X68kMachine) -> (Vec<u32>, Vec<u8>) {
    write_byte(machine, 0x2000, 0x5A);
    write_byte(machine, 0xE8E001, 12);
    write_byte(machine, 0xE88001, 0x40);
    machine.push_keyboard_scancode(0x1E);
    machine.push_mouse_delta(7, -5);
    machine.set_mouse_buttons(true, false, false);
    machine.run_for(20_000);
    let mut audio = vec![0.0; 512];
    machine.generate_audio_samples(0.75, &mut audio);
    (
        audio.into_iter().map(f32::to_bits).collect(),
        capture(machine).payload().to_vec(),
    )
}

fn start_partial_sasi_read(machine: &mut X68kMachine) {
    machine.insert_hdd(0, patterned_sasi_hdf(), None).unwrap();
    write_byte(machine, SASI_SELECT, 1);
    write_byte(machine, SASI_STATUS, 0);
    for byte in [0x08, 0x00, 0x00] {
        write_byte(machine, SASI_DATA, byte);
    }
}

fn finish_partial_sasi_read(machine: &mut X68kMachine) -> (Vec<u8>, Vec<u8>) {
    for byte in [0x05, 1, 0] {
        write_byte(machine, SASI_DATA, byte);
    }
    let data = (0..256)
        .map(|_| read_byte(machine, SASI_DATA))
        .collect::<Vec<_>>();
    let _status = read_byte(machine, SASI_DATA);
    let _message = read_byte(machine, SASI_DATA);
    (data, capture(machine).payload().to_vec())
}

#[test]
fn all_models_restore_immediately_without_mutation() {
    for model in [
        X68kModel::X68000,
        X68kModel::X68000Super,
        X68kModel::X68000Xvi,
    ] {
        let mut machine = machine(model);
        let saved = capture(&mut machine);
        machine.restore_state(&saved).expect("restore must succeed");
        assert_eq!(saved.payload(), capture(&mut machine).payload(), "{model}");
    }
}

#[test]
fn cpu_video_input_and_audio_replay_exactly() {
    let mut machine = machine(X68kModel::X68000Xvi);
    machine.run_for(10_000);
    let saved = capture(&mut machine);

    let expected = exercise(&mut machine);
    machine.restore_state(&saved).unwrap();
    let actual = exercise(&mut machine);

    assert_eq!(expected, actual);
}

#[test]
fn partial_sasi_command_replays_through_data_completion() {
    let mut machine = machine(X68kModel::X68000);
    start_partial_sasi_read(&mut machine);
    let saved = capture(&mut machine);

    let expected = finish_partial_sasi_read(&mut machine);
    machine.restore_state(&saved).unwrap();
    let actual = finish_partial_sasi_read(&mut machine);

    assert_eq!(expected, actual);
}

#[test]
fn active_midi_serializer_replays_exactly() {
    let mut machine = machine(X68kModel::X68000Super);
    machine.install_midi_card();
    write_byte(&mut machine, MIDI_CONTROL, 0x04);
    write_byte(&mut machine, MIDI_RATE, 0x08);
    write_byte(&mut machine, MIDI_CONTROL, 0x05);
    write_byte(&mut machine, MIDI_ENABLE, 0x01);
    for byte in [0x90, 0x40, 0x7F, 0x80] {
        write_byte(&mut machine, MIDI_DATA, byte);
    }
    machine.run_for(5_000);
    let saved = capture(&mut machine);

    let finish = |machine: &mut X68kMachine| {
        machine.run_for(20_000);
        let mut midi = [0; 8];
        let length = machine.flush_midi_into(&mut midi);
        (midi[..length].to_vec(), capture(machine).payload().to_vec())
    };
    let expected = finish(&mut machine);
    machine.restore_state(&saved).unwrap();
    let actual = finish(&mut machine);

    assert_eq!(expected, actual);
}

#[test]
fn partial_internal_scsi_command_replays_exactly() {
    let mut machine = machine(X68kModel::X68000Super);
    machine.insert_hdd(0, patterned_scsi_hdf(2), None).unwrap();
    spc::select(&mut machine, 0);
    write_byte(&mut machine, spc::SPC_PCTL, spc::PHASE_COMMAND);
    spc::set_transfer_counter(&mut machine, 10);
    write_byte(
        &mut machine,
        spc::SPC_SCMD,
        spc::SCMD_TRANSFER | spc::SCMD_PROGRAM_TRANSFER,
    );
    let command = [0x28, 0, 0, 0, 0, 3, 0, 0, 1, 0];
    for &byte in &command[..5] {
        write_byte(&mut machine, spc::SPC_DREG, byte);
    }
    let saved = capture(&mut machine);

    let finish = |machine: &mut X68kMachine| {
        for &byte in &command[5..] {
            write_byte(machine, spc::SPC_DREG, byte);
        }
        write_byte(machine, spc::SPC_INTS, 0xFF);
        let data = spc::read_data_in(machine, 512);
        let status = spc::read_status_and_message(machine);
        (data, status, capture(machine).payload().to_vec())
    };
    let expected = finish(&mut machine);
    machine.restore_state(&saved).unwrap();
    let actual = finish(&mut machine);

    assert_eq!(expected, actual);
}

#[test]
fn partial_floppy_command_replays_exactly() {
    let path = std::env::temp_dir().join("neetan_x68k_savestate.xdf");
    std::fs::write(&path, vec![0x5A; 1_261_568]).unwrap();
    let mut machine = machine(X68kModel::X68000);
    machine.insert_floppy(0, &path).unwrap();
    write_byte(&mut machine, 0xE94007, 0x80);
    write_byte(&mut machine, 0xE94003, 0x07);
    write_byte(&mut machine, 0xE94003, 0x00);
    let command = [0x46, 0, 0, 0, 1, 3, 1, 0x1B, 0xFF];
    for &byte in &command[..4] {
        write_byte(&mut machine, 0xE94003, byte);
    }
    let saved = capture(&mut machine);

    let finish = |machine: &mut X68kMachine| {
        for &byte in &command[4..] {
            write_byte(machine, 0xE94003, byte);
        }
        machine.run_for(100_000);
        capture(machine).payload().to_vec()
    };
    let expected = finish(&mut machine);
    machine.restore_state(&saved).unwrap();
    let actual = finish(&mut machine);

    assert_eq!(expected, actual);
    let _ = std::fs::remove_file(path);
}

#[test]
fn corrupt_payload_is_rejected_transactionally() {
    let mut machine = machine(X68kModel::X68000Super);
    let saved = capture(&mut machine);
    let corrupt = saved
        .with_payload(saved.payload()[..saved.payload().len() - 1].to_vec())
        .unwrap();

    assert!(machine.restore_state(&corrupt).is_err());
    assert_eq!(saved.payload(), capture(&mut machine).payload());
}

#[test]
fn model_and_rom_mismatches_are_rejected() {
    let mut source = machine(X68kModel::X68000Super);
    let saved = capture(&mut source);
    let mut wrong_model = machine(X68kModel::X68000Xvi);
    assert!(wrong_model.restore_state(&saved).is_err());

    let mut roms = test_roms(X68kModel::X68000Super);
    roms.ipl[0] ^= 0xFF;
    let mut wrong_rom = machine_from_roms(X68kModel::X68000Super, common::CpuMode::High, roms);
    assert!(wrong_rom.restore_state(&saved).is_err());
}
