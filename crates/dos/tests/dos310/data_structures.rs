use crate::harness::*;

/// Offset of the list of lists in the DOS data segment of DOS 3.10.
const LIST_OF_LISTS_OFFSET: u16 = 0x0026;
/// Offset of the first SFT block in the DOS data segment of DOS 3.10.
const FIRST_SFT_BLOCK_OFFSET: u16 = 0x0098;
/// Offset of the NUL device header in the DOS data segment of DOS 3.10.
const NUL_DEVICE_OFFSET: u16 = 0x0048;

/// List of lists offset of the far pointer to the first DPB.
const LIST_OF_LISTS_DPB_POINTER: u32 = 0x00;
/// List of lists offset of the far pointer to the first SFT block.
const LIST_OF_LISTS_SFT_POINTER: u32 = 0x04;
/// List of lists offset of the largest sector size.
const LIST_OF_LISTS_MAX_SECTOR: u32 = 0x10;
/// List of lists offset of the far pointer to the first disk buffer.
const LIST_OF_LISTS_BUFFER_POINTER: u32 = 0x12;
/// List of lists offset of the far pointer to the CDS array.
const LIST_OF_LISTS_CDS_POINTER: u32 = 0x16;
/// List of lists offset of the block device count.
const LIST_OF_LISTS_BLOCK_DEVICE_COUNT: u32 = 0x20;
/// List of lists offset of the NUL device header.
const LIST_OF_LISTS_NUL_HEADER: u32 = 0x22;
/// List of lists offset of the first byte behind the NUL device header.
const LIST_OF_LISTS_NUL_HEADER_END: u32 = 0x34;
/// Distance of the first MCB word below the list of lists.
const FIRST_MCB_DISTANCE: u32 = 2;
/// MCB type byte of a block in the middle of the chain.
const MCB_TYPE_MIDDLE: u8 = b'M';

/// Device header offset of the name.
const DEVICE_HEADER_NAME: u32 = 0x0A;
/// Size of an SFT block header.
const SFT_HEADER_SIZE: u32 = 6;
/// SFT block header offset of the entry count.
const SFT_BLOCK_COUNT: u32 = 0x04;
/// Size of a DOS 3.10 SFT entry.
const SFT_ENTRY_SIZE: u32 = 0x35;
/// SFT entry offset of the FCB name.
const SFT_ENTRY_NAME: u32 = 0x20;
/// Entries of the first SFT block.
const SFT_INITIAL_COUNT: u16 = 5;

/// Size of a DOS 3.10 CDS entry.
const CDS_ENTRY_SIZE: u32 = 0x51;
/// Entries of the CDS array.
const CDS_ENTRY_COUNT: u32 = 26;
/// CDS entry offset of the flags word.
const CDS_FLAGS: u32 = 0x43;
/// CDS entry offset of the far pointer to the DPB.
const CDS_DPB_POINTER: u32 = 0x45;
/// CDS entry offset of the root length.
const CDS_ROOT_LENGTH: u32 = 0x4F;
/// CDS flag of a physical drive.
const CDS_FLAG_PHYSICAL: u16 = 0x4000;
/// CDS flag of a network drive.
const CDS_FLAG_NETWORK: u16 = 0x8000;
/// Drive index of the virtual drive Z:.
const VIRTUAL_DRIVE_INDEX: u32 = 25;

/// Size of a DOS 3.10 DPB.
const DPB_SIZE: u32 = 0x20;
/// DPB offset of the drive number.
const DPB_DRIVE_NUMBER: u32 = 0x00;
/// DPB offset of the bytes per sector.
const DPB_BYTES_PER_SECTOR: u32 = 0x02;
/// DPB offset of the byte sectors per FAT of DOS 3.10.
const DPB_SECTORS_PER_FAT: u32 = 0x0F;
/// DPB offset of the first root directory sector of DOS 3.10.
const DPB_FIRST_ROOT_SECTOR: u32 = 0x10;
/// DPB offset of the media descriptor of DOS 3.10.
const DPB_MEDIA_DESCRIPTOR: u32 = 0x16;
/// DPB offset of the far pointer to the next DPB of DOS 3.10.
const DPB_NEXT: u32 = 0x18;
/// Offset of a far pointer that ends a chain.
const END_OF_CHAIN_OFFSET: u16 = 0xFFFF;
/// Upper bound of DPBs in the chain.
const MAXIMUM_DPB_COUNT: usize = 26;

/// Disk buffer header offset of the far pointer to the next buffer.
const BUFFER_NEXT: u32 = 0x00;
/// Disk buffer header offset of the drive byte.
const BUFFER_DRIVE: u32 = 0x04;
/// Drive byte of an unused disk buffer.
const BUFFER_DRIVE_NONE: u8 = 0xFF;
/// Far pointer that ends a chain.
const END_OF_CHAIN: (u16, u16) = (0xFFFF, 0xFFFF);

/// Bytes per sector of the test floppy.
const FLOPPY_BYTES_PER_SECTOR: u16 = 1024;
/// Sectors per FAT of the test floppy.
const FLOPPY_SECTORS_PER_FAT: u8 = 2;
/// First root directory sector of the test floppy.
const FLOPPY_FIRST_ROOT_SECTOR: u16 = 5;
/// Media descriptor of the test floppy.
const FLOPPY_MEDIA_DESCRIPTOR: u8 = 0xFE;

/// Size of the swappable data area of DOS 3.10.
const SWAPPABLE_DATA_AREA_SIZE: u16 = 0x0738;
/// Size of the always swapped part of the SDA of DOS 3.10.
const SWAPPABLE_DATA_AREA_ALWAYS_SIZE: u16 = 0x0018;
/// Offset of the SDA in the DOS data segment of DOS 3.10.
const SWAPPABLE_DATA_AREA_OFFSET: u16 = 0x02CE;
/// Bytes per paragraph.
const PARAGRAPH_BYTES: u16 = 16;

/// Directory levels of the deep `CD` test.
const DEEP_DIRECTORY_LEVELS: usize = 6;
/// Offset of the first path in the data area of the deep `CD` test.
const DEEP_PATH_OFFSET: u16 = 0x0240;
/// Distance between two paths in the data area of the deep `CD` test.
const DEEP_PATH_STRIDE: u16 = 0x0040;
/// `INT 21h` function that creates a directory.
const MAKE_DIRECTORY_FUNCTION: u8 = 0x39;
/// `INT 21h` function that changes the current directory.
const CHANGE_DIRECTORY_FUNCTION: u8 = 0x3B;
/// Carry flag in the flags word.
const CARRY_FLAG: u16 = 0x0001;

/// Returns the PC-98 3.10 data area layout.
fn data_area() -> &'static dos::layout::DataAreaLayout {
    dos::DataArea::Pc98Dos310.layout()
}

/// Returns the linear ranges of the `IO.SYS` work area fields that alias
/// the DOS data segment: the drive table pointer and the extended drive
/// table.
fn iosys_alias_ranges() -> [std::ops::Range<u32>; 2] {
    use dos::tables::{
        IOSYS_BASE, IOSYS_EXT_DAUA_TABLE_SIZE, IOSYS_OFF_DAUA_PTR, IOSYS_OFF_EXT_DAUA_TABLE,
    };
    let pointer = IOSYS_BASE + IOSYS_OFF_DAUA_PTR;
    let table = IOSYS_BASE + IOSYS_OFF_EXT_DAUA_TABLE;
    [
        pointer..pointer + 4,
        table..table + IOSYS_EXT_DAUA_TABLE_SIZE,
    ]
}

/// Boots with the standard floppy as A: and returns the machine and the
/// linear address of the list of lists.
fn boot_and_find_list_of_lists() -> (Machine, u32) {
    let mut machine = boot_with_floppy(create_floppy(&standard_files()));
    let pointer = list_of_lists_pointer(&mut machine);
    assert_eq!(pointer, (DOS_DATA_SEGMENT, LIST_OF_LISTS_OFFSET));
    (machine, far_to_linear(pointer))
}

#[test]
fn list_of_lists_lies_at_offset_0026h_with_the_nul_header_at_0048h() {
    let (machine, list_of_lists) = boot_and_find_list_of_lists();
    let bus = &machine.bus;
    let nul_header = list_of_lists + LIST_OF_LISTS_NUL_HEADER;
    assert_eq!(
        nul_header,
        far_to_linear((DOS_DATA_SEGMENT, NUL_DEVICE_OFFSET))
    );
    assert_eq!(
        read_bytes(bus, nul_header + DEVICE_HEADER_NAME, 8),
        b"NUL     "
    );
    let (con_segment, con_offset) = read_far_pointer(bus, nul_header);
    assert_eq!(con_segment, DOS_DATA_SEGMENT);
    assert_ne!(con_offset, NUL_DEVICE_OFFSET);
    assert_eq!(
        read_bytes(
            bus,
            far_to_linear((con_segment, con_offset)) + DEVICE_HEADER_NAME,
            8
        ),
        b"CON     "
    );
    let first_mcb = read_word(bus, list_of_lists - FIRST_MCB_DISTANCE);
    assert!(first_mcb > DOS_DATA_SEGMENT);
    assert_eq!(
        read_byte(bus, far_to_linear((first_mcb, 0))),
        MCB_TYPE_MIDDLE
    );
}

#[test]
fn first_sft_block_lies_at_offset_0098h_with_35h_byte_entries() {
    let (machine, list_of_lists) = boot_and_find_list_of_lists();
    let bus = &machine.bus;
    let block = read_far_pointer(bus, list_of_lists + LIST_OF_LISTS_SFT_POINTER);
    assert_eq!(block, (DOS_DATA_SEGMENT, FIRST_SFT_BLOCK_OFFSET));
    let block_address = far_to_linear(block);
    assert_eq!(
        read_word(bus, block_address + SFT_BLOCK_COUNT),
        SFT_INITIAL_COUNT
    );
    let names: [&[u8]; 5] = [
        b"CON        ",
        b"CON        ",
        b"CON        ",
        b"AUX        ",
        b"PRN        ",
    ];
    for (index, name) in names.into_iter().enumerate() {
        let entry = block_address + SFT_HEADER_SIZE + index as u32 * SFT_ENTRY_SIZE;
        assert_eq!(
            read_bytes(bus, entry + SFT_ENTRY_NAME, name.len()),
            name,
            "SFT entry {index}"
        );
    }
}

#[test]
fn current_directory_structure_has_51h_byte_entries() {
    let (machine, list_of_lists) = boot_and_find_list_of_lists();
    let bus = &machine.bus;
    let array = read_far_pointer(bus, list_of_lists + LIST_OF_LISTS_CDS_POINTER);
    assert_eq!(array.0, DOS_DATA_SEGMENT);
    let array_address = far_to_linear(array);

    let drive_a = array_address;
    assert_eq!(read_bytes(bus, drive_a, 4), b"A:\\\0");
    assert_eq!(read_word(bus, drive_a + CDS_FLAGS), CDS_FLAG_PHYSICAL);
    assert_eq!(
        read_byte(bus, drive_a + CDS_FLAGS + 1),
        (CDS_FLAG_PHYSICAL >> 8) as u8
    );
    assert_eq!(read_word(bus, drive_a + CDS_ROOT_LENGTH), 2);
    assert_eq!(
        read_far_pointer(bus, drive_a + CDS_DPB_POINTER),
        read_far_pointer(bus, list_of_lists + LIST_OF_LISTS_DPB_POINTER)
    );

    let drive_z = array_address + VIRTUAL_DRIVE_INDEX * CDS_ENTRY_SIZE;
    assert_eq!(read_bytes(bus, drive_z, 3), b"Z:\\");
    assert_eq!(read_word(bus, drive_z + CDS_FLAGS), CDS_FLAG_NETWORK);
}

#[test]
fn drive_parameter_blocks_chain_through_offset_18h() {
    let (machine, list_of_lists) = boot_and_find_list_of_lists();
    let bus = &machine.bus;
    let mut pointer = read_far_pointer(bus, list_of_lists + LIST_OF_LISTS_DPB_POINTER);
    let mut blocks = Vec::new();
    while pointer.1 != END_OF_CHAIN_OFFSET {
        assert!(blocks.len() < MAXIMUM_DPB_COUNT, "DPB chain does not end");
        let address = far_to_linear(pointer);
        blocks.push(address);
        pointer = read_far_pointer(bus, address + DPB_NEXT);
    }
    assert_eq!(
        blocks.len(),
        usize::from(read_byte(
            bus,
            list_of_lists + LIST_OF_LISTS_BLOCK_DEVICE_COUNT
        ))
    );
    assert!(blocks.len() >= 2, "A: and Z: have a DPB");
    for pair in blocks.windows(2) {
        assert_eq!(pair[1] - pair[0], DPB_SIZE);
    }

    let window = data_area()
        .private_area
        .parameter_block_window
        .expect("the PC-98 3.10 data area has a DPB window");
    assert_eq!(
        blocks[0],
        far_to_linear((DOS_DATA_SEGMENT, window.offset)),
        "the DPBs lie in the window"
    );

    let floppy = blocks[0];
    assert_eq!(read_byte(bus, floppy + DPB_DRIVE_NUMBER), 0);
    assert_eq!(
        read_word(bus, floppy + DPB_BYTES_PER_SECTOR),
        FLOPPY_BYTES_PER_SECTOR
    );
    assert_eq!(
        read_byte(bus, floppy + DPB_SECTORS_PER_FAT),
        FLOPPY_SECTORS_PER_FAT
    );
    assert_eq!(
        read_word(bus, floppy + DPB_FIRST_ROOT_SECTOR),
        FLOPPY_FIRST_ROOT_SECTOR
    );
    assert_eq!(
        read_byte(bus, floppy + DPB_MEDIA_DESCRIPTOR),
        FLOPPY_MEDIA_DESCRIPTOR
    );
}

#[test]
fn swappable_data_area_lies_at_offset_02ceh() {
    let mut machine = boot();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x06, 0x5D,             // MOV AX, 5D06h
        0xCD, 0x21,                   // INT 21h
        0x2E, 0x89, 0x36, 0x00, 0x01, // MOV CS:[0100h], SI
        0x2E, 0x8C, 0x1E, 0x02, 0x01, // MOV CS:[0102h], DS
        0x2E, 0x89, 0x0E, 0x04, 0x01, // MOV CS:[0104h], CX
        0x2E, 0x89, 0x16, 0x06, 0x01, // MOV CS:[0106h], DX
        0xFA,                         // CLI
        0xF4,                         // HLT
    ];
    inject_and_run(&mut machine, code);
    let bus = &machine.bus;
    assert_eq!(result_word(bus, 0), SWAPPABLE_DATA_AREA_OFFSET);
    assert_eq!(result_word(bus, 2), DOS_DATA_SEGMENT);
    assert_eq!(result_word(bus, 4), SWAPPABLE_DATA_AREA_SIZE);
    assert_eq!(result_word(bus, 6), SWAPPABLE_DATA_AREA_ALWAYS_SIZE);
}

#[test]
fn list_of_lists_names_a_dos_3_disk_buffer_and_ends_at_the_nul_header() {
    let (machine, list_of_lists) = boot_and_find_list_of_lists();
    let bus = &machine.bus;
    let buffer = read_far_pointer(bus, list_of_lists + LIST_OF_LISTS_BUFFER_POINTER);
    assert_eq!(
        buffer,
        (DOS_DATA_SEGMENT, data_area().private_area.system_data)
    );
    let buffer_address = far_to_linear(buffer);
    assert_eq!(
        read_far_pointer(bus, buffer_address + BUFFER_NEXT),
        END_OF_CHAIN
    );
    assert_eq!(
        read_byte(bus, buffer_address + BUFFER_DRIVE),
        BUFFER_DRIVE_NONE
    );
    assert_eq!(
        read_word(bus, list_of_lists + LIST_OF_LISTS_MAX_SECTOR),
        FLOPPY_BYTES_PER_SECTOR
    );

    let first_sft_block = far_to_linear((DOS_DATA_SEGMENT, FIRST_SFT_BLOCK_OFFSET));
    let behind_nul_header = list_of_lists + LIST_OF_LISTS_NUL_HEADER_END;
    let length = (first_sft_block - behind_nul_header) as usize;
    assert!(
        read_bytes(bus, behind_nul_header, length)
            .iter()
            .all(|&byte| byte == 0),
        "the HLE leaves the bytes behind the NUL header zero"
    );
}

#[test]
fn first_mcb_lies_at_or_behind_the_system_data() {
    let (machine, list_of_lists) = boot_and_find_list_of_lists();
    let first_mcb = read_word(&machine.bus, list_of_lists - FIRST_MCB_DISTANCE);
    let system_data_paragraphs = data_area().private_area.system_data / PARAGRAPH_BYTES;
    assert!(first_mcb >= DOS_DATA_SEGMENT + system_data_paragraphs);
}

/// Returns the path of `drive` with `levels` directory levels and a
/// terminating NUL.
fn deep_path(drive: u8, levels: usize) -> Vec<u8> {
    let mut path = vec![drive, b':'];
    for level in 1..=levels {
        path.extend_from_slice(format!("\\LEVEL{level:03}").as_bytes());
    }
    path.push(0);
    path
}

/// Creates the deep directory chain on `drive` and changes into it.
/// Returns the flags of the `CD` call.
fn change_into_deep_directory(machine: &mut Machine, drive: u8) -> u16 {
    let mut code = Vec::new();
    let mut deepest_offset = DEEP_PATH_OFFSET;
    for level in 1..=DEEP_DIRECTORY_LEVELS {
        let offset = DEEP_PATH_OFFSET + (level as u16 - 1) * DEEP_PATH_STRIDE;
        write_bytes(
            &mut machine.bus,
            INJECT_CODE_BASE + u32::from(offset),
            &deep_path(drive, level),
        );
        let [offset_low, offset_high] = offset.to_le_bytes();
        #[rustfmt::skip]
        code.extend_from_slice(&[
            0xBA, offset_low, offset_high,   // MOV DX, path
            0xB4, MAKE_DIRECTORY_FUNCTION,   // MOV AH, 39h
            0xCD, 0x21,                      // INT 21h
        ]);
        deepest_offset = offset;
    }
    let [offset_low, offset_high] = deepest_offset.to_le_bytes();
    #[rustfmt::skip]
    code.extend_from_slice(&[
        0xBA, offset_low, offset_high,   // MOV DX, path
        0xB4, CHANGE_DIRECTORY_FUNCTION, // MOV AH, 3Bh
        0xCD, 0x21,                      // INT 21h
        0x9C, 0x58,                      // PUSHF; POP AX
        0xA3, 0x00, 0x01,                // MOV [0100h], AX
        0xFA,                            // CLI
        0xF4,                            // HLT
    ]);
    inject_and_run_with_budget(machine, &code, INJECT_BUDGET_DISK_IO);
    result_word(&machine.bus, 0)
}

#[test]
fn current_directory_array_leaves_the_io_sys_work_area_alone() {
    let mut machine = boot_with_floppy_and_hdd(
        create_floppy(&standard_files()),
        create_hdd(&standard_files()),
    );
    let list_of_lists = far_to_linear(list_of_lists_pointer(&mut machine));
    let array = far_to_linear(read_far_pointer(
        &machine.bus,
        list_of_lists + LIST_OF_LISTS_CDS_POINTER,
    ));
    let array_range = array..array + CDS_ENTRY_COUNT * CDS_ENTRY_SIZE;
    let alias_ranges = iosys_alias_ranges();
    for range in &alias_ranges {
        assert!(
            range.end <= array_range.start || array_range.end <= range.start,
            "the CDS array {array_range:X?} covers {range:X?}"
        );
    }
    let saved: Vec<Vec<u8>> = alias_ranges
        .iter()
        .map(|range| read_bytes(&machine.bus, range.start, range.len()))
        .collect();
    assert_ne!(saved[0], [0; 4], "IO.SYS writes the drive table pointer");

    let physical_drives: Vec<u32> = (0..CDS_ENTRY_COUNT)
        .filter(|&index| {
            read_word(&machine.bus, array + index * CDS_ENTRY_SIZE + CDS_FLAGS) == CDS_FLAG_PHYSICAL
        })
        .collect();
    assert_eq!(physical_drives.len(), 2, "the floppy and the hard disk");
    for index in physical_drives {
        let drive = b'A' + index as u8;
        let flags = change_into_deep_directory(&mut machine, drive);
        assert_eq!(flags & CARRY_FLAG, 0, "CD on {}: failed", char::from(drive));
        let path = deep_path(drive, DEEP_DIRECTORY_LEVELS);
        assert_eq!(
            read_bytes(&machine.bus, array + index * CDS_ENTRY_SIZE, path.len()),
            path
        );
    }

    for (range, saved) in alias_ranges.iter().zip(&saved) {
        assert_eq!(
            &read_bytes(&machine.bus, range.start, range.len()),
            saved,
            "{range:X?}"
        );
    }
}
