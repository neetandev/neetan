//! Block units of a guest driver of the `IO.SYS` chain: volume load, file
//! read, media check, media change and write back.

use common::Cpu as _;

use crate::{
    harness::GuestMachine,
    kernel_harness::{
        CDS_ENTRY_SIZE, CDS_SEGMENT, DPB_NEXT, IOSYS_SEGMENT, MEDIA_CHANGE, RAMDISK_DATA_SECTOR,
        RAMDISK_HEADER, RAMDISK_SECTOR_SIZE, RAMDISK_SEGMENT, SYSINIT_SEGMENT, TAG_RAMDISK,
        WRITE_COUNT, WRITE_FAULT, booted_kernel, clear_request_log, driver_address, iso_image,
        linear, list_of_lists, parameter_block, ramdisk_image, request_log, run,
    },
};

/// Sectors of the RAM disk.
const RAMDISK_SECTORS: usize = 128;

/// Program buffer of the BPB for `AH=53h`.
const BPB_BUFFER: u16 = 0x0700;

/// Program buffer of the DPB of a `CONFIG.SYS` block driver.
const NEW_PARAMETER_BLOCK: u16 = 0x0800;

/// Drive index of E:.
const DRIVE_E: u8 = 4;

/// Size of the first disk buffer: the 1024-byte sectors of the floppy units.
const DISK_BUFFER_SIZE: usize = 0x400;

/// Returns a machine whose RAM disk (drive D:) holds `HELLO.TXT` with `content`.
fn machine(content: &[u8]) -> GuestMachine {
    booted_kernel(
        None,
        &iso_image(b"MENU", &[]),
        &ramdisk_image(&[(b"HELLO   TXT", content)]),
    )
}

/// Opens `path`, reads up to 64 bytes and returns them.
pub fn read_file(machine: &mut GuestMachine, path: &[u8]) -> Vec<u8> {
    let mut name = path.to_vec();
    name.push(0);
    machine.write_bytes(linear(SYSINIT_SEGMENT, 0x0200), &name);
    machine.write_bytes(linear(SYSINIT_SEGMENT, 0x0300), &[0; 64]);
    run(
        machine,
        &[
            0xB8, 0x00, 0x3D, // MOV AX,3D00h
            0xBA, 0x00, 0x02, // MOV DX,0200h
            0xCD, 0x21, // INT 21h
            0x72, 0x0B, // JC done
            0x89, 0xC3, // MOV BX,AX
            0xB4, 0x3F, // MOV AH,3Fh
            0xB9, 0x40, 0x00, // MOV CX,64
            0xBA, 0x00, 0x03, // MOV DX,0300h
            0xCD, 0x21, // INT 21h
            0xF4, // done: HLT
        ],
    );
    assert_eq!(machine.cpu.flags() & 1, 0, "open and read succeed");
    let count = usize::from(machine.cpu.ax());
    let start = linear(SYSINIT_SEGMENT, 0x0300) as usize;
    machine.bus.ram[start..start + count].to_vec()
}

/// Returns the commands that the RAM disk received.
fn ramdisk_commands(machine: &mut GuestMachine) -> Vec<u8> {
    request_log(machine)
        .into_iter()
        .filter(|(device, _)| *device == TAG_RAMDISK)
        .map(|(_, command)| command)
        .collect()
}

#[test]
fn guest_unit_mounts_and_reads_a_file() {
    let mut machine = machine(b"guest block data");
    clear_request_log(&mut machine);
    assert_eq!(
        read_file(&mut machine, b"D:\\HELLO.TXT"),
        b"guest block data"
    );
    // Read of the first FAT sector, build BPB, one read per disk buffer of
    // sectors that ends at the end of each area: the reserved sector, the
    // two FAT sectors, the root directory sector and the data sectors.
    let commands = ramdisk_commands(&mut machine);
    let data_sectors = RAMDISK_SECTORS - RAMDISK_DATA_SECTOR;
    assert_eq!(
        commands.len(),
        2 + 3 + data_sectors * RAMDISK_SECTOR_SIZE / DISK_BUFFER_SIZE
    );
    assert_eq!(commands[1], 0x02);
    assert!(
        commands
            .iter()
            .enumerate()
            .all(|(index, &command)| index == 1 || command == 0x04)
    );
    assert_eq!(machine.indos(), 0);
}

#[test]
fn a_later_open_checks_the_medium() {
    let mut machine = machine(b"first");
    read_file(&mut machine, b"D:\\HELLO.TXT");
    clear_request_log(&mut machine);
    assert_eq!(read_file(&mut machine, b"D:\\HELLO.TXT"), b"first");
    assert_eq!(ramdisk_commands(&mut machine), [0x01]);
}

#[test]
fn changed_medium_loads_the_volume_again() {
    let mut machine = machine(b"before");
    assert_eq!(read_file(&mut machine, b"D:\\HELLO.TXT"), b"before");
    machine.write_bytes(
        linear(RAMDISK_SEGMENT, 0),
        &ramdisk_image(&[(b"HELLO   TXT", b"after!")]),
    );
    machine.write_bytes(driver_address(MEDIA_CHANGE), &[0xFF]);
    assert_eq!(read_file(&mut machine, b"D:\\HELLO.TXT"), b"after!");
}

#[test]
fn medium_of_unknown_state_loads_the_volume_again() {
    let mut machine = machine(b"before");
    assert_eq!(read_file(&mut machine, b"D:\\HELLO.TXT"), b"before");
    machine.write_bytes(
        linear(RAMDISK_SEGMENT, 0),
        &ramdisk_image(&[(b"HELLO   TXT", b"after!")]),
    );
    machine.write_bytes(driver_address(MEDIA_CHANGE), &[0x00]);
    assert_eq!(read_file(&mut machine, b"D:\\HELLO.TXT"), b"after!");
}

#[test]
fn failed_write_of_the_guest_driver_fails_the_dos_call() {
    let mut machine = machine(b"x");
    machine.write_bytes(driver_address(WRITE_FAULT), &[0x01]);
    machine.write_bytes(linear(SYSINIT_SEGMENT, 0x0200), b"D:\\NEW.TXT\0");
    run(
        &mut machine,
        &[
            0xB4, 0x3C, // MOV AH,3Ch
            0x31, 0xC9, // XOR CX,CX
            0xBA, 0x00, 0x02, // MOV DX,0200h
            0xCD, 0x21, // INT 21h
            0xF4,
        ],
    );
    assert!(machine.read_word(driver_address(WRITE_COUNT)) > 0);
    assert_eq!(machine.cpu.flags() & 1, 1, "the create fails");
    assert_eq!(machine.cpu.ax(), 0x0005);
    assert_eq!(machine.indos(), 0);
    run(&mut machine, &[0xB4, 0x59, 0x31, 0xDB, 0xCD, 0x21, 0xF4]);
    assert_eq!(
        machine.cpu.ax(),
        0x0053,
        "the INT 24h handler of the kernel answers fail"
    );
}

#[test]
fn file_writes_reach_the_guest_driver() {
    let mut machine = machine(b"x");
    machine.write_bytes(linear(SYSINIT_SEGMENT, 0x0200), b"D:\\NEW.TXT\0");
    machine.write_bytes(linear(SYSINIT_SEGMENT, 0x0300), b"written");
    run(
        &mut machine,
        &[
            0xB4, 0x3C, // MOV AH,3Ch
            0x31, 0xC9, // XOR CX,CX
            0xBA, 0x00, 0x02, // MOV DX,0200h
            0xCD, 0x21, // INT 21h
            0x89, 0xC3, // MOV BX,AX
            0xB4, 0x40, // MOV AH,40h
            0xB9, 0x07, 0x00, // MOV CX,7
            0xBA, 0x00, 0x03, // MOV DX,0300h
            0xCD, 0x21, // INT 21h
            0xB4, 0x3E, // MOV AH,3Eh
            0xCD, 0x21, // INT 21h
            0xF4,
        ],
    );
    assert!(machine.read_word(driver_address(WRITE_COUNT)) > 0);
    let ramdisk = linear(RAMDISK_SEGMENT, 0) as usize;
    let image = &machine.bus.ram[ramdisk..ramdisk + RAMDISK_SECTORS * RAMDISK_SECTOR_SIZE];
    assert!(image.chunks(32).any(|entry| &entry[..11] == b"NEW     TXT"));
    assert!(image.windows(7).any(|window| window == b"written"));
}

#[test]
fn parameter_block_of_a_config_sys_driver_becomes_a_guest_unit() {
    let mut machine = machine(b"config.sys driver");
    let bpb = linear(SYSINIT_SEGMENT, BPB_BUFFER) as usize;
    let boot_sector = linear(RAMDISK_SEGMENT, 0x0B) as usize;
    machine
        .bus
        .ram
        .copy_within(boot_sector..boot_sector + 13, bpb);

    // SYSINIT raises LoL+20h and links the DPB with its drive and unit, then
    // fills it: MOV SI,BPB_BUFFER; MOV BP,NEW_PARAMETER_BLOCK; MOV AH,53h;
    // INT 21h; HLT. The driver pointer follows the call.
    let new_block = linear(SYSINIT_SEGMENT, NEW_PARAMETER_BLOCK);
    machine.write_bytes(new_block, &[DRIVE_E, 0]);
    machine.write_word(new_block + 0x12, 0);
    machine.write_word(new_block + 0x14, 0);
    machine.write_word(new_block + DPB_NEXT, 0xFFFF);
    machine.write_word(new_block + DPB_NEXT + 2, 0xFFFF);
    let (segment, offset) = parameter_block(&mut machine, DRIVE_E - 1).expect("DPB of D:");
    let last_block = linear(segment, offset);
    machine.write_word(last_block + DPB_NEXT, NEW_PARAMETER_BLOCK);
    machine.write_word(last_block + DPB_NEXT + 2, SYSINIT_SEGMENT);
    machine.write_bytes(list_of_lists() + 0x20, &[DRIVE_E + 1]);
    let [bpb_low, bpb_high] = BPB_BUFFER.to_le_bytes();
    let [dpb_low, dpb_high] = NEW_PARAMETER_BLOCK.to_le_bytes();
    run(
        &mut machine,
        &[
            0xBE, bpb_low, bpb_high, 0xBD, dpb_low, dpb_high, 0xB4, 0x53, 0xCD, 0x21, 0xF4,
        ],
    );
    machine.write_word(new_block + 0x12, RAMDISK_HEADER);
    machine.write_word(new_block + 0x14, IOSYS_SEGMENT);
    let cds_entry = linear(CDS_SEGMENT, 0) + u32::from(DRIVE_E) * CDS_ENTRY_SIZE;
    machine.write_bytes(cds_entry, b"E:\\\0");
    machine.write_word(cds_entry + 0x43, 0x4000);
    machine.write_word(cds_entry + 0x45, NEW_PARAMETER_BLOCK);
    machine.write_word(cds_entry + 0x47, SYSINIT_SEGMENT);
    machine.write_word(cds_entry + 0x4F, 2);

    assert_eq!(
        read_file(&mut machine, b"E:\\HELLO.TXT"),
        b"config.sys driver"
    );
}

#[test]
fn disk_reset_and_change_directory_leave_the_ax_of_the_dos_310_kernel() {
    let mut machine = machine(b"x");
    run(
        &mut machine,
        &[
            0xB8, 0x77, 0x0D, // MOV AX,0D77h
            0xCD, 0x21, // INT 21h
            0xF4,
        ],
    );
    assert_eq!(machine.cpu.ax(), 0x0D01);

    machine.write_bytes(linear(SYSINIT_SEGMENT, 0x0200), b"D:\\\0");
    run(
        &mut machine,
        &[
            0xB8, 0x77, 0x3B, // MOV AX,3B77h
            0xBA, 0x00, 0x02, // MOV DX,0200h
            0xCD, 0x21, // INT 21h
            0xF4,
        ],
    );
    assert_eq!(
        machine.cpu.flags() & 1,
        0,
        "the change of directory succeeds"
    );
    assert_eq!(machine.cpu.ax(), 0x0000);
}
