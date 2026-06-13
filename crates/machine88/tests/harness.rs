//! Shared test helpers for the machine88 integration tests.

#![allow(dead_code)]

use machine88::{ClockSelect, LoadedRoms, Pc8801Bus, Pc8801Machine, Pc8801Model};

/// Builds a PC-8801 machine, letting `configure` set up the bus (load ROMs)
/// before the CPU is wired in.
pub fn build_machine_with(configure: impl FnOnce(&mut Pc8801Bus)) -> Pc8801Machine {
    let mut bus = Pc8801Bus::new(Pc8801Model::PC8801MC, ClockSelect::FourMhz, 48_000);
    // These tests exercise the N88/disk boot path; disable the CD-ROM BIOS bank
    // that the MC otherwise maps over 0x0000-0x7FFF at reset.
    bus.set_cdrom_bios_bank(false);
    configure(&mut bus);
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    let sub_cpu = cpu::Z80::new(bus.sub_clock_hz());
    Pc8801Machine::new(main_cpu, sub_cpu, bus)
}

/// Builds a machine with a hand-assembled main ROM image.
pub fn build_machine_with_rom(rom: &[u8]) -> Pc8801Machine {
    build_machine_with(|bus| bus.load_main_rom(rom))
}

/// Builds a machine with a full loaded ROM set.
pub fn build_machine_with_roms(roms: &LoadedRoms) -> Pc8801Machine {
    build_machine_with(|bus| bus.load_roms(roms))
}

/// A ROM set of correctly-sized but zeroed banks, so the bus can be wired up
/// without any copyrighted dump. Tests patch individual fields with synthetic
/// content (a font glyph, a kanji pattern, ...) before loading it.
pub fn synthetic_roms() -> LoadedRoms {
    LoadedRoms {
        n88: vec![0u8; 0x8000],
        n88_ext: [
            vec![0u8; 0x2000],
            vec![0u8; 0x2000],
            vec![0u8; 0x2000],
            vec![0u8; 0x2000],
        ],
        n_basic: vec![0u8; 0x8000],
        n80_mkii: None,
        n80_mkiisr: None,
        n80sr: None,
        dictionary: vec![0u8; 0x8_0000],
        kanji1: vec![0u8; 0x2_0000],
        kanji2: vec![0u8; 0x2_0000],
        disk: vec![0u8; 0x2000],
        cdrom_bios: vec![0u8; 0x1_0000],
    }
}

/// Builds a machine from a `synthetic_roms()` set after letting `configure`
/// patch in the bytes the test cares about.
pub fn build_machine_with_synthetic_roms(configure: impl FnOnce(&mut LoadedRoms)) -> Pc8801Machine {
    let mut roms = synthetic_roms();
    configure(&mut roms);
    build_machine_with_roms(&roms)
}
