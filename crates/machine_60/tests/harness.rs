//! Shared test helpers for the machine_60 integration tests.

#![allow(dead_code)]

use device::floppy::{D88Disk, D88MediaType, D88Sector, FloppyImage};
use machine_60::{LoadedRoms, Pc6000Bus, Pc6000Machine, Pc6000Model};

/// Key-release flag ORed into a scancode to signal a key-up.
pub const RELEASE_FLAG: u8 = 0x80;

/// Builds a machine, letting `configure` set up the bus (load ROMs, mount media)
/// before the CPU is wired in.
pub fn build_machine_with(
    model: Pc6000Model,
    configure: impl FnOnce(&mut Pc6000Bus),
) -> Pc6000Machine {
    let mut bus = Pc6000Bus::new(model, 48_000);
    configure(&mut bus);
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    Pc6000Machine::new(main_cpu, bus)
}

/// Builds a machine with a synthetic ROM set, after letting `configure` patch in
/// the bytes the test cares about (a font glyph, a system-ROM opcode, ...).
pub fn build_machine_with_synthetic_roms(
    model: Pc6000Model,
    configure: impl FnOnce(&mut LoadedRoms),
) -> Pc6000Machine {
    let mut roms = synthetic_roms(model);
    configure(&mut roms);
    build_machine_with(model, |bus| bus.load_roms(&roms))
}

/// Builds a machine with an unmodified synthetic ROM set.
pub fn build_machine(model: Pc6000Model) -> Pc6000Machine {
    build_machine_with_synthetic_roms(model, |_| {})
}

/// A ROM set of correctly-sized but zeroed banks for `model`, so a bus can be
/// wired with no copyrighted dump. Absent roles stay `None`.
pub fn synthetic_roms(model: Pc6000Model) -> LoadedRoms {
    let mut roms = LoadedRoms {
        model,
        basic: None,
        system_rom1: None,
        system_rom2: None,
        sub_rom: None,
        cg_base: None,
        cg_ext: None,
        cg_sr: None,
        kanji: None,
        voice: None,
    };
    match model {
        Pc6000Model::Pc6001 => {
            roms.basic = Some(vec![0u8; 0x4000]);
            roms.cg_base = Some(vec![0u8; 0x1000]);
        }
        Pc6000Model::Pc6001Mk2 | Pc6000Model::Pc6601 => {
            roms.basic = Some(vec![0u8; 0x8000]);
            roms.cg_base = Some(vec![0u8; 0x2000]);
            roms.cg_ext = Some(vec![0u8; 0x2000]);
            roms.kanji = Some(vec![0u8; 0x8000]);
            roms.voice = Some(vec![0u8; 0x4000]);
        }
        Pc6000Model::Pc6001Mk2Sr => {
            roms.system_rom1 = Some(vec![0u8; 0x1_0000]);
            roms.system_rom2 = Some(vec![0u8; 0x1_0000]);
            roms.cg_sr = Some(vec![0u8; 0x4000]);
        }
        Pc6000Model::Pc6601Sr => {
            roms.system_rom1 = Some(vec![0u8; 0x1_0000]);
            roms.system_rom2 = Some(vec![0u8; 0x1_0000]);
            roms.cg_sr = Some(vec![0u8; 0x4000]);
            roms.basic = Some(vec![0u8; 0x8000]);
            roms.sub_rom = Some(vec![0u8; 0x2000]);
            roms.cg_base = Some(vec![0u8; 0x2000]);
            roms.cg_ext = Some(vec![0u8; 0x2000]);
            roms.kanji = Some(vec![0u8; 0x8000]);
            roms.voice = Some(vec![0u8; 0x4000]);
        }
    }
    roms
}

/// Advances the bus clock by `cycles`, processing every event that comes due so
/// the frame interrupt renders and timers fire, without running the CPU.
pub fn run_bus_cycles(bus: &mut Pc6000Bus, cycles: u64) {
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

/// Runs the machine for `frames` worth of main-clock cycles.
pub fn run_frames(machine: &mut Pc6000Machine, frames: u32) {
    let frame = u64::from(machine.bus.cpu_clock_hz()) / 60;
    for _ in 0..frames {
        machine.run_for(frame);
    }
}

/// Advances to the next scheduled event and processes it, returning the
/// acknowledged interrupt vector if one became pending.
pub fn fire_next_event(bus: &mut Pc6000Bus) -> Option<u8> {
    let next = bus.next_event_cycle()?;
    bus.set_current_cycle(next);
    bus.process_events();
    bus.has_irq().then(|| bus.acknowledge_irq())
}

/// Builds a 256-byte sector whose data ramps from `first_value`.
pub fn make_sector(record: u8, sector_count: u16, first_value: u8) -> D88Sector {
    D88Sector {
        cylinder: 0,
        head: 0,
        record,
        size_code: 1,
        sector_count,
        mfm_flag: 0x00,
        deleted: 0x00,
        status: 0x00,
        reserved: [0; 5],
        data: (0..256)
            .map(|index| first_value.wrapping_add(index as u8))
            .collect(),
        source_offset: None,
    }
}

/// Builds an in-memory single-track D88 image from `sectors`.
pub fn synthetic_d88(name: &str, media: D88MediaType, sectors: Vec<D88Sector>) -> FloppyImage {
    FloppyImage::from_d88(D88Disk::from_tracks(
        String::from(name),
        false,
        media,
        vec![Some(sectors)],
    ))
}
