//! INT 10h AH=00h mode set: captured register files, per-mode BDA state,
//! font vectors, the plane 2 font upload and the no-clear bit. Expected
//! values were captured from the real AMI + ET4000AX BIOS.

use common::Bus;

use super::{
    RESULT, boot_and_run, create_machine_dx50,
    harness::{ModeVector, assert_vector_applied, read_back_vga_registers, text_cell},
    inject_and_run,
    mode_vectors::{
        MODE_0D, MODE_0E, MODE_0F, MODE_01, MODE_2E, MODE_2F, MODE_03, MODE_04, MODE_06, MODE_07,
        MODE_10, MODE_11, MODE_12, MODE_13, MODE_30,
    },
    read_ivt_vector, read_ram_u8, read_ram_u16,
};

/// Cycle budget for one injected INT 10h program.
const BUDGET: u64 = 8_000_000;

/// BIOS data area: current video mode.
const BDA_VIDEO_MODE: u32 = 0x449;
/// BIOS data area: text columns (word).
const BDA_VIDEO_COLUMNS: u32 = 0x44A;
/// BIOS data area: display page size in bytes (word).
const BDA_VIDEO_PAGE_SIZE: u32 = 0x44C;
/// BIOS data area: active page regen start offset (word).
const BDA_VIDEO_PAGE_START: u32 = 0x44E;
/// BIOS data area: cursor position of page zero (word).
const BDA_CURSOR_PAGE_0: u32 = 0x450;
/// BIOS data area: cursor shape (word).
const BDA_CURSOR_SHAPE: u32 = 0x460;
/// BIOS data area: active display page.
const BDA_ACTIVE_PAGE: u32 = 0x462;
/// BIOS data area: CRTC index port base (word).
const BDA_CRTC_BASE: u32 = 0x463;
/// BIOS data area: CGA mode select register image.
const BDA_MODE_SELECT: u32 = 0x465;
/// BIOS data area: CGA palette register image.
const BDA_CGA_PALETTE: u32 = 0x466;
/// BIOS data area: text rows minus one.
const BDA_VIDEO_ROWS: u32 = 0x484;
/// BIOS data area: character cell height (word).
const BDA_CHAR_HEIGHT: u32 = 0x485;
/// BIOS data area: video control bits.
const BDA_VIDEO_CONTROL: u32 = 0x487;
/// BIOS data area: video feature switches.
const BDA_VIDEO_SWITCHES: u32 = 0x488;
/// BIOS data area: video mode set control.
const BDA_MODESET_CONTROL: u32 = 0x489;
/// BIOS data area: display combination code table index.
const BDA_DCC_INDEX: u32 = 0x48A;

/// Physical base of the VGA BIOS ROM.
const VGA_ROM_BASE: u32 = 0xC0000;
/// Guest address of the VGA ROM metadata word: 8x8 font offset.
const VGA_METADATA_FONT_8X8: u32 = 0xC0010;
/// Guest address of the VGA ROM metadata word: 8x8 upper-half font offset.
const VGA_METADATA_FONT_8X8_UPPER: u32 = 0xC0012;
/// Guest address of the VGA ROM metadata word: 8x14 font offset.
const VGA_METADATA_FONT_8X14: u32 = 0xC0014;
/// Guest address of the VGA ROM metadata word: 8x16 font offset.
const VGA_METADATA_FONT_8X16: u32 = 0xC0016;
/// Real-mode segment of the VGA BIOS ROM.
const VGA_ROM_SEGMENT: u16 = 0xC000;

/// Builds `MOV AX, mode; INT 10h; HLT`.
fn set_mode_code(mode: u8) -> [u8; 6] {
    [0xB8, mode, 0x00, 0xCD, 0x10, 0xF4]
}

/// AH=0Fh: stores AX and BH.
#[rustfmt::skip]
const GET_MODE_CODE: &[u8] = &[
    0xB4, 0x0F,             // MOV AH, 0x0F
    0xCD, 0x10,             // INT 10h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0x88, 0x3E, 0x02, 0x06, // MOV [0x0602], BH
    0xF4,                   // HLT
];

/// AH=1Ah AL=00h: stores AX and BX.
#[rustfmt::skip]
const READ_DCC_CODE: &[u8] = &[
    0xB8, 0x00, 0x1A,       // MOV AX, 0x1A00
    0xBB, 0x00, 0x00,       // MOV BX, 0x0000
    0xCD, 0x10,             // INT 10h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0x89, 0x1E, 0x02, 0x06, // MOV [0x0602], BX
    0xF4,                   // HLT
];

#[test]
fn mode_set_programs_captured_register_files() {
    let cases: [(u8, &ModeVector, &str); 18] = [
        (0x00, &MODE_01, "mode 00h"),
        (0x01, &MODE_01, "mode 01h"),
        (0x02, &MODE_03, "mode 02h"),
        (0x03, &MODE_03, "mode 03h"),
        (0x04, &MODE_04, "mode 04h"),
        (0x05, &MODE_04, "mode 05h"),
        (0x06, &MODE_06, "mode 06h"),
        (0x07, &MODE_07, "mode 07h"),
        (0x0D, &MODE_0D, "mode 0Dh"),
        (0x0E, &MODE_0E, "mode 0Eh"),
        (0x0F, &MODE_0F, "mode 0Fh"),
        (0x10, &MODE_10, "mode 10h"),
        (0x11, &MODE_11, "mode 11h"),
        (0x12, &MODE_12, "mode 12h"),
        (0x13, &MODE_13, "mode 13h"),
        (0x2E, &MODE_2E, "mode 2Eh"),
        (0x2F, &MODE_2F, "mode 2Fh"),
        (0x30, &MODE_30, "mode 30h"),
    ];
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);
    for (mode, vector, label) in cases {
        inject_and_run(&mut machine, &set_mode_code(mode), &[], BUDGET);
        let actual = read_back_vga_registers(&mut machine.bus);
        assert_vector_applied(&actual, vector, label);
    }
}

/// Per-mode BDA expectations captured from the real BIOS.
struct BdaCase {
    mode: u8,
    columns: u16,
    page_size: u16,
    crtc_base: u16,
    mode_select: u8,
    cga_palette: u8,
    rows_minus_one: u8,
    char_height: u16,
    video_control: u8,
    switches: u8,
    text: bool,
}

#[rustfmt::skip]
const BDA_CASES: [BdaCase; 18] = [
    BdaCase { mode: 0x00, columns: 40, page_size: 0x0800, crtc_base: 0x3D4, mode_select: 0x2C, cga_palette: 0x30, rows_minus_one: 24, char_height: 16, video_control: 0x60, switches: 0x89, text: true },
    BdaCase { mode: 0x01, columns: 40, page_size: 0x0800, crtc_base: 0x3D4, mode_select: 0x28, cga_palette: 0x30, rows_minus_one: 24, char_height: 16, video_control: 0x60, switches: 0x89, text: true },
    BdaCase { mode: 0x02, columns: 80, page_size: 0x1000, crtc_base: 0x3D4, mode_select: 0x2D, cga_palette: 0x30, rows_minus_one: 24, char_height: 16, video_control: 0x60, switches: 0x89, text: true },
    BdaCase { mode: 0x03, columns: 80, page_size: 0x1000, crtc_base: 0x3D4, mode_select: 0x29, cga_palette: 0x30, rows_minus_one: 24, char_height: 16, video_control: 0x60, switches: 0x89, text: true },
    BdaCase { mode: 0x04, columns: 40, page_size: 0x4000, crtc_base: 0x3D4, mode_select: 0x2A, cga_palette: 0x30, rows_minus_one: 24, char_height: 8, video_control: 0x60, switches: 0x89, text: false },
    BdaCase { mode: 0x05, columns: 40, page_size: 0x4000, crtc_base: 0x3D4, mode_select: 0x2E, cga_palette: 0x30, rows_minus_one: 24, char_height: 8, video_control: 0x60, switches: 0x89, text: false },
    BdaCase { mode: 0x06, columns: 80, page_size: 0x4000, crtc_base: 0x3D4, mode_select: 0x1E, cga_palette: 0x3F, rows_minus_one: 24, char_height: 8, video_control: 0x60, switches: 0x89, text: false },
    BdaCase { mode: 0x07, columns: 80, page_size: 0x1000, crtc_base: 0x3B4, mode_select: 0x29, cga_palette: 0x30, rows_minus_one: 24, char_height: 16, video_control: 0x62, switches: 0x8B, text: true },
    BdaCase { mode: 0x0D, columns: 40, page_size: 0x2000, crtc_base: 0x3D4, mode_select: 0x29, cga_palette: 0x30, rows_minus_one: 24, char_height: 8, video_control: 0x60, switches: 0x89, text: false },
    BdaCase { mode: 0x0E, columns: 80, page_size: 0x4000, crtc_base: 0x3D4, mode_select: 0x29, cga_palette: 0x30, rows_minus_one: 24, char_height: 8, video_control: 0x60, switches: 0x89, text: false },
    BdaCase { mode: 0x0F, columns: 80, page_size: 0x8000, crtc_base: 0x3B4, mode_select: 0x29, cga_palette: 0x30, rows_minus_one: 24, char_height: 14, video_control: 0x62, switches: 0x8B, text: false },
    BdaCase { mode: 0x10, columns: 80, page_size: 0x8000, crtc_base: 0x3D4, mode_select: 0x29, cga_palette: 0x30, rows_minus_one: 24, char_height: 14, video_control: 0x60, switches: 0x89, text: false },
    BdaCase { mode: 0x11, columns: 80, page_size: 0xA000, crtc_base: 0x3D4, mode_select: 0x29, cga_palette: 0x30, rows_minus_one: 29, char_height: 16, video_control: 0x60, switches: 0x89, text: false },
    BdaCase { mode: 0x12, columns: 80, page_size: 0xA000, crtc_base: 0x3D4, mode_select: 0x29, cga_palette: 0x30, rows_minus_one: 29, char_height: 16, video_control: 0x60, switches: 0x89, text: false },
    BdaCase { mode: 0x13, columns: 40, page_size: 0x2000, crtc_base: 0x3D4, mode_select: 0x29, cga_palette: 0x30, rows_minus_one: 24, char_height: 8, video_control: 0x60, switches: 0x89, text: false },
    BdaCase { mode: 0x2E, columns: 80, page_size: 0x0000, crtc_base: 0x3D4, mode_select: 0x29, cga_palette: 0x30, rows_minus_one: 29, char_height: 16, video_control: 0x60, switches: 0x89, text: false },
    BdaCase { mode: 0x2F, columns: 80, page_size: 0x0000, crtc_base: 0x3D4, mode_select: 0x29, cga_palette: 0x30, rows_minus_one: 24, char_height: 16, video_control: 0x60, switches: 0x89, text: false },
    BdaCase { mode: 0x30, columns: 100, page_size: 0x0000, crtc_base: 0x3D4, mode_select: 0x29, cga_palette: 0x30, rows_minus_one: 36, char_height: 16, video_control: 0x60, switches: 0x89, text: false },
];

#[test]
fn mode_set_writes_bda_state() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);
    for case in BDA_CASES {
        inject_and_run(&mut machine, &set_mode_code(case.mode), &[], BUDGET);
        let label = format!("mode {:02X}h", case.mode);
        assert_eq!(
            read_ram_u8(&machine, BDA_VIDEO_MODE),
            case.mode,
            "{label}: 40:49"
        );
        assert_eq!(
            read_ram_u16(&machine, BDA_VIDEO_COLUMNS),
            case.columns,
            "{label}: 40:4A"
        );
        assert_eq!(
            read_ram_u16(&machine, BDA_VIDEO_PAGE_SIZE),
            case.page_size,
            "{label}: 40:4C"
        );
        assert_eq!(
            read_ram_u16(&machine, BDA_VIDEO_PAGE_START),
            0,
            "{label}: 40:4E"
        );
        assert_eq!(
            read_ram_u16(&machine, BDA_CURSOR_PAGE_0),
            0,
            "{label}: 40:50"
        );
        let cursor_shape = if case.text { 0x0D0E } else { 0x0000 };
        assert_eq!(
            read_ram_u16(&machine, BDA_CURSOR_SHAPE),
            cursor_shape,
            "{label}: 40:60"
        );
        assert_eq!(read_ram_u8(&machine, BDA_ACTIVE_PAGE), 0, "{label}: 40:62");
        assert_eq!(
            read_ram_u16(&machine, BDA_CRTC_BASE),
            case.crtc_base,
            "{label}: 40:63"
        );
        assert_eq!(
            read_ram_u8(&machine, BDA_MODE_SELECT),
            case.mode_select,
            "{label}: 40:65"
        );
        assert_eq!(
            read_ram_u8(&machine, BDA_CGA_PALETTE),
            case.cga_palette,
            "{label}: 40:66"
        );
        assert_eq!(
            read_ram_u8(&machine, BDA_VIDEO_ROWS),
            case.rows_minus_one,
            "{label}: 40:84"
        );
        assert_eq!(
            read_ram_u16(&machine, BDA_CHAR_HEIGHT),
            case.char_height,
            "{label}: 40:85"
        );
        assert_eq!(
            read_ram_u8(&machine, BDA_VIDEO_CONTROL),
            case.video_control,
            "{label}: 40:87"
        );
        assert_eq!(
            read_ram_u8(&machine, BDA_VIDEO_SWITCHES),
            case.switches,
            "{label}: 40:88"
        );
        assert_eq!(
            read_ram_u8(&machine, BDA_MODESET_CONTROL),
            0x51,
            "{label}: 40:89"
        );
        assert_eq!(read_ram_u8(&machine, BDA_DCC_INDEX), 0x0B, "{label}: 40:8A");
    }
}

#[test]
fn mode_set_installs_font_vectors() {
    let cases: [(u8, u32); 9] = [
        (0x03, VGA_METADATA_FONT_8X8),
        (0x04, VGA_METADATA_FONT_8X8),
        (0x0D, VGA_METADATA_FONT_8X8),
        (0x13, VGA_METADATA_FONT_8X8),
        (0x0F, VGA_METADATA_FONT_8X14),
        (0x10, VGA_METADATA_FONT_8X14),
        (0x11, VGA_METADATA_FONT_8X16),
        (0x12, VGA_METADATA_FONT_8X16),
        (0x2E, VGA_METADATA_FONT_8X16),
    ];
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);
    let upper_offset = read_ram_u16(&machine, VGA_METADATA_FONT_8X8_UPPER);
    assert_ne!(upper_offset, 0);
    for (mode, metadata_address) in cases {
        inject_and_run(&mut machine, &set_mode_code(mode), &[], BUDGET);
        let font_offset = read_ram_u16(&machine, metadata_address);
        assert_ne!(font_offset, 0, "mode {mode:02X}h: font metadata word");
        assert_eq!(
            read_ivt_vector(&machine, 0x43),
            (VGA_ROM_SEGMENT, font_offset),
            "mode {mode:02X}h: IVT 43h"
        );
        assert_eq!(
            read_ivt_vector(&machine, 0x1F),
            (VGA_ROM_SEGMENT, upper_offset),
            "mode {mode:02X}h: IVT 1Fh"
        );
    }
}

#[test]
fn text_mode_set_uploads_rom_font_to_plane_2() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);
    let font_offset = u32::from(read_ram_u16(&machine, VGA_METADATA_FONT_8X16));
    for code in 0..256u32 {
        for row in 0..16u32 {
            let expected = read_ram_u8(&machine, VGA_ROM_BASE + font_offset + code * 16 + row);
            let actual = machine.bus.vga().vram()[((code * 32 + row) * 4 + 2) as usize];
            assert_eq!(actual, expected, "glyph {code:02X} row {row}");
        }
    }
}

#[test]
fn no_clear_bit_preserves_regen_content() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);
    machine.bus.write_byte(0xB8000, b'Z');
    machine.bus.write_byte(0xB8001, 0x17);

    inject_and_run(&mut machine, &set_mode_code(0x83), &[], BUDGET);
    assert_eq!(text_cell(&machine, 0, 0x1000, 80, 0, 0), (b'Z', 0x17));
    assert_eq!(read_ram_u8(&machine, BDA_VIDEO_CONTROL) & 0x80, 0x80);

    // AH=0Fh reports the no-clear bit on top of the mode number.
    inject_and_run(&mut machine, GET_MODE_CODE, &[], BUDGET);
    assert_eq!(read_ram_u16(&machine, RESULT), 0x5083);
    assert_eq!(read_ram_u8(&machine, RESULT + 2), 0x00);

    inject_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);
    assert_eq!(text_cell(&machine, 0, 0x1000, 80, 0, 0), (0x20, 0x07));
    assert_eq!(read_ram_u8(&machine, BDA_VIDEO_CONTROL) & 0x80, 0x00);
}

#[test]
fn invalid_mode_is_ignored() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);
    machine.bus.write_byte(0xB8000, b'K');

    inject_and_run(&mut machine, &set_mode_code(0x08), &[], BUDGET);
    assert_eq!(read_ram_u8(&machine, BDA_VIDEO_MODE), 0x03);
    assert_eq!(text_cell(&machine, 0, 0x1000, 80, 0, 0).0, b'K');
    let actual = read_back_vga_registers(&mut machine.bus);
    assert_vector_applied(&actual, &MODE_03, "mode 03h after invalid set");
}

#[test]
fn display_combination_code_reads_vga_color() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, READ_DCC_CODE, &[], BUDGET);
    assert_eq!(read_ram_u16(&machine, RESULT) as u8, 0x1A);
    assert_eq!(read_ram_u16(&machine, RESULT + 2), 0x0008);
}

/// Guest address of the VGA ROM metadata word: AH=1Bh static table offset.
const VGA_METADATA_FUNCTIONALITY: u32 = 0xC0018;

/// The AH=1Bh static functionality table, captured from the real BIOS.
const STATIC_FUNCTIONALITY_TABLE: [u8; 16] = [
    0xFF, 0xE0, 0x0F, 0x00, 0x00, 0x00, 0x00, 0x07, 0x02, 0x08, 0xFF, 0x0E, 0x00, 0x00, 0x3F, 0x00,
];

/// Builds a program that sets the mode, fills the buffer at 0300:0100 with
/// 0xAA, calls AH=1Bh and stores AX.
fn functionality_state_code(mode: u8) -> Vec<u8> {
    #[rustfmt::skip]
    let code = vec![
        0xB8, mode, 0x00,       // MOV AX, mode
        0xCD, 0x10,             // INT 10h
        0xB8, 0x00, 0x03,       // MOV AX, 0x0300
        0x8E, 0xC0,             // MOV ES, AX
        0xBF, 0x00, 0x01,       // MOV DI, 0x0100
        0xB9, 0x40, 0x00,       // MOV CX, 0x0040
        0xB0, 0xAA,             // MOV AL, 0xAA
        0xF3, 0xAA,             // REP STOSB
        0xBF, 0x00, 0x01,       // MOV DI, 0x0100
        0xB8, 0x2F, 0x1B,       // MOV AX, 0x1B2F
        0xBB, 0x00, 0x00,       // MOV BX, 0x0000
        0xCD, 0x10,             // INT 10h
        0xA3, 0x00, 0x06,       // MOV [0x0600], AX
        0xF4,                   // HLT
    ];
    code
}

/// The AH=1Bh state buffers (offsets 04h-3Fh) captured from the real BIOS
/// for the mode sequence 03h, 04h, 12h, 13h, 30h. The bytes at 20h/21h are
/// the BDA 40:65/40:66 images, which the extended mode sets leave at the
/// values mode 04h wrote.
#[rustfmt::skip]
const FUNCTIONALITY_CASES: [(u8, [u8; 60]); 5] = [
    (0x03, [
        0x03, 0x50, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0E, 0x0D, 0x00, 0xD4, 0x03,
        0x29, 0x30, 0x19, 0x10, 0x00, 0x08, 0x00, 0x10, 0x00, 0x08, 0x02, 0x00, 0x00, 0x31, 0x00, 0x00,
        0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]),
    (0x04, [
        0x04, 0x28, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xD4, 0x03,
        0x2A, 0x30, 0x19, 0x08, 0x00, 0x08, 0x00, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00,
        0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]),
    (0x12, [
        0x12, 0x50, 0x00, 0x00, 0xA0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xD4, 0x03,
        0x2A, 0x30, 0x1E, 0x10, 0x00, 0x08, 0x00, 0x10, 0x00, 0x01, 0x03, 0x00, 0x00, 0x11, 0x00, 0x00,
        0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]),
    (0x13, [
        0x13, 0x28, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xD4, 0x03,
        0x2A, 0x30, 0x19, 0x08, 0x00, 0x08, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00,
        0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]),
    (0x30, [
        0x30, 0x64, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xD4, 0x03,
        0x2A, 0x30, 0x25, 0x10, 0x00, 0x08, 0x00, 0x00, 0x01, 0x01, 0x05, 0x00, 0x00, 0x11, 0x00, 0x00,
        0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]),
];

#[test]
fn functionality_state_matches_capture() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);

    let table_offset = read_ram_u16(&machine, VGA_METADATA_FUNCTIONALITY);
    assert_ne!(table_offset, 0, "static table metadata word");
    for (index, &expected) in STATIC_FUNCTIONALITY_TABLE.iter().enumerate() {
        assert_eq!(
            read_ram_u8(
                &machine,
                VGA_ROM_BASE + u32::from(table_offset) + index as u32
            ),
            expected,
            "static table byte {index}"
        );
    }

    for (mode, expected) in FUNCTIONALITY_CASES {
        inject_and_run(&mut machine, &functionality_state_code(mode), &[], BUDGET);
        assert_eq!(
            read_ram_u16(&machine, RESULT) as u8,
            0x1B,
            "mode {mode:02X}h: AL"
        );
        assert_eq!(
            read_ram_u16(&machine, 0x3100),
            table_offset,
            "mode {mode:02X}h: static table offset"
        );
        assert_eq!(
            read_ram_u16(&machine, 0x3102),
            0xC000,
            "mode {mode:02X}h: static table segment"
        );
        for (index, &value) in expected.iter().enumerate() {
            assert_eq!(
                read_ram_u8(&machine, 0x3104 + index as u32),
                value,
                "mode {mode:02X}h: buffer byte {:02X}h",
                index + 4
            );
        }
    }
}

/// AH=FEh with the carry pre-set: the real BIOS returns unknown functions
/// with every register and the flags untouched.
#[rustfmt::skip]
const UNKNOWN_FUNCTION_CODE: &[u8] = &[
    0xB8, 0x00, 0xB8,       // MOV AX, 0xB800
    0x8E, 0xC0,             // MOV ES, AX
    0xBF, 0x34, 0x12,       // MOV DI, 0x1234
    0xB8, 0x03, 0xFE,       // MOV AX, 0xFE03
    0xF9,                   // STC
    0xCD, 0x10,             // INT 10h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x02, 0x06,       // MOV [0x0602], AX
    0x8C, 0xC0,             // MOV AX, ES
    0xA3, 0x04, 0x06,       // MOV [0x0604], AX
    0x89, 0x3E, 0x06, 0x06, // MOV [0x0606], DI
    0xF4,                   // HLT
];

#[test]
fn unknown_function_preserves_registers_and_flags() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, UNKNOWN_FUNCTION_CODE, &[], BUDGET);
    assert_eq!(read_ram_u16(&machine, RESULT), 0xFE03, "AX untouched");
    assert_eq!(
        read_ram_u16(&machine, RESULT + 2) & 0x0001,
        0x0001,
        "caller carry flag preserved"
    );
    assert_eq!(read_ram_u16(&machine, RESULT + 4), 0xB800, "ES untouched");
    assert_eq!(read_ram_u16(&machine, RESULT + 6), 0x1234, "DI untouched");
}

#[test]
fn extended_mode_set_keeps_cga_register_images() {
    let mut machine = create_machine_dx50();
    // Mode 06h writes 40:65/40:66, the mode 12h set must leave both alone.
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x06, 0x00,       // MOV AX, 0x0006
        0xCD, 0x10,             // INT 10h
        0xB8, 0x12, 0x00,       // MOV AX, 0x0012
        0xCD, 0x10,             // INT 10h
        0xF4,                   // HLT
    ];
    boot_and_run(&mut machine, code, &[], BUDGET);
    assert_eq!(read_ram_u8(&machine, BDA_MODE_SELECT), 0x1E);
    assert_eq!(read_ram_u8(&machine, BDA_CGA_PALETTE), 0x3F);
}
