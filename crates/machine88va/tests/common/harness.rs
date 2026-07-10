//! Shared helpers for the PC-88VA machine integration tests: the synthetic ROM
//! set, the machine factory, and the framebuffer/TSP helpers used by the video,
//! text and sprite tests. Included by each test file with
//! `#[path = "common/harness.rs"] mod harness;`.

#![allow(dead_code)]

use common::Bus;
use machine88va::{LoadedRoms, Pc88VaBus, Pc88VaMachine, Pc88VaModel};

/// Deterministic filler bytes for synthetic ROM images.
pub fn fill(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| {
            seed.wrapping_add(index as u8)
                .wrapping_add((index >> 8) as u8)
                .wrapping_add((index >> 16) as u8)
        })
        .collect()
}

/// Seed of the synthetic font ROM; tests that recompute glyph bytes reuse it.
pub const FONT_SEED: u8 = 0x40;

pub fn synthetic_roms() -> LoadedRoms {
    LoadedRoms {
        rom00: fill(0x10, 0x8_0000),
        rom08: fill(0x20, 0x2_0000),
        rom1: fill(0x30, 0x2_0000),
        font: fill(FONT_SEED, 0x5_0000),
        dictionary: fill(0x50, 0x8_0000),
        subsys: fill(0x60, 0x2000),
    }
}

pub fn machine() -> Pc88VaMachine {
    machine_from_roms(synthetic_roms())
}

/// Builds a reset PC-88VA2 machine from an explicit ROM set.
pub fn machine_from_roms(roms: LoadedRoms) -> Pc88VaMachine {
    let bus = Pc88VaBus::new(Pc88VaModel::PC88VA2, roms, 48_000);
    let sub_cpu = cpu::Z80::new(bus.clock_config().sub_clock_hz);
    Pc88VaMachine::new(Pc88VaMachine::reset_cpu(), sub_cpu, bus)
}

pub const SURFACE_WIDTH: usize = 640;

/// Reads a packed RGBA pixel from the display framebuffer at `(x, y)`.
pub fn pixel(framebuffer: &[u8], x: usize, y: usize) -> u32 {
    let base = (y * SURFACE_WIDTH + x) * 4;
    u32::from(framebuffer[base])
        | (u32::from(framebuffer[base + 1]) << 8)
        | (u32::from(framebuffer[base + 2]) << 16)
        | (u32::from(framebuffer[base + 3]) << 24)
}

/// VA 16-bit color code to packed RGBA, matching the renderer.
pub fn va_rgba(color: u16) -> u32 {
    let level5 = |value: u16| -> u32 {
        let scaled = (value << 3) as u8;
        u32::from(if value != 0 { scaled | 0x07 } else { scaled })
    };
    let level6 = |value: u16| -> u32 {
        let scaled = (value << 2) as u8;
        u32::from(if value != 0 { scaled | 0x03 } else { scaled })
    };
    let green = level6((color & 0xFC00) >> 10);
    let red = level5((color & 0x03E0) >> 5);
    let blue = level5(color & 0x001F);
    red | (green << 8) | (blue << 16) | 0xFF00_0000
}

/// Issues a TSP command with its parameter bytes through ports 0x142 / 0x146.
pub fn tsp_command(machine: &mut Pc88VaMachine, command: u8, params: &[u8]) {
    machine.bus.io_write_byte(0x142, command);
    for &byte in params {
        machine.bus.io_write_byte(0x146, byte);
    }
}

/// Writes a 16-bit value to an I/O port pair (`port`, `port + 1`).
pub fn put_word(machine: &mut Pc88VaMachine, port: u16, value: u16) {
    machine.bus.io_write_byte(port, value as u8);
    machine.bus.io_write_byte(port + 1, (value >> 8) as u8);
}

/// Advances the scheduler until a frame has been rendered (display height set).
pub fn render_one_frame(machine: &mut Pc88VaMachine) {
    for _ in 0..200 {
        let next = machine
            .bus
            .next_event_cycle()
            .expect("an event is always scheduled");
        machine.bus.set_current_cycle(next);
        if machine.display_dimensions().1 != 0 {
            return;
        }
    }
    panic!("no frame rendered");
}
