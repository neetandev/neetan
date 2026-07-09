//! Shared-RAM aliasing between the main `0xFC80-0xFCFF` window and the sub
//! `0xD380-0xD3FF` window while the sub CPU is halted.

mod harness;

use harness::build_machine_with_synthetic_roms;
use machinefm7::BootMode;

/// `0xFD05` write bit requesting the sub CPU halt.
const FD05_HALT: u8 = 0x80;

#[test]
fn window_aliases_sub_shared_ram_only_while_halted() {
    let mut machine = build_machine_with_synthetic_roms(BootMode::Basic, |roms| {
        harness::park_main_cpu(roms);
        harness::park_sub_cpu(roms);
    });

    machine.bus.write_byte(0xFD05, FD05_HALT);
    machine.run_for(400);
    assert!(machine.bus.is_sub_halted());

    // Main writes at both ends of the window land in sub shared RAM.
    machine.bus.write_byte(0xFC80, 0xAB);
    machine.bus.write_byte(0xFCFF, 0xCD);
    assert_eq!(machine.bus.sub_peek_byte(0xD380), 0xAB);
    assert_eq!(machine.bus.sub_peek_byte(0xD3FF), 0xCD);

    // A sub-side write is visible through the main window at the aliased offset.
    machine.bus.sub_poke_byte(0xD390, 0x5A);
    assert_eq!(machine.bus.read_byte(0xFC90), 0x5A);

    // Releasing HALT closes the window: reads float and writes are dropped.
    machine.bus.write_byte(0xFD05, 0x00);
    machine.run_for(400);
    assert!(!machine.bus.is_sub_halted());
    assert_eq!(machine.bus.read_byte(0xFC80), 0xFF);
    machine.bus.write_byte(0xFC80, 0x99);
    assert_eq!(machine.bus.sub_peek_byte(0xD380), 0xAB);
}
