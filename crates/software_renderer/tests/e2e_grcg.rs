//! End-to-end tests for the GRCG (Graphic Charger) path on a PC-9801VM.
//!
//! Each test boots the `debug_grcg.asm` ROM on a PC-9801VM (V30 CPU,
//! GRCG v1) with a different mode-selector byte and asserts the
//! framebuffer matches the expected pattern. This exercises the
//! direct-write, TDW, TCR, RMW, word-wide, and monochrome paths
//! through CPU -> bus I/O -> GRCG -> compose -> framebuffer bytes,
//! using the standard 16-color analog palette programmed by the ROM.

use common::{Bus, Cpu, CpuMode, MachineModel};
use machine::{Pc9801Bus, Pc9801Vm};

const DEBUG_GRCG_ROM: &[u8] = include_bytes!("../../../utils/debug/debug_grcg.rom");
const FONT_ROM_DATA: &[u8] = include_bytes!("../../../utils/font/font.rom");

const MODE_BYTE_ADDR: u32 = 0x0500;
const RUN_BUDGET_CYCLES: u64 = 50_000_000;

const FB_WIDTH: u32 = 640;
const FB_HEIGHT_MAX: u32 = 480;

fn run_mode(mode: u8) -> (Pc9801Vm, Vec<u8>, u32) {
    let bus = Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    let mut machine = Pc9801Vm::new(cpu::V30::new(), bus);
    machine.bus.load_font_rom(FONT_ROM_DATA);
    machine.bus.load_bios_rom(DEBUG_GRCG_ROM);
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
    (machine, framebuffer, height)
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

/// 16-color analog palette programmed by the GRCG debug ROM. Encodes the
/// (G,R,B) 4-bit components from `palette_data` in `debug_grcg.asm`,
/// expanded by the renderer's `*17` factor and reordered to (R,G,B).
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

/// Per-bit palette index for a byte-aligned plane stripe. `bit` ranges
/// 0..=7 where bit 7 is the leftmost pixel of the cell.
fn pixel_index(b: u8, r: u8, g: u8, e: u8, bit: u8) -> u8 {
    let mask = 1u8 << bit;
    let bb = u8::from(b & mask != 0);
    let rr = u8::from(r & mask != 0) << 1;
    let gg = u8::from(g & mask != 0) << 2;
    let ee = u8::from(e & mask != 0) << 3;
    bb | rr | gg | ee
}

/// Sample one cell (8 pixels) at line `y`, validating each pixel against
/// the per-bit palette index derived from the plane byte values.
#[allow(clippy::too_many_arguments)]
fn assert_cell_pattern(
    framebuffer: &[u8],
    x_base: u32,
    y: u32,
    b: u8,
    r: u8,
    g: u8,
    e: u8,
    context: &str,
) {
    for col in 0..8u32 {
        let bit = 7 - col as u8;
        let index = pixel_index(b, r, g, e, bit);
        let expected = palette_rgb(index);
        assert_pixel(
            framebuffer,
            x_base + col,
            y,
            expected,
            &format!("{context} bit{bit}"),
        );
    }
}

#[test]
fn grcg_mode_1_solid_white() {
    let (_machine, framebuffer, height) = run_mode(1);
    assert_eq!(height, 400);

    // Sample a handful of pixels across the screen - all should be white.
    let white = palette_rgb(15);
    for &(x, y) in &[(0u32, 0u32), (320, 200), (639, 399), (100, 50), (500, 350)] {
        assert_pixel(&framebuffer, x, y, white, "mode 1 solid white");
    }
}

#[test]
fn grcg_mode_2_individual_planes() {
    let (machine, framebuffer, height) = run_mode(2);
    assert_eq!(height, 400);

    // 4 horizontal bands of 100 lines, one plane per band.
    assert_pixel(&framebuffer, 320, 50, palette_rgb(1), "mode 2 B-band");
    assert_pixel(&framebuffer, 320, 150, palette_rgb(2), "mode 2 R-band");
    assert_pixel(&framebuffer, 320, 250, palette_rgb(4), "mode 2 G-band");
    assert_pixel(&framebuffer, 320, 350, palette_rgb(8), "mode 2 E-band");

    // Plane isolation: each band's active plane is 0xFF and the other
    // three planes are 0x00.
    const VRAM_B: u32 = 0xA8000;
    const VRAM_R: u32 = 0xB0000;
    const VRAM_G: u32 = 0xB8000;
    const VRAM_E: u32 = 0xE0000;
    let bands: [(u32, u32, [u32; 3], &str); 4] = [
        (50, VRAM_B, [VRAM_R, VRAM_G, VRAM_E], "B-band"),
        (150, VRAM_R, [VRAM_B, VRAM_G, VRAM_E], "R-band"),
        (250, VRAM_G, [VRAM_B, VRAM_R, VRAM_E], "G-band"),
        (350, VRAM_E, [VRAM_B, VRAM_R, VRAM_G], "E-band"),
    ];
    for (y, active, others, label) in bands {
        let offset = y * 80;
        assert_eq!(
            machine.bus.read_byte_direct(active + offset),
            0xFF,
            "mode 2 {label} active plane should be 0xFF at y={y}",
        );
        for off in others {
            assert_eq!(
                machine.bus.read_byte_direct(off + offset),
                0x00,
                "mode 2 {label} plane 0x{off:05X} should be 0x00 at y={y}",
            );
        }
    }
}

#[test]
fn grcg_mode_3_tdw_full() {
    let (_machine, framebuffer, height) = run_mode(3);
    assert_eq!(height, 400);

    // VRAM bytes: B=0xAA, R=0x55, G=0xF0, E=0x0F. Same pattern across the
    // entire screen, so the first cell repeats every 8 pixels.
    assert_cell_pattern(&framebuffer, 0, 200, 0xAA, 0x55, 0xF0, 0x0F, "mode 3 TDW");
    assert_cell_pattern(
        &framebuffer,
        320,
        100,
        0xAA,
        0x55,
        0xF0,
        0x0F,
        "mode 3 TDW mid",
    );
}

#[test]
fn grcg_mode_4_tdw_selective() {
    let (_machine, framebuffer, height) = run_mode(4);
    assert_eq!(height, 400);

    // 4 quadrants, each pre-filled to 0xFF then TDW-overwritten with tile
    // 0x00 under a different plane-disable mask. Disabled planes keep
    // 0xFF, enabled planes drop to 0x00:
    //   TL (mask 0x8A, R+E disabled): index 10 bright red
    //   TR (mask 0x8C, G+E disabled): index 12 bright green
    //   BL (mask 0x86, R+G disabled): index 6  yellow
    //   BR (mask 0x83, B+R disabled): index 3  magenta
    assert_pixel(
        &framebuffer,
        160,
        100,
        palette_rgb(10),
        "mode 4 TL bright red",
    );
    assert_pixel(
        &framebuffer,
        480,
        100,
        palette_rgb(12),
        "mode 4 TR bright green",
    );
    assert_pixel(&framebuffer, 160, 300, palette_rgb(6), "mode 4 BL yellow");
    assert_pixel(&framebuffer, 480, 300, palette_rgb(3), "mode 4 BR magenta");
}

#[test]
fn grcg_mode_5_tcr() {
    let (machine, framebuffer, height) = run_mode(5);
    assert_eq!(height, 400);

    // Band 0 (lines 0-99): B=0xFF, R=0x00, G=0xFF, E=0x00 -> uniform cyan (5).
    assert_pixel(&framebuffer, 320, 50, palette_rgb(5), "mode 5 band 0");
    // Band 1 (lines 100-199): all planes 0xFF -> bright white (15).
    assert_pixel(&framebuffer, 320, 150, palette_rgb(15), "mode 5 band 1");
    // Band 2 (lines 200-299): all planes 0x00 -> black (0).
    assert_pixel(&framebuffer, 320, 250, palette_rgb(0), "mode 5 band 2");
    // Band 3 (lines 300-399): B=0xAA, R=0x55, G=0xAA, E=0x55 - per-bit alternating.
    assert_cell_pattern(
        &framebuffer,
        0,
        350,
        0xAA,
        0x55,
        0xAA,
        0x55,
        "mode 5 band 3",
    );

    // TCR result bytes stored at RAM 0x0500..0x0503 by the ROM. The mode
    // byte we wrote at 0x0500 is overwritten by the first TCR result.
    assert_eq!(
        machine.bus.read_byte_direct(0x0500),
        0xFF,
        "mode 5 TCR band 0 (all match)",
    );
    assert_eq!(
        machine.bus.read_byte_direct(0x0501),
        0x00,
        "mode 5 TCR band 1 (R,E mismatch)",
    );
    assert_eq!(
        machine.bus.read_byte_direct(0x0502),
        0x00,
        "mode 5 TCR band 2 (B,G mismatch)",
    );
    assert_eq!(
        machine.bus.read_byte_direct(0x0503),
        0xAA,
        "mode 5 TCR band 3 (partial match)",
    );
}

#[test]
fn grcg_mode_6_rmw_all() {
    let (_machine, framebuffer, height) = run_mode(6);
    assert_eq!(height, 400);

    // Resulting VRAM: B=0xF5, R=0x05, G=0xA5, E=0x55. Same byte across
    // the entire screen.
    assert_cell_pattern(&framebuffer, 0, 200, 0xF5, 0x05, 0xA5, 0x55, "mode 6 RMW");
    assert_cell_pattern(
        &framebuffer,
        320,
        100,
        0xF5,
        0x05,
        0xA5,
        0x55,
        "mode 6 RMW mid",
    );
}

#[test]
fn grcg_mode_7_rmw_selective() {
    let (_machine, framebuffer, height) = run_mode(7);
    assert_eq!(height, 400);

    // 4 quadrants, each pre-filled to 0xAA then RMW-overwritten under a
    // different plane-disable mask. Enabled planes become 0xFF, disabled
    // planes keep 0xAA. Per-bit pattern alternates index 15 (all 1) with
    // a quadrant-specific color (only the enabled planes are 1):
    //   TL (mask 0xCC, G+E disabled): white / magenta (3)
    //   TR (mask 0xCA, R+E disabled): white / cyan (5)
    //   BL (mask 0xC6, R+G disabled): white / bright blue (9)
    //   BR (mask 0xC3, B+R disabled): white / bright green (12)
    // The quadrant span runs from offset 0 (TL), 40 (TR), 16000 (BL),
    // 16040 (BR); each 320x200.
    assert_cell_pattern(
        &framebuffer,
        0,
        100,
        0xFF,
        0xFF,
        0xAA,
        0xAA,
        "mode 7 TL (G+E disabled)",
    );
    assert_cell_pattern(
        &framebuffer,
        320,
        100,
        0xFF,
        0xAA,
        0xFF,
        0xAA,
        "mode 7 TR (R+E disabled)",
    );
    assert_cell_pattern(
        &framebuffer,
        0,
        300,
        0xFF,
        0xAA,
        0xAA,
        0xFF,
        "mode 7 BL (R+G disabled)",
    );
    assert_cell_pattern(
        &framebuffer,
        320,
        300,
        0xAA,
        0xAA,
        0xFF,
        0xFF,
        "mode 7 BR (B+R disabled)",
    );
}

#[test]
fn grcg_mode_8_word_ops() {
    let (_machine, framebuffer, height) = run_mode(8);
    assert_eq!(height, 400);

    // Top half (lines 0-199): TDW B=0x33, R=0xCC, G=0x55, E=0xAA.
    assert_cell_pattern(
        &framebuffer,
        0,
        100,
        0x33,
        0xCC,
        0x55,
        0xAA,
        "mode 8 top TDW",
    );

    // Bottom half (lines 200-399): RMW result B=0xF5, R=0x5F, G=0xFF, E=0x55.
    assert_cell_pattern(
        &framebuffer,
        0,
        300,
        0xF5,
        0x5F,
        0xFF,
        0x55,
        "mode 8 bottom RMW",
    );
}

/// Fixed BRG text color used by the monochrome path. Matches the
/// renderer's `FIXED_TEXT_LUT`: bit 0=blue, bit 1=red, bit 2=green.
fn text_color_rgb(index: u8) -> [u8; 3] {
    let blue = if index & 0x01 != 0 { 0xFF } else { 0x00 };
    let red = if index & 0x02 != 0 { 0xFF } else { 0x00 };
    let green = if index & 0x04 != 0 { 0xFF } else { 0x00 };
    [red, green, blue]
}

#[test]
fn grcg_mode_9_monochrome() {
    let (_machine, framebuffer, height) = run_mode(9);
    assert_eq!(height, 400);

    // Sample one pixel per band, centered on the screen and inside a
    // space character cell.
    assert_pixel(
        &framebuffer,
        320,
        40,
        text_color_rgb(7),
        "mode 9 band 0 (mono ON, attr white)",
    );
    assert_pixel(
        &framebuffer,
        320,
        120,
        text_color_rgb(2),
        "mode 9 band 1 (mono ON, attr red)",
    );
    assert_pixel(
        &framebuffer,
        320,
        200,
        text_color_rgb(4),
        "mode 9 band 2 (mono ON, attr green)",
    );
    assert_pixel(
        &framebuffer,
        320,
        280,
        text_color_rgb(5),
        "mode 9 band 3 (mono ON, attr cyan)",
    );
    assert_pixel(
        &framebuffer,
        320,
        360,
        [0, 0, 0],
        "mode 9 band 4 (mono OFF, falls through to black)",
    );
}
