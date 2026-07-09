//! Boot-vector tests for the main MC6809.

mod harness;

use common::Cpu6809;
use harness::build_machine_with_synthetic_roms;
use machinefm7::BootMode;

#[test]
fn reset_vector_forces_boot_rom_entry() {
    let machine = build_machine_with_synthetic_roms(BootMode::Basic, |_| {});
    assert_eq!(machine.bus.peek_byte(0xFFFE), 0xFE);
    assert_eq!(machine.bus.peek_byte(0xFFFF), 0x00);
    assert_eq!(machine.main_cpu.pc(), 0xFE00);
}

#[test]
fn boot_stub_executes_and_branches_to_basic_rom() {
    let mut machine = build_machine_with_synthetic_roms(BootMode::Basic, |roms| {
        let boot = roms.boot_bas.as_mut().expect("basic boot ROM exists");
        boot[..3].copy_from_slice(&[0x7E, 0x80, 0x00]);

        // LDA #0x5A ; STA 0x0100 ; BRA $
        roms.fbasic[..7].copy_from_slice(&[0x86, 0x5A, 0xB7, 0x01, 0x00, 0x20, 0xFE]);
    });

    machine.run_for(200);

    assert_eq!(machine.bus.peek_byte(0x0100), 0x5A);
    assert!((0x8000..=0x8006).contains(&machine.main_cpu.pc()));
}
