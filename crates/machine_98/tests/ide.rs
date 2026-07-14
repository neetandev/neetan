use common::{Bus, CpuMode, MachineModel};
use device::disk::{HddFormat, HddGeometry, HddImage};
use machine_98::{NoTracing, Pc9801Bus};

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

/// IDE HLE stack frame helper for tests.
///
/// Simulates the stack layout created by the real BIOS's INT 1Bh dispatch:
/// the BIOS pushes DS, SI, DI, ES, BP, DX, CX, BX, AX (9 words) before
/// jumping to the IDE ROM entry. The stack at SS:SP contains:
/// SP+0x00: AX, SP+0x02: BX, SP+0x04: CX, SP+0x06: DX, SP+0x08: BP,
/// SP+0x0A: ES, SP+0x0C: DI, SP+0x0E: SI, SP+0x10: DS,
/// SP+0x12: IP, SP+0x14: CS, SP+0x16: FLAGS
struct IdeTestFrame {
    ss_base: u32,
    sp: u16,
}

impl IdeTestFrame {
    const SS: u16 = 0x0000;
    const SP: u16 = 0x1000;

    fn new(
        bus: &mut Pc9801Bus<NoTracing>,
        ax: u16,
        bx: u16,
        cx: u16,
        dx: u16,
        bp: u16,
        es: u16,
    ) -> Self {
        let ss_base = u32::from(Self::SS) << 4;
        let base = ss_base + u32::from(Self::SP);
        let words: [u16; 12] = [
            ax, bx, cx, dx, bp, es, // registers
            0, 0, 0, // DI, SI, DS
            0, 0, 0x0200, // IP, CS, FLAGS
        ];
        for (i, &w) in words.iter().enumerate() {
            let addr = base + (i as u32) * 2;
            bus.write_byte(addr, w as u8);
            bus.write_byte(addr + 1, (w >> 8) as u8);
        }
        Self {
            ss_base,
            sp: Self::SP,
        }
    }

    fn result_ah(&self, bus: &mut Pc9801Bus<NoTracing>) -> u8 {
        let base = self.ss_base + u32::from(self.sp);
        bus.read_byte(base + 1)
    }

    fn result_cf(&self, bus: &mut Pc9801Bus<NoTracing>) -> bool {
        let base = self.ss_base + u32::from(self.sp);
        bus.read_byte(base + 0x16) & 0x01 != 0
    }

    fn read_stack_word(&self, bus: &mut Pc9801Bus<NoTracing>, offset: u32) -> u16 {
        let base = self.ss_base + u32::from(self.sp);
        let lo = bus.read_byte(base + offset) as u16;
        let hi = bus.read_byte(base + offset + 1) as u16;
        lo | (hi << 8)
    }
}

#[test]
fn ide_rom_mapped_at_d8000_when_hdd_inserted() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);

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
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
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
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
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
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
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
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
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
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
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
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
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
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);

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

#[test]
fn ide_hle_trap_port_triggers() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    assert!(!bus.ide_hle_pending());

    // IDE uses a single-byte trap at port 0x07EE.
    bus.io_write_byte(0x07EE, 0x00);
    assert!(bus.ide_hle_pending());
}

#[test]
fn ide_hle_init_sets_disk_equipment_word() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // AH=0x03 (init), AL=0x80 (drive 0).
    let frame = IdeTestFrame::new(&mut bus, 0x0380, 0, 0, 0, 0, 0);

    bus.execute_ide_hle(frame.ss_base, frame.sp);

    // Disk equipment word at 0000:055C should indicate drive 0 present.
    let equip_lo = bus.read_byte(0x055C);
    let equip_hi = bus.read_byte(0x055D);
    let equip = u16::from(equip_lo) | (u16::from(equip_hi) << 8);
    assert_eq!(
        equip & 0x0100,
        0x0100,
        "drive 0 should be present in equipment word"
    );

    assert_eq!(frame.result_ah(&mut bus), 0x00);
    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
    assert!(!bus.ide_hle_pending(), "pending flag should be cleared");
}

#[test]
fn ide_hle_read_copies_sector_to_memory() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Clear destination buffer at ES:BP = 0x2000:0x0000 = 0x20000.
    for i in 0..512u32 {
        bus.write_byte(0x20000 + i, 0x00);
    }

    // AH=0x06 (read), AL=0x80 (drive 0, CHS mode), BX=512 (1 sector).
    // CHS(0, 0, 5) = LBA 5 for geometry 20C/4H/17S.
    // CX=0x0000 (cylinder 0), DX=0x0005 (DH=0 head 0, DL=5 sector 5).
    let frame = IdeTestFrame::new(&mut bus, 0x0680, 0x0200, 0x0000, 0x0005, 0x0000, 0x2000);

    bus.execute_ide_hle(frame.ss_base, frame.sp);

    // LBA 5 marker bytes: 0x00, 0x05.
    assert_eq!(bus.read_byte(0x20000), 0x00, "sector 5 byte 0");
    assert_eq!(bus.read_byte(0x20001), 0x05, "sector 5 byte 1");

    assert_eq!(frame.result_ah(&mut bus), 0x00, "read should succeed");
    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
}

#[test]
fn ide_hle_write_modifies_drive_image() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Fill source buffer at 0x20000 with 0xCC.
    for i in 0..512u32 {
        bus.write_byte(0x20000 + i, 0xCC);
    }

    // AH=0x05 (write), AL=0x80 (drive 0), BX=512 (1 sector), CX=10 (LBA 10).
    let frame = IdeTestFrame::new(&mut bus, 0x0580, 0x0200, 0x000A, 0x0000, 0x0000, 0x2000);

    bus.execute_ide_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x00, "write should succeed");

    // Read the sector back via HLE read to verify.
    for i in 0..512u32 {
        bus.write_byte(0x30000 + i, 0x00);
    }
    let frame2 = IdeTestFrame::new(&mut bus, 0x0680, 0x0200, 0x000A, 0x0000, 0x0000, 0x3000);
    bus.execute_ide_hle(frame2.ss_base, frame2.sp);

    for i in 0..512u32 {
        assert_eq!(
            bus.read_byte(0x30000 + i),
            0xCC,
            "read-back mismatch at offset {i}"
        );
    }
}

#[test]
fn ide_hle_sense_returns_media_type() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // AH=0x04 (legacy sense), AL=0x80 (drive 0).
    let frame = IdeTestFrame::new(&mut bus, 0x0480, 0, 0, 0, 0x0000, 0x2000);

    // Keep a sentinel value in ES:BP.
    bus.write_byte(0x20000, 0xFF);

    bus.execute_ide_hle(frame.ss_base, frame.sp);

    // IDE media type is 0x0F.
    assert_eq!(
        frame.result_ah(&mut bus),
        0x0F,
        "IDE drive should return media type 0x0F in AH"
    );

    // Legacy sense should not write sense output to ES:BP.
    assert_eq!(bus.read_byte(0x20000), 0xFF);

    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
}

#[test]
fn ide_hle_new_sense_84_returns_geometry_in_registers() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // AH=0x84 (new sense), AL=0x80 (drive 0).
    let frame = IdeTestFrame::new(&mut bus, 0x8480, 0xFFFF, 0xFFFF, 0xFFFF, 0, 0);

    bus.execute_ide_hle(frame.ss_base, frame.sp);

    assert_eq!(
        frame.result_ah(&mut bus),
        0x0F,
        "new sense should return IDE media code 0x0F in AH"
    );

    // Geometry returned in BX/CX/DX on the stack.
    // Test drive geometry: 20 cylinders, 4 heads, 17 sectors, 512-byte sectors.
    // New sense returns CX = cylinders - 1.
    assert_eq!(
        frame.read_stack_word(&mut bus, 0x02),
        0x0200,
        "BX should be sector size (512)"
    );
    assert_eq!(
        frame.read_stack_word(&mut bus, 0x04),
        19,
        "CX should be cylinders - 1"
    );
    assert_eq!(
        frame.read_stack_word(&mut bus, 0x06),
        0x0411,
        "DX should encode DH=heads(4), DL=sectors(17)"
    );

    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
}

#[test]
fn ide_hle_read_no_drive_sets_error_and_carry() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    // No drive inserted.

    let frame = IdeTestFrame::new(&mut bus, 0x0680, 0x0200, 0x0000, 0x0000, 0x0000, 0x2000);

    bus.execute_ide_hle(frame.ss_base, frame.sp);

    assert_ne!(
        frame.result_ah(&mut bus),
        0x00,
        "read on missing drive should return error"
    );
    assert!(frame.result_cf(&mut bus), "CF should be set on error");
}

#[test]
fn ide_hle_yield_flag_triggers_and_clears() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Initially no yield pending.
    assert!(!bus.ide_hle_pending());

    // Trigger the trap.
    bus.io_write_byte(0x07EE, 0x00);

    // Yield should be pending.
    assert!(bus.ide_hle_pending());

    // Set up a valid stack frame and execute.
    let frame = IdeTestFrame::new(&mut bus, 0x0180, 0, 0, 0, 0, 0); // AH=0x01 (verify, no-op)
    bus.execute_ide_hle(frame.ss_base, frame.sp);

    // Flag should be cleared after execution.
    assert!(!bus.ide_hle_pending());
}

#[test]
fn ide_hle_unsupported_function_returns_error() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // AH=0x02 is unsupported. Should return 0x40 (Equipment Check) with CF set.
    let frame = IdeTestFrame::new(&mut bus, 0x0280, 0, 0, 0, 0, 0);
    bus.execute_ide_hle(frame.ss_base, frame.sp);

    assert_eq!(
        frame.result_ah(&mut bus),
        0x40,
        "unsupported function should return 0x40"
    );
    assert!(frame.result_cf(&mut bus), "CF should be set on error");
}

#[test]
fn ide_hle_check_power_mode() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // AH=0xD0 (check power mode), AL=0x80 (drive 0).
    let frame = IdeTestFrame::new(&mut bus, 0xD080, 0, 0, 0, 0, 0);
    bus.execute_ide_hle(frame.ss_base, frame.sp);

    assert_eq!(
        frame.result_ah(&mut bus),
        0x00,
        "check power mode should return 0x00"
    );
    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
}

#[test]
fn ide_hle_motor_on() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // AH=0xE0 (motor on), AL=0x80 (drive 0).
    let frame = IdeTestFrame::new(&mut bus, 0xE080, 0, 0, 0, 0, 0);
    bus.execute_ide_hle(frame.ss_base, frame.sp);

    assert_eq!(
        frame.result_ah(&mut bus),
        0x00,
        "motor on should return 0x00"
    );
    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
}

#[test]
fn ide_hle_motor_off() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // AH=0xF0 (motor off), AL=0x80 (drive 0).
    let frame = IdeTestFrame::new(&mut bus, 0xF080, 0, 0, 0, 0, 0);
    bus.execute_ide_hle(frame.ss_base, frame.sp);

    assert_eq!(
        frame.result_ah(&mut bus),
        0x00,
        "motor off should return 0x00"
    );
    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
}

fn write_dword(bus: &mut Pc9801Bus<NoTracing>, addr: u32, value: u32) {
    bus.write_byte(addr, value as u8);
    bus.write_byte(addr + 1, (value >> 8) as u8);
    bus.write_byte(addr + 2, (value >> 16) as u8);
    bus.write_byte(addr + 3, (value >> 24) as u8);
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

fn setup_ide_page_tables(bus: &mut Pc9801Bus<NoTracing>) {
    let page_dir: u32 = 0x80000;
    let page_table: u32 = 0x81000;

    const PTE_P: u32 = 0x01;
    const PTE_RW: u32 = 0x02;

    // PDE 0 points to page table.
    write_dword(bus, page_dir, page_table | PTE_P | PTE_RW);

    // Identity-map all 256 pages (covers 0x00000-0xFFFFF).
    for i in 0..256u32 {
        let phys = i * 0x1000;
        write_dword(bus, page_table + i * 4, phys | PTE_P | PTE_RW);
    }

    // Remap linear page 0x20 (0x20000-0x20FFF) -> physical page 0x30 (0x30000-0x30FFF).
    write_dword(bus, page_table + 0x20 * 4, 0x30000 | PTE_P | PTE_RW);
}

#[test]
fn ide_hle_read_with_paging() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    setup_ide_page_tables(&mut bus);
    bus.set_hle_paging(0x8000_0001, 0x80000);

    // Clear both regions.
    for i in 0..512u32 {
        bus.write_byte(0x20000 + i, 0x00);
        bus.write_byte(0x30000 + i, 0x00);
    }

    // AH=0x06 (read), AL=0x80 (drive 0), BX=512 (1 sector), CHS(0,0,5)=LBA 5.
    // ES:BP = 0x2000:0x0000 -> linear 0x20000 -> remapped to physical 0x30000.
    let frame = IdeTestFrame::new(&mut bus, 0x0680, 0x0200, 0x0000, 0x0005, 0x0000, 0x2000);

    bus.execute_ide_hle(frame.ss_base, frame.sp);

    // Data should appear at physical 0x30000, NOT at 0x20000.
    assert_eq!(
        bus.read_byte(0x30000),
        0x00,
        "sector 5 LBA high byte at remapped physical address"
    );
    assert_eq!(
        bus.read_byte(0x30001),
        0x05,
        "sector 5 LBA low byte at remapped physical address"
    );
    assert_eq!(
        bus.read_byte(0x20000),
        0x00,
        "original linear address should be untouched"
    );
    assert_eq!(
        bus.read_byte(0x20001),
        0x00,
        "original linear address should be untouched"
    );

    assert_eq!(frame.result_ah(&mut bus), 0x00, "read should succeed");
    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
}

#[test]
fn ide_hle_write_with_paging() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    setup_ide_page_tables(&mut bus);
    bus.set_hle_paging(0x8000_0001, 0x80000);

    // Write 0xCC pattern to physical 0x30000 (the remapped destination).
    for i in 0..512u32 {
        bus.write_byte(0x30000 + i, 0xCC);
    }

    // AH=0x05 (write), AL=0x80 (drive 0), BX=512 (1 sector), CX=10 (LBA 10).
    // ES:BP = 0x2000:0x0000 -> linear 0x20000 -> remapped to physical 0x30000.
    let frame = IdeTestFrame::new(&mut bus, 0x0580, 0x0200, 0x000A, 0x0000, 0x0000, 0x2000);

    bus.execute_ide_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x00, "write should succeed");

    // Read the sector back (without paging, to verify the written data).
    bus.set_hle_paging(0, 0);
    for i in 0..512u32 {
        bus.write_byte(0x40000 + i, 0x00);
    }
    let frame2 = IdeTestFrame::new(&mut bus, 0x0680, 0x0200, 0x000A, 0x0000, 0x0000, 0x4000);
    bus.execute_ide_hle(frame2.ss_base, frame2.sp);

    for i in 0..512u32 {
        assert_eq!(
            bus.read_byte(0x40000 + i),
            0xCC,
            "read-back mismatch at offset {i}"
        );
    }
}

#[test]
fn ide_sasi_compat_read_single_sector_via_pio() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
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
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
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
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
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
fn ide_sasi_compat_hle_sense_returns_sasi_media_type() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_sasi_compat_test_drive(), None);

    // AH=0x04 (legacy sense), AL=0x80 (drive 0).
    let frame = IdeTestFrame::new(&mut bus, 0x0480, 0, 0, 0, 0x0000, 0x2000);
    bus.execute_ide_hle(frame.ss_base, frame.sp);

    // 5 MB SASI type returns 0x00 (new sense code for type 0).
    assert_eq!(
        frame.result_ah(&mut bus),
        0x00,
        "SASI-compat drive should return SASI media type in AH"
    );
    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
}

#[test]
fn ide_sasi_compat_hle_new_sense_returns_geometry() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_sasi_compat_test_drive(), None);

    // AH=0x84 (new sense), AL=0x80 (drive 0).
    let frame = IdeTestFrame::new(&mut bus, 0x8480, 0xFFFF, 0xFFFF, 0xFFFF, 0, 0);
    bus.execute_ide_hle(frame.ss_base, frame.sp);

    assert_eq!(
        frame.result_ah(&mut bus),
        0x00,
        "new sense should return SASI media code 0x00 in AH"
    );

    // BX = sector size (256), CX = cylinders - 1, DX = DH:heads | DL:sectors.
    assert_eq!(
        frame.read_stack_word(&mut bus, 0x02),
        0x0100,
        "BX should be sector size (256)"
    );
    assert_eq!(
        frame.read_stack_word(&mut bus, 0x04),
        152,
        "CX should be cylinders - 1"
    );
    assert_eq!(
        frame.read_stack_word(&mut bus, 0x06),
        0x0421,
        "DX should encode DH=heads(4), DL=sectors(33)"
    );

    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
}

#[test]
fn ide_sasi_compat_hle_read_copies_256_byte_sector() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_sasi_compat_test_drive(), None);

    // Clear destination buffer at ES:BP = 0x2000:0x0000 = 0x20000.
    for i in 0..256u32 {
        bus.write_byte(0x20000 + i, 0x00);
    }

    // AH=0x06 (read), AL=0x80 (drive 0, CHS mode), BX=256 (1 sector).
    // CHS(0, 0, 5) = LBA 5 for geometry 153C/4H/33S.
    let frame = IdeTestFrame::new(&mut bus, 0x0680, 0x0100, 0x0000, 0x0005, 0x0000, 0x2000);
    bus.execute_ide_hle(frame.ss_base, frame.sp);

    // LBA 5 marker bytes: 0x00, 0x05.
    assert_eq!(bus.read_byte(0x20000), 0x00, "sector 5 byte 0");
    assert_eq!(bus.read_byte(0x20001), 0x05, "sector 5 byte 1");

    assert_eq!(frame.result_ah(&mut bus), 0x00, "read should succeed");
    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
}

#[test]
fn ide_sasi_compat_hle_write_modifies_drive_image() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_sasi_compat_test_drive(), None);

    // Fill source buffer at 0x20000 with 0xCC.
    for i in 0..256u32 {
        bus.write_byte(0x20000 + i, 0xCC);
    }

    // AH=0x05 (write), AL=0x80 (drive 0), BX=256 (1 sector), CX=10 (LBA 10).
    let frame = IdeTestFrame::new(&mut bus, 0x0580, 0x0100, 0x000A, 0x0000, 0x0000, 0x2000);
    bus.execute_ide_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x00, "write should succeed");

    // Read the sector back via HLE read to verify.
    for i in 0..256u32 {
        bus.write_byte(0x30000 + i, 0x00);
    }
    let frame2 = IdeTestFrame::new(&mut bus, 0x0680, 0x0100, 0x000A, 0x0000, 0x0000, 0x3000);
    bus.execute_ide_hle(frame2.ss_base, frame2.sp);

    for i in 0..256u32 {
        assert_eq!(
            bus.read_byte(0x30000 + i),
            0xCC,
            "read-back mismatch at offset {i}"
        );
    }
}

#[test]
fn ide_sasi_compat_sdip_updated_on_insertion() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);

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
