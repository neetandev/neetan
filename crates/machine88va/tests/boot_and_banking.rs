//! Boot and banking integration tests: the V30 reset vector, ROM fetch through
//! the system bus, and a small program executing from RAM that drives a banking
//! write visible in the memory window.

use common::{Bus, Cpu};
use machine88va::{Pc88VaMachine, Pc88VaModel};

#[path = "common/harness.rs"]
mod harness;
use harness::*;

const ROM1_SEED: u8 = 0x30;

#[test]
fn reset_vector_points_at_rom1() {
    let mut machine = Pc88VaMachine::new(Pc88VaModel::PC88VA2, synthetic_roms());
    assert_eq!(machine.cpu.cs(), 0xF000);
    assert_eq!(machine.cpu.ip(), 0xFFF0);

    let rom1 = fill(ROM1_SEED, 0x2_0000);
    assert_eq!(machine.bus.read_byte(0xF_FFF0), rom1[0xFFF0]);
}

#[test]
fn program_in_ram_drives_a_banking_write() {
    let mut machine = Pc88VaMachine::new(Pc88VaModel::PC88VA2, synthetic_roms());

    // MOV DX, 0x0152 ; MOV AL, 0x05 ; OUT DX, AL ; HLT
    let program: [u8; 7] = [0xBA, 0x52, 0x01, 0xB0, 0x05, 0xEE, 0xF4];
    let base = 0x0400u32;
    for (offset, byte) in program.iter().enumerate() {
        machine.bus.write_byte(base + offset as u32, *byte);
    }

    machine.cpu.set_ip(base as u16);
    machine.cpu.set_cs(0x0000);

    machine.run_for(1000);

    assert!(machine.cpu.halted());
    assert_eq!(machine.cpu.ax() & 0xFF, 0x05);
    // VA1 decode of 0x152: ROM0 bank in the low nibble.
    assert_eq!(machine.bus.io_read_byte(0x152), 0x05);
    // The selected ROM0 bank is now visible in the 0xE0000 window.
    assert_eq!(
        machine.bus.read_byte(0xE_0000),
        fill(0x10, 0x8_0000)[0x5_0000]
    );
}
