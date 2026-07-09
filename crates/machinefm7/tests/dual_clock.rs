//! Sub/main clock ratio and the VRAM-access contention divisor.

mod harness;

use common::Bus;
use harness::build_machine_with_synthetic_roms;
use machinefm7::{BootMode, SubBusView};

/// Base sub clock in Hz (fast mode).
const SUB_CLOCK_HZ: f64 = 2_000_000.0;
/// Main clock in Hz (fast mode).
const MAIN_CLOCK_HZ: f64 = 1_798_000.0;
/// Sub clock divisor applied while the sub contends for VRAM without cycle steal.
const CONTENTION_DIVISOR: f64 = 3.0;

#[test]
fn sub_and_main_advance_at_their_clock_ratio() {
    let mut machine = build_machine_with_synthetic_roms(BootMode::Basic, |roms| {
        harness::park_main_cpu(roms);
        harness::park_sub_cpu(roms);
    });

    machine.run_for(1_000_000);
    let main_cycles = machine.bus.current_cycle() as f64;
    let sub_cycles = machine.bus.sub_cycle() as f64;

    let ratio = sub_cycles / main_cycles;
    let expected = SUB_CLOCK_HZ / MAIN_CLOCK_HZ;
    assert!(
        (ratio - expected).abs() < 0.005,
        "sub/main ratio {ratio} should be near {expected}"
    );
}

#[test]
fn vram_access_divides_the_sub_clock_by_three() {
    let mut machine = build_machine_with_synthetic_roms(BootMode::Basic, |roms| {
        harness::park_main_cpu(roms);
        harness::park_sub_cpu(roms);
    });

    machine.run_for(50_000);

    // A sub read of 0xD409 sets the VRAM access flag; with cycle steal off on the
    // FM-7, the sub clock drops to a third.
    {
        let mut view = SubBusView {
            bus: &mut machine.bus,
        };
        view.read_byte(0xD409);
    }

    let main_start = machine.bus.current_cycle();
    let sub_start = machine.bus.sub_cycle();
    machine.run_for(600_000);
    let main_delta = (machine.bus.current_cycle() - main_start) as f64;
    let sub_delta = (machine.bus.sub_cycle() - sub_start) as f64;

    let ratio = sub_delta / main_delta;
    let expected = (SUB_CLOCK_HZ / CONTENTION_DIVISOR) / MAIN_CLOCK_HZ;
    assert!(
        (ratio - expected).abs() < 0.01,
        "contended sub/main ratio {ratio} should be near {expected}"
    );
}
