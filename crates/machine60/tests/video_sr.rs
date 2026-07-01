//! Native SR video tests: 20/25-row text geometry, the 4bpp bitmap with its pen
//! lookup, hardware scroll and bitmap offset, and the mkII-compatibility route.

use machine60::{Pc6000Bus, Pc6000Model};

mod harness;
use harness::{build_machine, build_machine_with_synthetic_roms, run_bus_cycles};

const WIDTH: usize = 320;
const SR_TEXT_BASE_ADDRESS: u16 = 0xE000;

/// Renders at least one full frame by letting the vertical-retrace event fire.
fn render_frame(bus: &mut Pc6000Bus) {
    let cycles = u64::from(bus.cpu_clock_hz()) / 30;
    run_bus_cycles(bus, cycles);
}

fn pixel(framebuffer: &[u8], x: usize, y: usize) -> [u8; 4] {
    let index = (y * WIDTH + x) * 4;
    framebuffer[index..index + 4].try_into().unwrap()
}

/// SR mode register (port 0xC8): bit 3 selects text, bit 2 selects 20 rows over
/// 25, bit 0 is the mkII-compatibility route.
const MODE_TEXT_20: u8 = 0x0C;
const MODE_TEXT_25: u8 = 0x08;
const MODE_BITMAP: u8 = 0x00;
const MODE_COMPAT: u8 = 0x09;

#[test]
fn sr_text_mode_draws_a_glyph_foreground() {
    let mut machine = build_machine_with_synthetic_roms(Pc6000Model::Pc6001Mk2Sr, |roms| {
        // Tile 1, line 0: only the leftmost pixel lit.
        roms.cg_sr.as_mut().unwrap()[0x10] = 0x80;
    });
    let bus = &mut machine.bus;

    bus.io_write(0xC8, MODE_TEXT_20);
    // Cell 0: tile 1, foreground pen 0x0F.
    bus.poke_byte(SR_TEXT_BASE_ADDRESS, 0x01);
    bus.poke_byte(SR_TEXT_BASE_ADDRESS + 1, 0x0F);
    render_frame(bus);

    let framebuffer = bus.display_framebuffer();
    assert_ne!(
        pixel(framebuffer, 0, 0),
        pixel(framebuffer, 1, 0),
        "the lit glyph column should differ from the unlit one"
    );
}

#[test]
fn sr_text_second_row_uses_the_column_stride() {
    let mut machine = build_machine_with_synthetic_roms(Pc6000Model::Pc6001Mk2Sr, |roms| {
        roms.cg_sr.as_mut().unwrap()[0x10] = 0x80;
    });
    let bus = &mut machine.bus;

    bus.io_write(0xC8, MODE_TEXT_20);
    // 40-column text: row 1, column 0 begins at cell (1 * 40) * 2 = 80.
    bus.poke_byte(SR_TEXT_BASE_ADDRESS + 80, 0x01);
    bus.poke_byte(SR_TEXT_BASE_ADDRESS + 81, 0x0F);
    render_frame(bus);

    let framebuffer = bus.display_framebuffer();
    // The glyph lands twelve scanlines down (one cell height).
    assert_ne!(pixel(framebuffer, 0, 12), pixel(framebuffer, 1, 12));
    // Row 0 stays blank.
    assert_eq!(pixel(framebuffer, 0, 0), pixel(framebuffer, 1, 0));
}

#[test]
fn sr_25_row_mode_renders_without_panicking() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    let bus = &mut machine.bus;
    bus.io_write(0xC8, MODE_TEXT_25);
    render_frame(bus);
    assert_eq!(bus.display_dimensions(), (320, 240));
}

#[test]
fn sr_bitmap_pen_maps_through_the_palette() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    let bus = &mut machine.bus;

    // Route page 0 to work RAM so the bitmap overlay catches the writes.
    bus.io_write(0x68, 0x00);
    bus.io_write(0xC8, MODE_BITMAP);
    bus.poke_byte(0x0000, 0x0F); // pixel (0,0): one pen
    bus.poke_byte(0x0001, 0x00); // pixel (1,0): another pen
    render_frame(bus);

    let framebuffer = bus.display_framebuffer();
    assert_ne!(
        pixel(framebuffer, 0, 0),
        pixel(framebuffer, 1, 0),
        "distinct nibbles map to distinct pens"
    );
}

#[test]
fn sr_bitmap_hardware_scroll_x_shifts_the_image() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    let bus = &mut machine.bus;

    bus.io_write(0x68, 0x00);
    bus.io_write(0xC8, MODE_BITMAP);
    bus.poke_byte(0x0000, 0x00); // gvram[0]
    bus.poke_byte(0x0001, 0x0F); // gvram[1]

    // Without scroll, pixel (0,0) reads gvram[0].
    render_frame(bus);
    let unscrolled = pixel(bus.display_framebuffer(), 0, 0);

    // Scrolling X by one brings gvram[1] to the origin.
    bus.io_write(0xCA, 0x01);
    render_frame(bus);
    let scrolled = pixel(bus.display_framebuffer(), 0, 0);

    assert_ne!(unscrolled, scrolled, "horizontal scroll shifted the image");
}

#[test]
fn sr_bitmap_offset_relocates_cpu_writes() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    let bus = &mut machine.bus;

    bus.io_write(0x68, 0x00);
    bus.io_write(0xC8, MODE_BITMAP);
    // Y offset of one scanline moves the write target down one bitmap row.
    bus.io_write(0xCE, 0x01);
    bus.poke_byte(0x0000, 0x0F);
    render_frame(bus);

    let framebuffer = bus.display_framebuffer();
    assert_eq!(
        pixel(framebuffer, 0, 0),
        pixel(framebuffer, 1, 0),
        "row 0 stays blank"
    );
    assert_ne!(
        pixel(framebuffer, 0, 1),
        pixel(framebuffer, 1, 1),
        "write landed on row 1"
    );
}

#[test]
fn sr_compat_mode_bypasses_the_native_text_path() {
    let mut machine = build_machine_with_synthetic_roms(Pc6000Model::Pc6001Mk2Sr, |roms| {
        roms.cg_sr.as_mut().unwrap()[0x10] = 0x80;
    });
    let bus = &mut machine.bus;

    // Native SR text content that would draw a distinct glyph at the origin.
    bus.io_write(0xC8, MODE_TEXT_20);
    bus.poke_byte(SR_TEXT_BASE_ADDRESS, 0x01);
    bus.poke_byte(SR_TEXT_BASE_ADDRESS + 1, 0x0F);

    // The compatibility bit routes rendering through the legacy mkII path, which
    // does not read the SR text window, so the origin glyph disappears.
    bus.io_write(0xC8, MODE_COMPAT);
    render_frame(bus);

    let framebuffer = bus.display_framebuffer();
    assert_eq!(
        pixel(framebuffer, 0, 0),
        pixel(framebuffer, 1, 0),
        "compat mode must not draw the native SR glyph"
    );
}
