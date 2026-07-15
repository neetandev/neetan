use device::vga::{
    RetraceStatus, VGA_PORT_ATC_WRITE, VGA_PORT_CRTC_DATA_COLOR, VGA_PORT_CRTC_DATA_MONO,
    VGA_PORT_CRTC_INDEX_COLOR, VGA_PORT_CRTC_INDEX_MONO, VGA_PORT_DAC_DATA, VGA_PORT_DAC_MASK,
    VGA_PORT_DAC_READ_INDEX, VGA_PORT_DAC_WRITE_INDEX, VGA_PORT_GC_DATA, VGA_PORT_GC_INDEX,
    VGA_PORT_HERCULES_COMPAT, VGA_PORT_MODE_CONTROL_COLOR, VGA_PORT_SEGMENT_SELECT,
    VGA_PORT_SEQ_DATA, VGA_PORT_SEQ_INDEX, VGA_PORT_STATUS_COLOR, VGA_PORT_STATUS0_MISC_WRITE, Vga,
};

const RETRACE_IDLE: RetraceStatus = RetraceStatus {
    display_disabled: false,
    vertical_retrace: false,
};

/// Writes an indexed register through its index/data port pair.
fn write_indexed(vga: &mut Vga, index_port: u16, data_port: u16, index: u8, value: u8) {
    vga.io_write(index_port, index);
    vga.io_write(data_port, value);
}

fn write_seq(vga: &mut Vga, index: u8, value: u8) {
    write_indexed(vga, VGA_PORT_SEQ_INDEX, VGA_PORT_SEQ_DATA, index, value);
}

fn write_gc(vga: &mut Vga, index: u8, value: u8) {
    write_indexed(vga, VGA_PORT_GC_INDEX, VGA_PORT_GC_DATA, index, value);
}

fn write_crtc(vga: &mut Vga, index: u8, value: u8) {
    write_indexed(
        vga,
        VGA_PORT_CRTC_INDEX_COLOR,
        VGA_PORT_CRTC_DATA_COLOR,
        index,
        value,
    );
}

/// A device in color decode with the planar graphics pipeline neutral:
/// odd/even off, all planes writable, write mode 0, no rotate/ALU/set-reset,
/// bit mask fully open, 64 KiB map at 0xA0000.
fn planar_vga() -> Vga {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_STATUS0_MISC_WRITE, 0x01);
    write_seq(&mut vga, 0x02, 0x0F);
    write_seq(&mut vga, 0x04, 0x06);
    write_gc(&mut vga, 0x01, 0x00);
    write_gc(&mut vga, 0x03, 0x00);
    write_gc(&mut vga, 0x05, 0x00);
    write_gc(&mut vga, 0x06, 0x05);
    write_gc(&mut vga, 0x08, 0xFF);
    vga
}

/// Unlocks the ET4000 extended registers with the databook KEY sequence.
fn unlock_key(vga: &mut Vga) {
    vga.io_write(VGA_PORT_HERCULES_COMPAT, 0x03);
    vga.io_write(VGA_PORT_MODE_CONTROL_COLOR, 0xA0);
}

/// Reads one plane byte directly from the interleaved display memory.
fn plane_byte(vga: &Vga, plane_offset: u32, plane: usize) -> u8 {
    vga.vram[(plane_offset as usize) * 4 + plane]
}

#[test]
fn write_mode_0_map_mask_selects_planes() {
    let mut vga = planar_vga();
    write_seq(&mut vga, 0x02, 0x05);
    vga.mem_write(0x1234, 0xA5);
    assert_eq!(plane_byte(&vga, 0x1234, 0), 0xA5);
    assert_eq!(plane_byte(&vga, 0x1234, 1), 0x00);
    assert_eq!(plane_byte(&vga, 0x1234, 2), 0xA5);
    assert_eq!(plane_byte(&vga, 0x1234, 3), 0x00);
}

#[test]
fn write_mode_0_rotate() {
    let mut vga = planar_vga();
    write_gc(&mut vga, 0x03, 0x02);
    vga.mem_write(0x0000, 0b1000_0001);
    assert_eq!(plane_byte(&vga, 0x0000, 0), 0b0110_0000);
}

#[test]
fn write_mode_0_set_reset_and_bit_mask() {
    let mut vga = planar_vga();
    // Preload the latches with a known pattern.
    write_seq(&mut vga, 0x02, 0x0F);
    vga.mem_write(0x0100, 0x0F);
    vga.mem_read(0x0100);
    // Set/reset planes 0 and 1 (values 1 and 0), bit mask upper nibble.
    write_gc(&mut vga, 0x00, 0x01);
    write_gc(&mut vga, 0x01, 0x03);
    write_gc(&mut vga, 0x08, 0xF0);
    vga.mem_write(0x0100, 0x00);
    // Plane 0: set/reset expands to 0xFF, masked into the latch 0x0F.
    assert_eq!(plane_byte(&vga, 0x0100, 0), 0xFF);
    // Plane 1: set/reset expands to 0x00, masked into the latch 0x0F.
    assert_eq!(plane_byte(&vga, 0x0100, 1), 0x0F);
    // Plane 2: not set/reset enabled, the written byte 0x00 masked in.
    assert_eq!(plane_byte(&vga, 0x0100, 2), 0x0F);
}

#[test]
fn write_mode_0_logical_ops() {
    let mut vga = planar_vga();
    vga.mem_write(0x0200, 0b1100_1100);
    vga.mem_read(0x0200);
    // AND.
    write_gc(&mut vga, 0x03, 0x08);
    vga.mem_write(0x0200, 0b1010_1010);
    assert_eq!(plane_byte(&vga, 0x0200, 0), 0b1000_1000);
    vga.mem_read(0x0200);
    // OR.
    write_gc(&mut vga, 0x03, 0x10);
    vga.mem_write(0x0200, 0b0000_0001);
    assert_eq!(plane_byte(&vga, 0x0200, 0), 0b1000_1001);
    vga.mem_read(0x0200);
    // XOR.
    write_gc(&mut vga, 0x03, 0x18);
    vga.mem_write(0x0200, 0xFF);
    assert_eq!(plane_byte(&vga, 0x0200, 0), 0b0111_0110);
}

#[test]
fn write_mode_1_copies_latches() {
    let mut vga = planar_vga();
    vga.mem_write(0x0300, 0x12);
    write_seq(&mut vga, 0x02, 0x02);
    vga.mem_write(0x0300, 0x34);
    write_seq(&mut vga, 0x02, 0x0F);
    vga.mem_read(0x0300);
    write_gc(&mut vga, 0x05, 0x01);
    vga.mem_write(0x0400, 0x00);
    assert_eq!(plane_byte(&vga, 0x0400, 0), 0x12);
    assert_eq!(plane_byte(&vga, 0x0400, 1), 0x34);
}

#[test]
fn write_mode_2_expands_color() {
    let mut vga = planar_vga();
    write_gc(&mut vga, 0x05, 0x02);
    write_gc(&mut vga, 0x08, 0x3C);
    vga.mem_write(0x0500, 0x05);
    assert_eq!(plane_byte(&vga, 0x0500, 0), 0x3C);
    assert_eq!(plane_byte(&vga, 0x0500, 1), 0x00);
    assert_eq!(plane_byte(&vga, 0x0500, 2), 0x3C);
    assert_eq!(plane_byte(&vga, 0x0500, 3), 0x00);
}

#[test]
fn write_mode_3_masks_with_rotated_data() {
    let mut vga = planar_vga();
    write_gc(&mut vga, 0x05, 0x03);
    write_gc(&mut vga, 0x00, 0x0F);
    write_gc(&mut vga, 0x08, 0xFF);
    // Rotate right by 4: 0x0F becomes 0xF0, so only the upper nibble is set.
    write_gc(&mut vga, 0x03, 0x04);
    vga.mem_write(0x0600, 0x0F);
    assert_eq!(plane_byte(&vga, 0x0600, 0), 0xF0);
}

#[test]
fn read_mode_0_selects_plane() {
    let mut vga = planar_vga();
    for plane in 0..4u8 {
        write_seq(&mut vga, 0x02, 1 << plane);
        vga.mem_write(0x0700, 0x10 + plane);
    }
    write_seq(&mut vga, 0x02, 0x0F);
    for plane in 0..4u8 {
        write_gc(&mut vga, 0x04, plane);
        assert_eq!(vga.mem_read(0x0700), Some(0x10 + plane));
    }
}

#[test]
fn read_mode_1_color_compare() {
    let mut vga = planar_vga();
    // Pixel color 0x5 in the low nibble, color 0xA in the high nibble.
    write_seq(&mut vga, 0x02, 0x01);
    vga.mem_write(0x0800, 0x0F);
    write_seq(&mut vga, 0x02, 0x02);
    vga.mem_write(0x0800, 0xF0);
    write_seq(&mut vga, 0x02, 0x04);
    vga.mem_write(0x0800, 0x0F);
    write_seq(&mut vga, 0x02, 0x08);
    vga.mem_write(0x0800, 0xF0);
    write_seq(&mut vga, 0x02, 0x0F);
    write_gc(&mut vga, 0x05, 0x08);
    write_gc(&mut vga, 0x02, 0x05);
    write_gc(&mut vga, 0x07, 0x0F);
    assert_eq!(vga.mem_read(0x0800), Some(0x0F));
    // With no plane cared about, every pixel matches.
    write_gc(&mut vga, 0x07, 0x00);
    assert_eq!(vga.mem_read(0x0800), Some(0xFF));
}

#[test]
fn odd_even_text_path_lands_in_plane_pairs() {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_STATUS0_MISC_WRITE, 0x01);
    // Text mode memory setup: odd/even, map 3 at 0xB8000.
    write_seq(&mut vga, 0x02, 0x03);
    write_seq(&mut vga, 0x04, 0x02);
    write_gc(&mut vga, 0x05, 0x10);
    write_gc(&mut vga, 0x06, 0x0E);
    write_gc(&mut vga, 0x08, 0xFF);
    // Window offset for 0xB8000 within the 0xA0000 window is 0x18000.
    vga.mem_write(0x18000, b'A');
    vga.mem_write(0x18001, 0x07);
    assert_eq!(plane_byte(&vga, 0x0000, 0), b'A');
    assert_eq!(plane_byte(&vga, 0x0000, 1), 0x07);
    assert_eq!(vga.mem_read(0x18000), Some(b'A'));
    assert_eq!(vga.mem_read(0x18001), Some(0x07));
}

#[test]
fn bios_font_load_sequence_lands_in_plane_2() {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_STATUS0_MISC_WRITE, 0x01);
    // The INT 10h font upload path: plane 2 only, sequential access, map 1.
    write_seq(&mut vga, 0x02, 0x04);
    write_seq(&mut vga, 0x04, 0x06);
    write_gc(&mut vga, 0x05, 0x00);
    write_gc(&mut vga, 0x06, 0x04);
    write_gc(&mut vga, 0x08, 0xFF);
    vga.mem_write(0x0000, 0x7E);
    vga.mem_write(0x0001, 0x81);
    assert_eq!(plane_byte(&vga, 0x0000, 2), 0x7E);
    assert_eq!(plane_byte(&vga, 0x0001, 2), 0x81);
    assert_eq!(plane_byte(&vga, 0x0000, 0), 0x00);
}

#[test]
fn chain4_addresses_interleaved_memory_linearly() {
    let mut vga = planar_vga();
    write_seq(&mut vga, 0x04, 0x0E);
    for offset in 0..8u32 {
        vga.mem_write(offset, 0x40 + offset as u8);
    }
    for offset in 0..8u32 {
        assert_eq!(vga.vram[offset as usize], 0x40 + offset as u8);
        assert_eq!(vga.mem_read(offset), Some(0x40 + offset as u8));
    }
}

#[test]
fn et4000_banking_steers_independent_read_and_write_windows() {
    let mut vga = planar_vga();
    unlock_key(&mut vga);
    // Write bank 2, read bank 0.
    vga.io_write(VGA_PORT_SEGMENT_SELECT, 0x02);
    vga.mem_write(0x0000, 0x55);
    assert_eq!(plane_byte(&vga, 0x2_0000, 0), 0x55);
    assert_eq!(vga.mem_read(0x0000), Some(0x00));
    // Read bank 2 sees the written byte.
    vga.io_write(VGA_PORT_SEGMENT_SELECT, 0x22);
    assert_eq!(vga.mem_read(0x0000), Some(0x55));
}

#[test]
fn et4000_banking_disabled_by_vsconf1() {
    let mut vga = planar_vga();
    unlock_key(&mut vga);
    vga.io_write(VGA_PORT_SEGMENT_SELECT, 0x22);
    write_crtc(&mut vga, 0x36, 0x10);
    vga.mem_write(0x0000, 0x66);
    assert_eq!(plane_byte(&vga, 0x0000, 0), 0x66);
    assert_eq!(vga.mem_read(0x0000), Some(0x66));
}

#[test]
fn key_gates_extended_registers() {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_STATUS0_MISC_WRITE, 0x01);
    // Locked: segment select and extended CRTC writes are dropped.
    vga.io_write(VGA_PORT_SEGMENT_SELECT, 0x77);
    assert_eq!(vga.io_read(VGA_PORT_SEGMENT_SELECT, RETRACE_IDLE), None);
    write_crtc(&mut vga, 0x34, 0x02);
    assert_eq!(vga.crtc[0x34], 0x00);
    write_seq(&mut vga, 0x07, 0xFF);
    assert_eq!(vga.seq[7], 0x00);
    // The extended start address register is not KEY protected.
    write_crtc(&mut vga, 0x33, 0x03);
    assert_eq!(vga.crtc[0x33], 0x03);

    unlock_key(&mut vga);
    vga.io_write(VGA_PORT_SEGMENT_SELECT, 0x21);
    assert_eq!(
        vga.io_read(VGA_PORT_SEGMENT_SELECT, RETRACE_IDLE),
        Some(0x21)
    );
    write_crtc(&mut vga, 0x34, 0x02);
    assert_eq!(vga.crtc[0x34], 0x02);
    write_seq(&mut vga, 0x07, 0xAC);
    assert_eq!(vga.seq[7], 0xAC);
}

#[test]
fn key_requires_the_prefix_write_first() {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_STATUS0_MISC_WRITE, 0x01);
    // The completion write alone must not unlock.
    vga.io_write(VGA_PORT_MODE_CONTROL_COLOR, 0xA0);
    assert!(!vga.key_unlocked);
    // A different Hercules compatibility value disarms the prefix.
    vga.io_write(VGA_PORT_HERCULES_COMPAT, 0x03);
    vga.io_write(VGA_PORT_HERCULES_COMPAT, 0x01);
    vga.io_write(VGA_PORT_MODE_CONTROL_COLOR, 0xA0);
    assert!(!vga.key_unlocked);
    unlock_key(&mut vga);
    assert!(vga.key_unlocked);
}

#[test]
fn synchronous_reset_clears_the_key() {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_STATUS0_MISC_WRITE, 0x01);
    unlock_key(&mut vga);
    assert!(vga.key_unlocked);
    write_seq(&mut vga, 0x00, 0x01);
    assert!(!vga.key_unlocked);
    write_seq(&mut vga, 0x00, 0x03);
    assert!(!vga.key_unlocked);
}

#[test]
fn dac_write_and_read_cycles() {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_DAC_WRITE_INDEX, 0x10);
    vga.io_write(VGA_PORT_DAC_DATA, 0x3F);
    vga.io_write(VGA_PORT_DAC_DATA, 0x20);
    vga.io_write(VGA_PORT_DAC_DATA, 0x01);
    assert_eq!(vga.dac[0x10], [0x3F, 0x20, 0x01]);
    assert_eq!(vga.dac_write_index, 0x11);

    vga.io_write(VGA_PORT_DAC_READ_INDEX, 0x10);
    assert_eq!(
        vga.io_read(VGA_PORT_DAC_READ_INDEX, RETRACE_IDLE),
        Some(0x03)
    );
    assert_eq!(vga.io_read(VGA_PORT_DAC_DATA, RETRACE_IDLE), Some(0x3F));
    assert_eq!(vga.io_read(VGA_PORT_DAC_DATA, RETRACE_IDLE), Some(0x20));
    assert_eq!(vga.io_read(VGA_PORT_DAC_DATA, RETRACE_IDLE), Some(0x01));
    assert_eq!(vga.dac_read_index, 0x11);
}

#[test]
fn dac_write_index_wraps() {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_DAC_WRITE_INDEX, 0xFF);
    for _ in 0..3 {
        vga.io_write(VGA_PORT_DAC_DATA, 0x11);
    }
    assert_eq!(vga.dac[0xFF], [0x11, 0x11, 0x11]);
    assert_eq!(vga.dac_write_index, 0x00);
}

#[test]
fn hidden_dac_register_after_four_mask_reads() {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_DAC_MASK, 0xFF);
    for _ in 0..4 {
        assert_eq!(vga.io_read(VGA_PORT_DAC_MASK, RETRACE_IDLE), Some(0xFF));
    }
    // The fifth access reaches the hidden control register.
    vga.io_write(VGA_PORT_DAC_MASK, 0xA0);
    assert_eq!(vga.dac_hidden_control, 0xA0);
    assert_eq!(vga.dac_mask, 0xFF);
    for _ in 0..4 {
        vga.io_read(VGA_PORT_DAC_MASK, RETRACE_IDLE);
    }
    assert_eq!(vga.io_read(VGA_PORT_DAC_MASK, RETRACE_IDLE), Some(0xA0));
    // A write index read resets the sequence.
    for _ in 0..4 {
        vga.io_read(VGA_PORT_DAC_MASK, RETRACE_IDLE);
    }
    vga.io_read(VGA_PORT_DAC_WRITE_INDEX, RETRACE_IDLE);
    assert_eq!(vga.io_read(VGA_PORT_DAC_MASK, RETRACE_IDLE), Some(0xFF));
}

#[test]
fn atc_flip_flop_resets_on_status_read() {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_STATUS0_MISC_WRITE, 0x01);
    // Index write (mode control, palette address source set), then data.
    vga.io_write(VGA_PORT_ATC_WRITE, 0x30);
    vga.io_write(VGA_PORT_ATC_WRITE, 0x0C);
    assert_eq!(vga.atc[0x10], 0x0C);
    // Half a cycle in, a status read resets to the index phase.
    vga.io_write(VGA_PORT_ATC_WRITE, 0x31);
    vga.io_read(VGA_PORT_STATUS_COLOR, RETRACE_IDLE);
    vga.io_write(VGA_PORT_ATC_WRITE, 0x32);
    vga.io_write(VGA_PORT_ATC_WRITE, 0x15);
    assert_eq!(vga.atc[0x12], 0x15);
}

#[test]
fn atc_palette_writes_blocked_while_display_enabled() {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_STATUS0_MISC_WRITE, 0x01);
    // With the palette address source set, palette registers are locked.
    vga.io_write(VGA_PORT_ATC_WRITE, 0x25);
    vga.io_write(VGA_PORT_ATC_WRITE, 0x3F);
    assert_eq!(vga.atc[0x05], 0x00);
    // With it clear (screen blanked) the write lands.
    vga.io_read(VGA_PORT_STATUS_COLOR, RETRACE_IDLE);
    vga.io_write(VGA_PORT_ATC_WRITE, 0x05);
    vga.io_write(VGA_PORT_ATC_WRITE, 0x3F);
    assert_eq!(vga.atc[0x05], 0x3F);
    assert!(vga.resolve().blanked);
}

#[test]
fn mono_color_port_mirroring_follows_misc_output() {
    let mut vga = Vga::new();
    // Power-on: monochrome decode, the color ports are dead.
    vga.io_write(VGA_PORT_CRTC_INDEX_COLOR, 0x0C);
    assert_eq!(vga.io_read(VGA_PORT_CRTC_INDEX_COLOR, RETRACE_IDLE), None);
    vga.io_write(VGA_PORT_CRTC_INDEX_MONO, 0x0C);
    vga.io_write(VGA_PORT_CRTC_DATA_MONO, 0x12);
    assert_eq!(vga.crtc[0x0C], 0x12);
    // Select color decode: the mono ports go dead.
    vga.io_write(VGA_PORT_STATUS0_MISC_WRITE, 0x01);
    assert_eq!(vga.io_read(VGA_PORT_CRTC_DATA_MONO, RETRACE_IDLE), None);
    vga.io_write(VGA_PORT_CRTC_INDEX_COLOR, 0x0D);
    vga.io_write(VGA_PORT_CRTC_DATA_COLOR, 0x34);
    assert_eq!(vga.crtc[0x0D], 0x34);
}

#[test]
fn crtc_protection_blocks_low_registers_except_line_compare_bit() {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_STATUS0_MISC_WRITE, 0x01);
    write_crtc(&mut vga, 0x00, 0x5F);
    write_crtc(&mut vga, 0x07, 0x00);
    write_crtc(&mut vga, 0x11, 0x80);
    write_crtc(&mut vga, 0x00, 0x11);
    assert_eq!(vga.crtc[0x00], 0x5F);
    write_crtc(&mut vga, 0x07, 0xFF);
    assert_eq!(vga.crtc[0x07], 0x10);
    write_crtc(&mut vga, 0x11, 0x00);
    write_crtc(&mut vga, 0x00, 0x11);
    assert_eq!(vga.crtc[0x00], 0x11);
}

#[test]
fn crtc_protection_also_covers_overflow_high() {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_STATUS0_MISC_WRITE, 0x01);
    unlock_key(&mut vga);
    write_crtc(&mut vga, 0x11, 0x80);
    write_crtc(&mut vga, 0x35, 0x1F);
    assert_eq!(vga.crtc[0x35], 0x00);
    write_crtc(&mut vga, 0x11, 0x00);
    write_crtc(&mut vga, 0x35, 0x1F);
    assert_eq!(vga.crtc[0x35], 0x1F);
}

#[test]
fn input_status_1_reports_retrace() {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_STATUS0_MISC_WRITE, 0x01);
    let status = vga
        .io_read(
            VGA_PORT_STATUS_COLOR,
            RetraceStatus {
                display_disabled: true,
                vertical_retrace: true,
            },
        )
        .unwrap();
    assert_eq!(status & 0x09, 0x09);
    let status = vga.io_read(VGA_PORT_STATUS_COLOR, RETRACE_IDLE).unwrap();
    assert_eq!(status & 0x09, 0x00);
}

#[test]
fn frame_timing_falls_back_until_programmed() {
    let vga = Vga::new();
    let timing = vga.frame_timing();
    assert_eq!(timing.active_dots, 720);
    assert_eq!(timing.active_scanlines, 400);
    // Roughly 70 Hz at a 25 MHz CPU clock.
    let frame_cycles = timing.frame_cycles(25_000_000);
    assert!((350_000..360_000).contains(&frame_cycles));
}

#[test]
fn frame_timing_follows_the_crtc() {
    let mut vga = Vga::new();
    vga.io_write(VGA_PORT_STATUS0_MISC_WRITE, 0x01);
    // Standard mode 12h vertical timing: 640x480 at 60 Hz, 8-dot cells.
    write_seq(&mut vga, 0x01, 0x01);
    write_crtc(&mut vga, 0x00, 0x5F);
    write_crtc(&mut vga, 0x01, 0x4F);
    write_crtc(&mut vga, 0x06, 0x0B);
    write_crtc(&mut vga, 0x07, 0x3E);
    write_crtc(&mut vga, 0x10, 0xEA);
    write_crtc(&mut vga, 0x11, 0x0C);
    write_crtc(&mut vga, 0x12, 0xDF);
    let timing = vga.frame_timing();
    assert_eq!(timing.active_dots, 640);
    assert_eq!(timing.active_scanlines, 480);
    assert_eq!(timing.total_scanlines, 525);
    assert_eq!(timing.dots_per_scanline, 800);
    // 25.175 MHz gives 59.9 Hz; TS7 selects MCLK/2 on the doubled clock.
    unlock_key(&mut vga);
    write_seq(&mut vga, 0x07, 0x40);
    let timing = vga.frame_timing();
    assert_eq!(timing.dot_clock_hz, 25_175_000);
    let status = timing.retrace_status(0, 25_000_000);
    assert!(status.vertical_retrace);
    assert!(status.display_disabled);
}

#[test]
fn encoded_state_round_trips_registers_latches_palette_and_vram() {
    let mut vga = Vga::new();
    write_seq(&mut vga, 0x02, 0x0F);
    write_gc(&mut vga, 0x08, 0xA5);
    vga.io_write(VGA_PORT_DAC_WRITE_INDEX, 7);
    vga.io_write(VGA_PORT_DAC_DATA, 1);
    vga.io_write(VGA_PORT_DAC_DATA, 2);
    vga.io_write(VGA_PORT_DAC_DATA, 3);
    vga.mem_write(0xA0000, 0x5A);
    let expected = vga.capture_state();
    let encoded = save_state::encode_runtime_state(&expected);
    let decoded = save_state::decode_runtime_state::<Vga>(&encoded, 1 << 20).unwrap();

    write_seq(&mut vga, 0x02, 0);
    vga.mem_write(0xA0000, 0);
    vga.restore_state(decoded).unwrap();
    assert!(vga == expected);
}
