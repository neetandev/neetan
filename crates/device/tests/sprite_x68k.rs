use device::sprite_x68k::{SpriteX68k, X68K_SPRITE_COUNT, X68K_SPRITE_PATTERN_WORDS};

#[test]
fn scroll_entries_mask_their_writable_bits() {
    let mut sprite = SpriteX68k::new();
    sprite.write_word(0xEB0000, 0xFFFF);
    sprite.write_word(0xEB0002, 0xFFFF);
    sprite.write_word(0xEB0004, 0xFFFF);
    sprite.write_word(0xEB0006, 0xFFFF);
    assert_eq!(sprite.read_word(0xEB0000), 0x03FF);
    assert_eq!(sprite.read_word(0xEB0002), 0x03FF);
    assert_eq!(sprite.read_word(0xEB0004), 0xCFFF);
    assert_eq!(sprite.read_word(0xEB0006), 0x0007);
    assert_eq!(sprite.sprite_x(0), 0x03FF);
    assert_eq!(sprite.sprite_y(0), 0x03FF);
    assert_eq!(sprite.sprite_pattern_word(0), 0xCFFF);
    assert_eq!(sprite.sprite_priority(0), 3);
}

#[test]
fn scroll_entries_are_independent_per_sprite() {
    let mut sprite = SpriteX68k::new();
    let last = 0xEB0000 + (X68K_SPRITE_COUNT as u32 - 1) * 8;
    sprite.write_word(0xEB0000, 0x0010);
    sprite.write_word(last, 0x0020);
    sprite.write_word(last + 4, 0x8123);
    assert_eq!(sprite.sprite_x(0), 0x0010);
    assert_eq!(sprite.sprite_x(127), 0x0020);
    assert_eq!(sprite.sprite_pattern_word(127), 0x8123);
    assert_eq!(sprite.sprite_pattern_word(0), 0);
}

#[test]
fn registers_mask_their_writable_bits() {
    let mut sprite = SpriteX68k::new();
    for offset in (0_u32..=0x10).step_by(2) {
        sprite.write_word(0xEB0800 + offset, 0xFFFF);
    }
    assert_eq!(sprite.read_word(0xEB0800), 0x03FF);
    assert_eq!(sprite.read_word(0xEB0802), 0x03FF);
    assert_eq!(sprite.read_word(0xEB0804), 0x03FF);
    assert_eq!(sprite.read_word(0xEB0806), 0x03FF);
    assert_eq!(sprite.read_word(0xEB0808), 0x063F);
    assert_eq!(sprite.read_word(0xEB080A), 0x00FF);
    assert_eq!(sprite.read_word(0xEB080C), 0x003F);
    assert_eq!(sprite.read_word(0xEB080E), 0x00FF);
    assert_eq!(sprite.read_word(0xEB0810), 0x001F);
    assert_eq!(sprite.background_scroll_x(0), 0x03FF);
    assert_eq!(sprite.background_scroll_y(0), 0x03FF);
    assert_eq!(sprite.background_scroll_x(1), 0x03FF);
    assert_eq!(sprite.background_scroll_y(1), 0x03FF);
    assert_eq!(sprite.background_control(), 0x063F);
    assert_eq!(sprite.horizontal_front_end(), 0x00FF);
    assert_eq!(sprite.horizontal_back_end(), 0x003F);
    assert_eq!(sprite.vertical_back_end(), 0x00FF);
    assert_eq!(sprite.resolution(), 0x001F);
}

#[test]
fn holes_read_ffff_and_ignore_writes() {
    let mut sprite = SpriteX68k::new();
    for address in [0xEB0400_u32, 0xEB07FE, 0xEB0812, 0xEB4000, 0xEB7FFE] {
        sprite.write_word(address, 0x1234);
        assert_eq!(sprite.read_word(address), 0xFFFF);
    }
}

#[test]
fn pattern_ram_round_trips_words() {
    let mut sprite = SpriteX68k::new();
    sprite.write_word(0xEB8000, 0x1234);
    sprite.write_word(0xEBFFFE, 0x5678);
    assert_eq!(sprite.read_word(0xEB8000), 0x1234);
    assert_eq!(sprite.read_word(0xEBFFFE), 0x5678);
    assert_eq!(sprite.pattern_data()[0], 0x1234);
    assert_eq!(sprite.pattern_data()[X68K_SPRITE_PATTERN_WORDS - 1], 0x5678);
}

#[test]
fn byte_writes_duplicate_into_scroll_and_pattern_words() {
    let mut sprite = SpriteX68k::new();
    sprite.write_byte(0xEB8001, 0x9A);
    assert_eq!(sprite.read_word(0xEB8000), 0x9A9A);
    sprite.write_byte(0xEB0000, 0xFF);
    assert_eq!(sprite.read_word(0xEB0000), 0x03FF);
    sprite.write_byte(0xEB0809, 0x55);
    assert_eq!(sprite.read_word(0xEB0808), 0x0015);
    sprite.write_byte(0xEB0808, 0xFF);
    assert_eq!(sprite.read_word(0xEB0808), 0x0615);
}

#[test]
fn byte_reads_select_the_word_lanes() {
    let mut sprite = SpriteX68k::new();
    sprite.write_word(0xEB8000, 0x1234);
    assert_eq!(sprite.read_byte(0xEB8000), 0x12);
    assert_eq!(sprite.read_byte(0xEB8001), 0x34);
    assert_eq!(sprite.read_byte(0xEB0400), 0xFF);
}

#[test]
fn reset_clears_registers_and_pattern_ram() {
    let mut sprite = SpriteX68k::new();
    sprite.write_word(0xEB0000, 0x0123);
    sprite.write_word(0xEB0808, 0x063F);
    sprite.write_word(0xEB8000, 0x4567);
    sprite.reset();
    assert_eq!(sprite.read_word(0xEB0000), 0);
    assert_eq!(sprite.read_word(0xEB0808), 0);
    assert_eq!(sprite.read_word(0xEB8000), 0);
}
