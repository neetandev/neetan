//! Skeleton test: the machine fetches and executes the first BIOS
//! instructions from the 486 reset vector at 0xFFFFFFF0.

use common::{Cpu, Machine};
use machine_at::{AtBus, AtMachine, LoadedRoms};

/// Builds a 64 KiB BIOS whose reset-vector entry point runs a short program:
/// `MOV AL, 0x42; OUT 0x80, AL; JMP $`.
fn reset_vector_bios() -> Vec<u8> {
    let mut bios = vec![0u8; 0x1_0000];
    // The 486 resets to CS base 0xFFFF0000, IP 0xFFF0, so the first fetch is at
    // BIOS offset 0xFFF0.
    let program = [0xB0, 0x42, 0xE6, 0x80, 0xEB, 0xFE];
    bios[0xFFF0..0xFFF0 + program.len()].copy_from_slice(&program);
    bios
}

#[test]
fn executes_first_bios_instructions_from_reset_vector() {
    let roms = LoadedRoms {
        system_bios: reset_vector_bios(),
        vga_bios: vec![0u8; 0x8000],
    };
    let bus = AtBus::<common::NoTrace>::new(66_000_000, 8 * 1024 * 1024, roms, 48_000);

    let mut cpu = cpu::I386::<{ cpu::CPU_MODEL_486_DX }, { cpu::ADDRESS_WIDTH_32 }>::new();
    cpu.reset();

    let mut machine = AtMachine::new(cpu, bus);

    // A few thousand cycles is far more than enough to reach the OUT and settle
    // into the JMP $ loop.
    machine.run_for(4096);

    assert_eq!(machine.bus.last_post_code(), 0x42);
}
