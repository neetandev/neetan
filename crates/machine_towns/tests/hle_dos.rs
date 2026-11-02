//! Integration tests for NeetanDos as the DOS kernel of the HLE firmware. The
//! HLE system ROM boots a synthetic CD whose IPL plays the part of `IO.SYS`
//! and SYSINIT: it far-calls the `MSDOS.SYS` stub of the HLE OS ROM, which
//! enters NeetanDos through the trap port, and asks for the DOS version
//! through `INT 21h` afterwards. A save state taken between DOSINIT and the
//! `INT 21h` call restores into a fresh machine. A floppy insert and a CD
//! eject between two `INT 21h` calls drop the DOS caches of the old medium.
//!
//! The shell tests boot a CD whose IPL loads the synthetic `IO.SYS` and its
//! RAM disk from the CD and `MSDOS.SYS` from the OS ROM through the disk
//! BIOS, calls DOSINIT, builds the CDS array and runs the Rust shell like
//! SYSINIT. The shell runs `AUTOEXEC.BAT` of the RAM disk and reads commands
//! from the CON driver of the chain. A save state of the waiting shell
//! continues identically in a fresh machine.

#[path = "common/harness.rs"]
mod harness;

use common::{Bus, Cpu, Machine, MediaBacking, MediaImage};
use harness::{linear_pc, machine_mx_hle};
use machine_towns::{TownsBootDevice, TownsMachine};

/// Synthetic `IO.SYS` device chain of the NeetanDos kernel entry tests: CON,
/// AUX, TOWNS_CD, MENU_DRV, CLOCK, a block driver with two floppy units and
/// the OS ROM unit, and a RAM disk. `crates/dos/tests/guest/kernel_drivers.asm`
/// holds its source.
const KERNEL_DRIVERS: &[u8] = include_bytes!("../../dos/tests/guest/kernel_drivers.bin");

/// Embedded HLE OS ROM image of the machine.
const HLE_OS_ROM: &[u8] = include_bytes!("../../../utils/dos_towns/osrom.rom");

/// Segment of the device chain.
const IOSYS_SEGMENT: u16 = 0x0A00;
/// Offset of the `INT 29h` handler of the device chain.
const IOSYS_INT29_ENTRY: u16 = 0x0440;
/// Segment that receives `MSDOS.SYS`, the DOS data segment.
const KERNEL_SEGMENT: u16 = 0x1000;
/// Top of conventional memory that SYSINIT passes to DOSINIT.
const MEMORY_TOP: u16 = 0x8000;
/// Segment of the IPL.
const IPL_SEGMENT: u16 = 0xB000;
/// Offset of the IPL code behind the `IPL4` signature.
const IPL_ENTRY_OFFSET: u16 = 0x0004;
/// Offset in the IPL segment of the registers that the IPL stores.
const RESULT_OFFSET: u16 = 0x1000;
/// Offset of the list of lists in the DOS 3.10 layout.
const LIST_OF_LISTS_OFFSET: u16 = 0x0026;
/// DOS version that NeetanDos reports on the FM Towns (3.10).
const DOS_VERSION: u16 = 0x0A03;
/// Cycle budget of the POST and of the IPL.
const PHASE_CYCLES: u64 = 40_000_000;
/// Sector size of the CD images.
const CD_SECTOR_SIZE: usize = 2048;
/// Sector of the root directory of the CD image.
const CD_ROOT_SECTOR: usize = 18;
/// Sector of the file of the CD image.
const CD_FILE_SECTOR: usize = 19;
/// Bytes per sector of the floppy images (2HD, 1232 KiB).
const FLOPPY_SECTOR_SIZE: usize = 1024;
/// Sectors of the floppy images.
const FLOPPY_SECTORS: usize = 1232;
/// Sectors of one FAT of the floppy images.
const FLOPPY_FAT_SECTORS: usize = 2;
/// Root directory entries of the floppy images.
const FLOPPY_ROOT_ENTRIES: usize = 0xC0;
/// Data clusters of the floppy images: all sectors behind the boot sector,
/// the two FATs and the root directory.
const FLOPPY_CLUSTERS: u16 = 1221;
/// Clusters that the second floppy image allocates.
const FLOPPY_USED_CLUSTERS: u16 = 8;

/// Returns the linear address of `segment:offset`.
fn linear(segment: u16, offset: u16) -> u32 {
    (u32::from(segment) << 4) + u32::from(offset)
}

/// Offset in the IPL sector of the data that the IPL code reads.
const IPL_DATA_OFFSET: u16 = 0x0600;
/// Offset in the IPL segment of the first file read buffer.
const FIRST_BUFFER_OFFSET: u16 = 0x1100;
/// Offset in the IPL segment of the second file read buffer.
const SECOND_BUFFER_OFFSET: u16 = 0x1200;
/// Command tail of the MSCDEX call of `IO.SYS`.
const MSCDEX_TAIL: &[u8] = b" /L:Q /D:TOWNS_CD /D:MENU_DRV /M:8";
/// Path of the file on the CD.
const CD_FILE_PATH: &[u8] = b"Q:\\README.TXT\0";
/// Content of the file on the CD.
const CD_FILE_CONTENT: &[u8] = b"game disc";
/// Bytes of the first read of the CD file.
const CD_FIRST_READ: u16 = 4;

/// Returns the bytes of `value` as the two operand bytes of an instruction.
fn word(value: u16) -> [u8; 2] {
    value.to_le_bytes()
}

/// Returns the IPL code up to the return of DOSINIT: the `IPL4` signature
/// and the DOSINIT call.
fn dosinit_code() -> Vec<u8> {
    let mut code = b"IPL4".to_vec();
    code.extend(dosinit_call());
    code
}

/// Returns the DOSINIT call with `DS:SI` at the device chain, `DX` at the
/// memory top and `ES` at the kernel segment, and the store of `ES`, `DI`,
/// `DS` and `DX` at `RESULT_OFFSET`.
fn dosinit_call() -> Vec<u8> {
    let [iosys_low, iosys_high] = word(IOSYS_SEGMENT);
    let [top_low, top_high] = word(MEMORY_TOP);
    let [kernel_low, kernel_high] = word(KERNEL_SEGMENT);
    let [result_low, result_high] = word(RESULT_OFFSET);
    let mut code = Vec::new();
    code.extend_from_slice(&[
        0xB8,
        iosys_low,
        iosys_high, // MOV AX, IOSYS_SEGMENT
        0x8E,
        0xD8, // MOV DS, AX
        0xBE,
        0x00,
        0x00, // MOV SI, 0
        0xBA,
        top_low,
        top_high, // MOV DX, MEMORY_TOP
        0xB8,
        kernel_low,
        kernel_high, // MOV AX, KERNEL_SEGMENT
        0x8E,
        0xC0, // MOV ES, AX
        0x9A,
        0x00,
        0x00,
        kernel_low,
        kernel_high, // CALL FAR KERNEL:0000
        0x2E,
        0x8C,
        0x06,
        result_low,
        result_high, // MOV CS:[RESULT], ES
        0x2E,
        0x89,
        0x3E,
        result_low + 2,
        result_high, // MOV CS:[RESULT+2], DI
        0x2E,
        0x8C,
        0x1E,
        result_low + 4,
        result_high, // MOV CS:[RESULT+4], DS
        0x2E,
        0x89,
        0x16,
        result_low + 6,
        result_high, // MOV CS:[RESULT+6], DX
    ]);
    code
}

/// Returns the offset of the first instruction after DOSINIT.
fn after_dosinit_offset() -> u16 {
    dosinit_code().len() as u16
}

/// Returns the IPL sector with the DOSINIT call, `after_dosinit` behind it
/// and `data` at `IPL_DATA_OFFSET`.
fn ipl_sector(after_dosinit: &[u8], data: &[u8]) -> Vec<u8> {
    let mut sector = dosinit_code();
    sector.extend_from_slice(after_dosinit);
    assert!(sector.len() <= usize::from(IPL_DATA_OFFSET));
    sector.resize(usize::from(IPL_DATA_OFFSET), 0);
    sector.extend_from_slice(data);
    sector.resize(CD_SECTOR_SIZE, 0);
    sector
}

/// Returns the code after DOSINIT of the version test: a `NOP`, `INT 21h
/// AH=30h` with the store of `AX` at `RESULT_OFFSET + 8` and `HLT`.
fn version_code() -> Vec<u8> {
    let [result_low, result_high] = word(RESULT_OFFSET + 8);
    vec![
        0x90, // NOP
        0xB4,
        0x30, // MOV AH, 30h
        0xCD,
        0x21, // INT 21h
        0x2E,
        0xA3,
        result_low,
        result_high, // MOV CS:[RESULT+8], AX
        0xF4,        // HLT
    ]
}

/// Code after DOSINIT and the offset of its `NOP` that marks the media
/// change of a test.
struct MediaChangeCode {
    code: Vec<u8>,
    swap_point: u16,
    data: Vec<u8>,
}

/// Returns the code after DOSINIT of the floppy media change test: the free
/// clusters of A: from `INT 21h AH=36h` before and after the `NOP` at
/// `RESULT_OFFSET + 10` and `RESULT_OFFSET + 12`, then `HLT`.
fn floppy_free_space_code() -> MediaChangeCode {
    let [first_low, first_high] = word(RESULT_OFFSET + 10);
    let [second_low, second_high] = word(RESULT_OFFSET + 12);
    let mut code = vec![
        0x0E, 0x1F, // PUSH CS; POP DS
        0xB4, 0x36, 0xB2, 0x01, 0xCD, 0x21, // MOV AH, 36h; MOV DL, 1; INT 21h
        0x89, 0x1E, first_low, first_high, // MOV [RESULT+10], BX
    ];
    let swap_point = code.len() as u16;
    code.extend_from_slice(&[
        0x90, // NOP
        0xB4,
        0x36,
        0xB2,
        0x01,
        0xCD,
        0x21, // MOV AH, 36h; MOV DL, 1; INT 21h
        0x89,
        0x1E,
        second_low,
        second_high, // MOV [RESULT+12], BX
        0xF4,        // HLT
    ]);
    MediaChangeCode {
        code,
        swap_point,
        data: Vec::new(),
    }
}

/// Returns the code after DOSINIT of the CD media change test. It installs
/// the HLE MSCDEX with `MSCDEX_TAIL` in the current PSP and stores its exit
/// code at `RESULT_OFFSET + 10`. It opens `CD_FILE_PATH`, stores the handle
/// at `RESULT_OFFSET + 12`, reads `CD_FIRST_READ` bytes and stores the count
/// at `RESULT_OFFSET + 14`. Behind the `NOP` it reads the rest of the file, stores the count at
/// `RESULT_OFFSET + 16` and halts. The data holds the tail and the path.
fn cd_read_code() -> MediaChangeCode {
    let mut data = vec![MSCDEX_TAIL.len() as u8];
    data.extend_from_slice(MSCDEX_TAIL);
    data.push(0x0D);
    let path_offset = IPL_DATA_OFFSET + data.len() as u16;
    data.extend_from_slice(CD_FILE_PATH);

    let [tail_low, tail_high] = word(IPL_DATA_OFFSET);
    let [length_low, length_high] = word(MSCDEX_TAIL.len() as u16 + 2);
    let [path_low, path_high] = word(path_offset);
    let [exit_low, exit_high] = word(RESULT_OFFSET + 10);
    let [handle_low, handle_high] = word(RESULT_OFFSET + 12);
    let [first_low, first_high] = word(RESULT_OFFSET + 14);
    let [second_low, second_high] = word(RESULT_OFFSET + 16);
    let [buffer_low, buffer_high] = word(FIRST_BUFFER_OFFSET);
    let [rest_low, rest_high] = word(SECOND_BUFFER_OFFSET);
    let [count_low, count_high] = word(CD_FIRST_READ);
    let mut code = vec![
        0x0E,
        0x1F, // PUSH CS; POP DS
        0xB4,
        0x62,
        0xCD,
        0x21, // MOV AH, 62h; INT 21h
        0x8E,
        0xC3, // MOV ES, BX
        0xBE,
        tail_low,
        tail_high, // MOV SI, tail
        0xBF,
        0x80,
        0x00, // MOV DI, 0080h
        0xB9,
        length_low,
        length_high, // MOV CX, length
        0xFC,
        0xF3,
        0xA4, // CLD; REP MOVSB
        0x9C,
        0x0E, // PUSHF; PUSH CS
        0xE8,
        0x02,
        0x00, // CALL trap
        0xEB,
        0x09, // JMP after the trap
        0x50,
        0x52, // trap: PUSH AX; PUSH DX
        0xBA,
        0xF0,
        0x07, // MOV DX, 07F0h
        0xB0,
        dos::DosTrapCode::MscdexInstall.code(), // MOV AL, trap code
        0xEE,
        0xCF, // OUT DX, AL; IRET
        0x2E,
        0xA2,
        exit_low,
        exit_high, // MOV CS:[RESULT+10], AL
        0x0E,
        0x1F, // PUSH CS; POP DS
        0xB8,
        0x00,
        0x3D, // MOV AX, 3D00h
        0xBA,
        path_low,
        path_high, // MOV DX, path
        0xCD,
        0x21, // INT 21h
        0x2E,
        0xA3,
        handle_low,
        handle_high, // MOV CS:[RESULT+12], AX
        0x89,
        0xC3, // MOV BX, AX
        0xB4,
        0x3F, // MOV AH, 3Fh
        0xB9,
        count_low,
        count_high, // MOV CX, CD_FIRST_READ
        0xBA,
        buffer_low,
        buffer_high, // MOV DX, first buffer
        0xCD,
        0x21, // INT 21h
        0x2E,
        0xA3,
        first_low,
        first_high, // MOV CS:[RESULT+14], AX
    ];
    let swap_point = code.len() as u16;
    code.extend_from_slice(&[
        0x90, // NOP
        0x2E,
        0x8B,
        0x1E,
        handle_low,
        handle_high, // MOV BX, CS:[RESULT+12]
        0xB4,
        0x3F, // MOV AH, 3Fh
        0xB9,
        0x10,
        0x00, // MOV CX, 10h
        0xBA,
        rest_low,
        rest_high, // MOV DX, second buffer
        0xCD,
        0x21, // INT 21h
        0x2E,
        0xA3,
        second_low,
        second_high, // MOV CS:[RESULT+16], AX
        0xF4,        // HLT
    ]);
    MediaChangeCode {
        code,
        swap_point,
        data,
    }
}

/// Writes a one-sector MODE1/2048 CD image with the IPL of the version test
/// and returns the folder that holds it.
fn write_cd_image() -> common::temporary::TemporaryFolder {
    write_cd(&ipl_sector(&version_code(), &[]))
}

/// Writes `image` as a MODE1/2048 CD image and returns the folder that
/// holds it.
fn write_cd(image: &[u8]) -> common::temporary::TemporaryFolder {
    let folder = common::temporary::TemporaryFolder::new("towns_hle_dos");
    folder.write_file("disc.iso", image);
    folder.write_file(
        "disc.cue",
        b"FILE \"disc.iso\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n",
    );
    folder
}

/// Returns an ISO 9660 image with `ipl` in sector 0 and `README.TXT` with
/// `CD_FILE_CONTENT` in the root directory.
fn iso_image(ipl: &[u8]) -> Vec<u8> {
    let mut image = vec![0u8; (CD_FILE_SECTOR + 1) * CD_SECTOR_SIZE];
    image[..ipl.len()].copy_from_slice(ipl);
    let both_endian_dword = |value: u32| {
        let mut bytes = [0u8; 8];
        bytes[..4].copy_from_slice(&value.to_le_bytes());
        bytes[4..].copy_from_slice(&value.to_be_bytes());
        bytes
    };
    let record = |name: &[u8], sector: usize, size: usize, flags: u8| {
        let length = 33 + name.len() + (name.len() + 1) % 2;
        let mut bytes = vec![0u8; length];
        bytes[0] = length as u8;
        bytes[2..10].copy_from_slice(&both_endian_dword(sector as u32));
        bytes[10..18].copy_from_slice(&both_endian_dword(size as u32));
        bytes[25] = flags;
        bytes[28] = 1;
        bytes[31] = 1;
        bytes[32] = name.len() as u8;
        bytes[33..33 + name.len()].copy_from_slice(name);
        bytes
    };

    let descriptor = 16 * CD_SECTOR_SIZE;
    image[descriptor] = 1;
    image[descriptor + 1..descriptor + 6].copy_from_slice(b"CD001");
    image[descriptor + 6] = 1;
    image[descriptor + 40..descriptor + 72].fill(b' ');
    image[descriptor + 80..descriptor + 88]
        .copy_from_slice(&both_endian_dword((CD_FILE_SECTOR + 1) as u32));
    image[descriptor + 128..descriptor + 130]
        .copy_from_slice(&(CD_SECTOR_SIZE as u16).to_le_bytes());
    image[descriptor + 130..descriptor + 132]
        .copy_from_slice(&(CD_SECTOR_SIZE as u16).to_be_bytes());
    let root = record(&[0], CD_ROOT_SECTOR, CD_SECTOR_SIZE, 2);
    image[descriptor + 156..descriptor + 156 + root.len()].copy_from_slice(&root);
    let terminator = 17 * CD_SECTOR_SIZE;
    image[terminator] = 0xFF;
    image[terminator + 1..terminator + 6].copy_from_slice(b"CD001");
    image[terminator + 6] = 1;

    let mut directory = record(&[0], CD_ROOT_SECTOR, CD_SECTOR_SIZE, 2);
    directory.extend(record(&[1], CD_ROOT_SECTOR, CD_SECTOR_SIZE, 2));
    directory.extend(record(
        b"README.TXT;1",
        CD_FILE_SECTOR,
        CD_FILE_CONTENT.len(),
        0,
    ));
    let root_start = CD_ROOT_SECTOR * CD_SECTOR_SIZE;
    image[root_start..root_start + directory.len()].copy_from_slice(&directory);
    let file_start = CD_FILE_SECTOR * CD_SECTOR_SIZE;
    image[file_start..file_start + CD_FILE_CONTENT.len()].copy_from_slice(CD_FILE_CONTENT);
    image
}

/// Returns an empty FAT12 floppy image in the HDM format with a BPB at `0Bh`
/// of the boot sector. The first `used_clusters` data clusters are marked as
/// allocated in both FATs.
fn floppy_image(used_clusters: u16) -> Vec<u8> {
    let mut image = vec![0u8; FLOPPY_SECTORS * FLOPPY_SECTOR_SIZE];
    image[0..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
    image[0x0B..0x0D].copy_from_slice(&(FLOPPY_SECTOR_SIZE as u16).to_le_bytes());
    image[0x0D] = 1;
    image[0x0E..0x10].copy_from_slice(&1u16.to_le_bytes());
    image[0x10] = 2;
    image[0x11..0x13].copy_from_slice(&(FLOPPY_ROOT_ENTRIES as u16).to_le_bytes());
    image[0x13..0x15].copy_from_slice(&(FLOPPY_SECTORS as u16).to_le_bytes());
    image[0x15] = 0xFE;
    image[0x16..0x18].copy_from_slice(&(FLOPPY_FAT_SECTORS as u16).to_le_bytes());
    image[0x18..0x1A].copy_from_slice(&8u16.to_le_bytes());
    image[0x1A..0x1C].copy_from_slice(&2u16.to_le_bytes());
    for fat in 0..2 {
        let start = (1 + fat * FLOPPY_FAT_SECTORS) * FLOPPY_SECTOR_SIZE;
        image[start..start + 3].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
        for cluster in 2..2 + usize::from(used_clusters) {
            let offset = start + cluster * 3 / 2;
            if cluster % 2 == 0 {
                image[offset] = 0xFF;
                image[offset + 1] |= 0x0F;
            } else {
                image[offset] |= 0xF0;
                image[offset + 1] = 0xFF;
            }
        }
    }
    image
}

/// Offset of the root directory in the OS ROM volume.
const OS_ROM_ROOT_DIRECTORY: usize = 0xA00;
/// Number of root directory entries of the OS ROM volume.
const OS_ROM_ROOT_ENTRIES: usize = 112;
/// Offset of cluster 2 in the OS ROM volume.
const OS_ROM_DATA_START: usize = 0x1800;
/// Bytes per cluster of the OS ROM volume.
const OS_ROM_CLUSTER_BYTES: usize = 0x400;

/// Returns the offset and the size of `MSDOS.SYS` in the FAT volume of the
/// HLE OS ROM. The volume has no boot sector BPB. Its geometry comes from the
/// OS ROM unit of `IO.SYS`.
fn kernel_location() -> (usize, usize) {
    let image = HLE_OS_ROM;
    let entry = (0..OS_ROM_ROOT_ENTRIES)
        .map(|index| OS_ROM_ROOT_DIRECTORY + index * 32)
        .find(|&entry| &image[entry..entry + 11] == b"MSDOS   SYS")
        .expect("MSDOS.SYS in the OS ROM");
    let cluster = usize::from(u16::from_le_bytes([image[entry + 26], image[entry + 27]]));
    let size = u32::from_le_bytes(image[entry + 28..entry + 32].try_into().unwrap()) as usize;
    (
        OS_ROM_DATA_START + (cluster - 2) * OS_ROM_CLUSTER_BYTES,
        size,
    )
}

/// Returns the bytes of `MSDOS.SYS` from the HLE OS ROM.
fn kernel_image() -> Vec<u8> {
    let (start, size) = kernel_location();
    HLE_OS_ROM[start..start + size].to_vec()
}

/// Runs single instructions until the CPU reaches `target` or `budget`
/// cycles pass. Returns whether the target was reached.
fn run_until_pc(
    machine: &mut TownsMachine<{ cpu::CPU_MODEL_486_DX }>,
    target: u32,
    budget: u64,
) -> bool {
    let end = machine.bus.current_cycle() + budget;
    while machine.bus.current_cycle() < end {
        if linear_pc(machine) == target {
            return true;
        }
        machine.run_for(1);
    }
    false
}

/// Runs the machine until the CPU halts or `budget` cycles pass.
fn run_until_halt(machine: &mut TownsMachine<{ cpu::CPU_MODEL_486_DX }>, budget: u64) {
    let end = machine.bus.current_cycle() + budget;
    while !machine.cpu.halted() && machine.bus.current_cycle() < end {
        machine.run_for(1);
    }
    assert!(machine.cpu.halted(), "the IPL did not reach its HLT");
}

/// Reads the word at `segment:offset`.
fn read_word(
    machine: &mut TownsMachine<{ cpu::CPU_MODEL_486_DX }>,
    segment: u16,
    offset: u16,
) -> u16 {
    let address = linear(segment, offset);
    u16::from(machine.bus.read_byte(address)) | (u16::from(machine.bus.read_byte(address + 1)) << 8)
}

/// Builds an HLE machine with the test CD and boots it to the IPL entry with
/// the device chain, `MSDOS.SYS` and the `INT 29h` vector in memory, like
/// `IO.SYS` leaves them before DOSINIT.
fn boot_to_ipl(
    folder: &common::temporary::TemporaryFolder,
) -> TownsMachine<{ cpu::CPU_MODEL_486_DX }> {
    let mut machine = machine_mx_hle();
    machine
        .insert_cdrom(&folder.path().join("disc.cue"))
        .expect("CD image");
    machine.set_boot_device(TownsBootDevice::Cd);
    assert!(run_until_pc(
        &mut machine,
        linear(IPL_SEGMENT, IPL_ENTRY_OFFSET),
        PHASE_CYCLES
    ));
    let mut write = |address: u32, bytes: &[u8]| {
        for (index, &byte) in bytes.iter().enumerate() {
            machine.bus.write_byte(address + index as u32, byte);
        }
    };
    write(linear(IOSYS_SEGMENT, 0), KERNEL_DRIVERS);
    write(linear(KERNEL_SEGMENT, 0), &kernel_image());
    let [entry_low, entry_high] = IOSYS_INT29_ENTRY.to_le_bytes();
    let [segment_low, segment_high] = IOSYS_SEGMENT.to_le_bytes();
    write(
        0x29 * 4,
        &[entry_low, entry_high, segment_low, segment_high],
    );
    machine
}

#[test]
fn msdos_sys_of_the_hle_os_rom_enters_neetan_dos_and_returns_to_sysinit() {
    let folder = write_cd_image();
    let mut machine = boot_to_ipl(&folder);
    run_until_halt(&mut machine, PHASE_CYCLES);

    assert_eq!(
        read_word(&mut machine, IPL_SEGMENT, RESULT_OFFSET),
        KERNEL_SEGMENT
    );
    assert_eq!(
        read_word(&mut machine, IPL_SEGMENT, RESULT_OFFSET + 2),
        LIST_OF_LISTS_OFFSET
    );
    let first_mcb = read_word(&mut machine, KERNEL_SEGMENT, LIST_OF_LISTS_OFFSET - 2);
    assert_eq!(
        read_word(&mut machine, IPL_SEGMENT, RESULT_OFFSET + 4),
        first_mcb + 1
    );
    assert_eq!(
        read_word(&mut machine, IPL_SEGMENT, RESULT_OFFSET + 6),
        first_mcb + 1
    );
    assert_eq!(machine.bus.read_byte(linear(first_mcb, 0)), b'Z');
    assert_eq!(
        read_word(&mut machine, IPL_SEGMENT, RESULT_OFFSET + 8),
        DOS_VERSION
    );
}

#[test]
fn save_state_with_an_initialized_neetan_dos_restores_into_a_fresh_machine() {
    let folder = write_cd_image();
    let mut machine = boot_to_ipl(&folder);
    let after_dosinit = linear(IPL_SEGMENT, after_dosinit_offset());
    assert!(run_until_pc(&mut machine, after_dosinit, PHASE_CYCLES));
    let saved = machine.capture_state().expect("capture");

    let mut restored = machine_mx_hle();
    restored
        .insert_cdrom(&folder.path().join("disc.cue"))
        .expect("CD image");
    restored.restore_state(&saved).expect("restore");
    assert_eq!(saved.payload(), restored.capture_state().unwrap().payload());

    run_until_halt(&mut machine, PHASE_CYCLES);
    run_until_halt(&mut restored, PHASE_CYCLES);
    assert_eq!(
        read_word(&mut restored, IPL_SEGMENT, RESULT_OFFSET + 8),
        DOS_VERSION
    );
    assert_eq!(
        machine.capture_state().unwrap().payload(),
        restored.capture_state().unwrap().payload()
    );
}

/// Runs `machine` to the `NOP` of `code`.
fn run_to_swap_point(
    machine: &mut TownsMachine<{ cpu::CPU_MODEL_486_DX }>,
    code: &MediaChangeCode,
) {
    let swap_point = linear(IPL_SEGMENT, after_dosinit_offset() + code.swap_point);
    assert!(run_until_pc(machine, swap_point, PHASE_CYCLES));
}

#[test]
fn inserting_a_floppy_drops_the_volume_of_the_old_disk() {
    let code = floppy_free_space_code();
    let folder = write_cd(&ipl_sector(&code.code, &code.data));
    let mut machine = boot_to_ipl(&folder);
    let empty = floppy_image(0);
    machine
        .insert_floppy(
            0,
            MediaImage {
                name: "empty.hdm",
                bytes: &empty,
            },
            MediaBacking::Ram,
        )
        .expect("floppy image");
    run_to_swap_point(&mut machine, &code);
    let used = floppy_image(FLOPPY_USED_CLUSTERS);
    machine
        .insert_floppy(
            0,
            MediaImage {
                name: "used.hdm",
                bytes: &used,
            },
            MediaBacking::Ram,
        )
        .expect("floppy image");
    run_until_halt(&mut machine, PHASE_CYCLES);

    assert_eq!(
        read_word(&mut machine, IPL_SEGMENT, RESULT_OFFSET + 10),
        FLOPPY_CLUSTERS
    );
    assert_eq!(
        read_word(&mut machine, IPL_SEGMENT, RESULT_OFFSET + 12),
        FLOPPY_CLUSTERS - FLOPPY_USED_CLUSTERS
    );
}

#[test]
fn ejecting_the_cd_drops_the_open_files_of_the_disc() {
    let code = cd_read_code();
    let folder = write_cd(&iso_image(&ipl_sector(&code.code, &code.data)));
    let mut machine = boot_to_ipl(&folder);
    run_to_swap_point(&mut machine, &code);
    assert_eq!(
        read_word(&mut machine, IPL_SEGMENT, RESULT_OFFSET + 10) & 0xFF,
        0
    );
    assert_eq!(
        read_word(&mut machine, IPL_SEGMENT, RESULT_OFFSET + 14),
        CD_FIRST_READ
    );
    let first_read = linear(IPL_SEGMENT, FIRST_BUFFER_OFFSET);
    let read: Vec<u8> = (0..u32::from(CD_FIRST_READ))
        .map(|index| machine.bus.read_byte(first_read + index))
        .collect();
    assert_eq!(read, CD_FILE_CONTENT[..usize::from(CD_FIRST_READ)]);

    machine.eject_cdrom();
    run_until_halt(&mut machine, PHASE_CYCLES);
    assert_eq!(read_word(&mut machine, IPL_SEGMENT, RESULT_OFFSET + 16), 0);
}

/// Segment of the PSP that the SYSINIT part of the boot IPL creates.
const SYSINIT_PSP_SEGMENT: u16 = 0xB200;
/// Segment of the RAM disk of the device chain.
const RAMDISK_SEGMENT: u16 = 0xA000;
/// Bytes per sector of the RAM disk.
const RAMDISK_SECTOR_SIZE: usize = 512;
/// Sectors of the RAM disk.
const RAMDISK_SECTORS: usize = 128;
/// Sector of the root directory of the RAM disk.
const RAMDISK_ROOT_SECTOR: usize = 3;
/// Sector of cluster 2 of the RAM disk.
const RAMDISK_DATA_SECTOR: usize = 4;
/// CD sector of the device chain in the boot image.
const BOOT_IOSYS_SECTOR: u32 = 1;
/// First CD sector of the RAM disk in the boot image.
const BOOT_RAMDISK_SECTOR: u32 = 2;
/// Disk BIOS device code of the CD-ROM drive.
const DISK_BIOS_CDROM: u8 = 0xC0;
/// Disk BIOS device code of the OS ROM window.
const DISK_BIOS_OS_ROM: u8 = 0x42;
/// Segment of the CDS array that the boot IPL builds.
const CDS_SEGMENT: u16 = 0xB300;
/// Entries of the CDS array, `max(units, 5)` like SYSINIT.
const CDS_ENTRIES: u8 = 5;
/// Size of a DOS 3.10 CDS entry.
const CDS_ENTRY_SIZE: u8 = 0x51;
/// Bytes per block of the disk BIOS memory windows.
const MEMORY_WINDOW_BLOCK: usize = 1024;
/// Offset of the stack top of the boot IPL in the IPL segment.
const BOOT_STACK_OFFSET: u16 = 0x0F00;
/// Offset of the EXEC parameter block in the IPL segment.
const EXEC_BLOCK_OFFSET: u16 = IPL_DATA_OFFSET;
/// Offset of the shell path in the IPL segment.
const EXEC_PATH_OFFSET: u16 = IPL_DATA_OFFSET + 0x10;
/// Offset of the shell command tail in the IPL segment.
const EXEC_TAIL_OFFSET: u16 = IPL_DATA_OFFSET + 0x20;
/// Path of the shell. C: is the OS ROM unit of the device chain.
const SHELL_PATH: &[u8] = b"C:\\COMMAND.COM\0";
/// Command tail of the shell, as `IO.SYS` passes it from `SHELL=`.
const SHELL_TAIL: &[u8] = b" C:\\ /P";
/// Content of `AUTOEXEC.BAT` on the RAM disk.
const AUTOEXEC: &[u8] = b"@ECHO AUTOSTART\r\n";
/// Prompt of the shell on the RAM disk.
const SHELL_PROMPT: &[u8] = b"D:\\>";
/// Offset of the output count word of the device chain.
const OUTPUT_COUNT: u16 = 0x0104;
/// Offset of the input count word of the device chain.
const INPUT_COUNT: u16 = 0x0106;
/// Offset of the input position word of the device chain.
const INPUT_POSITION: u16 = 0x0108;
/// Offset of the input buffer of the device chain.
const INPUT_BUFFER: u16 = 0x0220;
/// Offset of the output log of the device chain.
const OUTPUT_LOG: u16 = 0x0240;
/// Size of the output log ring of the device chain.
const OUTPUT_LOG_SIZE: usize = 0x200;
/// Cycles of one run slice of the shell tests.
const RUN_SLICE: u64 = 100_000;
/// Cycle budget of the boot up to the shell prompt.
const SHELL_BOOT_CYCLES: u64 = 200_000_000;
/// Cycles that the shell gets for a command line.
const COMMAND_CYCLES: u64 = 20_000_000;

/// Returns the code of a disk BIOS read of `count` units at `position` of
/// `device` to `segment:0000`. A failed read halts the CPU.
fn disk_bios_read(device: u8, position: u32, count: u16, segment: u16) -> Vec<u8> {
    let [segment_low, segment_high] = word(segment);
    let [position_low, position_high] = word(position as u16);
    let [upper_low, upper_high] = word((position >> 16) as u16);
    let [count_low, count_high] = word(count);
    vec![
        0xB8,
        segment_low,
        segment_high, // MOV AX, segment
        0x8E,
        0xD8, // MOV DS, AX
        0x31,
        0xFF, // XOR DI, DI
        0xB8,
        device,
        0x05, // MOV AX, 05xxh (read)
        0xB9,
        upper_low,
        upper_high, // MOV CX, position high word
        0xBA,
        position_low,
        position_high, // MOV DX, position low word
        0xBB,
        count_low,
        count_high, // MOV BX, count
        0x9A,
        0x14,
        0x00,
        0xFB,
        0xFF, // CALL FAR FFFB:0014
        0x73,
        0x01, // JNC +1
        0xF4, // HLT
    ]
}

/// Returns the code that builds the CDS array like SYSINIT and `IO.SYS`
/// after DOSINIT: `CDS_ENTRIES` entries at `CDS_SEGMENT`, `X:\` with the
/// physical flag, the DPB, cluster `FFFFh` and `FFFF:FFFF` at `4Bh` for
/// every drive of the DPB chain.
fn cds_code() -> Vec<u8> {
    let [cds_low, cds_high] = word(CDS_SEGMENT);
    let [size_low, size_high] = word(CDS_ENTRIES as u16 * CDS_ENTRY_SIZE as u16);
    let [kernel_low, kernel_high] = word(KERNEL_SEGMENT);
    let [lol_low, lol_high] = word(LIST_OF_LISTS_OFFSET);
    let [array_low, array_high] = word(LIST_OF_LISTS_OFFSET + 0x16);
    let [array_segment_low, array_segment_high] = word(LIST_OF_LISTS_OFFSET + 0x18);
    let [last_low, last_high] = word(LIST_OF_LISTS_OFFSET + 0x21);
    vec![
        0xB8,
        cds_low,
        cds_high, // MOV AX, CDS_SEGMENT
        0x8E,
        0xC0, // MOV ES, AX
        0x31,
        0xFF, // XOR DI, DI
        0xB9,
        size_low,
        size_high, // MOV CX, array size
        0x30,
        0xC0, // XOR AL, AL
        0xFC,
        0xF3,
        0xAA, // CLD; REP STOSB
        0xB8,
        kernel_low,
        kernel_high, // MOV AX, KERNEL_SEGMENT
        0x8E,
        0xD8, // MOV DS, AX
        0xC5,
        0x36,
        lol_low,
        lol_high, // LDS SI, [LoL+00h] (first DPB)
        0x83,
        0xFE,
        0xFF, // loop: CMP SI, FFFFh
        0x74,
        0x40, // JE done
        0x8A,
        0x04, // MOV AL, [SI] (drive index)
        0xB3,
        CDS_ENTRY_SIZE, // MOV BL, CDS_ENTRY_SIZE
        0xF6,
        0xE3, // MUL BL
        0x89,
        0xC3, // MOV BX, AX
        0x8A,
        0x04, // MOV AL, [SI]
        0x04,
        b'A', // ADD AL, 'A'
        0x26,
        0x88,
        0x07, // MOV ES:[BX], AL
        0x26,
        0xC7,
        0x47,
        0x01,
        b':',
        b'\\', // MOV word ES:[BX+01h], ":\"
        0x26,
        0xC7,
        0x47,
        0x43,
        0x00,
        0x40, // MOV word ES:[BX+43h], 4000h
        0x26,
        0x89,
        0x77,
        0x45, // MOV ES:[BX+45h], SI
        0x26,
        0x8C,
        0x5F,
        0x47, // MOV ES:[BX+47h], DS
        0x26,
        0xC7,
        0x47,
        0x49,
        0xFF,
        0xFF, // MOV word ES:[BX+49h], FFFFh
        0x26,
        0xC7,
        0x47,
        0x4B,
        0xFF,
        0xFF, // MOV word ES:[BX+4Bh], FFFFh
        0x26,
        0xC7,
        0x47,
        0x4D,
        0xFF,
        0xFF, // MOV word ES:[BX+4Dh], FFFFh
        0x26,
        0xC7,
        0x47,
        0x4F,
        0x02,
        0x00, // MOV word ES:[BX+4Fh], 2
        0xC5,
        0x74,
        0x18, // LDS SI, [SI+18h] (next DPB)
        0xEB,
        0xBB, // JMP loop
        0xB8,
        kernel_low,
        kernel_high, // done: MOV AX, KERNEL_SEGMENT
        0x8E,
        0xD8, // MOV DS, AX
        0xC7,
        0x06,
        array_low,
        array_high,
        0x00,
        0x00, // MOV word [LoL+16h], 0
        0xC7,
        0x06,
        array_segment_low,
        array_segment_high,
        cds_low,
        cds_high, // MOV word [LoL+18h], CDS_SEGMENT
        0xC6,
        0x06,
        last_low,
        last_high,
        CDS_ENTRIES, // MOV byte [LoL+21h], CDS_ENTRIES
    ]
}

/// Returns the boot IPL of the shell tests. It plays the part of the
/// `IO.SYS` loader: it reads the device chain and the RAM disk from the CD
/// and `MSDOS.SYS` from the OS ROM through the disk BIOS of the HLE system
/// ROM, sets the `INT 29h` vector and calls DOSINIT. Then it plays the part of
/// SYSINIT: it builds the CDS array, creates its PSP with `AH=55h`, selects D:
/// and runs the shell with `AX=4B00h`.
fn shell_boot_ipl() -> Vec<u8> {
    let (kernel_offset, kernel_size) = kernel_location();
    let [ipl_low, ipl_high] = word(IPL_SEGMENT);
    let [stack_low, stack_high] = word(BOOT_STACK_OFFSET);
    let [int29_low, int29_high] = word(IOSYS_INT29_ENTRY);
    let [iosys_low, iosys_high] = word(IOSYS_SEGMENT);
    let [psp_low, psp_high] = word(SYSINIT_PSP_SEGMENT);
    let [top_low, top_high] = word(MEMORY_TOP);
    let [path_low, path_high] = word(EXEC_PATH_OFFSET);
    let [block_low, block_high] = word(EXEC_BLOCK_OFFSET);

    let mut code = b"IPL4".to_vec();
    code.extend_from_slice(&[
        0xB8, ipl_low, ipl_high, // MOV AX, IPL_SEGMENT
        0x8E, 0xD0, // MOV SS, AX
        0xBC, stack_low, stack_high, // MOV SP, BOOT_STACK_OFFSET
    ]);
    code.extend(disk_bios_read(
        DISK_BIOS_CDROM,
        BOOT_IOSYS_SECTOR,
        1,
        IOSYS_SEGMENT,
    ));
    code.extend(disk_bios_read(
        DISK_BIOS_CDROM,
        BOOT_RAMDISK_SECTOR,
        (RAMDISK_SECTORS * RAMDISK_SECTOR_SIZE / CD_SECTOR_SIZE) as u16,
        RAMDISK_SEGMENT,
    ));
    code.extend(disk_bios_read(
        DISK_BIOS_OS_ROM,
        (kernel_offset / MEMORY_WINDOW_BLOCK) as u32,
        kernel_size.div_ceil(MEMORY_WINDOW_BLOCK) as u16,
        KERNEL_SEGMENT,
    ));
    code.extend_from_slice(&[
        0x31, 0xC0, // XOR AX, AX
        0x8E, 0xC0, // MOV ES, AX
        0x26, 0xC7, 0x06, 0xA4, 0x00, int29_low,
        int29_high, // MOV ES:[00A4h], IOSYS_INT29_ENTRY
        0x26, 0xC7, 0x06, 0xA6, 0x00, iosys_low, iosys_high, // MOV ES:[00A6h], IOSYS_SEGMENT
    ]);
    code.extend(dosinit_call());
    code.extend(cds_code());
    code.extend_from_slice(&[
        0x0E, 0x1F, // PUSH CS; POP DS
        0xBA, psp_low, psp_high, // MOV DX, SYSINIT_PSP_SEGMENT
        0xBE, top_low, top_high, // MOV SI, MEMORY_TOP
        0xB4, 0x55, // MOV AH, 55h
        0xCD, 0x21, // INT 21h
        0xBB, psp_low, psp_high, // MOV BX, SYSINIT_PSP_SEGMENT
        0xB4, 0x50, // MOV AH, 50h
        0xCD, 0x21, // INT 21h
        0xB2, 0x03, // MOV DL, 3
        0xB4, 0x0E, // MOV AH, 0Eh
        0xCD, 0x21, // INT 21h
        0x0E, 0x1F, // PUSH CS; POP DS
        0x0E, 0x07, // PUSH CS; POP ES
        0xBA, path_low, path_high, // MOV DX, EXEC_PATH_OFFSET
        0xBB, block_low, block_high, // MOV BX, EXEC_BLOCK_OFFSET
        0xB8, 0x00, 0x4B, // MOV AX, 4B00h
        0xCD, 0x21, // INT 21h
        0xF4, // HLT
    ]);
    assert!(code.len() <= usize::from(IPL_DATA_OFFSET));
    code.resize(usize::from(IPL_DATA_OFFSET), 0);

    let mut block = vec![0u8; 14];
    block[2..4].copy_from_slice(&EXEC_TAIL_OFFSET.to_le_bytes());
    block[4..6].copy_from_slice(&IPL_SEGMENT.to_le_bytes());
    block[6..8].copy_from_slice(&0x005Cu16.to_le_bytes());
    block[8..10].copy_from_slice(&SYSINIT_PSP_SEGMENT.to_le_bytes());
    block[10..12].copy_from_slice(&0x006Cu16.to_le_bytes());
    block[12..14].copy_from_slice(&SYSINIT_PSP_SEGMENT.to_le_bytes());
    code.extend(block);
    code.resize(usize::from(EXEC_PATH_OFFSET), 0);
    code.extend_from_slice(SHELL_PATH);
    code.resize(usize::from(EXEC_TAIL_OFFSET), 0);
    code.push(SHELL_TAIL.len() as u8);
    code.extend_from_slice(SHELL_TAIL);
    code.push(0x0D);
    code.resize(CD_SECTOR_SIZE, 0);
    code
}

/// Returns a FAT12 RAM disk image of 512-byte sectors with `AUTOEXEC.BAT`
/// in cluster 2.
fn ramdisk_image() -> Vec<u8> {
    let mut image = vec![0u8; RAMDISK_SECTORS * RAMDISK_SECTOR_SIZE];
    image[0..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
    image[0x0B..0x0D].copy_from_slice(&(RAMDISK_SECTOR_SIZE as u16).to_le_bytes());
    image[0x0D] = 1;
    image[0x0E..0x10].copy_from_slice(&1u16.to_le_bytes());
    image[0x10] = 2;
    image[0x11..0x13].copy_from_slice(&16u16.to_le_bytes());
    image[0x13..0x15].copy_from_slice(&(RAMDISK_SECTORS as u16).to_le_bytes());
    image[0x15] = 0xF8;
    image[0x16..0x18].copy_from_slice(&1u16.to_le_bytes());
    image[0x1FE] = 0x55;
    image[0x1FF] = 0xAA;
    for fat in 1..3 {
        let start = fat * RAMDISK_SECTOR_SIZE;
        image[start..start + 5].copy_from_slice(&[0xF8, 0xFF, 0xFF, 0xFF, 0x0F]);
    }
    let entry = RAMDISK_ROOT_SECTOR * RAMDISK_SECTOR_SIZE;
    image[entry..entry + 11].copy_from_slice(b"AUTOEXECBAT");
    image[entry + 0x0B] = 0x20;
    image[entry + 0x1A..entry + 0x1C].copy_from_slice(&2u16.to_le_bytes());
    image[entry + 0x1C..entry + 0x20].copy_from_slice(&(AUTOEXEC.len() as u32).to_le_bytes());
    let data = RAMDISK_DATA_SECTOR * RAMDISK_SECTOR_SIZE;
    image[data..data + AUTOEXEC.len()].copy_from_slice(AUTOEXEC);
    image
}

/// Writes the CD of the shell tests: the boot IPL, the device chain and the
/// RAM disk.
fn write_shell_cd() -> common::temporary::TemporaryFolder {
    let mut image = shell_boot_ipl();
    let mut chain = KERNEL_DRIVERS.to_vec();
    chain.resize(CD_SECTOR_SIZE, 0);
    image.extend(chain);
    image.extend(ramdisk_image());
    write_cd(&image)
}

/// Returns the bytes that reached CON or `INT 29h` of the device chain.
fn console_output(machine: &mut TownsMachine<{ cpu::CPU_MODEL_486_DX }>) -> Vec<u8> {
    let count = usize::from(read_word(machine, IOSYS_SEGMENT, OUTPUT_COUNT)).min(OUTPUT_LOG_SIZE);
    let start = linear(IOSYS_SEGMENT, OUTPUT_LOG);
    (0..count as u32)
        .map(|index| machine.bus.read_byte(start + index))
        .collect()
}

/// Hands `input` to the CON input of the device chain.
fn type_input(machine: &mut TownsMachine<{ cpu::CPU_MODEL_486_DX }>, input: &[u8]) {
    let buffer = linear(IOSYS_SEGMENT, INPUT_BUFFER);
    for (index, &byte) in input.iter().enumerate() {
        machine.bus.write_byte(buffer + index as u32, byte);
    }
    let mut write_word = |offset: u16, value: u16| {
        let [low, high] = value.to_le_bytes();
        machine.bus.write_byte(linear(IOSYS_SEGMENT, offset), low);
        machine
            .bus
            .write_byte(linear(IOSYS_SEGMENT, offset) + 1, high);
    };
    write_word(INPUT_COUNT, input.len() as u16);
    write_word(INPUT_POSITION, 0);
}

/// Returns whether the console output contains `expected`.
fn output_contains(machine: &mut TownsMachine<{ cpu::CPU_MODEL_486_DX }>, expected: &[u8]) -> bool {
    console_output(machine)
        .windows(expected.len())
        .any(|window| window == expected)
}

/// Runs the machine in slices until the console output contains `expected`
/// or `budget` cycles pass. Returns whether the output appeared.
fn run_until_output(
    machine: &mut TownsMachine<{ cpu::CPU_MODEL_486_DX }>,
    expected: &[u8],
    budget: u64,
) -> bool {
    let end = machine.bus.current_cycle() + budget;
    while machine.bus.current_cycle() < end {
        if output_contains(machine, expected) {
            return true;
        }
        machine.run_for(RUN_SLICE);
    }
    output_contains(machine, expected)
}

/// Runs the machine for `cycles` cycles in slices.
fn run_slices(machine: &mut TownsMachine<{ cpu::CPU_MODEL_486_DX }>, cycles: u64) {
    for _ in 0..cycles / RUN_SLICE {
        machine.run_for(RUN_SLICE);
    }
}

/// Builds an HLE machine with the shell CD and boots it to the shell prompt.
fn boot_to_shell(
    folder: &common::temporary::TemporaryFolder,
) -> TownsMachine<{ cpu::CPU_MODEL_486_DX }> {
    let mut machine = machine_mx_hle();
    machine
        .insert_cdrom(&folder.path().join("disc.cue"))
        .expect("CD image");
    machine.set_boot_device(TownsBootDevice::Cd);
    assert!(
        run_until_output(&mut machine, SHELL_PROMPT, SHELL_BOOT_CYCLES),
        "no shell prompt: {:?}",
        String::from_utf8_lossy(&console_output(&mut machine))
    );
    machine
}

#[test]
fn synthetic_io_sys_boots_through_the_hle_roms_to_the_shell() {
    let folder = write_shell_cd();
    let mut machine = boot_to_shell(&folder);
    assert!(!machine.cpu.halted(), "the IPL stopped at a failed call");
    assert_eq!(
        read_word(&mut machine, IPL_SEGMENT, RESULT_OFFSET),
        KERNEL_SEGMENT
    );
    let output = String::from_utf8_lossy(&console_output(&mut machine)).into_owned();
    assert!(output.contains("AUTOSTART\r\n"), "{output:?}");
    assert!(!output.contains("@ECHO"), "{output:?}");

    type_input(&mut machine, b"VER\r");
    assert!(
        run_until_output(&mut machine, b"3.10", COMMAND_CYCLES),
        "{:?}",
        String::from_utf8_lossy(&console_output(&mut machine))
    );
}

#[test]
fn save_state_of_a_running_shell_continues_identically_after_a_restore() {
    let folder = write_shell_cd();
    let mut machine = boot_to_shell(&folder);
    let saved = machine.capture_state().expect("capture");

    let mut restored = machine_mx_hle();
    restored
        .insert_cdrom(&folder.path().join("disc.cue"))
        .expect("CD image");
    restored.restore_state(&saved).expect("restore");
    assert_eq!(saved.payload(), restored.capture_state().unwrap().payload());

    for running in [&mut machine, &mut restored] {
        type_input(running, b"ECHO RESTORED\r");
        run_slices(running, COMMAND_CYCLES);
    }
    assert!(
        output_contains(&mut restored, b"\nRESTORED\r\n"),
        "{:?}",
        String::from_utf8_lossy(&console_output(&mut restored))
    );
    assert_eq!(console_output(&mut machine), console_output(&mut restored));
    assert_eq!(
        machine.capture_state().unwrap().payload(),
        restored.capture_state().unwrap().payload()
    );
}
