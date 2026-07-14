//! Bus-level tests for the AT FDC wiring: DMA channel 2 transfers,
//! terminal count, DIR media change, data-rate mismatch, the reset
//! polling drain, and CMOS floppy drive types.

use common::Bus;
use device::floppy::FloppyImage;
use machine_at::{AtBus, LoadedRoms};

/// DOR value: reset released, IRQ/DMA gate on, drive 0 selected, motor 0 on.
const DOR_RUNNING: u8 = 0x1C;

/// Test CPU clock (the DX2-50 configuration).
const CPU_CLOCK_HZ: u32 = 50_000_000;

fn build_bus() -> AtBus {
    let roms = LoadedRoms {
        system_bios: vec![0u8; 0x1_0000],
        vga_bios: vec![0u8; 0x8000],
    };
    AtBus::new(CPU_CLOCK_HZ, 16 * 1024 * 1024, roms, 48_000)
}

/// Builds a 1.44 MB image where every 512-byte sector is filled with its
/// linear sector index.
fn patterned_1440k() -> FloppyImage {
    let mut data = vec![0u8; 1_474_560];
    for (index, byte) in data.iter_mut().enumerate() {
        *byte = (index / 512) as u8;
    }
    FloppyImage::from_img_bytes(&data).unwrap()
}

fn image_720k() -> FloppyImage {
    FloppyImage::from_img_bytes(&vec![0u8; 737_280]).unwrap()
}

fn image_1200k() -> FloppyImage {
    FloppyImage::from_img_bytes(&vec![0u8; 1_228_800]).unwrap()
}

/// Initializes both PICs and unmasks only the FDC cascade path
/// (IRQ 2 on the master, IRQ 6).
fn initialize_pic(bus: &mut AtBus) {
    bus.io_write_byte(0x20, 0x11);
    bus.io_write_byte(0x21, 0x08);
    bus.io_write_byte(0x21, 0x04);
    bus.io_write_byte(0x21, 0x01);
    bus.io_write_byte(0xA0, 0x11);
    bus.io_write_byte(0xA1, 0x70);
    bus.io_write_byte(0xA1, 0x02);
    bus.io_write_byte(0xA1, 0x01);
    bus.io_write_byte(0x21, !0x44);
    bus.io_write_byte(0xA1, 0xFF);
}

/// Programs DMA channel 2 for a device-to-memory (write) or
/// memory-to-device (read) transfer of `length` bytes at `address`.
fn program_dma_channel2(bus: &mut AtBus, address: u32, length: u16, to_memory: bool) {
    let mode = if to_memory { 0x46 } else { 0x4A };
    bus.io_write_byte(0x0B, mode);
    bus.io_write_byte(0x0C, 0x00);
    bus.io_write_byte(0x04, address as u8);
    bus.io_write_byte(0x04, (address >> 8) as u8);
    bus.io_write_byte(0x81, (address >> 16) as u8);
    let count = length - 1;
    bus.io_write_byte(0x0C, 0x00);
    bus.io_write_byte(0x05, count as u8);
    bus.io_write_byte(0x05, (count >> 8) as u8);
    bus.io_write_byte(0x0A, 0x02);
    bus.io_write_byte(0xD4, 0x00);
}

/// Pumps scheduled events until the FDC enters the result phase
/// (MSR = RQM | DIO | CB) or the iteration budget runs out.
fn pump_until_result(bus: &mut AtBus) {
    for _ in 0..64 {
        if bus.io_read_byte(0x3F4) & 0xD0 == 0xD0 {
            return;
        }
        let next = bus.next_event_cycle().expect("a pending event");
        bus.set_current_cycle(next);
    }
    panic!(
        "FDC did not reach the result phase, MSR = {:#04X}",
        bus.io_read_byte(0x3F4)
    );
}

/// Pumps events for roughly one millisecond of bus time.
fn pump_one_millisecond(bus: &mut AtBus) {
    let target = bus.current_cycle() + u64::from(CPU_CLOCK_HZ) / 1000;
    while bus.current_cycle() < target {
        let step = bus
            .next_event_cycle()
            .unwrap_or(target)
            .clamp(bus.current_cycle() + 1, target);
        bus.set_current_cycle(step);
    }
}

/// Reads the seven result bytes from the data register.
fn read_result7(bus: &mut AtBus) -> [u8; 7] {
    let mut result = [0u8; 7];
    for byte in &mut result {
        *byte = bus.io_read_byte(0x3F5);
    }
    result
}

/// Issues READ DATA for C/H/R with N=2 and EOT=18.
fn issue_read(bus: &mut AtBus, cylinder: u8, head: u8, record: u8) {
    for byte in [0x46, head << 2, cylinder, head, record, 2, 18, 0x1B, 0xFF] {
        bus.io_write_byte(0x3F5, byte);
    }
}

#[test]
fn multi_sector_dma_read_stops_at_terminal_count_and_raises_irq6() {
    let mut bus = build_bus();
    initialize_pic(&mut bus);
    bus.insert_floppy(0, patterned_1440k(), None).unwrap();

    bus.io_write_byte(0x3F2, DOR_RUNNING);
    pump_one_millisecond(&mut bus);
    bus.io_write_byte(0x3F7, 0x00);

    // Two sectors into RAM at 0x2000; EOT allows the full track.
    program_dma_channel2(&mut bus, 0x2000, 1024, true);
    issue_read(&mut bus, 0, 0, 1);
    pump_until_result(&mut bus);

    assert!(bus.has_irq(), "IRQ 6 pending after completion");
    let vector = bus.acknowledge_irq();
    assert_eq!(vector, 0x08 + 6);

    let result = read_result7(&mut bus);
    assert_eq!(result[0], 0x00, "ST0 normal termination");
    assert_eq!(result[1], 0x00, "ST1 clear");
    assert_eq!(&result[3..7], &[0, 0, 3, 2], "C/H/R/N after two sectors");

    for offset in 0..512u32 {
        assert_eq!(bus.read_byte(0x2000 + offset), 0, "sector 1 byte {offset}");
    }
    for offset in 0..512u32 {
        assert_eq!(bus.read_byte(0x2200 + offset), 1, "sector 2 byte {offset}");
    }
}

#[test]
fn dma_write_persists_sector_data() {
    let mut bus = build_bus();
    bus.insert_floppy(0, patterned_1440k(), None).unwrap();
    bus.io_write_byte(0x3F2, DOR_RUNNING);
    pump_one_millisecond(&mut bus);
    bus.io_write_byte(0x3F7, 0x00);

    for offset in 0..512u32 {
        bus.write_byte(0x3000 + offset, 0xC3);
    }
    program_dma_channel2(&mut bus, 0x3000, 512, false);
    // WRITE DATA, C=0 H=0 R=5, N=2, EOT=5.
    for byte in [0x45, 0x00, 0, 0, 5, 2, 5, 0x1B, 0xFF] {
        bus.io_write_byte(0x3F5, byte);
    }
    pump_until_result(&mut bus);
    let result = read_result7(&mut bus);
    assert_eq!(result[0], 0x00);

    // Read the sector back through the FDC.
    program_dma_channel2(&mut bus, 0x4000, 512, true);
    issue_read(&mut bus, 0, 0, 5);
    pump_until_result(&mut bus);
    read_result7(&mut bus);
    for offset in 0..512u32 {
        assert_eq!(bus.read_byte(0x4000 + offset), 0xC3);
    }
}

#[test]
fn dir_media_change_clears_on_seek() {
    let mut bus = build_bus();
    bus.io_write_byte(0x3F2, DOR_RUNNING);
    pump_one_millisecond(&mut bus);

    // Motor on, empty drive: change bit set.
    assert_eq!(bus.io_read_byte(0x3F7), 0xFF);

    bus.insert_floppy(0, patterned_1440k(), None).unwrap();
    assert_eq!(bus.io_read_byte(0x3F7), 0xFF, "insert latches the bit");

    // SEEK drive 0 to cylinder 1 steps the head and clears the latch.
    bus.io_write_byte(0x3F5, 0x0F);
    bus.io_write_byte(0x3F5, 0x00);
    bus.io_write_byte(0x3F5, 0x01);
    pump_one_millisecond(&mut bus);
    pump_one_millisecond(&mut bus);
    assert_eq!(bus.io_read_byte(0x3F7), 0x7F, "step clears the bit");
}

#[test]
fn data_rate_mismatch_fails_with_missing_address_mark() {
    let mut bus = build_bus();
    bus.insert_floppy(0, image_720k(), None).unwrap();
    bus.io_write_byte(0x3F2, DOR_RUNNING);
    pump_one_millisecond(&mut bus);

    // 500 kbps against 720 KB media.
    bus.io_write_byte(0x3F7, 0x00);
    program_dma_channel2(&mut bus, 0x2000, 512, true);
    issue_read(&mut bus, 0, 0, 1);
    pump_until_result(&mut bus);
    let result = read_result7(&mut bus);
    assert_eq!(result[0] & 0xC0, 0x40, "abnormal termination");
    assert_eq!(result[1], 0x01, "ST1 missing address mark");

    // 250 kbps matches.
    bus.io_write_byte(0x3F7, 0x02);
    program_dma_channel2(&mut bus, 0x2000, 512, true);
    issue_read(&mut bus, 0, 0, 1);
    pump_until_result(&mut bus);
    let result = read_result7(&mut bus);
    assert_eq!(result[0] & 0xC0, 0x00, "normal termination at 250 kbps");
}

#[test]
fn dor_reset_release_yields_polling_drain() {
    let mut bus = build_bus();

    // Release reset with the gate open.
    bus.io_write_byte(0x3F2, 0x0C);
    pump_one_millisecond(&mut bus);

    for drive in 0..4u8 {
        bus.io_write_byte(0x3F5, 0x08);
        let st0 = bus.io_read_byte(0x3F5);
        let pcn = bus.io_read_byte(0x3F5);
        assert_eq!(st0, 0xC0 | drive, "polling ST0 for drive {drive}");
        assert_eq!(pcn, 0);
    }
    bus.io_write_byte(0x3F5, 0x08);
    assert_eq!(bus.io_read_byte(0x3F5), 0x80, "fifth sense is invalid");
}

#[test]
fn cmos_floppy_types_follow_mounted_media() {
    let mut bus = build_bus();

    // Default: one 1.44 MB drive A.
    assert_eq!(bus.cmos_byte(0x10), 0x40);
    assert_eq!(bus.cmos_byte(0x14) & 0xC1, 0x01);

    // A 1.2 MB image in drive A and a 720 KB image in drive B.
    bus.insert_floppy(0, image_1200k(), None).unwrap();
    bus.insert_floppy(1, image_720k(), None).unwrap();
    assert_eq!(bus.cmos_byte(0x10), 0x23);
    assert_eq!(bus.cmos_byte(0x14) & 0xC1, 0x41, "two drives reported");

    // Ejecting drive B returns to a single 1.44 MB drive A default.
    bus.eject_floppy(1);
    bus.eject_floppy(0);
    assert_eq!(bus.cmos_byte(0x10), 0x40);
    assert_eq!(bus.cmos_byte(0x14) & 0xC1, 0x01);

    // The standard checksum stays valid after every update.
    let checksum: u16 = (0x10..=0x2D)
        .map(|index| u16::from(bus.cmos_byte(index)))
        .fold(0u16, u16::wrapping_add);
    let stored = (u16::from(bus.cmos_byte(0x2E)) << 8) | u16::from(bus.cmos_byte(0x2F));
    assert_eq!(checksum, stored);
}
