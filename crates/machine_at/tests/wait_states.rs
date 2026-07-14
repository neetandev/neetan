//! Tests for the ISA wait-state charging. Each access class charges the
//! configured penalty, and the penalties scale with the core clock so device
//! wall-clock timing stays consistent across the two variants.

use common::{Bus, NoTrace};
use machine_at::{AtBus, AtModel, LoadedRoms};

/// Builds a bus for `model` with placeholder ROMs.
fn bus_for(model: AtModel) -> AtBus<NoTrace> {
    let roms = LoadedRoms {
        system_bios: vec![0xFF; 0x1_0000],
        vga_bios: vec![0xFF; 0x8000],
    };
    AtBus::<NoTrace>::new(
        model.cpu_clock_hz(common::CpuMode::High),
        model.ram_size(),
        roms,
        48_000,
    )
}

#[test]
fn each_access_class_charges_its_penalty() {
    let mut bus = bus_for(AtModel::At486Dx50);
    let config = bus.clock_config();
    assert!(config.io_8bit_wait_cycles > 0);

    // An 8-bit I/O access.
    bus.io_read_byte(0x0060);
    assert_eq!(bus.drain_wait_cycles(), config.io_8bit_wait_cycles);

    // A 16-bit I/O access to the IDE data register.
    bus.io_read_word(0x01F0);
    assert_eq!(bus.drain_wait_cycles(), config.io_16bit_wait_cycles);

    // A VGA VRAM window access.
    bus.read_byte(0x000A_0000);
    assert_eq!(bus.drain_wait_cycles(), config.vga_memory_wait_cycles);

    // Cached DRAM access carries no wait (the i486 cache hides it).
    bus.read_byte(0x0000_1000);
    assert_eq!(bus.drain_wait_cycles(), 0);
}

#[test]
fn penalties_scale_with_the_core_clock() {
    let slow = bus_for(AtModel::At486Dx50).clock_config();
    let fast = bus_for(AtModel::At486Dx66).clock_config();

    // The faster core spends more core cycles on the same wall-clock ISA access.
    assert!(fast.io_8bit_wait_cycles > slow.io_8bit_wait_cycles);
    assert!(fast.io_16bit_wait_cycles > slow.io_16bit_wait_cycles);
    assert!(fast.vga_memory_wait_cycles > slow.vga_memory_wait_cycles);

    // The ratio tracks the clock ratio (66 / 50).
    let ratio = fast.io_8bit_wait_cycles as f64 / slow.io_8bit_wait_cycles as f64;
    assert!((ratio - 66.0 / 50.0).abs() < 0.05);
}
