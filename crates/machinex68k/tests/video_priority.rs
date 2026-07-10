//! Full-machine layer-mixing tests: the video-controller priority register
//! orders the sprite, text, and graphic screens, and the R2 extension bits
//! select special priority and translucency, all programmed through the
//! CPU-visible bus and verified on the rendered framebuffer.

#[path = "common/harness.rs"]
mod harness;

use harness::{machine, pixel, read_word, write_byte, write_word};
use machinex68k::{X68kMachine, X68kModel};

/// Black pixel of an unlit display position.
const BLACK: [u8; 4] = [0, 0, 0, 255];
/// Graphics palette entry rendered at full contrast (GRBI 0x07C0).
const RED: [u8; 4] = [251, 0, 0, 255];
/// Text palette entry rendered at full contrast (GRBI 0xF800).
const GREEN: [u8; 4] = [0, 251, 0, 255];
/// Sprite palette entry rendered at full contrast (GRBI 0x003E).
const BLUE: [u8; 4] = [0, 0, 251, 255];

/// Builds one priority register value from screen priorities and the
/// default graphic page order.
const fn priority(sprite: u16, text: u16, graphic: u16) -> u16 {
    sprite << 12 | text << 10 | graphic << 8 | 0x00E4
}

/// Builds a machine showing a 16x1 display with a red graphic pixel and a
/// green text pixel in column 0, under a blue full-coverage sprite.
fn layered_machine() -> X68kMachine {
    let mut machine = machine(X68kModel::X68000);
    // CRTC R00-R09: an 11-column, 4-raster frame with a 16x1 display.
    for (index, value) in [10u16, 0, 0, 2, 3, 0, 0, 1, 0, 3].into_iter().enumerate() {
        write_word(&mut machine, 0xE80000 + index as u32 * 2, value);
    }
    write_byte(&mut machine, 0xE8E001, 15);
    // Graphics palette entry 1: red. Text entry 1: green. Sprite palette
    // block 1 entry 5: blue.
    write_word(&mut machine, 0xE82002, 0x07C0);
    write_word(&mut machine, 0xE82202, 0xF800);
    write_word(&mut machine, 0xE8222A, 0x003E);
    // Graphic page 0 and text plane 0: one pixel each in column 0.
    write_word(&mut machine, 0xC00000, 1);
    write_byte(&mut machine, 0xE00000, 0x80);
    // Sprite pattern 1: every dot shows palette code 5.
    for word in 0..64u32 {
        write_word(&mut machine, 0xEB8000 + (64 + word) * 2, 0x5555);
    }
    // Sprite 0 covers the whole display row from position (0, 0).
    write_word(&mut machine, 0xEB0000, 16);
    write_word(&mut machine, 0xEB0002, 16);
    write_word(&mut machine, 0xEB0004, 0x0101);
    write_word(&mut machine, 0xEB0006, 3);
    // Sprite screen porch registers.
    write_word(&mut machine, 0xEB080C, 0x0004);
    write_word(&mut machine, 0xEB0808, 0x0200);
    machine
}

/// Programs the mixing and priority registers and renders some frames.
fn render(machine: &mut X68kMachine, mixing: u16, priority: u16) {
    write_word(machine, 0xE82600, mixing);
    write_word(machine, 0xE82500, priority);
    machine.run_for(10_000);
}

/// R2 value enabling the sprite screen, the text screen, and graphic page 0.
const ALL_SCREENS: u16 = 0x0061;

#[test]
fn priority_register_orders_the_three_screens() {
    let mut machine = layered_machine();

    render(&mut machine, ALL_SCREENS, priority(0, 1, 2));
    assert_eq!(pixel(&machine, 0, 0), BLUE, "the sprite screen is in front");

    render(&mut machine, ALL_SCREENS, priority(1, 0, 2));
    assert_eq!(pixel(&machine, 0, 0), GREEN, "the text screen is in front");
    assert_eq!(pixel(&machine, 1, 0), BLUE, "the sprite shows beside it");

    render(&mut machine, ALL_SCREENS, priority(1, 2, 0));
    assert_eq!(pixel(&machine, 0, 0), RED, "the graphic screen is in front");
}

#[test]
fn frontmost_graphic_priority_blanks_sprite_and_text() {
    let mut machine = layered_machine();
    render(&mut machine, ALL_SCREENS, priority(0, 1, 3));
    assert_eq!(pixel(&machine, 0, 0), RED, "the graphic pixel still shows");
    assert_eq!(
        pixel(&machine, 1, 0),
        BLACK,
        "the covering sprite is switched off"
    );
}

#[test]
fn special_priority_by_color_jumps_odd_colors_to_the_front() {
    let mut machine = layered_machine();
    // Graphic code 2 whose palette color has the intensity bit set.
    write_word(&mut machine, 0xC00000, 2);
    write_word(&mut machine, 0xE82004, 0x07C1);

    render(&mut machine, ALL_SCREENS, priority(0, 1, 2));
    assert_eq!(
        pixel(&machine, 0, 0),
        BLUE,
        "without the extension the sprite hides the graphic"
    );

    render(&mut machine, ALL_SCREENS | 0x1000, priority(0, 1, 2));
    assert_eq!(
        pixel(&machine, 0, 0),
        [255, 4, 4, 255],
        "the odd color jumps in front of the sprite"
    );
}

#[test]
fn special_priority_by_palette_shows_the_evened_entry() {
    let mut machine = layered_machine();
    // Graphic code 3: the odd palette code selects the region, and the
    // evened neighbor entry 2 supplies the displayed color.
    write_word(&mut machine, 0xC00000, 3);
    write_word(&mut machine, 0xE82004, 0x07C0);
    write_word(&mut machine, 0xE82006, 0xF800);

    render(&mut machine, ALL_SCREENS | 0x1400, priority(0, 1, 2));
    assert_eq!(
        pixel(&machine, 0, 0),
        RED,
        "the front jump shows entry 2, not the written entry 3"
    );
}

#[test]
fn sprite_priority_word_bit_two_selects_the_absent_chip() {
    let mut machine = layered_machine();
    // Priority word 7: placement 3 with the chip-select bit set. The dots
    // come from the missing second pattern chip, so the sprite vanishes,
    // while the bit itself stays readable.
    write_word(&mut machine, 0xEB0006, 7);
    render(&mut machine, ALL_SCREENS, priority(0, 1, 2));
    assert_eq!(read_word(&mut machine, 0xEB0006), 7);
    assert_eq!(
        pixel(&machine, 0, 0),
        GREEN,
        "the text pixel shows where the sprite vanished"
    );
    assert_eq!(pixel(&machine, 1, 0), BLACK, "no sprite dot renders");

    write_word(&mut machine, 0xEB0006, 3);
    machine.run_for(10_000);
    assert_eq!(
        pixel(&machine, 1, 0),
        BLUE,
        "clearing the bit restores the sprite"
    );
}

#[test]
fn translucency_blends_the_graphic_with_the_screen_behind() {
    let mut machine = layered_machine();
    // Graphic code 3 in front: entry 2 is odd, marking the region
    // translucent against the sprite behind the graphic screen.
    write_word(&mut machine, 0xC00000, 3);
    write_word(&mut machine, 0xE82004, 0x0001);
    write_word(&mut machine, 0xE82006, 0x07C0);

    render(&mut machine, ALL_SCREENS | 0x1900, priority(1, 2, 0));
    // The red graphic (GRBI 0x07C0) averages with the blue sprite
    // (GRBI 0x003E) to GRBI 0x03DE.
    assert_eq!(
        pixel(&machine, 0, 0),
        [121, 0, 121, 255],
        "the graphic blends with the sprite behind it"
    );
    assert_eq!(
        pixel(&machine, 1, 0),
        BLUE,
        "unmarked positions show the sprite normally"
    );
}
