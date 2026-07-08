//! Base-X1 video composition tests.

mod harness;

use harness::{build_machine_with_synthetic_roms, pixel, program_standard_crtc, run_bus_cycles};
use machinex1::X1Model;

const WHITE: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
const BLACK: [u8; 4] = [0x00, 0x00, 0x00, 0xFF];
const BLUE: [u8; 4] = [0x00, 0x00, 0xFF, 0xFF];

/// Runs one frame so the VBlank event composites the framebuffer.
fn render_one_frame(machine: &mut machinex1::X1Machine) {
    let frame = u64::from(machine.bus.cpu_clock_hz()) / 60 + 1;
    run_bus_cycles(&mut machine.bus, frame);
}

#[test]
fn ank_text_renders_a_glyph_in_the_cell_pen() {
    // CG-ROM glyph 1: a fully-lit top row (glyph 1, row 0).
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1, |roms| {
        roms.cgrom_8x8[8] = 0xFF;
    });

    program_standard_crtc(&mut machine.bus);
    // Cell 0: character 1, white pen (7), ANK, no reverse.
    machine.bus.io_write(0x3000, 1);
    machine.bus.io_write(0x2000, 0x07);

    render_one_frame(&mut machine);
    let framebuffer = machine.bus.display_framebuffer();

    assert_eq!(machine.bus.display_dimensions(), (640, 200));
    assert_eq!(pixel(framebuffer, 0, 0), WHITE);
    assert_eq!(pixel(framebuffer, 7, 0), WHITE);
    // Second row of the cell is background.
    assert_eq!(pixel(framebuffer, 0, 1), BLACK);
}

#[test]
fn priority_register_swaps_text_and_bitmap() {
    let render_priority = |priority: u8| {
        let mut machine = build_machine_with_synthetic_roms(X1Model::X1, |_| {});
        program_standard_crtc(&mut machine.bus);

        // Opaque reversed text (pen 0 reversed -> white) in cell 0.
        machine.bus.io_write(0x3000, 0);
        machine.bus.io_write(0x2000, 0x08);
        // Bitmap blue plane lit in cell 0 row 0 (I/O window 0x4000), and the
        // blue gun latched for pen 1.
        machine.bus.io_write(0x4000, 0xFF);
        machine.bus.io_write(0x1000, 0x02);
        machine.bus.io_write(0x1300, priority);

        render_one_frame(&mut machine);
        pixel(machine.bus.display_framebuffer(), 0, 0)
    };

    let text_pixel = render_priority(0x00);
    let bitmap_pixel = render_priority(0xFF);

    assert_eq!(text_pixel, WHITE);
    assert_eq!(bitmap_pixel, BLUE);
}

#[test]
fn palette_gun_writes_change_bitmap_colour() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1, |_| {});
    program_standard_crtc(&mut machine.bus);

    // Bitmap red plane lit in cell 0 (I/O window plane R at 0x8000) -> pen 2.
    machine.bus.io_write(0x8000, 0xFF);
    // Bitmap over text everywhere.
    machine.bus.io_write(0x1300, 0xFF);
    // Latch the red gun for pen 2.
    machine.bus.io_write(0x1100, 0x04);

    render_one_frame(&mut machine);
    assert_eq!(
        pixel(machine.bus.display_framebuffer(), 0, 0),
        [0xFF, 0x00, 0x00, 0xFF]
    );
}

#[test]
fn bitmap_writes_after_hblank_affect_the_next_frame_for_that_line() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1, |_| {});
    program_standard_crtc(&mut machine.bus);

    machine.bus.io_write(0x1000, 0x02); // blue gun for bitmap pen 1
    machine.bus.io_write(0x1300, 0xFF); // bitmap over text

    // Line 0 is latched at the start of its horizontal blank, around cycle 200
    // for the test CRTC geometry. Writing row-0 bitmap data after that point
    // must not change the frame that is already being scanned out.
    run_bus_cycles(&mut machine.bus, 220);
    machine.bus.io_write(0x4000, 0xFF);
    run_bus_cycles(&mut machine.bus, 50_000);
    assert_eq!(pixel(machine.bus.display_framebuffer(), 0, 0), BLACK);

    run_bus_cycles(&mut machine.bus, 50_000);
    assert_eq!(pixel(machine.bus.display_framebuffer(), 0, 0), BLUE);
}
