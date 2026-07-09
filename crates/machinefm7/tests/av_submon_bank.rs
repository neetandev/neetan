//! FM-77AV sub-monitor bank switching (0xFD13) tests.

mod harness;

use common::{Bus, Cpu6809};
use harness::{
    build_av_bus_with_synthetic_roms, build_av_machine_with_synthetic_roms,
    build_bus_with_synthetic_roms,
};
use machinefm7::{BootMode, SubBusView};

/// Sub-monitor ROM offset of the byte visible at sub `0xE000` in bank C.
const BANK_C_E000_OFFSET: usize = 0x0800;
/// Sub-monitor ROM offset of the reset vector in the type-C monitor.
const BANK_C_RESET_VECTOR: usize = 0x27FE;
/// Type-A/B/CG monitor ROM offset of the reset vector.
const ALT_RESET_VECTOR: usize = 0x1FFE;

/// CG ROM offset of the first byte of window bank 2.
const CG_BANK2_OFFSET: usize = 0x1000;

#[test]
fn fd13_switches_the_banked_monitor_and_cg_window() {
    let mut bus = build_av_bus_with_synthetic_roms(BootMode::Basic, |roms| {
        roms.subsys_c[BANK_C_E000_OFFSET] = 0xC0;
        roms.subsys_a.as_mut().unwrap()[0] = 0xA0;
        roms.subsys_b.as_mut().unwrap()[0] = 0xB0;
        let subsyscg = roms.subsyscg.as_mut().unwrap();
        subsyscg[0] = 0xCE;
        subsyscg[CG_BANK2_OFFSET] = 0xC2;
    });

    // Bank C (default): the type-C monitor at 0xE000, the CG window at 0xD800.
    assert_eq!(bus.sub_monitor_bank(), 0);
    assert_eq!(bus.sub_peek_byte(0xE000), 0xC0);
    assert_eq!(bus.sub_peek_byte(0xD800), 0xCE);

    // Bank A exposes the type-A monitor at 0xE000; the CG window stays.
    bus.write_byte(0xFD13, 1);
    assert_eq!(bus.sub_monitor_bank(), 1);
    assert_eq!(bus.sub_peek_byte(0xE000), 0xA0);
    assert_eq!(bus.sub_peek_byte(0xD800), 0xCE);

    // 0xD430 bits 1-0 select the CG ROM bank shown in the window.
    {
        let mut view = SubBusView { bus: &mut bus };
        view.write_byte(0xD430, 0x02);
    }
    assert_eq!(bus.sub_peek_byte(0xD800), 0xC2);

    // Bank B selects the type-B monitor.
    bus.write_byte(0xFD13, 2);
    assert_eq!(bus.sub_peek_byte(0xE000), 0xB0);

    // Returning to bank C restores the type-C monitor.
    bus.write_byte(0xFD13, 0);
    assert_eq!(bus.sub_peek_byte(0xE000), 0xC0);
}

#[test]
fn hidden_ram_is_av_only() {
    let mut av_bus = build_av_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    {
        let mut view = SubBusView { bus: &mut av_bus };
        view.write_byte(0xD500, 0x12);
        view.write_byte(0xD7FF, 0x34);
        assert_eq!(view.read_byte(0xD500), 0x12);
        assert_eq!(view.read_byte(0xD7FF), 0x34);
    }

    // The FM-7 decodes the whole 0xD400-0xD7FF region as I/O instead, so 0xD500
    // mirrors the 0xD400 port rather than storing RAM.
    let mut fm7_bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    let mut view = SubBusView { bus: &mut fm7_bus };
    view.write_byte(0xD500, 0x12);
    let mirrored = view.read_byte(0xD400);
    assert_eq!(view.read_byte(0xD500), mirrored);
}

#[test]
fn fd13_pulse_resets_the_sub_cpu_into_the_new_bank() {
    let mut machine = build_av_machine_with_synthetic_roms(BootMode::Basic, |roms| {
        harness::park_main_cpu_av(roms);

        // Bank C: BRA $ at 0xE000, reset vector -> 0xE000.
        roms.subsys_c[BANK_C_E000_OFFSET] = 0x20;
        roms.subsys_c[BANK_C_E000_OFFSET + 1] = 0xFE;
        roms.subsys_c[BANK_C_RESET_VECTOR] = 0xE0;
        roms.subsys_c[BANK_C_RESET_VECTOR + 1] = 0x00;

        // Bank A: BRA $ at 0xE100, reset vector -> 0xE100.
        let bank_a = roms.subsys_a.as_mut().unwrap();
        bank_a[0x0100] = 0x20;
        bank_a[0x0101] = 0xFE;
        bank_a[ALT_RESET_VECTOR] = 0xE1;
        bank_a[ALT_RESET_VECTOR + 1] = 0x00;
    });

    machine.run_for(2_000);
    assert_eq!(machine.sub_cpu.pc(), 0xE000);

    // Switching the sub-monitor bank pulse-resets the sub CPU into bank A.
    machine.bus.write_byte(0xFD13, 1);
    machine.run_for(2_000);
    assert_eq!(machine.sub_cpu.pc(), 0xE100);
}
