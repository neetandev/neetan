//! The video save pointer chain: BDA 40:A8, the save pointer table in the VGA
//! stub ROM, the video parameter table it leads to, and the character set
//! overrides a guest installs through it.
//!
//! The expected values are the ones the real AMI + ET4000AX BIOS publishes,
//! probed in the emulator: the pointer lives in the VGA BIOS segment, the
//! table holds the standard 29 entries of 64 bytes, and the dynamic save area,
//! both character set override pointers and the secondary save pointer stay
//! null after POST.

use common::NoTrace;
use machine_at::AtMachine;

use super::{
    boot_and_run, create_machine_dx50, create_machine_dx66, inject_and_run, read_ram_u8,
    read_ram_u16, read_ram_u32, write_bytes,
};

/// Cycle budget for one injected INT 10h sequence.
const BUDGET: u64 = 200_000_000;
/// BIOS data area: current video mode.
const BDA_VIDEO_MODE: u32 = 0x449;
/// BIOS data area: text rows minus one.
const BDA_VIDEO_ROWS: u32 = 0x484;
/// BIOS data area: character cell height (word).
const BDA_CHAR_HEIGHT: u32 = 0x485;
/// BIOS data area: SAVE_PTR, the far pointer to the save pointer table.
const BDA_SAVE_POINTER: u32 = 0x4A8;
/// Real-mode segment of the VGA stub ROM.
const VGA_ROM_SEGMENT: u32 = 0xC000;
/// Physical base of the VGA stub ROM.
const VGA_ROM_BASE: u32 = 0xC0000;
/// VGA ROM metadata word: video parameter table offset.
const VGA_METADATA_VIDEO_PARAMETER_TABLE: u32 = VGA_ROM_BASE + 0x1A;
/// VGA ROM metadata word: video parameter table entry count.
const VGA_METADATA_VIDEO_PARAMETER_COUNT: u32 = VGA_ROM_BASE + 0x1C;
/// VGA ROM metadata word: video save pointer table offset.
const VGA_METADATA_SAVE_POINTER_TABLE: u32 = VGA_ROM_BASE + 0x1E;
/// Size of one video parameter table entry in bytes.
const ENTRY_SIZE: u32 = 64;
/// Video parameter table index of mode 03h, the mode POST leaves behind.
const MODE_03H_INDEX: u32 = 24;
/// Guest address of the save pointer table copy the override tests install.
const GUEST_SAVE_POINTER_TABLE: u32 = 0x3000;
/// Guest address of the character set override table.
const GUEST_OVERRIDE_TABLE: u32 = 0x3100;
/// Guest address of the override glyph bitmaps.
const GUEST_GLYPHS: u32 = 0x3200;
/// First character code the alphanumeric override replaces.
const OVERRIDE_FIRST_CODE: u32 = 0x41;

/// Video parameter table entry the generator produces for mode 03h, byte
/// identical to the entry the real ET4000AX BIOS publishes at the same index.
#[rustfmt::skip]
const MODE_03H_ENTRY: [u8; 64] = [
    0x50, 0x18, 0x10,               // 80 columns, 24 rows minus one, 16 scan lines
    0x00, 0x10,                     // page size 0x1000
    0x00, 0x03, 0x00, 0x02,         // sequencer 1-4
    0x67,                           // miscellaneous output
    0x5F, 0x4F, 0x50, 0x82, 0x55, 0x81, 0xBF, 0x1F,
    0x00, 0x4F, 0x0D, 0x0E, 0x00, 0x00, 0x00, 0x00,
    0x9C, 0x8E, 0x8F, 0x28, 0x1F, 0x96, 0xB9, 0xA3,
    0xFF,                           // CRTC 0x00-0x18
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x14, 0x07,
    0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F,
    0x0C, 0x00, 0x0F, 0x08,         // attribute controller 0x00-0x13
    0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x0E, 0x00,
    0xFF,                           // graphics controller 0x00-0x08
];

/// Builds the code for one INT 10h AH=00h mode set.
fn set_mode_code(mode: u8) -> Vec<u8> {
    vec![
        0xB8, mode, 0x00, // MOV AX, 00<mode>
        0xCD, 0x10, // INT 10h
        0xF4, // HLT
    ]
}

/// Linear address a far pointer stored as a dword resolves to.
fn linear(pointer: u32) -> u32 {
    ((pointer >> 16) << 4).wrapping_add(pointer & 0xFFFF)
}

/// Linear address of the video parameter table the BDA pointer chain leads to.
fn parameter_table(machine: &AtMachine<NoTrace>) -> u32 {
    let table = linear(read_ram_u32(machine, BDA_SAVE_POINTER));
    linear(read_ram_u32(machine, table))
}

/// Reads one glyph row from plane 2, the character generator plane.
fn plane_2_byte(machine: &AtMachine<NoTrace>, code: u32, row: u32) -> u8 {
    machine.bus.vga().vram()[((code * 32 + row) * 4 + 2) as usize]
}

/// Copies the ROM save pointer table into guest RAM, points BDA 40:A8 at the
/// copy, and stores `pointer` in the copy's dword at `offset`.
fn install_guest_save_pointer_table(machine: &mut AtMachine<NoTrace>, offset: u32, pointer: u32) {
    let rom_table = linear(read_ram_u32(machine, BDA_SAVE_POINTER));
    let bytes: Vec<u8> = (0..28)
        .map(|index| read_ram_u8(machine, rom_table + index))
        .collect();
    write_bytes(machine, GUEST_SAVE_POINTER_TABLE, &bytes);
    write_bytes(
        machine,
        GUEST_SAVE_POINTER_TABLE + offset,
        &pointer.to_le_bytes(),
    );
    write_bytes(
        machine,
        BDA_SAVE_POINTER,
        &GUEST_SAVE_POINTER_TABLE.to_le_bytes(),
    );
}

/// Installs an alphanumeric character set override for two glyphs of
/// `glyph_height` scan lines, with the given displayed row count and
/// applicable mode list. Returns the glyph bitmap bytes it wrote.
fn install_alpha_override(
    machine: &mut AtMachine<NoTrace>,
    glyph_height: u8,
    rows: u8,
    modes: &[u8],
) -> Vec<u8> {
    install_guest_save_pointer_table(machine, 8, GUEST_OVERRIDE_TABLE);

    let mut table = vec![
        glyph_height,
        0x00, // character generator block 0
        0x02,
        0x00, // two characters
        OVERRIDE_FIRST_CODE as u8,
        0x00, // starting at 'A'
        (GUEST_GLYPHS & 0xFF) as u8,
        ((GUEST_GLYPHS >> 8) & 0xFF) as u8,
        0x00,
        0x00, // font table at 0000:3200
        rows,
    ];
    table.extend_from_slice(modes);
    write_bytes(machine, GUEST_OVERRIDE_TABLE, &table);

    let bitmaps: Vec<u8> = (0..2u8)
        .flat_map(|glyph| (0..glyph_height).map(move |row| 0xA0 | glyph | (row << 1)))
        .collect();
    write_bytes(machine, GUEST_GLYPHS, &bitmaps);
    bitmaps
}

#[test]
fn post_publishes_the_save_pointer_chain() {
    for mut machine in [create_machine_dx50(), create_machine_dx66()] {
        boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);

        let expected_offset = u32::from(read_ram_u16(&machine, VGA_METADATA_SAVE_POINTER_TABLE));
        let save_pointer = read_ram_u32(&machine, BDA_SAVE_POINTER);
        assert_eq!(save_pointer, (VGA_ROM_SEGMENT << 16) | expected_offset);

        let table = linear(save_pointer);
        let parameters_offset =
            u32::from(read_ram_u16(&machine, VGA_METADATA_VIDEO_PARAMETER_TABLE));
        assert_eq!(
            read_ram_u32(&machine, table),
            (VGA_ROM_SEGMENT << 16) | parameters_offset
        );
        assert_eq!(
            read_ram_u16(&machine, VGA_METADATA_VIDEO_PARAMETER_COUNT),
            29
        );

        // The dynamic save area, both character set override pointers, the
        // secondary save pointer table and the reserved bytes are null.
        for index in 1..5u32 {
            assert_eq!(
                read_ram_u32(&machine, table + index * 4),
                0,
                "dword {index}"
            );
        }
        for offset in 20..28u32 {
            assert_eq!(
                read_ram_u8(&machine, table + offset),
                0,
                "reserved {offset}"
            );
        }
    }
}

#[test]
fn published_parameter_table_holds_the_mode_03h_entry() {
    let mut machine = create_machine_dx66();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);

    let entry = parameter_table(&machine) + MODE_03H_INDEX * ENTRY_SIZE;
    let published: Vec<u8> = (0..ENTRY_SIZE)
        .map(|offset| read_ram_u8(&machine, entry + offset))
        .collect();

    assert_eq!(published, MODE_03H_ENTRY);
}

#[test]
fn mode_sets_leave_a_guest_save_pointer_alone() {
    let mut machine = create_machine_dx66();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);

    install_guest_save_pointer_table(&mut machine, 4, 0);
    for mode in [0x03u8, 0x12, 0x13] {
        inject_and_run(&mut machine, &set_mode_code(mode), &[], BUDGET);
        assert_eq!(read_ram_u8(&machine, BDA_VIDEO_MODE), mode);
        assert_eq!(
            read_ram_u32(&machine, BDA_SAVE_POINTER),
            GUEST_SAVE_POINTER_TABLE,
            "mode {mode:02X}h took the pointer back"
        );
    }
}

#[test]
fn alpha_override_replaces_the_rom_font_on_mode_set() {
    let mut machine = create_machine_dx66();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);

    let bitmaps = install_alpha_override(&mut machine, 8, 0xFF, &[0x03, 0xFF]);
    inject_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);

    for (index, expected) in bitmaps.iter().enumerate() {
        let code = OVERRIDE_FIRST_CODE + index as u32 / 8;
        let row = index as u32 % 8;
        assert_eq!(
            plane_2_byte(&machine, code, row),
            *expected,
            "glyph {code:02X} row {row}"
        );
    }
    // 400 scan lines of 8-line cells: 50 rows, and the cell height follows.
    assert_eq!(read_ram_u16(&machine, BDA_CHAR_HEIGHT), 8);
    assert_eq!(read_ram_u8(&machine, BDA_VIDEO_ROWS), 49);
}

#[test]
fn alpha_override_row_count_overrides_the_calculated_rows() {
    let mut machine = create_machine_dx66();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);

    install_alpha_override(&mut machine, 8, 25, &[0x03, 0xFF]);
    inject_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);

    assert_eq!(read_ram_u8(&machine, BDA_VIDEO_ROWS), 24);
    assert_eq!(read_ram_u16(&machine, BDA_CHAR_HEIGHT), 8);
}

#[test]
fn alpha_override_is_ignored_for_a_mode_outside_its_list() {
    let mut machine = create_machine_dx66();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);

    install_alpha_override(&mut machine, 8, 0xFF, &[0x12, 0xFF]);
    inject_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);

    // The ROM 8x16 font stays installed and the BDA keeps 16-line cells.
    assert_eq!(read_ram_u16(&machine, BDA_CHAR_HEIGHT), 16);
    assert_eq!(read_ram_u8(&machine, BDA_VIDEO_ROWS), 24);
    let font_offset = u32::from(read_ram_u16(&machine, VGA_ROM_BASE + 0x16));
    for row in 0..16u32 {
        let expected = read_ram_u8(
            &machine,
            VGA_ROM_BASE + font_offset + OVERRIDE_FIRST_CODE * 16 + row,
        );
        assert_eq!(plane_2_byte(&machine, OVERRIDE_FIRST_CODE, row), expected);
    }
}

#[test]
fn graphics_override_retargets_the_int43h_font_vector() {
    let mut machine = create_machine_dx66();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);

    install_guest_save_pointer_table(&mut machine, 12, GUEST_OVERRIDE_TABLE);
    write_bytes(
        &mut machine,
        GUEST_OVERRIDE_TABLE,
        &[
            0x19, // 25 displayed rows
            0x08, 0x00, // eight bytes per character
            0x78, 0x56, 0x34, 0x12, // font table at 1234:5678
            0x0D, 0xFF, // applicable modes
        ],
    );

    inject_and_run(&mut machine, &set_mode_code(0x0D), &[], BUDGET);

    assert_eq!(read_ram_u32(&machine, 0x43 * 4), 0x1234_5678);
    assert_eq!(read_ram_u16(&machine, BDA_CHAR_HEIGHT), 8);
    assert_eq!(read_ram_u8(&machine, BDA_VIDEO_ROWS), 24);
}

#[test]
fn null_save_pointer_leaves_the_mode_set_unchanged() {
    let mut machine = create_machine_dx66();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);
    write_bytes(&mut machine, BDA_SAVE_POINTER, &0u32.to_le_bytes());

    inject_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);

    assert_eq!(read_ram_u8(&machine, BDA_VIDEO_MODE), 0x03);
    assert_eq!(read_ram_u16(&machine, BDA_CHAR_HEIGHT), 16);
    assert_eq!(read_ram_u8(&machine, BDA_VIDEO_ROWS), 24);
}

#[test]
fn malformed_override_table_is_rejected() {
    let mut machine = create_machine_dx66();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);

    install_guest_save_pointer_table(&mut machine, 8, GUEST_OVERRIDE_TABLE);
    // Zero cell height, 65535 glyphs and a modes list without a terminator:
    // every field a guest can get wrong at once.
    write_bytes(
        &mut machine,
        GUEST_OVERRIDE_TABLE,
        &[
            0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x32, 0x00, 0x00, 0xFF, 0x03, 0x03, 0x03,
        ],
    );

    inject_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);

    assert_eq!(read_ram_u8(&machine, BDA_VIDEO_MODE), 0x03);
    assert_eq!(read_ram_u16(&machine, BDA_CHAR_HEIGHT), 16);
    assert_eq!(read_ram_u8(&machine, BDA_VIDEO_ROWS), 24);
}
