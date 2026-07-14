//! FM-77AV video: analog palette and 4096-color mode, draw/display page flip,
//! fine display scroll, and the main-CPU direct-VRAM window.

mod harness;

use common::Bus;
use harness::{build_av_bus_with_synthetic_roms, run_bus_cycles};
use machine_fm7::{BootMode, Fm7Bus, SubBusView};

/// Byte offset of the blue plane in VRAM page 0.
const PLANE_BLUE: u16 = 0x0000;
/// Byte offset of the green plane in VRAM page 0.
const PLANE_GREEN: u16 = 0x8000;
/// Plane byte carrying a set leftmost (most significant) pixel.
const LEFT_PIXEL: u8 = 0x80;
/// Framebuffer width in pixels.
const WIDTH: usize = 640;
/// `0xFD12` bit selecting 320x200 (4096-color) mode.
const MODE320: u8 = 0x40;
/// `0xD430` bit selecting the drawn VRAM page.
const DRAW_PAGE: u8 = 0x20;
/// `0xD430` bit selecting the displayed VRAM page.
const DISPLAY_PAGE: u8 = 0x40;
/// `0xD430` bit enabling fine display scroll.
const FINE_OFFSET: u8 = 0x04;

/// Builds an FM-77AV bus with synthetic ROMs for the video tests.
fn build_av_bus() -> Fm7Bus {
    build_av_bus_with_synthetic_roms(BootMode::Basic, |_| {})
}

/// Reads a byte from the sub address space through a sub bus view.
fn sub_read(bus: &mut Fm7Bus, address: u16) -> u8 {
    let mut view = SubBusView { bus };
    view.read_byte(u32::from(address))
}

/// Writes a byte to the sub address space through a sub bus view.
fn sub_write(bus: &mut Fm7Bus, address: u16, value: u8) {
    let mut view = SubBusView { bus };
    view.write_byte(u32::from(address), value);
}

/// Turns the CRT output on by reading the sub `0xD408` flag.
fn enable_crt(bus: &mut Fm7Bus) {
    sub_read(bus, 0xD408);
}

/// Commits a display offset through the two-byte `0xD40E`/`0xD40F` latch.
fn set_display_offset(bus: &mut Fm7Bus, offset: u16) {
    sub_write(bus, 0xD40E, (offset >> 8) as u8);
    sub_write(bus, 0xD40F, offset as u8);
}

/// Writes one analog palette entry through `0xFD30-0xFD34`.
fn set_analog_palette(bus: &mut Fm7Bus, index: u16, blue: u8, red: u8, green: u8) {
    bus.write_byte(0xFD30, ((index >> 8) & 0x0F) as u8);
    bus.write_byte(0xFD31, (index & 0xFF) as u8);
    bus.write_byte(0xFD32, blue);
    bus.write_byte(0xFD33, red);
    bus.write_byte(0xFD34, green);
}

/// Runs the display pipeline for the given number of whole frames.
fn render_frames(bus: &mut Fm7Bus, frames: u64) {
    let cycles = bus.frame_period_cycles() * frames;
    run_bus_cycles(bus, cycles);
}

/// The RGBA pixel at `(x, y)` in the presented framebuffer.
fn pixel(bus: &Fm7Bus, x: usize, y: usize) -> [u8; 4] {
    let framebuffer = bus.display_framebuffer();
    let start = (y * WIDTH + x) * 4;
    [
        framebuffer[start],
        framebuffer[start + 1],
        framebuffer[start + 2],
        framebuffer[start + 3],
    ]
}

#[test]
fn analog_palette_colors_a_4096_mode_pixel() {
    let mut bus = build_av_bus();
    bus.write_byte(0xFD12, MODE320);
    // Blue channel bit 3 (most significant sub-plane) set for the leftmost pixel.
    bus.sub_poke_byte(PLANE_BLUE, LEFT_PIXEL);
    // Pixel index is blue = 8 -> 0x008; map it to pure blue in the palette.
    set_analog_palette(&mut bus, 0x008, 0x0F, 0x00, 0x00);
    enable_crt(&mut bus);
    render_frames(&mut bus, 2);

    // 4096-mode pixels are doubled across the 640-wide surface.
    assert_eq!(pixel(&bus, 0, 0), [0x00, 0x00, 0xFF, 0xFF]);
    assert_eq!(pixel(&bus, 1, 0), [0x00, 0x00, 0xFF, 0xFF]);
}

#[test]
fn analog_palette_change_takes_effect_on_the_next_frame() {
    let mut bus = build_av_bus();
    bus.write_byte(0xFD12, MODE320);
    bus.sub_poke_byte(PLANE_GREEN, LEFT_PIXEL); // green = 8 -> index 0x800
    set_analog_palette(&mut bus, 0x800, 0x00, 0x00, 0x0F); // pure green
    enable_crt(&mut bus);
    render_frames(&mut bus, 2);
    assert_eq!(pixel(&bus, 0, 0), [0x00, 0xFF, 0x00, 0xFF]);

    // Recolor the same index to pure red; the next frame reflects it.
    set_analog_palette(&mut bus, 0x800, 0x00, 0x0F, 0x00);
    render_frames(&mut bus, 2);
    assert_eq!(pixel(&bus, 0, 0), [0xFF, 0x00, 0x00, 0xFF]);
}

#[test]
fn display_offset_latches_are_independent_per_draw_page() {
    let mut bus = build_av_bus();

    sub_write(&mut bus, 0xD430, FINE_OFFSET);
    sub_write(&mut bus, 0xD40E, 0x00);

    sub_write(&mut bus, 0xD430, DRAW_PAGE | FINE_OFFSET);
    sub_write(&mut bus, 0xD40E, 0x00);
    sub_write(&mut bus, 0xD40F, 0x02);

    sub_write(&mut bus, 0xD430, FINE_OFFSET);
    sub_write(&mut bus, 0xD40F, 0x01);
    assert_eq!(bus.display_offset(), 0x0001);

    sub_write(&mut bus, 0xD430, DISPLAY_PAGE | FINE_OFFSET);
    assert_eq!(bus.display_offset(), 0x0002);
}

#[test]
fn mode4096_uses_independent_page_offsets() {
    let mut bus = build_av_bus();
    bus.write_byte(0xFD12, MODE320);

    sub_write(&mut bus, 0xD430, FINE_OFFSET);
    sub_write(&mut bus, PLANE_BLUE, LEFT_PIXEL); // blue bit 3 at page 0 offset 0

    sub_write(&mut bus, 0xD430, DRAW_PAGE | FINE_OFFSET);
    sub_write(&mut bus, PLANE_BLUE + 1, LEFT_PIXEL); // blue bit 1 at page 1 offset 1
    set_display_offset(&mut bus, 0x0001);

    sub_write(&mut bus, 0xD430, FINE_OFFSET);
    set_analog_palette(&mut bus, 0x008, 0x00, 0x0F, 0x00); // old single-offset result
    set_analog_palette(&mut bus, 0x00A, 0x0F, 0x00, 0x00); // page0 + page1 result
    enable_crt(&mut bus);
    render_frames(&mut bus, 2);

    assert_eq!(pixel(&bus, 0, 0), [0x00, 0x00, 0xFF, 0xFF]);
}

#[test]
fn switching_display_mode_rerenders_the_frame() {
    let mut bus = build_av_bus();
    // Start in 640x200 8-color: a blue pixel renders through the fixed table.
    bus.sub_poke_byte(PLANE_BLUE, LEFT_PIXEL);
    enable_crt(&mut bus);
    render_frames(&mut bus, 2);
    assert_eq!(pixel(&bus, 0, 0), [0x00, 0x00, 0xFF, 0xFF]);

    // Switch to 320x200 4096-color; the same plane byte now indexes the analog
    // palette (blue = 8 -> index 0x008).
    bus.write_byte(0xFD12, MODE320);
    set_analog_palette(&mut bus, 0x008, 0x00, 0x0F, 0x00); // pure red
    render_frames(&mut bus, 2);
    assert_eq!(pixel(&bus, 0, 0), [0xFF, 0x00, 0x00, 0xFF]);
}

#[test]
fn draw_page_and_display_page_are_independent() {
    let mut bus = build_av_bus();
    // Draw into page 1, still displaying page 0.
    sub_write(&mut bus, 0xD430, DRAW_PAGE);
    sub_write(&mut bus, PLANE_BLUE, LEFT_PIXEL);
    enable_crt(&mut bus);
    render_frames(&mut bus, 2);
    // Page 0 is empty, so the shown pixel is black.
    assert_eq!(pixel(&bus, 0, 0), [0x00, 0x00, 0x00, 0xFF]);

    // Now display page 1 as well; the pixel drawn earlier appears.
    sub_write(&mut bus, 0xD430, DRAW_PAGE | DISPLAY_PAGE);
    render_frames(&mut bus, 2);
    assert_eq!(pixel(&bus, 0, 0), [0x00, 0x00, 0xFF, 0xFF]);
}

#[test]
fn fine_scroll_honors_the_low_offset_bits() {
    // With fine scroll enabled, a one-byte offset is honored.
    let mut bus = build_av_bus();
    sub_write(&mut bus, 0xD430, FINE_OFFSET);
    bus.sub_poke_byte(PLANE_BLUE + 1, LEFT_PIXEL);
    set_display_offset(&mut bus, 0x0001);
    enable_crt(&mut bus);
    render_frames(&mut bus, 2);
    assert_eq!(pixel(&bus, 0, 0), [0x00, 0x00, 0xFF, 0xFF]);

    // With fine scroll disabled, the low five bits are masked away, so a one-byte
    // offset rounds down to zero and the pixel does not appear.
    let mut bus = build_av_bus();
    sub_write(&mut bus, 0xD430, 0x00);
    bus.sub_poke_byte(PLANE_BLUE + 1, LEFT_PIXEL);
    set_display_offset(&mut bus, 0x0001);
    enable_crt(&mut bus);
    render_frames(&mut bus, 2);
    assert_eq!(pixel(&bus, 0, 0), [0x00, 0x00, 0x00, 0xFF]);
}

#[test]
fn direct_vram_window_reaches_vram_only_while_halted() {
    use harness::{build_av_machine_with_synthetic_roms, park_main_cpu_av, park_sub_cpu};

    let mut machine = build_av_machine_with_synthetic_roms(BootMode::Basic, |roms| {
        park_main_cpu_av(roms);
        park_sub_cpu(roms);
    });
    // Halt the sub CPU so the main CPU owns the VRAM bus.
    machine.bus.write_byte(0xFD05, 0x80);
    machine.run_for(400);
    assert!(machine.bus.is_sub_halted());

    // Map CPU block 2 (0x2000-0x2FFF) onto physical bank 0x10, the first direct-VRAM
    // window bank (sub address space 0x0000), and enable the MMR.
    machine.bus.write_byte(0xFD90, 0x00); // segment 0
    machine.bus.write_byte(0xFD82, 0x10); // page register for block 2
    machine.bus.write_byte(0xFD93, 0x80); // MMR enable

    // A write through the window lands in VRAM and reads back through it.
    machine.bus.write_byte(0x2000, LEFT_PIXEL);
    assert_eq!(machine.bus.read_byte(0x2000), LEFT_PIXEL);
    assert_eq!(machine.bus.sub_peek_byte(PLANE_BLUE), LEFT_PIXEL);

    // The written pixel appears in the rendered image. The machine started
    // mid-frame, so allow a few frames for a fully re-latched frame to present.
    enable_crt(&mut machine.bus);
    render_frames(&mut machine.bus, 3);
    assert_eq!(pixel(&machine.bus, 0, 0), [0x00, 0x00, 0xFF, 0xFF]);

    // While the sub CPU is not halted the window is closed.
    let mut bus = build_av_bus();
    bus.write_byte(0xFD90, 0x00);
    bus.write_byte(0xFD82, 0x10);
    bus.write_byte(0xFD93, 0x80);
    assert!(!bus.is_sub_halted());
    assert_eq!(bus.read_byte(0x2000), 0xFF);
    bus.write_byte(0x2000, LEFT_PIXEL);
    assert_eq!(bus.sub_peek_byte(PLANE_BLUE), 0x00);
}
