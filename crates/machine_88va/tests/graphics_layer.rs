//! Graphic bitmap layer integration tests: the graphics access controller
//! (GACTRLVA) CPU path, the framebuffer descriptor registers, the two graphic
//! bitmap screens in their color modes, single- versus multi-plane rendering,
//! and the GMSP reset behavior, all driven through the public bus surface.

use common::Bus;
use machine_88va::Pc88VaMachine;

#[path = "common/harness.rs"]
mod harness;
use harness::*;

const GRAPHICS_WINDOW: u32 = 0xA_0000;

/// Programs the TSP timing so a frame renders (400-line, 15 kHz geometry).
fn program_sync(machine: &mut Pc88VaMachine) {
    let mut sync = [0u8; 14];
    sync[0x0A] = 0x90;
    sync[0x0B] = 0x40;
    tsp_command(machine, 0x10, &sync);
}

/// Writes a full framebuffer-0 descriptor covering the screen from VRAM 0.
fn program_framebuffer0(machine: &mut Pc88VaMachine, frame_width: u16) {
    // fsa = 0, fbw, fbl, dot = 0, ofx = 0, ofy = 0, dsa = 0, dsh = 400, dsp = 0.
    put_word(machine, 0x204, frame_width); // fbw
    put_word(machine, 0x206, 400); // fbl
    machine.bus.io_write_byte(0x208, 0); // dot
    put_word(machine, 0x20A, 0); // ofx
    put_word(machine, 0x20C, 0); // ofy
    machine.bus.io_write_byte(0x20E, 0); // dsa low
    machine.bus.io_write_byte(0x20F, 0);
    machine.bus.io_write_byte(0x210, 0); // dsa high
    put_word(machine, 0x212, 400); // dsh
    put_word(machine, 0x216, 0); // dsp
}

/// Selects the GVRAM window in single-plane CPU-data write mode.
fn open_single_plane_write(machine: &mut Pc88VaMachine) {
    machine.bus.io_write_byte(0x153, 0x14); // sysm bank 4 + GMSP (single plane)
    machine.bus.io_write_byte(0x580, 0x10); // single-plane write mode = CPU data
}

/// Selects the GVRAM window in multi-plane independent CPU access.
fn open_multi_plane_write(machine: &mut Pc88VaMachine) {
    machine.bus.io_write_byte(0x153, 0x04); // sysm bank 4, GMSP clear (multi plane)
}

fn write_gvram(machine: &mut Pc88VaMachine, offset: u32, value: u8) {
    machine.bus.write_byte(GRAPHICS_WINDOW + offset, value);
}

fn set_palette(machine: &mut Pc88VaMachine, entry: u16, color: u16) {
    let port = 0x300 + entry * 2;
    machine.bus.io_write_byte(port, color as u8);
    machine.bus.io_write_byte(port + 1, (color >> 8) as u8);
}

#[test]
fn gactrlva_single_plane_cpu_write_round_trips() {
    let mut machine = machine();
    open_single_plane_write(&mut machine);
    write_gvram(&mut machine, 0x0000, 0xAB);
    write_gvram(&mut machine, 0x1234, 0xCD);
    assert_eq!(machine.bus.read_byte(GRAPHICS_WINDOW), 0xAB);
    assert_eq!(machine.bus.read_byte(GRAPHICS_WINDOW + 0x1234), 0xCD);
}

#[test]
fn gactrlva_single_plane_rop_pattern_modes() {
    let mut machine = machine();
    machine.bus.io_write_byte(0x153, 0x14); // single-plane GVRAM window

    // Pattern register mode (write mode 1): the pattern is written regardless of
    // the CPU value. Pattern[0] low byte at port 0x590.
    machine.bus.io_write_byte(0x580, 0x08); // (write_mode >> 3) & 3 == 1
    machine.bus.io_write_byte(0x590, 0x5A); // pattern[0] low
    write_gvram(&mut machine, 0x0000, 0x00);
    assert_eq!(machine.bus.read_byte(GRAPHICS_WINDOW), 0x5A);

    // No-operation mode (write mode 3): memory is preserved.
    machine.bus.io_write_byte(0x580, 0x18); // (write_mode >> 3) & 3 == 3
    write_gvram(&mut machine, 0x0000, 0xFF);
    assert_eq!(machine.bus.read_byte(GRAPHICS_WINDOW), 0x5A);
}

#[test]
fn gactrlva_multi_plane_independent_access_round_trips() {
    let mut machine = machine();
    open_multi_plane_write(&mut machine);
    // Independent access (access mode 0) writes the addressed plane byte directly.
    write_gvram(&mut machine, 0x0000, 0x12);
    write_gvram(&mut machine, 0x1_0000, 0x34); // plane 1
    write_gvram(&mut machine, 0x2_0000, 0x56); // plane 2
    assert_eq!(machine.bus.read_byte(GRAPHICS_WINDOW), 0x12);
    assert_eq!(machine.bus.read_byte(GRAPHICS_WINDOW + 0x1_0000), 0x34);
    assert_eq!(machine.bus.read_byte(GRAPHICS_WINDOW + 0x2_0000), 0x56);
}

#[test]
fn framebuffer_descriptor_registers_read_back_masked() {
    let mut machine = machine();
    // fsa: low byte masked to 0xFC, high two bits to 0x03.
    machine.bus.io_write_byte(0x200, 0xFF);
    machine.bus.io_write_byte(0x201, 0x12);
    machine.bus.io_write_byte(0x202, 0xFF);
    assert_eq!(machine.bus.io_read_byte(0x200), 0xFC);
    assert_eq!(machine.bus.io_read_byte(0x201), 0x12);
    assert_eq!(machine.bus.io_read_byte(0x202), 0x03);

    // dsh high bit masked to 0x01.
    machine.bus.io_write_byte(0x213, 0xFF);
    assert_eq!(machine.bus.io_read_byte(0x213), 0x01);

    // Descriptor 1's fsa is read-only (the no-wrap sentinel stays 0xFFFFFFFF).
    machine.bus.io_write_byte(0x220, 0x00);
    assert_eq!(machine.bus.io_read_byte(0x220), 0xFF);
    // Descriptor 1's dsa is writable.
    machine.bus.io_write_byte(0x22E, 0x40);
    assert_eq!(machine.bus.io_read_byte(0x22E), 0x40);
}

#[test]
fn gmsp_change_resets_the_graphics_controller() {
    let mut machine = machine();
    open_multi_plane_write(&mut machine);
    // Program a multi-plane register away from its reset value.
    machine.bus.io_write_byte(0x514, 0x05); // read plane = 0x05 | 0xF0
    assert_eq!(machine.bus.io_read_byte(0x514), 0xF5);

    // Re-selecting the same GMSP value does not reset the controller.
    machine.bus.io_write_byte(0x153, 0x04);
    assert_eq!(machine.bus.io_read_byte(0x514), 0xF5);

    // Toggling GMSP and back resets the controller to its defaults.
    machine.bus.io_write_byte(0x153, 0x14); // GMSP set (also resets the SGP)
    machine.bus.io_write_byte(0x153, 0x04); // GMSP clear
    assert_eq!(machine.bus.io_read_byte(0x514), 0xFF);
}

/// Programs a single-plane graphic-0 screen and fills the start of GVRAM with a
/// constant byte. Does not render: the caller sets the palette/backdrop first.
fn setup_single_plane_graphic0(
    pixelmode: u16,
    frame_width: u16,
    fill_byte: u8,
    byte_count: u32,
) -> Pc88VaMachine {
    let mut machine = machine();
    open_single_plane_write(&mut machine);
    for offset in 0..byte_count {
        write_gvram(&mut machine, offset, fill_byte);
    }
    program_framebuffer0(&mut machine, frame_width);

    // grres: screen 0 pixel mode, 640 dots.
    put_word(&mut machine, 0x102, pixelmode);
    // colcomp: screen 0 = graphic 0 (kind 8 | layer 2).
    put_word(&mut machine, 0x106, 0x000A);
    // grmode: GDEN | XVSP | SYNCEN | single-plane.
    put_word(&mut machine, 0x100, 0xB400);

    program_sync(&mut machine);
    machine
}

#[test]
fn single_plane_4bpp_palette_renders() {
    // 4 bpp, 640 dots: each byte holds two index-2 pixels (0x22).
    let mut machine = setup_single_plane_graphic0(0x0001, 640, 0x22, 320);
    set_palette(&mut machine, 2, 0x03E0); // pure red
    render_one_frame(&mut machine);

    let framebuffer = machine.display_framebuffer();
    assert_eq!(pixel(framebuffer, 0, 0), va_rgba(0x03E0));
    assert_eq!(pixel(framebuffer, 320, 0), va_rgba(0x03E0));
    assert_eq!(pixel(framebuffer, 639, 0), va_rgba(0x03E0));
}

#[test]
fn single_plane_8bpp_palette_renders() {
    // 8 bpp, 640 dots: one byte per pixel, index 5.
    let mut machine = setup_single_plane_graphic0(0x0002, 640, 0x05, 640);
    set_palette(&mut machine, 5, 0xFC00); // pure green
    render_one_frame(&mut machine);

    let framebuffer = machine.display_framebuffer();
    assert_eq!(pixel(framebuffer, 0, 0), va_rgba(0xFC00));
    assert_eq!(pixel(framebuffer, 400, 0), va_rgba(0xFC00));
}

#[test]
fn graphic_transparent_color_shows_backdrop() {
    // 8 bpp screen filled with index 3; mark index 3 transparent for graphic 0.
    let mut machine = setup_single_plane_graphic0(0x0002, 640, 0x03, 640);
    // Backdrop pure blue.
    machine.bus.io_write_byte(0x10E, 0x1F);
    machine.bus.io_write_byte(0x10F, 0x00);
    // xpar_g0: index 3 transparent.
    put_word(&mut machine, 0x124, 1 << 3);
    render_one_frame(&mut machine);

    assert_eq!(
        pixel(machine.display_framebuffer(), 100, 0),
        va_rgba(0x001F)
    );
}

#[test]
fn framebuffer_dot_offset_shifts_start_pixel() {
    let mut machine = machine();
    open_single_plane_write(&mut machine);
    // 8 bpp: source byte 0 = index 1, the rest = index 2.
    write_gvram(&mut machine, 0, 0x01);
    for offset in 1..640u32 {
        write_gvram(&mut machine, offset, 0x02);
    }
    program_framebuffer0(&mut machine, 640);
    machine.bus.io_write_byte(0x208, 1); // dot = 1: skip the first source byte

    put_word(&mut machine, 0x102, 0x0002); // grres: screen 0 = 8 bpp
    put_word(&mut machine, 0x106, 0x000A); // colcomp: screen 0 = graphic 0
    put_word(&mut machine, 0x100, 0xB400); // GDEN | XVSP | SYNCEN | single plane
    set_palette(&mut machine, 1, 0xFC00); // green
    set_palette(&mut machine, 2, 0x03E0); // red
    program_sync(&mut machine);
    render_one_frame(&mut machine);

    // The dot offset drops index-1 byte 0, so pixel 0 shows index 2 (red).
    assert_eq!(pixel(machine.display_framebuffer(), 0, 0), va_rgba(0x03E0));
}

#[test]
fn single_plane_16bpp_direct_color_renders() {
    let mut machine = machine();
    open_single_plane_write(&mut machine);
    // 16 bpp direct color: two bytes per pixel, color 0x03E0 (red), 640 pixels.
    for offset in 0..640u32 {
        write_gvram(&mut machine, offset * 2, 0xE0);
        write_gvram(&mut machine, offset * 2 + 1, 0x03);
    }
    program_framebuffer0(&mut machine, 0x500); // 1280 bytes per line

    put_word(&mut machine, 0x102, 0x0003); // grres: screen 0 = 16 bpp
    put_word(&mut machine, 0x108, 0x0008); // rgbcomp: rgb screen 0 = graphic 0
    put_word(&mut machine, 0x106, 0x0000); // colcomp: no palette screens
    put_word(&mut machine, 0x100, 0xB400); // GDEN | XVSP | SYNCEN | single plane

    program_sync(&mut machine);
    render_one_frame(&mut machine);

    let framebuffer = machine.display_framebuffer();
    assert_eq!(pixel(framebuffer, 0, 0), va_rgba(0x03E0));
    assert_eq!(pixel(framebuffer, 320, 0), va_rgba(0x03E0));
}

#[test]
fn multi_plane_4bpp_combines_planes() {
    let mut machine = machine();
    open_multi_plane_write(&mut machine);
    // Index 5 = 0b0101: plane 0 and plane 2 set, planes 1 and 3 clear.
    for offset in 0..80u32 {
        write_gvram(&mut machine, offset, 0xFF); // plane 0
        write_gvram(&mut machine, 0x2_0000 + offset, 0xFF); // plane 2
    }
    program_framebuffer0(&mut machine, 0x400);

    put_word(&mut machine, 0x102, 0x0001); // grres: screen 0 = 4 bpp
    put_word(&mut machine, 0x106, 0x000A); // colcomp: screen 0 = graphic 0
    put_word(&mut machine, 0x100, 0xB000); // GDEN | XVSP | SYNCEN, multi plane
    set_palette(&mut machine, 5, 0x7FFF); // a distinct color

    program_sync(&mut machine);
    render_one_frame(&mut machine);

    let framebuffer = machine.display_framebuffer();
    assert_eq!(pixel(framebuffer, 0, 0), va_rgba(0x7FFF));
    assert_eq!(pixel(framebuffer, 320, 0), va_rgba(0x7FFF));
}

#[test]
fn graphics_disabled_when_gden_clear() {
    // Same as the 8 bpp screen but with GDEN clear: graphics must not appear.
    let mut machine = setup_single_plane_graphic0(0x0002, 640, 0x05, 640);
    set_palette(&mut machine, 5, 0xFC00);
    // Backdrop pure blue; clear GDEN (bit 15) but keep video output enabled.
    machine.bus.io_write_byte(0x10E, 0x1F);
    machine.bus.io_write_byte(0x10F, 0x00);
    put_word(&mut machine, 0x100, 0x3400); // XVSP | SYNCEN | single plane, no GDEN
    render_one_frame(&mut machine);

    assert_eq!(
        pixel(machine.display_framebuffer(), 100, 0),
        va_rgba(0x001F)
    );
}
