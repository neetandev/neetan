//! Boot tests: the CPU runs a hand-assembled program from the IPL ROM.

mod harness;

use common::CpuZ80;
use harness::build_machine_with_synthetic_roms;
use machinex1::X1Model;

#[test]
fn cpu_executes_from_ipl_rom() {
    // A NOP stream lets the CPU advance from the reset vector without trapping.
    let mut machine =
        build_machine_with_synthetic_roms(X1Model::X1, |roms| roms.ipl[..0x100].fill(0x00));
    let start_pc = machine.main_cpu.pc();

    let ran = machine.run_for(100);

    assert!(ran >= 100, "the cycle budget is consumed");
    assert_ne!(
        machine.main_cpu.pc(),
        start_pc,
        "the CPU advances through ROM"
    );
}

#[test]
fn ipl_program_writes_to_work_ram() {
    // LD A,0x42 ; LD (0x9000),A ; JR $
    let program = [0x3E, 0x42, 0x32, 0x00, 0x90, 0x18, 0xFE];
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1, |roms| {
        roms.ipl[..program.len()].copy_from_slice(&program);
    });

    machine.run_for(10_000);

    // 0x9000 is in the upper 32 KiB, which is always work RAM.
    assert_eq!(machine.bus.peek_byte(0x9000), 0x42);
}
