use std::{
    path::{Path, PathBuf},
    process::Command,
};

use common::{Bus, CpuMode, JisChar, MachineModel};
use machine_98::{Pc9801Bus, Pc9801Vm};

const CYCLES_PER_STEP: u64 = 200_000;
const STEPS_PER_PAGE: usize = 200;

fn debug_firmware_dir() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("utils/debug/")
}

fn ensure_debug_text_debug_firmware_exists() -> PathBuf {
    let directory = debug_firmware_dir();
    let firmware_path = directory.join("debug_text.rom");

    let status = Command::new("make")
        .arg("-C")
        .arg(&directory)
        .arg("debug_text.rom")
        .status()
        .expect("Failed to run make for debug_text.rom");
    assert!(status.success(), "Failed to build debug_text.rom");

    firmware_path
}

fn run_steps(machine: &mut Pc9801Vm, steps: usize) {
    for _ in 0..steps {
        machine.run_for(CYCLES_PER_STEP);
    }
}

fn send_enter(bus: &mut Pc9801Bus) {
    bus.push_keyboard_scancode(0x1C);
    bus.push_keyboard_scancode(0x9C);
}

fn text_offset(row: u32, column: u32) -> u32 {
    ((row * 80) + column) * 2
}

fn read_text_char(bus: &Pc9801Bus, row: u32, column: u32) -> JisChar {
    let offset = text_offset(row, column) as usize;
    let text_vram = bus.text_vram();
    JisChar::from_vram_bytes(text_vram[offset], text_vram[offset + 1])
}

fn gdc_data(bus: &mut Pc9801Bus, value: u8) {
    bus.io_write_byte(0x60, value);
}

fn gdc_command(bus: &mut Pc9801Bus, value: u8) {
    bus.io_write_byte(0x62, value);
}

fn gdc_set_cursor(bus: &mut Pc9801Bus, address: u16) {
    gdc_command(bus, 0x49);
    gdc_data(bus, address as u8);
    gdc_data(bus, (address >> 8) as u8);
    gdc_data(bus, 0);
}

fn gdc_set_mask(bus: &mut Pc9801Bus, mask: u16) {
    gdc_command(bus, 0x4A);
    gdc_data(bus, mask as u8);
    gdc_data(bus, (mask >> 8) as u8);
}

fn gdc_set_vector_right(bus: &mut Pc9801Bus, count: u16) {
    gdc_command(bus, 0x4C);
    gdc_data(bus, 2);
    gdc_data(bus, count as u8);
    gdc_data(bus, (count >> 8) as u8);
}

fn gdc_write_word(bus: &mut Pc9801Bus, address: u16, value: u16) {
    gdc_set_cursor(bus, address);
    gdc_set_mask(bus, 0xFFFF);
    gdc_set_vector_right(bus, 0);
    gdc_command(bus, 0x20);
    gdc_data(bus, value as u8);
    gdc_data(bus, (value >> 8) as u8);
}

fn gdc_read_word(bus: &mut Pc9801Bus, address: u16) -> u16 {
    gdc_set_cursor(bus, address);
    gdc_set_vector_right(bus, 1);
    gdc_command(bus, 0xA0);
    let low = bus.io_read_byte(0x62);
    let high = bus.io_read_byte(0x62);
    u16::from(low) | (u16::from(high) << 8)
}

#[test]
fn master_gdc_wdat_rdat_accesses_text_vram_through_character_ports() {
    let mut bus = Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48_000);

    gdc_write_word(&mut bus, 0x0000, 0x1241);
    gdc_write_word(&mut bus, 0x0001, 0x5678);
    gdc_write_word(&mut bus, 0x1000, 0x00E1);

    let text_vram = bus.text_vram();
    assert_eq!(text_vram[0], 0x41);
    assert_eq!(text_vram[1], 0x12);
    assert_eq!(text_vram[2], 0x78);
    assert_eq!(text_vram[3], 0x56);
    assert_eq!(text_vram[0x2000], 0xE1);
    assert_eq!(text_vram[0x2001], 0x00);

    assert_eq!(gdc_read_word(&mut bus, 0x0000), 0x1241);
    assert_eq!(gdc_read_word(&mut bus, 0x0001), 0x5678);
    assert_eq!(gdc_read_word(&mut bus, 0x1000), 0x00E1);
}

#[test]
fn debug_text_debug_firmware_pages_and_wrap() {
    let firmware_path = ensure_debug_text_debug_firmware_exists();

    let firmware_data = std::fs::read(&firmware_path)
        .unwrap_or_else(|error| panic!("Failed to read {}: {error}", firmware_path.display()));

    let mut bus = Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48_000);
    bus.load_bios_rom(&firmware_data);

    let mut machine = Pc9801Vm::new(cpu::V30::new(), bus);

    // Page 0 (ANK): verify corners of 16x16 table at row 2..17 and columns 0,2,..,30.
    run_steps(&mut machine, STEPS_PER_PAGE);

    assert_eq!(
        read_text_char(&machine.bus, 2, 0),
        JisChar::from_u16(0x0000)
    );
    assert_eq!(
        read_text_char(&machine.bus, 2, 2),
        JisChar::from_u16(0x0001)
    );
    assert_eq!(
        read_text_char(&machine.bus, 17, 30),
        JisChar::from_u16(0x00FF)
    );

    // Page 1 (Kanji block 1): starts at VRAM row 0x01, column 0x21.
    // JIS: ku = 0x01 + 0x20 = 0x21, ten = 0x21 -> JIS 0x2121
    send_enter(&mut machine.bus);
    run_steps(&mut machine, STEPS_PER_PAGE);

    assert_eq!(
        read_text_char(&machine.bus, 1, 0),
        JisChar::from_u16(0x2121)
    );

    // Last entry on block-1 page (index 959): VRAM row 0x0B, column 0x34.
    // JIS: ku = 0x0B + 0x20 = 0x2B, ten = 0x34 -> JIS 0x2B34
    assert_eq!(
        read_text_char(&machine.bus, 24, 78),
        JisChar::from_u16(0x2B34)
    );

    // Page 2 (Kanji block 2): starts at VRAM row 0x0B, column 0x35.
    // JIS: ku = 0x0B + 0x20 = 0x2B, ten = 0x35 -> JIS 0x2B35
    send_enter(&mut machine.bus);
    run_steps(&mut machine, STEPS_PER_PAGE);

    assert_eq!(
        read_text_char(&machine.bus, 1, 0),
        JisChar::from_u16(0x2B35)
    );

    // Advance through remaining pages and verify wrap back to page 0.
    for _ in 0..5 {
        send_enter(&mut machine.bus);
        run_steps(&mut machine, STEPS_PER_PAGE);
    }

    assert_eq!(
        read_text_char(&machine.bus, 2, 0),
        JisChar::from_u16(0x0000)
    );
    assert_eq!(
        read_text_char(&machine.bus, 17, 30),
        JisChar::from_u16(0x00FF)
    );
}
