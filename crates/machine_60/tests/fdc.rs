//! Built-in uPD765A FDC tests, driven over the PIO data port. The PC-6601 and
//! PC-6601SR share the same non-intelligent interface.

use device::floppy::{D88MediaType, D88Sector};
use machine_60::{Pc6000Bus, Pc6000Machine, Pc6000Model};

mod harness;
use harness::{build_machine, make_sector, synthetic_d88};

const SECTOR_SIZE: usize = 256;
const SIZE_CODE_256: u8 = 1;
/// Main status register bit 7: RQM (host may transfer a byte).
const MSR_RQM: u8 = 0x80;

/// Builds a machine with `sectors` mounted in drive 0 and the motor running.
fn machine_with_disk(model: Pc6000Model, sectors: Vec<D88Sector>) -> Pc6000Machine {
    let image = synthetic_d88("DISK", D88MediaType::Disk2D, sectors);
    let mut machine = build_machine(model);
    machine.bus.insert_floppy(0, image, None);
    machine.bus.io_write(0xD6, 0x00); // motor on (active low)
    specify_non_dma(&mut machine.bus);
    machine
}

/// Issues a READ DATA command for drive 0, head 0, sectors 1..=eot at 256 bytes.
fn issue_read(bus: &mut Pc6000Bus, eot: u8) {
    issue_read_cmd(bus, 0x46, eot); // READ DATA, MFM
}

/// Issues a read-family command (`command` includes flag bits) for drive 0,
/// head 0, starting at sector 1, with the given EOT and a 256-byte size code.
fn issue_read_cmd(bus: &mut Pc6000Bus, command: u8, eot: u8) {
    bus.io_write(0xDD, command);
    for byte in [0x00, 0x00, 0x00, 0x01, SIZE_CODE_256, eot, 0x1B, 0xFF] {
        bus.io_write(0xDD, byte);
    }
}

fn specify_non_dma(bus: &mut Pc6000Bus) {
    bus.io_write(0xDD, 0x03);
    bus.io_write(0xDD, 0xBF);
    bus.io_write(0xDD, 0x27);
}

/// Builds a 256-byte sector with explicit deleted-mark and FDC status bytes.
fn sector_with(
    record: u8,
    sector_count: u16,
    first_value: u8,
    deleted: u8,
    status: u8,
) -> D88Sector {
    let mut sector = make_sector(record, sector_count, first_value);
    sector.deleted = deleted;
    sector.status = status;
    sector
}

/// Drains `count` sectors' worth of PIO bytes, returning the 7 result bytes.
fn drain_and_result(bus: &mut Pc6000Bus, sectors: usize) -> Vec<u8> {
    for _ in 0..(sectors * SECTOR_SIZE) {
        read_ready_byte(bus);
    }
    (0..7).map(|_| bus.io_read(0xDD).0).collect()
}

/// D88 deleted-data address-mark flag.
const D88_DELETED: u8 = 0x10;
/// D88 status byte: data-field CRC error.
const D88_DATA_CRC_ERROR: u8 = 0xB0;
/// ST2 bit 6: control mark.
const ST2_CONTROL_MARK: u8 = 0x40;
/// ST1 bit 5: data error (CRC).
const ST1_DATA_ERROR: u8 = 0x20;
/// ST2 bit 5: data error in data field (CRC).
const ST2_DATA_ERROR: u8 = 0x20;

/// Pumps scheduled events until the FDC releases the next PIO byte and reads it.
fn read_ready_byte(bus: &mut Pc6000Bus) -> u8 {
    for _ in 0..16_384 {
        let fire = bus
            .next_event_cycle()
            .expect("an event while waiting for RQM");
        bus.set_current_cycle(fire);
        bus.process_events();
        if bus.io_read(0xDC).0 & MSR_RQM != 0 {
            return bus.io_read(0xDD).0;
        }
    }
    panic!("FDC did not release a PIO byte");
}

#[test]
fn pio_read_streams_a_sector() {
    let mut machine = machine_with_disk(Pc6000Model::Pc6601, vec![make_sector(1, 1, 0)]);
    let bus = &mut machine.bus;
    issue_read(bus, 1);

    for expected in 0..SECTOR_SIZE {
        assert_eq!(read_ready_byte(bus), expected as u8);
    }

    let result: Vec<u8> = (0..7).map(|_| bus.io_read(0xDD).0).collect();
    assert_eq!(result[0] & 0xC0, 0x00, "normal termination");
    assert_eq!(result[5], 1, "result reports the last sector read");
}

#[test]
fn pio_read_continues_across_two_sectors() {
    let mut machine = machine_with_disk(
        Pc6000Model::Pc6601,
        vec![make_sector(1, 2, 0x00), make_sector(2, 2, 0x80)],
    );
    let bus = &mut machine.bus;
    issue_read(bus, 2);

    for expected in 0..SECTOR_SIZE {
        assert_eq!(read_ready_byte(bus), expected as u8);
    }
    for offset in 0..SECTOR_SIZE {
        assert_eq!(read_ready_byte(bus), 0x80u8.wrapping_add(offset as u8));
    }

    let result: Vec<u8> = (0..7).map(|_| bus.io_read(0xDD).0).collect();
    assert_eq!(result[5], 2, "result reports sector 2 at EOT");
}

#[test]
fn pio_write_round_trips_through_the_image() {
    let mut machine = machine_with_disk(Pc6000Model::Pc6601, vec![make_sector(1, 1, 0)]);
    let bus = &mut machine.bus;

    bus.io_write(0xDD, 0x45); // WRITE DATA, MFM
    for byte in [0x00, 0x00, 0x00, 0x01, SIZE_CODE_256, 0x01, 0x1B, 0xFF] {
        bus.io_write(0xDD, byte);
    }
    for value in 0..SECTOR_SIZE {
        for _ in 0..16_384 {
            let fire = bus.next_event_cycle().expect("a write event");
            bus.set_current_cycle(fire);
            bus.process_events();
            if bus.io_read(0xDC).0 & MSR_RQM != 0 {
                bus.io_write(0xDD, 0xA0u8.wrapping_add(value as u8));
                break;
            }
        }
    }
    let _result: Vec<u8> = (0..7).map(|_| bus.io_read(0xDD).0).collect();

    issue_read(bus, 1);
    for value in 0..SECTOR_SIZE {
        assert_eq!(read_ready_byte(bus), 0xA0u8.wrapping_add(value as u8));
    }
}

#[test]
fn empty_drive_read_reports_a_missing_address_mark() {
    let mut machine = build_machine(Pc6000Model::Pc6601);
    let bus = &mut machine.bus;
    bus.io_write(0xD6, 0x00); // motor on, no disk -> forced ready

    specify_non_dma(bus);
    issue_read(bus, 1);
    let result: Vec<u8> = (0..7).map(|_| bus.io_read(0xDD).0).collect();
    assert_eq!(result[0] & 0xC0, 0x40, "abnormal termination");
    assert_eq!(result[1] & 0x01, 0x01, "ST1 missing address mark");
}

#[test]
fn pc6601sr_reads_a_sector_identically() {
    let mut machine = machine_with_disk(Pc6000Model::Pc6601Sr, vec![make_sector(1, 1, 0)]);
    let bus = &mut machine.bus;
    issue_read(bus, 1);

    for expected in 0..SECTOR_SIZE {
        assert_eq!(read_ready_byte(bus), expected as u8);
    }
}

#[test]
fn read_data_over_deleted_sector_raises_the_control_mark() {
    let mut machine = machine_with_disk(
        Pc6000Model::Pc6601,
        vec![sector_with(1, 1, 0, D88_DELETED, 0)],
    );
    let bus = &mut machine.bus;
    issue_read(bus, 1);

    // The sector data still transfers, then the command stops with the mark set.
    for expected in 0..SECTOR_SIZE {
        assert_eq!(read_ready_byte(bus), expected as u8);
    }
    let result: Vec<u8> = (0..7).map(|_| bus.io_read(0xDD).0).collect();
    assert_eq!(result[0] & 0xC0, 0x00, "normal termination");
    assert_eq!(
        result[2] & ST2_CONTROL_MARK,
        ST2_CONTROL_MARK,
        "ST2 control mark"
    );
}

#[test]
fn read_deleted_data_reads_a_deleted_sector_without_the_mark() {
    let mut machine = machine_with_disk(
        Pc6000Model::Pc6601,
        vec![sector_with(1, 1, 0, D88_DELETED, 0)],
    );
    let bus = &mut machine.bus;
    issue_read_cmd(bus, 0x4C, 1); // READ DELETED DATA, MFM

    let result = drain_and_result(bus, 1);
    assert_eq!(result[0] & 0xC0, 0x00, "normal termination");
    assert_eq!(
        result[2] & ST2_CONTROL_MARK,
        0,
        "no control mark when marks match"
    );
}

#[test]
fn skip_flag_skips_a_deleted_mismatch() {
    let mut machine = machine_with_disk(
        Pc6000Model::Pc6601,
        vec![
            sector_with(1, 2, 0x00, D88_DELETED, 0),
            sector_with(2, 2, 0x80, 0, 0),
        ],
    );
    let bus = &mut machine.bus;
    issue_read_cmd(bus, 0x66, 2); // READ DATA, MFM, SK

    // The deleted sector 1 is skipped; sector 2's data is delivered instead.
    for offset in 0..SECTOR_SIZE {
        assert_eq!(read_ready_byte(bus), 0x80u8.wrapping_add(offset as u8));
    }
    let result: Vec<u8> = (0..7).map(|_| bus.io_read(0xDD).0).collect();
    assert_eq!(result[0] & 0xC0, 0x00, "normal termination");
    assert_eq!(
        result[2] & ST2_CONTROL_MARK,
        0,
        "skipped sector raises no mark"
    );
}

#[test]
fn data_crc_error_sets_the_data_error_bits() {
    let mut machine = machine_with_disk(
        Pc6000Model::Pc6601,
        vec![sector_with(1, 1, 0, 0, D88_DATA_CRC_ERROR)],
    );
    let bus = &mut machine.bus;
    issue_read(bus, 1);

    let result = drain_and_result(bus, 1);
    assert_eq!(result[0] & 0xC0, 0x40, "abnormal termination");
    assert_eq!(result[1] & ST1_DATA_ERROR, ST1_DATA_ERROR, "ST1 data error");
    assert_eq!(result[2] & ST2_DATA_ERROR, ST2_DATA_ERROR, "ST2 data error");
}

#[test]
fn read_track_returns_every_sector_in_physical_order() {
    // Records are out of numeric order to prove physical-order traversal.
    let mut machine = machine_with_disk(
        Pc6000Model::Pc6601,
        vec![sector_with(2, 2, 0x10, 0, 0), sector_with(1, 2, 0x90, 0, 0)],
    );
    let bus = &mut machine.bus;
    issue_read_cmd(bus, 0x42, 2); // READ DIAGNOSTIC (READ TRACK), MFM

    for offset in 0..SECTOR_SIZE {
        assert_eq!(read_ready_byte(bus), 0x10u8.wrapping_add(offset as u8));
    }
    for offset in 0..SECTOR_SIZE {
        assert_eq!(read_ready_byte(bus), 0x90u8.wrapping_add(offset as u8));
    }
    let result: Vec<u8> = (0..7).map(|_| bus.io_read(0xDD).0).collect();
    assert_eq!(result[0] & 0xC0, 0x00, "normal termination");
    assert_eq!(
        result[5], 1,
        "result reports the last physical sector's record"
    );
}

#[test]
fn external_interface_select_returns_open_bus_then_built_in() {
    let mut machine = machine_with_disk(Pc6000Model::Pc6601, vec![make_sector(1, 1, 0)]);
    let bus = &mut machine.bus;

    // Selecting the external intelligent unit (port 0xB1 bit 2) masks the
    // built-in data port.
    bus.io_write(0xB1, 0x04);
    assert_eq!(bus.io_read(0xDC).0, 0xFF, "external select reads open bus");

    // Returning to the built-in interface restores normal PIO operation.
    bus.io_write(0xB1, 0x00);
    issue_read(bus, 1);
    assert_eq!(read_ready_byte(bus), 0x00, "built-in drive streams again");
}
