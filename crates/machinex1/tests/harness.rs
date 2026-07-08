//! Shared test helpers for the machinex1 integration tests.

#![allow(dead_code)]

use machinex1::{LoadedRoms, X1Bus, X1Machine, X1Model};

/// Builds a machine, letting `configure` set up the bus (load ROMs, patch memory)
/// before the CPU is wired in.
pub fn build_machine_with(model: X1Model, configure: impl FnOnce(&mut X1Bus)) -> X1Machine {
    let mut bus = X1Bus::new(model, 48_000);
    configure(&mut bus);
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    X1Machine::new(main_cpu, bus)
}

/// Builds a machine with a synthetic ROM set, after letting `configure` patch in
/// the bytes the test cares about (e.g. a hand-assembled IPL program).
pub fn build_machine_with_synthetic_roms(
    model: X1Model,
    configure: impl FnOnce(&mut LoadedRoms),
) -> X1Machine {
    let mut roms = synthetic_roms(model);
    configure(&mut roms);
    build_machine_with(model, |bus| bus.load_roms(&roms))
}

/// Builds a machine with an unmodified synthetic ROM set.
pub fn build_machine(model: X1Model) -> X1Machine {
    build_machine_with_synthetic_roms(model, |_| {})
}

/// A ROM set of correctly-sized but zeroed banks for `model`, so a bus can be
/// wired with no copyrighted dump. The turbo adds the four kanji ROMs.
pub fn synthetic_roms(model: X1Model) -> LoadedRoms {
    let kanji = match model {
        X1Model::X1 => None,
        X1Model::X1Turbo => Some(vec![0u8; 4 * 0x8000]),
    };
    LoadedRoms {
        model,
        ipl: vec![0u8; model.ipl_rom_size()],
        cgrom_8x8: vec![0u8; 0x0800],
        ank: vec![0u8; 0x2000],
        kanji,
    }
}

/// Advances the bus clock by `cycles`, processing every event that comes due,
/// without running the CPU.
pub fn run_bus_cycles(bus: &mut X1Bus, cycles: u64) {
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
pub fn run_frames(machine: &mut X1Machine, frames: u32) {
    let frame = u64::from(machine.bus.cpu_clock_hz()) / 60;
    for _ in 0..frames {
        machine.run_for(frame);
    }
}

/// Base-X1 framebuffer width in pixels (for the [`pixel`] helper).
pub const FRAMEBUFFER_WIDTH: usize = 640;

/// Reads one RGBA pixel from a packed framebuffer.
pub fn pixel(framebuffer: &[u8], x: usize, y: usize) -> [u8; 4] {
    let index = (y * FRAMEBUFFER_WIDTH + x) * 4;
    framebuffer[index..index + 4].try_into().unwrap()
}

/// Programs the CRTC to the standard 80x25, 8-scanline text geometry.
pub fn program_standard_crtc(bus: &mut X1Bus) {
    let mut set = |register: u8, value: u8| {
        bus.io_write(0x1800, register);
        bus.io_write(0x1801, value);
    };
    set(1, 80); // horizontal displayed
    set(6, 25); // vertical displayed
    set(9, 7); // scanlines per row - 1
    set(4, 24); // vertical total - 1, in character rows: 25 * 8 = 200 lines
    set(5, 0); // vertical total adjust
    set(0, 99); // horizontal total (approximate)
}

/// Programs the CRTC to the turbo 24 kHz hi-res geometry: 80x25 with 16
/// scanlines per row and a vertical total above 400 raster lines.
pub fn program_hires_crtc(bus: &mut X1Bus) {
    let mut set = |register: u8, value: u8| {
        bus.io_write(0x1800, register);
        bus.io_write(0x1801, value);
    };
    set(1, 80); // horizontal displayed
    set(6, 25); // vertical displayed
    set(9, 15); // scanlines per row - 1
    set(4, 25); // vertical total - 1, in character rows: 26 * 16 = 416 lines
    set(5, 0); // vertical total adjust
    set(0, 106); // horizontal total (approximate)
}

/// Advances to the next scheduled event and processes it, returning the
/// acknowledged interrupt vector if one became pending.
pub fn fire_next_event(bus: &mut X1Bus) -> Option<u8> {
    let next = bus.next_event_cycle()?;
    bus.set_current_cycle(next);
    bus.process_events();
    bus.has_irq().then(|| bus.acknowledge_irq())
}
