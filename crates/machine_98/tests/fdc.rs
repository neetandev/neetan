use common::{Bus, CpuMode, MachineModel};
use device::floppy::FloppyImage;
use machine_98::{NoTracing, Pc9801Bus};

/// Builds a minimal D88 2HD image with sectors on track 0 (head 0).
fn build_test_d88(sectors: &[(u8, u8, u8, u8, &[u8])], write_protected: bool) -> Vec<u8> {
    const HEADER_SIZE: usize = 0x2B0;
    const SECTOR_HEADER_SIZE: usize = 16;

    let mut image = vec![0u8; HEADER_SIZE];
    image[..4].copy_from_slice(b"TEST");
    if write_protected {
        image[0x1A] = 0x10;
    }
    image[0x1B] = 0x20; // 2HD

    let track_offset = HEADER_SIZE as u32;
    image[0x20..0x24].copy_from_slice(&track_offset.to_le_bytes());

    let mut track_data = Vec::new();
    for &(c, h, r, n, data) in sectors {
        let mut header = [0u8; SECTOR_HEADER_SIZE];
        header[0] = c;
        header[1] = h;
        header[2] = r;
        header[3] = n;
        let sc = sectors.len() as u16;
        header[4..6].copy_from_slice(&sc.to_le_bytes());
        let data_size = data.len() as u16;
        header[0x0E..0x10].copy_from_slice(&data_size.to_le_bytes());
        track_data.extend_from_slice(&header);
        track_data.extend_from_slice(data);
    }

    image.extend_from_slice(&track_data);
    let disk_size = image.len() as u32;
    image[0x1C..0x20].copy_from_slice(&disk_size.to_le_bytes());
    image
}

/// Sets up DMA channel 2 to read from memory at `mem_addr` (memory -> FDC).
fn setup_dma_for_write(bus: &mut Pc9801Bus<NoTracing>, mem_addr: u32, byte_count: u16) {
    let addr_low = (mem_addr & 0xFF) as u8;
    let addr_high = ((mem_addr >> 8) & 0xFF) as u8;
    let page = ((mem_addr >> 16) & 0x0F) as u8;
    let count = byte_count - 1; // DMA count register = N-1
    let count_low = (count & 0xFF) as u8;
    let count_high = ((count >> 8) & 0xFF) as u8;

    bus.io_write_byte(0x19, 0x00); // Clear flip-flop
    bus.io_write_byte(0x09, addr_low); // Channel 2 address low
    bus.io_write_byte(0x09, addr_high); // Channel 2 address high
    bus.io_write_byte(0x19, 0x00); // Clear flip-flop
    bus.io_write_byte(0x0B, count_low); // Channel 2 count low
    bus.io_write_byte(0x0B, count_high); // Channel 2 count high
    bus.io_write_byte(0x23, page); // Channel 2 page register
    bus.io_write_byte(0x17, 0x4A); // Mode: single, read (mem->dev), increment, ch2
    bus.io_write_byte(0x15, 0x02); // Unmask channel 2
}

/// Issues a WRITE DATA command to the 1MB FDC via port 0x92.
fn issue_write_data(bus: &mut Pc9801Bus<NoTracing>, c: u8, h: u8, r: u8, n: u8, eot: u8) {
    let hd_us = h << 2; // Head in bit 2, drive 0
    bus.io_write_byte(0x92, 0x45); // WRITE DATA: MF=1, cmd=0x05
    bus.io_write_byte(0x92, hd_us);
    bus.io_write_byte(0x92, c);
    bus.io_write_byte(0x92, h);
    bus.io_write_byte(0x92, r);
    bus.io_write_byte(0x92, n);
    bus.io_write_byte(0x92, eot);
    bus.io_write_byte(0x92, 0x1B); // GPL
    bus.io_write_byte(0x92, 0xFF); // DTL
}

/// Reads 7 FDC result bytes from port 0x92.
fn read_fdc_results(bus: &mut Pc9801Bus<NoTracing>) -> [u8; 7] {
    let mut result = [0u8; 7];
    for byte in &mut result {
        *byte = bus.io_read_byte(0x92);
    }
    result
}

/// Issues a SENSE DRIVE STATUS command to the 1MB FDC and returns ST3.
fn issue_sense_drive_status(bus: &mut Pc9801Bus<NoTracing>, drive: u8, head: u8) -> u8 {
    let hd_us = (head << 2) | drive;
    bus.io_write_byte(0x92, 0x04); // Sense Drive Status command
    bus.io_write_byte(0x92, hd_us);
    bus.io_read_byte(0x92) // Read ST3 result
}

#[test]
fn fdc_sense_drive_status_two_side_with_disk() {
    let sector_data = vec![0x55u8; 1024];
    let d88_bytes = build_test_d88(&[(0, 0, 1, 3, &sector_data)], false);
    let disk = FloppyImage::from_d88_bytes(&d88_bytes).expect("floppy image parse failed");

    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    bus.insert_floppy(0, disk, None);

    let st3 = issue_sense_drive_status(&mut bus, 0, 0);

    assert_eq!(st3 & 0x20, 0x20, "ST3 Ready bit should be set");
    assert_eq!(st3 & 0x10, 0x10, "ST3 Track 0 bit should be set");
    assert_eq!(
        st3 & 0x08,
        0x08,
        "ST3 Two-Side bit should be set (required for BIOS 2HD sense)"
    );
}

#[test]
fn fdc_sense_drive_status_two_side_no_disk() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    let st3 = issue_sense_drive_status(&mut bus, 0, 0);

    assert_eq!(
        st3 & 0x08,
        0x08,
        "ST3 Two-Side bit should be set even without disk"
    );
    assert_eq!(
        st3 & 0x20,
        0x00,
        "ST3 Ready bit should NOT be set without disk"
    );
}

#[test]
fn fdc_sense_drive_status_after_eject_reinsert() {
    let sector_data = vec![0x55u8; 1024];
    let d88_bytes = build_test_d88(&[(0, 0, 1, 3, &sector_data)], false);
    let disk = FloppyImage::from_d88_bytes(&d88_bytes).expect("floppy image parse failed");

    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    bus.insert_floppy(0, disk, None);

    let st3 = issue_sense_drive_status(&mut bus, 0, 0);
    assert_eq!(st3 & 0x20, 0x20, "should be ready with disk");

    bus.eject_floppy(0);
    let st3 = issue_sense_drive_status(&mut bus, 0, 0);
    assert_eq!(st3 & 0x20, 0x00, "should not be ready after eject");
    assert_eq!(
        st3 & 0x08,
        0x08,
        "two-side should still be set (drive exists)"
    );

    let d88_bytes2 = build_test_d88(&[(0, 0, 1, 3, &vec![0xAAu8; 1024])], false);
    let disk2 = FloppyImage::from_d88_bytes(&d88_bytes2).expect("floppy image parse failed");
    bus.insert_floppy(0, disk2, None);

    let st3 = issue_sense_drive_status(&mut bus, 0, 0);
    assert_eq!(st3 & 0x20, 0x20, "should be ready after reinserting disk");
    assert_eq!(st3 & 0x08, 0x08, "two-side should be set");
}

#[test]
fn fdc_write_data_single_sector() {
    let sector_data = vec![0x55u8; 1024]; // N=3 -> 1024 bytes
    let d88_bytes = build_test_d88(&[(0, 0, 1, 3, &sector_data)], false);
    let disk = FloppyImage::from_d88_bytes(&d88_bytes).expect("floppy image parse failed");

    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    bus.insert_floppy(0, disk, None);

    // Write 0xAA pattern to RAM at physical address 0x10000.
    for i in 0..1024u32 {
        bus.write_byte(0x10000 + i, 0xAA);
    }

    setup_dma_for_write(&mut bus, 0x10000, 1024);
    issue_write_data(&mut bus, 0, 0, 1, 3, 1);

    // Advance time past the FdcExecution event (scheduled at cycle 512).
    bus.set_current_cycle(1024);

    let result = read_fdc_results(&mut bus);

    // ST0: normal termination (IC=00).
    assert_eq!(
        result[0] & 0xC0,
        0x00,
        "ST0 should indicate normal termination"
    );
    assert_eq!(result[1], 0x00, "ST1 should be clean");
    assert_eq!(result[2], 0x00, "ST2 should be clean");

    // Verify sector data was overwritten from 0x55 to 0xAA.
    let disk = bus.floppy_disk(0).expect("disk should still be inserted");
    let sector = disk
        .find_sector_near_track_index(0, 0, 0, 1, 3)
        .expect("sector should exist");
    assert!(
        sector.data.iter().all(|&b| b == 0xAA),
        "sector data should be all 0xAA"
    );

    assert!(
        bus.is_floppy_dirty(0),
        "drive 0 should be dirty after write"
    );
}

#[test]
fn fdc_write_data_write_protected() {
    let sector_data = vec![0x55u8; 1024];
    let d88_bytes = build_test_d88(&[(0, 0, 1, 3, &sector_data)], true);
    let disk = FloppyImage::from_d88_bytes(&d88_bytes).expect("floppy image parse failed");

    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    bus.insert_floppy(0, disk, None);

    for i in 0..1024u32 {
        bus.write_byte(0x10000 + i, 0xAA);
    }

    setup_dma_for_write(&mut bus, 0x10000, 1024);
    issue_write_data(&mut bus, 0, 0, 1, 3, 1);
    bus.set_current_cycle(1024);

    let result = read_fdc_results(&mut bus);

    // ST0: abnormal termination (IC=01).
    assert_eq!(
        result[0] & 0xC0,
        0x40,
        "ST0 should indicate abnormal termination"
    );
    // ST1: NW (not writable) bit set.
    assert_eq!(result[1] & 0x02, 0x02, "ST1 should have NW bit set");

    // Sector data should be unchanged.
    let disk = bus.floppy_disk(0).expect("disk should still be inserted");
    let sector = disk
        .find_sector_near_track_index(0, 0, 0, 1, 3)
        .expect("sector should exist");
    assert!(
        sector.data.iter().all(|&b| b == 0x55),
        "sector data should still be all 0x55"
    );
}

#[test]
fn fdc_write_data_no_disk() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    for i in 0..1024u32 {
        bus.write_byte(0x10000 + i, 0xAA);
    }

    setup_dma_for_write(&mut bus, 0x10000, 1024);
    issue_write_data(&mut bus, 0, 0, 1, 3, 1);
    bus.set_current_cycle(1024);

    let result = read_fdc_results(&mut bus);

    // ST0: abnormal termination with NOT READY.
    assert_eq!(
        result[0] & 0xC0,
        0x40,
        "ST0 should indicate abnormal termination"
    );
    assert_eq!(
        result[0] & 0x08,
        0x08,
        "ST0 should have NR (not ready) bit set"
    );
}

#[test]
fn fdc_write_data_updates_dirty_flag() {
    let sector_data = vec![0x55u8; 1024];
    let d88_bytes = build_test_d88(&[(0, 0, 1, 3, &sector_data)], false);
    let disk = FloppyImage::from_d88_bytes(&d88_bytes).expect("floppy image parse failed");

    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    bus.insert_floppy(0, disk, None);

    assert!(
        !bus.is_floppy_dirty(0),
        "drive 0 should not be dirty before write"
    );

    for i in 0..1024u32 {
        bus.write_byte(0x10000 + i, 0xAA);
    }

    setup_dma_for_write(&mut bus, 0x10000, 1024);
    issue_write_data(&mut bus, 0, 0, 1, 3, 1);
    bus.set_current_cycle(1024);

    let _result = read_fdc_results(&mut bus);

    assert!(
        bus.is_floppy_dirty(0),
        "drive 0 should be dirty after write"
    );

    // Eject clears the dirty flag (no path, so no file I/O).
    bus.eject_floppy(0);
    assert!(
        !bus.is_floppy_dirty(0),
        "drive 0 should not be dirty after eject"
    );
}
