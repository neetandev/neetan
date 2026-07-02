//! Integration tests for the FM Towns video and sprite pipeline, driven through
//! the public bus I/O surface and the composed framebuffer.

#[path = "common/harness.rs"]
mod harness;

use common::{Bus, Machine};
use harness::{
    MX_CPU_CLOCK_HZ, VRAM_BASE, machine_mx, read_vram_word, write_crtc, write_sprite_reg,
};

/// The FMR-compatible palette-setup routine programs the eight digital palette
/// registers (0xFD98-0xFD9F) and reads them back to look up the analog color for
/// each entry. When the read path was missing the port returned 0xFF, so every
/// entry collapsed onto color 15.
#[test]
fn digital_palette_ports_round_trip_through_io() {
    let mut machine = machine_mx();
    let codes = [0x00u8, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
    for (index, &code) in codes.iter().enumerate() {
        machine.bus.io_write_byte(0xFD98 + index as u16, code);
    }
    for (index, &code) in codes.iter().enumerate() {
        assert_eq!(machine.bus.io_read_byte(0xFD98 + index as u16), code);
    }
}

/// Writing FMR character VRAM raises the TVRAM-dirty status at I/O 0x05C8
/// (bit 7), which the firmware polls to know when to repaint text. The read is
/// self-clearing.
#[test]
fn tvram_dirty_status_reports_and_clears_via_io() {
    let mut machine = machine_mx();
    assert_eq!(machine.bus.io_read_byte(0x05C8), 0x00);
    machine.bus.write_byte(0x000C_8000, 0x41);
    assert_eq!(machine.bus.io_read_byte(0x05C8), 0x80);
    assert_eq!(machine.bus.io_read_byte(0x05C8), 0x00);
}

/// Drives the whole sprite pipeline through the public bus: program a screen
/// mode that accepts sprites, load a sprite pattern and attribute, enable the
/// engine, then advance time so the scheduled VSYNC/finish events paint the
/// sprite into VRAM layer 1.
#[test]
fn sprite_pipeline_blits_into_vram_layer_1() {
    let mut machine = machine_mx();

    // Two-page mode with layer 1 as a 16 bpp, 512-byte-per-line page so the
    // screen mode accepts sprites (CR0 page-1 color = 1, LO1 = 0x80).
    write_crtc(&mut machine.bus, 0x1C, 0x0004);
    write_crtc(&mut machine.bus, 0x18, 0x0080);

    // A 32768-color pattern (index 64) with one opaque pixel at (0,0).
    let pattern_base = 0x8100_0000 + (64u32 << 7);
    machine.bus.write_byte(pattern_base, 0x34);
    machine.bus.write_byte(pattern_base + 1, 0x12);

    // Attribute entry 1000 at position (100, 100), pattern 64, visible 32K.
    let attribute_base = 0x8100_0000 + 1000 * 8;
    machine.bus.write_byte(attribute_base, 100); // X low
    machine.bus.write_byte(attribute_base + 2, 100); // Y low
    machine.bus.write_byte(attribute_base + 4, 64); // attribute: pattern 64
    machine.bus.write_byte(attribute_base + 6, 0); // 32K, visible

    // First sprite index 1000 (draws indices 1000..1023) and enable (SPEN).
    write_sprite_reg(&mut machine.bus, 0, 0xE8);
    write_sprite_reg(&mut machine.bus, 1, 0x83);

    // Advance time so the scheduled VSYNC starts the transfer and the finish
    // event paints it. Stepping keeps the run robust to the frame period.
    let mut cycle = 0u64;
    for _ in 0..400 {
        cycle += 50_000;
        machine.bus.set_current_cycle(cycle);
    }

    // The pixel must be present in one of the two double-buffered sprite pages
    // at (100, 100): offset = 512 * y + 2 * x within the page.
    let pixel_offset = 512 * 100 + 2 * 100;
    let page0 = read_vram_word(&mut machine.bus, 0x0004_0000 + pixel_offset);
    let page1 = read_vram_word(&mut machine.bus, 0x0006_0000 + pixel_offset);
    assert!(
        page0 == 0x1234 || page1 == 0x1234,
        "sprite pixel not painted: page0={page0:#06X} page1={page1:#06X}"
    );

    // The DPMD / sprite-status register at 0x044C is reachable.
    let _status = machine.bus.io_read_byte(0x044C);
}

/// With a screen mode that does not accept sprites, the engine must not blit.
#[test]
fn sprite_pipeline_idle_when_mode_rejects_sprites() {
    let mut machine = machine_mx();

    // Default power-on mode: layer 1 is 4 bpp, which does not accept sprites.
    let pattern_base = 0x8100_0000 + (64u32 << 7);
    machine.bus.write_byte(pattern_base, 0x34);
    machine.bus.write_byte(pattern_base + 1, 0x12);
    let attribute_base = 0x8100_0000 + 1000 * 8;
    machine.bus.write_byte(attribute_base, 100);
    machine.bus.write_byte(attribute_base + 2, 100);
    machine.bus.write_byte(attribute_base + 4, 64);
    write_sprite_reg(&mut machine.bus, 0, 0xE8);
    write_sprite_reg(&mut machine.bus, 1, 0x83);

    let mut cycle = 0u64;
    for _ in 0..400 {
        cycle += 50_000;
        machine.bus.set_current_cycle(cycle);
    }

    let pixel_offset = 512 * 100 + 2 * 100;
    let page0 = read_vram_word(&mut machine.bus, 0x0004_0000 + pixel_offset);
    let page1 = read_vram_word(&mut machine.bus, 0x0006_0000 + pixel_offset);
    assert_eq!(page0, 0x0000);
    assert_eq!(page1, 0x0000);
}

/// An upper bound on one frame period in CPU cycles. The real refresh rate
/// derives from the power-on CRTC totals (roughly 55-75 Hz), so `clock / 50`
/// comfortably spans one full frame regardless of the exact rate.
const FRAME_SPAN_CYCLES: u64 = (MX_CPU_CLOCK_HZ / 50) as u64;

/// Regression test for the FM Towns display-status hang: the FMR sync status
/// register (I/O 0xFDA0) must report VSYNC in bit 0 and HSYNC in bit 1, and both
/// must toggle over a frame. When bit 0 was stuck high, sprite games (After
/// Burner) spun forever in their VSYNC-edge wait.
#[test]
fn display_status_sync_bits_toggle() {
    let mut machine = machine_mx();

    let mut vsync_set = false;
    let mut vsync_clear = false;
    let mut hsync_set = false;
    let mut hsync_clear = false;
    for step in 0..=2000u64 {
        machine
            .bus
            .set_current_cycle(FRAME_SPAN_CYCLES * step / 2000);
        let status = machine.bus.io_read_byte(0xFDA0);
        if status & 0x01 != 0 {
            vsync_set = true;
        } else {
            vsync_clear = true;
        }
        if status & 0x02 != 0 {
            hsync_set = true;
        } else {
            hsync_clear = true;
        }
    }

    assert!(
        vsync_set && vsync_clear,
        "VSYNC (0xFDA0 bit 0) must both set and clear over a frame"
    );
    assert!(
        hsync_set && hsync_clear,
        "HSYNC (0xFDA0 bit 1) must both set and clear over a frame"
    );
}

/// Reproduces the exact instruction loop that hung After Burner: wait for the
/// VSYNC status bit to clear, then wait for it to rise. With the stuck-high bug
/// this never terminates; the deadline asserts progress. The deadline (two
/// seconds of CPU cycles) is far longer than any frame, so it fails only on a
/// genuine hang, not on a misjudged frame period.
#[test]
fn display_status_vsync_edge_wait_terminates() {
    let mut machine = machine_mx();
    let step = 512u64;
    let deadline = u64::from(MX_CPU_CLOCK_HZ) * 2;

    let mut cycle = 0u64;
    // Phase 1: spin while VSYNC is high (`jnz`), waiting for it to fall.
    while machine.bus.io_read_byte(0xFDA0) & 0x01 != 0 {
        cycle += step;
        machine.bus.set_current_cycle(cycle);
        assert!(
            cycle < deadline,
            "VSYNC bit never cleared (stuck-high hang)"
        );
    }
    // Phase 2: spin while VSYNC is low (`jz`), waiting for the rising edge.
    while machine.bus.io_read_byte(0xFDA0) & 0x01 == 0 {
        cycle += step;
        machine.bus.set_current_cycle(cycle);
        assert!(cycle < deadline, "VSYNC bit never set (no rising edge)");
    }
}

/// The FMR HSYNC/VSYNC alias (0xFF86) reports VSYNC in bit 2 with bit 4 always
/// set, matching the 0xFDA0 VSYNC bit at the same cycle. Writes are ignored.
#[test]
fn fmr_hsync_vsync_alias_reports_sync() {
    let mut machine = machine_mx();
    // Cycle 0 is the start of the frame, inside the vertical retrace.
    machine.bus.set_current_cycle(0);
    let status = machine.bus.io_read_byte(0xFF86);
    assert_eq!(status & 0x10, 0x10);
    assert_eq!(status & 0x04, 0x04);
    assert_eq!(machine.bus.io_read_byte(0xFDA0) & 0x01, 0x01);
    // The status port ignores writes.
    machine.bus.io_write_byte(0xFF86, 0xFF);
    assert_eq!(machine.bus.io_read_byte(0xFF86) & 0x10, 0x10);
}

/// Programs a 16-color palette and paints vertical color bars into layer 0, then
/// composes a frame by advancing past the next VSYNC edge and checks the
/// composited framebuffer shows several distinct non-black colors.
#[test]
fn palette_color_bars_compose_into_framebuffer() {
    let mut machine = machine_mx();

    // A green-to-red ramp across palette indices 1..15 (index 0 is transparent
    // on the priority page).
    for index in 1u8..16 {
        let step = (index - 1) * 18;
        machine.bus.io_write_byte(0xFD90, index);
        machine.bus.io_write_byte(0xFD94, step); // red rises left -> right
        machine.bus.io_write_byte(0xFD96, 255 - step); // green falls left -> right
        machine.bus.io_write_byte(0xFD92, 0x00); // blue
    }
    // Enable both display pages.
    machine.bus.io_write_byte(0xFDA0, 0x0F);

    // Paint 15 vertical color bars into layer 0 (native packed 4bpp, 320
    // bytes/line): index 1 (green) on the left through index 15 (red) on the
    // right.
    for y in 0..400u32 {
        for x in 0..640u32 {
            let index = (1 + x * 15 / 640) as u8;
            let address = VRAM_BASE + y * 320 + x / 2;
            let existing = machine.bus.read_byte(address);
            let byte = if x & 1 == 0 {
                (existing & 0xF0) | index
            } else {
                (existing & 0x0F) | (index << 4)
            };
            machine.bus.write_byte(address, byte);
        }
    }

    // Compose one frame by advancing the bus clock past the next VSYNC edge.
    machine.bus.set_current_cycle(FRAME_SPAN_CYCLES);

    let (width, height) = machine.display_dimensions();
    let total_pixels = (width * height) as usize;
    assert!(total_pixels > 0, "a frame must have been composed");

    let framebuffer = machine.display_framebuffer();
    let mut distinct_colors = std::collections::BTreeSet::new();
    let mut non_black = 0usize;
    for pixel in framebuffer.chunks_exact(4) {
        if pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0 {
            non_black += 1;
            distinct_colors.insert((pixel[0], pixel[1], pixel[2]));
        }
    }
    assert!(
        non_black > total_pixels / 2,
        "the color bars should cover most of the frame ({non_black}/{total_pixels})"
    );
    assert!(
        distinct_colors.len() >= 3,
        "the color-bar ramp should render several distinct colors (got {})",
        distinct_colors.len()
    );
}
