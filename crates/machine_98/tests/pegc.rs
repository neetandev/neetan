use common::{Bus, CpuMode, MachineModel, NoTrace};
use machine_98::Pc9801Bus;

const PEGC_VRAM_A: u32 = 0xA8000;
const PEGC_MMIO_BANK_A8: u32 = 0xE0004;
const BANK_SIZE: u32 = 0x8000;
const GRID_ROWS: u32 = 16;
const GRID_COLS: u32 = 16;
const CELL_WIDTH: u32 = 40;
const LINES_PER_ROW_400: u32 = 25;

fn create_pegc_bus() -> Pc9801Bus<NoTrace> {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::High, 48000);
    bus.io_write_byte(0x6A, 0x01); // analog mode (mode2 bit 0)
    bus.io_write_byte(0x6A, 0x07); // mode change permission (mode2 bit 3)
    bus.set_graphics_extension_enabled(true);
    bus
}

fn enable_pegc_packed_pixel(bus: &mut Pc9801Bus<NoTrace>) {
    bus.io_write_byte(0x6A, 0x21); // PEGC 256-color enable
    bus.io_write_byte(0x6A, 0x68); // two-screen mode (640x400)
}

fn hsv_to_grb(i: u8) -> (u8, u8, u8) {
    let h6 = i as u32 * 6;
    let sector = h6 / 256;
    let f = (h6 % 256) as u8;
    let t = f;
    let q = 255 - f;
    match sector {
        0 => (t, 255, 0),
        1 => (255, q, 0),
        2 => (255, 0, t),
        3 => (q, 0, 255),
        4 => (0, t, 255),
        5 => (0, 255, q),
        _ => unreachable!(),
    }
}

fn program_hsv_palette(bus: &mut Pc9801Bus<NoTrace>) {
    for i in 0..=255u8 {
        let (green, red, blue) = hsv_to_grb(i);
        bus.io_write_byte(0xA8, i);
        bus.io_write_byte(0xAA, green);
        bus.io_write_byte(0xAC, red);
        bus.io_write_byte(0xAE, blue);
    }
}

fn set_bank_a8(bus: &mut Pc9801Bus<NoTrace>, bank: u8) {
    bus.write_byte(PEGC_MMIO_BANK_A8, bank);
}

fn fill_pegc_grid(bus: &mut Pc9801Bus<NoTrace>) {
    let mut current_bank: u8 = 0;
    let mut bank_offset: u32 = 0;
    set_bank_a8(bus, 0);

    for row in 0..GRID_ROWS {
        for _line in 0..LINES_PER_ROW_400 {
            for col in 0..GRID_COLS {
                let palette_index = (row * 16 + col) as u8;
                for _pixel in 0..CELL_WIDTH {
                    if bank_offset >= BANK_SIZE {
                        current_bank += 1;
                        set_bank_a8(bus, current_bank);
                        bank_offset = 0;
                    }
                    bus.write_byte(PEGC_VRAM_A + bank_offset, palette_index);
                    bank_offset += 1;
                }
            }
        }
    }
}

#[test]
fn pegc_hsv_palette_all_distinct() {
    let mut colors = std::collections::HashSet::new();
    for i in 0..=255u8 {
        let grb = hsv_to_grb(i);
        assert!(
            colors.insert(grb),
            "palette index {i} produces duplicate color ({}, {}, {})",
            grb.0,
            grb.1,
            grb.2
        );
    }
}

#[test]
fn pegc_hsv_palette_boundary_values() {
    assert_eq!(hsv_to_grb(0), (0, 255, 0)); // pure red
    assert_eq!(hsv_to_grb(43), (255, 253, 0)); // near yellow (sector 1 start)
    assert_eq!(hsv_to_grb(86), (255, 0, 4)); // near green (sector 2 start)
    assert_eq!(hsv_to_grb(128), (255, 0, 255)); // cyan
    assert_eq!(hsv_to_grb(171), (0, 2, 255)); // near blue (sector 4 start)
    assert_eq!(hsv_to_grb(214), (0, 255, 251)); // near magenta (sector 5 start)
    assert_eq!(hsv_to_grb(255), (0, 255, 5)); // near red (wrapping)
}

#[test]
fn pegc_palette_program_and_readback() {
    let mut bus = create_pegc_bus();
    enable_pegc_packed_pixel(&mut bus);
    program_hsv_palette(&mut bus);

    for i in 0..=255u8 {
        let (expected_green, expected_red, expected_blue) = hsv_to_grb(i);

        bus.io_write_byte(0xA8, i);
        let green = bus.io_read_byte(0xAA);
        let red = bus.io_read_byte(0xAC);
        let blue = bus.io_read_byte(0xAE);

        assert_eq!(
            green, expected_green,
            "palette[{i}] green: expected {expected_green}, got {green}"
        );
        assert_eq!(
            red, expected_red,
            "palette[{i}] red: expected {expected_red}, got {red}"
        );
        assert_eq!(
            blue, expected_blue,
            "palette[{i}] blue: expected {expected_blue}, got {blue}"
        );
    }
}

#[test]
fn pegc_packed_pixel_grid_all_pixels() {
    let mut bus = create_pegc_bus();
    enable_pegc_packed_pixel(&mut bus);
    program_hsv_palette(&mut bus);
    fill_pegc_grid(&mut bus);

    let mut current_bank: u8 = 0;
    let mut bank_offset: u32 = 0;
    set_bank_a8(&mut bus, 0);

    for row in 0..GRID_ROWS {
        for line in 0..LINES_PER_ROW_400 {
            for col in 0..GRID_COLS {
                let expected_index = (row * 16 + col) as u8;
                for pixel in 0..CELL_WIDTH {
                    if bank_offset >= BANK_SIZE {
                        current_bank += 1;
                        set_bank_a8(&mut bus, current_bank);
                        bank_offset = 0;
                    }
                    let actual = bus.read_byte(PEGC_VRAM_A + bank_offset);
                    assert_eq!(
                        actual, expected_index,
                        "mismatch at row={row}, line={line}, col={col}, pixel={pixel}: \
                         expected {expected_index}, got {actual}"
                    );
                    bank_offset += 1;
                }
            }
        }
    }
}

#[test]
fn pegc_bank_switching_across_boundary() {
    let mut bus = create_pegc_bus();
    enable_pegc_packed_pixel(&mut bus);

    set_bank_a8(&mut bus, 0);
    bus.write_byte(PEGC_VRAM_A + BANK_SIZE - 1, 0xAA);

    set_bank_a8(&mut bus, 1);
    bus.write_byte(PEGC_VRAM_A, 0xBB);

    set_bank_a8(&mut bus, 0);
    assert_eq!(bus.read_byte(PEGC_VRAM_A + BANK_SIZE - 1), 0xAA);

    set_bank_a8(&mut bus, 1);
    assert_eq!(bus.read_byte(PEGC_VRAM_A), 0xBB);
}

const PEGC_LINEAR_FB_LOW: u32 = 0xF00000;
const PEGC_LINEAR_FB_HIGH: u32 = 0xFFF00000;

fn enable_pegc_linear_fb(bus: &mut Pc9801Bus<NoTrace>) {
    enable_pegc_packed_pixel(bus);
    bus.write_byte(0xE0102, 0x01);
    // The linear FB at F00000h sits above the 1 MB A20 line. Without A20 the
    // address is masked to E00000h and falls into DRAM.
    bus.io_write_byte(0xF2, 0);
}

#[test]
fn pegc_linear_fb_word_access_low_alias() {
    let mut bus = create_pegc_bus();
    enable_pegc_linear_fb(&mut bus);

    bus.write_word(PEGC_LINEAR_FB_LOW + 0x100, 0xBEEF);
    assert_eq!(bus.read_word(PEGC_LINEAR_FB_LOW + 0x100), 0xBEEF);
    assert_eq!(bus.read_byte(PEGC_LINEAR_FB_LOW + 0x100), 0xEF);
    assert_eq!(bus.read_byte(PEGC_LINEAR_FB_LOW + 0x101), 0xBE);
}

#[test]
fn pegc_linear_fb_dword_access_both_aliases() {
    let mut bus = create_pegc_bus();
    enable_pegc_linear_fb(&mut bus);

    bus.write_dword(PEGC_LINEAR_FB_LOW + 0x200, 0xDEAD_BEEF);
    // Both linear FB aliases see the same 512 KB buffer.
    assert_eq!(bus.read_dword(PEGC_LINEAR_FB_LOW + 0x200), 0xDEAD_BEEF);
    assert_eq!(bus.read_dword(PEGC_LINEAR_FB_HIGH + 0x200), 0xDEAD_BEEF);
}

#[test]
fn pegc_linear_fb_disabled_returns_all_ones() {
    let mut bus = create_pegc_bus();
    enable_pegc_packed_pixel(&mut bus);
    bus.io_write_byte(0xF2, 0); // open A20 so F00000h is reachable
    // E0102h bit 0 not set: linear FB disabled.
    assert_eq!(bus.read_word(PEGC_LINEAR_FB_LOW), 0xFFFF);
    assert_eq!(bus.read_dword(PEGC_LINEAR_FB_LOW), 0xFFFF_FFFF);
}

#[test]
fn pegc_linear_fb_not_shadowed_by_extended_ram() {
    let mut bus = create_pegc_bus();
    enable_pegc_linear_fb(&mut bus);

    // Extended RAM lies in 0x100000..0xF00000 on PC9821AS by default.
    // Write a distinctive value just below F00000 (last DRAM word) and at the
    // PEGC FB base. They must remain independent.
    bus.write_word(PEGC_LINEAR_FB_LOW - 2, 0x1234);
    bus.write_word(PEGC_LINEAR_FB_LOW, 0xABCD);
    assert_eq!(bus.read_word(PEGC_LINEAR_FB_LOW - 2), 0x1234);
    assert_eq!(bus.read_word(PEGC_LINEAR_FB_LOW), 0xABCD);
}

#[test]
fn pegc_port_09a8_readback_round_trips() {
    let mut bus = create_pegc_bus();
    assert_eq!(bus.io_read_byte(0x09A8), 0);
    bus.io_write_byte(0x09A8, 1);
    assert_eq!(bus.io_read_byte(0x09A8), 1);
    bus.io_write_byte(0x09A8, 0);
    assert_eq!(bus.io_read_byte(0x09A8), 0);
}

const BDA_GRPH_ARCH: u32 = 0x045C;
const BDA_GRPH_ARCH_PEGC_BIT: u8 = 0x40;

#[test]
fn pegc_machines_advertise_extended_graph_architecture() {
    for model in [MachineModel::PC9821AS, MachineModel::PC9821AP] {
        let mut bus = Pc9801Bus::<NoTrace>::new(model, CpuMode::High, 48000);
        let flags = bus.read_byte(BDA_GRPH_ARCH);
        assert_eq!(
            flags & BDA_GRPH_ARCH_PEGC_BIT,
            BDA_GRPH_ARCH_PEGC_BIT,
            "{model}: BDA 0x045C bit 6 (extended graph architecture) must be set",
        );
    }
}

#[test]
fn non_pegc_machines_do_not_advertise_extended_graph_architecture() {
    for model in [
        MachineModel::PC9801F,
        MachineModel::PC9801VM,
        MachineModel::PC9801VX,
        MachineModel::PC9801RA,
    ] {
        let mut bus = Pc9801Bus::<NoTrace>::new(model, CpuMode::High, 48000);
        let flags = bus.read_byte(BDA_GRPH_ARCH);
        assert_eq!(
            flags & BDA_GRPH_ARCH_PEGC_BIT,
            0,
            "{model}: BDA 0x045C bit 6 (extended graph architecture) must be clear",
        );
    }
}

#[test]
fn pegc_plane_dword_write_bus_dispatch() {
    let mut bus = create_pegc_bus();
    enable_pegc_packed_pixel(&mut bus);
    // Switch to plane mode via MMIO E0100h and enable the linear FB so we can
    // verify the 32 pixels written via the plane bank window are visible at
    // the same VRAM bytes through the linear F00000h alias.
    bus.write_word(0xE0100, 0x0001);
    bus.write_byte(0xE0102, 0x01);
    bus.io_write_byte(0xF2, 0); // open A20 to reach F00000h

    // ROP register: source = CPU data (bit 8 set).
    bus.write_word(0xE0108, 0x0100);
    bus.write_word(0xE010C, 0xFFFF); // write mask bits 0..15
    bus.write_word(0xE010E, 0xFFFF); // write mask bits 16..31
    bus.write_word(0xE0110, 0x0FFF); // block length max

    // A dword write at A8000 in plane mode should flip 32 pixels.
    bus.write_dword(0xA8000, 0xFFFF_FFFF);

    // Each of the 32 pixel bytes at the start of plane VRAM must now be 0xFF.
    for i in 0..32 {
        assert_eq!(
            bus.read_byte(PEGC_LINEAR_FB_LOW + i),
            0xFF,
            "pixel {i} of 32-pixel dword write",
        );
    }
}

#[test]
fn pegc_plane_dword_vram_source_word_block_copies_one_configured_block() {
    let mut bus = create_pegc_bus();
    enable_pegc_packed_pixel(&mut bus);
    bus.write_word(0xE0100, 0x0001);
    bus.write_byte(0xE0102, 0x01);
    bus.io_write_byte(0xF2, 0);

    bus.write_word(0xE0108, 0x10F0);
    bus.write_word(0xE010C, 0xFFFF);
    bus.write_word(0xE010E, 0x0000);
    bus.write_word(0xE0110, 0x000F);

    let source_address = 0xB7000;
    let destination_address = 0xA9000;
    let source_pixel_base = (source_address - PEGC_VRAM_A) * 8;
    let destination_pixel_base = (destination_address - PEGC_VRAM_A) * 8;

    for pixel in 0..32u32 {
        let value = if pixel < 16 {
            0x20 + pixel as u8
        } else {
            0x60 + (pixel - 16) as u8
        };
        bus.write_byte(PEGC_LINEAR_FB_LOW + source_pixel_base + pixel, value);
        bus.write_byte(PEGC_LINEAR_FB_LOW + destination_pixel_base + pixel, 0xE7);
    }

    let _ = bus.read_dword(source_address);
    bus.write_dword(destination_address, 0);

    for pixel in 0..16u32 {
        let expected = 0x20 + pixel as u8;
        let actual = bus.read_byte(PEGC_LINEAR_FB_LOW + destination_pixel_base + pixel);
        assert_eq!(
            actual, expected,
            "pixel {pixel} should be copied by the word-sized dword ROP"
        );
    }
    for pixel in 16..32u32 {
        let actual = bus.read_byte(PEGC_LINEAR_FB_LOW + destination_pixel_base + pixel);
        assert_eq!(
            actual, 0xE7,
            "pixel {pixel} should be outside the word-sized dword ROP"
        );
    }
}

/// Verifies that the renderer reads PEGC packed-pixel VRAM with a 640-byte
/// scanline stride when the GDC is in 2.5 MHz mode.
#[test]
fn pegc_packed_pixel_pitch_in_2_5mhz_mode_renders_correct_scanlines() {
    let mut bus = create_pegc_bus();
    enable_pegc_packed_pixel(&mut bus);

    // Two distinct palette colors for the two scanlines under test.
    bus.io_write_byte(0xA8, 0x11);
    bus.io_write_byte(0xAA, 0xFF); // green
    bus.io_write_byte(0xAC, 0x00); // red
    bus.io_write_byte(0xAE, 0x00); // blue

    bus.io_write_byte(0xA8, 0x22);
    bus.io_write_byte(0xAA, 0x00);
    bus.io_write_byte(0xAC, 0xFF);
    bus.io_write_byte(0xAE, 0x00);

    // Fill the first 1280 bytes of PEGC VRAM: bytes 0..640 with palette index
    // 0x11 (green), bytes 640..1280 with palette index 0x22 (red). At 1 byte
    // per pixel and 640 pixels per line this is exactly scanlines 0 and 1.
    set_bank_a8(&mut bus, 0);
    for offset in 0..640u32 {
        bus.write_byte(PEGC_VRAM_A + offset, 0x11);
    }
    for offset in 640..1280u32 {
        bus.write_byte(PEGC_VRAM_A + offset, 0x22);
    }

    // Enable display: global display on (mode1 bit 7) and slave-GDC SYNC with
    // DE=1 so the renderer treats graphics as enabled.
    bus.io_write_byte(0x68, 0x0F); // mode1 bit 7 = display on
    bus.io_write_byte(0xA2, 0x0F); // GDC SYNC command, DE bit set

    bus.render_display_frame();
    let framebuffer = bus.display_framebuffer();

    // Read the first pixel of scanlines 0 and 1.
    let pixel_at = |x: u32, y: u32| -> (u8, u8, u8) {
        let i = ((y * 640 + x) * 4) as usize;
        (framebuffer[i], framebuffer[i + 1], framebuffer[i + 2])
    };
    let scanline_0 = pixel_at(0, 0);
    let scanline_1 = pixel_at(0, 1);

    // Palette[0x11] = (G=0xFF, R=0x00, B=0x00) -> the renderer multiplies the
    // 4-bit components by 17, so 0xF maps to 255 in the corresponding channel.
    // Framebuffer pixels are packed `R, G, B, A`.
    assert_eq!(
        scanline_0,
        (0, 255, 0),
        "scanline 0 should render with palette index 0x11 (green)",
    );
    // Palette[0x22] = (G=0x00, R=0xFF, B=0x00). With the pitch bug, scanline 1
    // would skip past the 640..1280 block (reading byte 1280 instead) and the
    // expected red would not appear.
    assert_eq!(
        scanline_1,
        (255, 0, 0),
        "scanline 1 should render with palette index 0x22 (red); a doubled \
         pitch would skip past the second 640-byte block",
    );
}
