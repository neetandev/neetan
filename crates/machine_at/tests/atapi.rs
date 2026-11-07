//! Bus-level tests for the AT IDE secondary channel: the ATAPI CD-ROM at
//! ports 0x170-0x177/0x376 with IRQ 15, and the CD audio status report.

use common::{Bus, temporary::TemporaryFolder};
use device::cdrom::CdImage;
use machine_at::{AtBus, LoadedRoms};

/// Test CPU clock (the DX2-50 configuration).
const CPU_CLOCK_HZ: u32 = 50_000_000;

fn build_bus() -> AtBus {
    let roms = LoadedRoms {
        system_bios: vec![0u8; 0x1_0000],
        vga_bios: vec![0u8; 0x8000],
        hle: false,
    };
    AtBus::new(CPU_CLOCK_HZ, 16 << 20, roms, 48_000)
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

/// Issues a PACKET command with the given byte-count limit, ready for the CDB.
fn start_packet(bus: &mut AtBus, byte_count_limit: u16) {
    bus.io_write_byte(0x174, byte_count_limit as u8);
    bus.io_write_byte(0x175, (byte_count_limit >> 8) as u8);
    bus.io_write_byte(0x177, 0xA0);
}

/// Sends a PACKET command with the 12-byte CDB through the data port.
fn send_packet(bus: &mut AtBus, cdb: &[u8; 12]) {
    start_packet(bus, 0xFFFE);
    for pair in cdb.as_chunks::<2>().0 {
        bus.io_write_word(0x170, u16::from_le_bytes(*pair));
    }
}

/// Reads `word_count` words from the data port as little-endian bytes.
fn read_data_bytes(bus: &mut AtBus, word_count: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(word_count * 2);
    for _ in 0..word_count {
        bytes.extend_from_slice(&bus.io_read_word(0x170).to_le_bytes());
    }
    bytes
}

/// Reads the INQUIRY response, which also clears the media-change UNIT_ATTENTION.
fn clear_media_attention(bus: &mut AtBus) {
    send_packet(bus, &[0x12, 0, 0, 0, 36, 0, 0, 0, 0, 0, 0, 0]);
    read_data_bytes(bus, 18);
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

#[test]
fn iso_pvd_is_readable_over_secondary_ports() {
    let mut iso_data = vec![0u8; 2048 * 20];
    let descriptor_offset = 16 * 2048;
    iso_data[descriptor_offset] = 0x01;
    iso_data[descriptor_offset + 1..descriptor_offset + 6].copy_from_slice(b"CD001");
    iso_data[descriptor_offset + 6] = 0x01;
    let folder = TemporaryFolder::new("at_atapi_iso");
    let path = folder.write_file("disc.iso", &iso_data);

    let (image, _) = device::cdrom::load_cd_image(&path).expect("ISO image should load");
    let mut bus = build_bus();
    bus.insert_cdrom(image).unwrap();
    clear_media_attention(&mut bus);

    send_packet(&mut bus, &[0x28, 0, 0, 0, 0, 16, 0, 0, 1, 0, 0, 0]);
    let bytes = read_data_bytes(&mut bus, 1024);

    assert_eq!(bytes[0], 0x01, "PVD type byte should be 1");
    assert_eq!(&bytes[1..6], b"CD001", "PVD should expose ISO9660 magic");
    assert_eq!(bytes[6], 0x01, "PVD version byte should be 1");
}
