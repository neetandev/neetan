//! Full-machine floppy tests: container loading through the `Machine` trait,
//! CPU-scripted DMA sector transfers, write flush-back into the source
//! container, and eject handling.

#[path = "common/harness.rs"]
mod harness;

use common::{Bus, Machine};
use device::floppy::{D88Disk, D88MediaType, D88Sector, FloppyImage};
use harness::{byte_write_script, machine, read_byte, scripted_machine, write_byte};
use machine_x68k::{X68kMachine, X68kModel};

/// Bytes in one 2HD sector.
const SECTOR_BYTES: usize = 1024;
/// Size code (128 << N) for a 1024-byte sector.
const SECTOR_SIZE_CODE: u8 = 3;
/// Exact byte size of a raw XDF image.
const XDF_IMAGE_BYTES: usize = 1_261_568;
/// RAM address of the DMA transfer buffer.
const BUFFER: u32 = 0x2000;

/// Assembles the DMAC channel-0 program moving `SECTOR_BYTES` between the
/// FDC data register and the RAM buffer. `to_memory` selects device-to-memory.
fn dmac_floppy_writes(to_memory: bool) -> Vec<(u32, u8)> {
    let mtc = SECTOR_BYTES as u16;
    vec![
        (0xE84004, 0x00),
        (0xE84005, if to_memory { 0x82 } else { 0x02 }),
        (0xE84006, 0x04),
        (0xE8400A, (mtc >> 8) as u8),
        (0xE8400B, mtc as u8),
        (0xE8400C, (BUFFER >> 24) as u8),
        (0xE8400D, (BUFFER >> 16) as u8),
        (0xE8400E, (BUFFER >> 8) as u8),
        (0xE8400F, BUFFER as u8),
        (0xE84014, 0x00),
        (0xE84015, 0xE9),
        (0xE84016, 0x40),
        (0xE84017, 0x03),
        (0xE84025, 0x40),
        (0xE84007, 0x80),
    ]
}

/// Builds the scripted machine performing one DMA sector transfer with the
/// given FDC command bytes, spinning in place afterwards.
fn transfer_machine(command: &[u8], to_memory: bool) -> X68kMachine {
    let mut writes = vec![(0xE94007u32, 0x80u8)];
    writes.extend(dmac_floppy_writes(to_memory));
    writes.extend([(0xE9C001, 0x0F), (0xE9C003, 0x40)]);
    writes.extend([(0xE94003, 0x07), (0xE94003, 0x00)]);
    writes.extend(command.iter().map(|&byte| (0xE94003, byte)));
    let mut program = byte_write_script(&writes);
    // bra.s * keeps the CPU running at interrupt mask 7.
    program.push(0x60FE);
    scripted_machine(X68kModel::X68000, &program)
}

/// Runs the machine until DMAC channel 0 reports operation complete, then a
/// little longer so the FDC finishes its result phase.
fn run_until_fdc_completion(machine: &mut X68kMachine) {
    for _ in 0..4_000 {
        machine.run_for(2_000);
        if read_byte(machine, 0xE84000) & 0x80 != 0 {
            machine.run_for(20_000);
            return;
        }
    }
    panic!("the FDC transfer never completed");
}

/// The READ DATA (MFM) command for cylinder 0, head 0, sector 1.
const READ_SECTOR_ONE: [u8; 9] = [
    0x46,
    0x00,
    0x00,
    0x00,
    0x01,
    SECTOR_SIZE_CODE,
    0x01,
    0x1B,
    0xFF,
];
/// The WRITE DATA (MFM) command for cylinder 0, head 0, sector 1.
const WRITE_SECTOR_ONE: [u8; 9] = [
    0x45,
    0x00,
    0x00,
    0x00,
    0x01,
    SECTOR_SIZE_CODE,
    0x01,
    0x1B,
    0xFF,
];

#[test]
fn xdf_images_load_by_extension_and_read_through_dma() {
    let mut image = vec![0u8; XDF_IMAGE_BYTES];
    for (index, byte) in image[..SECTOR_BYTES].iter_mut().enumerate() {
        *byte = index as u8;
    }
    let path = std::env::temp_dir().join("neetan_fdc_read.xdf");
    std::fs::write(&path, &image).unwrap();

    let mut machine = transfer_machine(&READ_SECTOR_ONE, true);
    let label = machine.insert_floppy(0, &path).unwrap();
    assert!(
        label.contains("XDF"),
        "label reports the container: {label}"
    );

    run_until_fdc_completion(&mut machine);
    for index in 0..SECTOR_BYTES {
        assert_eq!(
            machine.bus.ram_byte(BUFFER + index as u32),
            Some(index as u8),
            "sector byte {index}"
        );
    }
    assert_eq!(machine.bus.m68000_acknowledge_interrupt(1), 0x40);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn d88_writes_flush_back_into_the_source_container() {
    let sector = D88Sector {
        cylinder: 0,
        head: 0,
        record: 1,
        size_code: SECTOR_SIZE_CODE,
        sector_count: 1,
        mfm_flag: 0x40,
        deleted: 0x00,
        status: 0x00,
        reserved: [0; 5],
        data: vec![0; SECTOR_BYTES],
        source_offset: None,
    };
    let disk = D88Disk::from_tracks(
        String::from("SCRATCH"),
        false,
        D88MediaType::Disk2HD,
        vec![Some(vec![sector])],
    );
    let path = std::env::temp_dir().join("neetan_fdc_write.d88");
    std::fs::write(&path, FloppyImage::from_d88(disk).to_bytes()).unwrap();

    let mut machine = transfer_machine(&WRITE_SECTOR_ONE, false);
    machine.insert_floppy(0, &path).unwrap();
    let payload: Vec<u8> = (0..SECTOR_BYTES)
        .map(|index| (index as u8).wrapping_mul(7).wrapping_add(3))
        .collect();
    for (index, &byte) in payload.iter().enumerate() {
        write_byte(&mut machine, BUFFER + index as u32, byte);
    }

    run_until_fdc_completion(&mut machine);
    machine.flush_floppies();

    let flushed = std::fs::read(&path).unwrap();
    let reloaded = device::floppy::load_floppy_image(&path, &flushed).unwrap();
    assert_eq!(reloaded.format_name(), "D88", "the container is preserved");
    assert!(
        flushed
            .windows(SECTOR_BYTES)
            .any(|window| window == payload),
        "the written sector data must reach the source file"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn trait_insert_and_eject_latch_the_disk_change_interrupt() {
    let mut image = vec![0u8; XDF_IMAGE_BYTES];
    image[0] = 0x5A;
    let path = std::env::temp_dir().join("neetan_fdc_eject.xdf");
    std::fs::write(&path, &image).unwrap();

    let mut machine = machine(X68kModel::X68000);
    write_byte(&mut machine, 0xE9C001, 0x0F);
    write_byte(&mut machine, 0xE9C003, 0x40);

    machine.insert_floppy(0, &path).unwrap();
    assert_eq!(machine.bus.m68000_interrupt_level(), 1);
    assert_eq!(machine.bus.m68000_acknowledge_interrupt(1), 0x41);

    machine.eject_floppy(0);
    assert_eq!(machine.bus.m68000_interrupt_level(), 1);
    assert_eq!(machine.bus.m68000_acknowledge_interrupt(1), 0x41);
    assert_eq!(machine.bus.m68000_interrupt_level(), 0);
    let _ = std::fs::remove_file(&path);
}
