//! Whole-machine PC/AT save-state replay tests.

#[path = "common/harness.rs"]
mod harness;

use common::{Bus, Machine};
use device::{cdrom::CdImage, disk::HddImage, floppy::FloppyImage};
use harness::{
    machine_for_model, machine_with_roms, reset_vector_bios, run_millis, synthetic_roms,
};
use machine_at::AtModel;

fn capture(machine: &mut impl Machine) -> common::MachineStateBlob {
    machine.capture_state().expect("capture must succeed")
}

fn exercise(machine: &mut machine_at::AtMachine) -> (Vec<u32>, Vec<u8>) {
    machine.bus.write_byte(0x4000, 0x5A);
    machine.bus.io_write_byte(0x22, 0x19);
    machine.bus.io_write_byte(0x23, 0x40);
    machine.push_keyboard_scancode(0x1C);
    machine.push_mouse_delta(7, -4);
    machine.set_mouse_buttons(true, false, false);
    machine.set_joystick_axes(0, Some((1200, -800)));
    run_millis(machine, 3);
    let mut audio = vec![0.0f32; 512];
    machine.generate_audio_samples(0.75, &mut audio);
    let state = capture(machine);
    (
        audio.into_iter().map(f32::to_bits).collect(),
        state.payload().to_vec(),
    )
}

fn patterned_hdd() -> HddImage {
    let mut data = vec![0u8; 2 * 16 * 63 * 512];
    for logical_block in 0..(data.len() / 512) {
        data[logical_block * 512] = logical_block as u8;
        data[logical_block * 512 + 1] = (logical_block >> 8) as u8;
    }
    HddImage::from_at_flat(data).unwrap()
}

fn test_cdimage() -> CdImage {
    let cue = "FILE \"test.bin\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n";
    let mut data = vec![0u8; 2048 * 100];
    for (index, byte) in data.iter_mut().enumerate() {
        *byte = index as u8;
    }
    CdImage::from_cue(cue, data).unwrap()
}

fn test_floppy(fill: u8) -> FloppyImage {
    FloppyImage::from_img_bytes(&vec![fill; 1_474_560]).unwrap()
}

fn start_atapi_packet(machine: &mut machine_at::AtMachine) {
    machine.bus.io_write_byte(0x174, 0xFE);
    machine.bus.io_write_byte(0x175, 0xFF);
    machine.bus.io_write_byte(0x177, 0xA0);
}

fn clear_atapi_media_attention(machine: &mut machine_at::AtMachine) {
    start_atapi_packet(machine);
    for word in [0x0012, 0x0000, 0x0024, 0x0000, 0x0000, 0x0000] {
        machine.bus.io_write_word(0x170, word);
    }
    for _ in 0..18 {
        machine.bus.io_read_word(0x170);
    }
}

#[test]
fn hle_bios_boot_round_trips() {
    let mut boot = vec![0u8; 1_474_560];
    boot[0] = 0xFA;
    boot[1] = 0xF4;
    let mut machine = machine_with_roms::<common::NoTrace>(
        AtModel::At486Dx66,
        machine_at::LoadedRoms::hle_stub_set(),
    );
    machine
        .bus
        .insert_floppy(0, FloppyImage::from_img_bytes(&boot).unwrap(), None)
        .unwrap();
    run_millis(&mut machine, 100);

    let saved = capture(&mut machine);
    machine.restore_state(&saved).expect("restore must succeed");
    let restored = capture(&mut machine);
    assert_eq!(saved.payload(), restored.payload());
}

/// Boot sector that keeps every HLE BIOS bus field alive: the teletype bell
/// tops up the beeper tick countdown, AH=12h BL=33h holds the gray-scale
/// summing request in BDA 40:89, and the non-destructive INT 16h read keeps
/// the keyboard path busy.
#[rustfmt::skip]
const HLE_FIELD_BOOT_CODE: &[u8] = &[
    0xFB,                   // STI
    0xB8, 0x07, 0x0E,       // MOV AX, 0x0E07 (teletype BEL)
    0xCD, 0x10,             // INT 10h
    0xB8, 0x00, 0x12,       // MOV AX, 0x1200 (gray-scale summing enable)
    0xBB, 0x33, 0x00,       // MOV BX, 0x0033
    0xCD, 0x10,             // INT 10h
    0xB4, 0x01,             // MOV AH, 0x01 (peek at the keyboard buffer)
    0xCD, 0x16,             // INT 16h
    0xEB, 0xED,             // JMP back to the BEL
];

#[test]
fn hle_bios_fields_replay_exactly() {
    let mut boot = vec![0u8; 1_474_560];
    boot[..HLE_FIELD_BOOT_CODE.len()].copy_from_slice(HLE_FIELD_BOOT_CODE);
    let mut machine = machine_with_roms::<common::NoTrace>(
        AtModel::At486Dx66,
        machine_at::LoadedRoms::hle_stub_set(),
    );
    machine
        .bus
        .insert_floppy(0, FloppyImage::from_img_bytes(&boot).unwrap(), None)
        .unwrap();
    run_millis(&mut machine, 100);

    // The scancode latch behind port 0x07F1 and the BDA buffer are live.
    machine.push_keyboard_scancode(0x1E);
    machine.push_keyboard_scancode(0x9E);
    run_millis(&mut machine, 5);

    // The boot code really ran and left the state the replay depends on.
    assert_eq!(
        machine.bus.read_byte(0x489) & 0x02,
        0x02,
        "gray-scale summing requested in BDA 40:89"
    );
    assert_ne!(
        machine.bus.read_byte(0x41C),
        machine.bus.read_byte(0x41A),
        "a keystroke is waiting in the BDA buffer"
    );

    let saved = capture(&mut machine);
    let expected = exercise(&mut machine);
    machine.restore_state(&saved).expect("restore must succeed");
    let actual = exercise(&mut machine);

    assert_eq!(expected, actual);
}

/// The HLE ROM images are built deterministically, which is what makes the
/// `rom:vga-bios` resource identity in a save state stable across processes.
#[test]
fn hle_rom_images_are_byte_identical_across_builds() {
    let first = machine_at::LoadedRoms::hle_stub_set();
    let second = machine_at::LoadedRoms::hle_stub_set();

    assert_eq!(first.system_bios, second.system_bios);
    assert_eq!(first.vga_bios, second.vga_bios);
}

#[test]
fn both_models_restore_immediately_without_mutation() {
    for model in [AtModel::At486Dx50, AtModel::At486Dx66] {
        let mut machine = machine_for_model(model);
        let saved = capture(&mut machine);
        machine.restore_state(&saved).expect("restore must succeed");
        let restored = capture(&mut machine);
        assert_eq!(saved.payload(), restored.payload());
    }
}

#[test]
fn protected_cpu_chipset_video_and_audio_replay_exactly() {
    let mut machine = machine_for_model(AtModel::At486Dx66);
    run_millis(&mut machine, 1);
    let saved = capture(&mut machine);

    let expected = exercise(&mut machine);
    machine.restore_state(&saved).expect("restore must succeed");
    let actual = exercise(&mut machine);

    assert_eq!(expected, actual);
}

#[test]
fn corrupt_payload_is_rejected_transactionally() {
    let mut machine = machine_for_model(AtModel::At486Dx50);
    let saved = capture(&mut machine);
    let corrupt = saved
        .with_payload(saved.payload()[..saved.payload().len() - 1].to_vec())
        .unwrap();

    assert!(machine.restore_state(&corrupt).is_err());
    assert_eq!(saved.payload(), capture(&mut machine).payload());
}

#[test]
fn system_rom_mismatch_is_rejected() {
    let mut source = machine_for_model(AtModel::At486Dx66);
    let saved = capture(&mut source);
    let mut roms = synthetic_roms();
    roms.system_bios = reset_vector_bios(&[0x90, 0xF4]);
    let mut target = machine_with_roms::<common::NoTrace>(AtModel::At486Dx66, roms);

    assert!(target.restore_state(&saved).is_err());
}

#[test]
fn floppy_mismatch_reports_expected_path_and_succeeds_after_swap() {
    let mut machine = machine_for_model(AtModel::At486Dx50);
    let expected_path = std::path::PathBuf::from("media/game-disk-1.img");
    machine
        .bus
        .insert_floppy(0, test_floppy(0x11), Some(expected_path.clone()))
        .unwrap();
    let saved = capture(&mut machine);

    machine.bus.eject_floppy(0);
    machine
        .bus
        .insert_floppy(
            0,
            test_floppy(0x22),
            Some(std::path::PathBuf::from("media/game-disk-2.img")),
        )
        .unwrap();

    let error = machine.restore_state(&saved).unwrap_err();
    let common::SaveStateError::MediaMismatch(mismatch) = error else {
        panic!("expected a structured media mismatch");
    };
    assert_eq!(mismatch.entries().len(), 1);
    assert_eq!(
        mismatch.entries()[0]
            .expected()
            .unwrap()
            .source_path
            .as_ref()
            .unwrap(),
        &save_state::MediaSourcePath::from_path(&expected_path)
    );

    machine.bus.eject_floppy(0);
    machine
        .bus
        .insert_floppy(0, test_floppy(0x33), Some(expected_path))
        .unwrap();
    machine.restore_state(&saved).unwrap();
}

#[test]
fn pending_ide_read_replays_exactly() {
    let mut machine = machine_for_model(AtModel::At486Dx50);
    machine.bus.insert_hdd(0, patterned_hdd(), None).unwrap();
    machine.bus.io_write_byte(0x1F6, 0xA1);
    machine.bus.io_write_byte(0x1F2, 1);
    machine.bus.io_write_byte(0x1F3, 1);
    machine.bus.io_write_byte(0x1F4, 0);
    machine.bus.io_write_byte(0x1F5, 0);
    machine.bus.io_write_byte(0x1F7, 0x20);
    let saved = capture(&mut machine);

    let complete = |machine: &mut machine_at::AtMachine| {
        let deadline = machine.bus.next_event_cycle().unwrap();
        machine.bus.set_current_cycle(deadline);
        machine.bus.io_read_byte(0x1F7);
        let words: Vec<_> = (0..256).map(|_| machine.bus.io_read_word(0x1F0)).collect();
        (words, capture(machine).payload().to_vec())
    };
    let expected = complete(&mut machine);
    machine.restore_state(&saved).unwrap();
    let actual = complete(&mut machine);

    assert_eq!(expected, actual);
}

#[test]
fn active_atapi_cd_playback_replays_exactly() {
    let mut machine = machine_for_model(AtModel::At486Dx66);
    machine.bus.insert_cdrom(test_cdimage()).unwrap();
    clear_atapi_media_attention(&mut machine);
    start_atapi_packet(&mut machine);
    for word in [0x0047, 0x0000, 0x0002, 0x0300, 0x0000, 0x0000] {
        machine.bus.io_write_word(0x170, word);
    }
    let saved = capture(&mut machine);

    let render = |machine: &mut machine_at::AtMachine| {
        let mut audio = vec![0.0f32; 1024];
        machine.generate_audio_samples(1.0, &mut audio);
        (
            audio.into_iter().map(f32::to_bits).collect::<Vec<_>>(),
            capture(machine).payload().to_vec(),
        )
    };
    let expected = render(&mut machine);
    machine.restore_state(&saved).unwrap();
    let actual = render(&mut machine);

    assert_eq!(expected, actual);
}
