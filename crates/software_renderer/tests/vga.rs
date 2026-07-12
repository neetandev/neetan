//! End-to-end check of the VGA pipeline: program the device through its I/O
//! ports like the VGA BIOS would, write text through the CPU memory window,
//! resolve a frame and render it.

use device::vga::{
    ResolvedVgaFrame, RetraceStatus, VGA_PORT_ATC_WRITE, VGA_PORT_CRTC_DATA_COLOR,
    VGA_PORT_CRTC_INDEX_COLOR, VGA_PORT_DAC_DATA, VGA_PORT_DAC_WRITE_INDEX, VGA_PORT_GC_DATA,
    VGA_PORT_GC_INDEX, VGA_PORT_SEQ_DATA, VGA_PORT_SEQ_INDEX, VGA_PORT_STATUS_COLOR,
    VGA_PORT_STATUS0_MISC_WRITE, Vga, VgaRenderMode as DeviceVgaRenderMode,
};
use software_renderer::{RenderInputsVga, VgaRenderMode, VgaRenderer};

/// Copies the resolved device frame into renderer inputs over the live VRAM.
fn render_inputs<'a>(resolved: &ResolvedVgaFrame, vram: &'a [u8]) -> RenderInputsVga<'a> {
    RenderInputsVga {
        vram,
        render_mode: match resolved.render_mode {
            DeviceVgaRenderMode::Text => VgaRenderMode::Text,
            DeviceVgaRenderMode::Planar16 => VgaRenderMode::Planar16,
            DeviceVgaRenderMode::Packed256 => VgaRenderMode::Packed256,
            DeviceVgaRenderMode::CgaInterleaved => VgaRenderMode::CgaInterleaved,
            DeviceVgaRenderMode::Mono1bpp => VgaRenderMode::Mono1bpp,
        },
        blanked: resolved.blanked,
        columns: resolved.columns,
        character_width: resolved.character_width,
        character_height: resolved.character_height,
        scan_doubled: resolved.scan_doubled,
        active_scanlines: resolved.active_scanlines,
        start_address: resolved.start_address,
        row_pitch: resolved.row_pitch,
        address_step: resolved.address_step,
        plane_address_mask: resolved.plane_address_mask,
        map13_from_row_scan: resolved.map13_from_row_scan,
        map14_from_row_scan: resolved.map14_from_row_scan,
        line_compare: resolved.line_compare,
        pel_pan_reset_on_split: resolved.pel_pan_reset_on_split,
        preset_row_scan: resolved.preset_row_scan,
        cursor_address: resolved.cursor_address,
        cursor_start_row: resolved.cursor_start_row,
        cursor_end_row: resolved.cursor_end_row,
        cursor_visible: resolved.cursor_visible,
        blink_enabled: resolved.blink_enabled,
        blink_visible: resolved.blink_visible,
        line_graphics: resolved.line_graphics,
        font_offset_map_a: resolved.font_offset_map_a,
        font_offset_map_b: resolved.font_offset_map_b,
        pel_pan: resolved.pel_pan,
        packed_half_rate: resolved.packed_half_rate,
        border_color: resolved.border_color,
        pens: resolved.pens,
        pens_256: resolved.pens_256,
    }
}

fn write_seq(vga: &mut Vga, index: u8, value: u8) {
    vga.io_write(VGA_PORT_SEQ_INDEX, index);
    vga.io_write(VGA_PORT_SEQ_DATA, value);
}

fn write_gc(vga: &mut Vga, index: u8, value: u8) {
    vga.io_write(VGA_PORT_GC_INDEX, index);
    vga.io_write(VGA_PORT_GC_DATA, value);
}

fn write_crtc(vga: &mut Vga, index: u8, value: u8) {
    vga.io_write(VGA_PORT_CRTC_INDEX_COLOR, index);
    vga.io_write(VGA_PORT_CRTC_DATA_COLOR, value);
}

fn write_atc(vga: &mut Vga, index: u8, value: u8) {
    vga.io_read(VGA_PORT_STATUS_COLOR, RetraceStatus::default());
    vga.io_write(VGA_PORT_ATC_WRITE, index);
    vga.io_write(VGA_PORT_ATC_WRITE, value);
}

/// Programs the register subset of the standard 80x25 16-color text mode 3.
fn program_text_mode_3(vga: &mut Vga) {
    vga.io_write(VGA_PORT_STATUS0_MISC_WRITE, 0x67);
    write_seq(vga, 0x00, 0x03);
    write_seq(vga, 0x01, 0x00);
    write_seq(vga, 0x02, 0x03);
    write_seq(vga, 0x03, 0x00);
    write_seq(vga, 0x04, 0x02);
    for (index, value) in [
        (0x00u8, 0x5Fu8),
        (0x01, 0x4F),
        (0x06, 0xBF),
        (0x07, 0x1F),
        (0x09, 0x4F),
        (0x0A, 0x0D),
        (0x0B, 0x0E),
        (0x0C, 0x00),
        (0x0D, 0x00),
        (0x10, 0x9C),
        (0x11, 0x0E),
        (0x12, 0x8F),
        (0x13, 0x28),
        (0x17, 0xA3),
    ] {
        write_crtc(vga, index, value);
    }
    write_gc(vga, 0x05, 0x10);
    write_gc(vga, 0x06, 0x0E);
    write_gc(vga, 0x08, 0xFF);
    for index in 0..16u8 {
        write_atc(vga, index, index);
    }
    write_atc(vga, 0x10, 0x0C);
    write_atc(vga, 0x12, 0x0F);
    write_atc(vga, 0x13, 0x08);
    // DAC entry 7: light gray.
    vga.io_write(VGA_PORT_DAC_WRITE_INDEX, 0x07);
    vga.io_write(VGA_PORT_DAC_DATA, 0x2A);
    vga.io_write(VGA_PORT_DAC_DATA, 0x2A);
    vga.io_write(VGA_PORT_DAC_DATA, 0x2A);
    // Re-enable the display (palette address source).
    vga.io_read(VGA_PORT_STATUS_COLOR, RetraceStatus::default());
    vga.io_write(VGA_PORT_ATC_WRITE, 0x20);
}

/// Uploads one glyph to plane 2 through the font load register path.
fn upload_glyph(vga: &mut Vga, character: u8, rows: &[u8]) {
    write_seq(vga, 0x02, 0x04);
    write_seq(vga, 0x04, 0x06);
    write_gc(vga, 0x05, 0x00);
    write_gc(vga, 0x06, 0x04);
    for (row, bits) in rows.iter().enumerate() {
        vga.mem_write(u32::from(character) * 32 + row as u32, *bits);
    }
    // Restore the text mode memory path.
    write_seq(vga, 0x02, 0x03);
    write_seq(vga, 0x04, 0x02);
    write_gc(vga, 0x05, 0x10);
    write_gc(vga, 0x06, 0x0E);
}

#[test]
fn text_mode_pipeline_renders_a_character() {
    let mut vga = Vga::new();
    program_text_mode_3(&mut vga);
    upload_glyph(
        &mut vga,
        b'H',
        &[0x00, 0x81, 0x81, 0xFF, 0x81, 0x81, 0x00, 0x00],
    );

    // Write "H" with attribute 0x07 at the top-left cell through 0xB8000.
    vga.mem_write(0x18000, b'H');
    vga.mem_write(0x18001, 0x07);

    // Advance past the cursor blink phase so the frame is deterministic.
    for _ in 0..16 {
        vga.on_vsync_start();
    }

    let resolved = vga.resolve();
    assert_eq!(resolved.render_mode, DeviceVgaRenderMode::Text);
    assert!(!resolved.blanked);
    assert_eq!(resolved.columns, 80);
    assert_eq!(resolved.character_width, 9);
    assert_eq!(resolved.character_height, 16);
    assert_eq!(resolved.active_scanlines, 400);

    let mut renderer = VgaRenderer::new();
    let inputs = render_inputs(&resolved, vga.vram());
    let (width, height) = renderer.render(&inputs);
    assert_eq!((width, height), (720, 400));

    let pixel = |x: u32, y: u32| {
        let offset = ((y * width + x) as usize) * 4;
        u32::from_le_bytes(
            renderer.framebuffer()[offset..offset + 4]
                .try_into()
                .unwrap(),
        )
    };
    let light_gray = u32::from_le_bytes([0xAA, 0xAA, 0xAA, 0xFF]);
    // Glyph row 3 is fully lit across the eight font dots.
    for x in 0..8 {
        assert_eq!(pixel(x, 3), light_gray);
    }
    // Row 0 is empty and the neighboring cell is background.
    assert_eq!(pixel(1, 0), 0xFF00_0000);
    assert_eq!(pixel(9, 3), 0xFF00_0000);
}
