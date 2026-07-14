//! Text-display integration tests: the uPD3301 CRTC, the uPD8257 text DMA, and
//! the PC-88 text renderer.

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

/// Programs text DMA channel 2 to read `byte_count` bytes from `address` with
/// autoload, and enables the channel.
fn program_text_dma(bus: &mut Pc8801Bus, address: u16, byte_count: u16) {
    bus.io_write(0x68, 0x80 | 0x04); // autoload + enable channel 2
    bus.io_write(0x64, (address & 0xFF) as u8); // ch2 address low
    bus.io_write(0x64, (address >> 8) as u8); // ch2 address high
    let count = byte_count - 1;
    bus.io_write(0x65, (count & 0xFF) as u8); // ch2 count low
    bus.io_write(0x65, ((count >> 8) & 0x3F) as u8); // ch2 count high
}

#[test]
fn dma_expands_tvram_into_text_cells() {
    let mut machine = build_machine_with(|_| {});
    let bus = &mut machine.bus;

    let columns = 4u8;
    let rows = 2u8;
    let char_height = 8u8;
    let attribute_count = 1u16;
    let stride = 80 + attribute_count * 2;

    // RESET geometry: 4 columns, 2 rows, 8-line cells, retrace 1, transparent
    // monochrome with one attribute strip.
    bus.io_write(0x51, 0x00);
    bus.io_write(0x50, columns - 2);
    bus.io_write(0x50, rows - 1);
    bus.io_write(0x50, char_height - 1);
    bus.io_write(0x50, 0);
    bus.io_write(0x50, (attribute_count - 1) as u8);

    // Fill text VRAM: row 0 characters, then a one-pair attribute strip, then
    // row 1 characters.
    bus.write_tvram(0, b"ABCD");
    bus.write_tvram(80, &[0, 0]); // attribute strip: column 0, value 0
    bus.write_tvram(usize::from(stride), b"EFGH");
    bus.write_tvram(usize::from(stride) + 80, &[0, 0]);

    program_text_dma(bus, 0xF000, stride * u16::from(rows));
    bus.io_write(0x51, 0x20); // START DISPLAY

    run_bus_cycles(bus, 40_000);

    let text = bus.crtc_text_expand();
    assert_eq!(&text[0..4], b"ABCD");
    assert_eq!(&text[80..84], b"EFGH");
}

#[test]
fn renderer_draws_white_glyph_on_black() {
    // Synthetic font: an 'A' glyph that lights several pixels in the cell.
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
    let attribute_count = 1u16;
    let stride = 80 + attribute_count * 2;

    bus.io_write(0x51, 0x00);
    bus.io_write(0x50, columns - 2);
    bus.io_write(0x50, rows - 1);
    bus.io_write(0x50, char_height - 1);
    bus.io_write(0x50, 0);
    bus.io_write(0x50, (attribute_count - 1) as u8);

    // 'A' at cell (0, 0), white via the default 0xE0 attribute.
    bus.write_tvram(0, b"A");
    bus.write_tvram(80, &[0, 0]);

    program_text_dma(bus, 0xF000, stride * u16::from(rows));
    bus.io_write(0x51, 0x20); // START DISPLAY

    run_bus_cycles(bus, 200_000);

    assert_eq!(bus.display_dimensions(), (640, 200));

    // The 8x8 cell at (0, 0) must contain white glyph pixels.
    let framebuffer = bus.display_framebuffer();
    let white_pixels = count_white_pixels(framebuffer, 0, 0, 8, 8);
    assert!(
        white_pixels > 0,
        "expected the 'A' glyph to render white pixels, found {white_pixels}"
    );

    // A region well below the single character row stays background (black).
    let lit_below = count_white_pixels(framebuffer, 0, 100, PC88_WIDTH, 8);
    assert_eq!(lit_below, 0, "background region should be black");
}

/// Counts pixels in the given region whose RGB channels are all near-maximum.
fn count_white_pixels(
    framebuffer: &[u8],
    x0: usize,
    y0: usize,
    width: usize,
    height: usize,
) -> usize {
    let mut count = 0;
    for y in y0..y0 + height {
        for x in x0..x0 + width {
            let index = (y * PC88_WIDTH + x) * 4;
            if index + 2 < framebuffer.len()
                && framebuffer[index] > 200
                && framebuffer[index + 1] > 200
                && framebuffer[index + 2] > 200
            {
                count += 1;
            }
        }
    }
    count
}
