use common::{Bus, CpuMode, MachineModel, NoTrace};
use device::disk::{HddFormat, HddGeometry, HddImage};
use machine_98::Pc9801Bus;

/// Creates a small 5 MB SASI test drive (153C/4H/33S, 256 bytes/sector).
/// Each sector's first two bytes contain the LBA high/low for verification.
fn make_test_drive() -> HddImage {
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

/// Sets up DMA channel 0 to write from device to memory at `mem_addr`.
fn setup_dma_for_sasi_read(bus: &mut Pc9801Bus<NoTrace>, mem_addr: u32, byte_count: u16) {
    let addr_low = (mem_addr & 0xFF) as u8;
    let addr_high = ((mem_addr >> 8) & 0xFF) as u8;
    let page = ((mem_addr >> 16) & 0x0F) as u8;
    let count = byte_count - 1;
    let count_low = (count & 0xFF) as u8;
    let count_high = ((count >> 8) & 0xFF) as u8;

    bus.io_write_byte(0x19, 0x00); // Clear flip-flop
    bus.io_write_byte(0x01, addr_low); // Channel 0 address low
    bus.io_write_byte(0x01, addr_high); // Channel 0 address high
    bus.io_write_byte(0x19, 0x00); // Clear flip-flop
    bus.io_write_byte(0x03, count_low); // Channel 0 count low
    bus.io_write_byte(0x03, count_high); // Channel 0 count high
    bus.io_write_byte(0x27, page); // Channel 0 page register
    bus.io_write_byte(0x17, 0x44); // Mode: single, write (dev->mem), increment, ch0
    bus.io_write_byte(0x15, 0x00); // Unmask channel 0
}

/// Sets up DMA channel 0 to read from memory at `mem_addr` (for SASI write).
fn setup_dma_for_sasi_write(bus: &mut Pc9801Bus<NoTrace>, mem_addr: u32, byte_count: u16) {
    let addr_low = (mem_addr & 0xFF) as u8;
    let addr_high = ((mem_addr >> 8) & 0xFF) as u8;
    let page = ((mem_addr >> 16) & 0x0F) as u8;
    let count = byte_count - 1;
    let count_low = (count & 0xFF) as u8;
    let count_high = ((count >> 8) & 0xFF) as u8;

    bus.io_write_byte(0x19, 0x00); // Clear flip-flop
    bus.io_write_byte(0x01, addr_low); // Channel 0 address low
    bus.io_write_byte(0x01, addr_high); // Channel 0 address high
    bus.io_write_byte(0x19, 0x00); // Clear flip-flop
    bus.io_write_byte(0x03, count_low); // Channel 0 count low
    bus.io_write_byte(0x03, count_high); // Channel 0 count high
    bus.io_write_byte(0x27, page); // Channel 0 page register
    bus.io_write_byte(0x17, 0x48); // Mode: single, read (mem->dev), increment, ch0
    bus.io_write_byte(0x15, 0x00); // Unmask channel 0
}

/// Sends a 6-byte SASI command via port 0x80.
/// First selects the device (write 1 to port 0x80 in Free phase),
/// then sends the 6 command bytes.
fn send_sasi_command(bus: &mut Pc9801Bus<NoTrace>, cmd: [u8; 6]) {
    // Select device
    bus.io_write_byte(0x80, 0x01);
    // Send 6-byte command
    for byte in cmd {
        bus.io_write_byte(0x80, byte);
    }
}

/// Reads the SASI status and message bytes after a command completes.
/// Returns (status, message).
fn read_sasi_result(bus: &mut Pc9801Bus<NoTrace>) -> (u8, u8) {
    let status = bus.io_read_byte(0x80);
    let message = bus.io_read_byte(0x80);
    (status, message)
}

/// SASI HLE stack frame helper for tests.
///
/// Simulates the stack layout created by the real BIOS's INT 1Bh dispatch:
/// the BIOS pushes DS, SI, DI, ES, BP, DX, CX, BX, AX (9 words) before
/// jumping to the SASI ROM entry. The stack at SS:SP contains:
/// SP+0x00: AX, SP+0x02: BX, SP+0x04: CX, SP+0x06: DX, SP+0x08: BP,
/// SP+0x0A: ES, SP+0x0C: DI, SP+0x0E: SI, SP+0x10: DS,
/// SP+0x12: IP, SP+0x14: CS, SP+0x16: FLAGS
struct SasiTestFrame {
    ss_base: u32,
    sp: u16,
}

impl SasiTestFrame {
    const SS: u16 = 0x0000;
    const SP: u16 = 0x1000;

    fn new(
        bus: &mut Pc9801Bus<NoTrace>,
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

    fn result_ah(&self, bus: &mut Pc9801Bus<NoTrace>) -> u8 {
        let base = self.ss_base + u32::from(self.sp);
        bus.read_byte(base + 1)
    }

    fn result_cf(&self, bus: &mut Pc9801Bus<NoTrace>) -> bool {
        let base = self.ss_base + u32::from(self.sp);
        bus.read_byte(base + 0x16) & 0x01 != 0
    }

    fn read_stack_word(&self, bus: &mut Pc9801Bus<NoTrace>, offset: u32) -> u16 {
        let base = self.ss_base + u32::from(self.sp);
        let lo = bus.read_byte(base + offset) as u16;
        let hi = bus.read_byte(base + offset + 1) as u16;
        lo | (hi << 8)
    }
}

#[test]
fn sasi_rom_mapped_at_d7000_when_hdd_inserted() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    // Before inserting HDD, D7000 should return 0xFF (unmapped).
    assert_eq!(bus.read_byte(0xD7000), 0xFF);
    assert_eq!(bus.read_byte(0xD7001), 0xFF);

    bus.insert_hdd(0, make_test_drive(), None);

    // After inserting HDD, D7000 should return ROM bytes.
    // The first bytes of SASI_HLE_ROM are: 0xCB, 0x90, 0x90, ...
    assert_eq!(bus.read_byte(0xD7000), 0xCB);
    assert_eq!(bus.read_byte(0xD7001), 0x90);
    assert_eq!(bus.read_byte(0xD7002), 0x90);

    // ROM signature at offset 0x09: 0x55, 0xAA (expansion ROM marker)
    assert_eq!(bus.read_byte(0xD7009), 0x55);
    assert_eq!(bus.read_byte(0xD700A), 0xAA);

    // Past the ROM (D8000+), should still be unmapped.
    assert_eq!(bus.read_byte(0xD8000), 0xFF);
}

#[test]
fn sasi_test_unit_ready_with_drive() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Test Unit Ready (cmd 0x00) for unit 0
    send_sasi_command(&mut bus, [0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

    // Advance past the completion event.
    bus.set_current_cycle(2048);

    let (status, _message) = read_sasi_result(&mut bus);
    assert_eq!(status, 0x00, "unit 0 should be ready");
}

#[test]
fn sasi_test_unit_ready_without_drive() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    // Test Unit Ready (cmd 0x00) for unit 0 (no drive)
    send_sasi_command(&mut bus, [0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

    bus.set_current_cycle(2048);

    let (status, _message) = read_sasi_result(&mut bus);
    assert_eq!(
        status, 0x02,
        "unit 0 should report check condition (no drive)"
    );
}

#[test]
fn sasi_read_single_sector_via_dma() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Clear destination memory.
    for i in 0..256u32 {
        bus.write_byte(0x10000 + i, 0x00);
    }

    // Set up DMA channel 0 to write 256 bytes to 0x10000.
    setup_dma_for_sasi_read(&mut bus, 0x10000, 256);

    // Enable DMA on the SASI controller (DMAE | INTE).
    bus.io_write_byte(0x82, 0x03);

    // Read 1 sector from LBA 0 on unit 0.
    // CMD: 0x08, unit bits=0, LBA=0x000000, block count=1
    send_sasi_command(&mut bus, [0x08, 0x00, 0x00, 0x00, 0x01, 0x00]);

    // Advance past DMA and completion events.
    bus.set_current_cycle(4096);

    // Verify data at 0x10000: first two bytes should be LBA 0 marker (0x00, 0x00).
    assert_eq!(bus.read_byte(0x10000), 0x00, "sector 0 byte 0");
    assert_eq!(bus.read_byte(0x10001), 0x00, "sector 0 byte 1");

    // Read status/message.
    let (status, _message) = read_sasi_result(&mut bus);
    assert_eq!(status, 0x00, "read should succeed");
}

#[test]
fn sasi_read_sector_at_nonzero_lba() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    for i in 0..256u32 {
        bus.write_byte(0x10000 + i, 0x00);
    }

    setup_dma_for_sasi_read(&mut bus, 0x10000, 256);
    bus.io_write_byte(0x82, 0x03); // DMAE | INTE

    // Read 1 sector from LBA 42 (0x00002A) on unit 0.
    // cmd[1] = (unit << 5) | (lba >> 16) = 0x00
    // cmd[2] = (lba >> 8) & 0xFF = 0x00
    // cmd[3] = lba & 0xFF = 0x2A
    send_sasi_command(&mut bus, [0x08, 0x00, 0x00, 0x2A, 0x01, 0x00]);

    bus.set_current_cycle(4096);

    // LBA 42 marker: high = 0x00, low = 0x2A.
    assert_eq!(bus.read_byte(0x10000), 0x00, "sector 42 byte 0 (lba high)");
    assert_eq!(bus.read_byte(0x10001), 0x2A, "sector 42 byte 1 (lba low)");

    let (status, _) = read_sasi_result(&mut bus);
    assert_eq!(status, 0x00, "read should succeed");
}

#[test]
fn sasi_write_single_sector_via_dma() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Write a test pattern to RAM at 0x10000.
    for i in 0..256u32 {
        bus.write_byte(0x10000 + i, 0xBB);
    }

    // Set up DMA channel 0 to read 256 bytes from 0x10000.
    setup_dma_for_sasi_write(&mut bus, 0x10000, 256);

    // Enable DMA on the SASI controller (DMAE | INTE).
    bus.io_write_byte(0x82, 0x03);

    // Write 1 sector to LBA 10 on unit 0.
    send_sasi_command(&mut bus, [0x0A, 0x00, 0x00, 0x0A, 0x01, 0x00]);

    // Advance past DMA and completion events.
    bus.set_current_cycle(4096);

    let (status, _) = read_sasi_result(&mut bus);
    assert_eq!(status, 0x00, "write should succeed");

    // Now read the same sector back to verify.
    for i in 0..256u32 {
        bus.write_byte(0x20000 + i, 0x00);
    }

    setup_dma_for_sasi_read(&mut bus, 0x20000, 256);
    bus.io_write_byte(0x82, 0x03);

    send_sasi_command(&mut bus, [0x08, 0x00, 0x00, 0x0A, 0x01, 0x00]);
    bus.set_current_cycle(8192);

    // Verify data at 0x20000 should be the 0xBB pattern we wrote.
    for i in 0..256u32 {
        assert_eq!(
            bus.read_byte(0x20000 + i),
            0xBB,
            "read-back mismatch at offset {i}"
        );
    }
}

#[test]
fn sasi_read_nonexistent_drive_returns_error() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    // No drive inserted.

    setup_dma_for_sasi_read(&mut bus, 0x10000, 256);
    bus.io_write_byte(0x82, 0x03);

    // Try to read from unit 0 (no drive).
    send_sasi_command(&mut bus, [0x08, 0x00, 0x00, 0x00, 0x01, 0x00]);

    bus.set_current_cycle(4096);

    let (status, _) = read_sasi_result(&mut bus);
    // Status should indicate an error (check condition).
    assert_eq!(
        status, 0x02,
        "should report check condition for missing drive"
    );
}

#[test]
fn sasi_status_register_reports_drive_capacity() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    // With no drives, both slots report type 7 (no drive).
    // NRDSW=0 reads capacity indicators.
    bus.io_write_byte(0x82, 0x00); // Clear NRDSW
    let cap = bus.io_read_byte(0x82);
    // bits 3-5 = drive 0 type, bits 0-2 = drive 1 type
    assert_eq!(cap & 0x38, 7 << 3, "drive 0 should report type 7 (absent)");
    assert_eq!(cap & 0x07, 7, "drive 1 should report type 7 (absent)");

    // Insert a 5 MB drive on unit 0.
    bus.insert_hdd(0, make_test_drive(), None);
    bus.io_write_byte(0x82, 0x00);
    let cap = bus.io_read_byte(0x82);
    // 5 MB = type 0 (153C/4H/33S/256B).
    assert_eq!(cap & 0x38, 0 << 3, "drive 0 should report type 0 (5 MB)");
    assert_eq!(cap & 0x07, 7, "drive 1 should still report type 7 (absent)");
}

#[test]
fn sasi_hle_trap_port_triggers_on_magic() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Writing partial magic should not trigger anything special.
    for &byte in b"sasibio" {
        bus.io_write_byte(0x07EF, byte);
    }
    // The last byte completes the magic sequence and sets sasi_hle_pending.
    bus.io_write_byte(0x07EF, b's');
    assert!(bus.sasi_hle_pending());
}

#[test]
fn sasi_hle_init_sets_disk_equipment_word() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // AH=0x03 (init), AL=0x80 (drive 0).
    let frame = SasiTestFrame::new(&mut bus, 0x0380, 0, 0, 0, 0, 0);

    bus.execute_sasi_hle(frame.ss_base, frame.sp);

    // Disk equipment word at 0000:055C should indicate drive 0 present.
    let equip_lo = bus.read_byte(0x055C);
    let equip_hi = bus.read_byte(0x055D);
    let equip = u16::from(equip_lo) | (u16::from(equip_hi) << 8);
    assert_eq!(
        equip & 0x0100,
        0x0100,
        "drive 0 should be present in equipment word"
    );

    // AH should be 0 (success).
    assert_eq!(frame.result_ah(&mut bus), 0x00);

    // CF should be clear.
    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");

    assert!(!bus.sasi_hle_pending(), "pending flag should be cleared");
}

#[test]
fn sasi_hle_read_copies_sector_to_memory() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Clear destination buffer at ES:BP = 0x2000:0x0000 = 0x20000.
    for i in 0..256u32 {
        bus.write_byte(0x20000 + i, 0x00);
    }

    // AH=0x06 (read), AL=0x80 (drive 0, CHS mode), BX=256 (1 sector).
    // CHS(0, 0, 5) = LBA 5 for geometry 153C/4H/33S.
    // CX=0x0000 (cylinder 0), DX=0x0005 (DH=0 head 0, DL=5 sector 5).
    let frame = SasiTestFrame::new(&mut bus, 0x0680, 0x0100, 0x0000, 0x0005, 0x0000, 0x2000);

    bus.execute_sasi_hle(frame.ss_base, frame.sp);

    // LBA 5 marker bytes: 0x00, 0x05.
    assert_eq!(bus.read_byte(0x20000), 0x00, "sector 5 byte 0");
    assert_eq!(bus.read_byte(0x20001), 0x05, "sector 5 byte 1");

    assert_eq!(frame.result_ah(&mut bus), 0x00, "read should succeed");
    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
}

#[test]
fn sasi_hle_read_uses_vram_access_page() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    bus.io_write_byte(0xA6, 0x00);
    bus.write_byte(0xA8001, 0xAA);
    bus.io_write_byte(0xA6, 0x01);
    bus.write_byte(0xA8001, 0xAA);

    // AH=0x06 (read), AL=0x80 (drive 0, CHS mode), BX=256 (1 sector).
    // CHS(0, 0, 5) = LBA 5. ES:BP = A800:0000 targets graphics VRAM.
    let frame = SasiTestFrame::new(&mut bus, 0x0680, 0x0100, 0x0000, 0x0005, 0x0000, 0xA800);

    bus.execute_sasi_hle(frame.ss_base, frame.sp);

    assert_eq!(bus.graphics_vram()[1], 0xAA, "page 0 must be untouched");
    assert_eq!(
        bus.graphics_vram()[0x18000 + 1],
        0x05,
        "sector data should land on access page 1"
    );
    assert_eq!(frame.result_ah(&mut bus), 0x00, "read should succeed");
    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
}

#[test]
fn sasi_hle_write_modifies_drive_image() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Fill source buffer at 0x20000 with 0xCC.
    for i in 0..256u32 {
        bus.write_byte(0x20000 + i, 0xCC);
    }

    // AH=0x05 (write), AL=0x80 (drive 0), BX=256 (1 sector), CX=10 (LBA 10).
    let frame = SasiTestFrame::new(&mut bus, 0x0580, 0x0100, 0x000A, 0x0000, 0x0000, 0x2000);

    bus.execute_sasi_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x00, "write should succeed");

    // Read the sector back via HLE read to verify.
    for i in 0..256u32 {
        bus.write_byte(0x30000 + i, 0x00);
    }
    let frame2 = SasiTestFrame::new(&mut bus, 0x0680, 0x0100, 0x000A, 0x0000, 0x0000, 0x3000);
    bus.execute_sasi_hle(frame2.ss_base, frame2.sp);

    for i in 0..256u32 {
        assert_eq!(
            bus.read_byte(0x30000 + i),
            0xCC,
            "read-back mismatch at offset {i}"
        );
    }
}

#[test]
fn sasi_hle_sense_returns_media_type() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // AH=0x04 (legacy sense), AL=0x80 (drive 0).
    let frame = SasiTestFrame::new(&mut bus, 0x0480, 0, 0, 0, 0x0000, 0x2000);

    // Keep a sentinel value in ES:BP to ensure legacy sense does not write a buffer byte.
    bus.write_byte(0x20000, 0xFF);

    bus.execute_sasi_hle(frame.ss_base, frame.sp);

    // 5 MB SASI = media type 0 in AH.
    assert_eq!(
        frame.result_ah(&mut bus),
        0x00,
        "5 MB SASI should return media type 0 in AH"
    );

    // Legacy sense should not write sense output to ES:BP.
    assert_eq!(bus.read_byte(0x20000), 0xFF);

    // Success => CF clear.
    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
}

#[test]
fn sasi_hle_new_sense_84_returns_geometry_in_registers() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // AH=0x84 (new sense), AL=0x80 (drive 0).
    let frame = SasiTestFrame::new(&mut bus, 0x8480, 0xFFFF, 0xFFFF, 0xFFFF, 0, 0);

    bus.execute_sasi_hle(frame.ss_base, frame.sp);

    assert_eq!(
        frame.result_ah(&mut bus),
        0x00,
        "new sense should return 5 MB media code in AH"
    );

    // Geometry returned in BX/CX/DX on the stack.
    // Test drive geometry: 153 cylinders, 4 heads, 33 sectors, 256-byte sectors.
    // New sense returns CX = cylinders - 1.
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
        "DX should encode DH=heads, DL=sectors"
    );

    // Success => CF clear.
    assert!(!frame.result_cf(&mut bus), "CF should be clear on success");
}

#[test]
fn sasi_hle_read_no_drive_sets_error_and_carry() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);
    // No drive inserted.

    let frame = SasiTestFrame::new(&mut bus, 0x0680, 0x0100, 0x0000, 0x0000, 0x0000, 0x2000);

    bus.execute_sasi_hle(frame.ss_base, frame.sp);

    assert_ne!(
        frame.result_ah(&mut bus),
        0x00,
        "read on missing drive should return error"
    );
    assert!(frame.result_cf(&mut bus), "CF should be set on error");
}

#[test]
fn sasi_hle_yield_flag_triggers_and_clears() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Initially no yield pending.
    assert!(!bus.sasi_hle_pending());

    // Trigger the magic sequence.
    for &byte in b"sasibios" {
        bus.io_write_byte(0x07EF, byte);
    }

    // Yield should be pending (cpu_should_yield via Bus trait).
    assert!(bus.sasi_hle_pending());

    // Set up a valid stack frame and execute.
    let frame = SasiTestFrame::new(&mut bus, 0x0180, 0, 0, 0, 0, 0); // AH=0x01 (verify, no-op)
    bus.execute_sasi_hle(frame.ss_base, frame.sp);

    // Flag should be cleared after execution.
    assert!(!bus.sasi_hle_pending());
}

#[test]
fn sasi_recalibrate() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // Recalibrate (cmd 0x01) unit 0.
    send_sasi_command(&mut bus, [0x01, 0x00, 0x00, 0x00, 0x00, 0x00]);

    bus.set_current_cycle(2048);

    let (status, _) = read_sasi_result(&mut bus);
    assert_eq!(status, 0x00, "recalibrate should succeed");
}

#[test]
fn sasi_request_sense_after_error() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);
    // No drive - commands will fail.

    // Test Unit Ready on nonexistent drive.
    send_sasi_command(&mut bus, [0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    bus.set_current_cycle(2048);
    let (status, _) = read_sasi_result(&mut bus);
    assert_eq!(status, 0x02, "should report check condition");

    // Request Sense (cmd 0x03) - returns 4 bytes of sense data.
    send_sasi_command(&mut bus, [0x03, 0x00, 0x00, 0x00, 0x00, 0x00]);

    // Sense data is returned directly from the data register (no DMA needed).
    let sense0 = bus.io_read_byte(0x80);
    let sense1 = bus.io_read_byte(0x80);
    let sense2 = bus.io_read_byte(0x80);
    let sense3 = bus.io_read_byte(0x80);

    // sense0 should contain the error code (0x7F = drive not ready).
    assert_eq!(sense0, 0x7F, "sense data byte 0 should be error code 0x7F");

    // After reading all 4 sense bytes, controller transitions to completion.
    bus.set_current_cycle(4096);
    let (status, _) = read_sasi_result(&mut bus);
    assert_eq!(status, 0x00, "request sense itself should succeed");

    // Verify sense data is valid (non-zero).
    assert!(
        sense0 != 0 || sense1 != 0 || sense2 != 0 || sense3 != 0,
        "sense data should contain error information"
    );
}

#[test]
fn sasi_hle_unsupported_function_returns_error() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    // AH=0x02 is unsupported. Should return 0x40 (Equipment Check) with CF set.
    let frame = SasiTestFrame::new(&mut bus, 0x0280, 0, 0, 0, 0, 0);
    bus.execute_sasi_hle(frame.ss_base, frame.sp);

    assert_eq!(
        frame.result_ah(&mut bus),
        0x40,
        "unsupported function should return 0x40"
    );
    assert!(frame.result_cf(&mut bus), "CF should be set on error");
}

fn write_dword(bus: &mut Pc9801Bus<NoTrace>, addr: u32, value: u32) {
    bus.write_byte(addr, value as u8);
    bus.write_byte(addr + 1, (value >> 8) as u8);
    bus.write_byte(addr + 2, (value >> 16) as u8);
    bus.write_byte(addr + 3, (value >> 24) as u8);
}

fn setup_sasi_page_tables(bus: &mut Pc9801Bus<NoTrace>) {
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
fn sasi_hle_read_with_paging() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    setup_sasi_page_tables(&mut bus);
    bus.set_hle_paging(0x8000_0001, 0x80000);

    // Clear both regions.
    for i in 0..256u32 {
        bus.write_byte(0x20000 + i, 0x00);
        bus.write_byte(0x30000 + i, 0x00);
    }

    // AH=0x06 (read), AL=0x80 (drive 0), BX=256 (1 sector), CHS(0,0,5)=LBA 5.
    // ES:BP = 0x2000:0x0000 -> linear 0x20000 -> remapped to physical 0x30000.
    let frame = SasiTestFrame::new(&mut bus, 0x0680, 0x0100, 0x0000, 0x0005, 0x0000, 0x2000);

    bus.execute_sasi_hle(frame.ss_base, frame.sp);

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
fn sasi_hle_write_with_paging() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
    bus.insert_hdd(0, make_test_drive(), None);

    setup_sasi_page_tables(&mut bus);
    bus.set_hle_paging(0x8000_0001, 0x80000);

    // Write 0xCC pattern to physical 0x30000 (the remapped destination).
    for i in 0..256u32 {
        bus.write_byte(0x30000 + i, 0xCC);
    }

    // AH=0x05 (write), AL=0x80 (drive 0), BX=256 (1 sector), CX=10 (LBA 10).
    // ES:BP = 0x2000:0x0000 -> linear 0x20000 -> remapped to physical 0x30000.
    let frame = SasiTestFrame::new(&mut bus, 0x0580, 0x0100, 0x000A, 0x0000, 0x0000, 0x2000);

    bus.execute_sasi_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x00, "write should succeed");

    // Read the sector back (without paging, to verify the written data).
    bus.set_hle_paging(0, 0);
    for i in 0..256u32 {
        bus.write_byte(0x40000 + i, 0x00);
    }
    let frame2 = SasiTestFrame::new(&mut bus, 0x0680, 0x0100, 0x000A, 0x0000, 0x0000, 0x4000);
    bus.execute_sasi_hle(frame2.ss_base, frame2.sp);

    for i in 0..256u32 {
        assert_eq!(
            bus.read_byte(0x40000 + i),
            0xCC,
            "read-back mismatch at offset {i}"
        );
    }
}
