//! X1 turbo video tests: 400-line hi-res, the kanji plane, the ANK 8x16 font,
//! and the double bitmap page.

mod harness;

use harness::{
    build_machine_with_synthetic_roms, pixel, program_hires_crtc, program_standard_crtc,
    run_bus_cycles,
};
use machine_x1::X1Model;

const WHITE: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
const BLACK: [u8; 4] = [0x00, 0x00, 0x00, 0xFF];
const RED: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];
const BLUE: [u8; 4] = [0x00, 0x00, 0xFF, 0xFF];

/// SCRN register (mirror 0x1FD0-0x1FDF).
const SCRN: u16 = 0x1FD0;

/// Runs one frame so the VBlank event composites the framebuffer.
fn render_one_frame(machine: &mut machine_x1::X1Machine) {
    let frame = u64::from(machine.bus.cpu_clock_hz()) / 30;
    run_bus_cycles(&mut machine.bus, frame);
}

/// Sets a CRTC register.
fn crtc(bus: &mut machine_x1::X1Bus, register: u8, value: u8) {
    bus.io_write(0x1800, register);
    bus.io_write(0x1801, value);
}

#[test]
fn hires_crtc_reports_400_line_dimensions() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |_| {});
    program_hires_crtc(&mut machine.bus);
    machine.bus.io_write(SCRN, 0x03);

    render_one_frame(&mut machine);
    assert_eq!(machine.bus.display_dimensions(), (640, 400));
}

#[test]
fn scrn_v400_with_200_line_crtc_stays_200_lines() {
    // The surface height follows the CRTC scan, not the SCRN bits: a game that
    // sets SCRN while the CRTC still holds 200-line values keeps a 200-line
    // display.
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |_| {});
    program_standard_crtc(&mut machine.bus);
    machine.bus.io_write(SCRN, 0x03);

    render_one_frame(&mut machine);
    assert_eq!(machine.bus.display_dimensions(), (640, 200));
}

#[test]
fn true_400_mode_interleaves_bitmap_pages_per_raster() {
    // SCRN bit1 clear on a hi-res scan: even rasters read bitmap page 0, odd
    // rasters read page 1.
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |_| {});
    program_hires_crtc(&mut machine.bus);

    machine.bus.io_write(SCRN, 0x01); // 24 kHz, one raster per pixel, page 0
    machine.bus.io_write(0x4000, 0xFF); // page 0: blue plane, cell 0 bank 0 -> pen 1
    machine.bus.io_write(SCRN, 0x11); // switch the CPU write page to 1
    machine.bus.io_write(0x8000, 0xFF); // page 1: red plane, cell 0 bank 0 -> pen 2
    machine.bus.io_write(SCRN, 0x01);

    machine.bus.io_write(0x1000, 0x02); // blue gun for pen 1
    machine.bus.io_write(0x1100, 0x04); // red gun for pen 2
    machine.bus.io_write(0x1300, 0xFF); // bitmap over text

    render_one_frame(&mut machine);
    let framebuffer = machine.bus.display_framebuffer();
    assert_eq!(pixel(framebuffer, 0, 0), BLUE);
    assert_eq!(pixel(framebuffer, 0, 1), RED);
    assert_eq!(pixel(framebuffer, 0, 2), BLACK);
}

#[test]
fn true_400_mode_fills_the_lower_half() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |_| {});
    program_hires_crtc(&mut machine.bus);

    machine.bus.io_write(SCRN, 0x01);
    // Blue plane, page 0, character row 13 (cell 13 * 80 = 0x410), bank 3.
    machine.bus.io_write(0x4000 | (3 * 0x800) | 0x410, 0xFF);
    machine.bus.io_write(0x1000, 0x02); // blue gun for pen 1
    machine.bus.io_write(0x1300, 0xFF); // bitmap over text

    render_one_frame(&mut machine);
    let framebuffer = machine.bus.display_framebuffer();
    // Bank 3 of row 13 lands on even raster 13 * 16 + 3 * 2 = 214.
    assert_eq!(pixel(framebuffer, 0, 214), BLUE);
    assert_eq!(pixel(framebuffer, 0, 215), BLACK);
}

#[test]
fn raster_double_mode_line_doubles_the_displayed_page() {
    // SCRN bit1 set on a hi-res scan: the displayed page is line-doubled and
    // the second page stays hidden.
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |_| {});
    program_hires_crtc(&mut machine.bus);

    machine.bus.io_write(SCRN, 0x03); // 24 kHz, two rasters per pixel, page 0
    machine.bus.io_write(0x4000, 0xFF); // page 0: blue plane, cell 0 bank 0 -> pen 1
    machine.bus.io_write(SCRN, 0x13); // switch the CPU write page to 1
    machine.bus.io_write(0x8000, 0xFF); // page 1: red plane, cell 0 bank 0 -> pen 2
    machine.bus.io_write(SCRN, 0x03);

    machine.bus.io_write(0x1000, 0x02); // blue gun for pen 1
    machine.bus.io_write(0x1100, 0x04); // red gun for pen 2
    machine.bus.io_write(0x1300, 0xFF); // bitmap over text

    render_one_frame(&mut machine);
    let framebuffer = machine.bus.display_framebuffer();
    assert_eq!(pixel(framebuffer, 0, 0), BLUE);
    assert_eq!(pixel(framebuffer, 0, 1), BLUE);
}

#[test]
fn blackclip_register_clips_bitmap_pens_zero_and_one() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |_| {});
    program_standard_crtc(&mut machine.bus);

    machine.bus.io_write(0x4000, 0x80); // pen 1 at x=0, pen 0 at x=1
    machine.bus.io_write(0x1000, 0x03); // bitmap pens 0 and 1 are blue
    machine.bus.io_write(0x1300, 0xFF); // bitmap over text

    render_one_frame(&mut machine);
    assert_eq!(pixel(machine.bus.display_framebuffer(), 0, 0), BLUE);
    assert_eq!(pixel(machine.bus.display_framebuffer(), 1, 0), BLUE);

    machine.bus.io_write(0x1FE0, 0x30);
    // The register is write-only; reads return the open bus value.
    assert_eq!(machine.bus.io_read(0x1FE0).0, 0xFF);

    render_one_frame(&mut machine);
    assert_eq!(pixel(machine.bus.display_framebuffer(), 0, 0), BLACK);
    assert_eq!(pixel(machine.bus.display_framebuffer(), 1, 0), BLACK);
}

#[test]
fn hires_clock_selects_the_16_row_ank_font() {
    // SCRN bit0 alone (without ank_sel) already switches text to the 8x16 ANK
    // font on the 24 kHz screen.
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |roms| {
        roms.ank[0x1000 + 16 + 8] = 0xFF;
    });
    program_hires_crtc(&mut machine.bus);

    machine.bus.io_write(SCRN, 0x01);
    machine.bus.io_write(0x3000, 1); // character 1
    machine.bus.io_write(0x2000, 0x07); // white pen

    render_one_frame(&mut machine);
    let framebuffer = machine.bus.display_framebuffer();
    assert_eq!(pixel(framebuffer, 0, 8), WHITE);
    assert_eq!(pixel(framebuffer, 0, 0), BLACK);
}

#[test]
fn kanji_cell_samples_the_kanji_rom() {
    // Character code 5 with kanji bank 0, left side: knj_tile = (5 << 1) = 10,
    // so row 0 of the glyph lives at kanji_rom[10 * 16]. On the 15 kHz screen
    // the 16-row glyph is compressed into 8 scanlines: scanline N shows ROM
    // row 2 * N.
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |roms| {
        let kanji = roms.kanji.as_mut().unwrap();
        kanji[10 * 16] = 0xFF; // glyph row 0 fully lit
        kanji[10 * 16 + 2] = 0xFF; // glyph row 2 fully lit
        kanji[10 * 16 + 3] = 0xFF; // glyph row 3, skipped by the compression
    });
    program_standard_crtc(&mut machine.bus);

    machine.bus.io_write(0x3000, 5); // text VRAM cell 0: character 5
    machine.bus.io_write(0x2000, 0x07); // attribute: white pen
    machine.bus.io_write(0x3800, 0x80); // kvram cell 0: kanji enable, side 0, bank 0

    render_one_frame(&mut machine);
    let framebuffer = machine.bus.display_framebuffer();
    assert_eq!(pixel(framebuffer, 0, 0), WHITE);
    assert_eq!(pixel(framebuffer, 7, 0), WHITE);
    // Scanline 1 shows ROM row 2.
    assert_eq!(pixel(framebuffer, 0, 1), WHITE);
    // Scanline 2 shows ROM row 4, which is dark.
    assert_eq!(pixel(framebuffer, 0, 2), BLACK);
}

#[test]
fn ank_sel_renders_the_8x16_font() {
    // The 8x16 ANK glyph for character 1 lives at ank_rom[0x1000 + 1 * 16 + row].
    // Row 8 only exists in the 8x16 font, so lighting it proves the tall glyph is
    // used rather than the 8x8 CG-ROM.
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |roms| {
        roms.ank[0x1000 + 16 + 8] = 0xFF;
    });
    program_standard_crtc(&mut machine.bus);
    crtc(&mut machine.bus, 9, 15); // 16 scanlines per character row

    machine.bus.io_write(SCRN, 0x04); // ank_sel: 8x16 ANK font
    machine.bus.io_write(0x3000, 1); // character 1
    machine.bus.io_write(0x2000, 0x07); // white pen

    render_one_frame(&mut machine);
    let framebuffer = machine.bus.display_framebuffer();
    assert_eq!(pixel(framebuffer, 0, 8), WHITE);
    assert_eq!(pixel(framebuffer, 0, 0), BLACK);
}

#[test]
fn disp_bank_flip_selects_the_displayed_bitmap_page() {
    let mut machine = build_machine_with_synthetic_roms(X1Model::X1Turbo, |_| {});
    program_standard_crtc(&mut machine.bus);

    // Draw a red bitmap pixel into the hidden page (write page 1, display page 0).
    machine.bus.io_write(SCRN, 0x10); // bitmap_page = 1, disp_bank = 0
    machine.bus.io_write(0x8000, 0xFF); // red plane, cell 0 row 0 -> pen 2
    machine.bus.io_write(0x1100, 0x04); // latch the red gun for pen 2
    machine.bus.io_write(0x1300, 0xFF); // bitmap over text

    render_one_frame(&mut machine);
    // Page 0 is displayed and still empty.
    assert_eq!(pixel(machine.bus.display_framebuffer(), 0, 0), BLACK);

    // Flip the displayed page to 1; the hidden drawing becomes visible.
    machine.bus.io_write(SCRN, 0x08); // disp_bank = 1
    render_one_frame(&mut machine);
    assert_eq!(pixel(machine.bus.display_framebuffer(), 0, 0), RED);
}
