//! POST state: golden checks of the guest-visible machine state the HLE POST
//! leaves behind. The values are the ones the real AMI BIOS produces, taken
//! from side-by-side captures against the real ct486 ROM set.

use common::NoTrace;
use machine_at::{AtMachine, AtModel, LoadedRoms};

use super::{
    harness, make_halt_boot_floppy, read_ivt_vector, read_ram_u8, read_ram_u16, read_ram_u32,
};

macro_rules! check {
    ($f:ident, $left:expr, $right:expr, $label:expr) => {
        let left = $left;
        let right = $right;
        if left != right {
            $f.push(format!("{}: left={:?}, right={:?}", $label, left, right));
        }
    };
}

fn report_failures(failures: &[String], machine: &str) {
    if !failures.is_empty() {
        let msg = failures.join("\n  ");
        panic!(
            "{machine}: {n} assertion(s) failed:\n  {msg}",
            n = failures.len()
        );
    }
}

/// Boots a machine over the given ROM set with a CLI;HLT floppy to halt.
fn boot_to_halt_with_roms(model: AtModel, roms: LoadedRoms) -> AtMachine<NoTrace> {
    let mut machine = harness::machine_with_roms(model, roms);
    machine
        .bus
        .insert_floppy(0, make_halt_boot_floppy(), None)
        .expect("insert boot floppy");
    let _cycles = boot_to_halt_with_budget!(machine, 60_000_000_000u64);
    machine
}

/// BIOS data area: SAVE_PTR, the far pointer to the video save pointer table.
const BDA_SAVE_POINTER: u32 = 0x4A8;
/// Real-mode segment both video BIOS images are mapped at.
const VIDEO_BIOS_SEGMENT: u16 = 0xC000;
/// Size of the video BIOS window in bytes.
const VIDEO_BIOS_WINDOW_SIZE: u32 = 0x8000;
/// Size of the video save pointer table in bytes.
const SAVE_POINTER_TABLE_SIZE: u32 = 28;
/// Size of one video parameter table entry in bytes.
const VIDEO_PARAMETER_ENTRY_SIZE: u32 = 64;
/// Number of entries in the standard video parameter table.
const VIDEO_PARAMETER_ENTRIES: u32 = 29;

/// Linear address a far pointer stored as a dword resolves to in real mode.
fn far_pointer_address(pointer: u32) -> u32 {
    ((pointer >> 16) << 4).wrapping_add(pointer & 0xFFFF)
}

/// Golden checks of the HLE POST state.
fn check_post_state(machine: &mut AtMachine<NoTrace>, label: &str) {
    let mut f: Vec<String> = Vec::new();

    check!(
        f,
        read_ram_u16(machine, 0x400),
        0x03F8,
        "BDA 40:00 COM1 base"
    );
    check!(
        f,
        read_ram_u16(machine, 0x410),
        0x0223,
        "BDA 40:10 equipment"
    );
    check!(
        f,
        read_ram_u16(machine, 0x413),
        640,
        "BDA 40:13 memory size"
    );
    check!(
        f,
        read_ram_u16(machine, 0x41A),
        0x001E,
        "BDA 40:1A buffer head"
    );
    check!(
        f,
        read_ram_u16(machine, 0x41C),
        0x001E,
        "BDA 40:1C buffer tail"
    );
    check!(f, read_ram_u8(machine, 0x449), 0x03, "BDA 40:49 video mode");
    check!(f, read_ram_u16(machine, 0x44A), 80, "BDA 40:4A columns");
    check!(
        f,
        read_ram_u16(machine, 0x44C),
        0x1000,
        "BDA 40:4C page size"
    );
    check!(
        f,
        read_ram_u16(machine, 0x460),
        0x0D0E,
        "BDA 40:60 cursor shape"
    );
    check!(
        f,
        read_ram_u16(machine, 0x463),
        0x03D4,
        "BDA 40:63 CRTC base"
    );
    check!(
        f,
        read_ram_u16(machine, 0x480),
        0x001E,
        "BDA 40:80 buffer start"
    );
    check!(
        f,
        read_ram_u16(machine, 0x482),
        0x003E,
        "BDA 40:82 buffer end"
    );
    check!(
        f,
        read_ram_u8(machine, 0x484),
        24,
        "BDA 40:84 rows minus one"
    );
    check!(f, read_ram_u16(machine, 0x485), 16, "BDA 40:85 char height");

    // Stub ROM vectors: INT 19h and the hardware IRQ vectors live in the
    // F-segment; unlisted vectors stay empty.
    for vector in [
        0x05u8, 0x08, 0x09, 0x10, 0x13, 0x14, 0x15, 0x16, 0x17, 0x19, 0x1A, 0x70, 0x75, 0x76,
    ] {
        let (segment, _offset) = read_ivt_vector(machine, vector);
        check!(f, segment, 0xF000, format!("IVT {vector:02X} segment"));
    }

    // INT 41h/46h point at the fixed disk parameter tables published by the
    // stub ROM metadata words 16 and 18.
    let fdpt_0 = read_ram_u16(machine, 0xF0010);
    let fdpt_1 = read_ram_u16(machine, 0xF0012);
    check!(
        f,
        read_ivt_vector(machine, 0x41),
        (0xF000, fdpt_0),
        "IVT 41h"
    );
    check!(
        f,
        read_ivt_vector(machine, 0x46),
        (0xF000, fdpt_1),
        "IVT 46h"
    );

    // Save pointer chain: 40:A8 leads to the VGA stub table, whose dword 0
    // leads to the 29-entry video parameter table. INT 1Dh stays on the
    // system BIOS dummy handler like the real AMI BIOS leaves it.
    let save_pointer = read_ram_u32(machine, BDA_SAVE_POINTER);
    check!(
        f,
        (save_pointer >> 16) as u16,
        VIDEO_BIOS_SEGMENT,
        "BDA 40:A8 segment"
    );
    let table_offset = save_pointer & 0xFFFF;
    check!(
        f,
        table_offset != 0 && table_offset + SAVE_POINTER_TABLE_SIZE <= VIDEO_BIOS_WINDOW_SIZE,
        true,
        "save pointer table fits the video BIOS window"
    );
    let table = far_pointer_address(save_pointer);

    // The dynamic save area and both character set override pointers start
    // null, and so does the secondary save pointer table: AH=1Ah is
    // synthesized directly instead of published as a table.
    for (index, name) in [
        (1u32, "dynamic save area"),
        (2, "alphanumeric character set override"),
        (3, "graphics character set override"),
        (4, "secondary save pointer table"),
    ] {
        check!(
            f,
            read_ram_u32(machine, table + index * 4),
            0,
            format!("save pointer table {name}")
        );
    }
    for offset in 20..SAVE_POINTER_TABLE_SIZE {
        check!(
            f,
            read_ram_u8(machine, table + offset),
            0,
            format!("save pointer table reserved byte {offset}")
        );
    }

    let parameters = read_ram_u32(machine, table);
    check!(
        f,
        (parameters >> 16) as u16,
        VIDEO_BIOS_SEGMENT,
        "video parameter table segment"
    );
    let parameters_offset = parameters & 0xFFFF;
    let parameters_size = VIDEO_PARAMETER_ENTRIES * VIDEO_PARAMETER_ENTRY_SIZE;
    check!(
        f,
        parameters_offset != 0 && parameters_offset + parameters_size <= VIDEO_BIOS_WINDOW_SIZE,
        true,
        "video parameter table fits the video BIOS window"
    );
    check!(
        f,
        u32::from(read_ram_u16(machine, 0xC001A)),
        parameters & 0xFFFF,
        "video parameter table offset matches the ROM metadata word"
    );
    check!(
        f,
        read_ram_u16(machine, 0xC001C),
        VIDEO_PARAMETER_ENTRIES as u16,
        "video parameter table entry count"
    );
    let (segment_1dh, _) = read_ivt_vector(machine, 0x1D);
    check!(f, segment_1dh, 0xF000, "IVT 1Dh segment");

    // The mode 03h entry at index 24, byte for byte from the register file
    // the POST programs.
    #[rustfmt::skip]
    const MODE_03H_ENTRY: [u8; 64] = [
        0x50, 0x18, 0x10, 0x00, 0x10,
        0x00, 0x03, 0x00, 0x02,
        0x67,
        0x5F, 0x4F, 0x50, 0x82, 0x55, 0x81, 0xBF, 0x1F,
        0x00, 0x4F, 0x0D, 0x0E, 0x00, 0x00, 0x00, 0x00,
        0x9C, 0x8E, 0x8F, 0x28, 0x1F, 0x96, 0xB9, 0xA3,
        0xFF,
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x14, 0x07,
        0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F,
        0x0C, 0x00, 0x0F, 0x08,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x0E, 0x00,
        0xFF,
    ];
    let entry_base = far_pointer_address(parameters) + 24 * VIDEO_PARAMETER_ENTRY_SIZE;
    for (offset, &expected) in MODE_03H_ENTRY.iter().enumerate() {
        check!(
            f,
            read_ram_u8(machine, entry_base + offset as u32),
            expected,
            format!("video parameter entry 24 byte {offset}")
        );
    }

    check!(f, read_ram_u8(machine, 0x474), 0, "BDA 40:74 disk status");
    check!(f, read_ram_u8(machine, 0x475), 0, "BDA 40:75 disk count");
    check!(
        f,
        read_ram_u8(machine, 0x48E),
        0,
        "BDA 40:8E interrupt flag"
    );

    let state = machine.inspection_state();
    check!(
        f,
        state.pic.chips[0].icw,
        [0x11, 0x08, 0x04, 0x01],
        "Master PIC ICW"
    );
    check!(f, state.pic.chips[0].imr, 0xB8, "Master PIC IMR");
    check!(
        f,
        state.pic.chips[1].icw,
        [0x11, 0x70, 0x02, 0x01],
        "Slave PIC ICW"
    );
    check!(f, state.pic.chips[1].imr, 0xDD, "Slave PIC IMR");
    check!(f, state.pit.channels[0].ctrl, 0x36, "PIT ch0 ctrl");
    check!(f, state.pit.channels[0].value, 0x0000, "PIT ch0 value");
    check!(f, state.pit.channels[1].ctrl, 0x14, "PIT ch1 ctrl");
    check!(f, state.pit.channels[1].value, 0x0012, "PIT ch1 value");
    check!(f, state.pit.channels[2].ctrl, 0x36, "PIT ch2 ctrl");
    check!(f, state.pit.channels[2].value, 0x0505, "PIT ch2 value");
    check!(f, state.a20_enabled, false, "A20 disabled after POST");

    report_failures(&f, label);
}

#[test]
fn post_fdpt_describes_the_mounted_disks() {
    let mut machine =
        harness::machine_with_roms::<NoTrace>(AtModel::At486Dx50, LoadedRoms::hle_stub_set());
    machine
        .bus
        .insert_hdd(0, super::make_halt_boot_hdd(), None)
        .expect("insert boot hard disk");
    let _cycles = boot_to_halt_with_budget!(machine, 60_000_000_000u64);

    // One cylinder of 16 heads and 63 sectors: cylinders word, heads, zero
    // padding, write precompensation FFFF, control byte C8h (more than 8
    // heads), landing zone and sectors per track.
    let table_offset = read_ram_u16(&machine, 0xF0010);
    let expected: [u8; 16] = [
        0x01, 0x00, 0x10, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0xC8, 0x00, 0x00, 0x00, 0x01, 0x00, 0x3F,
        0x00,
    ];
    for (index, &byte) in expected.iter().enumerate() {
        assert_eq!(
            read_ram_u8(&machine, 0xF0000 + u32::from(table_offset) + index as u32),
            byte,
            "FDPT drive 0 byte {index}"
        );
    }
    // The second table stays zeroed without a second drive.
    let table_offset = read_ram_u16(&machine, 0xF0012);
    for index in 0..16u32 {
        assert_eq!(
            read_ram_u8(&machine, 0xF0000 + u32::from(table_offset) + index),
            0,
            "FDPT drive 1 byte {index}"
        );
    }
    assert_eq!(read_ram_u8(&machine, 0x475), 1, "BDA 40:75 disk count");
}

#[test]
fn post_bios_state_dx50() {
    let mut machine = boot_to_halt_with_roms(AtModel::At486Dx50, LoadedRoms::hle_stub_set());
    check_post_state(&mut machine, "post_bios_state_dx50");
}

#[test]
fn post_bios_state_dx66() {
    let mut machine = boot_to_halt_with_roms(AtModel::At486Dx66, LoadedRoms::hle_stub_set());
    check_post_state(&mut machine, "post_bios_state_dx66");
}
