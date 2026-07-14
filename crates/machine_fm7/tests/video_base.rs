//! Base FM-7 video: plane composition, digital palette, multipage access and
//! display masks, CRT enable and coarse display-offset scrolling.

mod harness;

use common::Bus;
use harness::{build_bus_with_synthetic_roms, run_bus_cycles};
use machine_fm7::{BootMode, Fm7Bus, SubBusView};

/// Byte offset of the blue plane in VRAM.
const PLANE_BLUE: u16 = 0x0000;
/// Byte offset of the red plane in VRAM.
const PLANE_RED: u16 = 0x4000;
/// Byte offset of the green plane in VRAM.
const PLANE_GREEN: u16 = 0x8000;
/// Plane byte carrying a set leftmost (most significant) pixel.
const LEFT_PIXEL: u8 = 0x80;
/// Plane bytes per scanline.
const BYTES_PER_LINE: u16 = 80;
/// Framebuffer width in pixels.
const WIDTH: usize = 640;

/// Builds a base FM-7 bus with synthetic ROMs for the video tests.
fn build_bus() -> Fm7Bus {
    build_bus_with_synthetic_roms(BootMode::Basic, |_| {})
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

/// Commits a coarse display offset through the two-byte `0xD40E`/`0xD40F` latch.
fn set_display_offset(bus: &mut Fm7Bus, offset: u16) {
    sub_write(bus, 0xD40E, (offset >> 8) as u8);
    sub_write(bus, 0xD40F, offset as u8);
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

/// The fixed RGBA colour for a three-bit code (bit 0 blue, bit 1 red, bit 2 green).
fn expected_rgba(code: u8) -> [u8; 4] {
    let blue = if code & 1 != 0 { 0xFF } else { 0x00 };
    let red = if code & 2 != 0 { 0xFF } else { 0x00 };
    let green = if code & 4 != 0 { 0xFF } else { 0x00 };
    [red, green, blue, 0xFF]
}

/// Sets the leftmost pixel of line `line` to colour `code` across the planes.
fn set_left_pixel(bus: &mut Fm7Bus, line: u16, code: u8) {
    let byte_index = line * BYTES_PER_LINE;
    bus.sub_poke_byte(
        PLANE_BLUE + byte_index,
        if code & 1 != 0 { LEFT_PIXEL } else { 0 },
    );
    bus.sub_poke_byte(
        PLANE_RED + byte_index,
        if code & 2 != 0 { LEFT_PIXEL } else { 0 },
    );
    bus.sub_poke_byte(
        PLANE_GREEN + byte_index,
        if code & 4 != 0 { LEFT_PIXEL } else { 0 },
    );
}

#[test]
fn all_eight_colors_compose_through_the_identity_palette() {
    let mut bus = build_bus();
    for code in 0u8..8 {
        set_left_pixel(&mut bus, u16::from(code), code);
    }
    enable_crt(&mut bus);
    render_frames(&mut bus, 2);

    for code in 0u8..8 {
        assert_eq!(
            pixel(&bus, 0, usize::from(code)),
            expected_rgba(code),
            "colour {code}"
        );
    }
}

#[test]
fn digital_palette_remaps_colors_and_reads_back_with_high_bits_set() {
    let mut bus = build_bus();
    // Leftmost pixel of line 0 selects palette entry 1 (blue plane set).
    set_left_pixel(&mut bus, 0, 1);
    // Remap palette entry 1 to colour 2 (red).
    bus.write_byte(0xFD39, 0x02);
    enable_crt(&mut bus);
    render_frames(&mut bus, 2);
    assert_eq!(pixel(&bus, 0, 0), expected_rgba(2));

    // Palette registers read back the stored colour with the upper bits forced high.
    bus.write_byte(0xFD3A, 0x05);
    assert_eq!(bus.read_byte(0xFD3A).0, 0x05 | 0xF8);
}

#[test]
fn display_mask_excludes_a_plane_from_the_output() {
    let mut bus = build_bus();
    // White pixel (all three planes) at the top-left.
    set_left_pixel(&mut bus, 0, 7);
    // Hide the green plane: multipage high nibble bit 2.
    bus.write_byte(0xFD37, 0x04 << 4);
    enable_crt(&mut bus);
    render_frames(&mut bus, 2);
    // White minus green is magenta (red + blue).
    assert_eq!(pixel(&bus, 0, 0), expected_rgba(0b011));
}

#[test]
fn access_mask_blocks_sub_cpu_plane_reads_and_writes() {
    let mut bus = build_bus();
    bus.sub_poke_byte(PLANE_BLUE, 0x11);
    bus.sub_poke_byte(PLANE_RED, 0x22);
    // Block the blue plane (multipage low nibble bit 0).
    bus.write_byte(0xFD37, 0x01);

    // Blocked plane reads float high; an unblocked plane reads through.
    assert_eq!(sub_read(&mut bus, PLANE_BLUE), 0xFF);
    assert_eq!(sub_read(&mut bus, PLANE_RED), 0x22);

    // A blocked-plane write is dropped; an unblocked-plane write lands.
    sub_write(&mut bus, PLANE_BLUE, 0x55);
    sub_write(&mut bus, PLANE_RED, 0x66);
    assert_eq!(bus.sub_peek_byte(PLANE_BLUE), 0x11);
    assert_eq!(bus.sub_peek_byte(PLANE_RED), 0x66);
}

#[test]
fn disabling_the_crt_blanks_the_frame() {
    let mut bus = build_bus();
    set_left_pixel(&mut bus, 0, 7);
    enable_crt(&mut bus);
    render_frames(&mut bus, 2);
    assert_eq!(pixel(&bus, 0, 0), expected_rgba(7));

    // Writing 0xD408 turns the CRT off; the next frames present black.
    sub_write(&mut bus, 0xD408, 0x00);
    render_frames(&mut bus, 2);
    assert_eq!(pixel(&bus, 0, 0), expected_rgba(0));
}

#[test]
fn coarse_display_offset_scrolls_and_wraps_within_the_plane() {
    let mut bus = build_bus();
    // A blue pixel at plane byte 32 shows at the top-left once the display
    // starts 32 bytes in.
    bus.sub_poke_byte(PLANE_BLUE + 32, LEFT_PIXEL);
    set_display_offset(&mut bus, 0x0020);
    enable_crt(&mut bus);
    render_frames(&mut bus, 2);
    assert_eq!(pixel(&bus, 0, 0), expected_rgba(1));

    // Display sampling wraps around the 16 KiB physical plane.
    let mut bus = build_bus();
    bus.sub_poke_byte(PLANE_BLUE, LEFT_PIXEL);
    set_display_offset(&mut bus, 0x3FE0);
    enable_crt(&mut bus);
    render_frames(&mut bus, 2);
    // Column 32 (byte offset 0x3FE0 + 32 = 0x4000 wraps to 0) holds the pixel.
    assert_eq!(pixel(&bus, 32 * 8, 0), expected_rgba(1));
}

#[test]
fn coarse_display_offset_translates_sub_vram_access() {
    let mut bus = build_bus();
    set_display_offset(&mut bus, 0x0020);

    sub_write(&mut bus, PLANE_BLUE, LEFT_PIXEL);

    assert_eq!(bus.sub_peek_byte(PLANE_BLUE), 0x00);
    assert_eq!(bus.sub_peek_byte(PLANE_BLUE + 32), LEFT_PIXEL);
    assert_eq!(sub_read(&mut bus, PLANE_BLUE), LEFT_PIXEL);

    enable_crt(&mut bus);
    render_frames(&mut bus, 2);
    assert_eq!(pixel(&bus, 0, 0), expected_rgba(1));
}
