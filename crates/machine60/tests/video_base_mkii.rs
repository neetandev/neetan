//! Machine-level tests for the legacy base PC-6001 and mkII render paths: text
//! content, a graphics mode, and the mkII video mode register.

use machine60::{Pc6000Bus, Pc6000Model};

mod harness;
use harness::{build_machine_with_synthetic_roms, run_bus_cycles};

/// Renders at least one frame and returns a snapshot of the framebuffer.
fn render_snapshot(bus: &mut Pc6000Bus) -> Vec<u8> {
    let cycles = u64::from(bus.cpu_clock_hz()) / 30;
    run_bus_cycles(bus, cycles);
    bus.display_framebuffer().to_vec()
}

#[test]
fn base_text_content_changes_the_frame() {
    let mut machine = build_machine_with_synthetic_roms(Pc6000Model::Pc6001, |roms| {
        // Tile 0x41 glyph fully lit, so a referenced cell differs from a blank one.
        for line in 0..12 {
            roms.cg_base.as_mut().unwrap()[0x41 * 0x10 + line] = 0xFF;
        }
    });
    let bus = &mut machine.bus;

    // System latch selects the 0xC000 video base and leaves text mode.
    bus.io_write(0xB0, 0x00);
    let blank = render_snapshot(bus);

    // Global attribute byte = text mode; tile map cell 0 references the lit glyph.
    bus.poke_byte(0xC000, 0x00);
    bus.poke_byte(0xC000 + 0x200, 0x41);
    let drawn = render_snapshot(bus);

    assert_ne!(blank, drawn, "base text content did not change the frame");
}

#[test]
fn base_graphics_mode_changes_the_frame() {
    let mut machine = build_machine_with_synthetic_roms(Pc6000Model::Pc6001, |_| {});
    let bus = &mut machine.bus;

    bus.io_write(0xB0, 0x00);
    let text = render_snapshot(bus);

    // Global attribute bit 7 selects a graphics mode; fill the bitmap with a
    // non-zero pattern.
    bus.poke_byte(0xC000, 0x80);
    for offset in 0..0x400u16 {
        bus.poke_byte(0xC000 + offset, 0xFF);
    }
    bus.poke_byte(0xC000, 0x80);
    let graphics = render_snapshot(bus);

    assert_ne!(text, graphics, "the graphics mode did not change the frame");
}

#[test]
fn mkii_legacy_text_content_changes_the_frame() {
    let mut machine = build_machine_with_synthetic_roms(Pc6000Model::Pc6001Mk2, |roms| {
        for line in 0..12 {
            roms.cg_base.as_mut().unwrap()[0x41 * 0x10 + line] = 0xFF;
        }
    });
    let bus = &mut machine.bus;

    // Latch the video base to the 0x8000 work-RAM window.
    bus.io_write(0xB0, 0x00);
    let blank = render_snapshot(bus);

    bus.poke_byte(0x8000, 0x00);
    bus.poke_byte(0x8000 + 0x200, 0x41);
    let drawn = render_snapshot(bus);

    assert_ne!(
        blank, drawn,
        "mkII legacy text content did not change the frame"
    );
}

#[test]
fn mkii_video_mode_register_selects_extended_text() {
    let mut machine = build_machine_with_synthetic_roms(Pc6000Model::Pc6001Mk2, |roms| {
        for line in 0..12 {
            roms.cg_base.as_mut().unwrap()[0x41 * 0x10 + line] = 0xFF;
        }
    });
    let bus = &mut machine.bus;

    // Fill the 0x8000 video window (the base both mode values resolve to) so the
    // only difference between the two renders is the mode register itself.
    bus.io_write(0xB0, 0x00);
    for offset in 0..0x600u16 {
        bus.poke_byte(0x8000 + offset, 0xFF);
    }
    bus.poke_byte(0x8000, 0x00);
    bus.poke_byte(0x8000 + 0x200, 0x41);

    // Extended text mode (bit 1 clear) versus the extended bitmap mode (bit 3).
    bus.io_write(0xC1, 0x00);
    let text = render_snapshot(bus);

    bus.io_write(0xC1, 0x08);
    let bitmap = render_snapshot(bus);

    assert_ne!(
        text, bitmap,
        "the mkII mode register did not change the frame"
    );
}
