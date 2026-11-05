use common::{Bus, CpuMode, MachineModel, NoTrace};
use device::disk::{HddFormat, HddGeometry, HddImage};
use machine_98::Pc9801Bus;

/// Creates a small IDE test drive (20C/4H/17S, 512 bytes/sector).
/// Each sector's first two bytes contain the LBA high/low for verification.
fn make_test_drive() -> HddImage {
    let geometry = HddGeometry {
        cylinders: 20,
        heads: 4,
        sectors_per_track: 17,
        sector_size: 512,
    };
    let total = geometry.total_bytes() as usize;
    let mut data = vec![0u8; total];
    for lba in 0..geometry.total_sectors() {
        let offset = lba as usize * 512;
        data[offset] = (lba >> 8) as u8;
        data[offset + 1] = lba as u8;
    }
    HddImage::from_raw(geometry, HddFormat::Hdi, data)
}

#[test]
fn ide_rom_mapped_at_d8000_when_hdd_inserted() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);

    // Before inserting HDD, D8000 should return 0xFF (unmapped).
    assert_eq!(bus.read_byte(0xD8000), 0xFF);
    assert_eq!(bus.read_byte(0xD8001), 0xFF);

    bus.insert_hdd(0, make_test_drive(), None);

    // After inserting HDD, D8000 should return ROM bytes.
    assert_eq!(bus.read_byte(0xD8000), 0xCB);
    assert_eq!(bus.read_byte(0xD8001), 0x90);
    assert_eq!(bus.read_byte(0xD8002), 0x90);

    // ROM signature at offset 0x09: 0x55, 0xAA (expansion ROM marker).
    assert_eq!(bus.read_byte(0xD8009), 0x55);
    assert_eq!(bus.read_byte(0xD800A), 0xAA);

    // Past the ROM (DA000+), should still be unmapped.
    assert_eq!(bus.read_byte(0xDA000), 0xFF);
}

#[test]
fn ide_read_single_sector_via_pio() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Select drive 0, LBA mode, LBA bits 24-27 = 0.
    bus.io_write_byte(0x064C, 0xE0);
    // Sector count = 1.
    bus.io_write_byte(0x0644, 0x01);
    // LBA 0: sector number = 0, cylinder low = 0, cylinder high = 0.
    bus.io_write_byte(0x0646, 0x00);
    bus.io_write_byte(0x0648, 0x00);
    bus.io_write_byte(0x064A, 0x00);

    // Issue Read Sector command (0x20).
    bus.io_write_byte(0x064E, 0x20);

    // Advance past the completion event.
    bus.set_current_cycle(4096);

    // Read 256 words (512 bytes) from data register.
    let mut data = vec![0u16; 256];
    for word in data.iter_mut() {
        *word = bus.io_read_word(0x0640);
    }

    // First word should be LBA 0 marker: high=0x00, low=0x00.
    assert_eq!(data[0], 0x0000, "sector 0 first word (LBA marker)");

    // Status should indicate success (DRDY set, no ERR).
    let status = bus.io_read_byte(0x064E);
    assert_eq!(status & 0x01, 0x00, "no error expected");
    assert_ne!(status & 0x40, 0x00, "DRDY should be set");
}

#[test]
fn ide_read_sector_at_nonzero_lba() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Select drive 0, LBA mode.
    bus.io_write_byte(0x064C, 0xE0);
    bus.io_write_byte(0x0644, 0x01);
    // LBA 42 = 0x2A.
    bus.io_write_byte(0x0646, 0x2A);
    bus.io_write_byte(0x0648, 0x00);
    bus.io_write_byte(0x064A, 0x00);

    bus.io_write_byte(0x064E, 0x20);
    bus.set_current_cycle(4096);

    let mut data = vec![0u16; 256];
    for word in data.iter_mut() {
        *word = bus.io_read_word(0x0640);
    }

    // LBA 42 marker: high = 0x00, low = 0x2A -> word = 0x2A00.
    assert_eq!(data[0], 0x2A00, "sector 42 first word (LBA marker)");

    let status = bus.io_read_byte(0x064E);
    assert_eq!(status & 0x01, 0x00, "no error expected");
}

#[test]
fn ide_write_single_sector_via_pio() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Select drive 0, LBA mode, LBA 10 = 0x0A.
    bus.io_write_byte(0x064C, 0xE0);
    bus.io_write_byte(0x0644, 0x01);
    bus.io_write_byte(0x0646, 0x0A);
    bus.io_write_byte(0x0648, 0x00);
    bus.io_write_byte(0x064A, 0x00);

    // Issue Write Sector command (0x30).
    bus.io_write_byte(0x064E, 0x30);

    // Write 256 words of 0xBBBB to data register.
    for _ in 0..256 {
        bus.io_write_word(0x0640, 0xBBBB);
    }

    // Advance past the completion event.
    bus.set_current_cycle(4096);

    let status = bus.io_read_byte(0x064E);
    assert_eq!(status & 0x01, 0x00, "write should succeed");

    // Now read the same sector back to verify.
    bus.io_write_byte(0x064C, 0xE0);
    bus.io_write_byte(0x0644, 0x01);
    bus.io_write_byte(0x0646, 0x0A);
    bus.io_write_byte(0x0648, 0x00);
    bus.io_write_byte(0x064A, 0x00);
    bus.io_write_byte(0x064E, 0x20);

    bus.set_current_cycle(8192);

    let mut data = vec![0u16; 256];
    for word in data.iter_mut() {
        *word = bus.io_read_word(0x0640);
    }

    for (i, &word) in data.iter().enumerate() {
        assert_eq!(word, 0xBBBB, "read-back mismatch at word {i}");
    }
}

#[test]
fn ide_read_nonexistent_drive_returns_error() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    // No drive inserted.

    bus.io_write_byte(0x064C, 0xE0);
    bus.io_write_byte(0x0644, 0x01);
    bus.io_write_byte(0x0646, 0x00);
    bus.io_write_byte(0x0648, 0x00);
    bus.io_write_byte(0x064A, 0x00);

    bus.io_write_byte(0x064E, 0x20);
    bus.set_current_cycle(4096);

    let status = bus.io_read_byte(0x064E);
    assert_ne!(
        status & 0x01,
        0x00,
        "ERR bit should be set for missing drive"
    );
}

#[test]
fn ide_identify_device() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Select drive 0.
    bus.io_write_byte(0x064C, 0xE0);

    // Issue Identify Device command (0xEC).
    bus.io_write_byte(0x064E, 0xEC);
    bus.set_current_cycle(4096);

    // Read 256 words.
    let mut data = vec![0u16; 256];
    for word in data.iter_mut() {
        *word = bus.io_read_word(0x0640);
    }

    assert_eq!(data[0], 0x0040, "word 0: general configuration");
    assert_eq!(data[1], 20, "word 1: cylinders");
    assert_eq!(data[3], 4, "word 3: heads");
    assert_eq!(data[6], 17, "word 6: sectors per track");
    assert_ne!(data[49] & 0x0200, 0, "word 49: LBA supported");
}

#[test]
fn ide_recalibrate() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    bus.io_write_byte(0x064C, 0xE0);
    bus.io_write_byte(0x064E, 0x10); // Recalibrate

    bus.set_current_cycle(4096);

    let status = bus.io_read_byte(0x064E);
    assert_ne!(status & 0x40, 0x00, "DRDY should be set");
    assert_eq!(status & 0x01, 0x00, "no error expected");
}

#[test]
fn ide_presence_detection() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);

    // Channel 1 not selected: always returns 0x00.
    let presence = bus.io_read_byte(0x0433);
    assert_eq!(
        presence & 0x02,
        0x00,
        "channel 1 not selected should return 0x00"
    );

    // Select channel 1 (bank[1] bit 0 set), no CD-ROM: still 0x00.
    bus.io_write_byte(0x0432, 0x01);
    let presence = bus.io_read_byte(0x0433);
    assert_eq!(
        presence & 0x02,
        0x00,
        "channel 1 selected without CD-ROM should return 0x00"
    );
}

/// Creates a standard 5 MB SASI-compatible test drive (153C/4H/33S, 256 bytes/sector).
/// Each sector's first two bytes contain the LBA high/low for verification.
fn make_sasi_compat_test_drive() -> HddImage {
    let geometry = HddGeometry {
        cylinders: 153,
        heads: 4,
        sectors_per_track: 33,
        sector_size: 256,
    };
    let total = geometry.total_bytes() as usize;
    let mut data = vec![0u8; total];
    for lba in 0..geometry.total_sectors() {
        let offset = lba as usize * 256;
        data[offset] = (lba >> 8) as u8;
        data[offset + 1] = lba as u8;
    }
    HddImage::from_raw(geometry, HddFormat::Thd, data)
}

#[test]
fn ide_sasi_compat_read_single_sector_via_pio() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_sasi_compat_test_drive(), None);

    // Select drive 0, LBA mode.
    bus.io_write_byte(0x064C, 0xE0);
    bus.io_write_byte(0x0644, 0x01);
    // LBA 0.
    bus.io_write_byte(0x0646, 0x00);
    bus.io_write_byte(0x0648, 0x00);
    bus.io_write_byte(0x064A, 0x00);

    // Issue Read Sector command (0x20).
    bus.io_write_byte(0x064E, 0x20);
    bus.set_current_cycle(4096);

    // Read 128 words (256 bytes) from data register.
    let mut data = vec![0u16; 128];
    for word in data.iter_mut() {
        *word = bus.io_read_word(0x0640);
    }

    // First word should be LBA 0 marker: high=0x00, low=0x00.
    assert_eq!(data[0], 0x0000, "sector 0 first word (LBA marker)");

    // Status should indicate success (DRDY set, no ERR).
    let status = bus.io_read_byte(0x064E);
    assert_eq!(status & 0x01, 0x00, "no error expected");
    assert_ne!(status & 0x40, 0x00, "DRDY should be set");
}

#[test]
fn ide_sasi_compat_write_single_sector_via_pio() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_sasi_compat_test_drive(), None);

    // Select drive 0, LBA mode, LBA 10.
    bus.io_write_byte(0x064C, 0xE0);
    bus.io_write_byte(0x0644, 0x01);
    bus.io_write_byte(0x0646, 0x0A);
    bus.io_write_byte(0x0648, 0x00);
    bus.io_write_byte(0x064A, 0x00);

    // Issue Write Sector command (0x30).
    bus.io_write_byte(0x064E, 0x30);

    // Write 128 words of 0xBBBB to data register.
    for _ in 0..128 {
        bus.io_write_word(0x0640, 0xBBBB);
    }

    bus.set_current_cycle(4096);

    let status = bus.io_read_byte(0x064E);
    assert_eq!(status & 0x01, 0x00, "write should succeed");

    // Read the same sector back to verify.
    bus.io_write_byte(0x064C, 0xE0);
    bus.io_write_byte(0x0644, 0x01);
    bus.io_write_byte(0x0646, 0x0A);
    bus.io_write_byte(0x0648, 0x00);
    bus.io_write_byte(0x064A, 0x00);
    bus.io_write_byte(0x064E, 0x20);

    bus.set_current_cycle(8192);

    let mut data = vec![0u16; 128];
    for word in data.iter_mut() {
        *word = bus.io_read_word(0x0640);
    }

    for (i, &word) in data.iter().enumerate() {
        assert_eq!(word, 0xBBBB, "read-back mismatch at word {i}");
    }
}

#[test]
fn ide_sasi_compat_identify_reports_256_byte_sectors() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_sasi_compat_test_drive(), None);

    bus.io_write_byte(0x064C, 0xE0);
    bus.io_write_byte(0x064E, 0xEC);
    bus.set_current_cycle(4096);

    // IDENTIFY is always 256 words (512 bytes) per ATA spec.
    let mut data = vec![0u16; 256];
    for word in data.iter_mut() {
        *word = bus.io_read_word(0x0640);
    }

    assert_eq!(data[0], 0x0040, "word 0: general configuration");
    assert_eq!(data[1], 153, "word 1: cylinders");
    assert_eq!(data[3], 4, "word 3: heads");
    assert_eq!(data[4], 33 * 256, "word 4: bytes per track (33 * 256)");
    assert_eq!(data[6], 33, "word 6: sectors per track");
}

#[test]
fn ide_sasi_compat_sdip_updated_on_insertion() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);

    // Default SDIP register 0 (0x841E) = 0xF8, bit 6 = 1 (512-byte sectors).
    let default_sdip = bus.io_read_byte(0x841E);
    assert_ne!(
        default_sdip & 0x40,
        0x00,
        "default should have bit 6 set (512B)"
    );

    bus.insert_hdd(0, make_sasi_compat_test_drive(), None);

    // After inserting 256-byte sector image, bit 6 should be cleared.
    let updated_sdip = bus.io_read_byte(0x841E);
    assert_eq!(
        updated_sdip & 0x40,
        0x00,
        "bit 6 should be cleared (256-byte sector mode)"
    );
}
