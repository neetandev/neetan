//! Sprite-layer integration tests: the sprite layer composited through the
//! public bus surface, plus the TSP sprite-table command path (SPRDEF stream and
//! the CURDEF cursor-enable write into text VRAM).

use common::Bus;
use machine_88va::Pc88VaMachine;

#[path = "common/harness.rs"]
mod harness;
use harness::*;

/// Text VRAM base in the CPU address space (sysm bank 1 after reset).
const TEXT_VRAM_BASE: u32 = 0xA_0000;
/// Sprite-table base offset within text VRAM.
const SPRITE_TABLE: u32 = 0x7E00;

/// Writes a 16-bit value to a memory address pair.
fn write_word(machine: &mut Pc88VaMachine, address: u32, value: u16) {
    machine.bus.write_byte(address, value as u8);
    machine.bus.write_byte(address + 1, (value >> 8) as u8);
}

/// Programs a 24.8 kHz / 400-line screen with the sprite layer on screen 0.
fn program_sprite_screen(machine: &mut Pc88VaMachine) {
    // Palette entry 1 = pure green; backdrop = pure blue.
    machine.bus.io_write_byte(0x302, 0x00);
    machine.bus.io_write_byte(0x303, 0xFC); // palette[1] = 0xfc00
    machine.bus.io_write_byte(0x10E, 0x1F); // dropcol = 0x001f
    machine.bus.io_write_byte(0x10F, 0x00);

    // Screen 0 = sprite layer; text/sprite boundary color 15 so sprite codes
    // route to the sprite layer; code 0 stays transparent.
    machine.bus.io_write_byte(0x106, 0x09); // colcomp: screen 0 = sprite
    machine.bus.io_write_byte(0x110, 0x00); // pagemsk low
    machine.bus.io_write_byte(0x111, 0xF0); // pagemsk high -> boundary 15
    machine.bus.io_write_byte(0x12E, 0x00); // xpar_txtspr low (bit 0 forced set)
    machine.bus.io_write_byte(0x12F, 0x00);

    // Enable the video output.
    machine.bus.io_write_byte(0x100, 0x00); // grmode low
    machine.bus.io_write_byte(0x101, 0x30); // grmode high: XVSP | SYNCEN

    // 400 screen lines through SYNC.
    let mut sync = [0u8; 14];
    sync[0x0A] = 0x90;
    sync[0x0B] = 0x40;
    tsp_command(machine, 0x10, &sync);
}

#[test]
fn sprite_layer_composites_through_io() {
    let mut machine = machine();
    program_sprite_screen(&mut machine);

    // Sprite bitmap data at text VRAM 0x8000 (spda word 0x4000): one byte whose
    // high nibble is palette index 1 at x0.
    machine.bus.write_byte(TEXT_VRAM_BASE + 0x8000, 0x10);

    // Enable sprite display with the table at 0x7E00 (SPRON sets sprtable, which
    // the following SPRDEF stream writes relative to).
    tsp_command(&mut machine, 0x82, &[0x7E, 0x00, 0x00]);

    // Sprite-table entry 0 written through the SPRDEF stream: enabled, vlines
    // code 0 (=4), yp 0 | xp 0, 4 bytes, 16-color | spda 0x4000 | fg/bg unused.
    tsp_command(
        &mut machine,
        0x84,
        &[
            0x00, // SPRDEF offset within the table
            0x00, 0x02, // word 0
            0x00, 0x00, // word 1
            0x00, 0x40, // word 2 (spda 0x4000)
            0x00, 0x00, // word 3
        ],
    );
    machine.bus.io_write_byte(0x142, 0x88); // EXIT ends the SPRDEF stream

    render_one_frame(&mut machine);
    assert_eq!(machine.display_dimensions(), (640, 400));

    let framebuffer = machine.display_framebuffer();
    // The sprite pixel composites as palette[1] (green); a neighbouring
    // transparent pixel falls through to the backdrop (blue).
    assert_eq!(pixel(framebuffer, 0, 0), va_rgba(0xFC00));
    assert_eq!(pixel(framebuffer, 320, 200), va_rgba(0x001F));
}

#[test]
fn sprdef_stream_writes_the_sprite_table() {
    let mut machine = machine();
    // Set the table base via SPRON, then stream two bytes at offset 0x04.
    tsp_command(&mut machine, 0x82, &[0x7E, 0x00, 0x00]);
    tsp_command(&mut machine, 0x84, &[0x04, 0xAB, 0xCD]);

    assert_eq!(
        machine.bus.read_byte(TEXT_VRAM_BASE + SPRITE_TABLE + 0x04),
        0xAB
    );
    assert_eq!(
        machine.bus.read_byte(TEXT_VRAM_BASE + SPRITE_TABLE + 0x05),
        0xCD
    );
}

#[test]
fn curdef_sets_the_cursor_sprite_enable_bit() {
    let mut machine = machine();
    tsp_command(&mut machine, 0x82, &[0x7E, 0x00, 0x00]);

    // Cursor sprite 2 starts disabled (word 0 = 0).
    write_word(&mut machine, TEXT_VRAM_BASE + SPRITE_TABLE + 2 * 8, 0x0000);

    // CURDEF: curn = 2 (<<3), show cursor (bit 1), blink enable (bit 0).
    tsp_command(&mut machine, 0x15, &[(2 << 3) | 0x02 | 0x01]);

    // Word 0 bit 9 (the enable bit) is now set in text VRAM.
    let high = machine
        .bus
        .read_byte(TEXT_VRAM_BASE + SPRITE_TABLE + 2 * 8 + 1);
    assert_eq!(high & 0x02, 0x02);
}
