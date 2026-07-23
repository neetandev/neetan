//! Register-level VGA mode walk: each mode is entered by programming the
//! ET4000 registers directly (the same register files the BIOS would load),
//! display memory is painted with a deterministic pattern, a frame is rendered
//! and the host asserts exact framebuffer pixels and display dimensions.
//!
//! `NEETAN_DUMP_FRAMES=<dir>` writes one PNG per step for debugging;
//! `NEETAN_RECORD_FRAME_HASHES=1` prints the golden hash table instead of
//! asserting it.

use common::{Bus, Machine, NoTrace};
use machine_at::{AtBus, AtMachine, AtModel};

#[path = "common/harness.rs"]
mod harness;
#[path = "common/mode_vectors.rs"]
mod mode_vectors;
use harness::{
    ModeVector, fill_vram, fill_vram_ramp, framebuffer_hash, machine_for_model, mode_pixel, pen6,
    pixel_rgba, render_frame, route_vga_window, run_millis, write_vram,
};
use mode_vectors::{
    ATC_PACKED_256, MODE_0D, MODE_0E, MODE_0F, MODE_01, MODE_2E, MODE_2F, MODE_03, MODE_04,
    MODE_06, MODE_10, MODE_11, MODE_12, MODE_13, MODE_30, PALETTE_PACKED_256,
};

/// Graphics display memory window base (segment 0xA000).
const VGA_GRAPHICS: u32 = 0x000A_0000;
/// Text/CGA display memory window base (segment 0xB800).
const VGA_TEXT: u32 = 0x000B_8000;

/// Synthetic step id: unchained Mode X with a page flip.
const STEP_MODE_X: u8 = 0xF0;
/// Synthetic step id: split screen with pel panning on mode 0Dh.
const STEP_SPLIT_PAN: u8 = 0xF1;

/// The order the mode walk visits the steps.
const STEPS: &[u8] = &[
    0x03,
    0x01,
    0x04,
    0x05,
    0x06,
    0x0D,
    0x0E,
    0x0F,
    0x10,
    0x11,
    0x12,
    0x13,
    STEP_MODE_X,
    STEP_SPLIT_PAN,
    0x2F,
    0x2E,
    0x30,
];

const MODE_X: ModeVector = ModeVector {
    misc: 0x63,
    seq: [0x03, 0x01, 0x0F, 0x00, 0x06, 0x00, 0x00, 0xBC],
    crtc: [
        0x5F, 0x4F, 0x50, 0x82, 0x54, 0x80, 0xBF, 0x1F, 0x00, 0x41, 0x00, 0x00, 0x40, 0x00, 0x00,
        0x00, 0x9C, 0x8E, 0x8F, 0x28, 0x00, 0x96, 0xB9, 0xE3, 0xFF,
    ],
    gc: [0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x05, 0x0F, 0xFF],
    atc: ATC_PACKED_256,
    palette: &PALETTE_PACKED_256,
    segment_select: 0x00,
};

const MODE_SPLIT_PAN: ModeVector = ModeVector {
    misc: 0x63,
    seq: [0x03, 0x09, 0x01, 0x00, 0x06, 0x00, 0x00, 0xBC],
    crtc: [
        0x2D, 0x27, 0x28, 0x90, 0x2B, 0x80, 0xBF, 0x0F, 0x00, 0x80, 0x00, 0x00, 0x00, 0x28, 0x00,
        0x00, 0x9C, 0x0E, 0x8F, 0x14, 0x00, 0x96, 0xB9, 0xE3, 0x64,
    ],
    gc: [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x0F, 0xFF],
    atc: [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16,
        0x17, 0x21, 0x00, 0x0F, 0x04, 0x00, 0x00, 0x00,
    ],
    palette: &PALETTE_SPLIT,
    segment_select: 0x00,
};

/// The split-screen / pel-pan mode (0xF1). Captured from the real ET4000AX VGA BIOS.
const PALETTE_SPLIT: [u8; 768] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x2A, 0x00, 0x2A, 0x00, 0x00, 0x2A, 0x2A, 0x2A, 0x00, 0x00, 0x2A,
    0x00, 0x2A, 0x2A, 0x15, 0x00, 0x2A, 0x2A, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2A, 0x00, 0x2A,
    0x00, 0x00, 0x2A, 0x2A, 0x2A, 0x00, 0x00, 0x2A, 0x00, 0x2A, 0x2A, 0x15, 0x00, 0x2A, 0x2A, 0x2A,
    0x15, 0x15, 0x15, 0x15, 0x15, 0x3F, 0x15, 0x3F, 0x15, 0x15, 0x3F, 0x3F, 0x3F, 0x15, 0x15, 0x3F,
    0x15, 0x3F, 0x3F, 0x3F, 0x15, 0x3F, 0x3F, 0x3F, 0x15, 0x15, 0x15, 0x15, 0x15, 0x3F, 0x15, 0x3F,
    0x15, 0x15, 0x3F, 0x3F, 0x3F, 0x15, 0x15, 0x3F, 0x15, 0x3F, 0x3F, 0x3F, 0x15, 0x3F, 0x3F, 0x3F,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x2A, 0x00, 0x2A, 0x00, 0x00, 0x2A, 0x2A, 0x2A, 0x00, 0x00, 0x2A,
    0x00, 0x2A, 0x2A, 0x15, 0x00, 0x2A, 0x2A, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2A, 0x00, 0x2A,
    0x00, 0x00, 0x2A, 0x2A, 0x2A, 0x00, 0x00, 0x2A, 0x00, 0x2A, 0x2A, 0x15, 0x00, 0x2A, 0x2A, 0x2A,
    0x15, 0x15, 0x15, 0x15, 0x15, 0x3F, 0x15, 0x3F, 0x15, 0x15, 0x3F, 0x3F, 0x3F, 0x15, 0x15, 0x3F,
    0x15, 0x3F, 0x3F, 0x3F, 0x15, 0x3F, 0x3F, 0x3F, 0x15, 0x15, 0x15, 0x15, 0x15, 0x3F, 0x15, 0x3F,
    0x15, 0x15, 0x3F, 0x3F, 0x3F, 0x15, 0x15, 0x3F, 0x15, 0x3F, 0x3F, 0x3F, 0x15, 0x3F, 0x3F, 0x3F,
    0x3F, 0x1F, 0x1F, 0x3F, 0x27, 0x1F, 0x3F, 0x2F, 0x1F, 0x3F, 0x37, 0x1F, 0x3F, 0x3F, 0x1F, 0x37,
    0x3F, 0x1F, 0x2F, 0x3F, 0x1F, 0x27, 0x3F, 0x1F, 0x1F, 0x3F, 0x1F, 0x1F, 0x3F, 0x27, 0x1F, 0x3F,
    0x2F, 0x1F, 0x3F, 0x37, 0x1F, 0x3F, 0x3F, 0x1F, 0x37, 0x3F, 0x1F, 0x2F, 0x3F, 0x1F, 0x27, 0x3F,
    0x2D, 0x2D, 0x3F, 0x31, 0x2D, 0x3F, 0x36, 0x2D, 0x3F, 0x3A, 0x2D, 0x3F, 0x3F, 0x2D, 0x3F, 0x3F,
    0x2D, 0x3A, 0x3F, 0x2D, 0x36, 0x3F, 0x2D, 0x31, 0x3F, 0x2D, 0x2D, 0x3F, 0x31, 0x2D, 0x3F, 0x36,
    0x2D, 0x3F, 0x3A, 0x2D, 0x3F, 0x3F, 0x2D, 0x3A, 0x3F, 0x2D, 0x36, 0x3F, 0x2D, 0x31, 0x3F, 0x2D,
    0x2D, 0x3F, 0x2D, 0x2D, 0x3F, 0x31, 0x2D, 0x3F, 0x36, 0x2D, 0x3F, 0x3A, 0x2D, 0x3F, 0x3F, 0x2D,
    0x3A, 0x3F, 0x2D, 0x36, 0x3F, 0x2D, 0x31, 0x3F, 0x00, 0x00, 0x1C, 0x07, 0x00, 0x1C, 0x0E, 0x00,
    0x1C, 0x15, 0x00, 0x1C, 0x1C, 0x00, 0x1C, 0x1C, 0x00, 0x15, 0x1C, 0x00, 0x0E, 0x1C, 0x00, 0x07,
    0x1C, 0x00, 0x00, 0x1C, 0x07, 0x00, 0x1C, 0x0E, 0x00, 0x1C, 0x15, 0x00, 0x1C, 0x1C, 0x00, 0x15,
    0x1C, 0x00, 0x0E, 0x1C, 0x00, 0x07, 0x1C, 0x00, 0x00, 0x1C, 0x00, 0x00, 0x1C, 0x07, 0x00, 0x1C,
    0x0E, 0x00, 0x1C, 0x15, 0x00, 0x1C, 0x1C, 0x00, 0x15, 0x1C, 0x00, 0x0E, 0x1C, 0x00, 0x07, 0x1C,
    0x0E, 0x0E, 0x1C, 0x11, 0x0E, 0x1C, 0x15, 0x0E, 0x1C, 0x18, 0x0E, 0x1C, 0x1C, 0x0E, 0x1C, 0x1C,
    0x0E, 0x18, 0x1C, 0x0E, 0x15, 0x1C, 0x0E, 0x11, 0x1C, 0x0E, 0x0E, 0x1C, 0x11, 0x0E, 0x1C, 0x15,
    0x0E, 0x1C, 0x18, 0x0E, 0x1C, 0x1C, 0x0E, 0x18, 0x1C, 0x0E, 0x15, 0x1C, 0x0E, 0x11, 0x1C, 0x0E,
    0x0E, 0x1C, 0x0E, 0x0E, 0x1C, 0x11, 0x0E, 0x1C, 0x15, 0x0E, 0x1C, 0x18, 0x0E, 0x1C, 0x1C, 0x0E,
    0x18, 0x1C, 0x0E, 0x15, 0x1C, 0x0E, 0x11, 0x1C, 0x14, 0x14, 0x1C, 0x16, 0x14, 0x1C, 0x18, 0x14,
    0x1C, 0x1A, 0x14, 0x1C, 0x1C, 0x14, 0x1C, 0x1C, 0x14, 0x1A, 0x1C, 0x14, 0x18, 0x1C, 0x14, 0x16,
    0x1C, 0x14, 0x14, 0x1C, 0x16, 0x14, 0x1C, 0x18, 0x14, 0x1C, 0x1A, 0x14, 0x1C, 0x1C, 0x14, 0x1A,
    0x1C, 0x14, 0x18, 0x1C, 0x14, 0x16, 0x1C, 0x14, 0x14, 0x1C, 0x14, 0x14, 0x1C, 0x16, 0x14, 0x1C,
    0x18, 0x14, 0x1C, 0x1A, 0x14, 0x1C, 0x1C, 0x14, 0x1A, 0x1C, 0x14, 0x18, 0x1C, 0x14, 0x16, 0x1C,
    0x00, 0x00, 0x10, 0x04, 0x00, 0x10, 0x08, 0x00, 0x10, 0x0C, 0x00, 0x10, 0x10, 0x00, 0x10, 0x10,
    0x00, 0x0C, 0x10, 0x00, 0x08, 0x10, 0x00, 0x04, 0x10, 0x00, 0x00, 0x10, 0x04, 0x00, 0x10, 0x08,
    0x00, 0x10, 0x0C, 0x00, 0x10, 0x10, 0x00, 0x0C, 0x10, 0x00, 0x08, 0x10, 0x00, 0x04, 0x10, 0x00,
    0x00, 0x10, 0x00, 0x00, 0x10, 0x04, 0x00, 0x10, 0x08, 0x00, 0x10, 0x0C, 0x00, 0x10, 0x10, 0x00,
    0x0C, 0x10, 0x00, 0x08, 0x10, 0x00, 0x04, 0x10, 0x08, 0x08, 0x10, 0x0A, 0x08, 0x10, 0x0C, 0x08,
    0x10, 0x0E, 0x08, 0x10, 0x10, 0x08, 0x10, 0x10, 0x08, 0x0E, 0x10, 0x08, 0x0C, 0x10, 0x08, 0x0A,
    0x10, 0x08, 0x08, 0x10, 0x0A, 0x08, 0x10, 0x0C, 0x08, 0x10, 0x0E, 0x08, 0x10, 0x10, 0x08, 0x0E,
    0x10, 0x08, 0x0C, 0x10, 0x08, 0x0A, 0x10, 0x08, 0x08, 0x10, 0x08, 0x08, 0x10, 0x0A, 0x08, 0x10,
    0x0C, 0x08, 0x10, 0x0E, 0x08, 0x10, 0x10, 0x08, 0x0E, 0x10, 0x08, 0x0C, 0x10, 0x08, 0x0A, 0x10,
    0x0B, 0x0B, 0x10, 0x0C, 0x0B, 0x10, 0x0D, 0x0B, 0x10, 0x0F, 0x0B, 0x10, 0x10, 0x0B, 0x10, 0x10,
    0x0B, 0x0F, 0x10, 0x0B, 0x0D, 0x10, 0x0B, 0x0C, 0x10, 0x0B, 0x0B, 0x10, 0x0C, 0x0B, 0x10, 0x0D,
    0x0B, 0x10, 0x0F, 0x0B, 0x10, 0x10, 0x0B, 0x0F, 0x10, 0x0B, 0x0D, 0x10, 0x0B, 0x0C, 0x10, 0x0B,
    0x0B, 0x10, 0x0B, 0x0B, 0x10, 0x0C, 0x0B, 0x10, 0x0D, 0x0B, 0x10, 0x0F, 0x0B, 0x10, 0x10, 0x0B,
    0x0F, 0x10, 0x0B, 0x0D, 0x10, 0x0B, 0x0C, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Sets the sequencer map mask (which planes a CPU write reaches).
fn set_map_mask(bus: &mut AtBus<NoTrace>, mask: u8) {
    bus.io_write_byte(0x03C4, 0x02);
    bus.io_write_byte(0x03C5, mask);
}

/// Sets the ET4000 segment select register (read and write banks equal).
fn set_bank(bus: &mut AtBus<NoTrace>, bank: u8) {
    bus.io_write_byte(0x03CD, bank * 0x11);
}

/// Loads a synthetic text font into plane 2 (the BIOS normally does this). Only
/// the glyphs the text assertions touch are defined: a full block (0xDB) and a
/// 'B' with lit pixels; the space glyph stays blank. Configures a linear
/// plane-2 write path first; the caller applies the text mode vector afterwards
/// to restore the display registers, leaving the font in place.
fn load_text_font(bus: &mut AtBus<NoTrace>) {
    // Plane 2 only, sequential addressing, write mode 0, full bit mask, A0000
    // 64 KiB window.
    bus.io_write_byte(0x03C4, 0x02);
    bus.io_write_byte(0x03C5, 0x04);
    bus.io_write_byte(0x03C4, 0x04);
    bus.io_write_byte(0x03C5, 0x06);
    bus.io_write_byte(0x03CE, 0x05);
    bus.io_write_byte(0x03CF, 0x00);
    bus.io_write_byte(0x03CE, 0x08);
    bus.io_write_byte(0x03CF, 0xFF);
    bus.io_write_byte(0x03CE, 0x06);
    bus.io_write_byte(0x03CF, 0x04);
    for row in 0..16u32 {
        bus.write_byte(VGA_GRAPHICS + 0xDB * 32 + row, 0xFF);
        bus.write_byte(VGA_GRAPHICS + 0x42 * 32 + row, 0x7E);
    }
}

/// Paints the two top-left text cells plus the hardware cursor cell used by the
/// text assertions: a full block yellow on blue, spaces on blue and red, then a
/// blinking 'B'.
fn paint_text(bus: &mut AtBus<NoTrace>) {
    // Clear the page to spaces on attribute 0x07, the way the BIOS does, so the
    // hardware cursor cell shows the light-gray foreground.
    for cell in 0..80 * 25u32 {
        bus.write_byte(VGA_TEXT + cell * 2, 0x20);
        bus.write_byte(VGA_TEXT + cell * 2 + 1, 0x07);
    }
    // Character/attribute pairs: 0xDB block (yellow on blue), space on blue,
    // space on red, blinking 'B' (attribute 0x87).
    write_vram(
        bus,
        VGA_TEXT,
        &[0xDB, 0x1E, 0x20, 0x1E, 0x20, 0x40, 0x42, 0x87],
    );
}

/// Paints both CGA interleave halves with a repeating bar byte.
fn paint_cga(bus: &mut AtBus<NoTrace>, bar_byte: u8) {
    fill_vram(bus, VGA_TEXT, bar_byte, 80 * 100);
    fill_vram(bus, VGA_TEXT + 0x2000, bar_byte, 80 * 100);
}

/// Paints planar color bars: planes 0-2 with constant patterns encoding the low
/// three color bits, plane 3 alternating so the odd bytes light the top bit.
fn paint_planar(bus: &mut AtBus<NoTrace>, plane_bytes: u32, planes: u8) {
    for (plane, pattern) in [(0x01u8, 0xAAu8), (0x02, 0xCC), (0x04, 0xF0)] {
        if planes & plane == 0 {
            continue;
        }
        set_map_mask(bus, plane);
        fill_vram(bus, VGA_GRAPHICS, pattern, plane_bytes);
    }
    if planes & 0x08 != 0 {
        set_map_mask(bus, 0x08);
        for offset in 0..plane_bytes {
            let value = if offset & 1 == 0 { 0x00 } else { 0xFF };
            bus.write_byte(VGA_GRAPHICS + offset, value);
        }
    }
}

/// Applies the mode vector, loads the palette and paints the pattern for a step.
fn setup_step(bus: &mut AtBus<NoTrace>, step: u8) {
    route_vga_window(bus);
    match step {
        0x03 => {
            load_text_font(bus);
            MODE_03.apply(bus);
            paint_text(bus);
        }
        0x01 => {
            load_text_font(bus);
            MODE_01.apply(bus);
            paint_text(bus);
        }
        // Mode 05h shares mode 04h's register file (only the composite color
        // burst differs, which the RGB renderer ignores).
        0x04 | 0x05 => {
            MODE_04.apply(bus);
            paint_cga(bus, 0x1B);
        }
        0x06 => {
            MODE_06.apply(bus);
            paint_cga(bus, 0xAA);
        }
        0x0D => {
            MODE_0D.apply(bus);
            paint_planar(bus, 40 * 200, 0x0F);
        }
        0x0E => {
            MODE_0E.apply(bus);
            paint_planar(bus, 80 * 200, 0x0F);
        }
        0x0F => {
            MODE_0F.apply(bus);
            // Mode 0Fh is two-plane monochrome: only planes 0 and 2 display.
            paint_planar(bus, 80 * 350, 0x05);
        }
        0x10 => {
            MODE_10.apply(bus);
            paint_planar(bus, 80 * 350, 0x0F);
        }
        0x11 => {
            MODE_11.apply(bus);
            paint_planar(bus, 80 * 480, 0x0F);
        }
        0x12 => {
            MODE_12.apply(bus);
            paint_planar(bus, 80 * 480, 0x0F);
        }
        0x13 => {
            MODE_13.apply(bus);
            fill_vram_ramp(bus, VGA_GRAPHICS, 0x00, 320 * 200);
        }
        STEP_MODE_X => {
            MODE_X.apply(bus);
            // Page zero: a flat color-5 fill the page flip must hide. Page one
            // at plane address 0x4000: a ramp from color 1. All planes writable,
            // so each stored byte paints four adjacent pixels.
            set_map_mask(bus, 0x0F);
            fill_vram(bus, VGA_GRAPHICS, 0x05, 80 * 200);
            fill_vram_ramp(bus, VGA_GRAPHICS + 0x4000, 0x01, 80 * 200);
        }
        STEP_SPLIT_PAN => {
            MODE_SPLIT_PAN.apply(bus);
            // Plane zero markers: host byte 0 (below the split) and byte 41
            // (above it, panned by the pel pan of four).
            set_map_mask(bus, 0x01);
            bus.write_byte(VGA_GRAPHICS, 0x80);
            bus.write_byte(VGA_GRAPHICS + 41, 0x80);
        }
        0x2F => setup_svga(bus, &MODE_2F, 4),
        0x2E => setup_svga(bus, &MODE_2E, 5),
        0x30 => setup_svga(bus, &MODE_30, 8),
        _ => panic!("unexpected step {step:02X}"),
    }
}

/// Applies an SVGA vector and fills every 64 KiB bank with a position ramp.
fn setup_svga(bus: &mut AtBus<NoTrace>, vector: &ModeVector, bank_count: u8) {
    vector.apply(bus);
    for bank in 0..bank_count {
        set_bank(bus, bank);
        // A bank is 65536 bytes; each starts at color 0 because a bank base is
        // a multiple of 256, so the ramp is continuous across banks.
        fill_vram_ramp(bus, VGA_GRAPHICS, 0x00, 0x1_0000);
    }
    set_bank(bus, 0);
}

/// Canonical VGA colors used by the text assertions.
const BLACK: u32 = pen6(0x00, 0x00, 0x00);
const BLUE: u32 = pen6(0x00, 0x00, 0x2A);
const RED: u32 = pen6(0x2A, 0x00, 0x00);
const LIGHT_GRAY: u32 = pen6(0x2A, 0x2A, 0x2A);
const YELLOW: u32 = pen6(0x3F, 0x3F, 0x15);

/// The resolved 16-pen palette of the current frame.
fn pens(machine: &AtMachine<NoTrace>) -> [u32; 16] {
    machine.bus.vga().resolve().pens
}

/// The resolved 256-pen palette of the current frame.
fn pens_256(machine: &AtMachine<NoTrace>) -> [u32; 256] {
    machine.bus.vga().resolve().pens_256
}

/// Waits until the pixel at (x, y) takes the expected value (blink and cursor
/// phases toggle over frames).
fn wait_for_pixel(
    machine: &mut AtMachine<NoTrace>,
    x: u32,
    y: u32,
    expected: u32,
    timeout_millis: u64,
) -> bool {
    let mut elapsed = 0u64;
    while elapsed < timeout_millis {
        if pixel_rgba(machine, x, y) == expected {
            return true;
        }
        run_millis(machine, 20);
        elapsed += 20;
    }
    pixel_rgba(machine, x, y) == expected
}

/// Asserts the rendered dimensions of the current step.
fn assert_dims(machine: &AtMachine<NoTrace>, step: u8, expected: (u32, u32)) {
    assert_eq!(
        machine.display_dimensions(),
        expected,
        "step {step:02X}: wrong display dimensions"
    );
}

/// Per-step assertions of the mode walk.
fn assert_step(machine: &mut AtMachine<NoTrace>, step: u8) {
    match step {
        0x03 | 0x01 => {
            let width = if step == 0x03 { 720 } else { 360 };
            assert_dims(machine, step, (width, 400));
            // Cell 0: full block glyph, yellow on blue, 9th dot replicated.
            assert_eq!(pixel_rgba(machine, 0, 0), YELLOW, "step {step:02X}");
            assert_eq!(pixel_rgba(machine, 8, 0), YELLOW, "step {step:02X}");
            // Cell 1: space on blue; cell 2: space on red.
            assert_eq!(pixel_rgba(machine, 9, 0), BLUE, "step {step:02X}");
            assert_eq!(pixel_rgba(machine, 18, 0), RED, "step {step:02X}");
            // The hardware cursor at row 4 renders a solid block in the
            // foreground color of its (empty) cell within its blink phase.
            let cursor_y = 4 * 16 + 13;
            assert!(
                wait_for_pixel(machine, 0, cursor_y, LIGHT_GRAY, 2_000),
                "step {step:02X}: cursor block did not appear"
            );
            assert_eq!(pixel_rgba(machine, 8, cursor_y), LIGHT_GRAY);
            assert_eq!(pixel_rgba(machine, 0, cursor_y - 1), BLACK);
        }
        0x04 | 0x05 => {
            assert_dims(machine, step, (320, 400));
            let pens = pens(machine);
            let logical = (320, 200);
            // Byte 0x1B fills the screen with repeating 2bpp bars 0,1,2,3.
            for (row_label, y) in [("top", 0u32), ("interleaved", 1), ("bottom", 199)] {
                for value in 0..4u32 {
                    assert_eq!(
                        mode_pixel(machine, logical, value, y),
                        pens[value as usize],
                        "step {step:02X}: {row_label} bar pixel {value}"
                    );
                }
                assert_eq!(mode_pixel(machine, logical, 4, y), pens[0]);
                assert_eq!(mode_pixel(machine, logical, 319, y), pens[3]);
            }
            assert_ne!(pens[1], pens[0], "step {step:02X}: flat palette");
        }
        0x06 => {
            assert_dims(machine, step, (640, 400));
            let pens = pens(machine);
            let logical = (640, 200);
            // Byte 0xAA fills the screen with 1bpp bars 1,0.
            for y in [0u32, 1, 199] {
                assert_eq!(mode_pixel(machine, logical, 0, y), pens[1]);
                assert_eq!(mode_pixel(machine, logical, 1, y), pens[0]);
                assert_eq!(mode_pixel(machine, logical, 638, y), pens[1]);
            }
        }
        0x0D | 0x0E | 0x10 | 0x12 => {
            let logical = match step {
                0x0D => (320, 200),
                0x0E => (640, 200),
                0x10 => (640, 350),
                _ => (640, 480),
            };
            let rendered_height = if logical.1 < 350 { 400 } else { logical.1 };
            assert_dims(machine, step, (logical.0, rendered_height));
            let pens = pens(machine);
            // Full-screen 16-color bars: colors 7,6,..,0 then 15,14,..,8 across
            // each sixteen pixel unit.
            for y in [0u32, logical.1 - 1] {
                for (x, index) in [(0u32, 7usize), (7, 0), (8, 15), (15, 8), (16, 7)] {
                    assert_eq!(
                        mode_pixel(machine, logical, x, y),
                        pens[index],
                        "step {step:02X}: bar pixel {x} at y {y}"
                    );
                }
            }
            assert_ne!(pens[7], pens[0], "step {step:02X}: flat palette");
        }
        0x0F | 0x11 => {
            let logical = if step == 0x0F { (640, 350) } else { (640, 480) };
            assert_dims(machine, step, logical);
            let top = mode_pixel(machine, logical, 0, 0);
            assert_eq!(
                mode_pixel(machine, logical, 0, logical.1 - 1),
                top,
                "step {step:02X}: bars are not vertical"
            );
            let mut shades = 0;
            for x in [0u32, 3, 7, 11, 15] {
                if mode_pixel(machine, logical, x, 0) != top {
                    shades += 1;
                }
            }
            assert!(shades > 0, "step {step:02X}: the screen is a flat color");
        }
        0x13 => {
            assert_dims(machine, step, (640, 400));
            let logical = (320, 200);
            let palette = pens_256(machine);
            // Full-screen position ramp: color = (linear offset) & 0xFF.
            let ramp = |x: u32, y: u32| palette[((y * 320 + x) & 0xFF) as usize];
            for (x, y) in [(0u32, 0u32), (1, 0), (255, 0), (0, 1), (0, 199), (319, 199)] {
                assert_eq!(
                    mode_pixel(machine, logical, x, y),
                    ramp(x, y),
                    "step 13: ramp at ({x}, {y})"
                );
            }
        }
        STEP_MODE_X => {
            assert_dims(machine, step, (640, 400));
            let logical = (320, 200);
            let palette = pens_256(machine);
            // Page one holds a ramp starting at color 1 (four pixel blocks from
            // the unchained addressing); the flat color-5 page zero must not
            // show through after the start address flip.
            let block = |x: u32, y: u32| palette[((1 + y * 80 + x / 4) & 0xFF) as usize];
            assert_eq!(mode_pixel(machine, logical, 0, 0), palette[1]);
            assert_eq!(mode_pixel(machine, logical, 3, 0), palette[1]);
            assert_eq!(mode_pixel(machine, logical, 4, 0), palette[2]);
            assert_eq!(mode_pixel(machine, logical, 0, 199), block(0, 199));
            assert_ne!(palette[1], palette[5], "step F0: page zero showed through");
        }
        STEP_SPLIT_PAN => {
            assert_dims(machine, step, (320, 400));
            let pens = pens(machine);
            // Above the split: the start address shows host byte 41 (dot 8 of
            // memory row one) shifted left by the pel pan of four.
            assert_eq!(pixel_rgba(machine, 4, 0), pens[1], "panned marker");
            assert_eq!(pixel_rgba(machine, 8, 0), pens[0]);
            // Below the split at scanline 100: address zero, panning reset.
            assert_eq!(pixel_rgba(machine, 0, 100), pens[1], "split marker");
            assert_eq!(pixel_rgba(machine, 1, 100), pens[0]);
            assert_eq!(pixel_rgba(machine, 0, 99), pens[0]);
        }
        0x2E..=0x30 => {
            let (width, height) = match step {
                0x2F => (640, 400),
                0x2E => (640, 480),
                _ => (800, 600),
            };
            assert_dims(machine, step, (width, height));
            let palette = pens_256(machine);
            // Full-screen position ramp across the banked framebuffer: color =
            // linear offset & 0xFF. Sampling on both sides of the 64 KiB bank
            // boundaries proves the scan-out crosses them without a seam.
            let ramp = |offset: u32| {
                let (x, y) = (offset % width, offset / width);
                (pixel_rgba(machine, x, y), palette[(offset & 0xFF) as usize])
            };
            for offset in [0u32, 0xFFFF, 0x10000, 0x10001, 0x20000, 0x30000] {
                let (got, expected) = ramp(offset);
                assert_eq!(
                    got, expected,
                    "step {step:02X}: ramp at linear 0x{offset:X}"
                );
            }
            let last = (height - 1) * width;
            let (got, expected) = ramp(last);
            assert_eq!(got, expected, "step {step:02X}: last row ramp");
        }
        _ => panic!("unexpected step {step:02X}"),
    }
}

/// Whether a step's framebuffer is hashed against the known-good table. The text
/// modes carry a blinking cursor and attribute, so their framebuffer is not
/// deterministic for a single golden hash; they are verified pixel by pixel.
fn frame_hash_is_checked(step: u8) -> bool {
    !matches!(step, 0x03 | 0x01)
}

/// Known-good BLAKE3 digests of each graphics step's rendered framebuffer.
///
/// These are the digests the real ET4000 BIOS path produced: driving the modes
/// from the captured register files and palettes reproduces them byte for byte,
/// so a mismatch means the rendering output changed and is no longer pixel
/// identical with real hardware. Regenerate the table (after confirming the new
/// output is correct) with `NEETAN_RECORD_FRAME_HASHES=1`.
const EXPECTED_FRAME_HASHES: &[(u8, &str)] = &[
    (
        0x04,
        "030f16332c36f53b398650a9e3fb39aa5468043eee2068c73538692c10f272c1",
    ),
    (
        0x05,
        "030f16332c36f53b398650a9e3fb39aa5468043eee2068c73538692c10f272c1",
    ),
    (
        0x06,
        "cc03d500c962651df3cc2f8f88e0583ca248de135aff2c85ad20aeb2f6c628d6",
    ),
    (
        0x0D,
        "e61929d4192d66757707a2771cb8fc405c8c9325f2226153b40b703df0ff4a53",
    ),
    (
        0x0E,
        "3829c2c7d711dd3c23ed935c367c3a93393811b91921ee381dd3fd0a16817e13",
    ),
    (
        0x0F,
        "b0c2c51708268ac5e51837e5a6f877cd8e11a471fc1d8ecd568fd6339a04ed3a",
    ),
    (
        0x10,
        "083eb7d58833ae7a5887ed2c2856061da4d23b26f632a152b1097513636084e9",
    ),
    (
        0x11,
        "15dd0609b2a820ac1a91940bfa832053f0948080d0a9db31dba201b1f80eab2e",
    ),
    (
        0x12,
        "68e914994fd715d9c617a09c783a60ea174cd4144125b7a680cd7b0645abdf0f",
    ),
    (
        0x13,
        "66825eb65c3b2e5396111c7c0a1e89f53ec2655fcf7d99a21a4e59ba8c05a434",
    ),
    (
        STEP_MODE_X,
        "ce7ba90a478d4a58260404c2f78083d3a001058e2f017a5e0a81f82e3229f99b",
    ),
    (
        STEP_SPLIT_PAN,
        "f54f1db4cd781f1e121a50e9edf5c844d31189c12682dc3eb49b474d24600b80",
    ),
    (
        0x2F,
        "e733ff95f1b39d50648ff6c4788ad16fb31c7353da55b2eaf2f568c14f352fee",
    ),
    (
        0x2E,
        "a0b47f4f2ce8c5e4a5554055a296ec712b78de72a3349bf1b5269a1fa232b2d3",
    ),
    (
        0x30,
        "49f8c6fc6cedde77c76324e5c2ed8f7597e30e1d615f883579a5321f0d2c2717",
    ),
];

/// Checks (or records) the framebuffer hash of a graphics step.
fn check_frame_hash(machine: &AtMachine<NoTrace>, step: u8) {
    if !frame_hash_is_checked(step) {
        return;
    }
    let hash = framebuffer_hash(machine);
    if std::env::var("NEETAN_RECORD_FRAME_HASHES").is_ok() {
        eprintln!("    ({step:#04X}, \"{hash}\"),");
        return;
    }
    let expected = EXPECTED_FRAME_HASHES
        .iter()
        .find(|(hashed_step, _)| *hashed_step == step)
        .map(|(_, hash)| *hash);
    assert_eq!(
        Some(hash.as_str()),
        expected,
        "step {step:02X}: framebuffer hash changed; the rendering is no longer \
         pixel identical with the known-good output (regenerate with \
         NEETAN_RECORD_FRAME_HASHES=1 once the new output is verified)"
    );
}

/// Dumps the frame as PNG when `NEETAN_DUMP_FRAMES` names a directory.
fn dump_frame(machine: &AtMachine<NoTrace>, step: u8) {
    let Ok(directory) = std::env::var("NEETAN_DUMP_FRAMES") else {
        return;
    };
    let (width, height) = machine.display_dimensions();
    let path = std::path::PathBuf::from(directory).join(format!("step_{step:02X}.png"));
    software_renderer::SoftwareRenderer::write_png(
        &path,
        machine.display_framebuffer(),
        width,
        height,
    )
    .expect("write the frame dump");
    eprintln!("frame dump: {}", path.display());
}

/// Builds a fresh machine, sets up the step and renders one settled frame.
fn render_step(model: AtModel, step: u8) -> AtMachine<NoTrace> {
    let mut machine = machine_for_model(model);
    setup_step(&mut machine.bus, step);
    render_frame(&mut machine);
    machine
}

/// Walks every mode step on the given model.
fn walk_all_modes(model: AtModel) {
    for &step in STEPS {
        let mut machine = render_step(model, step);
        let (width, height) = machine.display_dimensions();
        eprintln!("step {step:02X}: {width}x{height}");
        dump_frame(&machine, step);
        assert_step(&mut machine, step);
        check_frame_hash(&machine, step);
    }
}

#[test]
fn all_video_modes_render_expected_patterns() {
    walk_all_modes(AtModel::At486Dx66);
}

#[test]
fn all_video_modes_render_expected_patterns_on_the_dx50() {
    walk_all_modes(AtModel::At486Dx50);
}

#[test]
fn text_blink_and_cursor_phases_alternate() {
    let mut machine = render_step(AtModel::At486Dx66, 0x03);

    // Cell 3 blinks ('B', attribute 0x87); cell 0 is a steady full block.
    let blink_region = |machine: &AtMachine<NoTrace>| {
        let mut lit = 0u32;
        for y in 0..16u32 {
            for x in 27..36u32 {
                if pixel_rgba(machine, x, y) == LIGHT_GRAY {
                    lit += 1;
                }
            }
        }
        lit
    };
    let cursor_y = 4 * 16 + 13;
    let mut saw_blink_on = false;
    let mut saw_blink_off = false;
    let mut saw_cursor_on = false;
    let mut saw_cursor_off = false;
    for _ in 0..60 {
        run_millis(&mut machine, 20);
        if blink_region(&machine) > 0 {
            saw_blink_on = true;
        } else {
            saw_blink_off = true;
        }
        if pixel_rgba(&machine, 0, cursor_y) == LIGHT_GRAY {
            saw_cursor_on = true;
        } else {
            saw_cursor_off = true;
        }
        // The steady cell keeps its color in every phase.
        assert_eq!(pixel_rgba(&machine, 0, 0), YELLOW);
    }
    assert!(
        saw_blink_on,
        "the blinking cell never showed its foreground"
    );
    assert!(saw_blink_off, "the blinking cell never blanked");
    assert!(saw_cursor_on, "the cursor never appeared");
    assert!(saw_cursor_off, "the cursor never blinked off");
}
