//! INT 10h palette services: the ATC and DAC services of AH=10h, the CGA
//! compatibility interface AH=0Bh and the AH=12h alternate select flags.

use super::{
    RESULT, boot_and_run, create_machine_dx50, inject_and_run, read_ram_u8, read_ram_u16,
    write_bytes,
};

/// Cycle budget for one injected INT 10h program.
const BUDGET: u64 = 8_000_000;

/// BIOS data area: CGA palette register image.
const BDA_CGA_PALETTE: u32 = 0x466;
/// BIOS data area: video control bits.
const BDA_VIDEO_CONTROL: u32 = 0x487;
/// BIOS data area: video mode set control.
const BDA_MODESET_CONTROL: u32 = 0x489;

/// Builds `MOV AX, mode; INT 10h; HLT`.
fn set_mode_code(mode: u8) -> [u8; 6] {
    [0xB8, mode, 0x00, 0xCD, 0x10, 0xF4]
}

/// Mode 03h, AH=10h AL=00h sets ATC 0 to 0x2A, AL=07h reads it back.
#[rustfmt::skip]
const ATC_SINGLE_CODE: &[u8] = &[
    0xB8, 0x03, 0x00,       // MOV AX, 0x0003
    0xCD, 0x10,             // INT 10h
    0xB8, 0x00, 0x10,       // MOV AX, 0x1000
    0xBB, 0x00, 0x2A,       // MOV BX, 0x2A00
    0xCD, 0x10,             // INT 10h
    0xB8, 0x07, 0x10,       // MOV AX, 0x1007
    0xBB, 0x00, 0x00,       // MOV BX, 0x0000
    0xCD, 0x10,             // INT 10h
    0x88, 0x3E, 0x00, 0x06, // MOV [0x0600], BH
    0xF4,                   // HLT
];

#[test]
fn atc_single_register_set_and_read() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, ATC_SINGLE_CODE, &[], BUDGET);
    assert_eq!(machine.bus.vga().atc[0x00], 0x2A);
    assert_eq!(read_ram_u8(&machine, RESULT), 0x2A);
}

/// Mode 03h, AH=10h AL=02h loads ATC 0-15 plus overscan from 0x3000, AL=09h
/// stores them to 0x3020.
#[rustfmt::skip]
const ATC_BLOCK_CODE: &[u8] = &[
    0xB8, 0x03, 0x00,       // MOV AX, 0x0003
    0xCD, 0x10,             // INT 10h
    0xB8, 0x00, 0x03,       // MOV AX, 0x0300
    0x8E, 0xC0,             // MOV ES, AX
    0xBA, 0x00, 0x00,       // MOV DX, 0x0000
    0xB8, 0x02, 0x10,       // MOV AX, 0x1002
    0xCD, 0x10,             // INT 10h
    0xBA, 0x20, 0x00,       // MOV DX, 0x0020
    0xB8, 0x09, 0x10,       // MOV AX, 0x1009
    0xCD, 0x10,             // INT 10h
    0xF4,                   // HLT
];

#[test]
fn atc_block_load_and_store() {
    let block: [u8; 17] = [
        0x01, 0x04, 0x07, 0x0A, 0x0D, 0x10, 0x13, 0x16, 0x19, 0x1C, 0x1F, 0x22, 0x25, 0x28, 0x2B,
        0x2E, 0x31,
    ];
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);
    write_bytes(&mut machine, 0x3000, &block);
    inject_and_run(&mut machine, ATC_BLOCK_CODE, &[], BUDGET);
    for (index, &expected) in block[..16].iter().enumerate() {
        assert_eq!(machine.bus.vga().atc[index], expected, "ATC {index:02X}");
    }
    assert_eq!(machine.bus.vga().atc[0x11], block[16], "overscan");
    for (index, &expected) in block.iter().enumerate() {
        assert_eq!(
            read_ram_u8(&machine, 0x3020 + index as u32),
            expected,
            "stored byte {index}"
        );
    }
}

#[test]
fn blink_bit_toggle() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);
    // Mode 03h enables blink (ATC 10h = 0x0C).
    assert_eq!(machine.bus.vga().atc[0x10], 0x0C);

    #[rustfmt::skip]
    let blink_off: &[u8] = &[
        0xB8, 0x03, 0x10,       // MOV AX, 0x1003
        0xBB, 0x00, 0x00,       // MOV BX, 0x0000
        0xCD, 0x10,             // INT 10h
        0xF4,                   // HLT
    ];
    inject_and_run(&mut machine, blink_off, &[], BUDGET);
    assert_eq!(machine.bus.vga().atc[0x10], 0x04);

    #[rustfmt::skip]
    let blink_on: &[u8] = &[
        0xB8, 0x03, 0x10,       // MOV AX, 0x1003
        0xBB, 0x01, 0x00,       // MOV BX, 0x0001
        0xCD, 0x10,             // INT 10h
        0xF4,                   // HLT
    ];
    inject_and_run(&mut machine, blink_on, &[], BUDGET);
    assert_eq!(machine.bus.vga().atc[0x10], 0x0C);
}

/// Mode 03h, AH=10h AL=10h sets DAC 5, AL=15h reads it back into DH/CH/CL.
#[rustfmt::skip]
const DAC_SINGLE_CODE: &[u8] = &[
    0xB8, 0x03, 0x00,       // MOV AX, 0x0003
    0xCD, 0x10,             // INT 10h
    0xB8, 0x10, 0x10,       // MOV AX, 0x1010
    0xBB, 0x05, 0x00,       // MOV BX, 0x0005
    0xB6, 0x15,             // MOV DH, 0x15
    0xB5, 0x2A,             // MOV CH, 0x2A
    0xB1, 0x3F,             // MOV CL, 0x3F
    0xCD, 0x10,             // INT 10h
    0xB8, 0x15, 0x10,       // MOV AX, 0x1015
    0xBB, 0x05, 0x00,       // MOV BX, 0x0005
    0xCD, 0x10,             // INT 10h
    0x88, 0x36, 0x00, 0x06, // MOV [0x0600], DH
    0x89, 0x0E, 0x02, 0x06, // MOV [0x0602], CX
    0xF4,                   // HLT
];

#[test]
fn dac_single_entry_set_and_read() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, DAC_SINGLE_CODE, &[], BUDGET);
    assert_eq!(machine.bus.vga().dac[5], [0x15, 0x2A, 0x3F]);
    assert_eq!(read_ram_u8(&machine, RESULT), 0x15);
    assert_eq!(read_ram_u16(&machine, RESULT + 2), 0x2A3F);
}

/// Mode 03h, AH=10h AL=12h loads four DAC entries at index 0x10 from 0x3000,
/// AL=17h reads them back to 0x3020.
#[rustfmt::skip]
const DAC_BLOCK_CODE: &[u8] = &[
    0xB8, 0x03, 0x00,       // MOV AX, 0x0003
    0xCD, 0x10,             // INT 10h
    0xB8, 0x00, 0x03,       // MOV AX, 0x0300
    0x8E, 0xC0,             // MOV ES, AX
    0xBA, 0x00, 0x00,       // MOV DX, 0x0000
    0xBB, 0x10, 0x00,       // MOV BX, 0x0010
    0xB9, 0x04, 0x00,       // MOV CX, 0x0004
    0xB8, 0x12, 0x10,       // MOV AX, 0x1012
    0xCD, 0x10,             // INT 10h
    0xBA, 0x20, 0x00,       // MOV DX, 0x0020
    0xBB, 0x10, 0x00,       // MOV BX, 0x0010
    0xB9, 0x04, 0x00,       // MOV CX, 0x0004
    0xB8, 0x17, 0x10,       // MOV AX, 0x1017
    0xCD, 0x10,             // INT 10h
    0xF4,                   // HLT
];

#[test]
fn dac_block_load_and_read() {
    let block: [u8; 12] = [
        0x01, 0x02, 0x03, 0x11, 0x12, 0x13, 0x21, 0x22, 0x23, 0x31, 0x32, 0x33,
    ];
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);
    write_bytes(&mut machine, 0x3000, &block);
    inject_and_run(&mut machine, DAC_BLOCK_CODE, &[], BUDGET);
    for entry in 0..4usize {
        assert_eq!(
            machine.bus.vga().dac[0x10 + entry],
            [block[entry * 3], block[entry * 3 + 1], block[entry * 3 + 2]],
            "DAC entry {entry}"
        );
    }
    for (index, &expected) in block.iter().enumerate() {
        assert_eq!(
            read_ram_u8(&machine, 0x3020 + index as u32),
            expected,
            "read-back byte {index}"
        );
    }
}

/// Mode 03h, AH=10h AL=13h selects paging mode 1 and page 3, AL=1Ah reads
/// the page state into BL/BH.
#[rustfmt::skip]
const COLOR_PAGE_CODE: &[u8] = &[
    0xB8, 0x03, 0x00,       // MOV AX, 0x0003
    0xCD, 0x10,             // INT 10h
    0xB8, 0x13, 0x10,       // MOV AX, 0x1013
    0xBB, 0x00, 0x01,       // MOV BX, 0x0100
    0xCD, 0x10,             // INT 10h
    0xB8, 0x13, 0x10,       // MOV AX, 0x1013
    0xBB, 0x01, 0x03,       // MOV BX, 0x0301
    0xCD, 0x10,             // INT 10h
    0xB8, 0x1A, 0x10,       // MOV AX, 0x101A
    0xCD, 0x10,             // INT 10h
    0x89, 0x1E, 0x00, 0x06, // MOV [0x0600], BX
    0xF4,                   // HLT
];

#[test]
fn color_select_and_page_state() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, COLOR_PAGE_CODE, &[], BUDGET);
    assert_eq!(machine.bus.vga().atc[0x10] & 0x80, 0x80);
    assert_eq!(machine.bus.vga().atc[0x14] & 0x0F, 0x03);
    assert_eq!(read_ram_u16(&machine, RESULT), 0x0301);
}

#[test]
fn cga_palette_interface() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &set_mode_code(0x04), &[], BUDGET);
    assert_eq!(read_ram_u8(&machine, BDA_CGA_PALETTE), 0x30);

    // BH=1 BL=0: green/red/brown palette, 40:66 bit 5 clear.
    #[rustfmt::skip]
    let select_even: &[u8] = &[
        0xB4, 0x0B,             // MOV AH, 0x0B
        0xB7, 0x01,             // MOV BH, 0x01
        0xB3, 0x00,             // MOV BL, 0x00
        0xCD, 0x10,             // INT 10h
        0xF4,                   // HLT
    ];
    inject_and_run(&mut machine, select_even, &[], BUDGET);
    assert_eq!(machine.bus.vga().atc[1], 0x12);
    assert_eq!(machine.bus.vga().atc[2], 0x14);
    assert_eq!(machine.bus.vga().atc[3], 0x16);
    assert_eq!(read_ram_u8(&machine, BDA_CGA_PALETTE), 0x10);

    // BH=1 BL=1: cyan/magenta/white palette, 40:66 bit 5 set.
    #[rustfmt::skip]
    let select_odd: &[u8] = &[
        0xB4, 0x0B,             // MOV AH, 0x0B
        0xB7, 0x01,             // MOV BH, 0x01
        0xB3, 0x01,             // MOV BL, 0x01
        0xCD, 0x10,             // INT 10h
        0xF4,                   // HLT
    ];
    inject_and_run(&mut machine, select_odd, &[], BUDGET);
    assert_eq!(machine.bus.vga().atc[1], 0x13);
    assert_eq!(machine.bus.vga().atc[2], 0x15);
    assert_eq!(machine.bus.vga().atc[3], 0x17);
    assert_eq!(read_ram_u8(&machine, BDA_CGA_PALETTE), 0x30);

    // BH=0: background and border color 5, palette entry zero recolored.
    #[rustfmt::skip]
    let set_background: &[u8] = &[
        0xB4, 0x0B,             // MOV AH, 0x0B
        0xB7, 0x00,             // MOV BH, 0x00
        0xB3, 0x05,             // MOV BL, 0x05
        0xCD, 0x10,             // INT 10h
        0xF4,                   // HLT
    ];
    inject_and_run(&mut machine, set_background, &[], BUDGET);
    assert_eq!(machine.bus.vga().atc[0x11], 0x05);
    assert_eq!(machine.bus.vga().atc[0x00], 0x05);
    assert_eq!(read_ram_u8(&machine, BDA_CGA_PALETTE), 0x25);
}

/// Mode 03h, AH=12h BL=10h EGA information, then the BL=30h/31h/33h/34h
/// flag services storing each returned AL.
#[rustfmt::skip]
const ALTERNATE_SELECT_CODE: &[u8] = &[
    0xB8, 0x03, 0x00,       // MOV AX, 0x0003
    0xCD, 0x10,             // INT 10h
    0xB8, 0x00, 0x12,       // MOV AX, 0x1200
    0xBB, 0x10, 0x00,       // MOV BX, 0x0010
    0xCD, 0x10,             // INT 10h
    0x89, 0x1E, 0x04, 0x06, // MOV [0x0604], BX
    0x89, 0x0E, 0x06, 0x06, // MOV [0x0606], CX
    0xB8, 0x00, 0x12,       // MOV AX, 0x1200 (200 scan lines)
    0xBB, 0x30, 0x00,       // MOV BX, 0x0030
    0xCD, 0x10,             // INT 10h
    0xA2, 0x00, 0x06,       // MOV [0x0600], AL
    0xB8, 0x01, 0x12,       // MOV AX, 0x1201 (palette load disable)
    0xBB, 0x31, 0x00,       // MOV BX, 0x0031
    0xCD, 0x10,             // INT 10h
    0xA2, 0x01, 0x06,       // MOV [0x0601], AL
    0xB8, 0x00, 0x12,       // MOV AX, 0x1200 (gray-scale summing enable)
    0xBB, 0x33, 0x00,       // MOV BX, 0x0033
    0xCD, 0x10,             // INT 10h
    0xA2, 0x02, 0x06,       // MOV [0x0602], AL
    0xB8, 0x01, 0x12,       // MOV AX, 0x1201 (cursor emulation on)
    0xBB, 0x34, 0x00,       // MOV BX, 0x0034
    0xCD, 0x10,             // INT 10h
    0xA2, 0x03, 0x06,       // MOV [0x0603], AL
    0xF4,                   // HLT
];

#[test]
fn alternate_select_info_and_flags() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, ALTERNATE_SELECT_CODE, &[], BUDGET);
    // BL=10h: color adapter, 256 KiB, switches from BDA 40:88.
    assert_eq!(read_ram_u16(&machine, RESULT + 4), 0x0003);
    assert_eq!(read_ram_u16(&machine, RESULT + 6), 0x0009);
    // Every flag service returned AL=12h.
    for offset in 0..4u32 {
        assert_eq!(read_ram_u8(&machine, RESULT + offset), 0x12, "AL {offset}");
    }
    // 40:89 started at 0x51: BL=30h AL=0 sets 0xC1, BL=31h AL=1 sets bit 3,
    // BL=33h AL=0 sets bit 1.
    assert_eq!(read_ram_u8(&machine, BDA_MODESET_CONTROL), 0xCB);
    // BL=34h AL=1 sets 40:87 bit 0 on top of the mode set value 0x60.
    assert_eq!(read_ram_u8(&machine, BDA_VIDEO_CONTROL), 0x61);
}

#[test]
fn alternate_select_screen_off_and_on() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);
    assert_eq!(machine.bus.vga().seq[1] & 0x20, 0x00);

    #[rustfmt::skip]
    let screen_off: &[u8] = &[
        0xB8, 0x01, 0x12,       // MOV AX, 0x1201
        0xBB, 0x36, 0x00,       // MOV BX, 0x0036
        0xCD, 0x10,             // INT 10h
        0xA2, 0x00, 0x06,       // MOV [0x0600], AL
        0xF4,                   // HLT
    ];
    inject_and_run(&mut machine, screen_off, &[], BUDGET);
    assert_eq!(machine.bus.vga().seq[1] & 0x20, 0x20);
    assert_eq!(read_ram_u8(&machine, RESULT), 0x12);

    #[rustfmt::skip]
    let screen_on: &[u8] = &[
        0xB8, 0x00, 0x12,       // MOV AX, 0x1200
        0xBB, 0x36, 0x00,       // MOV BX, 0x0036
        0xCD, 0x10,             // INT 10h
        0xF4,                   // HLT
    ];
    inject_and_run(&mut machine, screen_on, &[], BUDGET);
    assert_eq!(machine.bus.vga().seq[1] & 0x20, 0x00);
}

/// Gray-scale sum of one DAC triple, the value the real BIOS produces.
fn gray_sum(entry: [u8; 3]) -> u8 {
    let sum = 77 * u32::from(entry[0]) + 151 * u32::from(entry[1]) + 28 * u32::from(entry[2]);
    (((sum + 0x80) >> 8).min(0x3F)) as u8
}

/// Mode 13h, AH=10h AL=12h loads six known DAC entries at index 0, AH=10h
/// AL=1Bh gray-scale sums entries 1 to 4.
#[rustfmt::skip]
const GRAY_SUM_BLOCK_CODE: &[u8] = &[
    0xB8, 0x13, 0x00,       // MOV AX, 0x0013
    0xCD, 0x10,             // INT 10h
    0xB8, 0x00, 0x03,       // MOV AX, 0x0300
    0x8E, 0xC0,             // MOV ES, AX
    0xBA, 0x00, 0x00,       // MOV DX, 0x0000
    0xBB, 0x00, 0x00,       // MOV BX, 0x0000
    0xB9, 0x06, 0x00,       // MOV CX, 0x0006
    0xB8, 0x12, 0x10,       // MOV AX, 0x1012
    0xCD, 0x10,             // INT 10h
    0xB8, 0x1B, 0x10,       // MOV AX, 0x101B
    0xBB, 0x01, 0x00,       // MOV BX, 0x0001
    0xB9, 0x04, 0x00,       // MOV CX, 0x0004
    0xCD, 0x10,             // INT 10h
    0xF4,                   // HLT
];

#[test]
fn gray_scale_sum_block_leaves_the_neighbours_alone() {
    /// The six loaded triples: entry 0 and 5 bracket the summed block.
    const BLOCK: [[u8; 3]; 6] = [
        [0x3F, 0x3F, 0x3F],
        [0x3F, 0x00, 0x00],
        [0x00, 0x3F, 0x00],
        [0x00, 0x00, 0x3F],
        [0x21, 0x14, 0x07],
        [0x10, 0x20, 0x30],
    ];

    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);
    write_bytes(&mut machine, 0x3000, &BLOCK.concat());
    inject_and_run(&mut machine, GRAY_SUM_BLOCK_CODE, &[], BUDGET);

    // The captured real BIOS values for the four summed triples.
    assert_eq!(machine.bus.vga().dac[1], [0x13; 3]);
    assert_eq!(machine.bus.vga().dac[2], [0x25; 3]);
    assert_eq!(machine.bus.vga().dac[3], [0x07; 3]);
    assert_eq!(machine.bus.vga().dac[4], [0x16; 3]);
    for entry in [0usize, 5] {
        assert_eq!(
            machine.bus.vga().dac[entry],
            BLOCK[entry],
            "DAC entry {entry}"
        );
    }
}

/// Mode 13h, four known DAC entries at 254, 255, 0 and 1, then AH=10h AL=1Bh
/// with BX=254 and CX=4 so the block wraps past the last entry.
#[rustfmt::skip]
const GRAY_SUM_WRAP_CODE: &[u8] = &[
    0xB8, 0x13, 0x00,       // MOV AX, 0x0013
    0xCD, 0x10,             // INT 10h
    0xB8, 0x00, 0x03,       // MOV AX, 0x0300
    0x8E, 0xC0,             // MOV ES, AX
    0xBA, 0x00, 0x00,       // MOV DX, 0x0000
    0xBB, 0xFE, 0x00,       // MOV BX, 0x00FE
    0xB9, 0x02, 0x00,       // MOV CX, 0x0002
    0xB8, 0x12, 0x10,       // MOV AX, 0x1012
    0xCD, 0x10,             // INT 10h
    0xBA, 0x06, 0x00,       // MOV DX, 0x0006
    0xBB, 0x00, 0x00,       // MOV BX, 0x0000
    0xB9, 0x02, 0x00,       // MOV CX, 0x0002
    0xB8, 0x12, 0x10,       // MOV AX, 0x1012
    0xCD, 0x10,             // INT 10h
    0xB8, 0x1B, 0x10,       // MOV AX, 0x101B
    0xBB, 0xFE, 0x00,       // MOV BX, 0x00FE
    0xB9, 0x04, 0x00,       // MOV CX, 0x0004
    0xCD, 0x10,             // INT 10h
    0xF4,                   // HLT
];

#[test]
fn gray_scale_sum_wraps_past_the_last_dac_entry() {
    /// The four loaded triples, in entry order 254, 255, 0, 1.
    const BLOCK: [[u8; 3]; 4] = [
        [0x3F, 0x00, 0x00],
        [0x00, 0x3F, 0x00],
        [0x00, 0x00, 0x3F],
        [0x21, 0x14, 0x07],
    ];

    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);
    write_bytes(&mut machine, 0x3000, &BLOCK.concat());
    inject_and_run(&mut machine, GRAY_SUM_WRAP_CODE, &[], BUDGET);

    for (entry, triple) in [254usize, 255, 0, 1].iter().zip(BLOCK.iter()) {
        assert_eq!(
            machine.bus.vga().dac[*entry],
            [gray_sum(*triple); 3],
            "DAC entry {entry}"
        );
    }
}

/// AH=12h BL=33h AL=0 enables gray-scale summing, then mode 13h loads its
/// palette through the sum.
#[rustfmt::skip]
const GRAY_SUM_MODE_SET_CODE: &[u8] = &[
    0xB8, 0x00, 0x12,       // MOV AX, 0x1200
    0xBB, 0x33, 0x00,       // MOV BX, 0x0033
    0xCD, 0x10,             // INT 10h
    0xB8, 0x13, 0x00,       // MOV AX, 0x0013
    0xCD, 0x10,             // INT 10h
    0xF4,                   // HLT
];

#[test]
fn gray_sum_flag_applies_to_the_mode_set_palette() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &set_mode_code(0x13), &[], BUDGET);
    let color_palette = machine.bus.vga().dac;

    inject_and_run(&mut machine, GRAY_SUM_MODE_SET_CODE, &[], BUDGET);
    assert_eq!(read_ram_u8(&machine, BDA_MODESET_CONTROL), 0x53);
    for (entry, triple) in color_palette.iter().enumerate() {
        assert_eq!(
            machine.bus.vga().dac[entry],
            [gray_sum(*triple); 3],
            "DAC entry {entry}"
        );
    }
}

/// AH=12h BL=31h AL=1 disables palette loading, a hand-written DAC entry then
/// survives the mode 12h set.
#[rustfmt::skip]
const PALETTE_LOAD_DISABLE_CODE: &[u8] = &[
    0xB8, 0x01, 0x12,       // MOV AX, 0x1201
    0xBB, 0x31, 0x00,       // MOV BX, 0x0031
    0xCD, 0x10,             // INT 10h
    0xB8, 0x10, 0x10,       // MOV AX, 0x1010
    0xBB, 0x07, 0x00,       // MOV BX, 0x0007
    0xB6, 0x3F,             // MOV DH, 0x3F
    0xB5, 0x2A,             // MOV CH, 0x2A
    0xB1, 0x15,             // MOV CL, 0x15
    0xCD, 0x10,             // INT 10h
    0xB8, 0x12, 0x00,       // MOV AX, 0x0012
    0xCD, 0x10,             // INT 10h
    0xF4,                   // HLT
];

/// AH=12h BL=31h AL=0 re-enables palette loading, the following mode 13h set
/// loads the mode palette again.
#[rustfmt::skip]
const PALETTE_LOAD_ENABLE_CODE: &[u8] = &[
    0xB8, 0x00, 0x12,       // MOV AX, 0x1200
    0xBB, 0x31, 0x00,       // MOV BX, 0x0031
    0xCD, 0x10,             // INT 10h
    0xB8, 0x13, 0x00,       // MOV AX, 0x0013
    0xCD, 0x10,             // INT 10h
    0xF4,                   // HLT
];

#[test]
fn palette_load_disable_preserves_the_dac_across_a_mode_set() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &set_mode_code(0x13), &[], BUDGET);
    let mode_palette = machine.bus.vga().dac;

    inject_and_run(&mut machine, PALETTE_LOAD_DISABLE_CODE, &[], BUDGET);
    assert_eq!(read_ram_u8(&machine, BDA_MODESET_CONTROL), 0x59);
    assert_eq!(machine.bus.vga().dac[7], [0x3F, 0x2A, 0x15]);

    inject_and_run(&mut machine, PALETTE_LOAD_ENABLE_CODE, &[], BUDGET);
    assert_eq!(read_ram_u8(&machine, BDA_MODESET_CONTROL), 0x51);
    assert_eq!(machine.bus.vga().dac, mode_palette);
}

/// Builds `MOV AX, 12<lines>; MOV BX, 0030h; INT 10h; MOV AX, 3; INT 10h; HLT`,
/// a scan line request followed by a mode set.
fn scan_line_request_code(lines: u8) -> [u8; 14] {
    [
        0xB8, lines, 0x12, 0xBB, 0x30, 0x00, 0xCD, 0x10, 0xB8, 0x03, 0x00, 0xCD, 0x10, 0xF4,
    ]
}

#[test]
fn scan_line_request_bits_survive_a_mode_set() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);
    assert_eq!(read_ram_u8(&machine, BDA_MODESET_CONTROL), 0x51);

    // 200 lines sets bit 7, 350 lines neither bit and 400 lines bit 4, the
    // captured real BIOS mapping. None of them is cleared by the mode set.
    for (lines, expected) in [(0x00u8, 0xC1u8), (0x01, 0x41), (0x02, 0x51)] {
        inject_and_run(&mut machine, &scan_line_request_code(lines), &[], BUDGET);
        assert_eq!(
            read_ram_u8(&machine, BDA_MODESET_CONTROL),
            expected,
            "AL={lines:#04X}"
        );
    }
}
