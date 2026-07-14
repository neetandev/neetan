use std::path::{Path, PathBuf};

use common::{Bus, CpuMode, Machine, MachineModel, NoTrace};
use machine_98::Pc9801Bus;

/// Builds a minimal D88 2HD image with the given disk name and sectors on track 0 head 0.
fn build_named_d88(disk_name: &[u8], sectors: &[(u8, &[u8])], write_protected: bool) -> Vec<u8> {
    const HEADER_SIZE: usize = 0x2B0;
    const SECTOR_HEADER_SIZE: usize = 16;

    let mut image = vec![0u8; HEADER_SIZE];
    let copy_len = disk_name.len().min(16);
    image[..copy_len].copy_from_slice(&disk_name[..copy_len]);
    if write_protected {
        image[0x1A] = 0x10;
    }
    image[0x1B] = 0x20; // 2HD

    let track_offset = HEADER_SIZE as u32;
    image[0x20..0x24].copy_from_slice(&track_offset.to_le_bytes());

    let num_sectors = sectors.len() as u16;
    for &(record, data) in sectors {
        let n: u8 = match data.len() {
            128 => 0,
            256 => 1,
            512 => 2,
            _ => 3,
        };
        let mut header = [0u8; SECTOR_HEADER_SIZE];
        header[0] = 0; // cylinder
        header[1] = 0; // head
        header[2] = record;
        header[3] = n;
        header[4..6].copy_from_slice(&num_sectors.to_le_bytes());
        let data_size = data.len() as u16;
        header[0x0E..0x10].copy_from_slice(&data_size.to_le_bytes());
        image.extend_from_slice(&header);
        image.extend_from_slice(data);
    }

    let disk_size = image.len() as u32;
    image[0x1C..0x20].copy_from_slice(&disk_size.to_le_bytes());
    image
}

/// Writes D88 bytes to a temporary file and returns the path.
/// The caller should clean up via [`cleanup_temp_file`].
fn write_temp_d88(file_name: &str, data: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(file_name);
    std::fs::write(&path, data).expect("failed to write temp D88 file");
    path
}

fn cleanup_temp_file(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn create_machine_vm() -> machine_98::Pc9801Vm {
    machine_98::Pc9801Vm::new(
        cpu::V30::new(),
        Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000),
    )
}

fn create_machine_vx() -> machine_98::Pc9801Vx {
    machine_98::Pc9801Vx::new(
        cpu::I286::new(),
        Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VX, CpuMode::High, 48000),
    )
}

fn create_machine_ra() -> machine_98::Pc9801Ra {
    machine_98::Pc9801Ra::new(
        cpu::I386::new(),
        Pc9801Bus::<NoTrace>::new(MachineModel::PC9801RA, CpuMode::High, 48000),
    )
}

#[test]
fn insert_floppy_from_file() {
    let sector_data = vec![0xAAu8; 1024];
    let d88 = build_named_d88(b"DISK_A", &[(1, &sector_data)], false);
    let path = write_temp_d88("neetan_test_insert.d88", &d88);

    let mut machine = create_machine_vm();
    let result = machine.insert_floppy(0, &path);
    cleanup_temp_file(&path);

    let desc = result.expect("insert_floppy should succeed");
    assert!(
        desc.contains("DISK_A"),
        "description should contain disk name: {desc}"
    );
    assert!(
        desc.contains("D88"),
        "description should contain format name: {desc}"
    );

    let disk = machine
        .bus
        .floppy_disk(0)
        .expect("drive 0 should have a disk");
    let sector = disk
        .find_sector_near_track_index(0, 0, 0, 1, 3)
        .expect("sector 1 should exist");
    assert!(
        sector.data.iter().all(|&b| b == 0xAA),
        "sector data should be 0xAA"
    );
}

#[test]
fn eject_floppy_clears_drive() {
    let d88 = build_named_d88(b"DISK_B", &[(1, &vec![0x55u8; 1024])], false);
    let path = write_temp_d88("neetan_test_eject.d88", &d88);

    let mut machine = create_machine_vm();
    machine
        .insert_floppy(0, &path)
        .expect("insert should succeed");
    cleanup_temp_file(&path);

    assert!(
        machine.bus.floppy_disk(0).is_some(),
        "disk should be present before eject"
    );

    machine.eject_floppy(0);

    assert!(
        machine.bus.floppy_disk(0).is_none(),
        "disk should be absent after eject"
    );
    assert!(
        !machine.bus.is_floppy_dirty(0),
        "dirty flag should be cleared after eject"
    );
}

#[test]
fn swap_floppy_replaces_disk_data() {
    let sector_a = vec![0xAAu8; 1024];
    let d88_a = build_named_d88(b"DISK_A", &[(1, &sector_a)], false);
    let path_a = write_temp_d88("neetan_test_swap_a.d88", &d88_a);

    let sector_b = vec![0xBBu8; 1024];
    let d88_b = build_named_d88(b"DISK_B", &[(1, &sector_b)], false);
    let path_b = write_temp_d88("neetan_test_swap_b.d88", &d88_b);

    let mut machine = create_machine_vm();

    // Insert disk A.
    machine
        .insert_floppy(0, &path_a)
        .expect("insert A should succeed");
    {
        let disk = machine
            .bus
            .floppy_disk(0)
            .expect("drive should have disk A");
        assert_eq!(disk.name, "DISK_A");
        let sector = disk
            .find_sector_near_track_index(0, 0, 0, 1, 3)
            .expect("sector should exist");
        assert!(sector.data.iter().all(|&b| b == 0xAA));
    }

    // Eject disk A and insert disk B.
    machine.eject_floppy(0);
    assert!(machine.bus.floppy_disk(0).is_none());

    machine
        .insert_floppy(0, &path_b)
        .expect("insert B should succeed");
    {
        let disk = machine
            .bus
            .floppy_disk(0)
            .expect("drive should have disk B");
        assert_eq!(disk.name, "DISK_B");
        let sector = disk
            .find_sector_near_track_index(0, 0, 0, 1, 3)
            .expect("sector should exist");
        assert!(sector.data.iter().all(|&b| b == 0xBB));
    }

    cleanup_temp_file(&path_a);
    cleanup_temp_file(&path_b);
}

#[test]
fn swap_floppy_on_drive_1() {
    let d88_a = build_named_d88(b"FDD2_A", &[(1, &vec![0x11u8; 1024])], false);
    let path_a = write_temp_d88("neetan_test_fdd2_a.d88", &d88_a);

    let d88_b = build_named_d88(b"FDD2_B", &[(1, &vec![0x22u8; 1024])], false);
    let path_b = write_temp_d88("neetan_test_fdd2_b.d88", &d88_b);

    let mut machine = create_machine_vm();

    machine
        .insert_floppy(1, &path_a)
        .expect("insert on drive 1 should succeed");
    assert_eq!(machine.bus.floppy_disk(1).expect("drive 1").name, "FDD2_A");

    machine.eject_floppy(1);
    assert!(machine.bus.floppy_disk(1).is_none());

    machine
        .insert_floppy(1, &path_b)
        .expect("insert B on drive 1 should succeed");
    assert_eq!(machine.bus.floppy_disk(1).expect("drive 1").name, "FDD2_B");

    cleanup_temp_file(&path_a);
    cleanup_temp_file(&path_b);
}

#[test]
fn insert_floppy_nonexistent_file_returns_error() {
    let mut machine = create_machine_vm();
    let result = machine.insert_floppy(0, Path::new("/tmp/neetan_nonexistent_disk.d88"));
    assert!(result.is_err(), "should fail for nonexistent file");
    let err = result.unwrap_err();
    assert!(
        err.contains("Failed to read"),
        "error message should mention read failure: {err}"
    );
}

#[test]
fn insert_floppy_invalid_data_returns_error() {
    let path = write_temp_d88("neetan_test_invalid.d88", b"not a valid d88 image");

    let mut machine = create_machine_vm();
    let result = machine.insert_floppy(0, &path);
    cleanup_temp_file(&path);

    assert!(result.is_err(), "should fail for invalid D88 data");
    let err = result.unwrap_err();
    assert!(
        err.contains("Failed to parse"),
        "error message should mention parse failure: {err}"
    );
}

#[test]
fn insert_floppy_write_protected_disk() {
    let d88 = build_named_d88(b"WP_DISK", &[(1, &vec![0xCCu8; 1024])], true);
    let path = write_temp_d88("neetan_test_wp.d88", &d88);

    let mut machine = create_machine_vm();
    let desc = machine
        .insert_floppy(0, &path)
        .expect("insert should succeed");
    cleanup_temp_file(&path);

    assert!(desc.contains("WP_DISK"));

    let disk = machine
        .bus
        .floppy_disk(0)
        .expect("drive 0 should have disk");
    assert!(disk.write_protected, "disk should be write-protected");
}

#[test]
fn independent_drives_do_not_interfere() {
    let d88_1 = build_named_d88(b"DRIVE0", &[(1, &vec![0x11u8; 1024])], false);
    let path_1 = write_temp_d88("neetan_test_indep_0.d88", &d88_1);

    let d88_2 = build_named_d88(b"DRIVE1", &[(1, &vec![0x22u8; 1024])], false);
    let path_2 = write_temp_d88("neetan_test_indep_1.d88", &d88_2);

    let mut machine = create_machine_vm();
    machine.insert_floppy(0, &path_1).expect("insert drive 0");
    machine.insert_floppy(1, &path_2).expect("insert drive 1");

    // Eject drive 0 - drive 1 should be unaffected.
    machine.eject_floppy(0);
    assert!(machine.bus.floppy_disk(0).is_none());
    assert_eq!(
        machine.bus.floppy_disk(1).expect("drive 1 intact").name,
        "DRIVE1"
    );

    // Re-insert drive 0 - drive 1 still unaffected.
    machine
        .insert_floppy(0, &path_1)
        .expect("re-insert drive 0");
    assert_eq!(
        machine.bus.floppy_disk(0).expect("drive 0 back").name,
        "DRIVE0"
    );
    assert_eq!(
        machine
            .bus
            .floppy_disk(1)
            .expect("drive 1 still intact")
            .name,
        "DRIVE1"
    );

    cleanup_temp_file(&path_1);
    cleanup_temp_file(&path_2);
}

#[test]
fn insert_floppy_works_on_all_cpu_types() {
    let d88 = build_named_d88(b"MULTI_CPU", &[(1, &vec![0xFFu8; 1024])], false);
    let path = write_temp_d88("neetan_test_multicpu.d88", &d88);

    // V30
    {
        let mut machine = create_machine_vm();
        let desc = machine.insert_floppy(0, &path).expect("V30 insert");
        assert!(desc.contains("MULTI_CPU"));
        machine.eject_floppy(0);
        assert!(machine.bus.floppy_disk(0).is_none());
    }

    // I286
    {
        let mut machine = create_machine_vx();
        let desc = machine.insert_floppy(0, &path).expect("I286 insert");
        assert!(desc.contains("MULTI_CPU"));
        machine.eject_floppy(0);
        assert!(machine.bus.floppy_disk(0).is_none());
    }

    // I386
    {
        let mut machine = create_machine_ra();
        let desc = machine.insert_floppy(0, &path).expect("I386 insert");
        assert!(desc.contains("MULTI_CPU"));
        machine.eject_floppy(0);
        assert!(machine.bus.floppy_disk(0).is_none());
    }

    cleanup_temp_file(&path);
}

#[test]
fn fdc_sees_disk_after_trait_insert() {
    let sector_data = vec![0x55u8; 1024];
    let d88 = build_named_d88(b"FDC_TEST", &[(1, &sector_data)], false);
    let path = write_temp_d88("neetan_test_fdc_sense.d88", &d88);

    let mut machine = create_machine_vm();
    machine.insert_floppy(0, &path).expect("insert");
    cleanup_temp_file(&path);

    // Issue Sense Drive Status via FDC I/O ports to verify the FDC knows about the disk.
    let hd_us = 0x00; // head 0, drive 0
    machine.bus.io_write_byte(0x92, 0x04); // Sense Drive Status command
    machine.bus.io_write_byte(0x92, hd_us);
    let st3 = machine.bus.io_read_byte(0x92);

    assert_eq!(
        st3 & 0x20,
        0x20,
        "ST3 Ready bit should be set after trait insert"
    );
    assert_eq!(st3 & 0x08, 0x08, "ST3 Two-Side bit should be set");
}

#[test]
fn fdc_not_ready_after_trait_eject() {
    let d88 = build_named_d88(b"EJECT_FDC", &[(1, &vec![0x55u8; 1024])], false);
    let path = write_temp_d88("neetan_test_fdc_eject.d88", &d88);

    let mut machine = create_machine_vm();
    machine.insert_floppy(0, &path).expect("insert");
    cleanup_temp_file(&path);

    machine.eject_floppy(0);

    // FDC should report not ready.
    machine.bus.io_write_byte(0x92, 0x04);
    machine.bus.io_write_byte(0x92, 0x00);
    let st3 = machine.bus.io_read_byte(0x92);

    assert_eq!(
        st3 & 0x20,
        0x00,
        "ST3 Ready bit should be clear after eject"
    );
}
