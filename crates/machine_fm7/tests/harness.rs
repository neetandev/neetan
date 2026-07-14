//! Shared test helpers for the machine_fm7 integration tests.

#![allow(dead_code)]

use machine_fm7::{BootMode, Fm7Bus, Fm7Machine, Fm7Model, LoadedRoms};

/// Builds a machine with synthetic ROMs after letting `configure` patch them.
pub fn build_machine_with_synthetic_roms(
    boot_mode: BootMode,
    configure: impl FnOnce(&mut LoadedRoms),
) -> Fm7Machine {
    let mut roms = synthetic_roms(Fm7Model::Fm7);
    configure(&mut roms);
    build_machine_with_roms(boot_mode, &roms)
}

/// Builds a machine from an existing ROM set.
pub fn build_machine_with_roms(boot_mode: BootMode, roms: &LoadedRoms) -> Fm7Machine {
    let mut bus = Fm7Bus::new(Fm7Model::Fm7, boot_mode, 48_000);
    bus.load_roms(roms);
    let main_cpu = cpu::M6809::new(bus.cpu_clock_hz());
    let sub_cpu = cpu::M6809::new(Fm7Model::Fm7.sub_clock_hz());
    Fm7Machine::new(main_cpu, sub_cpu, bus)
}

/// Builds a bus with synthetic ROMs after letting `configure` patch them.
pub fn build_bus_with_synthetic_roms(
    boot_mode: BootMode,
    configure: impl FnOnce(&mut LoadedRoms),
) -> Fm7Bus {
    let mut roms = synthetic_roms(Fm7Model::Fm7);
    configure(&mut roms);
    let mut bus = Fm7Bus::new(Fm7Model::Fm7, boot_mode, 48_000);
    bus.load_roms(&roms);
    bus
}

/// Builds an FM-77AV bus with synthetic ROMs after letting `configure` patch
/// them.
pub fn build_av_bus_with_synthetic_roms(
    boot_mode: BootMode,
    configure: impl FnOnce(&mut LoadedRoms),
) -> Fm7Bus {
    let mut roms = synthetic_roms(Fm7Model::Fm77Av);
    configure(&mut roms);
    let mut bus = Fm7Bus::new(Fm7Model::Fm77Av, boot_mode, 48_000);
    bus.load_roms(&roms);
    bus
}

/// Builds an FM-77AV machine from an existing ROM set.
pub fn build_av_machine_with_roms(boot_mode: BootMode, roms: &LoadedRoms) -> Fm7Machine {
    let mut bus = Fm7Bus::new(Fm7Model::Fm77Av, boot_mode, 48_000);
    bus.load_roms(roms);
    let main_cpu = cpu::M6809::new(bus.cpu_clock_hz());
    let sub_cpu = cpu::M6809::new(Fm7Model::Fm77Av.sub_clock_hz());
    Fm7Machine::new(main_cpu, sub_cpu, bus)
}

/// Builds an FM-77AV machine with synthetic ROMs after letting `configure` patch
/// them.
pub fn build_av_machine_with_synthetic_roms(
    boot_mode: BootMode,
    configure: impl FnOnce(&mut LoadedRoms),
) -> Fm7Machine {
    let mut roms = synthetic_roms(Fm7Model::Fm77Av);
    configure(&mut roms);
    build_av_machine_with_roms(boot_mode, &roms)
}

/// A correctly sized but zero-filled ROM set for `model`.
pub fn synthetic_roms(model: Fm7Model) -> LoadedRoms {
    let kanji = match model {
        Fm7Model::Fm7 => None,
        Fm7Model::Fm77Av => Some(vec![0u8; 0x2_0000]),
    };
    let (boot_bas, boot_dos, initiate, subsys_a, subsys_b, subsyscg) = match model {
        Fm7Model::Fm7 => (
            Some(vec![0u8; 0x0200]),
            Some(vec![0u8; 0x0200]),
            None,
            None,
            None,
            None,
        ),
        Fm7Model::Fm77Av => (
            None,
            None,
            Some(vec![0u8; 0x2000]),
            Some(vec![0u8; 0x2000]),
            Some(vec![0u8; 0x2000]),
            Some(vec![0u8; 0x2000]),
        ),
    };
    LoadedRoms {
        model,
        fbasic: vec![0u8; 0x7C00],
        subsys_c: vec![0u8; 0x2800],
        kanji,
        boot_bas,
        boot_dos,
        initiate,
        subsys_a,
        subsys_b,
        subsyscg,
    }
}

/// Patches the BASIC boot ROM so the main CPU branches to itself from reset,
/// keeping it out of the way while a test drives the sub CPU and the handshake.
pub fn park_main_cpu(roms: &mut LoadedRoms) {
    let boot = roms.boot_bas.as_mut().expect("basic boot ROM present");
    // BRA $ (branch to self) at the reset entry 0xFE00.
    boot[0] = 0x20;
    boot[1] = 0xFE;
}

/// Patches the FM-77AV initiator ROM so the main CPU branches to itself from
/// reset, keeping it out of the way while a test drives the sub CPU. The main
/// CPU boots through the initiator overlay at `0x6000-0x7FFF`, so the reset vector
/// is pointed at `0x6000` where a branch-to-self spins.
pub fn park_main_cpu_av(roms: &mut LoadedRoms) {
    let initiator = roms.initiate.as_mut().expect("AV initiator ROM present");
    // BRA $ at 0x6000 (initiator overlay offset 0x0000).
    initiator[0] = 0x20;
    initiator[1] = 0xFE;
    // Reset vector at 0xFFFE (initiator offset 0x1FFE) -> 0x6000.
    initiator[0x1FFE] = 0x60;
    initiator[0x1FFF] = 0x00;
}

/// Patches the sub-monitor ROM so the sub CPU branches to itself from reset,
/// spinning harmlessly. It never loads its stack, so NMI stays disarmed and the
/// sub keeps spinning across the periodic display NMI.
pub fn park_sub_cpu(roms: &mut LoadedRoms) {
    let rom = &mut roms.subsys_c;
    // BRA $ at 0xE000 (offset 0x0800).
    let program_offset = 0x0800;
    rom[program_offset] = 0x20;
    rom[program_offset + 1] = 0xFE;
    // Reset vector at 0xFFFE (offset 0x27FE) -> 0xE000.
    let reset_vector_offset = 0x27FE;
    rom[reset_vector_offset] = 0xE0;
    rom[reset_vector_offset + 1] = 0x00;
}

/// Advances the bus clock by `cycles`, processing every event that comes due.
pub fn run_bus_cycles(bus: &mut Fm7Bus, cycles: u64) {
    let end = bus.current_cycle() + cycles;
    loop {
        let next = bus.next_event_cycle().unwrap_or(end).min(end);
        bus.set_current_cycle(next);
        bus.process_events();
        if next >= end {
            break;
        }
    }
}
