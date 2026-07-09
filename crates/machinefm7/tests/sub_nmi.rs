//! Periodic display NMI delivered to the sub CPU.

mod harness;

use harness::build_machine_with_synthetic_roms;
use machinefm7::BootMode;

/// Sub work-RAM address the NMI handler increments on each interrupt.
const NMI_COUNTER: u16 = 0xD010;

#[test]
fn periodic_nmi_reaches_the_sub_cpu_handler() {
    let mut machine = build_machine_with_synthetic_roms(BootMode::Basic, |roms| {
        harness::park_main_cpu(roms);

        let rom = &mut roms.subsys_c;
        // Program at 0xE000: LDS #0xD000 (arms NMI) ; BRA $.
        rom[0x0800..0x0806].copy_from_slice(&[0x10, 0xCE, 0xD0, 0x00, 0x20, 0xFE]);
        // NMI handler at 0xE020: INC 0xD010 ; RTI.
        rom[0x0820..0x0824].copy_from_slice(&[0x7C, 0xD0, 0x10, 0x3B]);
        // Reset vector -> 0xE000, NMI vector -> 0xE020.
        rom[0x27FE] = 0xE0;
        rom[0x27FF] = 0x00;
        rom[0x27FC] = 0xE0;
        rom[0x27FD] = 0x20;
    });

    // A single 20 ms period is about 35_960 main cycles; run past one.
    machine.run_for(40_000);
    let first = machine.bus.sub_peek_byte(NMI_COUNTER);
    assert!(first >= 1, "the sub CPU serviced at least one display NMI");

    machine.run_for(80_000);
    let second = machine.bus.sub_peek_byte(NMI_COUNTER);
    assert!(second > first, "further periods deliver further NMIs");
}
