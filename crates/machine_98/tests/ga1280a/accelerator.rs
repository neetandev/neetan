use common::{Bus, CpuMode, MachineModel, NoTrace};
use device::ga1280a::Ga1280aPlaneMode;
use machine_98::Pc9801Bus;

const GA_GAPORT: u16 = 0x00D8;
const OPCODE_PIXEL_READ: u16 = 0x20E8;
const OPCODE_OPAQUE_PATTERN_EXPAND_RECTANGLE: u16 = 0x4A88;
const OPCODE_SOLID_RECTANGLE: u16 = 0x6FE8;
const POP1_NORMAL_WRITE: u16 = 0x1000;
const POP1_SCANLINE_PIXEL_READ: u16 = 0x3000;
const POP1_OPAQUE_PATTERN_EXPAND_RECTANGLE: u16 = 0x7000;
const MIX_SOURCE: u8 = 0x0C;

fn ga_port(selector: u8, offset: u8) -> u16 {
    (u16::from(selector) << 8) | (GA_GAPORT + u16::from(offset))
}

fn setup_bus() -> Pc9801Bus<NoTrace> {
    let mut bus = Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48000);
    bus.install_ga1280a();
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

fn assert_pixel(bus: &Pc9801Bus<NoTrace>, x: u32, y: u32, expected: [u8; 4]) {
    let (width, _) = bus.display_dimensions();
    let offset = ((y * width + x) * 4) as usize;
    assert_eq!(&bus.display_framebuffer()[offset..offset + 4], &expected);
}

fn set_normal_mix(bus: &mut Pc9801Bus<NoTrace>) {
    bus.io_write_byte(ga_port(0x14, 0), MIX_SOURCE);
    bus.io_write_word(ga_port(0x1E, 2), POP1_NORMAL_WRITE);
}

fn set_xor_mix(bus: &mut Pc9801Bus<NoTrace>) {
    bus.io_write_byte(ga_port(0x14, 0), 0x06);
    bus.io_write_word(ga_port(0x1E, 2), 0x0000);
}

fn fill_rect(bus: &mut Pc9801Bus<NoTrace>, x: u16, y: u16, width: u16, height: u16, color: u16) {
    bus.io_write_word(ga_port(0x09, 0), color);
    bus.io_write_word(ga_port(0x0A, 2), x);
    bus.io_write_word(ga_port(0x0B, 2), y);
    bus.io_write_word(ga_port(0x04, 2), width - 1);
    bus.io_write_word(ga_port(0x05, 2), height - 1);
    bus.io_write_word(ga_port(0x1F, 2), OPCODE_SOLID_RECTANGLE);
}

fn read_pixel(bus: &mut Pc9801Bus<NoTrace>, x: u16, y: u16) -> u16 {
    bus.io_write_word(ga_port(0x08, 2), x);
    bus.io_write_word(ga_port(0x09, 2), y);
    bus.io_write_word(ga_port(0x04, 2), 6);
    bus.io_write_word(ga_port(0x05, 2), 0);
    set_normal_mix(bus);
    bus.io_write_word(ga_port(0x1F, 2), OPCODE_PIXEL_READ);
    bus.io_read_word(ga_port(0x1C, 2))
}

fn scanline_word_count(width: u16, height: u16) -> usize {
    usize::from(width.div_ceil(16)) * usize::from(height)
}

fn pattern_word_bit_mask(column: u16) -> u16 {
    if column < 8 {
        0x0080 >> column
    } else if column < 16 {
        0x8000 >> (column - 8)
    } else {
        0
    }
}

fn full_pattern_words(width: u16, height: u16) -> Vec<u16> {
    let words_per_row = width.div_ceil(16);
    let mut words = Vec::with_capacity(scanline_word_count(width, height));
    for _ in 0..height {
        for word_index in 0..words_per_row {
            let mut value = 0;
            for column in 0..16 {
                if word_index * 16 + column < width {
                    value |= pattern_word_bit_mask(column);
                }
            }
            words.push(value);
        }
    }
    words
}

fn save_scanline_plane_words(
    bus: &mut Pc9801Bus<NoTrace>,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    plane_mask: u16,
) -> Vec<u16> {
    bus.io_write_word(ga_port(0x07, 0), plane_mask);
    bus.io_write_word(ga_port(0x08, 2), x);
    bus.io_write_word(ga_port(0x09, 2), y);
    bus.io_write_word(ga_port(0x04, 2), width - 1);
    bus.io_write_word(ga_port(0x05, 2), height - 1);
    bus.io_write_word(ga_port(0x1E, 2), POP1_SCANLINE_PIXEL_READ);
    bus.io_write_word(ga_port(0x1F, 2), OPCODE_PIXEL_READ);

    (0..scanline_word_count(width, height))
        .map(|_| bus.io_read_word(ga_port(0x1C, 2)))
        .collect()
}

fn restore_opaque_pattern_plane_words(
    bus: &mut Pc9801Bus<NoTrace>,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    plane_mask: u16,
    words: &[u16],
) {
    assert_eq!(words.len(), scanline_word_count(width, height));

    bus.io_write_word(ga_port(0x03, 0), plane_mask);
    bus.io_write_word(ga_port(0x10, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x12, 0), 0x0000);
    bus.io_write_word(
        ga_port(0x14, 0),
        u16::from(MIX_SOURCE) | (u16::from(MIX_SOURCE) << 8),
    );
    bus.io_write_word(ga_port(0x0A, 2), x);
    bus.io_write_word(ga_port(0x0B, 2), y);
    bus.io_write_word(ga_port(0x04, 2), width - 1);
    bus.io_write_word(ga_port(0x05, 2), height - 1);
    bus.io_write_word(ga_port(0x1E, 2), POP1_OPAQUE_PATTERN_EXPAND_RECTANGLE);
    bus.io_write_word(ga_port(0x1F, 2), OPCODE_OPAQUE_PATTERN_EXPAND_RECTANGLE);

    for word in words {
        bus.io_write_word(ga_port(0x1C, 2), *word);
    }
}

fn write_line_registers(bus: &mut Pc9801Bus<NoTrace>, sx: i16, sy: i16, ex: i16, ey: i16) -> u8 {
    let dx = (ex - sx).unsigned_abs();
    let dy = (ey - sy).unsigned_abs();
    let major = dx.max(dy);
    let minor = dx.min(dy);
    let mut direction = 0u8;
    if ex < sx {
        direction |= 0x04;
    }
    if ey < sy {
        direction |= 0x02;
    }
    if dx < dy {
        direction |= 0x01;
    }

    let k1 = (2 * minor) as i16;
    let k2 = (2 * minor as i32 - 2 * major as i32) as i16;
    let errs = (2 * minor as i32 - major as i32) as i16;
    bus.io_write_word(ga_port(0x0A, 2), sx as u16);
    bus.io_write_word(ga_port(0x0B, 2), sy as u16);
    bus.io_write_word(ga_port(0x04, 2), major);
    bus.io_write_word(ga_port(0x01, 2), errs as u16);
    bus.io_write_word(ga_port(0x02, 2), k1 as u16);
    bus.io_write_word(ga_port(0x03, 2), k2 as u16);
    direction
}

fn set_ga1280_plane_mode(bus: &mut Pc9801Bus<NoTrace>, plane_mode: Ga1280aPlaneMode) {
    bus.io_write_byte(ga_port(0x18, 1), 2);
    match plane_mode {
        Ga1280aPlaneMode::Indexed8 => bus.io_write_byte(ga_port(0x18, 0), 0x48),
        Ga1280aPlaneMode::DirectColor16 => bus.io_write_byte(ga_port(0x18, 0), 0x38),
        Ga1280aPlaneMode::FullColor24 => unreachable!("use mode 20/21 CRTC setup"),
    }
    bus.io_write_byte(ga_port(0x18, 1), 0);
}

#[test]
fn io_data_ga_accelerator_solid_rectangle_fill_and_pixel_readback_work_without_hdi_image() {
    let mut bus = setup_bus();
    write_palette(&mut bus, 5, 0x10, 0x80, 0x30);

    set_normal_mix(&mut bus);
    fill_rect(&mut bus, 2, 3, 4, 3, 5);

    assert_eq!(read_pixel(&mut bus, 4, 4), 5);
    assert_eq!(bus.io_read_word(ga_port(0x1C, 2)), 5);

    bus.ga1280a_present_now();
    assert_pixel(&bus, 2, 3, [0x10, 0x80, 0x30, 0xFF]);
    assert_pixel(&bus, 5, 5, [0x10, 0x80, 0x30, 0xFF]);
    assert_pixel(&bus, 6, 5, [0x00, 0x00, 0x00, 0xFF]);
}

#[test]
fn io_data_ga_accelerator_alternate_solid_rectangle_opcode_matches_box_fill_path() {
    let mut bus = setup_bus();

    set_normal_mix(&mut bus);
    bus.io_write_word(ga_port(0x09, 0), 0x0F);
    bus.io_write_word(ga_port(0x0A, 2), 488);
    bus.io_write_word(ga_port(0x0B, 2), 30);
    bus.io_write_word(ga_port(0x04, 2), 67);
    bus.io_write_word(ga_port(0x05, 2), 19);
    bus.io_write_word(ga_port(0x1F, 2), 0x4FF8);

    assert_eq!(read_pixel(&mut bus, 488, 30), 0x0F);
    assert_eq!(read_pixel(&mut bus, 555, 49), 0x0F);
    assert_eq!(read_pixel(&mut bus, 556, 49), 0x00);
}

#[test]
fn io_data_ga_accelerator_xor_pixel_toggles_active_indexed_planes() {
    let mut bus = setup_bus();

    set_normal_mix(&mut bus);
    fill_rect(&mut bus, 10, 10, 1, 1, 0x03);
    set_xor_mix(&mut bus);
    fill_rect(&mut bus, 10, 10, 1, 1, 0xFFFF);

    assert_eq!(read_pixel(&mut bus, 10, 10), 0x00FC);
}

#[test]
fn io_data_ga_accelerator_copy_rectangle_uses_direction_for_overlapping_right_shift() {
    let mut bus = setup_bus();

    set_normal_mix(&mut bus);
    for x in 0..5 {
        fill_rect(&mut bus, x, 0, 1, 1, x + 1);
    }

    bus.io_write_word(ga_port(0x08, 2), 4);
    bus.io_write_word(ga_port(0x09, 2), 0);
    bus.io_write_word(ga_port(0x0A, 2), 5);
    bus.io_write_word(ga_port(0x0B, 2), 0);
    bus.io_write_word(ga_port(0x04, 2), 4);
    bus.io_write_word(ga_port(0x05, 2), 0);
    bus.io_write_word(ga_port(0x1F, 2), 0x60EC);

    let row: Vec<u16> = (0..=5).map(|x| read_pixel(&mut bus, x, 0)).collect();
    assert_eq!(row, [1, 1, 2, 3, 4, 5]);
}

#[test]
fn io_data_ga_accelerator_solid_and_styled_lines_follow_galib_line_registers() {
    let mut bus = setup_bus();

    set_normal_mix(&mut bus);
    bus.io_write_word(ga_port(0x09, 0), 2);
    let direction = write_line_registers(&mut bus, 20, 20, 25, 23);
    bus.io_write_word(ga_port(0x1F, 2), 0x1FE8 | u16::from(direction));

    for (x, y) in [(20, 20), (21, 21), (22, 21), (23, 22), (24, 22), (25, 23)] {
        assert_eq!(read_pixel(&mut bus, x, y), 2);
    }
    assert_eq!(read_pixel(&mut bus, 22, 22), 0);

    bus.io_write_word(ga_port(0x09, 0), 3);
    bus.io_write_word(ga_port(0x06, 2), 0xAAAA);
    let direction = write_line_registers(&mut bus, 30, 20, 35, 20);
    bus.io_write_word(ga_port(0x1F, 2), 0x1348 | u16::from(direction));

    for x in [30, 32, 34] {
        assert_eq!(read_pixel(&mut bus, x, 20), 3);
    }
    for x in [31, 33, 35] {
        assert_eq!(read_pixel(&mut bus, x, 20), 0);
    }
}

#[test]
fn io_data_ga_accelerator_clipping_control_supports_inside_and_outside_regions() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x15, 0), 0x0002);
    bus.io_write_word(ga_port(0x15, 0), 0x1002);
    bus.io_write_word(ga_port(0x15, 0), 0x2004);
    bus.io_write_word(ga_port(0x15, 0), 0x3004);
    bus.io_write_word(ga_port(0x15, 0), 0x4001);

    set_normal_mix(&mut bus);
    fill_rect(&mut bus, 0, 0, 7, 7, 1);
    assert_eq!(read_pixel(&mut bus, 1, 1), 0);
    assert_eq!(read_pixel(&mut bus, 2, 2), 1);
    assert_eq!(read_pixel(&mut bus, 4, 4), 1);
    assert_eq!(read_pixel(&mut bus, 5, 5), 0);

    bus.io_write_word(ga_port(0x15, 0), 0x4003);
    fill_rect(&mut bus, 0, 0, 7, 7, 2);
    assert_eq!(read_pixel(&mut bus, 1, 1), 2);
    assert_eq!(read_pixel(&mut bus, 2, 2), 1);
    assert_eq!(read_pixel(&mut bus, 4, 4), 1);
    assert_eq!(read_pixel(&mut bus, 5, 5), 2);
}

#[test]
fn io_data_ga_accelerator_rop_line_uses_documented_xor_operation() {
    let mut bus = setup_bus();

    set_normal_mix(&mut bus);
    fill_rect(&mut bus, 40, 5, 6, 1, 0x03);

    bus.io_write_word(ga_port(0x10, 0), 0x05);
    bus.io_write_word(ga_port(0x12, 0), 0x00);
    bus.io_write_byte(ga_port(0x14, 0), 0x06);
    bus.io_write_byte(ga_port(0x14, 1), 0x0A);
    bus.io_write_word(ga_port(0x06, 2), 0xFFFF);
    let direction = write_line_registers(&mut bus, 40, 5, 45, 5);
    bus.io_write_word(ga_port(0x1F, 2), 0x1A48 | u16::from(direction));

    for x in 40..=45 {
        assert_eq!(read_pixel(&mut bus, x, 5), 0x06);
    }
}

#[test]
fn io_data_ga_accelerator_indexed_scanline_save_restores_opaque_pattern_without_hdi_image() {
    let mut bus = setup_bus();
    let x = 32;
    let y = 43;
    let width = 18;
    let height = 2;
    let expected_words = full_pattern_words(width, height);

    set_normal_mix(&mut bus);
    fill_rect(&mut bus, x, y, width, height, 0x0F);

    let saved_planes: Vec<_> = [0x0001, 0x0002, 0x0004, 0x0008]
        .into_iter()
        .map(|plane_mask| {
            let words = save_scanline_plane_words(&mut bus, x, y, width, height, plane_mask);
            assert_eq!(words, expected_words, "plane mask {plane_mask:04X}");
            (plane_mask, words)
        })
        .collect();
    assert_eq!(
        save_scanline_plane_words(&mut bus, x, y, width, height, 0x0010),
        vec![0; expected_words.len()]
    );

    fill_rect(&mut bus, x, y, width, height, 0x00);
    for (plane_mask, words) in saved_planes {
        restore_opaque_pattern_plane_words(&mut bus, x, y, width, height, plane_mask, &words);
    }

    for row in 0..height {
        for column in 0..width {
            assert_eq!(read_pixel(&mut bus, x + column, y + row), 0x0F);
        }
    }
    assert_eq!(read_pixel(&mut bus, x + width, y), 0x00);
}

#[test]
fn ga1280_accelerator_direct_color16_fill_and_readback_use_all_sixteen_planes() {
    let mut bus = setup_bus();
    set_ga1280_plane_mode(&mut bus, Ga1280aPlaneMode::DirectColor16);

    set_normal_mix(&mut bus);
    fill_rect(&mut bus, 2, 2, 2, 2, 0xF800);

    assert_eq!(read_pixel(&mut bus, 2, 2), 0xF800);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 2, 2, [0xFF, 0x00, 0x00, 0xFF]);
}

#[test]
fn ga1280_accelerator_direct_color16_scanline_save_restores_high_plane_without_hdi_image() {
    let mut bus = setup_bus();
    let x = 48;
    let y = 12;
    let width = 18;
    let height = 1;
    let high_plane_mask = 0x8000;
    let expected_words = full_pattern_words(width, height);

    set_ga1280_plane_mode(&mut bus, Ga1280aPlaneMode::DirectColor16);
    set_normal_mix(&mut bus);
    fill_rect(&mut bus, x, y, width, height, high_plane_mask);

    let saved_words = save_scanline_plane_words(&mut bus, x, y, width, height, high_plane_mask);
    assert_eq!(saved_words, expected_words);
    assert_eq!(
        save_scanline_plane_words(&mut bus, x, y, width, height, 0x0001),
        vec![0; expected_words.len()]
    );

    fill_rect(&mut bus, x, y, width, height, 0x0000);
    restore_opaque_pattern_plane_words(
        &mut bus,
        x,
        y,
        width,
        height,
        high_plane_mask,
        &saved_words,
    );

    for column in 0..width {
        assert_eq!(read_pixel(&mut bus, x + column, y), high_plane_mask);
    }
    assert_eq!(read_pixel(&mut bus, x + width, y), 0x0000);
}
