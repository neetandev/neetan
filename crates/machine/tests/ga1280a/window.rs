use common::{Bus, CpuMode, MachineModel};
use device::ga1280a::Ga1280aPlaneMode;
use machine::{NoTracing, Pc9801Bus};

const GA_GAPORT: u16 = 0x00D8;
const WINDOW_BASE: u32 = 0xC0000;
const GATEST_WINDOW_BASE: u32 = 0xC4000;
const GALIB_WINDOW_BASE: u32 = 0xE0000;
const FLAT_APERTURE_BASE: u32 = 0xF00000;
const COMPATIBILITY_MAPPED_REGISTER_BASE_OFFSET: u32 = 0x1F00;
const MAPPED_REGISTER_BASE_OFFSET: u32 = 0x3F00;
const MAPPED_REGISTER_PLUS_TWO_OFFSET: u32 = 0x3F40;
const HGA_MAPPED_REGISTER_BASE_OFFSET: u32 = 0x7F00;
const HGA_MAPPED_REGISTER_PLUS_TWO_OFFSET: u32 = 0x7F40;
const START_RASTER_WRITE_REGISTER_OFFSET: u32 = 2;

fn ga_port(selector: u8, offset: u8) -> u16 {
    (u16::from(selector) << 8) | (GA_GAPORT + u16::from(offset))
}

fn mapped_register_address(selector: u8, offset: u8) -> u32 {
    let aperture_base = match offset {
        0 | 1 => MAPPED_REGISTER_BASE_OFFSET,
        2 | 3 => MAPPED_REGISTER_PLUS_TWO_OFFSET,
        _ => panic!("unsupported mapped register offset"),
    };
    WINDOW_BASE + aperture_base + u32::from(selector) * 2 + u32::from(offset & 1)
}

fn hga_mapped_register_address(selector: u8, offset: u8) -> u32 {
    let aperture_base = match offset {
        0 | 1 => HGA_MAPPED_REGISTER_BASE_OFFSET,
        2 | 3 => HGA_MAPPED_REGISTER_PLUS_TWO_OFFSET,
        _ => panic!("unsupported mapped register offset"),
    };
    WINDOW_BASE + aperture_base + u32::from(selector) * 2 + u32::from(offset & 1)
}

fn hga_flat_mapped_register_address(selector: u8, offset: u8) -> u32 {
    let aperture_base = match offset {
        0 | 1 => HGA_MAPPED_REGISTER_BASE_OFFSET,
        2 | 3 => HGA_MAPPED_REGISTER_PLUS_TWO_OFFSET,
        _ => panic!("unsupported mapped register offset"),
    };
    FLAT_APERTURE_BASE + aperture_base + u32::from(selector) * 2 + u32::from(offset & 1)
}

fn setup_bus() -> Pc9801Bus<NoTracing> {
    let mut bus = Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48000);
    bus.install_ga1280a();
    bus.io_write_word(ga_port(0x16, 0), 0x20C1);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x03, 0), 0xFFFF);
    bus.io_write_byte(ga_port(0x07, 0), 0xFF);
    bus.io_write_byte(ga_port(0x1B, 0), 0xFF);
    bus
}

fn use_galib_wba2_window(bus: &mut Pc9801Bus<NoTracing>) {
    bus.io_write_word(ga_port(0x16, 0), 0x00E1);
    bus.io_write_word(ga_port(0x17, 0), 0x30C0);
}

fn write_palette(bus: &mut Pc9801Bus<NoTracing>, index: u8, red: u8, green: u8, blue: u8) {
    bus.io_write_byte(ga_port(0x18, 0), index);
    bus.io_write_byte(ga_port(0x1A, 0), red);
    bus.io_write_byte(ga_port(0x1A, 0), green);
    bus.io_write_byte(ga_port(0x1A, 0), blue);
}

fn set_ga1280_plane_mode(bus: &mut Pc9801Bus<NoTracing>, plane_mode: Ga1280aPlaneMode) {
    bus.io_write_byte(ga_port(0x18, 1), 2);
    match plane_mode {
        Ga1280aPlaneMode::Indexed8 => bus.io_write_byte(ga_port(0x18, 0), 0x48),
        Ga1280aPlaneMode::DirectColor16 => bus.io_write_byte(ga_port(0x18, 0), 0x38),
        Ga1280aPlaneMode::FullColor24 => unreachable!("use mode 20/21 CRTC setup"),
    }
    bus.io_write_byte(ga_port(0x18, 1), 0);
}

fn write_crtc_word(bus: &mut Pc9801Bus<NoTracing>, index: u8, value: u16) {
    bus.io_write_byte(ga_port(0x1E, 0), index);
    bus.io_write_word(ga_port(0x1F, 0), value);
}

fn set_display_start(bus: &mut Pc9801Bus<NoTracing>, value: u32) {
    write_crtc_word(bus, 0x30, (value & 0xFF) as u16);
    write_crtc_word(bus, 0x31, ((value >> 8) & 0xFF) as u16);
    write_crtc_word(bus, 0x32, ((value >> 16) & 0xFF) as u16);
}

fn set_normal_mix(bus: &mut Pc9801Bus<NoTracing>) {
    bus.io_write_byte(ga_port(0x14, 0), 0x0C);
    bus.io_write_word(ga_port(0x1E, 2), 0x1000);
}

fn fill_rect(bus: &mut Pc9801Bus<NoTracing>, x: u16, y: u16, width: u16, height: u16, color: u16) {
    bus.io_write_word(ga_port(0x09, 0), color);
    bus.io_write_word(ga_port(0x0A, 2), x);
    bus.io_write_word(ga_port(0x0B, 2), y);
    bus.io_write_word(ga_port(0x04, 2), width - 1);
    bus.io_write_word(ga_port(0x05, 2), height - 1);
    bus.io_write_word(ga_port(0x1F, 2), 0x6FE8);
}

fn assert_pixel(bus: &Pc9801Bus<NoTracing>, x: u32, y: u32, expected: [u8; 4]) {
    let (width, _) = bus.display_dimensions();
    let offset = ((y * width + x) * 4) as usize;
    assert_eq!(&bus.display_framebuffer()[offset..offset + 4], &expected);
}

#[test]
fn host_window_maps_only_configured_wba1_range() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x01, 0), 0);
    bus.io_write_word(ga_port(0x02, 0), 0);
    bus.io_write_word(ga_port(0x03, 0), 0x0001);
    bus.io_write_byte(ga_port(0x06, 0), 0);
    bus.write_byte(WINDOW_BASE, 0xAA);

    assert_eq!(bus.read_byte(WINDOW_BASE), 0xAA);
    assert_eq!(
        bus.read_byte(WINDOW_BASE + 16 * 1024),
        0xFF,
        "16 KB WBA1 window must not alias following expansion space"
    );
}

#[test]
fn fixed_window_register_alias_opens_host_window() {
    let mut bus = setup_bus();

    bus.io_write_byte(0x1600, 0x00);
    bus.io_write_byte(0x1601, 0x00);
    assert_eq!(
        bus.ga1280a_state().expect("GA board installed").wba1,
        0x0000
    );

    bus.io_write_byte(0x1600, 0xC1);
    bus.io_write_byte(0x1601, 0x20);
    assert_eq!(bus.io_read_byte(0x1600), 0xC1);
    assert_eq!(bus.io_read_byte(0x1601), 0x20);

    bus.io_write_word(ga_port(0x01, 0), 0);
    bus.io_write_word(ga_port(0x02, 0), 0);
    bus.io_write_word(ga_port(0x03, 0), 0x0001);
    bus.io_write_byte(ga_port(0x06, 0), 0);
    bus.write_byte(WINDOW_BASE, 0x5A);
    assert_eq!(bus.read_byte(WINDOW_BASE), 0x5A);
}

#[test]
fn wba2_size_fallback_opens_galib_e000_window() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x16, 0), 0x00E9);
    bus.io_write_word(ga_port(0x17, 0), 0x30E1);
    bus.io_write_word(ga_port(0x01, 0), 0);
    bus.io_write_word(ga_port(0x02, 0), 0);
    bus.io_write_word(ga_port(0x03, 0), 0x0001);
    bus.io_write_byte(ga_port(0x06, 0), 0);

    bus.write_byte(GALIB_WINDOW_BASE, 0xA5);
    assert_eq!(bus.read_byte(GALIB_WINDOW_BASE), 0xA5);
    assert_ne!(bus.read_byte(GALIB_WINDOW_BASE + 32 * 1024), 0xA5);
}

#[test]
fn pixel_map_width_controls_host_window_stride() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x03, 0), 0x0001);
    bus.io_write_word(ga_port(0x12, 2), 0x03FF);
    bus.io_write_word(ga_port(0x01, 0), 0);
    bus.write_byte(WINDOW_BASE + 128, 0x5A);

    bus.io_write_word(ga_port(0x02, 0), 1);
    assert_eq!(bus.read_byte(WINDOW_BASE), 0x5A);
    assert_ne!(bus.read_byte(WINDOW_BASE + 48), 0x5A);
}

#[test]
fn wba2_fallback_window_does_not_expose_mapped_register_aperture() {
    let mut bus = setup_bus();
    bus.io_write_word(ga_port(0x16, 0), 0x00E9);
    bus.io_write_word(ga_port(0x17, 0), 0x30E1);
    bus.io_write_word(ga_port(0x03, 0), 0x00FF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);

    let write_plane_mask_address = GALIB_WINDOW_BASE + MAPPED_REGISTER_BASE_OFFSET + 0x03 * 2;
    bus.write_word(write_plane_mask_address, 0x0000);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.wpm, 0x00FF);
}

#[test]
fn wba1_32k_window_exposes_hga_mapped_register_aperture() {
    let mut bus = setup_bus();
    bus.io_write_word(ga_port(0x16, 0), 0x30C1);
    bus.io_write_word(ga_port(0x03, 0), 0x00FF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);

    let write_plane_mask_address = hga_mapped_register_address(0x03, 0);
    bus.write_word(write_plane_mask_address, 0x0000);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.wpm, 0x0000);

    let destination_x_address = hga_mapped_register_address(0x0A, 2);
    bus.write_word(destination_x_address, 0x1234);
    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.dstx, 0x1234);
}

#[test]
fn wba1_16k_window_exposes_compatibility_and_stride_end_mapped_register_apertures() {
    let mut bus = setup_bus();
    bus.io_write_word(ga_port(0x16, 0), 0x20E1);

    let low_start_raster_write_address = GALIB_WINDOW_BASE
        + COMPATIBILITY_MAPPED_REGISTER_BASE_OFFSET
        + START_RASTER_WRITE_REGISTER_OFFSET;
    let end_start_raster_write_address =
        GALIB_WINDOW_BASE + MAPPED_REGISTER_BASE_OFFSET + START_RASTER_WRITE_REGISTER_OFFSET;

    bus.write_word(low_start_raster_write_address, 0x1234);
    assert_eq!(bus.read_word(low_start_raster_write_address), 0x1234);

    bus.write_word(end_start_raster_write_address, 0x5678);
    assert_eq!(bus.read_word(end_start_raster_write_address), 0x5678);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.srw, 0x5678);
}

#[test]
fn closed_wba1_mirrors_mapped_register_aperture_at_window_size_stride() {
    let mut bus = setup_bus();
    bus.io_write_word(ga_port(0x16, 0), 0x20C1);

    let c000_start_raster_write_address = mapped_register_address(0x01, 0);
    let c000_low_start_raster_write_address = WINDOW_BASE
        + COMPATIBILITY_MAPPED_REGISTER_BASE_OFFSET
        + START_RASTER_WRITE_REGISTER_OFFSET;
    let c400_start_raster_write_address =
        GATEST_WINDOW_BASE + MAPPED_REGISTER_BASE_OFFSET + START_RASTER_WRITE_REGISTER_OFFSET;
    let e000_low_start_raster_write_address = GALIB_WINDOW_BASE
        + COMPATIBILITY_MAPPED_REGISTER_BASE_OFFSET
        + START_RASTER_WRITE_REGISTER_OFFSET;
    let e000_start_raster_write_address =
        GALIB_WINDOW_BASE + MAPPED_REGISTER_BASE_OFFSET + START_RASTER_WRITE_REGISTER_OFFSET;
    let wba1_address = mapped_register_address(0x16, 0);
    bus.write_word(wba1_address, 0x0000);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.wba1, 0x0000);

    bus.write_word(c000_start_raster_write_address, 0x1234);
    assert_eq!(bus.read_word(c000_start_raster_write_address), 0x1234);

    bus.write_word(c000_low_start_raster_write_address, 0x3456);
    assert_eq!(bus.read_word(c000_low_start_raster_write_address), 0x3456);

    bus.write_word(c400_start_raster_write_address, 0x5678);
    assert_eq!(bus.read_word(c400_start_raster_write_address), 0x5678);

    bus.write_word(e000_low_start_raster_write_address, 0x789A);
    assert_eq!(bus.read_word(e000_low_start_raster_write_address), 0x789A);

    bus.write_word(e000_start_raster_write_address, 0x9ABC);
    assert_eq!(bus.read_word(e000_start_raster_write_address), 0x9ABC);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.srw, 0x9ABC);
}

#[test]
fn raw_e000_window_reads_selected_plane_with_rpe_clear() {
    let mut bus = setup_bus();
    bus.io_write_word(ga_port(0x16, 0), 0x30E1);
    bus.io_write_word(ga_port(0x01, 0), 0);
    bus.io_write_word(ga_port(0x02, 0), 0);
    bus.io_write_word(ga_port(0x03, 0), 0x0001);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_byte(ga_port(0x06, 0), 0);
    bus.io_write_byte(ga_port(0x07, 0), 0x00);

    bus.write_word(GALIB_WINDOW_BASE, 0xA5A5);

    assert_eq!(bus.read_word(GALIB_WINDOW_BASE), 0xA5A5);
}

#[test]
fn mod1_two_maps_base_registers_into_host_window() {
    let mut bus = setup_bus();
    bus.io_write_byte(ga_port(0x0E, 0), 0x02);

    let start_raster_write_address = mapped_register_address(0x01, 0);
    bus.write_word(start_raster_write_address, 0x1234);
    assert_eq!(bus.read_word(start_raster_write_address), 0x1234);
    assert_eq!(bus.read_byte(start_raster_write_address), 0x34);
    assert_eq!(bus.read_byte(start_raster_write_address + 1), 0x12);

    let mode_high_address = mapped_register_address(0x0E, 1);
    bus.write_byte(mode_high_address, 0x40);
    assert_eq!(bus.read_byte(mode_high_address), 0x40);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.srw, 0x1234);
    assert_eq!(state.mod1, 0x02);
    assert_eq!(state.mod2, 0x40);
}

#[test]
fn mod1_two_maps_plus_two_registers_into_host_window() {
    let mut bus = setup_bus();
    bus.io_write_byte(ga_port(0x0E, 0), 0x02);

    let error_status_address = mapped_register_address(0x01, 2);
    bus.write_word(error_status_address, 0x4567);
    assert_eq!(bus.read_word(error_status_address), 0x4567);
    assert_eq!(bus.read_byte(error_status_address), 0x67);
    assert_eq!(bus.read_byte(error_status_address + 1), 0x45);

    bus.write_byte(error_status_address + 1, 0x89);
    assert_eq!(bus.read_word(error_status_address), 0x8967);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.errs, 0x8967);
}

#[test]
fn mod1_two_mapped_tile_register_replays_last_eight_word_writes_in_order() {
    let mut bus = setup_bus();
    bus.io_write_byte(ga_port(0x0E, 0), 0x02);

    let tile_address = mapped_register_address(0x0B, 0);
    let pattern = [
        0x0800, 0x0701, 0x0602, 0x0503, 0x0404, 0x0305, 0x0206, 0x0107,
    ];

    for value in pattern {
        bus.write_word(tile_address, value);
    }

    for value in pattern {
        assert_eq!(bus.read_word(tile_address), value);
    }
    assert_eq!(bus.read_word(tile_address), pattern[0]);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.tile, 0x0107);
}

#[test]
fn all_plane_direct_window_writes_unpack_packed_indexed_pixels() {
    let mut bus = setup_bus();

    use_galib_wba2_window(&mut bus);
    write_palette(&mut bus, 0xA5, 0x22, 0x55, 0x88);
    bus.io_write_word(ga_port(0x01, 0), 1);
    bus.io_write_word(ga_port(0x03, 0), 0x00FF);
    bus.write_byte(WINDOW_BASE + 3, 0xA5);

    bus.io_write_word(ga_port(0x02, 0), 1);
    bus.io_write_word(ga_port(0x03, 0), 0x0001);
    for plane in 0..8 {
        bus.io_write_byte(ga_port(0x06, 0), plane);
        let expected = if 0xA5 & (1 << plane) != 0 { 0x10 } else { 0 };
        assert_eq!(bus.read_byte(WINDOW_BASE), expected);
    }

    bus.ga1280a_present_now();
    assert_pixel(&bus, 3, 1, [0x22, 0x55, 0x88, 0xFF]);
    assert_pixel(&bus, 4, 1, [0x00, 0x00, 0x00, 0xFF]);
}

#[test]
fn packed_indexed_host_reads_return_palette_indices() {
    let mut bus = setup_bus();

    use_galib_wba2_window(&mut bus);
    bus.io_write_word(ga_port(0x12, 2), 0x03FF);
    bus.io_write_word(ga_port(0x13, 2), 0x03FF);
    bus.io_write_word(ga_port(0x01, 0), 10);
    bus.io_write_word(ga_port(0x02, 0), 10);
    bus.write_byte(WINDOW_BASE + 19, 0xA5);

    assert_eq!(bus.read_byte(WINDOW_BASE + 19), 0xA5);
}

#[test]
fn flat_f00000_aperture_writes_indexed_pixels_on_pc9821() {
    let mut bus = Pc9801Bus::new(MachineModel::PC9821AP, CpuMode::High, 48000);
    bus.install_ga1280a();
    bus.io_write_byte(0xF2, 0);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x03, 0), 0x00FF);
    bus.io_write_byte(ga_port(0x1B, 0), 0xFF);
    write_palette(&mut bus, 0xA5, 0x22, 0x55, 0x88);

    bus.write_byte(FLAT_APERTURE_BASE + 3, 0xA5);

    assert_eq!(bus.read_byte(FLAT_APERTURE_BASE + 3), 0xA5);
    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.flat_aperture_write_count, 1);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 3, 0, [0x22, 0x55, 0x88, 0xFF]);
}

#[test]
fn flat_aperture_pixels_follow_crtc_display_start_on_pc9821() {
    let mut bus = Pc9801Bus::new(MachineModel::PC9821AP, CpuMode::High, 48000);
    bus.install_ga1280a();
    bus.io_write_byte(0xF2, 0);
    bus.io_write_word(ga_port(0x12, 2), 0x03FF);
    bus.io_write_word(ga_port(0x13, 2), 0x03FF);
    bus.io_write_word(ga_port(0x03, 0), 0x00FF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_byte(ga_port(0x1B, 0), 0xFF);
    write_palette(&mut bus, 0xA5, 0x22, 0x55, 0x88);

    let source_offset = 2 * 1024 + 8;
    bus.write_byte(FLAT_APERTURE_BASE + source_offset, 0xA5);

    set_display_start(&mut bus, source_offset / 4);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 0, [0x22, 0x55, 0x88, 0xFF]);

    set_display_start(&mut bus, 0);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 0, [0x00, 0x00, 0x00, 0xFF]);
    assert_pixel(&bus, 8, 2, [0x22, 0x55, 0x88, 0xFF]);
}

#[test]
fn wba1_f00000_window_uses_planar_raster_semantics_on_pc9821() {
    let mut bus = Pc9801Bus::new(MachineModel::PC9821AP, CpuMode::High, 48000);
    bus.install_ga1280a();
    bus.io_write_byte(0xF2, 0);
    bus.io_write_word(ga_port(0x16, 0), 0x3F01);
    bus.io_write_word(ga_port(0x01, 0), 2);
    bus.io_write_word(ga_port(0x03, 0), 0x0001);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_byte(ga_port(0x1B, 0), 0xFF);
    write_palette(&mut bus, 1, 0x22, 0x55, 0x88);

    bus.write_byte(FLAT_APERTURE_BASE, 0x80);

    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 2, [0x22, 0x55, 0x88, 0xFF]);
    assert_pixel(&bus, 1, 2, [0x00, 0x00, 0x00, 0xFF]);
}

#[test]
fn wba1_f00000_window_exposes_hga_mapped_registers_on_pc9821() {
    let mut bus: Pc9801Bus<NoTracing> =
        Pc9801Bus::new(MachineModel::PC9821AP, CpuMode::High, 48000);
    bus.install_ga1280a();
    bus.io_write_byte(0xF2, 0);
    bus.io_write_word(ga_port(0x16, 0), 0x3F01);

    let write_plane_mask_address = hga_flat_mapped_register_address(0x03, 0);
    bus.write_word(write_plane_mask_address, 0x0002);
    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.wpm, 0x0002);

    let destination_x_address = hga_flat_mapped_register_address(0x0A, 2);
    bus.write_word(destination_x_address, 0x1234);
    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.dstx, 0x1234);
    assert_eq!(state.unknown_command_warning_count, 0);
}

#[test]
fn wba1_f00000_window_executes_hga_rop_rectangle_on_pc9821() {
    let mut bus: Pc9801Bus<NoTracing> =
        Pc9801Bus::new(MachineModel::PC9821AP, CpuMode::High, 48000);
    bus.install_ga1280a();
    bus.io_write_byte(0xF2, 0);
    bus.io_write_word(ga_port(0x16, 0), 0x3F01);
    bus.io_write_word(ga_port(0x03, 0), 0x00FF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_byte(ga_port(0x1B, 0), 0xFF);
    write_palette(&mut bus, 1, 0x00, 0x66, 0x00);
    write_palette(&mut bus, 6, 0xCC, 0x00, 0x00);

    set_normal_mix(&mut bus);
    fill_rect(&mut bus, 3, 4, 1, 1, 0x03);
    fill_rect(&mut bus, 4, 4, 1, 1, 0x04);

    bus.write_word(hga_flat_mapped_register_address(0x10, 0), 0x0005);
    bus.write_word(hga_flat_mapped_register_address(0x14, 0), 0x0606);
    bus.write_word(hga_flat_mapped_register_address(0x0A, 2), 3);
    bus.write_word(hga_flat_mapped_register_address(0x0B, 2), 4);
    bus.write_word(hga_flat_mapped_register_address(0x04, 2), 1);
    bus.write_word(hga_flat_mapped_register_address(0x05, 2), 0);
    bus.write_word(hga_flat_mapped_register_address(0x1F, 2), 0x6A28);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.pop2, 0x6A28);
    assert_eq!(state.unknown_command_warning_count, 0);

    bus.ga1280a_present_now();
    assert_pixel(&bus, 3, 4, [0xCC, 0x00, 0x00, 0xFF]);
    assert_pixel(&bus, 4, 4, [0x00, 0x66, 0x00, 0xFF]);
}

#[test]
fn packed_indexed_host_writes_use_pixel_map_stride() {
    let mut bus = setup_bus();

    use_galib_wba2_window(&mut bus);
    bus.io_write_word(ga_port(0x12, 2), 0x03FF);
    bus.io_write_word(ga_port(0x13, 2), 0x03FF);
    write_palette(&mut bus, 0xA5, 0x22, 0x55, 0x88);
    bus.io_write_word(ga_port(0x01, 0), 10);
    bus.write_byte(WINDOW_BASE + 1024 + 3, 0xA5);

    bus.ga1280a_present_now();
    assert_pixel(&bus, 3, 11, [0x22, 0x55, 0x88, 0xFF]);
    assert_pixel(&bus, 3, 10, [0x00, 0x00, 0x00, 0xFF]);
}

#[test]
fn packed_indexed_host_writes_wrap_high_raster_bits() {
    let mut bus = setup_bus();

    use_galib_wba2_window(&mut bus);
    bus.io_write_word(ga_port(0x12, 2), 0x03FF);
    bus.io_write_word(ga_port(0x13, 2), 0x03FF);
    write_palette(&mut bus, 0xA5, 0x22, 0x55, 0x88);
    bus.io_write_word(ga_port(0x01, 0), 0xE0C0);
    bus.write_byte(WINDOW_BASE + 7, 0xA5);

    bus.io_write_word(ga_port(0x01, 0), 0xC0C0);
    bus.write_byte(WINDOW_BASE + 7, 0x00);

    bus.ga1280a_present_now();
    assert_pixel(&bus, 7, 0xC0, [0x00, 0x00, 0x00, 0xFF]);
}

#[test]
fn rance_wba2_pixels_and_gatest_wba1_planes_share_all_plane_flags() {
    let mut bus = setup_bus();

    // Rance/GALIB leaves WBA1 disabled and carries the C000h aperture in WBA2.
    use_galib_wba2_window(&mut bus);
    bus.io_write_word(ga_port(0x12, 2), 0x03FF);
    bus.io_write_word(ga_port(0x13, 2), 0x03FF);
    bus.io_write_word(ga_port(0x01, 0), 10);
    bus.io_write_word(ga_port(0x02, 0), 10);
    bus.io_write_word(ga_port(0x03, 0), 0x00FF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.write_byte(WINDOW_BASE + 19, 0xA5);
    assert_eq!(bus.read_byte(WINDOW_BASE + 19), 0xA5);

    let mut bus = setup_bus();

    // GATEST keeps the normal WBA1 aperture and uses the same all-plane flags
    // for rotated planar words, so reads must still honor PRS.
    bus.io_write_word(ga_port(0x01, 0), 0);
    bus.io_write_word(ga_port(0x02, 0), 0);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_byte(ga_port(0x0E, 0), 0x00);

    for plane in 0..8 {
        bus.io_write_word(ga_port(0x03, 0), 1 << plane);
        bus.write_word(WINDOW_BASE + 0x46, 1 << plane);
    }

    bus.io_write_word(ga_port(0x03, 0), 0x00FF);
    bus.io_write_word(ga_port(0x0D, 0), 3);
    bus.io_write_byte(ga_port(0x0E, 0), 0x02);
    bus.write_word(WINDOW_BASE + 0x46, 0xFFFF);
    bus.io_write_byte(ga_port(0x0E, 0), 0x00);

    for plane in 0..8 {
        bus.io_write_byte(ga_port(0x06, 0), plane);
        assert_eq!(bus.read_word(WINDOW_BASE + 0x46), 1 << (plane + 3));
    }
}

#[test]
fn prs_high_bit_selects_single_write_plane() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x01, 0), 0);
    bus.io_write_word(ga_port(0x02, 0), 0);
    bus.io_write_word(ga_port(0x03, 0), 0x0000);
    bus.io_write_byte(ga_port(0x06, 0), 0x82);
    bus.write_byte(WINDOW_BASE, 0x5A);

    bus.io_write_byte(ga_port(0x06, 0), 0);
    assert_eq!(bus.read_byte(WINDOW_BASE), 0x00);
    bus.io_write_byte(ga_port(0x06, 0), 2);
    assert_eq!(bus.read_byte(WINDOW_BASE), 0x5A);
}

#[test]
fn mod1_two_rotates_planar_words() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x01, 0), 0);
    bus.io_write_word(ga_port(0x02, 0), 0);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_byte(ga_port(0x0E, 0), 0x00);

    for plane in 0..8 {
        bus.io_write_word(ga_port(0x03, 0), 1 << plane);
        bus.write_word(WINDOW_BASE + 0x46, 1 << plane);
    }

    bus.io_write_word(ga_port(0x03, 0), 0x00FF);
    bus.io_write_word(ga_port(0x0D, 0), 3);
    bus.io_write_byte(ga_port(0x0E, 0), 0x02);
    bus.write_word(WINDOW_BASE + 0x46, 0xFFFF);
    bus.io_write_byte(ga_port(0x0E, 0), 0x00);
    bus.io_write_word(ga_port(0x03, 0), 0x0001);

    for plane in 0..8 {
        bus.io_write_byte(ga_port(0x06, 0), plane);
        assert_eq!(bus.read_word(WINDOW_BASE + 0x46), 1 << (plane + 3));
    }
}

#[test]
fn raw_host_writes_respect_plane_and_bit_masks() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x01, 0), 3);
    bus.io_write_word(ga_port(0x02, 0), 3);
    bus.io_write_word(ga_port(0x03, 0), 0x0005);
    bus.io_write_word(ga_port(0x05, 0), 0x00F0);
    bus.io_write_byte(ga_port(0x0E, 0), 0);
    bus.write_byte(WINDOW_BASE + 2, 0xFF);

    bus.io_write_byte(ga_port(0x07, 0), 0xFF);
    bus.io_write_byte(ga_port(0x06, 0), 0);
    assert_eq!(bus.read_byte(WINDOW_BASE + 2), 0xF0);
    bus.io_write_byte(ga_port(0x06, 0), 2);
    assert_eq!(bus.read_byte(WINDOW_BASE + 2), 0xF0);
    bus.io_write_byte(ga_port(0x06, 0), 1);
    assert_eq!(bus.read_byte(WINDOW_BASE + 2), 0x00);
}

#[test]
fn raw_host_writes_wrap_high_raster_bits() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x12, 2), 0x03FF);
    bus.io_write_word(ga_port(0x13, 2), 0x03FF);
    bus.io_write_word(ga_port(0x01, 0), 0xE0C0);
    bus.io_write_word(ga_port(0x02, 0), 0xC0C0);
    bus.io_write_word(ga_port(0x03, 0), 0x0001);
    bus.io_write_word(ga_port(0x05, 0), 0x00FF);
    bus.io_write_byte(ga_port(0x0E, 0), 0);
    bus.write_byte(WINDOW_BASE + 4, 0xAA);

    bus.io_write_byte(ga_port(0x06, 0), 0);
    assert_eq!(bus.read_byte(WINDOW_BASE + 4), 0xAA);
}

#[test]
fn mod1_pixel_writes_compose_indexed_palette_pixels() {
    let mut bus = setup_bus();

    write_palette(&mut bus, 5, 0x11, 0x22, 0x33);
    bus.io_write_word(ga_port(0x01, 0), 10);
    bus.io_write_word(ga_port(0x03, 0), 0x00FF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x09, 0), 5);
    bus.io_write_byte(ga_port(0x0E, 0), 1);
    bus.write_byte(WINDOW_BASE, 0x80);

    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 10, [0x11, 0x22, 0x33, 0xFF]);
    assert_pixel(&bus, 1, 10, [0x00, 0x00, 0x00, 0xFF]);
}

#[test]
fn mod1_color_expand_writes_text_foreground_and_background() {
    let mut bus = setup_bus();

    write_palette(&mut bus, 0, 0x00, 0x00, 0x00);
    write_palette(&mut bus, 4, 0x80, 0x80, 0x80);
    write_palette(&mut bus, 0xFF, 0xFF, 0xFF, 0xFF);

    set_normal_mix(&mut bus);
    fill_rect(&mut bus, 0, 10, 8, 2, 4);

    bus.io_write_word(ga_port(0x01, 0), 10);
    bus.io_write_word(ga_port(0x03, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_byte(ga_port(0x0E, 0), 4);

    bus.io_write_word(ga_port(0x10, 0), 0x0000);
    bus.io_write_word(ga_port(0x12, 0), 0x00FF);
    bus.io_write_byte(ga_port(0x14, 0), 0x0C);
    bus.io_write_byte(ga_port(0x14, 1), 0x0C);
    bus.io_write_word(ga_port(0x1F, 2), 0x0AC8);
    bus.write_byte(WINDOW_BASE, 0b1010_0000);

    bus.io_write_word(ga_port(0x01, 0), 11);
    bus.io_write_word(ga_port(0x10, 0), 0x00FF);
    bus.io_write_word(ga_port(0x12, 0), 0x0004);
    bus.io_write_word(ga_port(0x1F, 2), 0x0AC8);
    bus.write_byte(WINDOW_BASE, 0b0100_0000);

    bus.ga1280a_present_now();
    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.unknown_command_warning_count, 0);
    assert_pixel(&bus, 0, 10, [0x00, 0x00, 0x00, 0xFF]);
    assert_pixel(&bus, 1, 10, [0xFF, 0xFF, 0xFF, 0xFF]);
    assert_pixel(&bus, 2, 10, [0x00, 0x00, 0x00, 0xFF]);
    assert_pixel(&bus, 0, 11, [0x80, 0x80, 0x80, 0xFF]);
    assert_pixel(&bus, 1, 11, [0xFF, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn source_solid_rectangle_fills_selected_text_background() {
    let mut bus = setup_bus();

    write_palette(&mut bus, 4, 0x00, 0x00, 0x80);
    write_palette(&mut bus, 7, 0xFF, 0xFF, 0xFF);

    set_normal_mix(&mut bus);
    fill_rect(&mut bus, 0, 10, 4, 1, 7);

    bus.io_write_word(ga_port(0x09, 0), 4);
    bus.io_write_word(ga_port(0x0A, 2), 0);
    bus.io_write_word(ga_port(0x0B, 2), 10);
    bus.io_write_word(ga_port(0x04, 2), 3);
    bus.io_write_word(ga_port(0x05, 2), 0);
    bus.io_write_byte(ga_port(0x0E, 0), 4);
    bus.io_write_word(ga_port(0x1F, 2), 0x4FE8);

    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 10, [0x00, 0x00, 0x80, 0xFF]);
    assert_pixel(&bus, 3, 10, [0x00, 0x00, 0x80, 0xFF]);
    assert_pixel(&bus, 4, 10, [0x00, 0x00, 0x00, 0xFF]);
}

#[test]
fn mod1_pixel_reads_aggregate_enabled_planes() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x01, 0), 10);
    bus.io_write_word(ga_port(0x02, 0), 10);
    bus.io_write_word(ga_port(0x03, 0), 0x00FF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x09, 0), 6);
    bus.io_write_byte(ga_port(0x0E, 0), 1);
    bus.write_byte(WINDOW_BASE, 0xC0);

    bus.io_write_byte(ga_port(0x07, 0), 0xFF);
    assert_eq!(bus.read_byte(WINDOW_BASE), 0xC0);

    bus.io_write_byte(ga_port(0x07, 0), 0x01);
    assert_eq!(bus.read_byte(WINDOW_BASE), 0x00);

    bus.io_write_byte(ga_port(0x07, 0), 0x02);
    assert_eq!(bus.read_byte(WINDOW_BASE), 0xC0);
}

#[test]
fn mod1_pixel_writes_wrap_high_raster_bits_and_clear() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x12, 2), 0x03FF);
    bus.io_write_word(ga_port(0x13, 2), 0x03FF);
    write_palette(&mut bus, 5, 0x11, 0x22, 0x33);
    bus.io_write_word(ga_port(0x03, 0), 0x00FF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_byte(ga_port(0x0E, 0), 1);

    bus.io_write_word(ga_port(0x01, 0), 0xE0C0);
    bus.io_write_word(ga_port(0x09, 0), 5);
    bus.write_byte(WINDOW_BASE, 0x80);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 0xC0, [0x11, 0x22, 0x33, 0xFF]);

    bus.io_write_word(ga_port(0x01, 0), 0xC0C0);
    bus.io_write_word(ga_port(0x09, 0), 0);
    bus.write_byte(WINDOW_BASE, 0x80);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 0xC0, [0x00, 0x00, 0x00, 0xFF]);
}

#[test]
fn ga1280_16_plane_mod1_writes_compose_rgb565_pixels() {
    let mut bus = setup_bus();

    set_ga1280_plane_mode(&mut bus, Ga1280aPlaneMode::DirectColor16);
    assert_eq!(
        bus.ga1280a_state().expect("GA board installed").plane_mode,
        Ga1280aPlaneMode::DirectColor16
    );

    bus.io_write_word(ga_port(0x01, 0), 5);
    bus.io_write_word(ga_port(0x03, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x09, 0), 0xF800);
    bus.io_write_byte(ga_port(0x0E, 0), 1);
    bus.write_byte(WINDOW_BASE, 0x80);

    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 5, [0xFF, 0x00, 0x00, 0xFF]);
    assert_pixel(&bus, 1, 5, [0x00, 0x00, 0x00, 0xFF]);

    set_ga1280_plane_mode(&mut bus, Ga1280aPlaneMode::Indexed8);
    assert_eq!(
        bus.ga1280a_state().expect("GA board installed").plane_mode,
        Ga1280aPlaneMode::Indexed8
    );
}

#[test]
fn ga1280_normal_dac_mask_write_does_not_leave_direct_color16() {
    let mut bus = setup_bus();

    set_ga1280_plane_mode(&mut bus, Ga1280aPlaneMode::DirectColor16);
    bus.io_write_byte(ga_port(0x1B, 0), 0xFF);

    assert_eq!(
        bus.ga1280a_state().expect("GA board installed").plane_mode,
        Ga1280aPlaneMode::DirectColor16
    );

    bus.io_write_word(ga_port(0x01, 0), 5);
    bus.io_write_word(ga_port(0x03, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x09, 0), 0xF800);
    bus.io_write_byte(ga_port(0x0E, 0), 1);
    bus.write_byte(WINDOW_BASE, 0x80);

    bus.ga1280a_present_now();
    assert_pixel(&bus, 0, 5, [0xFF, 0x00, 0x00, 0xFF]);
}

#[test]
fn ga1280_vdac_extended_indices_select_plane_mode() {
    let mut bus = setup_bus();

    bus.io_write_byte(ga_port(0x18, 1), 2);
    bus.io_write_byte(ga_port(0x18, 0), 0x38);
    assert_eq!(
        bus.ga1280a_state().expect("GA board installed").plane_mode,
        Ga1280aPlaneMode::DirectColor16
    );

    bus.io_write_byte(ga_port(0x18, 0), 0x48);
    assert_eq!(
        bus.ga1280a_state().expect("GA board installed").plane_mode,
        Ga1280aPlaneMode::Indexed8
    );
}

#[test]
fn ga1280_indexed_mode_decodes_half_width_crtc_registers() {
    let mut bus = setup_bus();

    write_crtc_word(&mut bus, 0x02, 0x0027);
    write_crtc_word(&mut bus, 0x12, 0x018F);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!((state.active_width, state.active_height), (640, 400));
}

#[test]
fn ga1280_1024_wide_second_megabyte_is_distinct_and_displayable() {
    let mut bus = setup_bus();

    write_crtc_word(&mut bus, 0x02, 0x007F);
    write_crtc_word(&mut bus, 0x12, 0x02FF);
    bus.io_write_word(ga_port(0x12, 2), 0x03FF);
    bus.io_write_word(ga_port(0x13, 2), 0x07FF);
    write_palette(&mut bus, 1, 0x00, 0x66, 0x00);
    write_palette(&mut bus, 2, 0xCC, 0x00, 0x00);

    bus.io_write_word(ga_port(0x03, 0), 0x0001);
    bus.io_write_word(ga_port(0x01, 0), 0);
    bus.write_byte(WINDOW_BASE, 0x08);

    bus.io_write_word(ga_port(0x01, 0), 1024);
    bus.io_write_word(ga_port(0x03, 0), 0x0001);
    bus.write_byte(WINDOW_BASE, 0x00);
    bus.io_write_word(ga_port(0x03, 0), 0x0002);
    bus.write_byte(WINDOW_BASE, 0x08);

    set_display_start(&mut bus, 0);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 4, 0, [0x00, 0x66, 0x00, 0xFF]);

    set_display_start(&mut bus, 1024 * 1024 / 4);
    bus.ga1280a_present_now();
    assert_pixel(&bus, 4, 0, [0xCC, 0x00, 0x00, 0xFF]);
}

#[test]
fn mod2_reports_ga_monitor_routing() {
    let mut bus = setup_bus();

    assert!(!bus.ga1280a_is_driving_monitor());
    bus.io_write_byte(ga_port(0x0E, 1), 0x40);
    assert!(!bus.ga1280a_is_driving_monitor());
    bus.io_write_byte(ga_port(0x0E, 1), 0xC0);
    assert!(bus.ga1280a_is_driving_monitor());
}
