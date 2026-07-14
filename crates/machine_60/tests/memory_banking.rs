//! Memory-map and banking tests for the mkII decode-table map and the SR
//! read/write page mapper.

use machine_60::Pc6000Model;

mod harness;
use harness::{build_machine, build_machine_with_synthetic_roms};

#[test]
fn mkii_reset_maps_basic_low_and_work_ram_high() {
    let mut machine = build_machine_with_synthetic_roms(Pc6000Model::Pc6001Mk2, |roms| {
        roms.basic.as_mut().unwrap()[0] = 0xB1;
    });
    let bus = &mut machine.bus;

    // 0x0000 reads the first BASIC page at reset.
    assert_eq!(bus.peek_byte(0x0000), 0xB1);
    // 0xC000-0xFFFF is work RAM and reads back writes.
    bus.poke_byte(0xC000, 0x5A);
    assert_eq!(bus.peek_byte(0xC000), 0x5A);
}

#[test]
fn mkii_write_routes_to_extended_ram_at_reset() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2);
    let bus = &mut machine.bus;

    // At reset the low write bank targets extended work RAM. Write through page 0,
    // then expose extended RAM and work RAM in turn to prove where it landed.
    bus.poke_byte(0x0000, 0xE7);

    bus.io_write(0xF0, 0x0E); // read bank low: extended work RAM in pages 0-1
    assert_eq!(
        bus.peek_byte(0x0000),
        0xE7,
        "write reached extended work RAM"
    );

    bus.io_write(0xF0, 0x0D); // read bank low: work RAM in pages 0-1
    assert_eq!(bus.peek_byte(0x0000), 0x00, "work RAM page 0 was untouched");
}

#[test]
fn mkii_opt_bank_selects_kanji_over_character_generator() {
    let mut machine = build_machine_with_synthetic_roms(Pc6000Model::Pc6001Mk2, |roms| {
        roms.cg_base.as_mut().unwrap()[0] = 0xC6;
        roms.kanji.as_mut().unwrap()[0] = 0x9A;
    });
    let bus = &mut machine.bus;

    // Read bank 0x02 maps the character source into page 0; the opt bank chooses
    // the character generator (0) or the kanji ROM (1).
    bus.io_write(0xF0, 0x02);

    bus.io_write(0xC2, 0x00);
    assert_eq!(bus.peek_byte(0x0000), 0xC6, "opt bank 0 exposes the CG ROM");

    bus.io_write(0xC2, 0x01);
    assert_eq!(
        bus.peek_byte(0x0000),
        0x9A,
        "opt bank 1 exposes the kanji ROM"
    );
}

#[test]
fn sr_read_page_register_maps_an_8kib_page() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    let bus = &mut machine.bus;

    // Point write page 0 and read page 0 at extended RAM page 0x10 (physical
    // 0x20000), write through it, and read it back.
    bus.io_write(0x68, 0x10 << 1);
    bus.poke_byte(0x0000, 0xAB);
    bus.io_write(0x60, 0x10 << 1);
    assert_eq!(bus.peek_byte(0x0000), 0xAB);
}

#[test]
fn sr_write_page_is_independent_of_read_page() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    let bus = &mut machine.bus;

    // Read page 0 -> work RAM page 0; write page 0 -> work RAM page 1.
    bus.io_write(0x60, 0x00 << 1);
    bus.io_write(0x68, 0x01 << 1);
    bus.poke_byte(0x0000, 0xCD);

    // The read window still sees the untouched page 0.
    assert_eq!(
        bus.peek_byte(0x0000),
        0x00,
        "write did not disturb read page 0"
    );

    // Switching the read page to the written page reveals the byte.
    bus.io_write(0x60, 0x01 << 1);
    assert_eq!(
        bus.peek_byte(0x0000),
        0xCD,
        "write landed in work RAM page 1"
    );
}

#[test]
fn sr_bitmap_mode_overlays_gvram_on_page_zero() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    let bus = &mut machine.bus;

    // Work RAM page 0 in both windows.
    bus.io_write(0x60, 0x00);
    bus.io_write(0x68, 0x00);

    // Text mode (mode register bit 0x08 set): page 0 is work RAM.
    bus.io_write(0xC8, 0x08);
    bus.poke_byte(0x0000, 0x11);

    // Bitmap mode (bit 0x08 clear): page 0 is the graphics VRAM overlay.
    bus.io_write(0xC8, 0x00);
    bus.poke_byte(0x0000, 0x22);
    assert_eq!(bus.peek_byte(0x0000), 0x22, "bitmap mode reads GVRAM");

    // Back to text mode: the original work-RAM byte is intact.
    bus.io_write(0xC8, 0x08);
    assert_eq!(bus.peek_byte(0x0000), 0x11, "work RAM survived the overlay");
}

#[test]
fn sr_system_rom_is_readable_at_reset() {
    let machine = build_machine_with_synthetic_roms(Pc6000Model::Pc6001Mk2Sr, |roms| {
        // Reset read page 0 selects physical page 0x7C (system ROM half 1 + 0x8000).
        roms.system_rom1.as_mut().unwrap()[0x8000] = 0x3C;
    });
    assert_eq!(machine.bus.peek_byte(0x0000), 0x3C);
}

#[test]
fn sr_cartridge_exrom_window_reads_the_image() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    machine.bus.load_cartridge(&[0x7E; 0x100]);
    let bus = &mut machine.bus;

    // The cartridge exROM lives at physical 0xB4000 -> page 0x5A.
    bus.io_write(0x60, 0x5A << 1);
    assert_eq!(bus.peek_byte(0x0000), 0x7E);
}

#[test]
fn sr_compatibility_maps_voice_rom_window() {
    let mut machine = build_machine_with_synthetic_roms(Pc6000Model::Pc6001Mk2Sr, |roms| {
        roms.voice = Some(vec![0; 0x4000]);
        roms.voice.as_mut().unwrap()[0x2000] = 0x21;
    });
    let bus = &mut machine.bus;

    bus.io_write(0xC8, 0xFD);
    bus.io_write(0xF0, 0x61);
    bus.io_write(0xC2, 0xFE);

    assert_eq!(bus.peek_byte(0x6000), 0x21);
}
