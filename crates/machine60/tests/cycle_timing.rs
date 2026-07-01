//! Machine-level cycle-accuracy tests for the VRAM bus-request stall: the video
//! circuit steals the bus from the CPU for part of every active scanline while
//! the CRT is enabled, and releases it when the display is blanked.

use common::CpuZ80;
use machine60::{Pc6000Bus, Pc6000Model};

mod harness;
use harness::build_machine;

/// Scanline period in main-clock cycles, matching the bus model.
fn line_period(bus: &Pc6000Bus) -> u64 {
    u64::from(bus.cpu_clock_hz()) / 60 / 262
}

#[test]
fn busreq_stalls_then_releases_the_cpu() {
    let mut machine = build_machine(Pc6000Model::Pc6001);
    let period = line_period(&machine.bus);

    // Fire the first scanline so the video circuit grabs the bus.
    machine.bus.set_current_cycle(period);
    machine.bus.process_events();
    assert!(machine.bus.cpu_stalled(), "active display asserts busreq");

    // A short run while stalled executes no instructions.
    let pc_before = machine.main_cpu.pc();
    machine.run_for(8);
    assert_eq!(
        machine.main_cpu.pc(),
        pc_before,
        "the CPU is held off the bus"
    );

    // Running past the bus-request window resumes execution.
    machine.run_for(period);
    assert_ne!(
        machine.main_cpu.pc(),
        pc_before,
        "the CPU resumes once the bus is released"
    );
}

#[test]
fn crt_blanking_lets_the_cpu_run_more_instructions() {
    let mut crt_on = build_machine(Pc6000Model::Pc6001);
    let mut crt_off = build_machine(Pc6000Model::Pc6001);

    // Blank the CRT on one machine (PPI port C bit 1 reset) so no bus-request
    // stall steals its cycles; the other keeps the display enabled.
    crt_off.bus.io_write(0x93, 0x02);

    // The synthetic ROM is a run of NOPs, so the program counter advances one
    // step per executed instruction and never wraps within this short window.
    let budget = 4_000;
    crt_on.run_for(budget);
    crt_off.run_for(budget);

    assert!(
        crt_off.main_cpu.pc() > crt_on.main_cpu.pc(),
        "blanking the CRT removes the VRAM stall: {} should exceed {}",
        crt_off.main_cpu.pc(),
        crt_on.main_cpu.pc()
    );
}
