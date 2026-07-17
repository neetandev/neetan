use common::Bus as _;
use device::floppy::{
    FloppyImage,
    d88::{D88Disk, D88MediaType, D88Sector},
};
use machine_msx::{MainBusView, MsxBus, MsxModel};

/// PPI mode with primary-slot selection enabled.
const PPI_OUTPUT_MODE: u8 = 0x82;
/// PPI primary-slot register.
const PPI_PRIMARY_SLOTS: u16 = 0xA8;
/// PPI control register.
const PPI_CONTROL: u16 = 0xAB;
/// Disk-system status and command register.
const FDC_STATUS_COMMAND: u16 = 0x7FF8;
/// Disk-system sector register.
const FDC_SECTOR: u16 = 0x7FFA;
/// Disk-system data register.
const FDC_DATA: u16 = 0x7FFB;
/// Disk-system side register.
const FDC_SIDE: u16 = 0x7FFC;
/// Disk-system drive and motor register.
const FDC_DRIVE_CONTROL: u16 = 0x7FFD;
/// Disk-system IRQ and DRQ status register.
const FDC_LINE_STATUS: u16 = 0x7FFF;

/// Writes one I/O port through the Z80 bus adapter.
fn io_write(bus: &mut MsxBus, port: u16, value: u8) {
    MainBusView { bus }.io_write_byte(port, value);
}

/// Reads one memory address through the Z80 bus adapter.
fn memory_read(bus: &mut MsxBus, address: u16) -> u8 {
    MainBusView { bus }.read_byte(u32::from(address))
}

/// Writes one memory address through the Z80 bus adapter.
fn memory_write(bus: &mut MsxBus, address: u16, value: u8) {
    MainBusView { bus }.write_byte(u32::from(address), value);
}

/// Builds one double-density MSX sector.
fn floppy_image() -> FloppyImage {
    FloppyImage::from_d88(D88Disk::from_tracks(
        String::new(),
        false,
        D88MediaType::Disk2DD,
        vec![Some(vec![D88Sector {
            cylinder: 0,
            head: 0,
            record: 1,
            size_code: 2,
            sector_count: 1,
            mfm_flag: 0x00,
            deleted: 0,
            status: 0,
            reserved: [0; 5],
            data: vec![0xA5; 512],
            source_offset: None,
        }])],
    ))
}

#[test]
/// The selected disk ROM routes registers and scheduled PIO transfers.
fn sony_disk_rom_window_runs_the_wd2793_through_the_scheduler() {
    let mut bus = MsxBus::new(MsxModel::Msx2, 48_000);
    bus.insert_floppy(0, floppy_image(), std::path::PathBuf::new());

    io_write(&mut bus, PPI_CONTROL, PPI_OUTPUT_MODE);
    io_write(&mut bus, PPI_PRIMARY_SLOTS, 0x0C);
    memory_write(&mut bus, FDC_DRIVE_CONTROL, 0x80);
    assert_eq!(memory_read(&mut bus, FDC_SIDE), 0xFE);
    assert_eq!(memory_read(&mut bus, FDC_DRIVE_CONTROL), 0x84);
    memory_write(&mut bus, FDC_SECTOR, 1);
    memory_write(&mut bus, FDC_STATUS_COMMAND, 0x80);

    for _ in 0..2_000 {
        let cycle = bus.next_event_cycle().expect("disk transfer is scheduled");
        bus.set_current_cycle(cycle);
        bus.process_events();
        if memory_read(&mut bus, FDC_LINE_STATUS) & 0x80 == 0 {
            break;
        }
    }

    assert_eq!(memory_read(&mut bus, FDC_LINE_STATUS) & 0x80, 0);
    assert_eq!(memory_read(&mut bus, FDC_DATA), 0xA5);
}

#[test]
/// The mirrored disk ROM exposes the same registers in page two.
fn sony_disk_registers_are_mirrored_in_page_two() {
    let mut bus = MsxBus::new(MsxModel::Msx2, 48_000);
    io_write(&mut bus, PPI_CONTROL, PPI_OUTPUT_MODE);
    io_write(&mut bus, PPI_PRIMARY_SLOTS, 0x30);

    memory_write(&mut bus, 0xBFFC, 1);
    assert_eq!(memory_read(&mut bus, 0xBFFC), 0xFF);
}

#[test]
/// The MSX1 slot layout leaves disk-register addresses unhandled.
fn msx1_does_not_expose_the_disk_register_window() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    assert_eq!(memory_read(&mut bus, FDC_STATUS_COMMAND), 0xFF);
}
