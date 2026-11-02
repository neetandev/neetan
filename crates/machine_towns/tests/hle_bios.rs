//! Integration tests for the HLE system ROM: the POST, the boot device search
//! and the IPL entry of a synthetic CD-ROM, the disk BIOS and the wait loop
//! called by guest code, and a save state round trip in HLE mode.

#[path = "common/harness.rs"]
mod harness;

use common::{Bus, Cpu, Machine};
use cpu::SegReg32;
use harness::{linear_pc, machine_mx, machine_mx_hle};
use machine_towns::{TownsBootDevice, TownsMachine};

/// Physical address of the IPL entry B000:0004.
const IPL_ENTRY: u32 = 0xB_0004;
/// Physical address the test IPL reads sector 1 to.
const SECTOR_BUFFER: u32 = 0x2_0000;
/// Cycle budget for one boot phase.
const PHASE_CYCLES: u64 = 20_000_000;
/// Wait loop units the test IPL waits for (100 x 10 us = 1 ms).
const WAIT_UNITS: u16 = 100;

/// Test IPL: the `IPL4` signature, a disk BIOS read of CD sector 1 to 2000:0000,
/// a 1 ms wait through `FFFB:0023` and `HLT`.
fn ipl_sector() -> Vec<u8> {
    let mut code = b"IPL4".to_vec();
    code.extend_from_slice(&[
        0xB8,
        0xC0,
        0x05, // mov ax, 05C0h
        0xBB,
        0x01,
        0x00, // mov bx, 1
        0xB9,
        0x00,
        0x00, // mov cx, 0
        0xBA,
        0x01,
        0x00, // mov dx, 1
        0x68,
        0x00,
        0x20, // push 2000h
        0x1F, // pop ds
        0xBF,
        0x00,
        0x00, // mov di, 0
        0x9A,
        0x14,
        0x00,
        0xFB,
        0xFF, // call far FFFB:0014
        0xA3,
        0x00,
        0x10, // mov [1000h], ax
        0x9C, // pushf
        0x58, // pop ax
        0xA3,
        0x02,
        0x10, // mov [1002h], ax
        0xB9,
        WAIT_UNITS as u8,
        (WAIT_UNITS >> 8) as u8, // mov cx, WAIT_UNITS
        0x9A,
        0x23,
        0x00,
        0xFB,
        0xFF, // call far FFFB:0023
        0xF4, // hlt
    ]);
    code.resize(2048, 0);
    code
}

/// Writes a two-sector MODE1/2048 CD image with the test IPL and returns the
/// folder that holds it.
fn write_cd_image() -> common::temporary::TemporaryFolder {
    let folder = common::temporary::TemporaryFolder::new("towns_hle_ipl");
    let mut data = ipl_sector();
    data.extend(std::iter::repeat_n(0x5A, 2048));
    folder.write_file("disc.iso", &data);
    folder.write_file(
        "disc.cue",
        b"FILE \"disc.iso\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n",
    );
    folder
}

/// Runs single instructions until the CPU reaches `target` or `budget` cycles
/// pass. Returns whether the target was reached.
fn run_until_pc(
    machine: &mut TownsMachine<{ cpu::CPU_MODEL_486_DX }>,
    target: u32,
    budget: u64,
) -> bool {
    let end = machine.bus.current_cycle() + budget;
    while machine.bus.current_cycle() < end {
        if linear_pc(machine) == target {
            return true;
        }
        machine.run_for(1);
    }
    false
}

/// Builds an HLE machine with the test CD inserted and boots it to the IPL entry.
fn boot_to_ipl(
    folder: &common::temporary::TemporaryFolder,
) -> TownsMachine<{ cpu::CPU_MODEL_486_DX }> {
    let mut machine = machine_mx_hle();
    machine
        .insert_cdrom(&folder.path().join("disc.cue"))
        .expect("CD image");
    machine.set_boot_device(TownsBootDevice::Cd);
    assert!(run_until_pc(&mut machine, IPL_ENTRY, PHASE_CYCLES));
    machine
}

#[test]
fn hle_post_enters_the_cd_ipl_with_the_real_rom_registers() {
    let folder = write_cd_image();
    let machine = boot_to_ipl(&folder);
    let state = &machine.cpu.state;

    assert_eq!(machine.cpu.eax(), 0x0000_FFFF);
    assert_eq!(machine.cpu.ebx(), 0x0000_0008);
    assert_eq!(machine.cpu.ecx(), 0x0000_0000);
    assert_eq!(machine.cpu.edx(), 0x0000_31AC);
    assert_eq!(state.esi(), 0xFFFE_0CD5);
    assert_eq!(machine.cpu.cs(), 0xB000);
    assert_eq!(machine.cpu.ip(), 0x0004);
    assert_eq!(machine.cpu.ds(), 0xFC00);
    assert_eq!(machine.cpu.es(), 0xC000);
    assert_eq!(machine.cpu.ss(), 0xF7A0);
    assert_eq!(machine.cpu.sp(), 0x0574);
    assert_eq!(machine.cpu.flags() & 0x0FD7, 0x0097);
    assert_eq!(state.seg_bases[SegReg32::SS as usize], 0xF_7A00);
    assert_eq!(state.cr0 & 0x0000_0001, 0);
    assert_eq!(state.gdt_base, 0xFFFF_C080);
    assert_eq!(state.gdt_limit, 0x0040);
}

#[test]
fn ipl_reads_through_the_disk_bios_and_waits_through_the_wait_loop() {
    let folder = write_cd_image();
    let mut machine = boot_to_ipl(&folder);
    let before = machine.bus.current_cycle();
    let end = before + PHASE_CYCLES;
    while !machine.cpu.halted() && machine.bus.current_cycle() < end {
        machine.run_for(1);
    }
    assert!(machine.cpu.halted());

    let status = u16::from(machine.bus.read_byte(0x2_1000))
        | (u16::from(machine.bus.read_byte(0x2_1001)) << 8);
    let flags = machine.bus.read_byte(0x2_1002);
    assert_eq!(status, 0x00C0);
    assert_eq!(flags & 0x01, 0);
    assert_eq!(machine.bus.read_byte(SECTOR_BUFFER), 0x5A);
    assert_eq!(machine.bus.read_byte(SECTOR_BUFFER + 2047), 0x5A);

    let elapsed = machine.bus.current_cycle() - before;
    let wait_cycles = u64::from(machine.cpu_clock_hz() as u32) * u64::from(WAIT_UNITS) / 100_000;
    assert!(elapsed >= wait_cycles);
}

#[test]
fn hle_machine_reports_no_device_and_keeps_retrying() {
    let mut machine = machine_mx_hle();
    machine.run_for(PHASE_CYCLES);
    assert!(!machine.cpu.halted());
    assert!((0xF_C000..0x10_0000).contains(&linear_pc(&machine)));
}

#[test]
fn hle_save_state_round_trips_and_replays() {
    let folder = write_cd_image();
    let mut machine = boot_to_ipl(&folder);
    let saved = machine.capture_state().expect("capture");
    machine.restore_state(&saved).expect("restore");
    assert_eq!(saved.payload(), machine.capture_state().unwrap().payload());

    machine.run_for(2_000_000);
    let expected = machine.capture_state().unwrap().payload().to_vec();
    machine.restore_state(&saved).expect("restore");
    machine.run_for(2_000_000);
    assert_eq!(expected, machine.capture_state().unwrap().payload());
}

#[test]
fn hle_save_state_does_not_restore_into_a_real_rom_machine() {
    let mut hle = machine_mx_hle();
    hle.run_for(10_000);
    let saved = hle.capture_state().expect("capture");
    let mut real = machine_mx();
    assert!(real.restore_state(&saved).is_err());
}

#[test]
fn software_reset_runs_the_short_post_and_boots_again() {
    let folder = write_cd_image();
    let mut machine = boot_to_ipl(&folder);
    machine.bus.write_byte(0x3_0000, 0xA5);
    machine.bus.io_write_byte(0x0020, 0x01);
    machine.run_for(1);
    assert!(run_until_pc(&mut machine, IPL_ENTRY, PHASE_CYCLES));
    assert_eq!(machine.bus.read_byte(0x3_0000), 0xA5);
    assert_eq!(machine.cpu.ebx(), 0x0000_0008);
}
