use common::{Bus, CpuMode, MachineModel, NoTrace, TraceSink};
use device::floppy::FloppyImage;
use machine_98::Pc9801Bus;

const SECTOR_512: usize = 512;
const TRAP_PORT: u16 = 0x07ED;

/// Builds a minimal 2DD D88 image (80 cylinders, 2 heads, 8 sectors per track,
/// 512-byte sectors). The first three bytes of each sector are the sector
/// CHR markers (cylinder, head, record) followed by 0x00 fill.
fn build_test_2dd_d88(sectors: &[(u8, u8, u8, u8, &[u8])], write_protected: bool) -> Vec<u8> {
    build_test_d88(0x10, sectors, write_protected)
}

fn build_test_2hd_d88(sectors: &[(u8, u8, u8, u8, &[u8])], write_protected: bool) -> Vec<u8> {
    build_test_d88(0x20, sectors, write_protected)
}

fn build_test_d88(
    media_type: u8,
    sectors: &[(u8, u8, u8, u8, &[u8])],
    write_protected: bool,
) -> Vec<u8> {
    const HEADER_SIZE: usize = 0x2B0;
    const SECTOR_HEADER_SIZE: usize = 16;

    let mut image = vec![0u8; HEADER_SIZE];
    image[..4].copy_from_slice(b"TEST");
    if write_protected {
        image[0x1A] = 0x10;
    }
    image[0x1B] = media_type;

    let track_offset = HEADER_SIZE as u32;
    image[0x20..0x24].copy_from_slice(&track_offset.to_le_bytes());

    let mut track_data = Vec::new();
    for &(c, h, r, n, data) in sectors {
        let mut header = [0u8; SECTOR_HEADER_SIZE];
        header[0] = c;
        header[1] = h;
        header[2] = r;
        header[3] = n;
        let sector_count = sectors.len() as u16;
        header[4..6].copy_from_slice(&sector_count.to_le_bytes());
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

/// Builds an unprotected 2DD test floppy with one sector at C=0, H=0, R=1.
fn make_test_2dd_floppy() -> FloppyImage {
    let mut sector = vec![0u8; SECTOR_512];
    sector[0] = 0; // cylinder marker
    sector[1] = 0; // head marker
    sector[2] = 1; // record marker
    let bytes = build_test_2dd_d88(&[(0, 0, 1, 2, &sector)], false);
    FloppyImage::from_d88_bytes(&bytes).expect("2DD parse")
}

fn make_test_2dd_floppy_write_protected() -> FloppyImage {
    let mut sector = vec![0u8; SECTOR_512];
    sector[0] = 0;
    sector[1] = 0;
    sector[2] = 1;
    let bytes = build_test_2dd_d88(&[(0, 0, 1, 2, &sector)], true);
    FloppyImage::from_d88_bytes(&bytes).expect("2DD WP parse")
}

fn make_xanadu_style_2hd_floppy() -> FloppyImage {
    let sector1 = vec![0x11u8; 128];
    let sector2 = vec![0x22u8; 128];
    let bytes = build_test_2hd_d88(&[(0, 0, 1, 0, &sector1), (0, 0, 2, 0, &sector2)], false);
    FloppyImage::from_d88_bytes(&bytes).expect("2HD 128-byte sector parse")
}

/// Stack frame helper for FDD HLE tests.
///
/// The PC-9801-09 ROM pushes DS, SI, DI, ES, BP, DX, CX, BX, AX (9 words)
/// before dispatching to the entry-8 INT 1Bh handler. Layout at SS:SP:
/// SP+0x00: AX, SP+0x02: BX, SP+0x04: CX, SP+0x06: DX, SP+0x08: BP,
/// SP+0x0A: ES, SP+0x0C: DI, SP+0x0E: SI, SP+0x10: DS,
/// SP+0x12: IP, SP+0x14: CS, SP+0x16: FLAGS.
struct Fdd640kTestFrame {
    ss_base: u32,
    sp: u16,
}

impl Fdd640kTestFrame {
    const SS: u16 = 0x0000;
    const SP: u16 = 0x1000;

    fn new<T: TraceSink>(
        bus: &mut Pc9801Bus<T>,
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
            ax, bx, cx, dx, bp, es, 0, 0, 0, // DI, SI, DS
            0, 0, 0x0200, // IP, CS, FLAGS
        ];
        for (index, &word) in words.iter().enumerate() {
            let address = base + (index as u32) * 2;
            bus.write_byte(address, word as u8);
            bus.write_byte(address + 1, (word >> 8) as u8);
        }
        Self {
            ss_base,
            sp: Self::SP,
        }
    }

    fn result_ah<T: TraceSink>(&self, bus: &mut Pc9801Bus<T>) -> u8 {
        let base = self.ss_base + u32::from(self.sp);
        bus.read_byte(base + 1)
    }

    fn result_cf<T: TraceSink>(&self, bus: &mut Pc9801Bus<T>) -> bool {
        let base = self.ss_base + u32::from(self.sp);
        bus.read_byte(base + 0x16) & 0x01 != 0
    }

    fn read_stack_word<T: TraceSink>(&self, bus: &mut Pc9801Bus<T>, offset: u32) -> u16 {
        let base = self.ss_base + u32::from(self.sp);
        let lo = bus.read_byte(base + offset) as u16;
        let hi = bus.read_byte(base + offset + 1) as u16;
        lo | (hi << 8)
    }
}

fn initialize_fdd640k_hle<T: TraceSink>(bus: &mut Pc9801Bus<T>) {
    let frame = Fdd640kTestFrame::new(bus, 0x0370, 0, 0, 0, 0, 0);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);
    assert_eq!(frame.result_ah(bus), 0x00, "FDD HLE init");
}

#[test]
fn fdd_hle_rom_mapped_at_d6000_when_2dd_inserted_on_pc9801f() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);

    assert_eq!(bus.read_byte(0xD6000), 0xFF);
    assert_eq!(bus.read_byte(0xD6009), 0xFF);

    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    // ROM bytes are present after 2DD insertion. Signature 0x55, 0xAA at offset 0x09.
    assert_eq!(bus.read_byte(0xD6009), 0x55);
    assert_eq!(bus.read_byte(0xD600A), 0xAA);
    // Past the FDD ROM (0xD7000+) should still be unmapped on PC-9801F.
    assert_eq!(bus.read_byte(0xD7000), 0xFF);
}

#[test]
fn fdd_hle_rom_mapped_at_d6000_when_2hd_inserted_on_pc9801f() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);

    assert_eq!(bus.read_byte(0xD6009), 0xFF);

    bus.insert_floppy(0, make_xanadu_style_2hd_floppy(), None);

    assert_eq!(bus.read_byte(0xD6009), 0x55);
    assert_eq!(bus.read_byte(0xD600A), 0xAA);
}

#[test]
fn fdd_hle_rom_not_installed_on_non_f_machines() {
    for model in [
        MachineModel::PC9801VM,
        MachineModel::PC9801VX,
        MachineModel::PC9801RA,
    ] {
        let mut bus = Pc9801Bus::<NoTrace>::new(model, CpuMode::High, 48000);
        bus.insert_floppy(0, make_test_2dd_floppy(), None);
        assert_eq!(
            bus.read_byte(0xD6009),
            0xFF,
            "FDD ROM should not be installed on {model:?}"
        );
    }
}

#[test]
fn fdd_hle_trap_port_sets_pending_and_yield() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    assert!(!bus.fdd640k_hle_pending());

    bus.io_write_byte(TRAP_PORT, 0x70);
    assert!(bus.fdd640k_hle_pending());
    assert!(bus.cpu_should_yield());

    let frame = Fdd640kTestFrame::new(&mut bus, 0x0170, 0, 0, 0, 0, 0);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    assert!(!bus.fdd640k_hle_pending(), "pending should clear");
}

#[test]
fn fdd_hle_trap_port_ignored_without_rom() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    bus.io_write_byte(TRAP_PORT, 0x70);
    assert!(!bus.fdd640k_hle_pending(), "no FDD ROM means no pending");
}

#[test]
fn fdd_hle_init_sets_disk_equip_high_nibble() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    // AH=0x03 (init), AL=0x70 (640KB FDD device type, drive 0).
    let frame = Fdd640kTestFrame::new(&mut bus, 0x0370, 0, 0, 0, 0, 0);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    let equip_lo = bus.read_byte(0x055C);
    let equip_hi = bus.read_byte(0x055D);
    let equip = u16::from(equip_lo) | (u16::from(equip_hi) << 8);

    // Drives 0 and 1 are equipped by default on the 640KB FDC.
    assert_eq!(
        equip & 0xF000,
        0x3000,
        "high nibble of disk equip should encode equipped drives"
    );
    assert_eq!(bus.read_byte(0x0494), (0x03 & 0x03) << 6, "PPI flag");
    assert_eq!(bus.read_byte(0x05CA), 0xFF, "format state");
    assert_eq!(frame.result_ah(&mut bus), 0x00);
    assert!(!frame.result_cf(&mut bus));
}

#[test]
fn fdd_hle_init_unmasks_master_pic_irq_0() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    // Mask all IRQs on the master PIC (port 0x02 = master IMR).
    bus.io_write_byte(0x02, 0xFF);
    let (_, imr_before, _) = bus.pic_debug();
    assert_eq!(imr_before & 0x01, 0x01, "IRQ 0 masked before init");

    let frame = Fdd640kTestFrame::new(&mut bus, 0x0370, 0, 0, 0, 0, 0);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    let (_, imr_after, _) = bus.pic_debug();
    assert_eq!(
        imr_after & 0x01,
        0x00,
        "IRQ 0 should be unmasked after init"
    );
}

#[test]
fn fdd_hle_invalid_device_type_returns_0x40() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    // AH=0x06 read with AL=0x80 (HDD/SASI device type) should be rejected.
    let frame = Fdd640kTestFrame::new(&mut bus, 0x0680, 0x0200, 0, 0x0001, 0x0000, 0x2000);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x40);
    assert!(frame.result_cf(&mut bus));
}

#[test]
fn fdd_hle_sense_no_drive_returns_0x60() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    // Drive 1 was never inserted.
    let frame = Fdd640kTestFrame::new(&mut bus, 0x0471, 0, 0, 0, 0, 0);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x60);
    assert!(frame.result_cf(&mut bus));
}

#[test]
fn fdd_hle_sense_present_drive_sets_status_bits() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);
    initialize_fdd640k_hle(&mut bus);

    let frame = Fdd640kTestFrame::new(&mut bus, 0x0470, 0, 0, 0, 0, 0);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    // 640KB 2DD media reports ready, double-sided, and 80-cylinder mode.
    let result = frame.result_ah(&mut bus);
    assert_eq!(result, 0x05);
    assert!(!frame.result_cf(&mut bus));
}

#[test]
fn fdd_hle_sense_320kb_device_sets_double_sided_bits() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);
    initialize_fdd640k_hle(&mut bus);

    let frame = Fdd640kTestFrame::new(&mut bus, 0x0450, 0, 0, 0, 0, 0);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x03);
    assert!(!frame.result_cf(&mut bus));
}

#[test]
fn fdd_hle_sense_write_protected_sets_bit_4() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy_write_protected(), None);
    initialize_fdd640k_hle(&mut bus);

    let frame = Fdd640kTestFrame::new(&mut bus, 0x0470, 0, 0, 0, 0, 0);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    let result = frame.result_ah(&mut bus);
    assert_eq!(result, 0x15);
    assert!(!frame.result_cf(&mut bus));
}

#[test]
fn fdd_hle_set_operation_mode_updates_sense_bits() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);
    initialize_fdd640k_hle(&mut bus);

    let side_frame = Fdd640kTestFrame::new(&mut bus, 0x0E70, 0, 0, 0, 0, 0);
    bus.execute_fdd640k_hle(side_frame.ss_base, side_frame.sp);
    assert_eq!(side_frame.result_ah(&mut bus), 0x00);

    let sense_frame = Fdd640kTestFrame::new(&mut bus, 0x0470, 0, 0, 0, 0, 0);
    bus.execute_fdd640k_hle(sense_frame.ss_base, sense_frame.sp);
    assert_eq!(sense_frame.result_ah(&mut bus), 0x04);

    let cylinder_frame = Fdd640kTestFrame::new(&mut bus, 0x8E70, 0, 0, 0, 0, 0);
    bus.execute_fdd640k_hle(cylinder_frame.ss_base, cylinder_frame.sp);
    assert_eq!(cylinder_frame.result_ah(&mut bus), 0x00);

    let sense_frame = Fdd640kTestFrame::new(&mut bus, 0x0470, 0, 0, 0, 0, 0);
    bus.execute_fdd640k_hle(sense_frame.ss_base, sense_frame.sp);
    assert_eq!(sense_frame.result_ah(&mut bus), 0x00);
}

#[test]
fn fdd_hle_read_single_sector_copies_to_buffer() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    for index in 0..SECTOR_512 as u32 {
        bus.write_byte(0x20000 + index, 0x00);
    }

    // AH=0x06 read, AL=0x70 (640KB FDD drive 0), BX=512, CL=0 (cyl), DH=0
    // (head), DL=1 (record), CH=2 (N=2 -> 512 bytes).
    let frame = Fdd640kTestFrame::new(&mut bus, 0x0670, 0x0200, 0x0200, 0x0001, 0x0000, 0x2000);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x00, "read should succeed");
    assert!(!frame.result_cf(&mut bus));

    // Marker bytes from build_test_2dd_d88 sector at (0,0,1).
    assert_eq!(bus.read_byte(0x20000), 0x00, "cyl marker");
    assert_eq!(bus.read_byte(0x20001), 0x00, "head marker");
    assert_eq!(bus.read_byte(0x20002), 0x01, "record marker");
}

#[test]
fn fdd_hle_read_90h_rejects_2dd_image_for_boot_fallback() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    let frame = Fdd640kTestFrame::new(&mut bus, 0x0690, 0x0200, 0x0200, 0x0001, 0x0000, 0x2000);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    assert_eq!(
        frame.result_ah(&mut bus),
        0xE0,
        "2HD access should miss on a 2DD image"
    );
    assert!(frame.result_cf(&mut bus));
}

#[test]
fn fdd_hle_read_90h_uses_actual_128_byte_sectors_for_2hd_image() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_xanadu_style_2hd_floppy(), None);

    for index in 0..0x100u32 {
        bus.write_byte(0x20000 + index, 0x00);
    }

    // The PC-9801F extension asks for one 256-byte sector (N=1), but this
    // disk stores two 128-byte sectors (N=0), like Xanadu's IPL track.
    let frame = Fdd640kTestFrame::new(&mut bus, 0x0690, 0x0100, 0x0100, 0x0001, 0x0000, 0x2000);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x00, "read should succeed");
    assert!(!frame.result_cf(&mut bus));
    assert_eq!(bus.read_byte(0x20000), 0x11, "first sector copied");
    assert_eq!(bus.read_byte(0x2007F), 0x11, "end of first sector");
    assert_eq!(bus.read_byte(0x20080), 0x22, "second sector copied");
    assert_eq!(bus.read_byte(0x200FF), 0x22, "end of second sector");
}

#[test]
fn fdd_hle_read_no_drive_returns_0x60() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    // Drive 1 was never inserted.
    let frame = Fdd640kTestFrame::new(&mut bus, 0x0671, 0x0200, 0x0200, 0x0001, 0x0000, 0x2000);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x60);
    assert!(frame.result_cf(&mut bus));
}

#[test]
fn fdd_hle_read_id_returns_chrn_in_registers() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    // AH=0x0A read-id, AL=0x70 (drive 0), CL=0 (cyl), DH=0 (head).
    let frame = Fdd640kTestFrame::new(&mut bus, 0x0A70, 0, 0x0000, 0x0000, 0, 0);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x00);

    // CL = cylinder, CH = N (size code), DH = head, DL = record.
    let cx = frame.read_stack_word(&mut bus, 0x04);
    let dx = frame.read_stack_word(&mut bus, 0x06);
    assert_eq!(cx & 0xFF, 0x00, "CL = cylinder");
    assert_eq!((cx >> 8) & 0xFF, 0x02, "CH = N=2 (512-byte sector)");
    assert_eq!((dx >> 8) & 0xFF, 0x00, "DH = head");
    assert_eq!(dx & 0xFF, 0x01, "DL = record");
}

#[test]
fn fdd_hle_write_modifies_image_and_reads_back() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    // Source buffer 0xCC at 0x20000.
    for index in 0..SECTOR_512 as u32 {
        bus.write_byte(0x20000 + index, 0xCC);
    }

    // AH=0x05 write, AL=0x70, BX=512, CL=0, DH=0, DL=1, CH=2.
    let frame = Fdd640kTestFrame::new(&mut bus, 0x0570, 0x0200, 0x0200, 0x0001, 0x0000, 0x2000);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x00, "write should succeed");

    // Read back with AH=0x06 to a different buffer.
    for index in 0..SECTOR_512 as u32 {
        bus.write_byte(0x30000 + index, 0x00);
    }
    let read_frame =
        Fdd640kTestFrame::new(&mut bus, 0x0670, 0x0200, 0x0200, 0x0001, 0x0000, 0x3000);
    bus.execute_fdd640k_hle(read_frame.ss_base, read_frame.sp);

    assert_eq!(read_frame.result_ah(&mut bus), 0x00);
    for index in 0..SECTOR_512 as u32 {
        assert_eq!(
            bus.read_byte(0x30000 + index),
            0xCC,
            "read-back byte at offset {index}"
        );
    }
}

#[test]
fn fdd_hle_write_protected_returns_0x70() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy_write_protected(), None);

    for index in 0..SECTOR_512 as u32 {
        bus.write_byte(0x20000 + index, 0xCC);
    }

    let frame = Fdd640kTestFrame::new(&mut bus, 0x0570, 0x0200, 0x0200, 0x0001, 0x0000, 0x2000);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x70);
    assert!(frame.result_cf(&mut bus));
}

#[test]
fn fdd_hle_segment_wraps_returns_error() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    // BP = 0xFFF0, length = 512 -> wraps the 64KB segment.
    let frame = Fdd640kTestFrame::new(&mut bus, 0x0670, 0x0200, 0x0200, 0x0001, 0xFFF0, 0x2000);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    assert_eq!(
        frame.result_ah(&mut bus),
        0x20,
        "non-diagnostic wrap = 0x20"
    );
    assert!(frame.result_cf(&mut bus));
}

#[test]
fn fdd_hle_diagnostic_read_segment_wraps_returns_zero() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    // AH=0x02 (diagnostic read), wrapping buffer.
    let frame = Fdd640kTestFrame::new(&mut bus, 0x0270, 0x0200, 0x0200, 0x0001, 0xFFF0, 0x2000);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    assert_eq!(
        frame.result_ah(&mut bus),
        0x00,
        "diagnostic read returns 0x00 on wrap"
    );
}

#[test]
fn fdd_hle_unsupported_function_returns_0x40() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);
    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    // AH=0x08 is unmapped.
    let frame = Fdd640kTestFrame::new(&mut bus, 0x0870, 0, 0, 0, 0, 0);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    assert_eq!(frame.result_ah(&mut bus), 0x40);
    assert!(frame.result_cf(&mut bus));
}

#[test]
fn fdd_hle_summary_trace_emitted() {
    #[derive(Default, Debug)]
    struct CaptureTrace {
        calls: Vec<common::OwnedTraceCall>,
    }
    impl TraceSink for CaptureTrace {
        fn trace(&mut self, _context: common::TraceContext, event: common::TraceEvent<'_>) {
            if let common::TraceEvent::Call(call) = event
                && call.provider == "pc98.fdd.640k"
            {
                let common::OwnedTraceEvent::Call(call) =
                    common::OwnedTraceEvent::from(common::TraceEvent::Call(call))
                else {
                    unreachable!();
                };
                self.calls.push(call);
            }
        }
    }

    let mut bus = Pc9801Bus::new_with_trace_sink(
        MachineModel::PC9801F,
        CpuMode::High,
        48000,
        CaptureTrace::default(),
    );
    bus.insert_floppy(0, make_test_2dd_floppy(), None);

    let frame = Fdd640kTestFrame::new(&mut bus, 0x0670, 0x0200, 0x0200, 0x0001, 0x0000, 0x2000);
    bus.execute_fdd640k_hle(frame.ss_base, frame.sp);

    let trace = bus.tracer();
    assert_eq!(trace.calls.len(), 2, "one enter/exit pair per HLE call");
    assert_eq!(trace.calls[0].phase, common::TraceCallPhase::Enter);
    assert_eq!(trace.calls[1].phase, common::TraceCallPhase::Exit);
    let field = |name| {
        trace.calls[1]
            .fields
            .iter()
            .find(|field| field.name == name)
            .map(|field| &field.value)
    };
    assert_eq!(
        field("function"),
        Some(&common::OwnedTraceValue::Unsigned(0x06))
    );
    assert_eq!(
        field("subfunction"),
        Some(&common::OwnedTraceValue::Unsigned(0x70))
    );
    assert_eq!(
        field("result"),
        Some(&common::OwnedTraceValue::Unsigned(0x00))
    );
}
