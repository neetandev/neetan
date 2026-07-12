//! Bus-level tests for the AT IDE primary channel: IDENTIFY, CHS reads
//! and writes with IRQ 14, nIEN masking, and the CMOS user-type geometry.

use common::Bus;
use device::disk::HddImage;
use machineat::{AtBus, LoadedRoms};

/// Test CPU clock (the DX2-50 configuration).
const CPU_CLOCK_HZ: u32 = 50_000_000;

/// Bytes per cylinder of the AT flat geometry (16 heads x 63 sectors x 512).
const CYLINDER_BYTES: usize = 516_096;

fn build_bus() -> AtBus {
    let roms = LoadedRoms {
        system_bios: vec![0u8; 0x1_0000],
        vga_bios: vec![0u8; 0x8000],
    };
    AtBus::new(CPU_CLOCK_HZ, 16 * 1024 * 1024, roms, 48_000)
}

/// Builds a two-cylinder image where every sector starts with its LBA.
fn patterned_hdd() -> HddImage {
    let mut data = vec![0u8; 2 * CYLINDER_BYTES];
    for lba in 0..(data.len() / 512) {
        data[lba * 512] = lba as u8;
        data[lba * 512 + 1] = (lba >> 8) as u8;
    }
    HddImage::from_at_flat(data).unwrap()
}

/// Initializes both PICs and unmasks only the IDE path
/// (IRQ 2 cascade on the master, IRQ 14 on the slave).
fn initialize_pic(bus: &mut AtBus) {
    bus.io_write_byte(0x20, 0x11);
    bus.io_write_byte(0x21, 0x08);
    bus.io_write_byte(0x21, 0x04);
    bus.io_write_byte(0x21, 0x01);
    bus.io_write_byte(0xA0, 0x11);
    bus.io_write_byte(0xA1, 0x70);
    bus.io_write_byte(0xA1, 0x02);
    bus.io_write_byte(0xA1, 0x01);
    bus.io_write_byte(0x21, !0x04);
    bus.io_write_byte(0xA1, !0x40);
}

/// Pumps scheduled events until an IRQ is pending or the budget runs out.
fn pump_until_irq(bus: &mut AtBus) -> bool {
    for _ in 0..64 {
        if bus.has_irq() {
            return true;
        }
        let Some(next) = bus.next_event_cycle() else {
            return false;
        };
        bus.set_current_cycle(next);
    }
    bus.has_irq()
}

/// Selects drive 0 in CHS mode with the given head.
fn select_drive0(bus: &mut AtBus, head: u8) {
    bus.io_write_byte(0x1F6, 0xA0 | (head & 0x0F));
}

#[test]
fn identify_reports_the_flat_geometry() {
    let mut bus = build_bus();
    initialize_pic(&mut bus);
    bus.insert_hdd(0, patterned_hdd(), None).unwrap();

    select_drive0(&mut bus, 0);
    bus.io_write_byte(0x1F7, 0xEC);
    assert!(pump_until_irq(&mut bus), "IDENTIFY raises IRQ 14");
    assert_eq!(bus.acknowledge_irq(), 0x70 + 6, "slave vector for IRQ 14");

    let (status, _) = (bus.io_read_byte(0x1F7), ());
    assert_ne!(status & 0x08, 0, "DRQ set for the IDENTIFY data");

    let mut words = [0u16; 256];
    for word in &mut words {
        *word = bus.io_read_word(0x1F0);
    }
    assert_eq!(words[1], 2, "cylinders");
    assert_eq!(words[3], 16, "heads");
    assert_eq!(words[6], 63, "sectors per track");
}

#[test]
fn chs_read_returns_sector_data_and_status_clears_irq() {
    let mut bus = build_bus();
    initialize_pic(&mut bus);
    bus.insert_hdd(0, patterned_hdd(), None).unwrap();

    // READ SECTORS: CHS (0, 1, 1) = LBA 63, one sector.
    select_drive0(&mut bus, 1);
    bus.io_write_byte(0x1F2, 1);
    bus.io_write_byte(0x1F3, 1);
    bus.io_write_byte(0x1F4, 0);
    bus.io_write_byte(0x1F5, 0);
    bus.io_write_byte(0x1F7, 0x20);
    assert!(pump_until_irq(&mut bus), "READ raises IRQ 14");

    // Reading the status register deasserts the request.
    let status = bus.io_read_byte(0x1F7);
    assert_ne!(status & 0x08, 0, "DRQ set");
    bus.acknowledge_irq();
    assert!(!bus.has_irq(), "no second request pending");

    let first = bus.io_read_word(0x1F0);
    assert_eq!(first, 63, "sector starts with its LBA");
    for _ in 1..256 {
        bus.io_read_word(0x1F0);
    }
}

#[test]
fn chs_write_round_trips_through_the_image() {
    let mut bus = build_bus();
    initialize_pic(&mut bus);
    bus.insert_hdd(0, patterned_hdd(), None).unwrap();

    // WRITE SECTORS: CHS (1, 0, 2) = LBA 16*63 + 1.
    select_drive0(&mut bus, 0);
    bus.io_write_byte(0x1F2, 1);
    bus.io_write_byte(0x1F3, 2);
    bus.io_write_byte(0x1F4, 1);
    bus.io_write_byte(0x1F5, 0);
    bus.io_write_byte(0x1F7, 0x30);

    for index in 0..256u16 {
        bus.io_write_word(0x1F0, 0x5A00 | index);
    }
    assert!(pump_until_irq(&mut bus), "WRITE completion raises IRQ 14");
    bus.io_read_byte(0x1F7);
    bus.acknowledge_irq();

    // Read the sector back.
    select_drive0(&mut bus, 0);
    bus.io_write_byte(0x1F2, 1);
    bus.io_write_byte(0x1F3, 2);
    bus.io_write_byte(0x1F4, 1);
    bus.io_write_byte(0x1F5, 0);
    bus.io_write_byte(0x1F7, 0x20);
    assert!(pump_until_irq(&mut bus));
    bus.io_read_byte(0x1F7);

    for index in 0..256u16 {
        assert_eq!(bus.io_read_word(0x1F0), 0x5A00 | index, "word {index}");
    }
}

#[test]
fn nien_masks_the_interrupt_line() {
    let mut bus = build_bus();
    initialize_pic(&mut bus);
    bus.insert_hdd(0, patterned_hdd(), None).unwrap();

    // Set nIEN in the device control register, then issue IDENTIFY.
    bus.io_write_byte(0x3F6, 0x02);
    select_drive0(&mut bus, 0);
    bus.io_write_byte(0x1F7, 0xEC);
    assert!(!pump_until_irq(&mut bus), "nIEN suppresses IRQ 14");

    // The alternate status still shows the completed transfer.
    assert_ne!(bus.io_read_byte(0x3F6) & 0x08, 0, "DRQ set via alt status");
}

#[test]
fn cmos_carries_the_user_type_geometry() {
    let mut bus = build_bus();
    bus.insert_hdd(0, patterned_hdd(), None).unwrap();

    assert_eq!(
        bus.cmos_byte(0x12) >> 4,
        0xF,
        "drive 0 uses the extended type"
    );
    assert_eq!(bus.cmos_byte(0x19), 47, "user-defined type 47");
    let cylinders = u16::from_le_bytes([bus.cmos_byte(0x1B), bus.cmos_byte(0x1C)]);
    assert_eq!(cylinders, 2);
    assert_eq!(bus.cmos_byte(0x1D), 16, "heads");
    assert_eq!(bus.cmos_byte(0x23), 63, "sectors per track");

    // The standard checksum stays valid.
    let checksum: u16 = (0x10..=0x2D)
        .map(|index| u16::from(bus.cmos_byte(index)))
        .fold(0u16, u16::wrapping_add);
    let stored = (u16::from(bus.cmos_byte(0x2E)) << 8) | u16::from(bus.cmos_byte(0x2F));
    assert_eq!(checksum, stored);

    // Non-AT-flat images are rejected.
    let raw = HddImage::from_raw_flat(vec![0u8; 131_072]).unwrap();
    assert!(bus.insert_hdd(1, raw, None).is_err());
}
