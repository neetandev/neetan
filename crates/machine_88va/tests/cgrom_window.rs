//! CGROM window integration tests: the kanji/ANK character generator read window
//! that exposes font ROM glyph bytes through the public bus surface.

use common::Bus;

#[path = "common/harness.rs"]
mod harness;
use harness::*;

#[test]
fn cgrom_window_reads_ank_glyph() {
    let mut machine = machine();
    let font = fill(FONT_SEED, 0x5_0000);

    // Half-width 'A' (0x41), raster row 3. Row bit5 set selects the ANK half.
    machine.bus.io_write_byte(0x14C, 0x41);
    machine.bus.io_write_byte(0x14D, 0x00);
    machine.bus.io_write_byte(0x14F, 0x20 | 0x03);

    let expected = font[0x40000 + (0x41 << 4) + 3];
    assert_eq!(machine.bus.io_read_byte(0x14E), expected);
}

#[test]
fn cgrom_window_reads_full_width_glyph() {
    let mut machine = machine();
    let font = fill(FONT_SEED, 0x5_0000);

    // JIS code: low byte = jis1 - 0x20 = 0x01, high byte = jis2 = 0x21, row 2.
    machine.bus.io_write_byte(0x14C, 0x01);
    machine.bus.io_write_byte(0x14D, 0x21);
    machine.bus.io_write_byte(0x14F, 0x20 | 0x02);

    // jis1 = 0x21 (< 0x28): font = (0x20 << 8) + (0x01 << 10) + (0x01 << 5) = 0x2420.
    // width = 2, row 2 -> offset 0x2420 + 4.
    let expected = font[0x2420 + 4];
    assert_eq!(machine.bus.io_read_byte(0x14E), expected);
}
