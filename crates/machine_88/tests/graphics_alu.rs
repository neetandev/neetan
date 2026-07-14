//! Graphics integration tests: GVRAM plane and ALU writes via I/O, the graphics
//! display modes, and text-over-graphics compositing with the layer-disable flags.

mod harness;

use harness::{build_machine_with, build_machine_with_synthetic_roms};
use machine_88::Pc8801Bus;

const PC88_WIDTH: usize = 640;

/// Offset of the 8x8 ANK font inside the kanji1 ROM, where the renderer reads
/// alphanumeric glyphs.
const ANK_FONT_OFFSET: usize = 0x1000;

/// Writes an 8x8 glyph for character `code` into a synthetic kanji1 ROM.
fn write_ank_glyph(kanji1: &mut [u8], code: u8, rows: [u8; 8]) {
    let base = ANK_FONT_OFFSET + usize::from(code) * 8;
    kanji1[base..base + 8].copy_from_slice(&rows);
}

/// Advances the bus clock by `cycles`, processing every scheduled event in order.
fn run_bus_cycles(bus: &mut Pc8801Bus, cycles: u64) {
    let end = bus.current_cycle() + cycles;
    loop {
        let next = bus.next_event_cycle().unwrap_or(end);
        if next >= end {
            bus.set_current_cycle(end);
            break;
        }
        bus.set_current_cycle(next);
    }
}

/// Programs the CRTC geometry and starts the display.
fn program_crtc(bus: &mut Pc8801Bus, columns: u8, rows: u8, char_height: u8) {
    bus.io_write(0x51, 0x00); // RESET
    bus.io_write(0x50, columns - 2);
    bus.io_write(0x50, rows - 1);
    bus.io_write(0x50, char_height - 1);
    bus.io_write(0x50, 0);
    bus.io_write(0x50, 0); // one attribute strip
    bus.io_write(0x51, 0x20); // START DISPLAY
}

/// Programs text DMA channel 2 to read `byte_count` bytes from `address`.
fn program_text_dma(bus: &mut Pc8801Bus, address: u16, byte_count: u16) {
    bus.io_write(0x68, 0x80 | 0x04); // autoload + enable channel 2
    bus.io_write(0x64, (address & 0xFF) as u8);
    bus.io_write(0x64, (address >> 8) as u8);
    let count = byte_count - 1;
    bus.io_write(0x65, (count & 0xFF) as u8);
    bus.io_write(0x65, ((count >> 8) & 0x3F) as u8);
}

fn pixel(framebuffer: &[u8], x: usize, y: usize) -> [u8; 3] {
    let index = (y * PC88_WIDTH + x) * 4;
    [
        framebuffer[index],
        framebuffer[index + 1],
        framebuffer[index + 2],
    ]
}

fn count_color(
    framebuffer: &[u8],
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    rgb: [u8; 3],
) -> usize {
    let mut count = 0;
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            if pixel(framebuffer, x, y) == rgb {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn color8_renders_pen_from_three_planes() {
    let mut machine = build_machine_with(|_| {});
    let bus = &mut machine.bus;

    bus.io_write(0x53, 0x01); // disable text layer
    bus.io_write(0x31, 0x18); // GRPHE | HCOLOR -> 640x200 8-color
    bus.io_write(0x55, 0x01); // pen 1 -> blue (digital)
    bus.io_write(0x5C, 0); // select blue plane
    bus.poke_byte(0xC000, 0x80); // pixel (0,0): blue bit -> pen 1

    program_crtc(bus, 80, 25, 8);
    run_bus_cycles(bus, 200_000);

    assert_eq!(bus.display_dimensions(), (640, 200));
    let framebuffer = bus.display_framebuffer();
    assert_eq!(pixel(framebuffer, 0, 0), [0, 0, 255], "pen 1 is blue");
    assert_eq!(pixel(framebuffer, 1, 0), [0, 0, 0], "pen 0 is black");
}

#[test]
fn alu_logic_op_write_and_read_combine() {
    let mut machine = build_machine_with(|_| {});
    let bus = &mut machine.bus;

    bus.io_write(0x53, 0x01); // disable text layer
    bus.io_write(0x31, 0x18); // 640x200 8-color
    bus.io_write(0x5B, 0x07); // pen 7 -> white (digital)
    bus.io_write(0x32, 0x40); // GVAM: route 0xC000-0xFFFF through the ALU
    bus.io_write(0x34, 0x07); // alu_ctrl1: SET on all three planes
    bus.io_write(0x35, 0x87); // GAM + all planes normal, GDM = 0 (logic op)

    bus.poke_byte(0xC000, 0xF0); // ALU set: each plane |= 0xF0

    // Read-combine returns blue & red & green (all normal here).
    assert_eq!(bus.peek_byte(0xC000), 0xF0, "ALU read combines the planes");

    program_crtc(bus, 80, 25, 8);
    run_bus_cycles(bus, 200_000);

    let framebuffer = bus.display_framebuffer();
    // Top nibble set in all planes -> pen 7 (white); low nibble clear -> pen 0.
    assert_eq!(pixel(framebuffer, 0, 0), [255, 255, 255], "pen 7 (white)");
    assert_eq!(pixel(framebuffer, 4, 0), [0, 0, 0], "pen 0 (black)");
}

#[test]
fn alu_copy_modes_use_latched_read_registers() {
    let mut machine = build_machine_with(|_| {});
    let bus = &mut machine.bus;

    bus.io_write(0x5C, 0);
    bus.poke_byte(0xC000, 0x12);
    bus.io_write(0x5D, 0);
    bus.poke_byte(0xC000, 0xA5);

    bus.io_write(0x32, 0x40);
    bus.io_write(0x35, 0x87);
    assert_eq!(bus.peek_byte(0xC000), 0x00);

    bus.io_write(0x35, 0xA0);
    bus.poke_byte(0xC000, 0x00);
    bus.io_write(0x32, 0x00);
    bus.io_write(0x5C, 0);
    assert_eq!(bus.peek_byte(0xC000), 0xA5);

    bus.io_write(0x32, 0x40);
    bus.io_write(0x35, 0xB0);
    bus.poke_byte(0xC000, 0x00);
    bus.io_write(0x32, 0x00);
    bus.io_write(0x5D, 0);
    assert_eq!(bus.peek_byte(0xC000), 0x12);
}

#[test]
fn gvam_access_resets_independent_plane_select() {
    let mut machine = build_machine_with(|_| {});
    let bus = &mut machine.bus;

    bus.io_write(0x5C, 0);
    bus.poke_byte(0xC000, 0x12);

    bus.io_write(0x32, 0x40);
    bus.io_write(0x32, 0x00);
    bus.poke_byte(0xC000, 0x34);

    bus.io_write(0x5C, 0);
    assert_eq!(bus.peek_byte(0xC000), 0x12);
}

#[test]
fn attrib400_mode_reports_400_lines() {
    let mut machine = build_machine_with(|_| {});
    let bus = &mut machine.bus;

    bus.io_write(0x31, 0x08); // GRPHE only -> 640x400 attribute mode
    program_crtc(bus, 80, 25, 8);
    run_bus_cycles(bus, 300_000);

    assert_eq!(bus.display_dimensions(), (640, 400));
}

#[test]
fn text_over_graphics_and_layer_disable() {
    // Synthetic font: an 'A' glyph with both lit and blank pixels so graphics
    // can show through the blanks.
    let mut machine = build_machine_with_synthetic_roms(|roms| {
        write_ank_glyph(
            &mut roms.kanji1,
            b'A',
            [0x18, 0x3C, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x00],
        );
    });
    let bus = &mut machine.bus;

    let columns = 80u8;
    let rows = 25u8;
    let char_height = 8u8;
    let stride = 80 + 2; // one attribute strip pair

    // Program the text screen with 'A' at cell (0,0); the CRTC default attribute
    // renders it white.
    program_crtc(bus, columns, rows, char_height);
    bus.write_tvram(0, b"A");
    bus.write_tvram(80, &[0, 0]);
    program_text_dma(bus, 0xF000, stride * u16::from(rows));

    // 8-color graphics behind the text: pen 1 (blue), the (0,0) cell all set.
    bus.io_write(0x31, 0x18); // GRPHE | HCOLOR
    bus.io_write(0x55, 0x01); // pen 1 -> blue
    bus.io_write(0x5C, 0); // select blue plane
    for y in 0..u16::from(char_height) {
        bus.poke_byte(0xC000 + y * 80, 0xFF);
    }

    run_bus_cycles(bus, 200_000);

    let framebuffer = bus.display_framebuffer();
    let white = count_color(framebuffer, 0, 0, 8, 8, [255, 255, 255]);
    let blue = count_color(framebuffer, 0, 0, 8, 8, [0, 0, 255]);
    assert!(
        white > 0,
        "the 'A' glyph draws white text over the graphics"
    );
    assert!(blue > 0, "graphics show through the glyph's blank pixels");

    // Disabling the text layer (port 0x53 bit 0) leaves only graphics.
    bus.io_write(0x53, 0x01);
    run_bus_cycles(bus, 200_000);

    let framebuffer = bus.display_framebuffer();
    let white = count_color(framebuffer, 0, 0, 8, 8, [255, 255, 255]);
    let blue = count_color(framebuffer, 0, 0, 8, 8, [0, 0, 255]);
    assert_eq!(white, 0, "text layer disabled removes the glyph");
    assert_eq!(blue, 64, "the whole cell is graphics blue");
}
