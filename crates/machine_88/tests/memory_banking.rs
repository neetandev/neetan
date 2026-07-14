//! Memory banking tests: load a synthetic N88 ROM image, reset the main Z80,
//! and confirm it fetches from the N88 reset region and advances its program
//! counter without hitting an unmapped-memory panic.

use common::CpuZ80;

mod harness;
use harness::build_machine_with_rom;

#[test]
fn main_cpu_fetches_from_n88_reset_region() {
    // A 32 KiB N88 image with a recognizable opcode at the reset vector.
    let mut rom = vec![0u8; 0x8000];
    rom[0] = 0x3E; // LD A,n
    let mut machine = build_machine_with_rom(&rom);

    // The Z80 resets to PC = 0x0000, where the N88-BASIC ROM is mapped.
    assert_eq!(machine.main_cpu.pc(), 0x0000);
    assert_eq!(machine.bus.peek_byte(0x0000), rom[0]);
}

#[test]
fn main_cpu_advances_without_panicking() {
    // An all-NOP image walks the CPU forward through the ROM region.
    let rom = vec![0u8; 0x8000];
    let mut machine = build_machine_with_rom(&rom);

    let consumed = machine.run_for(100_000);
    assert!(consumed > 0, "main CPU consumed no cycles");
    assert_ne!(
        machine.main_cpu.pc(),
        0x0000,
        "program counter did not advance from the reset vector"
    );
}
