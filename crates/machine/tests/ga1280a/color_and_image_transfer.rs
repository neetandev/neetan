use common::{Bus, CpuMode, MachineModel};
use device::ga1280a::{Ga1280aPlaneMode, Ga1280aStreamState};
use machine::{NoTracing, Pc9801Bus};

const GA_GAPORT: u16 = 0x00D8;
const WINDOW_BASE: u32 = 0xC0000;
const FULL_COLOR_WIDTH: u32 = 512;

fn ga_port(selector: u8, offset: u8) -> u16 {
    (u16::from(selector) << 8) | (GA_GAPORT + u16::from(offset))
}

fn setup_bus() -> Pc9801Bus<NoTracing> {
    let mut bus = Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48000);
    bus.install_ga1280a();
    bus.io_write_word(ga_port(0x16, 0), 0x20C1);
    bus.io_write_word(ga_port(0x03, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_byte(ga_port(0x07, 0), 0xFF);
    bus.io_write_byte(ga_port(0x1B, 0), 0xFF);
    bus
}

fn write_palette(bus: &mut Pc9801Bus<NoTracing>, index: u8, red: u8, green: u8, blue: u8) {
    bus.io_write_byte(ga_port(0x18, 0), index);
    bus.io_write_byte(ga_port(0x1A, 0), red);
    bus.io_write_byte(ga_port(0x1A, 0), green);
    bus.io_write_byte(ga_port(0x1A, 0), blue);
}

fn write_crtc_word(bus: &mut Pc9801Bus<NoTracing>, index: u8, value: u16) {
    bus.io_write_byte(ga_port(0x1E, 0), index);
    bus.io_write_word(ga_port(0x1F, 0), value);
}

fn program_full_color_mode20(bus: &mut Pc9801Bus<NoTracing>) {
    write_crtc_word(bus, 0x00, 0x00A6);
    write_crtc_word(bus, 0x02, 0x007F);
    write_crtc_word(bus, 0x10, 0x020B);
    write_crtc_word(bus, 0x12, 0x01DF);
    write_crtc_word(bus, 0x36, 0x5084);
}

fn run_ga1280_full_color_helper(bus: &mut Pc9801Bus<NoTracing>) {
    for (selector, offset, value) in [
        (0x18, 1, 0x02),
        (0x18, 0, 0x18),
        (0x18, 1, 0x01),
        (0x1B, 0, 0x22),
        (0x18, 1, 0x00),
        (0x1C, 0, 0x03),
    ] {
        bus.io_write_byte(ga_port(selector, offset), value);
    }
}

fn set_ga1280_direct_color16(bus: &mut Pc9801Bus<NoTracing>) {
    bus.io_write_byte(ga_port(0x18, 1), 2);
    bus.io_write_byte(ga_port(0x18, 0), 0x38);
    bus.io_write_byte(ga_port(0x18, 1), 0);
}

fn set_normal_mix(bus: &mut Pc9801Bus<NoTracing>) {
    bus.io_write_byte(ga_port(0x14, 0), 0x0C);
    bus.io_write_word(ga_port(0x1E, 2), 0x1000);
}

fn set_xor_mix(bus: &mut Pc9801Bus<NoTracing>) {
    bus.io_write_byte(ga_port(0x14, 0), 0x06);
    bus.io_write_word(ga_port(0x1E, 2), 0x0000);
}

fn fill_rect(bus: &mut Pc9801Bus<NoTracing>, x: u16, y: u16, width: u16, height: u16, color: u16) {
    bus.io_write_word(ga_port(0x09, 0), color);
    bus.io_write_word(ga_port(0x0A, 2), x);
    bus.io_write_word(ga_port(0x0B, 2), y);
    bus.io_write_word(ga_port(0x04, 2), width - 1);
    bus.io_write_word(ga_port(0x05, 2), height - 1);
    bus.io_write_word(ga_port(0x1F, 2), 0x6FE8);
}

fn upload_rop_pattern(bus: &mut Pc9801Bus<NoTracing>, rows: [u8; 8]) {
    bus.io_write_byte(ga_port(0x15, 2), 0);
    for row in rows {
        bus.io_write_byte(ga_port(0x14, 2), row);
    }
}

fn read_pixel(bus: &mut Pc9801Bus<NoTracing>, x: u16, y: u16) -> u16 {
    bus.io_write_word(ga_port(0x08, 2), x);
    bus.io_write_word(ga_port(0x09, 2), y);
    bus.io_write_word(ga_port(0x1F, 2), 0x20E8);
    bus.io_read_word(ga_port(0x1C, 2))
}

fn start_image_restore(bus: &mut Pc9801Bus<NoTracing>, x: u16, y: u16, width: u16, height: u16) {
    bus.io_write_word(ga_port(0x0A, 2), x);
    bus.io_write_word(ga_port(0x0B, 2), y);
    bus.io_write_word(ga_port(0x04, 2), width - 1);
    bus.io_write_word(ga_port(0x05, 2), height - 1);
    bus.io_write_word(ga_port(0x1F, 2), 0x45E8);
}

fn start_opaque_pattern_expand(
    bus: &mut Pc9801Bus<NoTracing>,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
) {
    bus.io_write_word(ga_port(0x0A, 2), x);
    bus.io_write_word(ga_port(0x0B, 2), y);
    bus.io_write_word(ga_port(0x04, 2), width - 1);
    bus.io_write_word(ga_port(0x05, 2), height - 1);
    bus.io_write_word(ga_port(0x1F, 2), 0x4A88);
}

fn write_full_color_pixel(bus: &mut Pc9801Bus<NoTracing>, x: u32, y: u32, rgb: [u8; 3]) {
    let offset = (y * FULL_COLOR_WIDTH + x) * 3;
    bus.write_byte(WINDOW_BASE + offset, rgb[0]);
    bus.write_byte(WINDOW_BASE + offset + 1, rgb[1]);
    bus.write_byte(WINDOW_BASE + offset + 2, rgb[2]);
}

fn write_indexed_pixel(bus: &mut Pc9801Bus<NoTracing>, x: u32, y: u32, palette_index: u8) {
    bus.io_write_word(ga_port(0x01, 0), y as u16);
    bus.io_write_word(ga_port(0x03, 0), 0x00FF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x09, 0), u16::from(palette_index));
    bus.io_write_byte(ga_port(0x0E, 0), 1);
    bus.write_byte(WINDOW_BASE + x / 8, 0x80 >> (x & 7));
    bus.io_write_byte(ga_port(0x0E, 0), 0);
}

fn assert_pixel(bus: &Pc9801Bus<NoTracing>, x: u32, y: u32, expected: [u8; 4]) {
    let (width, _) = bus.display_dimensions();
    let offset = ((y * width + x) * 4) as usize;
    assert_eq!(&bus.display_framebuffer()[offset..offset + 4], &expected);
}

#[test]
fn hga_rop_rectangle_uses_uploaded_brush_pattern() {
    let mut bus = setup_bus();

    set_normal_mix(&mut bus);
    fill_rect(&mut bus, 0, 0, 8, 8, 4);
    upload_rop_pattern(
        &mut bus,
        [
            0b1010_1010,
            0b0101_0101,
            0b1010_1010,
            0b0101_0101,
            0b1010_1010,
            0b0101_0101,
            0b1010_1010,
            0b0101_0101,
        ],
    );

    bus.io_write_word(ga_port(0x10, 0), 0x00FF);
    bus.io_write_word(ga_port(0x12, 0), 0x0000);
    bus.io_write_word(ga_port(0x14, 0), 0x0606);
    bus.io_write_word(ga_port(0x08, 2), 0);
    bus.io_write_word(ga_port(0x09, 2), 0);
    bus.io_write_word(ga_port(0x0A, 2), 0);
    bus.io_write_word(ga_port(0x0B, 2), 0);
    bus.io_write_word(ga_port(0x04, 2), 7);
    bus.io_write_word(ga_port(0x05, 2), 7);
    bus.io_write_word(ga_port(0x1E, 2), 0);
    bus.io_write_word(ga_port(0x1F, 2), 0x6A28);

    for y in 0..8 {
        for x in 0..8 {
            let expected = if (x + y) % 2 == 0 { 0x00FB } else { 0x0004 };
            assert_eq!(read_pixel(&mut bus, x, y), expected, "pixel ({x},{y})");
        }
    }
}

#[test]
fn opaque_pattern_expand_consumes_rows_on_32_bit_boundaries() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x10, 0), 1);
    bus.io_write_word(ga_port(0x12, 0), 2);
    bus.io_write_word(ga_port(0x14, 0), 0x0C0C);
    start_opaque_pattern_expand(&mut bus, 0, 0, 16, 2);
    bus.io_write_word(ga_port(0x1C, 2), 0xFF00);
    bus.io_write_word(ga_port(0x1C, 2), 0x0000);
    bus.io_write_word(ga_port(0x1C, 2), 0x00FF);
    bus.io_write_word(ga_port(0x1C, 2), 0x0000);

    for x in 0..8 {
        assert_eq!(read_pixel(&mut bus, x, 0), 2, "row 0 background x={x}");
        assert_eq!(read_pixel(&mut bus, x + 8, 0), 1, "row 0 foreground x={x}");
        assert_eq!(read_pixel(&mut bus, x, 1), 1, "row 1 foreground x={x}");
        assert_eq!(read_pixel(&mut bus, x + 8, 1), 2, "row 1 background x={x}");
    }
}

#[test]
fn palette_updates_recompose_indexed_pixels() {
    let mut bus = setup_bus();

    write_palette(&mut bus, 7, 0x10, 0x20, 0x30);
    write_indexed_pixel(&mut bus, 0, 0, 7);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 0, [0x10, 0x20, 0x30, 0xFF]);

    write_palette(&mut bus, 7, 0x90, 0x40, 0xE0);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 0, [0x90, 0x40, 0xE0, 0xFF]);
}

#[test]
fn mode20_full_color_forces_512x480_and_ignores_palette_and_plane_switches() {
    let mut bus = setup_bus();

    program_full_color_mode20(&mut bus);
    run_ga1280_full_color_helper(&mut bus);
    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!((state.active_width, state.active_height), (512, 480));
    assert_eq!(
        bus.ga1280a_state().expect("GA board installed").plane_mode,
        Ga1280aPlaneMode::FullColor24
    );

    set_ga1280_direct_color16(&mut bus);
    assert_eq!(
        bus.ga1280a_state().expect("GA board installed").plane_mode,
        Ga1280aPlaneMode::FullColor24
    );

    write_palette(&mut bus, 0xFF, 0x00, 0xFF, 0x00);
    write_full_color_pixel(&mut bus, 0, 0, [0xFF, 0x00, 0x00]);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 0, [0xFF, 0x00, 0x00, 0xFF]);

    write_palette(&mut bus, 0xFF, 0x00, 0x00, 0xFF);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 0, [0xFF, 0x00, 0x00, 0xFF]);
}

#[test]
fn full_color_host_window_composes_rgb888_patterns() {
    let mut bus = setup_bus();
    program_full_color_mode20(&mut bus);

    for (x, rgb) in [
        (0, [0x00, 0x00, 0x00]),
        (1, [0xFF, 0xFF, 0xFF]),
        (2, [0xFF, 0x00, 0x00]),
        (3, [0x00, 0xFF, 0x00]),
        (4, [0x00, 0x00, 0xFF]),
        (5, [0x12, 0x80, 0xE4]),
    ] {
        write_full_color_pixel(&mut bus, x, 3, rgb);
    }

    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 3, [0x00, 0x00, 0x00, 0xFF]);
    assert_pixel(&bus, 1, 3, [0xFF, 0xFF, 0xFF, 0xFF]);
    assert_pixel(&bus, 2, 3, [0xFF, 0x00, 0x00, 0xFF]);
    assert_pixel(&bus, 3, 3, [0x00, 0xFF, 0x00, 0xFF]);
    assert_pixel(&bus, 4, 3, [0x00, 0x00, 0xFF, 0xFF]);
    assert_pixel(&bus, 5, 3, [0x12, 0x80, 0xE4, 0xFF]);
}

#[test]
fn direct_color16_xor_sentinel_still_uses_rgb565_planes() {
    let mut bus = setup_bus();
    set_ga1280_direct_color16(&mut bus);

    set_normal_mix(&mut bus);
    fill_rect(&mut bus, 2, 2, 1, 1, 0xF800);
    set_xor_mix(&mut bus);
    fill_rect(&mut bus, 2, 2, 1, 1, 0xFFDF);

    assert_eq!(read_pixel(&mut bus, 2, 2), 0x07DF);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 2, 2, [0x00, 0xFB, 0xFF, 0xFF]);
}

#[test]
fn pixel_mode_image_restore_streams_indexed_pixels() {
    let mut bus = setup_bus();
    for (index, rgb) in [
        (1, [0x20, 0x00, 0x00]),
        (2, [0x00, 0x40, 0x00]),
        (3, [0x00, 0x00, 0x60]),
        (4, [0x80, 0x80, 0x00]),
    ] {
        write_palette(&mut bus, index, rgb[0], rgb[1], rgb[2]);
    }

    start_image_restore(&mut bus, 10, 4, 4, 1);
    bus.io_write_word(ga_port(0x1C, 2), 0x0201);
    bus.io_write_word(ga_port(0x1C, 2), 0x0403);

    assert!(matches!(
        bus.ga1280a_state().expect("GA board installed").stream,
        Ga1280aStreamState::Inactive
    ));
    bus.ga1280a_present_now();
    assert_pixel(&bus, 10, 4, [0x20, 0x00, 0x00, 0xFF]);
    assert_pixel(&bus, 11, 4, [0x00, 0x40, 0x00, 0xFF]);
    assert_pixel(&bus, 12, 4, [0x00, 0x00, 0x60, 0xFF]);
    assert_pixel(&bus, 13, 4, [0x80, 0x80, 0x00, 0xFF]);
}

#[test]
fn pixel_mode_image_restore_ignores_indexed_row_padding() {
    let mut bus = setup_bus();
    for index in 1..=6 {
        let value = index * 0x20;
        write_palette(&mut bus, index, value, value, value);
    }
    write_palette(&mut bus, 0xEE, 0xEE, 0x00, 0x00);

    start_image_restore(&mut bus, 10, 4, 3, 2);
    bus.io_write_word(ga_port(0x1C, 2), 0x0201);
    bus.io_write_word(ga_port(0x1C, 2), 0xEE03);
    bus.io_write_word(ga_port(0x1C, 2), 0x0504);
    bus.io_write_word(ga_port(0x1C, 2), 0xEE06);

    bus.ga1280a_present_now();
    assert_pixel(&bus, 10, 4, [0x20, 0x20, 0x20, 0xFF]);
    assert_pixel(&bus, 11, 4, [0x40, 0x40, 0x40, 0xFF]);
    assert_pixel(&bus, 12, 4, [0x60, 0x60, 0x60, 0xFF]);
    assert_pixel(&bus, 10, 5, [0x80, 0x80, 0x80, 0xFF]);
    assert_pixel(&bus, 11, 5, [0xA0, 0xA0, 0xA0, 0xFF]);
    assert_pixel(&bus, 12, 5, [0xC0, 0xC0, 0xC0, 0xFF]);
}

#[test]
fn pixel_mode_image_restore_honors_xor_mix() {
    let mut bus = setup_bus();
    write_palette(&mut bus, 3, 0x30, 0x00, 0x00);
    write_palette(&mut bus, 4, 0x80, 0x80, 0x80);
    write_palette(&mut bus, 7, 0xFF, 0xFF, 0xFF);

    set_normal_mix(&mut bus);
    fill_rect(&mut bus, 10, 4, 2, 1, 4);
    bus.io_write_byte(ga_port(0x14, 0), 0x06);
    bus.io_write_word(ga_port(0x1E, 2), 0x4000);
    start_image_restore(&mut bus, 10, 4, 2, 1);
    bus.io_write_word(ga_port(0x1C, 2), 0x0700);

    bus.ga1280a_present_now();
    assert_pixel(&bus, 10, 4, [0x80, 0x80, 0x80, 0xFF]);
    assert_pixel(&bus, 11, 4, [0x30, 0x00, 0x00, 0xFF]);
}

#[test]
fn icon_style_mask_and_xor_image_sequence_preserves_black_and_transparency() {
    let mut bus = setup_bus();
    write_palette(&mut bus, 0, 0x00, 0x00, 0x00);
    write_palette(&mut bus, 4, 0x80, 0x80, 0x80);
    write_palette(&mut bus, 7, 0xFF, 0xFF, 0xFF);

    set_normal_mix(&mut bus);
    fill_rect(&mut bus, 10, 4, 20, 1, 4);

    bus.io_write_word(ga_port(0x10, 0), 0x00FF);
    bus.io_write_word(ga_port(0x12, 0), 0x0000);
    bus.io_write_word(ga_port(0x14, 0), 0x0808);
    bus.io_write_word(ga_port(0x0A, 2), 10);
    bus.io_write_word(ga_port(0x0B, 2), 4);
    bus.io_write_word(ga_port(0x04, 2), 19);
    bus.io_write_word(ga_port(0x05, 2), 0);
    bus.io_write_word(ga_port(0x1E, 2), 0x6000);
    bus.io_write_word(ga_port(0x1F, 2), 0x4A88);
    bus.io_write_word(ga_port(0x1C, 2), 0xFFBF);
    bus.io_write_word(ga_port(0x1C, 2), 0x0030);

    bus.io_write_word(ga_port(0x14, 0), 0x0606);
    bus.io_write_word(ga_port(0x1E, 2), 0x4000);
    start_image_restore(&mut bus, 10, 4, 20, 1);
    bus.io_write_word(ga_port(0x1C, 2), 0x0000);
    for _ in 0..7 {
        bus.io_write_word(ga_port(0x1C, 2), 0x0000);
    }
    bus.io_write_word(ga_port(0x1C, 2), 0x0700);
    bus.io_write_word(ga_port(0x1C, 2), 0x0000);

    bus.ga1280a_present_now();
    assert_pixel(&bus, 10, 4, [0x80, 0x80, 0x80, 0xFF]);
    assert_pixel(&bus, 11, 4, [0x00, 0x00, 0x00, 0xFF]);
    assert_pixel(&bus, 26, 4, [0x00, 0x00, 0x00, 0xFF]);
    assert_pixel(&bus, 27, 4, [0xFF, 0xFF, 0xFF, 0xFF]);
    assert_pixel(&bus, 28, 4, [0x80, 0x80, 0x80, 0xFF]);
}

#[test]
fn pixel_mode_image_restore_streams_rgb565_and_rgb888_pixels() {
    let mut bus = setup_bus();
    set_ga1280_direct_color16(&mut bus);
    start_image_restore(&mut bus, 20, 6, 2, 1);
    bus.io_write_word(ga_port(0x1C, 2), 0xF800);
    bus.io_write_word(ga_port(0x1C, 2), 0x07E0);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 20, 6, [0xFF, 0x00, 0x00, 0xFF]);
    assert_pixel(&bus, 21, 6, [0x00, 0xFF, 0x00, 0xFF]);

    let mut bus = setup_bus();
    program_full_color_mode20(&mut bus);
    start_image_restore(&mut bus, 30, 8, 2, 1);
    bus.io_write_word(ga_port(0x1C, 2), 0x3412);
    bus.io_write_word(ga_port(0x1C, 2), 0xAA56);
    bus.io_write_word(ga_port(0x1C, 2), 0xCCBB);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 30, 8, [0x12, 0x34, 0x56, 0xFF]);
    assert_pixel(&bus, 31, 8, [0xAA, 0xBB, 0xCC, 0xFF]);
}

#[test]
fn pixel_mode_image_restore_ignores_direct_color16_row_padding() {
    let mut bus = setup_bus();
    set_ga1280_direct_color16(&mut bus);

    start_image_restore(&mut bus, 20, 6, 3, 2);
    bus.io_write_word(ga_port(0x1C, 2), 0xF800);
    bus.io_write_word(ga_port(0x1C, 2), 0x07E0);
    bus.io_write_word(ga_port(0x1C, 2), 0x001F);
    bus.io_write_word(ga_port(0x1C, 2), 0xFFFF);
    bus.io_write_word(ga_port(0x1C, 2), 0xFFE0);
    bus.io_write_word(ga_port(0x1C, 2), 0xF81F);
    bus.io_write_word(ga_port(0x1C, 2), 0x07FF);
    bus.io_write_word(ga_port(0x1C, 2), 0xFFFF);

    assert!(matches!(
        bus.ga1280a_state().expect("GA board installed").stream,
        Ga1280aStreamState::Inactive
    ));
    bus.ga1280a_present_now();
    assert_pixel(&bus, 20, 6, [0xFF, 0x00, 0x00, 0xFF]);
    assert_pixel(&bus, 21, 6, [0x00, 0xFF, 0x00, 0xFF]);
    assert_pixel(&bus, 22, 6, [0x00, 0x00, 0xFF, 0xFF]);
    assert_pixel(&bus, 20, 7, [0xFF, 0xFF, 0x00, 0xFF]);
    assert_pixel(&bus, 21, 7, [0xFF, 0x00, 0xFF, 0xFF]);
    assert_pixel(&bus, 22, 7, [0x00, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn plane_byte_align_and_push_pop_style_transfers_are_board_local() {
    let mut bus = setup_bus();
    write_palette(&mut bus, 1, 0x40, 0x00, 0x00);
    write_palette(&mut bus, 2, 0x00, 0x50, 0x00);
    write_palette(&mut bus, 3, 0x00, 0x00, 0x60);

    bus.io_write_word(ga_port(0x03, 0), 0x0001);
    bus.write_byte(WINDOW_BASE, 0x80);
    bus.io_write_word(ga_port(0x03, 0), 0x0002);
    bus.write_byte(WINDOW_BASE + 1, 0x40);

    bus.io_write_word(ga_port(0x03, 0), 0xFFFF);
    set_normal_mix(&mut bus);
    fill_rect(&mut bus, 40, 10, 1, 1, 3);
    bus.io_write_word(ga_port(0x08, 2), 40);
    bus.io_write_word(ga_port(0x09, 2), 10);
    bus.io_write_word(ga_port(0x0A, 2), 44);
    bus.io_write_word(ga_port(0x0B, 2), 12);
    bus.io_write_word(ga_port(0x04, 2), 0);
    bus.io_write_word(ga_port(0x05, 2), 0);
    bus.io_write_word(ga_port(0x1F, 2), 0x60E8);

    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 0, [0x40, 0x00, 0x00, 0xFF]);
    assert_pixel(&bus, 9, 0, [0x00, 0x50, 0x00, 0xFF]);
    assert_pixel(&bus, 44, 12, [0x00, 0x00, 0x60, 0xFF]);
}
