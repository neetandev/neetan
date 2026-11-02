//! ROM fonts: the AX=1130h font pointers, the alternate glyph tables, the
//! alternate glyphs a 9 dot text mode set loads, and the system BIOS 8x8 font
//! at F000:FA6E. Expected behavior was captured from the real AMI + ET4000AX
//! BIOS.

use super::{RESULT, boot_and_run, create_machine_dx50, inject_and_run, read_ram_u8, read_ram_u16};

/// Cycle budget for one injected INT 10h program.
const BUDGET: u64 = 8_000_000;
/// Physical base of the VGA BIOS ROM.
const VGA_ROM_BASE: u32 = 0xC0000;
/// Real-mode segment of the VGA BIOS ROM.
const VGA_ROM_SEGMENT: u16 = 0xC000;
/// Guest address of the VGA ROM metadata word: 8x8 font offset.
const VGA_METADATA_FONT_8X8: u32 = 0xC0010;
/// Guest address of the VGA ROM metadata word: 8x8 upper-half font offset.
const VGA_METADATA_FONT_8X8_UPPER: u32 = 0xC0012;
/// Guest address of the VGA ROM metadata word: 8x14 font offset.
const VGA_METADATA_FONT_8X14: u32 = 0xC0014;
/// Guest address of the VGA ROM metadata word: 8x16 font offset.
const VGA_METADATA_FONT_8X16: u32 = 0xC0016;
/// Guest address of the VGA ROM metadata word: 9x14 alternate table offset.
const VGA_METADATA_FONT_9X14_ALTERNATE: u32 = 0xC0020;
/// Guest address of the VGA ROM metadata word: 9x16 alternate table offset.
const VGA_METADATA_FONT_9X16_ALTERNATE: u32 = 0xC0022;
/// Physical address of the system BIOS 8x8 font (F000:FA6E).
const SYSTEM_FONT_8X8: u32 = 0xFFA6E;
/// Codes of the 9x14 alternate glyphs of the ET4000 BIOS.
const ALTERNATE_9X14_CODES: [u8; 20] = [
    0x1D, 0x22, 0x2B, 0x2D, 0x4D, 0x54, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x6D, 0x76, 0x77, 0x91, 0x9B,
    0x9D, 0x9E, 0xF1, 0xF6,
];
/// Codes of the 9x16 alternate glyphs of the ET4000 BIOS.
const ALTERNATE_9X16_CODES: [u8; 19] = [
    0x1D, 0x30, 0x4D, 0x54, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x6D, 0x76, 0x77, 0x78, 0x91, 0x9B, 0x9D,
    0x9E, 0xAB, 0xAC,
];

/// Builds `MOV AX, mode; INT 10h; HLT`.
fn set_mode_code(mode: u8) -> [u8; 6] {
    [0xB8, mode, 0x00, 0xCD, 0x10, 0xF4]
}

/// Builds AX=1130h with the given BH and stores BP, ES, CX and DL at RESULT.
#[rustfmt::skip]
fn font_information_code(bh: u8) -> [u8; 24] {
    [
        0xB8, 0x30, 0x11,       // MOV AX, 0x1130
        0xB7, bh,               // MOV BH, bh
        0xCD, 0x10,             // INT 10h
        0x89, 0x2E, 0x00, 0x06, // MOV [0x0600], BP
        0x8C, 0x06, 0x02, 0x06, // MOV [0x0602], ES
        0x89, 0x0E, 0x04, 0x06, // MOV [0x0604], CX
        0x88, 0x16, 0x06, 0x06, // MOV [0x0606], DL
        0xF4,                   // HLT
    ]
}

/// Reads a glyph row of the character generator in plane 2.
fn plane_2_row(machine: &machine_at::AtMachine<common::NoTrace>, code: u32, row: u32) -> u8 {
    machine.bus.vga().vram()[((code * 32 + row) * 4 + 2) as usize]
}

/// Reads the code bytes of an alternate table and checks its terminator.
fn alternate_codes(
    machine: &machine_at::AtMachine<common::NoTrace>,
    offset: u16,
    height: u32,
) -> Vec<u8> {
    let mut codes = Vec::new();
    let mut entry = VGA_ROM_BASE + u32::from(offset);
    loop {
        let code = read_ram_u8(machine, entry);
        if code == 0 {
            return codes;
        }
        codes.push(code);
        entry += 1 + height;
        assert!(codes.len() <= 256, "alternate table without terminator");
    }
}

/// Returns the 8x16 glyph a 9 dot text mode set loads for a code: the 9x16
/// alternate glyph when the table holds one, the 8x16 glyph otherwise.
fn expected_9_dot_glyph(machine: &machine_at::AtMachine<common::NoTrace>, code: u32) -> Vec<u8> {
    let height = 16;
    let font = VGA_ROM_BASE + u32::from(read_ram_u16(machine, VGA_METADATA_FONT_8X16));
    let mut glyph: Vec<u8> = (0..height)
        .map(|row| read_ram_u8(machine, font + code * height + row))
        .collect();
    let mut entry =
        VGA_ROM_BASE + u32::from(read_ram_u16(machine, VGA_METADATA_FONT_9X16_ALTERNATE));
    while read_ram_u8(machine, entry) != 0 {
        if u32::from(read_ram_u8(machine, entry)) == code {
            glyph = (0..height)
                .map(|row| read_ram_u8(machine, entry + 1 + row))
                .collect();
        }
        entry += 1 + height;
    }
    glyph
}

/// Checks that plane 2 holds every glyph a 9 dot text mode set loads.
fn assert_9_dot_rom_font(machine: &machine_at::AtMachine<common::NoTrace>, label: &str) {
    for code in 0..256u32 {
        let expected = expected_9_dot_glyph(machine, code);
        let actual: Vec<u8> = (0..16).map(|row| plane_2_row(machine, code, row)).collect();
        assert_eq!(actual, expected, "{label}: glyph {code:02X}h");
    }
}

#[test]
fn font_information_returns_every_rom_table() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);
    let metadata = |address| read_ram_u16(&machine, address);
    let cases: [(u8, u16); 8] = [
        (0x00, metadata(VGA_METADATA_FONT_8X8_UPPER)),
        (0x01, metadata(VGA_METADATA_FONT_8X8)),
        (0x02, metadata(VGA_METADATA_FONT_8X14)),
        (0x03, metadata(VGA_METADATA_FONT_8X8)),
        (0x04, metadata(VGA_METADATA_FONT_8X8_UPPER)),
        (0x05, metadata(VGA_METADATA_FONT_9X14_ALTERNATE)),
        (0x06, metadata(VGA_METADATA_FONT_8X16)),
        (0x07, metadata(VGA_METADATA_FONT_9X16_ALTERNATE)),
    ];
    for (bh, offset) in cases {
        assert_ne!(offset, 0, "BH={bh:02X}h: metadata word");
        inject_and_run(&mut machine, &font_information_code(bh), &[], BUDGET);
        assert_eq!(read_ram_u16(&machine, RESULT), offset, "BH={bh:02X}h: BP");
        assert_eq!(
            read_ram_u16(&machine, RESULT + 2),
            VGA_ROM_SEGMENT,
            "BH={bh:02X}h: ES"
        );
        assert_eq!(read_ram_u16(&machine, RESULT + 4), 16, "BH={bh:02X}h: CX");
        assert_eq!(read_ram_u8(&machine, RESULT + 6), 24, "BH={bh:02X}h: DL");
    }
}

#[test]
fn alternate_tables_hold_the_et4000_codes() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], BUDGET);
    let offset = read_ram_u16(&machine, VGA_METADATA_FONT_9X14_ALTERNATE);
    assert_eq!(alternate_codes(&machine, offset, 14), ALTERNATE_9X14_CODES);
    let offset = read_ram_u16(&machine, VGA_METADATA_FONT_9X16_ALTERNATE);
    assert_eq!(alternate_codes(&machine, offset, 16), ALTERNATE_9X16_CODES);
}

#[test]
fn nine_dot_text_mode_set_loads_the_alternate_glyphs() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], BUDGET);
    for mode in [0x00, 0x01, 0x02, 0x03, 0x07] {
        inject_and_run(&mut machine, &set_mode_code(mode), &[], BUDGET);
        assert_9_dot_rom_font(&machine, &format!("mode {mode:02X}h"));
    }
}

/// Builds a mode 03h set followed by the AH=11h ROM font load in `al` with
/// BL=0.
#[rustfmt::skip]
fn mode_03_then_font_load_code(al: u8) -> [u8; 13] {
    [
        0xB8, 0x03, 0x00,       // MOV AX, 0x0003
        0xCD, 0x10,             // INT 10h
        0xB8, al, 0x11,         // MOV AX, 0x11xx
        0xB3, 0x00,             // MOV BL, 0x00
        0xCD, 0x10,             // INT 10h
        0xF4,                   // HLT
    ]
}

/// Checks that plane 2 holds every glyph of a plain ROM font.
fn assert_plain_rom_font(
    machine: &machine_at::AtMachine<common::NoTrace>,
    metadata: u32,
    height: u32,
    label: &str,
) {
    let font = VGA_ROM_BASE + u32::from(read_ram_u16(machine, metadata));
    for code in 0..256u32 {
        for row in 0..height {
            assert_eq!(
                plane_2_row(machine, code, row),
                read_ram_u8(machine, font + code * height + row),
                "{label}: glyph {code:02X}h row {row}"
            );
        }
    }
}

#[test]
fn rom_8x16_font_loads_keep_the_plain_glyphs() {
    for al in [0x04, 0x14] {
        let mut machine = create_machine_dx50();
        boot_and_run(&mut machine, &mode_03_then_font_load_code(al), &[], BUDGET);
        let label = format!("AX=11{al:02X}h");
        assert_plain_rom_font(&machine, VGA_METADATA_FONT_8X16, 16, &label);
    }
}

#[test]
fn rom_8x14_font_load_has_no_9x14_alternates() {
    let mut machine = create_machine_dx50();
    boot_and_run(
        &mut machine,
        &mode_03_then_font_load_code(0x01),
        &[],
        BUDGET,
    );
    assert_plain_rom_font(&machine, VGA_METADATA_FONT_8X14, 14, "AX=1101h");
}

#[test]
fn mode_set_after_a_rom_font_load_restores_the_alternates() {
    let mut machine = create_machine_dx50();
    boot_and_run(
        &mut machine,
        &mode_03_then_font_load_code(0x04),
        &[],
        BUDGET,
    );
    inject_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);
    assert_9_dot_rom_font(&machine, "mode 03h");
}

#[test]
fn system_bios_holds_the_8x8_font_at_fa6e() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], BUDGET);
    let glyph = |code: u32| -> Vec<u8> {
        (0..8)
            .map(|row| read_ram_u8(&machine, SYSTEM_FONT_8X8 + code * 8 + row))
            .collect()
    };
    assert_eq!(glyph(0x00), [0x00; 8]);
    assert_eq!(
        glyph(0x41),
        [0x38, 0x6C, 0xC6, 0xC6, 0xFE, 0xC6, 0xC6, 0x00]
    );
    assert_eq!(
        glyph(0x7F),
        [0x10, 0x38, 0x6C, 0xC6, 0xC6, 0xC6, 0xFE, 0x00]
    );
}
