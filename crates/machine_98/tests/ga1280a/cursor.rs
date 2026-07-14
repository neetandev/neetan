use common::{Bus, CpuMode, MachineModel, NoTrace};
use machine_98::Pc9801Bus;

const GA_GAPORT: u16 = 0x00D8;
const WINDOW_BASE: u32 = 0xC0000;
const CURSOR_MASK_BYTES: usize = 128;
const CURSOR_POSITION_BIAS: i32 = 0x20;

fn ga_port(selector: u8, offset: u8) -> u16 {
    (u16::from(selector) << 8) | (GA_GAPORT + u16::from(offset))
}

fn setup_bus() -> Pc9801Bus<NoTrace> {
    let mut bus = Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48000);
    bus.install_ga1280a();
    bus.io_write_word(ga_port(0x16, 0), 0x20C1);
    bus.io_write_word(ga_port(0x03, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_byte(ga_port(0x07, 0), 0xFF);
    bus.io_write_byte(ga_port(0x1B, 0), 0xFF);
    bus
}

fn write_palette(bus: &mut Pc9801Bus<NoTrace>, index: u8, red: u8, green: u8, blue: u8) {
    bus.io_write_byte(ga_port(0x18, 0), index);
    bus.io_write_byte(ga_port(0x1A, 0), red);
    bus.io_write_byte(ga_port(0x1A, 0), green);
    bus.io_write_byte(ga_port(0x1A, 0), blue);
}

fn write_indexed_pixel(bus: &mut Pc9801Bus<NoTrace>, x: u32, y: u32, palette_index: u8) {
    bus.io_write_word(ga_port(0x01, 0), y as u16);
    bus.io_write_word(ga_port(0x03, 0), 0x00FF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x09, 0), u16::from(palette_index));
    bus.io_write_byte(ga_port(0x0E, 0), 1);
    bus.write_byte(WINDOW_BASE + x / 8, 0x80 >> (x & 7));
    bus.io_write_byte(ga_port(0x0E, 0), 0);
}

fn read_indexed_pixel(bus: &mut Pc9801Bus<NoTrace>, x: u32, y: u32) -> u8 {
    bus.io_write_word(ga_port(0x02, 0), y as u16);
    let address = WINDOW_BASE + x / 8;
    let bit = 0x80 >> (x & 7);
    let mut palette_index = 0;

    for plane in 0..8 {
        bus.io_write_byte(ga_port(0x06, 0), plane);
        if bus.read_byte(address) & bit != 0 {
            palette_index |= 1 << plane;
        }
    }

    palette_index
}

fn write_cursor_colors(bus: &mut Pc9801Bus<NoTrace>, background: [u8; 3], foreground: [u8; 3]) {
    bus.io_write_byte(ga_port(0x18, 1), 1);
    bus.io_write_byte(ga_port(0x18, 0), 0);
    for value in background.into_iter().chain(foreground) {
        bus.io_write_byte(ga_port(0x1A, 0), value);
    }
    bus.io_write_byte(ga_port(0x18, 1), 0);
}

fn write_cursor_pattern(
    bus: &mut Pc9801Bus<NoTrace>,
    and_pattern: &[u8; CURSOR_MASK_BYTES],
    xor_pattern: &[u8; CURSOR_MASK_BYTES],
) {
    bus.io_write_byte(ga_port(0x18, 1), 2);
    for value in xor_pattern.iter().chain(and_pattern) {
        bus.io_write_byte(ga_port(0x19, 0), *value);
    }
    bus.io_write_byte(ga_port(0x18, 1), 0);
}

fn write_cursor_position(bus: &mut Pc9801Bus<NoTrace>, x: i32, y: i32) {
    write_cursor_position_raw(
        bus,
        (x + CURSOR_POSITION_BIAS) as u16,
        (y + CURSOR_POSITION_BIAS) as u16,
    );
}

fn write_cursor_position_raw(bus: &mut Pc9801Bus<NoTrace>, raw_x: u16, raw_y: u16) {
    bus.io_write_byte(ga_port(0x18, 1), 3);
    bus.io_write_byte(ga_port(0x18, 0), raw_x as u8);
    bus.io_write_byte(ga_port(0x1A, 0), (raw_x >> 8) as u8);
    bus.io_write_byte(ga_port(0x1B, 0), raw_y as u8);
    bus.io_write_byte(ga_port(0x19, 0), (raw_y >> 8) as u8);
    bus.io_write_byte(ga_port(0x18, 1), 0);
}

fn assert_pixel(bus: &Pc9801Bus<NoTrace>, x: u32, y: u32, expected: [u8; 4]) {
    let (width, _) = bus.display_dimensions();
    let offset = ((y * width + x) * 4) as usize;
    assert_eq!(&bus.display_framebuffer()[offset..offset + 4], &expected);
}

fn transparent_cursor_masks() -> ([u8; CURSOR_MASK_BYTES], [u8; CURSOR_MASK_BYTES]) {
    ([0xFF; CURSOR_MASK_BYTES], [0; CURSOR_MASK_BYTES])
}

#[test]
fn ga1280_hardware_cursor_composes_masks_without_touching_vram() {
    let mut bus = setup_bus();
    let background = [0x80, 0x10, 0x20];
    let foreground = [0xF0, 0xE0, 0x10];

    write_palette(&mut bus, 4, 0x10, 0x20, 0x30);
    for x in 10..14 {
        write_indexed_pixel(&mut bus, x, 20, 4);
    }

    write_cursor_colors(&mut bus, background, foreground);
    let (mut and_pattern, mut xor_pattern) = transparent_cursor_masks();
    and_pattern[0] = 0x9F;
    xor_pattern[0] = 0x30;
    write_cursor_pattern(&mut bus, &and_pattern, &xor_pattern);
    write_cursor_position(&mut bus, 10, 20);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.cursor_colors, [background, foreground]);
    assert_eq!(state.cursor_and_pattern[0], 0x9F);
    assert_eq!(state.cursor_xor_pattern[0], 0x30);

    bus.ga1280a_present_now();
    assert_pixel(&bus, 10, 20, [0x10, 0x20, 0x30, 0xFF]);
    assert_pixel(&bus, 11, 20, [0x80, 0x10, 0x20, 0xFF]);
    assert_pixel(&bus, 12, 20, [0xF0, 0xE0, 0x10, 0xFF]);
    assert_pixel(&bus, 13, 20, [0xEF, 0xDF, 0xCF, 0xFF]);

    for x in 10..14 {
        assert_eq!(read_indexed_pixel(&mut bus, x, 20), 4);
    }
}

#[test]
fn ga1280_hardware_cursor_erase_position_hides_overlay() {
    let mut bus = setup_bus();

    write_palette(&mut bus, 3, 0x11, 0x22, 0x33);
    write_indexed_pixel(&mut bus, 5, 6, 3);
    write_cursor_colors(&mut bus, [0x00, 0x00, 0x00], [0xAA, 0xBB, 0xCC]);
    let (mut and_pattern, mut xor_pattern) = transparent_cursor_masks();
    and_pattern[0] &= !0x80;
    xor_pattern[0] |= 0x80;
    write_cursor_pattern(&mut bus, &and_pattern, &xor_pattern);

    write_cursor_position(&mut bus, 5, 6);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 5, 6, [0xAA, 0xBB, 0xCC, 0xFF]);

    write_cursor_position_raw(&mut bus, 0x0520, 0x0420);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 5, 6, [0x11, 0x22, 0x33, 0xFF]);
}

#[test]
fn ga1280_hardware_cursor_clips_negative_position() {
    let mut bus = setup_bus();

    write_palette(&mut bus, 2, 0x22, 0x33, 0x44);
    write_indexed_pixel(&mut bus, 0, 0, 2);
    write_cursor_colors(&mut bus, [0x00, 0x00, 0x00], [0x66, 0x77, 0x88]);
    let (mut and_pattern, mut xor_pattern) = transparent_cursor_masks();
    and_pattern[0] &= !0x40;
    xor_pattern[0] |= 0x40;
    write_cursor_pattern(&mut bus, &and_pattern, &xor_pattern);

    write_cursor_position(&mut bus, -1, 0);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 0, [0x66, 0x77, 0x88, 0xFF]);
}
