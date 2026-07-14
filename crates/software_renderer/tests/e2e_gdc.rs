//! End-to-end tests for the standard GDC planar graphics path.
//!
//! Each test boots the `debug_gdc.asm` ROM on a PC-9801F (8086, no GRCG,
//! no EGC) with a different mode-selector byte and asserts the
//! framebuffer matches the expected band pattern. This exercises the
//! direct B/R/G/E plane write path through CPU -> bus I/O -> GDC ->
//! compose -> framebuffer bytes, for both 8-color digital and 16-color
//! analog palettes, in 400-line and 200-line modes.

use common::{BUILTIN_FONT_ROM, Bus, Cpu, CpuMode, MachineModel};
use machine_98::{Pc9801Bus, Pc9801F};

const DEBUG_GDC_ROM: &[u8] = include_bytes!("../../../utils/debug/debug_gdc.rom");

const MODE_BYTE_ADDR: u32 = 0x0500;
const RUN_BUDGET_CYCLES: u64 = 50_000_000;

const FB_WIDTH: u32 = 640;
const FB_HEIGHT_MAX: u32 = 480;

fn run_mode(mode: u8) -> (Vec<u8>, u32) {
    let bus = Pc9801Bus::new(MachineModel::PC9801F, CpuMode::High, 48000);

    let mut machine = Pc9801F::new(cpu::I8086::new(), bus);
    machine.bus.load_font_rom(BUILTIN_FONT_ROM);
    machine.bus.load_bios_rom(DEBUG_GDC_ROM);
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

/// Fixed BRG colors for digital graphics indices 0..7. Bit 0 = blue,
/// bit 1 = red, bit 2 = green, each 0 or 0xFF. Matches
/// `pack_fixed_color` in `crates/machine_98/src/bus.rs`.
fn digital_rgb(index: u8) -> [u8; 3] {
    let blue = if index & 0x01 != 0 { 0xFF } else { 0x00 };
    let red = if index & 0x02 != 0 { 0xFF } else { 0x00 };
    let green = if index & 0x04 != 0 { 0xFF } else { 0x00 };
    [red, green, blue]
}

/// Grayscale ramp set by the analog ROM path: palette[N] = (N, N, N)
/// in 4-bit GRB; the renderer expands each component to (N * 17).
fn grayscale_rgb(index: u8) -> [u8; 3] {
    let v = index.wrapping_mul(17);
    [v, v, v]
}

#[test]
fn gdc_mode_1_8color_digital_400_line() {
    let (framebuffer, height) = run_mode(1);
    assert_eq!(height, 400, "mode 1 should render 400 lines");

    // 8 bands of 80 pixels, sampled in the middle of each band on a
    // mid-screen raster.
    for band in 0..8u32 {
        let x = band * 80 + 40;
        let y = 200;
        let expected = digital_rgb(band as u8);
        assert_pixel(&framebuffer, x, y, expected, &format!("mode 1 band {band}"));
    }
}

#[test]
fn gdc_mode_2_16color_analog_400_line() {
    let (framebuffer, height) = run_mode(2);
    assert_eq!(height, 400, "mode 2 should render 400 lines");

    for band in 0..16u32 {
        let x = band * 40 + 20;
        let y = 200;
        let expected = grayscale_rgb(band as u8);
        assert_pixel(&framebuffer, x, y, expected, &format!("mode 2 band {band}"));
    }
}

#[test]
fn gdc_mode_3_8color_digital_200_line() {
    let (framebuffer, height) = run_mode(3);
    assert_eq!(
        height, 400,
        "mode 3 still produces 400 output rasters; skipline dims, doesn't reduce height",
    );

    // Bright rasters: every even output line (lines_per_row = 2).
    for band in 0..8u32 {
        let x = band * 80 + 40;
        let expected = digital_rgb(band as u8);
        assert_pixel(
            &framebuffer,
            x,
            100,
            expected,
            &format!("mode 3 band {band} bright raster"),
        );
    }

    // Skipline rasters: every odd output line must be black regardless
    // of band color. Exercise the top, middle, and bottom of the
    // active area.
    for &y in &[1u32, 101, 201] {
        for band in 0..8u32 {
            let x = band * 80 + 40;
            assert_pixel(
                &framebuffer,
                x,
                y,
                [0, 0, 0],
                &format!("mode 3 band {band} skipline y={y}"),
            );
        }
    }
}

#[test]
fn gdc_mode_4_16color_analog_200_line() {
    let (framebuffer, height) = run_mode(4);
    assert_eq!(
        height, 400,
        "mode 4 still produces 400 output rasters; skipline dims, doesn't reduce height",
    );

    for band in 0..16u32 {
        let x = band * 40 + 20;
        let expected = grayscale_rgb(band as u8);
        assert_pixel(
            &framebuffer,
            x,
            100,
            expected,
            &format!("mode 4 band {band} bright raster"),
        );
    }

    for &y in &[1u32, 101, 201] {
        for band in 0..16u32 {
            let x = band * 40 + 20;
            assert_pixel(
                &framebuffer,
                x,
                y,
                [0, 0, 0],
                &format!("mode 4 band {band} skipline y={y}"),
            );
        }
    }
}
