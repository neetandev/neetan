//! Full-machine background-layer tests: per-layer tile-map selection, the
//! chip-select bits targeting the absent second sprite chip, and the
//! MPU-side pattern-window gating, all programmed through the CPU-visible
//! bus and verified on the rendered framebuffer.

#[path = "common/harness.rs"]
mod harness;

use harness::{machine, pixel, read_word, write_byte, write_word};
use machine_x68k::{X68kMachine, X68kModel};

/// Black pixel of an unlit display position.
const BLACK: [u8; 4] = [0, 0, 0, 255];
/// Palette block 1 entry 2 rendered at full contrast (GRBI 0x07C0).
const RED: [u8; 4] = [251, 0, 0, 255];
/// Palette block 1 entry 3 rendered at full contrast (GRBI 0xF800).
const GREEN: [u8; 4] = [0, 251, 0, 255];
/// Palette block 0 entry 2 rendered at full contrast (GRBI 0x003E).
const BLUE: [u8; 4] = [0, 0, 251, 255];

/// Background control bit enabling BG0.
const BG0_ENABLE: u16 = 0x0001;
/// Background control bit selecting BG0's tile map.
const BG0_MAP_SELECT: u16 = 0x0002;
/// Background control bit selecting the absent chip for BG0.
const BG0_CHIP_SELECT: u16 = 0x0004;
/// Background control bit enabling BG1.
const BG1_ENABLE: u16 = 0x0008;
/// Background control bit selecting BG1's tile map.
const BG1_MAP_SELECT: u16 = 0x0010;
/// Background control bit selecting the absent chip for BG1.
const BG1_CHIP_SELECT: u16 = 0x0020;
/// Background control bit enabling the sprite screen display.
const DISPLAY_ENABLE: u16 = 0x0200;
/// Background control bit routing CPU pattern access to the absent chip.
const MPU_CHIP_SELECT: u16 = 0x0400;

/// Builds a machine showing a 16x1 display with 8x8 pattern 2 (code 2) and
/// pattern 3 (code 3), tile (0, 0) of map 0 naming pattern 2 and tile
/// (0, 0) of map 1 naming pattern 3, both in palette block 1.
fn background_machine() -> X68kMachine {
    let mut machine = machine(X68kModel::X68000);
    // CRTC R00-R09: an 11-column, 4-raster frame with a 16x1 display.
    for (index, value) in [10u16, 0, 0, 2, 3, 0, 0, 1, 0, 3].into_iter().enumerate() {
        write_word(&mut machine, 0xE80000 + index as u32 * 2, value);
    }
    write_byte(&mut machine, 0xE8E001, 15);
    // Palette block 1 entries 2 and 3: red and green.
    write_word(&mut machine, 0xE82224, 0x07C0);
    write_word(&mut machine, 0xE82226, 0xF800);
    // 8x8 patterns 2 and 3: solid codes 2 and 3.
    for word in 0..16u32 {
        write_word(&mut machine, 0xEB8000 + (32 + word) * 2, 0x2222);
        write_word(&mut machine, 0xEB8000 + (48 + word) * 2, 0x3333);
    }
    // Tile (0, 0) of both maps in palette block 1.
    write_word(&mut machine, 0xEBC000, 0x0102);
    write_word(&mut machine, 0xEBE000, 0x0103);
    // Sprite screen porch registers and the sprite screen enable.
    write_word(&mut machine, 0xEB080C, 0x0004);
    write_word(&mut machine, 0xE82600, 0x0040);
    machine
}

/// Programs the background control register and renders some frames.
fn render(machine: &mut X68kMachine, control: u16) {
    write_word(machine, 0xEB0808, DISPLAY_ENABLE | control);
    machine.run_for(10_000);
}

#[test]
fn background_zero_selects_its_tile_map() {
    let mut machine = background_machine();
    render(&mut machine, BG0_ENABLE);
    assert_eq!(pixel(&machine, 0, 0), RED, "map 0 names pattern 2");
    assert_eq!(pixel(&machine, 8, 0), BLACK, "unmapped tiles stay black");
    render(&mut machine, BG0_ENABLE | BG0_MAP_SELECT);
    assert_eq!(pixel(&machine, 0, 0), GREEN, "map 1 names pattern 3");
}

#[test]
fn background_one_selects_its_tile_map() {
    let mut machine = background_machine();
    render(&mut machine, BG1_ENABLE);
    assert_eq!(pixel(&machine, 0, 0), RED, "map 0 names pattern 2");
    render(&mut machine, BG1_ENABLE | BG1_MAP_SELECT);
    assert_eq!(pixel(&machine, 0, 0), GREEN, "map 1 names pattern 3");
}

#[test]
fn chip_selected_backgrounds_show_only_pattern_zero() {
    let mut machine = background_machine();
    render(&mut machine, BG0_ENABLE | BG0_CHIP_SELECT);
    assert_eq!(
        pixel(&machine, 0, 0),
        BLACK,
        "the absent chip supplies an all-zero map for BG0"
    );
    render(&mut machine, BG1_ENABLE | BG1_CHIP_SELECT);
    assert_eq!(
        pixel(&machine, 0, 0),
        BLACK,
        "the absent chip supplies an all-zero map for BG1"
    );
    // The zero map still names pattern 0: its dots tile the whole layer
    // in palette block 0.
    write_word(&mut machine, 0xE82204, 0x003E);
    for word in 0..16u32 {
        write_word(&mut machine, 0xEB8000 + word * 2, 0x2222);
    }
    render(&mut machine, BG0_ENABLE | BG0_CHIP_SELECT);
    assert_eq!(pixel(&machine, 0, 0), BLUE, "pattern 0 tiles the layer");
    assert_eq!(pixel(&machine, 15, 0), BLUE, "pattern 0 tiles the layer");
}

#[test]
fn mpu_chip_select_gates_the_pattern_window() {
    let mut machine = background_machine();
    // With the MPU chip select raised the window reads the empty second
    // chip and discards writes.
    render(&mut machine, BG0_ENABLE | MPU_CHIP_SELECT);
    assert_eq!(read_word(&mut machine, 0xEB8040), 0x0000);
    write_word(&mut machine, 0xEB8040, 0xDEAD);
    write_word(&mut machine, 0xEBC000, 0xDEAD);
    assert_eq!(read_word(&mut machine, 0xEBC000), 0x0000);
    // The first chip keeps its contents and the display never changed:
    // the tile still renders from the untouched pattern and map.
    assert_eq!(pixel(&machine, 0, 0), RED);
    render(&mut machine, BG0_ENABLE);
    assert_eq!(read_word(&mut machine, 0xEB8040), 0x2222);
    assert_eq!(read_word(&mut machine, 0xEBC000), 0x0102);
    assert_eq!(pixel(&machine, 0, 0), RED);
}
