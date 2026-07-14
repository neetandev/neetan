use std::{
    path::{Path, PathBuf},
    process::Command,
};

use common::{CpuMode, MachineModel};
use machine_98::{Pc9801Bus, Pc9801Ra};

const CYCLES_PER_STEP: u64 = 200_000;
const STEPS_PER_PAGE: usize = 400;

/// Column where "OK" or "FAIL" status appears.
const STATUS_COL: u32 = 53;

/// Column where ULP distance is displayed.
const ULP_COL: u32 = 42;

fn debug_firmware_dir() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("utils/debug/")
}

fn ensure_debug_fpu_firmware_exists() -> PathBuf {
    let directory = debug_firmware_dir();
    let firmware_path = directory.join("debug_fpu.rom");

    let status = Command::new("make")
        .arg("-C")
        .arg(&directory)
        .arg("debug_fpu.rom")
        .status()
        .expect("Failed to run make for debug_fpu.rom");
    assert!(status.success(), "Failed to build debug_fpu.rom");

    firmware_path
}

fn run_steps(machine: &mut Pc9801Ra, steps: usize) {
    for _ in 0..steps {
        machine.run_for(CYCLES_PER_STEP);
    }
}

fn send_enter(bus: &mut Pc9801Bus) {
    bus.push_keyboard_scancode(0x1C);
    bus.push_keyboard_scancode(0x9C);
}

fn read_text_string(bus: &Pc9801Bus, row: u32, col: u32, length: usize) -> String {
    let text_vram = bus.text_vram();
    let mut result = String::new();
    for i in 0..length {
        let offset = ((row * 80 + col + i as u32) * 2) as usize;
        if offset >= text_vram.len() {
            break;
        }
        let ch = text_vram[offset];
        result.push(ch as char);
    }
    result
}

struct TestRow {
    row: u32,
    name: &'static str,
}

fn verify_page_all_ok(bus: &Pc9801Bus, tests: &[TestRow]) {
    let mut failures = Vec::new();
    for test in tests {
        let ulp_str = read_text_string(bus, test.row, ULP_COL, 10)
            .trim()
            .to_string();
        let status = read_text_string(bus, test.row, STATUS_COL, 4);
        let trimmed = status.trim().to_string();
        if !trimmed.starts_with("OK") {
            failures.push(format!(
                "{}: got '{}' (row {}, ULP={})",
                test.name, trimmed, test.row, ulp_str
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "Tests failed:\n{}",
        failures.join("\n")
    );
}

#[test]
fn debug_fpu_firmware_constants_page() {
    let firmware_path = ensure_debug_fpu_firmware_exists();

    let firmware_data = std::fs::read(&firmware_path)
        .unwrap_or_else(|error| panic!("Failed to read {}: {error}", firmware_path.display()));

    let mut bus = Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48_000);
    bus.load_bios_rom(&firmware_data);

    let mut machine = Pc9801Ra::new(cpu::I386::new(), bus);

    run_steps(&mut machine, STEPS_PER_PAGE);

    let header = read_text_string(&machine.bus, 0, 0, 27);
    assert_eq!(header, "DEBUG_FPU 1/4 FPU CONSTANTS");

    verify_page_all_ok(
        &machine.bus,
        &[
            TestRow {
                row: 3,
                name: "FLDPI",
            },
            TestRow {
                row: 5,
                name: "FLD1",
            },
            TestRow {
                row: 7,
                name: "FLDZ",
            },
            TestRow {
                row: 9,
                name: "FLDL2T",
            },
            TestRow {
                row: 11,
                name: "FLDL2E",
            },
            TestRow {
                row: 13,
                name: "FLDLG2",
            },
            TestRow {
                row: 15,
                name: "FLDLN2",
            },
        ],
    );
}

#[test]
fn debug_fpu_firmware_arithmetic_page() {
    let firmware_path = ensure_debug_fpu_firmware_exists();

    let firmware_data = std::fs::read(&firmware_path)
        .unwrap_or_else(|error| panic!("Failed to read {}: {error}", firmware_path.display()));

    let mut bus = Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48_000);
    bus.load_bios_rom(&firmware_data);

    let mut machine = Pc9801Ra::new(cpu::I386::new(), bus);

    run_steps(&mut machine, STEPS_PER_PAGE);
    send_enter(&mut machine.bus);

    run_steps(&mut machine, STEPS_PER_PAGE);

    let header = read_text_string(&machine.bus, 0, 0, 30);
    assert_eq!(header, "DEBUG_FPU 2/4 BASIC ARITHMETIC");

    verify_page_all_ok(
        &machine.bus,
        &[
            TestRow {
                row: 3,
                name: "3+4",
            },
            TestRow {
                row: 5,
                name: "10-3",
            },
            TestRow {
                row: 7,
                name: "6*7",
            },
            TestRow {
                row: 9,
                name: "355/113",
            },
            TestRow {
                row: 11,
                name: "-5+5",
            },
        ],
    );
}

#[test]
fn debug_fpu_firmware_transcendentals_page() {
    let firmware_path = ensure_debug_fpu_firmware_exists();

    let firmware_data = std::fs::read(&firmware_path)
        .unwrap_or_else(|error| panic!("Failed to read {}: {error}", firmware_path.display()));

    let mut bus = Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48_000);
    bus.load_bios_rom(&firmware_data);

    let mut machine = Pc9801Ra::new(cpu::I386::new(), bus);

    run_steps(&mut machine, STEPS_PER_PAGE);
    send_enter(&mut machine.bus);
    run_steps(&mut machine, STEPS_PER_PAGE);
    send_enter(&mut machine.bus);

    run_steps(&mut machine, STEPS_PER_PAGE);

    let header = read_text_string(&machine.bus, 0, 0, 29);
    assert_eq!(header, "DEBUG_FPU 3/4 TRANSCENDENTALS");

    verify_page_all_ok(
        &machine.bus,
        &[
            TestRow {
                row: 3,
                name: "SQRT(2)",
            },
            TestRow {
                row: 5,
                name: "SIN(PI/6)",
            },
            TestRow {
                row: 7,
                name: "COS(PI/3)",
            },
            TestRow {
                row: 9,
                name: "TAN(PI/4)",
            },
            TestRow {
                row: 11,
                name: "4*ATAN(1)",
            },
            TestRow {
                row: 13,
                name: "F2XM1(0.5)",
            },
            TestRow {
                row: 15,
                name: "FYL2X(1,8)",
            },
            TestRow {
                row: 17,
                name: "FYL2XP1(2,0.25)",
            },
            TestRow {
                row: 19,
                name: "FSCALE(1.5,2)",
            },
            TestRow {
                row: 21,
                name: "MACHIN PI",
            },
        ],
    );
}

#[test]
fn debug_fpu_firmware_golden_page() {
    let firmware_path = ensure_debug_fpu_firmware_exists();

    let firmware_data = std::fs::read(&firmware_path)
        .unwrap_or_else(|error| panic!("Failed to read {}: {error}", firmware_path.display()));

    let mut bus = Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48_000);
    bus.load_bios_rom(&firmware_data);

    let mut machine = Pc9801Ra::new(cpu::I386::new(), bus);

    run_steps(&mut machine, STEPS_PER_PAGE);
    send_enter(&mut machine.bus);
    run_steps(&mut machine, STEPS_PER_PAGE);
    send_enter(&mut machine.bus);
    run_steps(&mut machine, STEPS_PER_PAGE);
    send_enter(&mut machine.bus);

    run_steps(&mut machine, STEPS_PER_PAGE);

    let header = read_text_string(&machine.bus, 0, 0, 29);
    assert_eq!(header, "DEBUG_FPU 4/4 X87 TRIG QUIRKS");

    verify_page_all_ok(
        &machine.bus,
        &[
            TestRow {
                row: 3,
                name: "FSIN(PI)",
            },
            TestRow {
                row: 5,
                name: "FSIN(-PI)",
            },
            TestRow {
                row: 7,
                name: "FCOS(PI/2)",
            },
            TestRow {
                row: 9,
                name: "FSIN(2*PI)",
            },
            TestRow {
                row: 11,
                name: "FSIN(3*PI)",
            },
            TestRow {
                row: 13,
                name: "FCOS(PI)",
            },
            TestRow {
                row: 15,
                name: "FSIN(PI/2)",
            },
        ],
    );
}
