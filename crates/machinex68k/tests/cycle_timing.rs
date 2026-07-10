//! Full-machine timing calibration tests: per-region CPU wait states measured
//! through scripted read loops, XVI high/low clock behavior, and the frame
//! duration of a programmed standard 31 kHz mode.

#[path = "common/harness.rs"]
mod harness;

use common::{Bus, CpuM68000, CpuMode};
use harness::{
    STOP_MASKED, machine, machine_from_roms, read_byte, run_until_stop, scripted_roms, write_word,
};
use machinex68k::{X68kMachine, X68kModel};

/// Read-loop iterations per timing probe.
const PROBE_ITERATIONS: u64 = 64;

/// Assembles the timing probe: a `tst.b (address).l` loop of
/// `PROBE_ITERATIONS` iterations followed by a masked stop.
fn probe_program(address: u32) -> Vec<u16> {
    // moveq #(iterations - 1), d0
    let mut program = vec![0x7000 | (PROBE_ITERATIONS as u16 - 1)];
    program.extend([0x4A39, (address >> 16) as u16, address as u16]);
    // dbra d0, back to the tst.b
    program.extend([0x51C8, 0xFFF8]);
    program.extend(STOP_MASKED);
    program
}

/// Runs the probe loop against `address` and returns the consumed cycles.
fn probe_cycles(model: X68kModel, cpu_mode: CpuMode, address: u32) -> u64 {
    let roms = scripted_roms(model, &probe_program(address));
    let mut machine: X68kMachine = machine_from_roms(model, cpu_mode, roms);
    run_until_stop(&mut machine, 10_000)
}

#[test]
fn per_region_read_waits_match_the_calibrated_penalty_table() {
    let baseline = probe_cycles(X68kModel::X68000, CpuMode::High, 0x8000);
    for (name, address, wait) in [
        ("GVRAM", 0x00C0_0000u32, 1u64),
        ("IPL ROM", 0x00FE_0000, 1),
        ("TVRAM", 0x00E0_0000, 2),
        ("OPM", 0x00E9_0003, 2),
        ("palette", 0x00E8_2000, 3),
        ("MFP", 0x00E8_8001, 4),
        ("SCC", 0x00E9_8001, 6),
        ("DMAC", 0x00E8_4000, 15),
    ] {
        let cycles = probe_cycles(X68kModel::X68000, CpuMode::High, address);
        let delta = cycles - baseline;
        let expected = PROBE_ITERATIONS * wait;
        // The RAM baseline pays one DRAM refresh cycle per eight reads that
        // the register loop does not, so the delta sits just below the
        // whole-cycle product.
        assert!(
            (expected - 16..=expected).contains(&delta),
            "{name}: expected a wait delta near {expected}, got {delta}"
        );
    }
}

#[test]
fn xvi_speed_modes_share_cycle_counts_and_differ_in_clock() {
    let high = probe_cycles(X68kModel::X68000Xvi, CpuMode::High, 0x00E8_8001);
    let low = probe_cycles(X68kModel::X68000Xvi, CpuMode::Low, 0x00E8_8001);
    assert_eq!(high, low, "cycle counts are clock-independent");

    let roms_high = scripted_roms(X68kModel::X68000Xvi, &STOP_MASKED);
    let machine_high: X68kMachine =
        machine_from_roms(X68kModel::X68000Xvi, CpuMode::High, roms_high);
    assert_eq!(machine_high.cpu.clock_hz(), 16_666_667);
    let roms_low = scripted_roms(X68kModel::X68000Xvi, &STOP_MASKED);
    let machine_low: X68kMachine = machine_from_roms(X68kModel::X68000Xvi, CpuMode::Low, roms_low);
    assert_eq!(machine_low.cpu.clock_hz(), 10_000_000);
}

/// Programs CRTC R00-R09 and R20, then measures the cycle distance between
/// two rising edges of the MFP GPIP4 vertical-display signal.
fn frame_duration_cycles(registers: [u16; 10], memory_mode: u16) -> u64 {
    let mut machine = machine(X68kModel::X68000);
    for (index, value) in registers.into_iter().enumerate() {
        write_word(&mut machine, 0xE80000 + index as u32 * 2, value);
    }
    write_word(&mut machine, 0xE80028, memory_mode);

    let mut edges = Vec::new();
    let mut previous = read_byte(&mut machine, 0xE88001) & 0x10;
    for _ in 0..20_000 {
        machine.run_for(50);
        let current = read_byte(&mut machine, 0xE88001) & 0x10;
        if previous == 0 && current != 0 {
            edges.push(machine.bus.current_cycle());
            if edges.len() == 2 {
                break;
            }
        }
        previous = current;
    }
    assert_eq!(edges.len(), 2, "two vertical-display edges must arrive");
    edges[1] - edges[0]
}

/// The standard 768x512 mode with a caller-selected horizontal total: 138 x
/// 568 characters of 8 dots on the 69.551900 MHz oscillator divided by two.
fn standard_31khz_registers(horizontal_total: u16) -> [u16; 10] {
    [horizontal_total, 14, 28, 124, 567, 5, 40, 552, 27, 100]
}

#[test]
fn standard_31khz_frame_duration_follows_the_oscillator() {
    // 1104 x 568 dots at 34.775950 MHz is 18.0318 ms: 180,318 CPU cycles
    // at 10 MHz, within the coarse sampling step.
    let frame_cycles = frame_duration_cycles(standard_31khz_registers(137), 0x0016);
    assert!(
        (180_100..=180_600).contains(&frame_cycles),
        "expected a frame near 180318 cycles, got {frame_cycles}"
    );
}

#[test]
fn standard_15khz_frame_duration_follows_the_oscillator() {
    // The standard 512x512 low-resolution mode: 76 x 260 characters of 8
    // dots on the 38.863632 MHz oscillator divided by four. 608 x 260 dots
    // at 9.715908 MHz is 16.2702 ms: 162,702 CPU cycles at 10 MHz.
    let frame_cycles = frame_duration_cycles([75, 3, 5, 69, 259, 2, 16, 256, 27, 100], 0x0005);
    assert!(
        (162_500..=163_000).contains(&frame_cycles),
        "expected a frame near 162702 cycles, got {frame_cycles}"
    );
}

#[test]
fn crtc_r00_bit_zero_is_wired_to_one_in_the_frame_timing() {
    // Writing 136 or 137 to R00 selects the same 138-character line because
    // bit 0 reads as 1, so both frames must measure identically; 139 selects
    // a 140-character line: 1120 x 568 dots at 34.775950 MHz is 18.2931 ms.
    let even = frame_duration_cycles(standard_31khz_registers(136), 0x0016);
    let odd = frame_duration_cycles(standard_31khz_registers(137), 0x0016);
    assert_eq!(even, odd, "R00 bit 0 must be forced to 1");
    let longer = frame_duration_cycles(standard_31khz_registers(139), 0x0016);
    assert!(
        (182_700..=183_200).contains(&longer),
        "expected a frame near 182931 cycles, got {longer}"
    );
}
