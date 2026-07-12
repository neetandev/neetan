//! Bus-level tests for the AT IDE secondary channel: the ATAPI CD-ROM at
//! ports 0x170-0x177/0x376 with IRQ 15, and the CD audio status report.

use common::Bus;
use device::cdrom::CdImage;
use machineat::{AtBus, LoadedRoms};

/// Test CPU clock (the DX2-50 configuration).
const CPU_CLOCK_HZ: u32 = 50_000_000;

fn build_bus() -> AtBus {
    let roms = LoadedRoms {
        system_bios: vec![0u8; 0x1_0000],
        vga_bios: vec![0u8; 0x8000],
    };
    AtBus::new(CPU_CLOCK_HZ, 16 * 1024 * 1024, roms, 48_000)
}

/// Builds a single-data-track CD image whose sector N starts with `[N>>8, N]`.
fn make_test_cdimage() -> CdImage {
    let cue = r#"FILE "test.bin" BINARY
  TRACK 01 MODE1/2048
    INDEX 01 00:00:00
"#;
    let mut bin_data = vec![0u8; 2048 * 100];
    for i in 0..100u32 {
        let offset = i as usize * 2048;
        bin_data[offset] = (i >> 8) as u8;
        bin_data[offset + 1] = i as u8;
    }
    CdImage::from_cue(cue, bin_data).unwrap()
}

/// Initializes both PICs and unmasks the cascade plus IRQ 15 on the slave.
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
    bus.io_write_byte(0xA1, !0x80);
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

#[test]
fn atapi_signature_visible_on_secondary_ports_only() {
    let mut bus = build_bus();
    bus.insert_cdrom(make_test_cdimage()).unwrap();

    // Secondary cylinder registers carry the ATAPI signature 0xEB14.
    assert_eq!(bus.io_read_byte(0x174), 0x14);
    assert_eq!(bus.io_read_byte(0x175), 0xEB);

    // The primary channel (no drive) does not show the ATAPI signature.
    assert_ne!(bus.io_read_byte(0x1F5), 0xEB);
}

#[test]
fn inquiry_over_secondary_ports_returns_cdrom_type() {
    let mut bus = build_bus();
    bus.insert_cdrom(make_test_cdimage()).unwrap();

    // PACKET with byte-count limit 0xFFFE, then the INQUIRY CDB via 0x170.
    bus.io_write_byte(0x174, 0xFE);
    bus.io_write_byte(0x175, 0xFF);
    bus.io_write_byte(0x177, 0xA0);
    bus.io_write_word(0x170, 0x0012);
    bus.io_write_word(0x170, 0x0000);
    bus.io_write_word(0x170, 0x0024);
    bus.io_write_word(0x170, 0x0000);
    bus.io_write_word(0x170, 0x0000);
    bus.io_write_word(0x170, 0x0000);

    let first = bus.io_read_word(0x170);
    assert_eq!(first & 0xFF, 0x05, "device type CD-ROM");
    assert_eq!(first >> 8, 0x80, "removable");
}

#[test]
fn identify_packet_device_raises_irq15() {
    let mut bus = build_bus();
    initialize_pic(&mut bus);
    bus.insert_cdrom(make_test_cdimage()).unwrap();

    bus.io_write_byte(0x177, 0xA1); // IDENTIFY PACKET DEVICE.
    assert!(pump_until_irq(&mut bus), "IDENTIFY PACKET raises IRQ 15");
    assert_eq!(bus.acknowledge_irq(), 0x70 + 7, "slave vector for IRQ 15");

    // Reading the status register deasserts the request.
    let status = bus.io_read_byte(0x177);
    assert_ne!(status & 0x08, 0, "DRQ set for the IDENTIFY data");
    assert!(!bus.has_irq(), "status read cleared the interrupt");
}

#[test]
fn cd_audio_status_present_after_insert() {
    let mut bus = build_bus();
    assert!(bus.cd_audio_status().is_none(), "no disc, no status");

    bus.insert_cdrom(make_test_cdimage()).unwrap();
    assert!(
        bus.cd_audio_status().is_some(),
        "disc inserted, status present"
    );
}
