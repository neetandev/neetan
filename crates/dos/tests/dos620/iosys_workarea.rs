use crate::harness;

// IO.SYS work area is at segment 0060h. Linear address = 0x600 + offset.
const IOSYS_BASE: u32 = 0x0600;

#[test]
fn daua_mapping_table() {
    let machine = harness::boot_hle();
    // DA/UA mapping list at 0060:006Ch (linear 0x66C), 16 bytes for A:-P:.
    // With no media attached, all entries should be 0x00 (no drives assigned).
    for i in 0..16u32 {
        let daua = harness::read_byte(&machine.bus, IOSYS_BASE + 0x006C + i);
        assert!(
            daua == 0x00 || (0x70..=0xF3).contains(&daua),
            "DA/UA table entry {} should be 0x00 or a valid device address, got {:#04X}",
            i,
            daua
        );
    }
}

#[test]
fn kanji_graph_mode() {
    let machine = harness::boot_hle();
    let mode = harness::read_byte(&machine.bus, IOSYS_BASE + 0x008A);
    assert!(
        mode == 0x00 || mode == 0x01,
        "Kanji/graph mode at 0060:008Ah should be 0x00 (graphic) or 0x01 (kanji), got {:#04X}",
        mode
    );
}

#[test]
fn cursor_y_position() {
    let machine = harness::boot_hle();
    let cursor_y = harness::read_byte(&machine.bus, IOSYS_BASE + 0x0110);
    assert!(
        cursor_y <= 24,
        "Cursor Y position should be 0-24, got {}",
        cursor_y
    );
}

#[test]
fn function_key_display_state() {
    let machine = harness::boot_hle();
    let state = harness::read_byte(&machine.bus, IOSYS_BASE + 0x0111);
    assert!(
        state <= 0x02,
        "Function key display state should be 0x00, 0x01, or 0x02, got {:#04X}",
        state
    );
}

#[test]
fn screen_line_count() {
    let machine = harness::boot_hle();
    let lines = harness::read_byte(&machine.bus, IOSYS_BASE + 0x0113);
    assert!(
        lines == 0x00 || lines == 0x01,
        "Screen line count should be 0x00 (20-line) or 0x01 (25-line), got {:#04X}",
        lines
    );
}

#[test]
fn clear_attribute() {
    let machine = harness::boot_hle();
    let attr = harness::read_byte(&machine.bus, IOSYS_BASE + 0x0114);
    assert!(
        attr == 0xE1 || attr == 0x81,
        "Clear attribute should be 0xE1 or 0x81, got {:#04X}",
        attr
    );
}

#[test]
fn line_wrap_flag() {
    let machine = harness::boot_hle();
    let wrap = harness::read_byte(&machine.bus, IOSYS_BASE + 0x0117);
    assert!(
        wrap == 0x00 || wrap == 0x01,
        "Line wrap flag should be 0x00 (wrap) or 0x01 (no wrap), got {:#04X}",
        wrap
    );
}

#[test]
fn clear_character() {
    let machine = harness::boot_hle();
    let ch = harness::read_byte(&machine.bus, IOSYS_BASE + 0x0119);
    assert_eq!(
        ch, 0x20,
        "Clear character should be 0x20 (space), got {:#04X}",
        ch
    );
}

#[test]
fn cursor_visibility() {
    let machine = harness::boot_hle();
    let visible = harness::read_byte(&machine.bus, IOSYS_BASE + 0x011B);
    assert!(
        visible == 0x00 || visible == 0x01,
        "Cursor visibility should be 0x00 or 0x01, got {:#04X}",
        visible
    );
}

#[test]
fn cursor_x_position() {
    let machine = harness::boot_hle();
    let cursor_x = harness::read_byte(&machine.bus, IOSYS_BASE + 0x011C);
    assert!(
        cursor_x <= 79,
        "Cursor X position should be 0-79, got {}",
        cursor_x
    );
}

#[test]
fn product_number() {
    let machine = harness::boot_hle();
    let product = harness::read_word(&machine.bus, IOSYS_BASE + 0x0020);
    assert_ne!(
        product, 0x0000,
        "MS-DOS product number at 0060:0020h should be non-zero"
    );
}

#[test]
fn display_attribute() {
    let machine = harness::boot_hle();
    let attr = harness::read_byte(&machine.bus, IOSYS_BASE + 0x011D);
    assert_ne!(
        attr, 0x00,
        "Display attribute at 0060:011Dh should be non-zero, got {:#04X}",
        attr
    );
}

#[test]
fn scroll_range_upper() {
    let machine = harness::boot_hle();
    let upper = harness::read_byte(&machine.bus, IOSYS_BASE + 0x011E);
    assert!(
        upper <= 24,
        "Scroll range upper limit should be 0-24, got {}",
        upper
    );
}

#[test]
fn scroll_wait_value() {
    let machine = harness::boot_hle();
    let wait = harness::read_word(&machine.bus, IOSYS_BASE + 0x011F);
    assert_ne!(
        wait, 0x0000,
        "Scroll wait value at 0060:011Fh should be non-zero"
    );
}

#[test]
fn extended_attribute_mode() {
    let machine = harness::boot_hle();
    let mode = harness::read_word(&machine.bus, IOSYS_BASE + 0x05D6);
    assert!(
        mode == 0x0000 || mode == 0x0001,
        "Extended attribute mode should be 0x0000 (PC) or 0x0001 (EGH), got {:#06X}",
        mode
    );
}

#[test]
fn text_mode_flag() {
    let machine = harness::boot_hle();
    let mode = harness::read_word(&machine.bus, IOSYS_BASE + 0x05D8);
    assert!(
        mode == 0x0000 || mode == 0x0001,
        "Text mode flag should be 0x0000 (25-line) or 0x0001 (20/25), got {:#06X}",
        mode
    );
}
