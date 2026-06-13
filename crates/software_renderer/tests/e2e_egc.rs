//! End-to-end tests for the EGC (Enhanced Graphic Charger) path on a PC-9801VX.
//!
//! Each test boots the `debug_egc.asm` ROM on a PC-9801VX (I286 CPU,
//! EGC) with a different mode-selector byte and asserts the framebuffer
//! matches the expected quadrant pattern. This exercises the EGC FGC/BGC
//! fill, CPU broadcast with plane access mask, and ROP block copy paths
//! through CPU -> bus I/O -> EGC -> compose -> framebuffer bytes, using
//! the standard 16-color analog palette programmed by the ROM.

use common::{BUILTIN_FONT_ROM, Bus, Cpu, CpuMode, MachineModel};
use machine::{Pc9801Bus, Pc9801Vx};

const DEBUG_EGC_ROM: &[u8] = include_bytes!("../../../utils/debug/debug_egc.rom");

const MODE_BYTE_ADDR: u32 = 0x0500;
const RUN_BUDGET_CYCLES: u64 = 50_000_000;

const FB_WIDTH: u32 = 640;
const FB_HEIGHT_MAX: u32 = 480;

fn run_mode(mode: u8) -> (Vec<u8>, u32) {
    let bus = Pc9801Bus::new(MachineModel::PC9801VX, CpuMode::High, 48000);

    let mut machine = Pc9801Vx::new(cpu::I286::new(), bus);
    machine.bus.load_font_rom(BUILTIN_FONT_ROM);
    machine.bus.load_bios_rom(DEBUG_EGC_ROM);
    machine.bus.write_byte(MODE_BYTE_ADDR, mode);

    let _ = machine.run_for(RUN_BUDGET_CYCLES);

    assert!(
        machine.cpu.halted(),
        "ROM did not reach HLT for mode {mode}",
    );

    let framebuffer = machine.bus.display_framebuffer().to_vec();
    let (_, height) = machine.bus.display_dimensions();
    assert!(
        framebuffer.len() >= (FB_WIDTH * FB_HEIGHT_MAX * 4) as usize,
        "unexpected framebuffer length for mode {mode}",
    );
    (framebuffer, height)
}

fn pixel(framebuffer: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * FB_WIDTH + x) * 4) as usize;
    [framebuffer[i], framebuffer[i + 1], framebuffer[i + 2]]
}

fn assert_pixel(framebuffer: &[u8], x: u32, y: u32, expected: [u8; 3], context: &str) {
    let got = pixel(framebuffer, x, y);
    assert_eq!(
        got, expected,
        "{context}: pixel ({x},{y}) expected RGB {expected:?}, got {got:?}",
    );
}

/// 16-color analog palette programmed by the EGC debug ROM. Matches the
/// table in `e2e_grcg.rs` since both ROMs install the same standard
/// PC-98 colors via `set_palette`.
fn palette_rgb(index: u8) -> [u8; 3] {
    const TABLE_GRB: [[u8; 3]; 16] = [
        [0x00, 0x00, 0x00], // 0  Black
        [0x00, 0x00, 0x07], // 1  Blue
        [0x00, 0x07, 0x00], // 2  Red
        [0x00, 0x07, 0x07], // 3  Magenta
        [0x07, 0x00, 0x00], // 4  Green
        [0x07, 0x00, 0x07], // 5  Cyan
        [0x07, 0x07, 0x00], // 6  Yellow
        [0x07, 0x07, 0x07], // 7  White (dim)
        [0x04, 0x04, 0x04], // 8  Dark Gray
        [0x00, 0x00, 0x0F], // 9  Bright Blue
        [0x00, 0x0F, 0x00], // 10 Bright Red
        [0x00, 0x0F, 0x0F], // 11 Bright Magenta
        [0x0F, 0x00, 0x00], // 12 Bright Green
        [0x0F, 0x00, 0x0F], // 13 Bright Cyan
        [0x0F, 0x0F, 0x00], // 14 Bright Yellow
        [0x0F, 0x0F, 0x0F], // 15 Bright White
    ];
    let raw = TABLE_GRB[index as usize];
    [
        raw[1].wrapping_mul(17),
        raw[0].wrapping_mul(17),
        raw[2].wrapping_mul(17),
    ]
}

/// Four quadrant centers within a 640x400 frame.
const TL: (u32, u32) = (160, 100);
const TR: (u32, u32) = (480, 100);
const BL: (u32, u32) = (160, 300);
const BR: (u32, u32) = (480, 300);

#[test]
fn egc_mode_1_fgc_fill() {
    let (framebuffer, height) = run_mode(1);
    assert_eq!(height, 400);

    // FGC fill writes the foreground color register to all enabled
    // planes. TL=1 (blue), TR=2 (red), BL=4 (green), BR=8 (dark gray).
    assert_pixel(&framebuffer, TL.0, TL.1, palette_rgb(1), "mode 1 TL blue");
    assert_pixel(&framebuffer, TR.0, TR.1, palette_rgb(2), "mode 1 TR red");
    assert_pixel(&framebuffer, BL.0, BL.1, palette_rgb(4), "mode 1 BL green");
    assert_pixel(
        &framebuffer,
        BR.0,
        BR.1,
        palette_rgb(8),
        "mode 1 BR dark gray",
    );
}

#[test]
fn egc_mode_2_bgc_fill() {
    let (framebuffer, height) = run_mode(2);
    assert_eq!(height, 400);

    // BGC fill writes the background color register to all enabled
    // planes. TL=3 (magenta), TR=5 (cyan), BL=6 (yellow), BR=15 (white).
    assert_pixel(
        &framebuffer,
        TL.0,
        TL.1,
        palette_rgb(3),
        "mode 2 TL magenta",
    );
    assert_pixel(&framebuffer, TR.0, TR.1, palette_rgb(5), "mode 2 TR cyan");
    assert_pixel(&framebuffer, BL.0, BL.1, palette_rgb(6), "mode 2 BL yellow");
    assert_pixel(
        &framebuffer,
        BR.0,
        BR.1,
        palette_rgb(15),
        "mode 2 BR bright white",
    );
}

#[test]
fn egc_mode_3_cpu_broadcast() {
    let (framebuffer, height) = run_mode(3);
    assert_eq!(height, 400);

    // CPU broadcast (ope=0) writes CPU data to all planes selected by
    // the access register. TL=B-only (1 blue), TR=R-only (2 red),
    // BL=B+G (5 cyan), BR=R+E (10 bright red).
    assert_pixel(&framebuffer, TL.0, TL.1, palette_rgb(1), "mode 3 TL blue");
    assert_pixel(&framebuffer, TR.0, TR.1, palette_rgb(2), "mode 3 TR red");
    assert_pixel(&framebuffer, BL.0, BL.1, palette_rgb(5), "mode 3 BL cyan");
    assert_pixel(
        &framebuffer,
        BR.0,
        BR.1,
        palette_rgb(10),
        "mode 3 BR bright red",
    );
}

#[test]
fn egc_mode_4_rop_copy() {
    let (framebuffer, height) = run_mode(4);
    assert_eq!(height, 400);

    // Identical source (S = all planes 0xFF -> palette index 15) and
    // identical destination pre-fill (R+G planes 0xFF, B+E 0x00 ->
    // palette index 6 = yellow). Each quadrant applies a different ROP
    // truth table to the (S, D) pair:
    //   TL: ROP 0xF0 (S)  -> 15 bright white
    //   TR: ROP 0x0F (~S) -> 0  black
    //   BL: ROP 0xCC (D)  -> 6  yellow (dim)
    //   BR: ROP 0x33 (~D) -> 9  bright blue
    assert_pixel(
        &framebuffer,
        TL.0,
        TL.1,
        palette_rgb(15),
        "mode 4 TL ROP F0 (S=bright white)",
    );
    assert_pixel(
        &framebuffer,
        TR.0,
        TR.1,
        palette_rgb(0),
        "mode 4 TR ROP 0F (~S=black)",
    );
    assert_pixel(
        &framebuffer,
        BL.0,
        BL.1,
        palette_rgb(6),
        "mode 4 BL ROP CC (D=yellow)",
    );
    assert_pixel(
        &framebuffer,
        BR.0,
        BR.1,
        palette_rgb(9),
        "mode 4 BR ROP 33 (~D=bright blue)",
    );
}
