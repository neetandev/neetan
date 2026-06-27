//! Text layer integration tests: text VRAM access, the video controller register
//! file, the palette, and the text layer composited into the framebuffer, all
//! driven through the public bus surface.

use common::Bus;
use machine88va::Pc88VaMachine;

#[path = "common/harness.rs"]
mod harness;
use harness::*;

#[test]
fn text_vram_round_trips_through_the_sysm_window() {
    let mut machine = machine();
    // Reset selects sysm bank 1 (text VRAM) at 0xA0000-0xDFFFF.
    machine.bus.write_byte(0xA_0000, 0xAB);
    machine.bus.write_byte(0xB_2345, 0xCD);
    assert_eq!(machine.bus.read_byte(0xA_0000), 0xAB);
    assert_eq!(machine.bus.read_byte(0xB_2345), 0xCD);
}

#[test]
fn video_registers_read_back() {
    let mut machine = machine();
    machine.bus.io_write_byte(0x100, 0x34); // grmode low
    machine.bus.io_write_byte(0x101, 0x12); // grmode high
    assert_eq!(machine.bus.io_read_byte(0x100), 0x34);
    assert_eq!(machine.bus.io_read_byte(0x101), 0x12);

    machine.bus.io_write_byte(0x10C, 0x55); // palmode low
    machine.bus.io_write_byte(0x10D, 0x00);
    assert_eq!(machine.bus.io_read_byte(0x10C), 0x55);

    // A write-only register reads open bus.
    machine.bus.io_write_byte(0x106, 0x99);
    assert_eq!(machine.bus.io_read_byte(0x106), 0xFF);
}

/// Programs a minimal visible text screen: one 80x... text frame, a single
/// glyph cell, the text layer enabled, a palette and a backdrop.
fn program_text_screen(machine: &mut Pc88VaMachine) {
    // Frame-0 descriptor at text-table offset 0 (CPU 0xA0000).
    let put_word = |machine: &mut Pc88VaMachine, address: u32, value: u16| {
        machine.bus.write_byte(address, value as u8);
        machine.bus.write_byte(address + 1, (value >> 8) as u8);
    };
    put_word(machine, 0xA_0008, 160); // vw
    put_word(machine, 0xA_000A, 0x0000); // mode 0
    machine.bus.write_byte(0xA_000D, 0); // raster offset
    put_word(machine, 0xA_0010, 0x1000); // rsa
    put_word(machine, 0xA_0014, 0x01FE); // rh
    put_word(machine, 0xA_0016, 640); // rw
    put_word(machine, 0xA_001A, 0); // rxp

    // Character 'A' (0x41) in cell 0, attribute (mode 0) bg=0 fg=2.
    put_word(machine, 0xA_1000, 0x0041);
    machine.bus.write_byte(0xA_1000 + 0x2000, 0x02);

    // Palette entry 2 = pure red; backdrop = pure blue.
    machine.bus.io_write_byte(0x304, 0xE0); // palette[2] low (0x03e0)
    machine.bus.io_write_byte(0x305, 0x03); // palette[2] high
    machine.bus.io_write_byte(0x10E, 0x1F); // dropcol low (0x001f)
    machine.bus.io_write_byte(0x10F, 0x00);

    // Enable text layer on screen 0 and the video output.
    machine.bus.io_write_byte(0x106, 0x08); // colcomp: screen 0 = text
    machine.bus.io_write_byte(0x100, 0x00); // grmode low
    machine.bus.io_write_byte(0x101, 0x30); // grmode high: XVSP | SYNCEN

    // TSP text setup: DSPDEF (attroffset 0x2000, line height 16), DSPON
    // (texttable 0), SYNC (400 screen lines).
    tsp_command(machine, 0x14, &[0x00, 0x20, 0x00, 15, 0x00, 0x00]);
    tsp_command(machine, 0x12, &[0x00, 0x00, 0x00]);
    let mut sync = [0u8; 14];
    sync[0x0A] = 0x90;
    sync[0x0B] = 0x40;
    tsp_command(machine, 0x10, &sync);
}

#[test]
fn text_layer_composites_into_the_framebuffer() {
    let mut machine = machine();
    program_text_screen(&mut machine);
    render_one_frame(&mut machine);

    assert_eq!(machine.display_dimensions(), (640, 400));

    let red = va_rgba(0x03E0);
    let blue = va_rgba(0x001F);
    let framebuffer = machine.display_framebuffer();

    // A pixel far from any text shows the backdrop.
    assert_eq!(pixel(framebuffer, 320, 200), blue);

    // The first character cell carries glyph foreground (palette[2]) somewhere
    // in its 8x16 area, proving the text layer composited.
    let mut saw_foreground = false;
    for y in 0..16 {
        for x in 0..8 {
            if pixel(framebuffer, x, y) == red {
                saw_foreground = true;
            }
        }
    }
    assert!(
        saw_foreground,
        "glyph foreground must composite into the frame"
    );
}

#[test]
fn backdrop_color_register_drives_composed_pixels() {
    let mut machine = machine();
    program_text_screen(&mut machine);
    // Change the backdrop to pure green before rendering.
    machine.bus.io_write_byte(0x10E, 0x00);
    machine.bus.io_write_byte(0x10F, 0xFC); // 0xfc00
    render_one_frame(&mut machine);

    assert_eq!(
        pixel(machine.display_framebuffer(), 320, 200),
        va_rgba(0xFC00)
    );
}
